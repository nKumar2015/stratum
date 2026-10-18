//! wgpu/Vulkan implementation of the renderer boundary.
//!
//! Each Wayland surface gets a matching `wgpu::Surface`. Presentation goes
//! through `VK_KHR_wayland_surface`, constructed from the raw `wl_display` and
//! `wl_surface` pointers — no EGL or `wl_egl_window` is involved.
//!
//! M1 draws rounded rectangles with an SDF shader. That single primitive covers
//! fills, borders, and corner radii crisply at any scale, and is the base the
//! rest of the vector stack builds on.

use std::ptr::NonNull;

use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};
use tracing::{debug, info};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::Proxy;

use super::display_list::{DisplayList, Rect};
use super::Gpu;

/// Per-surface presentation state.
pub struct SurfaceRenderer {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    /// Vertices for every rectangle in the current frame, rebuilt on submit.
    vertex_buffer: wgpu::Buffer,
    vertex_capacity: u64,
    size: (u32, u32),
    scale: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 2],
    local: [f32; 2],
    half_size: [f32; 2],
    radius: f32,
    _pad: f32,
    color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    /// Surface size in physical pixels.
    size: [f32; 2],
    _pad: [f32; 2],
}

const SHADER: &str = r#"
struct Globals {
    size: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0)
var<uniform> globals: Globals;

struct VertexIn {
    @location(0) position: vec2<f32>,
    @location(1) local: vec2<f32>,
    @location(2) half_size: vec2<f32>,
    @location(3) radius: f32,
    @location(4) color: vec4<f32>,
};

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) half_size: vec2<f32>,
    @location(2) radius: f32,
    @location(3) color: vec4<f32>,
};

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    // Logical coordinates (origin top-left) -> clip space.
    let normalized = in.position / globals.size;
    out.position = vec4<f32>(normalized.x * 2.0 - 1.0, 1.0 - normalized.y * 2.0, 0.0, 1.0);
    out.local = in.local;
    out.half_size = in.half_size;
    out.radius = in.radius;
    out.color = in.color;
    return out;
}

// Signed distance to a rounded box centred at the origin.
fn rounded_box_sdf(p: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let q = abs(p) - half_size + vec2<f32>(radius);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - radius;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let d = rounded_box_sdf(in.local, in.half_size, in.radius);
    // One physical pixel of analytic coverage; crisp at any scale.
    let aa = 0.5;
    let coverage = 1.0 - smoothstep(-aa, aa, d);
    return vec4<f32>(in.color.rgb, in.color.a * coverage);
}
"#;

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
            raw_display_handle: Some(RawDisplayHandle::Wayland(
                WaylandDisplayHandle::new(display),
            )),
            raw_window_handle: RawWindowHandle::Wayland(WaylandWindowHandle::new(raw_surface)),
        };

        // SAFETY: the caller guarantees the handles outlive the surface, and we
        // hold the Wayland surface for the lifetime of this struct.
        let surface = unsafe { gpu.instance.create_surface_unsafe(target) }
            .map_err(|err| format!("failed to create Vulkan surface: {err}"))?;

        let caps = surface.get_capabilities(&gpu.adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
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
            label: Some("stratum rect shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let bind_group_layout =
            gpu.device
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

        let uniform_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("stratum globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("stratum globals bind group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("stratum pipeline layout"),
                bind_group_layouts: &[Some(&bind_group_layout)],
                immediate_size: 0,
            });

        let pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("stratum rect pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![
                            0 => Float32x2,
                            1 => Float32x2,
                            2 => Float32x2,
                            3 => Float32,
                            4 => Float32x4,
                        ],
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
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });

        let vertex_capacity = std::mem::size_of::<Vertex>() as u64 * 6 * 64;
        let vertex_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("stratum rect vertices"),
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
            uniform_buffer,
            bind_group,
            vertex_buffer,
            vertex_capacity,
            size,
            scale,
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

    pub fn submit(&mut self, gpu: &Gpu, list: &DisplayList) -> Result<(), String> {
        let vertices = build_vertices(list, self.scale);

        gpu.queue.write_buffer(
            &self.uniform_buffer,
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
                    label: Some("stratum rect vertices"),
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
                // Present what we have, then reconfigure for the next frame.
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
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.draw(0..vertices.len() as u32, 0..1);
            }
        }

        gpu.queue.submit(Some(encoder.finish()));
        gpu.queue.present(frame);
        Ok(())
    }
}

/// Expand each rectangle into two triangles carrying the rect's local
/// coordinates, so the fragment shader can evaluate the rounded-box SDF.
fn build_vertices(list: &DisplayList, scale: f32) -> Vec<Vertex> {
    let mut out = Vec::with_capacity(list.rects.len() * 6);
    for Rect {
        x,
        y,
        width,
        height,
        radius,
        color,
    } in list.rects.iter().copied()
    {
        // Logical -> physical.
        let (x, y, w, h, r) = (x * scale, y * scale, width * scale, height * scale, radius * scale);
        let half = [w * 0.5, h * 0.5];
        let center = [x + half[0], y + half[1]];

        // Corner local coordinates in the rect's own space (centre origin).
        let corners = [
            ([x, y], [-half[0], -half[1]]),
            ([x + w, y], [half[0], -half[1]]),
            ([x + w, y + h], [half[0], half[1]]),
            ([x, y], [-half[0], -half[1]]),
            ([x + w, y + h], [half[0], half[1]]),
            ([x, y + h], [-half[0], half[1]]),
        ];

        for (pos, local) in corners {
            out.push(Vertex {
                position: pos,
                local,
                half_size: half,
                radius: r.min(half[0]).min(half[1]),
                _pad: 0.0,
                color: color.to_array(),
            });
        }
        let _ = center;
    }
    out
}
