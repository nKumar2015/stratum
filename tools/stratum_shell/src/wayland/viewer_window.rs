//! The screenshot viewer's xdg-shell toplevel and its renderer.
//!
//! The same bookkeeping as the settings window, kept apart because that one
//! paints the control centre directly. A plain window rather than a layer
//! surface, so it can be moved, tiled and focused like any other.

use std::ptr::NonNull;

use smithay_client_toolkit::shell::xdg::window::Window;
use smithay_client_toolkit::shell::WaylandSurface;

use crate::render::display_list::{Color, DisplayList, Rect};
use crate::render::{Gpu, SurfaceRenderer};

pub struct ViewerWindow {
    /// Declared before `window` so the swapchain drops before the
    /// `wl_surface` it presents into. See `SurfaceEntry`.
    renderer: Option<SurfaceRenderer>,
    pub window: Window,
    pub scale: i32,
    pub size: (u32, u32),
    pub configured: bool,
    pub dirty: bool,
}

impl ViewerWindow {
    pub fn new(window: Window, scale: i32, size: (u32, u32)) -> Self {
        Self { renderer: None, window, scale, size, configured: false, dirty: true }
    }

    fn physical(&self) -> (u32, u32) {
        let scale = self.scale.max(1) as u32;
        (self.size.0 * scale, self.size.1 * scale)
    }

    pub fn resize(&mut self, gpu: &Gpu, width: u32, height: u32) {
        self.size = (width.max(1), height.max(1));
        self.configured = true;
        self.window.wl_surface().set_buffer_scale(self.scale.max(1));
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
            width: self.size.0 as f32,
            height: self.size.1 as f32,
            radius: 0.0,
            color: Color::TRANSPARENT,
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
                self.window.wl_surface(),
                physical,
                self.scale as f32,
            )?);
        }
        let renderer = self.renderer.as_mut().ok_or("renderer missing")?;
        renderer.submit(gpu, list, text)?;
        let wl = self.window.wl_surface();
        wl.damage_buffer(0, 0, physical.0 as i32, physical.1 as i32);
        self.window.commit();
        self.dirty = false;
        Ok(())
    }
}
