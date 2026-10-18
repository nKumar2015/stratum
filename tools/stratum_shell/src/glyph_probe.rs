//! Standalone glyph-atlas probe.
//!
//! Renders shaped text into an offscreen texture using the *same* atlas upload
//! and sampling path as the shell, reads it back, and writes a PNG. This
//! isolates the glyph pipeline from Wayland, layer surfaces, and the compositor
//! so a failure is unambiguous.
//!
//! Run with:
//!   cargo test -p stratum-shell --bin stratum-shell glyph_probe -- --nocapture
//!
//! Output: `$XDG_RUNTIME_DIR/glyph-probe.png` (or `/tmp/glyph-probe.png`).

use std::sync::Arc;

use crate::render::display_list::{Color, Transform};
use crate::text::{TextEngine, TextStyle};

const WIDTH: u32 = 200;
const HEIGHT: u32 = 60;
const ATLAS: u32 = 256;

#[test]
fn glyph_probe_renders_visible_text() {
    let pixels = pollster::block_on(render_probe());
    if pixels.is_empty() {
        eprintln!("glyph probe skipped: no GPU available");
        return;
    }

    let output = std::env::var("XDG_RUNTIME_DIR")
        .map(|dir| std::path::PathBuf::from(dir).join("glyph-probe.png"))
        .unwrap_or_else(|_| std::path::PathBuf::from("/tmp/glyph-probe.png"));

    write_png(&output, WIDTH, HEIGHT, &pixels);
    eprintln!("glyph probe written to {}", output.display());

    // The probe passes when enough non-background pixels exist to form glyphs.
    let lit = pixels
        .chunks_exact(4)
        .filter(|px| px[0] > 40 || px[1] > 40 || px[2] > 40)
        .count();
    eprintln!("lit pixels: {lit}");
    assert!(lit > 50, "expected visible text, found {lit} lit pixels");
}

/// Render one text run through the atlas path and return RGBA8 pixels.
async fn render_probe() -> Vec<u8> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = match instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        })
        .await
    {
        Ok(adapter) => adapter,
        Err(err) => {
            // The Vulkan loader lives in the devshell and is not on the default
            // library path, so skip rather than fail when it cannot be loaded.
            eprintln!("skipping glyph probe: no Vulkan adapter ({err})");
            return Vec::new();
        }
    };
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor::default())
        .await
        .expect("device");

    // Shape the text.
    let mut text = TextEngine::new();
    let style = TextStyle {
        family: Arc::from("JetBrainsMono NFM"),
        size: 20.0,
        weight: 500,
        color: Color::WHITE,
    };
    let run = text.shape("Desktop", &style, 4.0, 30.0);
    let rasters = text.rasterize_run(&run, 1.0);
    assert!(!rasters.is_empty(), "no glyphs rasterized");

    // --- Use the shell's real atlas implementation. ---
    let gpu = crate::render::Gpu {
        instance: wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        }),
        adapter,
        device,
        queue,
    };
    let device = &gpu.device;
    let queue = &gpu.queue;

    let atlas_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("probe atlas layout"),
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
    let mut atlas = crate::render::wgpu_renderer::Atlas::new(&gpu, &atlas_layout);
    let mut uv_rects = Vec::new();
    for (_, glyph) in &rasters {
        uv_rects.push(atlas.upload_mask(&gpu, glyph).expect("atlas region"));
    }

    // The shell binds its own texture view; clone it out for the probe.
    let atlas_view = atlas.view().clone();
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });

    // --- Output target. ---
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("probe target"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());

    // --- Vertex data: one quad per glyph, atlas-tinted. ---
    #[repr(C)]
    #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
    struct V {
        pos: [f32; 2],
        uv: [f32; 2],
    }
    let mut verts: Vec<V> = Vec::new();
    for ((glyph, raster), uv) in rasters.iter().zip(uv_rects.iter()) {
        let x = run.x + glyph.x + raster.left as f32;
        let y = run.y + glyph.y - raster.top as f32;
        let w = raster.width as f32;
        let h = raster.height as f32;
        let (u0, v0, u1, v1) = *uv;
        for (px, py, u, v) in [
            (x, y, u0, v0),
            (x + w, y, u1, v0),
            (x + w, y + h, u1, v1),
            (x, y, u0, v0),
            (x + w, y + h, u1, v1),
            (x, y + h, u0, v1),
        ] {
            verts.push(V {
                pos: [px, py],
                uv: [u, v],
            });
        }
    }
    let vertex_bytes = bytemuck::cast_slice(&verts).to_vec();
    let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("probe vertices"),
        size: vertex_bytes.len() as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&vertex_buffer, 0, &vertex_bytes);

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("probe shader"),
        source: wgpu::ShaderSource::Wgsl(
            r#"
            struct Globals { size: vec2<f32>, _pad: vec2<f32> };
            @group(0) @binding(0) var<uniform> globals: Globals;
            @group(1) @binding(0) var atlas: texture_2d<f32>;
            @group(1) @binding(1) var samp: sampler;

            struct In {
                @location(0) pos: vec2<f32>,
                @location(1) uv: vec2<f32>,
            };
            struct Out {
                @builtin(position) pos: vec4<f32>,
                @location(0) uv: vec2<f32>,
            };

            @vertex
            fn vs(in: In) -> Out {
                var out: Out;
                let n = in.pos / globals.size;
                out.pos = vec4<f32>(n.x * 2.0 - 1.0, 1.0 - n.y * 2.0, 0.0, 1.0);
                out.uv = in.uv;
                return out;
            }

            @fragment
            fn fs(in: Out) -> @location(0) vec4<f32> {
                let s = textureSampleLevel(atlas, samp, in.uv, 0.0);
                return vec4<f32>(s.a, s.a, s.a, 1.0);
            }
            "#
            .into(),
        ),
    });

    let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("probe globals layout"),
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
    let globals_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("probe globals"),
        size: 16,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(
        &globals_buffer,
        0,
        bytemuck::cast_slice(&[WIDTH as f32, HEIGHT as f32, 0.0, 0.0]),
    );
    let globals_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("probe globals bg"),
        layout: &globals_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: globals_buffer.as_entire_binding(),
        }],
    });

    let atlas_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("probe atlas bg"),
        layout: &atlas_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&atlas_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("probe pipeline layout"),
        bind_group_layouts: &[Some(&globals_layout), Some(&atlas_layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("probe pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<V>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2],
            })],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba8Unorm,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("probe encoder"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("probe pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &globals_bg, &[]);
        pass.set_bind_group(1, &atlas_bg, &[]);
        pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        pass.draw(0..verts.len() as u32, 0..1);
    }

    // Read back.
    let row_bytes = WIDTH * 4;
    let padded_row = row_bytes.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("probe readback"),
        size: (padded_row * HEIGHT) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_row),
                rows_per_image: Some(HEIGHT),
            },
        },
        wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));

    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .expect("poll");

    let mapped = slice.get_mapped_range().expect("mapped");
    let mut pixels = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
    for row in 0..HEIGHT {
        let start = (row * padded_row) as usize;
        pixels.extend_from_slice(&mapped[start..start + (WIDTH * 4) as usize]);
    }
    drop(mapped);
    readback.unmap();

    pixels
}

fn write_png(path: &std::path::Path, width: u32, height: u32, rgba: &[u8]) {
    // Minimal PNG writer via the `image` crate is not available in a test-only
    // path without adding a dependency, so emit a PPM instead; it opens in
    // every image viewer and is trivial to inspect.
    let _ = (width, height, rgba);
    let ppm_path = path.with_extension("ppm");
    let mut out = format!("P6\n{} {}\n255\n", width, height).into_bytes();
    for px in rgba.chunks_exact(4) {
        out.extend_from_slice(&px[0..3]);
    }
    if let Err(err) = std::fs::write(&ppm_path, out) {
        eprintln!("failed to write probe image: {err}");
    } else {
        eprintln!("wrote {}", ppm_path.display());
    }
}
