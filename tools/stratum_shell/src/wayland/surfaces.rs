//! Per-surface bookkeeping: the layer surface, its renderer, and its size.

use std::ptr::NonNull;

use smithay_client_toolkit::shell::wlr_layer::LayerSurface;
use smithay_client_toolkit::shell::WaylandSurface;
use tracing::debug;

use crate::render::display_list::{Color, DisplayList, Rect};
use crate::render::{Gpu, SurfaceRenderer};

/// Colours for the M1 placeholder: the shell's default Carbon palette.
const BG_MAIN: u32 = 0x161616;
const PRIMARY: u32 = 0x4589ff;

pub struct SurfaceEntry {
    pub layer: LayerSurface,
    pub renderer: Option<SurfaceRenderer>,
    pub scale: i32,
    pub logical_width: u32,
    pub logical_height: u32,
    pub physical: (u32, u32),
    pub configured: bool,
    pub dirty: bool,
    pub frame_pending: bool,
    pub output_name: String,
}

impl SurfaceEntry {
    pub fn new(layer: LayerSurface, scale: i32, output_name: String) -> Self {
        Self {
            layer,
            renderer: None,
            scale,
            logical_width: 0,
            logical_height: 0,
            physical: (0, 0),
            configured: false,
            dirty: true,
            frame_pending: false,
            output_name,
        }
    }

    /// Apply a compositor configure and reconfigure the swapchain.
    pub fn resize(&mut self, gpu: &Gpu, logical_width: u32, logical_height: u32) {
        let scale = self.scale.max(1) as u32;
        self.logical_width = logical_width.max(1);
        self.logical_height = logical_height.max(1);
        self.physical = (self.logical_width * scale, self.logical_height * scale);
        self.configured = self.physical.0 > 1 && self.physical.1 > 1;

        if let Some(renderer) = &mut self.renderer {
            renderer.resize(gpu, self.physical, self.scale as f32);
        }
        debug!(
            "surface '{}' configured {}x{} logical -> {:?} physical",
            self.output_name, self.logical_width, self.logical_height, self.physical
        );
    }

    /// Draw one frame and present it.
    pub fn render(&mut self, gpu: &Gpu, display: NonNull<std::ffi::c_void>) -> Result<(), String> {
        if !self.configured {
            return Ok(());
        }

        if self.renderer.is_none() {
            let surface = self.layer.wl_surface();
            self.renderer = Some(SurfaceRenderer::new(
                gpu,
                display,
                surface,
                self.physical,
                self.scale as f32,
            )?);
        }

        let list = build_display_list(self);
        let renderer = self
            .renderer
            .as_mut()
            .ok_or_else(|| "renderer missing".to_string())?;
        renderer.submit(gpu, &list)?;

        // Tell the compositor we are done with this surface state.
        self.layer
            .wl_surface()
            .damage_buffer(0, 0, self.physical.0 as i32, self.physical.1 as i32);
        self.layer.commit();
        Ok(())
    }
}

/// The M1 placeholder scene: the rail background plus a rounded accent block.
pub fn build_display_list(entry: &SurfaceEntry) -> DisplayList {
    let mut list = DisplayList::new(Color::TRANSPARENT);

    let w = entry.logical_width as f32;
    let h = entry.logical_height as f32;

    list.push_rect(Rect {
        x: 0.0,
        y: 0.0,
        width: w,
        height: h,
        radius: 0.0,
        color: Color::from_hex(BG_MAIN, 1.0),
    });

    // A centred rounded block proves colour, radius, and the AA path.
    let block = 24.0_f32.min(w);
    list.push_rect(Rect {
        x: (w - block) * 0.5,
        y: 48.0,
        width: block,
        height: block,
        radius: 8.0,
        color: Color::from_hex(PRIMARY, 1.0),
    });

    list
}
