//! One `ext-session-lock-v1` surface and its renderer.
//!
//! Lock surfaces are not layer surfaces, so they cannot share `SurfaceEntry`;
//! this is the same bookkeeping, cut down to what a lock surface needs. The
//! compositor sizes each one to its output, and shows nothing else while the
//! session is locked.

use std::ptr::NonNull;

use smithay_client_toolkit::session_lock::SessionLockSurface;

use crate::render::display_list::{DisplayList, Rect};
use crate::render::{Gpu, SurfaceRenderer};

pub struct LockSurface {
    /// Declared before `surface` for the same reason as in `SurfaceEntry`:
    /// the swapchain must be destroyed before the `wl_surface` it presents
    /// into, and fields drop in declaration order.
    pub renderer: Option<SurfaceRenderer>,
    pub surface: SessionLockSurface,
    pub output: wayland_client::protocol::wl_output::WlOutput,
    pub output_name: String,
    pub scale: i32,
    pub logical: (u32, u32),
    pub configured: bool,
    pub dirty: bool,
}

impl LockSurface {
    pub fn new(
        surface: SessionLockSurface,
        output: wayland_client::protocol::wl_output::WlOutput,
        output_name: String,
        scale: i32,
    ) -> Self {
        Self {
            renderer: None,
            surface,
            output,
            output_name,
            scale: scale.max(1),
            logical: (0, 0),
            configured: false,
            dirty: true,
        }
    }

    fn physical(&self) -> (u32, u32) {
        let scale = self.scale as u32;
        (self.logical.0 * scale, self.logical.1 * scale)
    }

    /// Apply the compositor's configure (always the full output size).
    pub fn resize(&mut self, gpu: &Gpu, width: u32, height: u32) {
        self.logical = (width.max(1), height.max(1));
        self.configured = width > 0 && height > 0;
        self.surface.wl_surface().set_buffer_scale(self.scale);
        let physical = self.physical();
        if let Some(renderer) = &mut self.renderer {
            renderer.resize(gpu, physical, self.scale as f32);
        }
        self.dirty = true;
    }

    pub fn logical_rect(&self) -> Rect {
        Rect {
            x: 0.0,
            y: 0.0,
            width: self.logical.0 as f32,
            height: self.logical.1 as f32,
            radius: 0.0,
            color: crate::render::display_list::Color::TRANSPARENT,
        }
    }

    pub fn render_list(
        &mut self,
        gpu: &Gpu,
        display: NonNull<std::ffi::c_void>,
        list: &DisplayList,
        text: &mut crate::text::TextEngine,
    ) -> Result<(), String> {
        if !self.configured {
            return Ok(());
        }
        let physical = self.physical();
        if self.renderer.is_none() {
            self.renderer = Some(SurfaceRenderer::new(
                gpu,
                display,
                self.surface.wl_surface(),
                physical,
                self.scale as f32,
            )?);
        }
        let renderer = self
            .renderer
            .as_mut()
            .ok_or_else(|| "renderer missing".to_string())?;
        renderer.submit(gpu, list, text)?;
        let wl = self.surface.wl_surface();
        wl.damage_buffer(0, 0, physical.0 as i32, physical.1 as i32);
        wl.commit();
        self.dirty = false;
        Ok(())
    }
}
