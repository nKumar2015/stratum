//! Per-surface bookkeeping: the layer surface, its renderer, and its size.

use std::ptr::NonNull;

use smithay_client_toolkit::shell::wlr_layer::LayerSurface;
use smithay_client_toolkit::shell::WaylandSurface;
use tracing::debug;

use crate::render::display_list::{
    Color, DisplayList, Path, PathCommand, Paint, Point, Rect, Transform,
};
use crate::render::{Gpu, SurfaceRenderer};
use crate::text::{TextEngine, TextStyle};

/// Colours for the M1 placeholder: the shell's default Carbon palette.
const BG_MAIN: u32 = 0x161616;
const PRIMARY: u32 = 0x4589ff;

pub struct SurfaceEntry {
    pub layer: LayerSurface,
    pub renderer: Option<SurfaceRenderer>,
    /// Integer buffer scale reported by SCTK. Fractional scales (e.g. 1.6)
    /// arrive here as their ceiling for buffer sizing purposes.
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
    pub fn render(
        &mut self,
        gpu: &Gpu,
        display: NonNull<std::ffi::c_void>,
        text: &mut TextEngine,
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

        let list = build_display_list(self, text);
        let renderer = self
            .renderer
            .as_mut()
            .ok_or_else(|| "renderer missing".to_string())?;
        renderer.submit(gpu, &list, text)?;

        // Tell the compositor we are done with this surface state.
        self.layer
            .wl_surface()
            .damage_buffer(0, 0, self.physical.0 as i32, self.physical.1 as i32);
        self.layer.commit();
        Ok(())
    }
}

/// The M2 sampler scene: a vertical rail with a rounded block, a tessellated
/// path, a gradient, and shaped text. It exercises every renderer feature and
/// is replaced by real widgets in M3.
pub fn build_display_list(entry: &SurfaceEntry, text: &mut TextEngine) -> DisplayList {
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

    // Rounded accent block (SDF path).
    let block = 24.0_f32.min(w);
    list.push_rect(Rect {
        x: (w - block) * 0.5,
        y: 48.0,
        width: block,
        height: block,
        radius: 8.0,
        color: Color::from_hex(PRIMARY, 1.0),
    });

    // A quadratic "swoosh" to exercise lyon tessellation.
    list.push_path(
        Path::new(
            vec![
                PathCommand::MoveTo(Point::new(w * 0.15, 110.0)),
                PathCommand::QuadTo(
                    Point::new(w * 1.2, 150.0),
                    Point::new(w * 0.15, 190.0),
                ),
            ],
            Paint::Solid(Color::from_hex(0x33b1ff, 0.9)),
        )
        .stroked(3.0),
    );

    // Shaped text rotated 270 degrees, exactly as the QML app-title does:
    // the run is laid out along x, pivoted about its origin, and reads up the
    // rail. This is the transform-only motion tier — no re-rasterization.
    let style = TextStyle {
        family: std::sync::Arc::from("JetBrainsMono NFM"),
        size: 12.0,
        weight: 600,
        color: Color::from_hex(0xf4f4f4, 1.0),
    };
    // Vertical label, matching the QML app-title: rotate the run -90 degrees
    // so it advances upward, then translate so its box sits in the rail.
    // Shaping happens at the origin, so the transform owns placement.
    let mut run = text.shape("Desktop", &style, 0.0, 0.0);
    // Rotating -90 degrees maps the run's advance (x) to "up" (negative y) and
    // its line height (y) to negative x. Shift by the line height first so the
    // ink lands centred in the rail, not clipped at its right edge.
    let line_height = style.size;
    let label_bottom = h - 80.0;
    run.transform = Transform::translate(0.0, line_height)
        .then(Transform::rotate(-std::f32::consts::FRAC_PI_2))
        .then(Transform::translate((w - line_height) * 0.5, label_bottom));
    list.push_text(run);

    list
}
