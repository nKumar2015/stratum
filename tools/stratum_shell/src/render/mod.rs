//! The renderer boundary.
//!
//! Everything the shell draws goes through [`SurfaceRenderer`]. Keeping this
//! layer narrow means the wgpu/Vulkan implementation can be swapped for a
//! leaner backend later without touching layout or widgets.

pub mod display_list;
pub mod wgpu_renderer;

pub use wgpu_renderer::SurfaceRenderer;

use tracing::{info, warn};
use wgpu::{Adapter, Device, Instance, Queue};

use crate::config::GpuPreference;

/// The Vulkan loader's filter: comma-separated globs matched against ICD
/// manifest file names. Every ICD the loader loads stays mapped, and the
/// NVIDIA one also opens `/dev/nvidia*`: an iGPU shell that loaded all of them
/// used 93 MiB, against 53 MiB with only RADV.
const DRIVERS_SELECT: &str = "VK_LOADER_DRIVERS_SELECT";

/// Mesa's drivers for integrated GPUs (RADV, AMDVLK, ANV, hasvk).
const INTEGRATED_DRIVERS: &str = "*radeon*,*amd*,*intel*";

/// A vendor's driver globs and PCI vendor id.
fn vendor(gpu: GpuPreference) -> Option<(&'static str, u32)> {
    match gpu {
        GpuPreference::Auto => None,
        GpuPreference::Nvidia => Some(("*nvidia*", 0x10de)),
        GpuPreference::Radeon => Some(("*radeon*,*amd*", 0x1002)),
        GpuPreference::Intel => Some(("*intel*", 0x8086)),
    }
}

/// Integrated first, the CPU last.
fn rank(kind: wgpu::DeviceType) -> u8 {
    match kind {
        wgpu::DeviceType::IntegratedGpu => 0,
        wgpu::DeviceType::DiscreteGpu => 1,
        wgpu::DeviceType::VirtualGpu => 2,
        wgpu::DeviceType::Other => 3,
        wgpu::DeviceType::Cpu => 4,
    }
}

/// Load only `drivers` (every driver when `None`) and return the best adapter
/// that `accept` allows, with the instance that owns it.
///
/// The filter is set only while the instance is created, so the programs the
/// shell starts do not inherit it. Setting the environment is not thread-safe;
/// this runs before the shell starts any thread.
async fn probe(
    drivers: Option<&str>,
    accept: impl Fn(&wgpu::AdapterInfo) -> bool,
) -> Option<(Instance, Adapter)> {
    if let Some(drivers) = drivers {
        std::env::set_var(DRIVERS_SELECT, drivers);
    }
    let instance = Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapters = instance.enumerate_adapters(wgpu::Backends::VULKAN).await;
    if drivers.is_some() {
        std::env::remove_var(DRIVERS_SELECT);
    }
    let adapter = adapters
        .into_iter()
        .filter(|adapter| accept(&adapter.get_info()))
        .min_by_key(|adapter| rank(adapter.get_info().device_type))?;
    Some((instance, adapter))
}

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
    ///
    /// `gpu` picks the adapter, and only that vendor's Vulkan driver is
    /// loaded while choosing it. A `VK_LOADER_DRIVERS_SELECT` already in the
    /// environment wins over `gpu`.
    pub async fn new(gpu: GpuPreference) -> Result<Self, String> {
        let found = if std::env::var_os(DRIVERS_SELECT).is_some() {
            probe(None, |_| true).await
        } else {
            let explicit = match vendor(gpu) {
                Some((drivers, id)) => probe(Some(drivers), |info| info.vendor == id).await,
                None => None,
            };
            if explicit.is_none() && gpu != GpuPreference::Auto {
                warn!("no {gpu:?} GPU found; choosing one automatically");
            }
            match explicit {
                Some(found) => Some(found),
                None => match probe(Some(INTEGRATED_DRIVERS), |info| {
                    info.device_type == wgpu::DeviceType::IntegratedGpu
                })
                .await
                {
                    Some(found) => Some(found),
                    None => probe(None, |_| true).await,
                },
            }
        };
        let (instance, adapter) = found.ok_or("no suitable Vulkan adapter")?;
        let info = adapter.get_info();
        info!("rendering on {} ({:?})", info.name, info.device_type);

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
