//! Per-surface bookkeeping: the layer surface, its renderer, and its size.

use std::ptr::NonNull;

use smithay_client_toolkit::shell::wlr_layer::LayerSurface;
use smithay_client_toolkit::shell::WaylandSurface;
use tracing::debug;

use crate::render::display_list::DisplayList;
use crate::render::{Gpu, SurfaceRenderer};

pub struct SurfaceEntry {
    pub layer: LayerSurface,
    pub renderer: Option<SurfaceRenderer>,
    /// Integer buffer scale reported by SCTK, used for `set_buffer_scale`,
    /// buffer sizing, and glyph rasterization. Fractional outputs (e.g. 1.6)
    /// arrive as their ceiling; rasterizing at the ceiling and letting the
    /// compositor downsample is what keeps text sharp on those outputs.
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

        // Tell the compositor that the buffer we attach is `scale` times the
        // logical size. Without this the compositor assumes scale 1 and
        // downsamples our buffer, halving every element and shifting its
        // placement — which is exactly what went wrong before.
        self.layer.wl_surface().set_buffer_scale(scale as i32);

        if let Some(renderer) = &mut self.renderer {
            renderer.resize(gpu, self.physical, scale as f32);
        }
        debug!(
            "surface '{}' configured {}x{} logical -> {:?} physical (buffer scale {})",
            self.output_name, self.logical_width, self.logical_height, self.physical, scale
        );
    }

    /// Draw one frame and present it.
    pub fn render_with(
        &mut self,
        gpu: &Gpu,
        display: NonNull<std::ffi::c_void>,
        ctx: &mut crate::ui::PaintCtx<'_>,
        sidebar: &mut crate::ui::sidebar::Sidebar,
        input: &crate::ui::InputState,
    ) -> Result<(), String> {
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

        let mut list = DisplayList::new(ctx.theme.bg_main.with_alpha(0.0));
        let surface_rect = crate::render::display_list::Rect {
            x: 0.0,
            y: 0.0,
            width: self.logical_width as f32,
            height: self.logical_height as f32,
            radius: 0.0,
            color: ctx.theme.bg_main,
        };
        sidebar.paint(ctx, &mut list, surface_rect, input);

        let renderer = self
            .renderer
            .as_mut()
            .ok_or_else(|| "renderer missing".to_string())?;
        renderer.submit(gpu, &list, ctx.text)?;

        // Tell the compositor we are done with this surface state.
        self.layer
            .wl_surface()
            .damage_buffer(0, 0, self.physical.0 as i32, self.physical.1 as i32);
        self.layer.commit();
        Ok(())
    }
}
