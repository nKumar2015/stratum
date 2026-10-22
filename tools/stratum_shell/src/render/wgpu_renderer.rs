//! wgpu/Vulkan implementation of the renderer boundary.
//!
//! Each Wayland surface gets a matching `wgpu::Surface`. Presentation goes
//! through `VK_KHR_wayland_surface`, constructed from the raw `wl_display` and
//! `wl_surface` pointers — no EGL or `wl_egl_window` is involved.
//!
//! One pipeline draws the whole display list:
//!
//! - **Rects** as SDF quads: fills, radii, and crisp edges at any scale.
//! - **Paths** tessellated with `lyon` and cached by geometry hash.
//! - **Text and images** as textured quads sampling a shared RGBA atlas.
//!
//! Solid fills, linear gradients, and radial gradients share the same vertex
//! format and are selected per-vertex, so most frames are a single draw call.

#![allow(dead_code)] // Renderer helpers used by M3 widgets.

use std::collections::HashMap;
use std::ptr::NonNull;

use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};
use tracing::{debug, info};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::Proxy;

use super::display_list::{Color, DisplayList, DrawCommand, Path, PathCommand, Paint, Rect, Transform};
use super::Gpu;
use crate::text::{RasterGlyph, TextEngine};

const ATLAS_SIZE: u32 = 1024;
/// Clear texels around each atlas entry, so linear filtering at a quad's edge
/// reads transparency rather than a neighbour (`Atlas::allocate`).
const ATLAS_GUTTER: u32 = 1;

/// Tessellated paths kept before unused ones are dropped.
const PATH_CACHE_LIMIT: usize = 256;
const SHADER: &str = include_str!("shader.wgsl");

/// A contiguous run of vertices sharing one scissor rectangle.
///
/// Scissor state lives on the render pass, so a clip is expressed by splitting
/// the frame into batches rather than by per-vertex data. Draw order is
/// preserved because every batch is a contiguous slice of the vertex buffer.
struct Batch {
    vertices: Vec<Vertex>,
    segments: Vec<Segment>,
    clip: Option<Rect>,
}

#[derive(Clone, Copy)]
struct Segment {
    start: u32,
    end: u32,
    clip: Option<Rect>,
}

impl Batch {
    fn new() -> Self {
        Self {
            vertices: Vec::new(),
            segments: Vec::new(),
            clip: None,
        }
    }

    fn set_clip(&mut self, clip: Option<Rect>) {
        self.clip = clip;
    }

    fn push(&mut self, vertex: Vertex) {
        let index = self.vertices.len() as u32;
        match self.segments.last_mut() {
            Some(last) if last.end == index && same_clip(last.clip, self.clip) => {
                last.end = index + 1;
            }
            _ => self.segments.push(Segment {
                start: index,
                end: index + 1,
                clip: self.clip,
            }),
        }
        self.vertices.push(vertex);
    }

    fn extend_from_slice(&mut self, vertices: &[Vertex]) {
        for vertex in vertices {
            self.push(*vertex);
        }
    }

    fn is_empty(&self) -> bool {
        self.vertices.is_empty()
    }

    fn len(&self) -> usize {
        self.vertices.len()
    }
}

fn same_clip(a: Option<Rect>, b: Option<Rect>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            (a.x - b.x).abs() < 0.01
                && (a.y - b.y).abs() < 0.01
                && (a.width - b.width).abs() < 0.01
                && (a.height - b.height).abs() < 0.01
        }
        _ => false,
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 2],
    local: [f32; 2],
    half_size: [f32; 2],
    radius: f32,
    /// 0 solid, 1 linear gradient, 2 radial gradient, 3 texture.
    kind: f32,
    color: [f32; 4],
    grad_a: [f32; 4],
    grad_b: [f32; 4],
    p0: [f32; 2],
    p1: [f32; 2],
    radial_radius: f32,
    /// Atlas UV rect (u0, v0, u1, v1).
    ///
    /// NOTE: no padding field before this. `vertex_attr_array!` derives offsets
    /// sequentially, so any extra field shifts `uv` and silently corrupts every
    /// textured quad.
    uv: [f32; 4],
}

impl Vertex {
    fn solid(
        position: [f32; 2],
        local: [f32; 2],
        half_size: [f32; 2],
        radius: f32,
        color: [f32; 4],
    ) -> Self {
        Self {
            position,
            local,
            half_size,
            radius,
            kind: 0.0,
            color,
            grad_a: color,
            grad_b: color,
            p0: [0.0; 2],
            p1: [0.0; 2],
            radial_radius: 0.0,
            uv: [0.0; 4],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    size: [f32; 2],
    _pad: [f32; 2],
}

const VERTEX_ATTRS: [wgpu::VertexAttribute; 12] = wgpu::vertex_attr_array![
    0 => Float32x2, // position
    1 => Float32x2, // local
    2 => Float32x2, // half_size
    3 => Float32,   // radius
    4 => Float32,   // kind
    5 => Float32x4, // color
    6 => Float32x4, // grad_a
    7 => Float32x4, // grad_b
    8 => Float32x2, // p0
    9 => Float32x2, // p1
    10 => Float32,  // radial_radius
    11 => Float32x4,// uv
];

/// Per-surface presentation state.
pub struct SurfaceRenderer {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    globals_buffer: wgpu::Buffer,
    globals_bind_group: wgpu::BindGroup,
    atlas: Atlas,
    vertex_buffer: wgpu::Buffer,
    vertex_capacity: u64,
    size: (u32, u32),
    scale: f32,
    path_cache: HashMap<u64, Vec<Vertex>>,
    /// The layout the atlas binds with, reused for the backdrop texture.
    texture_layout: wgpu::BindGroupLayout,
    backdrop: Option<BackdropTexture>,
}

/// A `Backdrop` uploaded to the GPU: its own texture (it is far larger than
/// the atlas) and the full-surface quad that draws it.
struct BackdropTexture {
    key: u64,
    _texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    quad: wgpu::Buffer,
}

impl SurfaceRenderer {
    /// Build a renderer for `surface`.
    ///
    /// # Safety
    /// `display` and `surface` must belong to a live Wayland connection that
    /// outlives this renderer.
    pub fn new(
        gpu: &Gpu,
        display: NonNull<std::ffi::c_void>,
        surface: &WlSurface,
        size: (u32, u32),
        scale: f32,
    ) -> Result<Self, String> {
        let raw_surface = NonNull::new(surface.id().as_ptr().cast())
            .ok_or_else(|| "wl_surface pointer was null".to_string())?;

        let target = wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: Some(RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
                display,
            ))),
            raw_window_handle: RawWindowHandle::Wayland(WaylandWindowHandle::new(raw_surface)),
        };

        // SAFETY: the caller guarantees the handles outlive the surface, and we
        // hold the Wayland surface for the lifetime of this struct.
        let surface = unsafe { gpu.instance.create_surface_unsafe(target) }
            .map_err(|err| format!("failed to create Vulkan surface: {err}"))?;

        let caps = surface.get_capabilities(&gpu.adapter);
        // Our shader emits sRGB-encoded colour values (matching the QML
        // implementation). Prefer a plain 8-bit UNORM format, which stores
        // those bytes unchanged; an *Srgb format would encode them again and
        // float formats are linear, so both would shift the colours.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| matches!(f, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm))
            .unwrap_or(caps.formats[0]);

        // Overlays (OSD, toasts) are cleared to transparent every frame, so the
        // surface must actually honour the alpha channel. `Auto` often resolves
        // to `Opaque` on Vulkan/Wayland, which is what turned a faded-out
        // overlay into an opaque black rectangle. Prefer a transparent mode and
        // fall back only when the compositor offers nothing better.
        let order = [
            wgpu::CompositeAlphaMode::PreMultiplied,
            wgpu::CompositeAlphaMode::PostMultiplied,
            wgpu::CompositeAlphaMode::Inherit,
        ];
        let alpha_mode = order
            .into_iter()
            .find(|mode| caps.alpha_modes.contains(mode))
            .unwrap_or(caps.alpha_modes[0]);

        // Not Fifo. On Wayland, Fifo makes a present wait for the surface's
        // frame callback, and a surface the compositor is not showing (an
        // overlay unmapped mid-frame, anything behind the lock screen) never
        // gets one: the present blocked forever and froze the whole event
        // loop. Wayland compositors never tear, and the shell paces its own
        // redraws, so Mailbox costs nothing and never waits on a callback.
        let present_mode = [wgpu::PresentMode::Mailbox, wgpu::PresentMode::Immediate]
            .into_iter()
            .find(|mode| caps.present_modes.contains(mode))
            .unwrap_or(wgpu::PresentMode::Fifo);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.0.max(1),
            height: size.1.max(1),
            present_mode,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&gpu.device, &config);

        let shader = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("stratum shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let globals_layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("stratum globals layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let globals_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("stratum globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let globals_bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("stratum globals bind group"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buffer.as_entire_binding(),
            }],
        });

        let atlas_layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("stratum atlas layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let atlas = Atlas::new(gpu, &atlas_layout);

        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("stratum pipeline layout"),
                bind_group_layouts: &[Some(&globals_layout), Some(&atlas_layout)],
                immediate_size: 0,
            });

        let pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("stratum pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &VERTEX_ATTRS,
                    })],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                // No culling: a -90 degree transform reverses winding, and the
                // vectorheart styling will rotate shapes freely.
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });

        let vertex_capacity = std::mem::size_of::<Vertex>() as u64 * 6 * 256;
        let vertex_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("stratum vertices"),
            size: vertex_capacity,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        info!(
            "surface renderer ready: {}x{} @ {scale}x, format {format:?}",
            size.0, size.1
        );

        Ok(Self {
            surface,
            config,
            pipeline,
            globals_buffer,
            globals_bind_group,
            atlas,
            vertex_buffer,
            vertex_capacity,
            size,
            scale,
            path_cache: HashMap::new(),
            texture_layout: atlas_layout,
            backdrop: None,
        })
    }

    /// Free the backdrop texture now. A hidden surface is not painted again,
    /// so waiting for a list without one would keep it alive indefinitely.
    pub fn release_backdrop(&mut self) {
        self.backdrop = None;
    }

    /// Upload `backdrop` unless it is the one already on the GPU.
    fn ensure_backdrop(&mut self, gpu: &Gpu, backdrop: &crate::render::display_list::Backdrop) {
        if self.backdrop.as_ref().is_some_and(|b| b.key == backdrop.key) {
            return;
        }
        let (width, height) = (backdrop.width.max(1), backdrop.height.max(1));
        if backdrop.pixels.len() < (width * height * 4) as usize {
            self.backdrop = None;
            return;
        }
        let size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("stratum backdrop"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &backdrop.pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            size,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("stratum backdrop sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("stratum backdrop bind group"),
            layout: &self.texture_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });
        let quad = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("stratum backdrop quad"),
            size: (std::mem::size_of::<Vertex>() * 6) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.backdrop = Some(BackdropTexture { key: backdrop.key, _texture: texture, bind_group, quad });
    }

    pub fn resize(&mut self, gpu: &Gpu, size: (u32, u32), scale: f32) {
        if size == self.size && (scale - self.scale).abs() < f32::EPSILON {
            return;
        }
        self.size = (size.0.max(1), size.1.max(1));
        self.scale = scale;
        self.config.width = self.size.0;
        self.config.height = self.size.1;
        self.surface.configure(&gpu.device, &self.config);
        debug!("surface resized to {:?} @ {scale}x", self.size);
    }

    /// Draw and present one frame.
    pub fn submit(&mut self, gpu: &Gpu, list: &DisplayList, text: &mut TextEngine) -> Result<(), String> {
        let mut batch = Batch::new();
        self.atlas.begin_frame();
        let mut used_paths = std::collections::HashSet::new();
        match &list.backdrop {
            Some(backdrop) => self.ensure_backdrop(gpu, backdrop),
            None => self.backdrop = None,
        }
        // The backdrop's scissor, in physical pixels; `None` when it is
        // clipped away entirely.
        let mut backdrop_scissor = None;
        if let (Some(backdrop), Some(spec)) = (&self.backdrop, &list.backdrop) {
            let s = self.scale;
            let (sw, sh) = (self.size.0 as f32, self.size.1 as f32);
            let (x, y, w, h) = spec
                .dest
                .map(|d| (d.x * s, d.y * s, d.width * s, d.height * s))
                .unwrap_or((0.0, 0.0, sw, sh));
            let mut quad = Batch::new();
            append_textured_px(&mut quad, x, y, w, h, (0.0, 0.0, 1.0, 1.0), [1.0; 4]);
            gpu.queue.write_buffer(&backdrop.quad, 0, bytemuck::cast_slice(&quad.vertices));
            let (cx, cy, cw, ch) = spec
                .clip
                .map(|c| (c.x * s, c.y * s, c.width * s, c.height * s))
                .unwrap_or((0.0, 0.0, sw, sh));
            let (x0, y0) = (cx.max(0.0).floor(), cy.max(0.0).floor());
            let (x1, y1) = ((cx + cw).min(sw).ceil(), (cy + ch).min(sh).ceil());
            if x1 > x0 && y1 > y0 {
                backdrop_scissor = Some((x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32));
            }
        }

        for command in &list.commands {
            match command {
                DrawCommand::Rect(rect) => self.append_rect(&mut batch, rect),
                DrawCommand::Path(path) => {
                    let key = path_hash(path);
                    used_paths.insert(key);
                    if let Some(cached) = self.path_cache.get(&key) {
                        batch.extend_from_slice(cached);
                    } else {
                        let tessellated = tessellate_path(path, self.scale);
                        self.path_cache.insert(key, tessellated.clone());
                        batch.extend_from_slice(&tessellated);
                    }
                }
                DrawCommand::Image(image) => {
                    if let Some(uv) = self.atlas.upload_rgba(
                        gpu,
                        &image.pixels,
                        image.width,
                        image.height,
                    ) {
                        append_textured(
                            &mut batch,
                            image.x,
                            image.y,
                            image.width_logical,
                            image.height_logical,
                            uv,
                            self.scale,
                            [1.0, 1.0, 1.0, 1.0],
                        );
                    }
                }
                DrawCommand::PushClip(clip) => batch.set_clip(Some(*clip)),
                DrawCommand::PopClip => batch.set_clip(None),
            }
        }

        // Glyphs last so text sits above geometry in the same draw. Each run
        // carries the clip that was active when it was pushed.
        let mut glyph_uploads = Vec::new();
        for run in &list.texts {
            let rasterized = text.rasterize_run(run, self.scale);
            for (glyph, raster) in rasterized {
                glyph_uploads.push((
                    glyph,
                    run.x,
                    run.y,
                    run.size,
                    run.color,
                    run.transform,
                    run.clip,
                    raster,
                ));
            }
        }
        // Paths whose geometry changes every frame (a dragged curve) would
        // otherwise add a cache entry per frame forever. Past a bound, keep
        // only what this frame drew.
        if self.path_cache.len() > PATH_CACHE_LIMIT {
            self.path_cache.retain(|key, _| used_paths.contains(key));
        }
        for (glyph, run_x, run_y, run_size, color, transform, clip, raster) in glyph_uploads {
            if let Some(uv) = self.atlas.upload_mask(gpu, &raster) {
                // `run_x`/`run_y` place the run's baseline origin; glyph
                // offsets are relative to it. Swash's bitmap origin is
                // (left, -top) from the pen.
                let scale = self.scale;
                let pen_x = run_x + glyph.x + glyph.x_offset * run_size;
                let pen_y = run_y + glyph.y - glyph.y_offset * run_size;
                let lx = pen_x + raster.left as f32 / scale;
                let ly = pen_y - raster.top as f32 / scale;
                let lw = raster.width as f32 / scale;
                let lh = raster.height as f32 / scale;
                batch.set_clip(clip);
                append_quad_transformed(
                    &mut batch,
                    transform,
                    scale,
                    lx,
                    ly,
                    lw,
                    lh,
                    uv,
                    color.with_alpha(1.0).to_array(),
                );
            }
        }

        gpu.queue.write_buffer(
            &self.globals_buffer,
            0,
            bytemuck::bytes_of(&Globals {
                size: [self.size.0 as f32, self.size.1 as f32],
                _pad: [0.0, 0.0],
            }),
        );

        if !batch.is_empty() {
            let needed = std::mem::size_of_val(batch.vertices.as_slice()) as u64;
            if needed > self.vertex_capacity {
                self.vertex_capacity = needed.next_power_of_two();
                self.vertex_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("stratum vertices"),
                    size: self.vertex_capacity,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
            }
            gpu.queue
                .write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&batch.vertices));
        }

        // Suboptimal and Outdated both mean "reconfigure and acquire again".
        // A suboptimal frame must be released before `configure`: wgpu treats
        // reconfiguring while a frame is held as a fatal validation error,
        // which aborted the shell the first time sway reported one. After one
        // reconfigure, a suboptimal frame is used as it is.
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            first @ (wgpu::CurrentSurfaceTexture::Suboptimal(_)
            | wgpu::CurrentSurfaceTexture::Outdated) => {
                drop(first);
                self.surface.configure(&gpu.device, &self.config);
                match self.surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(frame)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
                    other => return Err(format!("surface unavailable after reconfigure: {other:?}")),
                }
            }
            other => return Err(format!("no frame available: {other:?}")),
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("stratum frame"),
            });

        {
            let clear = list.clear;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("stratum pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: clear.r as f64,
                            g: clear.g as f64,
                            b: clear.b as f64,
                            a: clear.a as f64,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            // The backdrop first, beneath everything, from its own texture.
            if let (Some(backdrop), Some((x, y, w, h))) = (&self.backdrop, backdrop_scissor) {
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.globals_bind_group, &[]);
                pass.set_bind_group(1, &backdrop.bind_group, &[]);
                pass.set_vertex_buffer(0, backdrop.quad.slice(..));
                pass.set_scissor_rect(x, y, w, h);
                pass.draw(0..6, 0..1);
            }
            if !batch.is_empty() {
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.globals_bind_group, &[]);
                pass.set_bind_group(1, &self.atlas.bind_group, &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                for segment in &batch.segments {
                    match segment.clip {
                        Some(clip) => {
                            let Some(scissor) = scissor_rect(clip, self.scale, self.size) else {
                                continue;
                            };
                            pass.set_scissor_rect(
                                scissor.0,
                                scissor.1,
                                scissor.2,
                                scissor.3,
                            );
                        }
                        None => pass.set_scissor_rect(0, 0, self.size.0, self.size.1),
                    }
                    pass.draw(segment.start..segment.end, 0..1);
                }
            }
        }

        gpu.queue.submit(Some(encoder.finish()));
        gpu.queue.present(frame);
        Ok(())
    }

    fn append_rect(&self, out: &mut Batch, rect: &Rect) {
        let scale = self.scale;
        let (x, y) = (rect.x * scale, rect.y * scale);
        let (w, h) = (rect.width * scale, rect.height * scale);
        let half = [w * 0.5, h * 0.5];
        let r = (rect.radius * scale).min(half[0]).min(half[1]);
        let color = rect.color.to_array();

        for (px, py, lx, ly) in [
            (x, y, -half[0], -half[1]),
            (x + w, y, half[0], -half[1]),
            (x + w, y + h, half[0], half[1]),
            (x, y, -half[0], -half[1]),
            (x + w, y + h, half[0], half[1]),
            (x, y + h, -half[0], half[1]),
        ] {
            out.push(Vertex::solid([px, py], [lx, ly], half, r, color));
        }
    }
}

/// Convert a logical clip rect into a physical scissor rectangle.
///
/// Returns `None` when the clip is empty or entirely outside the surface, so
/// the caller can skip the draw instead of issuing invalid scissor state.
fn scissor_rect(rect: Rect, scale: f32, surface: (u32, u32)) -> Option<(u32, u32, u32, u32)> {
    let x = (rect.x * scale).round().clamp(0.0, surface.0 as f32);
    let y = (rect.y * scale).round().clamp(0.0, surface.1 as f32);
    let right = ((rect.x + rect.width) * scale).round().clamp(0.0, surface.0 as f32);
    let bottom = ((rect.y + rect.height) * scale).round().clamp(0.0, surface.1 as f32);
    let width = (right - x).max(0.0);
    let height = (bottom - y).max(0.0);
    if width < 1.0 || height < 1.0 {
        return None;
    }
    Some((x as u32, y as u32, width as u32, height as u32))
}

fn append_textured(
    out: &mut Batch,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    uv: (f32, f32, f32, f32),
    scale: f32,
    tint: [f32; 4],
) {
    append_textured_px(out, x * scale, y * scale, w * scale, h * scale, uv, tint);
}

fn append_textured_px(
    out: &mut Batch,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    uv: (f32, f32, f32, f32),
    tint: [f32; 4],
) {
    let (u0, v0, u1, v1) = uv;
    for (px, py, u, v) in [
        (x, y, u0, v0),
        (x + w, y, u1, v0),
        (x + w, y + h, u1, v1),
        (x, y, u0, v0),
        (x + w, y + h, u1, v1),
        (x, y + h, u0, v1),
    ] {
        let mut vertex = Vertex::solid([px, py], [0.0, 0.0], [0.0, 0.0], -1.0, tint);
        vertex.kind = 3.0;
        vertex.uv = [u, v, 0.0, 0.0];
        out.push(vertex);
    }
}

/// Emit a textured quad whose corners are transformed by `t`.
#[allow(clippy::too_many_arguments)] // Geometry, UVs, transform, and tint.
fn append_quad_transformed(
    out: &mut Batch,
    t: Transform,
    scale: f32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    uv: (f32, f32, f32, f32),
    tint: [f32; 4],
) {
    let (u0, v0, u1, v1) = uv;
    let corners = [
        (x, y, u0, v0),
        (x + w, y, u1, v0),
        (x + w, y + h, u1, v1),
        (x, y, u0, v0),
        (x + w, y + h, u1, v1),
        (x, y + h, u0, v1),
    ];
    for (px, py, u, v) in corners {
        let (tx, ty) = t.apply(px, py);
        let mut vertex = Vertex::solid([tx * scale, ty * scale], [0.0, 0.0], [0.0, 0.0], -1.0, tint);
        vertex.kind = 3.0;
        vertex.uv = [u, v, 0.0, 0.0];
        out.push(vertex);
    }
}

fn tessellate_path(path: &Path, scale: f32) -> Vec<Vertex> {
    use lyon::path::Path as LyonPath;
    use lyon::tessellation::{
        BuffersBuilder, FillOptions, FillTessellator, FillVertex, StrokeOptions,
        StrokeTessellator, StrokeVertex, VertexBuffers,
    };

    let mut builder = LyonPath::builder();
    let mut has_geometry = false;
    // `close()` already ends the subpath, so only an explicitly open subpath
    // (no trailing Close) needs ending before build().
    let mut subpath_open = false;
    for cmd in &path.commands {
        match cmd {
            PathCommand::MoveTo(p) => {
                builder.begin(tp(path.transform, *p, scale));
                has_geometry = true;
                subpath_open = true;
            }
            PathCommand::LineTo(p) => {
                builder.line_to(tp(path.transform, *p, scale));
            }
            PathCommand::QuadTo(a, b) => {
                builder.quadratic_bezier_to(tp(path.transform, *a, scale), tp(path.transform, *b, scale));
            }
            PathCommand::CubicTo(a, b, c) => {
                builder.cubic_bezier_to(
                    tp(path.transform, *a, scale),
                    tp(path.transform, *b, scale),
                    tp(path.transform, *c, scale),
                );
            }
            PathCommand::Close => {
                builder.close();
                subpath_open = false;
            }
        }
    }
    if !has_geometry {
        return Vec::new();
    }

    // lyon requires every subpath to be ended before the path is built.
    if subpath_open {
        builder.end(false);
    }
    let lyon_path = builder.build();
    let fill = match &path.paint {
        Paint::Solid(c) => *c,
        Paint::LinearGradient { stops, .. } | Paint::RadialGradient { stops, .. } => {
            stops.first().map(|(_, c)| *c).unwrap_or(Color::WHITE)
        }
    };

    let mut buffers: VertexBuffers<[f32; 2], u32> = VertexBuffers::new();
    if let Some(width) = path.stroke {
        let mut tess = StrokeTessellator::new();
        let mut opts = StrokeOptions::default().with_line_width(width * scale);
        if path.round {
            opts = opts
                .with_line_join(lyon::tessellation::LineJoin::Round)
                .with_line_cap(lyon::tessellation::LineCap::Round);
        }
        if tess
            .tessellate_path(
                &lyon_path,
                &opts,
                &mut BuffersBuilder::new(&mut buffers, |v: StrokeVertex| {
                    [v.position().x, v.position().y]
                }),
            )
            .is_err()
        {
            return Vec::new();
        }
    } else {
        let mut tess = FillTessellator::new();
        if tess
            .tessellate_path(
                &lyon_path,
                &FillOptions::default(),
                &mut BuffersBuilder::new(&mut buffers, |v: FillVertex| {
                    [v.position().x, v.position().y]
                }),
            )
            .is_err()
        {
            return Vec::new();
        }
    }

    let mut out = Vec::with_capacity(buffers.indices.len());
    for index in buffers.indices {
        let p = buffers.vertices[index as usize];
        out.push(Vertex::solid(
            p,
            [0.0, 0.0],
            [0.0, 0.0],
            -1.0,
            fill.to_array(),
        ));
    }
    out
}

fn tp(t: Transform, p: super::display_list::Point, scale: f32) -> lyon::math::Point {
    // Transform values are logical units; scale afterwards.
    let (x, y) = t.apply(p.x, p.y);
    lyon::math::point(x * scale, y * scale)
}

fn path_hash(path: &Path) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for cmd in &path.commands {
        match cmd {
            PathCommand::MoveTo(p) | PathCommand::LineTo(p) => {
                0u8.hash(&mut hasher);
                p.x.to_bits().hash(&mut hasher);
                p.y.to_bits().hash(&mut hasher);
            }
            PathCommand::QuadTo(a, b) => {
                1u8.hash(&mut hasher);
                for p in [a, b] {
                    p.x.to_bits().hash(&mut hasher);
                    p.y.to_bits().hash(&mut hasher);
                }
            }
            PathCommand::CubicTo(a, b, c) => {
                2u8.hash(&mut hasher);
                for p in [a, b, c] {
                    p.x.to_bits().hash(&mut hasher);
                    p.y.to_bits().hash(&mut hasher);
                }
            }
            PathCommand::Close => 3u8.hash(&mut hasher),
        }
    }
    path.stroke.map(f32::to_bits).hash(&mut hasher);
    path.round.hash(&mut hasher);
    for v in path.transform.m { v.to_bits().hash(&mut hasher); }
    // The paint must be part of the key. Tessellation bakes the colour into
    // the vertices, so two paths with identical geometry but different paint
    // are different vertices. Omitting this made a highlighted border (the
    // power menu's selection) keep the colour it was first cached with, even
    // after the highlight moved elsewhere.
    match &path.paint {
        Paint::Solid(color) => {
            0u8.hash(&mut hasher);
            for channel in [color.r, color.g, color.b, color.a] {
                channel.to_bits().hash(&mut hasher);
            }
        }
        Paint::LinearGradient { from, to, stops } => {
            1u8.hash(&mut hasher);
            for value in [from.x, from.y, to.x, to.y] {
                value.to_bits().hash(&mut hasher);
            }
            for (offset, color) in stops {
                offset.to_bits().hash(&mut hasher);
                for channel in [color.r, color.g, color.b, color.a] {
                    channel.to_bits().hash(&mut hasher);
                }
            }
        }
        Paint::RadialGradient {
            center,
            radius,
            stops,
        } => {
            2u8.hash(&mut hasher);
            center.x.to_bits().hash(&mut hasher);
            center.y.to_bits().hash(&mut hasher);
            radius.to_bits().hash(&mut hasher);
            for (offset, color) in stops {
                offset.to_bits().hash(&mut hasher);
                for channel in [color.r, color.g, color.b, color.a] {
                    channel.to_bits().hash(&mut hasher);
                }
            }
        }
    }
    hasher.finish()
}

/// A shared RGBA atlas for glyph masks and images.
pub(crate) struct Atlas {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
    data: Vec<u8>,
    packer: ShelfPacker,
}

/// Row-by-row placement of rectangles in the atlas.
///
/// Nothing keeps atlas coordinates between frames: every glyph and image is
/// uploaded again each frame. So the packer is rewound at the start of each
/// frame. Without that the cursor crept forward frame after frame until it
/// wrapped in the middle of one, and the uploads after the wrap overwrote
/// glyphs the same frame had already placed, garbling text for a frame.
#[derive(Debug, Default)]
struct ShelfPacker {
    cursor: (u32, u32),
    row_height: u32,
}

impl ShelfPacker {
    fn begin_frame(&mut self) {
        *self = Self::default();
    }

    /// Place a `w`×`h` rectangle. Returns its origin, and whether the atlas
    /// ran out of room and was restarted from the top to make it fit.
    fn allocate(&mut self, w: u32, h: u32) -> Option<((u32, u32), bool)> {
        if w == 0 || h == 0 || w > ATLAS_SIZE || h > ATLAS_SIZE {
            return None;
        }
        if self.cursor.0 + w > ATLAS_SIZE {
            self.cursor.0 = 0;
            self.cursor.1 += self.row_height;
            self.row_height = 0;
        }
        let mut wrapped = false;
        if self.cursor.1 + h > ATLAS_SIZE {
            self.cursor = (0, 0);
            self.row_height = 0;
            wrapped = true;
        }
        let pos = self.cursor;
        self.cursor.0 += w;
        self.row_height = self.row_height.max(h);
        Some((pos, wrapped))
    }
}

impl Atlas {
    /// A view of the atlas texture, for tests that need to bind it directly.
    pub(crate) fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    pub(crate) fn new(gpu: &Gpu, layout: &wgpu::BindGroupLayout) -> Self {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("stratum atlas"),
            size: wgpu::Extent3d {
                width: ATLAS_SIZE,
                height: ATLAS_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("stratum atlas sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("stratum atlas bind group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        Self {
            texture,
            view,
            bind_group,
            data: vec![0; (ATLAS_SIZE * ATLAS_SIZE * 4) as usize],
            packer: ShelfPacker::default(),
        }
    }

    /// Upload an RGBA image and return its UV rect.
    pub(crate) fn upload_rgba(
        &mut self,
        gpu: &Gpu,
        pixels: &[u8],
        width: u32,
        height: u32,
    ) -> Option<(f32, f32, f32, f32)> {
        let (x, y) = self.allocate(width, height)?;
        for row in 0..height {
            let src = (row * width * 4) as usize;
            let dst = ((y + row) * ATLAS_SIZE + x) as usize * 4;
            let len = (width * 4) as usize;
            if src + len <= pixels.len() {
                self.data[dst..dst + len].copy_from_slice(&pixels[src..src + len]);
            }
        }
        self.flush_entry(gpu, x, y, width, height);
        Some(self.uv(x, y, width, height))
    }

    /// Upload an 8-bit glyph mask as white-with-coverage RGBA.
    pub(crate) fn upload_mask(&mut self, gpu: &Gpu, glyph: &RasterGlyph) -> Option<(f32, f32, f32, f32)> {
        let (x, y) = self.allocate(glyph.width, glyph.height)?;
        for row in 0..glyph.height {
            for col in 0..glyph.width {
                let a = glyph.data[(row * glyph.width + col) as usize];
                let dst = (((y + row) * ATLAS_SIZE + x + col) * 4) as usize;
                self.data[dst] = 255;
                self.data[dst + 1] = 255;
                self.data[dst + 2] = 255;
                self.data[dst + 3] = a;
            }
        }
        self.flush_entry(gpu, x, y, glyph.width, glyph.height);
        Some(self.uv(x, y, glyph.width, glyph.height))
    }

    /// Rewind to the top of the atlas; see `ShelfPacker`.
    pub(crate) fn begin_frame(&mut self) {
        self.packer.begin_frame();
    }

    /// Room for a `w`×`h` entry inside a cleared gutter of `ATLAS_GUTTER`
    /// texels. Returns the entry's origin; upload it, then flush with
    /// [`Self::flush_entry`] so the gutter reaches the GPU too.
    ///
    /// The sampler filters linearly, so a quad at a fractional position reads
    /// a little past its entry's edge. Without a gutter that was the next
    /// entry, or a stale one from an earlier frame (the atlas is repacked
    /// every frame and never cleared), which drew as a faint line beside the
    /// glyph.
    fn allocate(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        let g = ATLAS_GUTTER;
        let ((x, y), wrapped) = self.packer.allocate(w + 2 * g, h + 2 * g)?;
        if wrapped {
            // Out of room within a single frame. A real atlas would evict by LRU.
            self.data.fill(0);
        }
        for row in y..y + h + 2 * g {
            let start = ((row * ATLAS_SIZE + x) * 4) as usize;
            self.data[start..start + ((w + 2 * g) * 4) as usize].fill(0);
        }
        Some((x + g, y + g))
    }

    /// Flush an entry placed by [`Self::allocate`], with its gutter.
    fn flush_entry(&mut self, gpu: &Gpu, x: u32, y: u32, w: u32, h: u32) {
        let g = ATLAS_GUTTER;
        self.flush_region(gpu, x - g, y - g, w + 2 * g, h + 2 * g);
    }

    fn uv(&self, x: u32, y: u32, w: u32, h: u32) -> (f32, f32, f32, f32) {
        (
            x as f32 / ATLAS_SIZE as f32,
            y as f32 / ATLAS_SIZE as f32,
            (x + w) as f32 / ATLAS_SIZE as f32,
            (y + h) as f32 / ATLAS_SIZE as f32,
        )
    }

    /// Upload one rectangle from the CPU atlas to the GPU texture.
    ///
    /// `bytes_per_row` must be a multiple of `COPY_BYTES_PER_ROW_ALIGNMENT`
    /// (256), so the region is copied into a padded scratch buffer first.
    fn flush_region(&mut self, gpu: &Gpu, x: u32, y: u32, w: u32, h: u32) {
        const ALIGN: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let unpadded = w * 4;
        let padded = unpadded.div_ceil(ALIGN) * ALIGN;

        let mut staged = vec![0u8; (padded * h) as usize];
        for row in 0..h {
            let src = (((y + row) * ATLAS_SIZE + x) * 4) as usize;
            let dst = (row * padded) as usize;
            let len = unpadded as usize;
            staged[dst..dst + len].copy_from_slice(&self.data[src..src + len]);
        }

        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            &staged,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
    }
}

/// Convenience: build a rounded rect from a colour.
pub fn fill_rect(x: f32, y: f32, width: f32, height: f32, radius: f32, color: Color) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
        radius,
        color,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packer_restarts_each_frame() {
        let mut packer = ShelfPacker::default();
        for _ in 0..10_000 {
            packer.begin_frame();
            // A frame's worth of glyphs never needs to wrap once the packer
            // is rewound, however many frames came before it.
            for _ in 0..40 {
                let (_, wrapped) = packer.allocate(24, 24).unwrap();
                assert!(!wrapped);
            }
        }
        assert_eq!(packer.allocate(8, 8), Some(((40 * 24, 0), false)));
    }

    #[test]
    fn packer_wraps_rows_then_restarts_when_full() {
        let mut packer = ShelfPacker::default();
        assert_eq!(packer.allocate(ATLAS_SIZE, 10), Some(((0, 0), false)));
        // No room left on the row: the next rectangle starts a new one.
        assert_eq!(packer.allocate(4, 4), Some(((0, 10), false)));
        assert_eq!(packer.allocate(4, ATLAS_SIZE), Some(((0, 0), true)));
        assert_eq!(packer.allocate(0, 4), None);
        assert_eq!(packer.allocate(ATLAS_SIZE + 1, 4), None);
    }
    use crate::render::display_list::{Color, Paint, Path, PathCommand, Point};

    #[test]
    fn tessellates_a_quadratic_stroke() {
        let path = Path::new(
            vec![
                PathCommand::MoveTo(Point::new(10.0, 10.0)),
                PathCommand::QuadTo(Point::new(50.0, 40.0), Point::new(90.0, 10.0)),
            ],
            Paint::Solid(Color::WHITE),
        )
        .stroked(3.0);
        let verts = tessellate_path(&path, 2.0);
        assert!(!verts.is_empty(), "expected stroke geometry");
    }

    #[test]
    fn tessellates_a_filled_triangle() {
        let path = Path::new(
            vec![
                PathCommand::MoveTo(Point::new(0.0, 0.0)),
                PathCommand::LineTo(Point::new(10.0, 0.0)),
                PathCommand::LineTo(Point::new(5.0, 10.0)),
                PathCommand::Close,
            ],
            Paint::Solid(Color::WHITE),
        );
        let verts = tessellate_path(&path, 1.0);
        assert_eq!(verts.len() % 3, 0);
        assert!(verts.len() >= 3);
    }

    #[test]
    fn path_cache_key_includes_paint() {
        // Tessellation bakes colour into the vertices. Two paths with the same
        // geometry but different colours therefore have different vertices and
        // must not share a cache entry: sharing one froze the power menu's
        // selection outline on whichever tile happened to be highlighted first.
        let geometry = || {
            vec![
                PathCommand::MoveTo(Point::new(0.0, 0.0)),
                PathCommand::LineTo(Point::new(10.0, 0.0)),
                PathCommand::LineTo(Point::new(10.0, 10.0)),
                PathCommand::Close,
            ]
        };
        let red = Path::new(geometry(), Paint::Solid(Color::rgba(1.0, 0.0, 0.0, 1.0)));
        let blue = Path::new(geometry(), Paint::Solid(Color::rgba(0.0, 0.0, 1.0, 1.0)));
        assert_ne!(path_hash(&red), path_hash(&blue));
        assert_eq!(path_hash(&red), path_hash(&red.clone()));

        // Stroke width is also baked into the vertices.
        assert_ne!(path_hash(&red), path_hash(&red.clone().stroked(2.0)));
    }
}
