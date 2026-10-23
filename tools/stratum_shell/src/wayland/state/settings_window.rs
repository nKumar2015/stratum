//! The control centre's toplevel window.

use super::*;

/// The control centre as a real toplevel window.
///
/// Using xdg-shell rather than a layer surface means the compositor owns the
/// window: it gets normal focus, workspace placement, alt-tab, and stacking,
/// so it feels like part of the shell rather than an overlay.
pub(super) struct SettingsWindow {
    /// Declared before `window` so the swapchain drops before the `wl_surface`
    /// it references. See `SurfaceEntry` for why the order matters.
    renderer: Option<crate::render::SurfaceRenderer>,
    pub(super) window: Window,
    scale: i32,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) configured: bool,
    pub(super) dirty: bool,
}

impl SettingsWindow {
    fn new(window: Window, scale: i32, metrics: &crate::ui::style::WindowMetrics) -> Self {
        Self {
            renderer: None,
            window,
            scale,
            width: metrics.width,
            height: metrics.height,
            configured: false,
            dirty: true,
        }
    }

    fn resize(&mut self, gpu: &Gpu, width: u32, height: u32) {
        self.width = width.max(1);
        self.height = height.max(1);
        self.configured = true;
        // Buffer scale applies to toplevels too. Without this the window is
        // rendered at 1x and upscaled by the compositor, which is what made it
        // look blurry next to the sharp layer surfaces.
        let scale = self.scale.max(1) as u32;
        self.window.wl_surface().set_buffer_scale(scale as i32);
        let physical = (self.width * scale, self.height * scale);
        if let Some(renderer) = &mut self.renderer {
            renderer.resize(gpu, physical, self.scale as f32);
        }
    }

    pub(super) fn render(
        &mut self,
        gpu: &Gpu,
        display: NonNull<std::ffi::c_void>,
        ctx: &mut crate::ui::PaintCtx<'_>,
        control_center: &mut crate::ui::control_center::ControlCenter,
        input: &crate::ui::InputState,
    ) -> Result<crate::render::display_list::Rect, String> {
        if self.renderer.is_none() {
            let surface = self.window.wl_surface();
            let scale = self.scale.max(1) as u32;
            self.renderer = Some(crate::render::SurfaceRenderer::new(
                gpu,
                display,
                surface,
                (self.width * scale, self.height * scale),
                self.scale as f32,
            )?);
        }

        let mut list = crate::render::display_list::DisplayList::new(
            ctx.theme.bg_main,
        );
        let surface_rect = crate::render::display_list::Rect {
            x: 0.0,
            y: 0.0,
            width: self.width as f32,
            height: self.height as f32,
            radius: 0.0,
            color: ctx.theme.bg_main,
        };
        let panel = control_center.paint_window(ctx, &mut list, surface_rect, input);

        let renderer = self.renderer.as_mut().ok_or("renderer missing")?;
        renderer.submit(gpu, &list, ctx.text)?;
        let scale = self.scale.max(1);
        self.window.wl_surface().damage_buffer(
            0,
            0,
            (self.width * scale as u32) as i32,
            (self.height * scale as u32) as i32,
        );
        self.window.commit();
        Ok(panel)
    }
}

impl App {
    /// Create the settings window the first time the control centre opens.
    pub(super) fn ensure_settings_window(&mut self) {
        if self.settings.is_some() {
            return;
        }
        let qh = self.queue_handle.clone();
        let surface = self.compositor_state.create_surface(&qh);
        let window = self
            .xdg_shell
            .create_window(surface, WindowDecorations::RequestServer, &qh);
        window.set_title("Stratum Settings");
        window.set_app_id("org.stratum.Settings");
        let win = &self.config.metrics.window;
        window.set_min_size(Some((win.min_width, win.min_height)));
        // Initial commit with no buffer; the compositor answers with a configure.
        window.commit();

        // Ask for focus so the window comes to the front when opened.
        if let Some(activation) = &self.activation {
            activation.request_token(
                &qh,
                RequestData {
                    seat_and_serial: None,
                    surface: Some(window.wl_surface().clone()),
                    app_id: Some("org.stratum.Settings".to_string()),
                    udata: (),
                },
            );
        }

        // Match the scale of an existing bar surface so the window is as sharp
        // as the rest of the shell on this output.
        let scale = self.surfaces.iter().map(|s| s.scale).max().unwrap_or(1);
        let metrics = self.config.metrics.window.clone();
        self.settings = Some(SettingsWindow::new(window, scale, &metrics));
    }

    pub(super) fn apply_action(&mut self, action: crate::ui::Action) {
        use crate::ui::Action;
        match action {
            Action::OpenPanel(page) => self.control_center.toggle(page),
            Action::ActivateTray(service) => {
                if let Some(item) = self
                    .state
                    .tray
                    .get()
                    .into_iter()
                    .find(|item| item.service == service)
                {
                    self.actions.push(crate::actions::SystemAction::TrayActivate {
                        service: item.service,
                        path: item.path,
                    });
                }
            }
            Action::SecondaryActivateTray(service) => {
                if let Some(item) = self
                    .state
                    .tray
                    .get()
                    .into_iter()
                    .find(|item| item.service == service)
                {
                    self.actions.push(
                        crate::actions::SystemAction::TraySecondaryActivate {
                            service: item.service,
                            path: item.path,
                        },
                    );
                }
            }
            Action::OpenTrayMenu { service, path, y } => {
                // Pin the menu to the output whose bar was clicked, then fetch
                // its contents off-thread. The overlay appears when the menu
                // publishes.
                let clicked_surface = self.pointer.as_ref().map(|(surface, _, _)| surface.clone());
                let bar = clicked_surface.as_ref().and_then(|surface| {
                    self.surfaces
                        .iter()
                        .find(|entry| entry.layer.wl_surface() == surface)
                });
                let output = bar
                    .map(|entry| entry.output_name.clone())
                    .unwrap_or_else(|| self.focused_output());
                self.tray_menu.open(&output, crate::tray_menu::Placement { y });
                self.tray_menu_geometry = None;
                self.actions.push(crate::actions::SystemAction::TrayMenuOpen {
                    service,
                    path,
                });
                for entry in &mut self.tray_menu_surfaces {
                    entry.dirty = true;
                }
            }
            Action::FocusWorkspace(id) => {
                // Dispatch to the compositor. The widget has already marked the
                // workspace active locally, so the bar updates on this frame;
                // the Hyprland event confirms it a moment later.
                if !crate::hyprland::focus_workspace(id) {
                    tracing::warn!("failed to focus workspace {id}");
                }
            }
            Action::OpenDashboard(output) => {
                if !self.dashboard.is_open() {
                    self.dashboard.open_on(&output, None);
                    self.mark_dashboard_dirty();
                }
            }
        }
        for entry in &mut self.surfaces {
            entry.dirty = true;
        }
    }
}

impl WindowHandler for App {
    fn request_close(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        window: &Window,
    ) {
        if self.viewer_window.as_ref().is_some_and(|w| &w.window == window) {
            self.close_viewer();
            return;
        }
        // Closing the settings window closes the control centre; the window
        // itself is kept for reuse.
        self.control_center.close();
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        window: &Window,
        configure: WindowConfigure,
        _serial: u32,
    ) {
        if let Some(viewer) = self.viewer_window.as_mut().filter(|w| &w.window == window) {
            let m = &self.config.metrics.screenshot_viewer;
            let width = configure.new_size.0.map(|v| v.get()).unwrap_or(m.width);
            let height = configure.new_size.1.map(|v| v.get()).unwrap_or(m.height);
            viewer.resize(&self.gpu, width, height);
            return;
        }
        let metrics = &self.config.metrics.window;
        let width = configure
            .new_size
            .0
            .map(|v| v.get())
            .unwrap_or(metrics.width);
        let height = configure
            .new_size
            .1
            .map(|v| v.get())
            .unwrap_or(metrics.height);
        if let Some(settings) = &mut self.settings {
            if settings.window.wl_surface() == window.wl_surface() {
                settings.resize(&self.gpu, width, height);
                settings.dirty = true;
            }
        }
    }
}
