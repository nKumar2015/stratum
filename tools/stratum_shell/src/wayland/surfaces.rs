//! Per-surface bookkeeping: the layer surface, its renderer, and its size.

use std::ptr::NonNull;

use smithay_client_toolkit::shell::wlr_layer::LayerSurface;
use smithay_client_toolkit::shell::WaylandSurface;
use tracing::debug;

use crate::render::display_list::DisplayList;
use crate::render::{Gpu, SurfaceRenderer};

pub struct SurfaceEntry {
    /// The Vulkan swapchain, which references the Wayland surface below.
    ///
    /// Field order is load-bearing: Rust drops fields in declaration order, and
    /// the swapchain must be torn down *before* the `wl_surface` it presents
    /// into. Declaring `layer` first destroyed the surface mid-swapchain and
    /// segfaulted inside the NVIDIA driver (`vkDestroySwapchainKHR` writing
    /// through a released `wl_proxy`).
    pub renderer: Option<SurfaceRenderer>,
    pub layer: LayerSurface,
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
    /// The output's logical size, used to place popups anchored to its edges.
    /// `(0, 0)` until the compositor reports it.
    pub output_logical_size: (f32, f32),
    /// The output this surface belongs to, so hotplug can be matched up.
    pub output: Option<wayland_client::protocol::wl_output::WlOutput>,
    /// True while a buffer is attached. Layer surfaces stay mapped even when
    /// nothing is drawn, so this tracks whether `unmap` has work to do.
    pub mapped: bool,
    /// Input region last sent to the compositor, so an unchanged region is not
    /// recreated every frame.
    pub input_region: Option<crate::render::display_list::Rect>,
    /// Whether this surface currently takes exclusive keyboard focus. Tracked
    /// so the protocol request is only sent when it changes.
    pub keyboard_exclusive: bool,
    /// A commit has been made that the compositor will answer with a
    /// configure. Cleared when it arrives.
    pub awaiting_configure: bool,
    /// A bar's rail within its surface, as `(x, width)`, when the surface is
    /// wider than the rail to hold the inverted corners. `None` is the whole
    /// surface.
    pub rail: Option<(f32, f32)>,
}

impl SurfaceEntry {
    pub fn new(layer: LayerSurface, scale: i32, output_name: String) -> Self {
        Self {
            renderer: None,
            layer,
            scale,
            logical_width: 0,
            logical_height: 0,
            physical: (0, 0),
            configured: false,
            dirty: true,
            frame_pending: false,
            output_name,
            output_logical_size: (0.0, 0.0),
            output: None,
            mapped: false,
            input_region: None,
            keyboard_exclusive: false,
            // Every surface is created with an initial commit.
            awaiting_configure: true,
            rail: None,
        }
    }

    /// Paint the bar into `(x, width)` of this surface and take input only
    /// there, leaving the rest for the inverted corners.
    pub fn with_rail(mut self, rail: Option<(f32, f32)>) -> Self {
        self.rail = rail;
        self
    }

    /// Whether a buffer may be attached now. When not, makes sure a configure
    /// is on its way.
    ///
    /// After `unmap`, wlroots treats the surface as new: it sends nothing until
    /// the client makes a fresh commit without a buffer, so an overlay shown a
    /// second time would wait forever for a configure. That commit is made
    /// here, once.
    pub fn ready(&mut self) -> bool {
        if self.configured {
            return true;
        }
        if !self.awaiting_configure {
            self.layer.commit();
            self.awaiting_configure = true;
        }
        false
    }

    /// Switch between exclusive keyboard focus and none.
    ///
    /// Overlays that accept typing must request `Exclusive`; leaving it on while
    /// hidden would keep the compositor's keys away from the focused window.
    pub fn set_keyboard_exclusive(&mut self, exclusive: bool) {
        if self.keyboard_exclusive == exclusive {
            return;
        }
        self.layer.set_keyboard_interactivity(if exclusive {
            smithay_client_toolkit::shell::wlr_layer::KeyboardInteractivity::Exclusive
        } else {
            smithay_client_toolkit::shell::wlr_layer::KeyboardInteractivity::None
        });
        self.layer.commit();
        self.keyboard_exclusive = exclusive;
    }

    /// Record the output's logical size, used to place edge-anchored popups.
    pub fn with_output_logical_size(mut self, size: (f32, f32)) -> Self {
        self.output_logical_size = size;
        self
    }

    /// Associate this surface with the output it was created for.
    pub fn with_output(mut self, output: wayland_client::protocol::wl_output::WlOutput) -> Self {
        self.output = Some(output);
        self
    }

    /// Detach the buffer and commit, hiding the surface.
    ///
    /// A layer surface that has finished fading must be unmapped rather than
    /// left attached with a fully transparent (or, worse, opaque) buffer:
    /// leaving it mapped keeps a stale frame on screen forever, which is the
    /// "black rectangle after fading away" bug.
    ///
    /// The renderer (and its Vulkan swapchain) is kept. Tearing it down and
    /// rebuilding it on the next show races the compositor's buffer release and
    /// surfaces as a `Timeout` from `get_current_texture`.
    ///
    /// Keyboard focus is dropped in the same commit. Changing it in a commit
    /// of its own first makes wlroots send a configure, which the unmap then
    /// invalidates; SCTK's late `ack_configure` for it is a fatal "wrong
    /// configure serial" error. (Hyprland tolerates it; sway does not.)
    pub fn unmap(&mut self) {
        if !self.mapped {
            return;
        }
        if self.keyboard_exclusive {
            self.layer.set_keyboard_interactivity(
                smithay_client_toolkit::shell::wlr_layer::KeyboardInteractivity::None,
            );
            self.keyboard_exclusive = false;
        }
        self.layer.attach(None, 0, 0);
        self.layer.commit();
        self.mapped = false;
        self.dirty = false;
        // The surface must be configured again before a buffer may be attached.
        // Leaving this true made the next show attach a buffer to an
        // unconfigured surface, which is a fatal protocol error. `ready`
        // requests that configure.
        self.configured = false;
        self.awaiting_configure = false;
    }

    /// Apply a compositor configure and reconfigure the swapchain.
    pub fn resize(&mut self, gpu: &Gpu, logical_width: u32, logical_height: u32) {
        let scale = self.scale.max(1) as u32;
        self.logical_width = logical_width.max(1);
        self.logical_height = logical_height.max(1);
        self.physical = (self.logical_width * scale, self.logical_height * scale);
        self.configured = self.physical.0 > 1 && self.physical.1 > 1;
        self.awaiting_configure = false;

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
    /// Render an arbitrary display list produced by another surface owner.
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
        let renderer = self
            .renderer
            .as_mut()
            .ok_or_else(|| "renderer missing".to_string())?;
        renderer.submit(gpu, list, text)?;
        self.mapped = true;
        self.layer.wl_surface().damage_buffer(
            0,
            0,
            self.physical.0 as i32,
            self.physical.1 as i32,
        );
        self.layer.commit();
        Ok(())
    }

    /// Restrict pointer input to `rect` (logical coordinates) on this surface.
    ///
    /// Overlay surfaces are full-height so their contents can grow, but only the
    /// cards themselves should be clickable. Without an input region the whole
    /// strip swallows clicks intended for the window underneath.
    pub fn set_input_region(
        &mut self,
        compositor: &smithay_client_toolkit::compositor::CompositorState,
        rect: Option<crate::render::display_list::Rect>,
    ) {
        if rect == self.input_region {
            return;
        }
        let Ok(region) = smithay_client_toolkit::compositor::Region::new(compositor) else {
            return;
        };
        if let Some(rect) = rect {
            region.add(
                rect.x.round() as i32,
                rect.y.round() as i32,
                rect.width.max(0.0).round() as i32,
                rect.height.max(0.0).round() as i32,
            );
        }
        self.layer
            .wl_surface()
            .set_input_region(Some(region.wl_region()));
        // `Region` destroys the protocol object on drop, which is what we want:
        // the surface has already copied it.
        self.input_region = rect;
    }

    /// Convenience rect for this surface in logical coordinates.
    pub fn logical_rect(&self) -> crate::render::display_list::Rect {
        crate::render::display_list::Rect {
            x: 0.0,
            y: 0.0,
            width: self.logical_width as f32,
            height: self.logical_height as f32,
            radius: 0.0,
            color: crate::render::display_list::Color::TRANSPARENT,
        }
    }

    pub fn render_with(
        &mut self,
        gpu: &Gpu,
        display: NonNull<std::ffi::c_void>,
        compositor: &smithay_client_toolkit::compositor::CompositorState,
        ctx: &mut crate::ui::PaintCtx<'_>,
        sidebar: &mut crate::ui::sidebar::Sidebar,
        input: &crate::ui::InputState,
    ) -> Result<(), String> {
        if !self.configured {
            return Ok(());
        }

        let (rail_x, rail_width) = self
            .rail
            .unwrap_or((0.0, self.logical_width as f32));
        let surface_rect = crate::render::display_list::Rect {
            x: rail_x,
            y: 0.0,
            width: rail_width,
            height: self.logical_height as f32,
            radius: 0.0,
            color: ctx.theme.bg_main,
        };
        // The corners overlap the windows beside the rail, as QML's did, so
        // clicks there must reach those windows. Set before this frame's
        // commit so it applies now.
        if self.rail.is_some() {
            self.set_input_region(compositor, Some(surface_rect));
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

        // Fully transparent, not `bg_main` with zero alpha: the swapchain is
        // premultiplied, so colour under zero alpha is added to whatever is
        // below, and the strip beside the rail came out solid `bg_main`.
        let mut list = DisplayList::new(crate::render::display_list::Color::TRANSPARENT);
        sidebar.paint(ctx, &mut list, surface_rect, input);

        let renderer = self
            .renderer
            .as_mut()
            .ok_or_else(|| "renderer missing".to_string())?;
        renderer.submit(gpu, &list, ctx.text)?;
        self.mapped = true;

        // Tell the compositor we are done with this surface state.
        self.layer
            .wl_surface()
            .damage_buffer(0, 0, self.physical.0 as i32, self.physical.1 as i32);
        self.layer.commit();
        Ok(())
    }
}

/// Anchor and margins (top, right, bottom, left) for a popup beside the rail.
///
/// The popup is anchored to the rail's own screen edge with an exclusive zone
/// of 0, so the compositor places it past every exclusive zone on that edge:
/// ours, and any other panel's. Measuring from the screen edge with the rail's
/// width put the popup on top of the rail whenever another panel came first,
/// as a second shell's does. `gap` separates it from the
/// rail; `along` is its offset along the rail.
pub fn beside_rail(
    position: crate::config::SidebarPosition,
    along: i32,
    gap: i32,
) -> (smithay_client_toolkit::shell::wlr_layer::Anchor, (i32, i32, i32, i32)) {
    use crate::config::SidebarPosition;
    use smithay_client_toolkit::shell::wlr_layer::Anchor;
    match position {
        SidebarPosition::Left => (Anchor::TOP | Anchor::LEFT, (along, 0, 0, gap)),
        SidebarPosition::Right => (Anchor::TOP | Anchor::RIGHT, (along, gap, 0, 0)),
        SidebarPosition::Top => (Anchor::TOP | Anchor::LEFT, (gap, 0, 0, along)),
        SidebarPosition::Bottom => (Anchor::BOTTOM | Anchor::LEFT, (0, 0, gap, along)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SidebarPosition;
    use smithay_client_toolkit::shell::wlr_layer::Anchor;

    #[test]
    fn popups_hang_off_the_rails_edge() {
        assert_eq!(beside_rail(SidebarPosition::Left, 300, 8), (Anchor::TOP | Anchor::LEFT, (300, 0, 0, 8)));
        assert_eq!(beside_rail(SidebarPosition::Right, 300, 8), (Anchor::TOP | Anchor::RIGHT, (300, 8, 0, 0)));
        assert_eq!(beside_rail(SidebarPosition::Top, 500, 8), (Anchor::TOP | Anchor::LEFT, (8, 0, 0, 500)));
        assert_eq!(beside_rail(SidebarPosition::Bottom, 500, 8), (Anchor::BOTTOM | Anchor::LEFT, (0, 0, 8, 500)));
    }
}
