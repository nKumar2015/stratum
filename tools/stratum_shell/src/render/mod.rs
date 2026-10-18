//! The renderer boundary.
//!
//! Everything the shell draws goes through [`SurfaceRenderer`]. Keeping this
//! layer narrow means the wgpu/Vulkan implementation can be swapped for a
//! leaner backend later without touching layout or widgets.

pub mod display_list;
pub mod wgpu_renderer;

pub use wgpu_renderer::SurfaceRenderer;

use wgpu::{Adapter, Device, Instance, Queue};

/// GPU context shared by every surface.
pub struct Gpu {
    pub instance: Instance,
    pub adapter: Adapter,
    pub device: Device,
    pub queue: Queue,
}

impl Gpu {
    /// Create a Vulkan-only instance and pick a suitable adapter.
    ///
    /// The GL/EGL backend is deliberately not compiled in: on Wayland we
    /// present through `VK_KHR_wayland_surface`, and dropping GL keeps the
    /// dependency graph and binary smaller.
    pub async fn new() -> Result<Self, String> {
        let instance = Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: None,
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await
            .map_err(|err| format!("no suitable Vulkan adapter: {err}"))?;

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("stratum-shell device"),
                ..Default::default()
            })
            .await
            .map_err(|err| format!("failed to create device: {err}"))?;

        Ok(Self {
            instance,
            adapter,
            device,
            queue,
        })
    }
}
