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
const SHADER: &str = include_str!("shader.wgsl");

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

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.0.max(1),
            height: size.1.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
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
        })
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
        let mut vertices = Vec::new();

        for command in &list.commands {
            match command {
                DrawCommand::Rect(rect) => self.append_rect(&mut vertices, rect),
                DrawCommand::Path(path) => {
                    let key = path_hash(path);
                    if let Some(cached) = self.path_cache.get(&key) {
                        vertices.extend_from_slice(cached);
                    } else {
                        let tessellated = tessellate_path(path, self.scale);
                        self.path_cache.insert(key, tessellated.clone());
                        vertices.extend_from_slice(&tessellated);
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
                            &mut vertices,
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
            }
        }

        // Glyphs last so text sits above geometry in the same draw.
        let mut glyph_uploads = Vec::new();
        for run in &list.texts {
            let rasterized = text.rasterize_run(run, self.scale);
            for (glyph, raster) in rasterized {
                glyph_uploads.push((glyph, run.x, run.y, run.size, run.color, run.transform, raster));
            }
        }
        for (glyph, run_x, run_y, run_size, color, transform, raster) in glyph_uploads {
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
                append_quad_transformed(
                    &mut vertices,
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

        if !vertices.is_empty() {
            let needed = std::mem::size_of_val(vertices.as_slice()) as u64;
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
                .write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&vertices));
        }

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                self.surface.configure(&gpu.device, &self.config);
                frame
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
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

            if !vertices.is_empty() {
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.globals_bind_group, &[]);
                pass.set_bind_group(1, &self.atlas.bind_group, &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.draw(0..vertices.len() as u32, 0..1);
            }
        }

        gpu.queue.submit(Some(encoder.finish()));
        gpu.queue.present(frame);
        Ok(())
    }

    fn append_rect(&self, out: &mut Vec<Vertex>, rect: &Rect) {
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

fn append_textured(
    out: &mut Vec<Vertex>,
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
    out: &mut Vec<Vertex>,
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
fn append_quad_transformed(
    out: &mut Vec<Vertex>,
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
        let opts = StrokeOptions::default().with_line_width(width * scale);
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
    for v in path.transform.m { v.to_bits().hash(&mut hasher); }
    hasher.finish()
}

/// A shared RGBA atlas for glyph masks and images.
pub(crate) struct Atlas {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
    data: Vec<u8>,
    cursor: (u32, u32),
    row_height: u32,
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
            cursor: (0, 0),
            row_height: 0,
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
        self.flush_region(gpu, x, y, width, height);
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
        self.flush_region(gpu, x, y, glyph.width, glyph.height);
        Some(self.uv(x, y, glyph.width, glyph.height))
    }

    fn allocate(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        if w == 0 || h == 0 || w > ATLAS_SIZE || h > ATLAS_SIZE {
            return None;
        }
        if self.cursor.0 + w > ATLAS_SIZE {
            self.cursor.0 = 0;
            self.cursor.1 += self.row_height;
            self.row_height = 0;
        }
        if self.cursor.1 + h > ATLAS_SIZE {
            // Out of room: reset. A real atlas would evict by LRU.
            self.data.fill(0);
            self.cursor = (0, 0);
            self.row_height = 0;
        }
        let pos = self.cursor;
        self.cursor.0 += w;
        self.row_height = self.row_height.max(h);
        Some(pos)
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
}
