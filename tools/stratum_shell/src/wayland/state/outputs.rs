//! Surfaces per output, and the compositor's output and layer events.

use super::*;

impl App {
    pub(super) fn create_surfaces(&mut self, qh: &QueueHandle<Self>) {
        let outputs: Vec<_> = self.output_state.outputs().collect();
        info!("found {} output(s)", outputs.len());

        // Output info (name, scale) arrives asynchronously. Creating a surface
        // before it is known would key the surface to an empty output name,
        // which silently breaks per-monitor state such as workspaces. Outputs
        // still missing info are created later from `OutputHandler`.
        let mut deferred = 0;
        for output in outputs {
            if !self.create_output_surfaces(qh, &output) {
                deferred += 1;
            }
        }
        if deferred > 0 {
            debug!("{deferred} output(s) awaiting info; surfaces will be created on update");
        }
    }

    /// Create the bar, OSD, and toast surfaces for `output`.
    ///
    /// Returns false when the output's information has not arrived yet, in
    /// which case the caller should retry when it does.
    fn create_output_surfaces(
        &mut self,
        qh: &QueueHandle<Self>,
        output: &wl_output::WlOutput,
    ) -> bool {
        // Already handled?
        if self.surfaces.iter().any(|entry| entry.output.as_ref() == Some(output)) {
            return true;
        }

        let Some(info) = self.output_state.info(output) else {
            return false;
        };
        // Without a name there is nothing to key per-output state by. The
        // model is a poor fallback but better than an empty string.
        let output_name = info
            .name
            .clone()
            .or_else(|| (!info.model.is_empty()).then(|| info.model.clone()))
            .unwrap_or_default();
        if output_name.is_empty() {
            return false;
        }
        // SCTK reports an integer scale. For fractional outputs (Hyprland
        // reports 1.6) an integer buffer scale of 2 is correct for sharpness;
        // the compositor downsamples the buffer to the logical size.
        let scale = info.scale_factor.max(1);
        let output_logical_size = info
            .logical_size
            .map(|(width, height)| (width as f32, height as f32))
            .unwrap_or((0.0, 0.0));
        info!("output '{}' (model '{}', scale {})", output_name, info.model, scale);

        {
            let wl_surface = self.compositor_state.create_surface(qh);
            let layer = self.layer_shell.create_layer_surface(
                qh,
                wl_surface,
                Layer::Top,
                Some("stratum-sidebar"),
                Some(output),
            );

            // Placement and width come from the configuration. A vertical rail
            // with inverted corners is wider than its exclusive zone by the
            // corner size, as QML's window was: the corners sit over the edge
            // of the windows beside it.
            let config = &self.config.sidebar;
            let corner = if config.is_vertical() && config.inverted_corners {
                self.config.metrics.bar.corner_size.max(0.0).ceil() as u32
            } else {
                0
            };
            let (anchor, size) = match config.position {
                SidebarPosition::Left => (
                    Anchor::TOP | Anchor::LEFT | Anchor::BOTTOM,
                    (config.width + corner, 0),
                ),
                SidebarPosition::Right => (
                    Anchor::TOP | Anchor::RIGHT | Anchor::BOTTOM,
                    (config.width + corner, 0),
                ),
                SidebarPosition::Top => (
                    Anchor::TOP | Anchor::LEFT | Anchor::RIGHT,
                    (0, config.height),
                ),
                SidebarPosition::Bottom => (
                    Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
                    (0, config.height),
                ),
            };
            layer.set_anchor(anchor);
            layer.set_exclusive_zone(match config.position {
                SidebarPosition::Top | SidebarPosition::Bottom => config.height as i32,
                _ => config.width as i32,
            });
            layer.set_margin(0, 0, 0, 0);
            layer.set_keyboard_interactivity(KeyboardInteractivity::None);
            layer.set_size(size.0, size.1);
            layer.commit();

            let rail = (corner > 0).then_some(match config.position {
                SidebarPosition::Right => (corner as f32, config.width as f32),
                _ => (0.0, config.width as f32),
            });
            self.surfaces.push(
                SurfaceEntry::new(layer, scale, output_name.clone()).with_output(output.clone())
                    .with_output_logical_size(output_logical_size)
                    .with_rail(rail),
            );

            // OSD: an overlay-layer surface anchored bottom-centre with no
            // exclusive zone, so it floats above windows without reserving
            // space. It takes no input.
            let osd_wl = self.compositor_state.create_surface(qh);
            let osd_layer = self.layer_shell.create_layer_surface(
                qh,
                osd_wl,
                Layer::Overlay,
                Some("stratum-osd"),
                Some(output),
            );
            osd_layer.set_anchor(Anchor::BOTTOM);
            osd_layer.set_exclusive_zone(-1);
            osd_layer.set_keyboard_interactivity(KeyboardInteractivity::None);
            let osd = &self.config.metrics.osd;
            osd_layer.set_margin(0, 0, osd.margin_bottom, 0);
            osd_layer.set_size(osd.width, osd.height);
            osd_layer.commit();
            self.osd_surfaces.push(
                SurfaceEntry::new(osd_layer, scale, output_name.clone()).with_output(output.clone())
                    .with_output_logical_size(output_logical_size),
            );

            // Toast stack: overlay layer, top-right, full height, no exclusive
            // zone. It ignores input so clicks reach the window underneath.
            let toast_wl = self.compositor_state.create_surface(qh);
            let toast_layer = self.layer_shell.create_layer_surface(
                qh,
                toast_wl,
                Layer::Overlay,
                Some("stratum-toasts"),
                Some(output),
            );
            // A zero height requires anchoring both top and bottom; the toast
            // column then sizes itself between the margins.
            toast_layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::RIGHT);
            toast_layer.set_exclusive_zone(-1);
            toast_layer.set_keyboard_interactivity(KeyboardInteractivity::None);
            let toasts = &self.config.metrics.toasts;
            toast_layer.set_margin(toasts.margin, toasts.margin, toasts.margin, toasts.margin);
            toast_layer.set_size(toasts.surface_width, 0);
            toast_layer.commit();
            self.toast_surfaces.push(
                SurfaceEntry::new(toast_layer, scale, output_name.clone()).with_output(output.clone())
                    .with_output_logical_size(output_logical_size),
            );

            // Fullscreen overlays, each mapped only on the output it opened
            // on. They are created with keyboard interactivity `None` and
            // switched to `Exclusive` while open, so they cannot steal keys
            // from a window when hidden. An exclusive zone of -1 puts them
            // over the bar's, so a screenshot can capture the bar too.
            let fullscreen = |namespace: &str| {
                let layer = self.layer_shell.create_layer_surface(
                    qh,
                    self.compositor_state.create_surface(qh),
                    Layer::Overlay,
                    Some(namespace),
                    Some(output),
                );
                layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
                layer.set_exclusive_zone(-1);
                layer.set_keyboard_interactivity(KeyboardInteractivity::None);
                layer.commit();
                SurfaceEntry::new(layer, scale, output_name.clone())
                    .with_output(output.clone())
                    .with_output_logical_size(output_logical_size)
            };
            let power = fullscreen("stratum-powermenu");
            let polkit = fullscreen("stratum-polkit");
            let themes = fullscreen("stratum-themes");
            let dashboard = fullscreen("stratum-dashboard");
            let screenshot = fullscreen("stratum-screenshot");
            self.power_surfaces.push(power);
            self.polkit_surfaces.push(polkit);
            self.theme_surfaces.push(themes);
            self.dashboard_surfaces.push(dashboard);
            self.screenshot_surfaces.push(screenshot);

            // Hover menu: sized to its contents and moved beside the icon with
            // margins, like the tray menu. A size is set before the first
            // commit so it cannot come up output-sized and swallow clicks.
            let hover_wl = self.compositor_state.create_surface(qh);
            let hover_layer = self.layer_shell.create_layer_surface(
                qh,
                hover_wl,
                Layer::Overlay,
                Some("stratum-hovermenu"),
                Some(output),
            );
            hover_layer.set_anchor(Anchor::TOP | Anchor::LEFT);
            // 0, not -1: placed past the rail's exclusive zone, and any other
            // panel's on that edge (`surfaces::beside_rail`).
            hover_layer.set_exclusive_zone(0);
            hover_layer.set_keyboard_interactivity(KeyboardInteractivity::None);
            let hover_metrics = &self.config.metrics.hover_menu;
            hover_layer.set_size(hover_metrics.width as u32, hover_metrics.row_height as u32);
            hover_layer.commit();
            self.hover_surfaces.push(
                SurfaceEntry::new(hover_layer, scale, output_name.clone())
                    .with_output(output.clone())
                    .with_output_logical_size(output_logical_size),
            );

            // Tray context menu: a small overlay beside the rail. It is sized
            // to its contents, so it starts hidden and is mapped only while a
            // menu is open; anchor and margins are set on open
            // (`surfaces::beside_rail`).
            let menu_wl = self.compositor_state.create_surface(qh);
            let menu_layer = self.layer_shell.create_layer_surface(
                qh,
                menu_wl,
                Layer::Overlay,
                Some("stratum-traymenu"),
                Some(output),
            );
            menu_layer.set_anchor(Anchor::TOP | Anchor::LEFT);
            menu_layer.set_exclusive_zone(0);
            menu_layer.set_keyboard_interactivity(KeyboardInteractivity::None);
            // A size must be set before the first commit: anchored top-left
            // without one, the compositor chooses a surface as large as the
            // output, and once mapped that would swallow clicks across the
            // whole screen. It is replaced with the menu's real size on open.
            let menu_metrics = &self.config.metrics.tray_menu;
            menu_layer.set_size(menu_metrics.width as u32, menu_metrics.row_height as u32);
            menu_layer.commit();
            self.tray_menu_surfaces.push(
                SurfaceEntry::new(menu_layer, scale, output_name.clone())
                    .with_output(output.clone())
                    .with_output_logical_size(output_logical_size),
            );

            debug!("created layer surface on output '{}' (scale {scale}x)", output_name);
        }
        true
    }
}

impl CompositorHandler for App {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        new_factor: i32,
    ) {
        if let Some(entry) = self
            .surfaces
            .iter_mut()
            .find(|e| e.layer.wl_surface() == surface)
        {
            entry.scale = new_factor.max(1);
            entry.dirty = true;
        }
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        // M1 has no animation; a completed frame simply allows the next redraw.
        if let Some(entry) = self
            .surfaces
            .iter_mut()
            .find(|e| e.layer.wl_surface() == surface)
        {
            entry.frame_pending = false;
        }
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        // The output's name and scale arrive in a later event, so this often
        // fails on the first try. `update_output` retries once the info is
        // complete.
        if !self.create_output_surfaces(qh, &output) {
            debug!("new output awaiting info; deferring surface creation");
        }
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        // Second chance for an output that was created without complete info.
        self.create_output_surfaces(qh, &output);
        if self.lock.active() {
            self.create_lock_surfaces(qh);
        }
        // `logical_size` often arrives after the surface was created, so keep
        // every surface on this output current. Popup placement depends on it.
        let size = self
            .output_state
            .info(&output)
            .and_then(|info| info.logical_size)
            .map(|(width, height)| (width as f32, height as f32));
        if let Some(size) = size {
            for entry in self
                .surfaces
                .iter_mut()
                .chain(self.osd_surfaces.iter_mut())
                .chain(self.toast_surfaces.iter_mut())
                .chain(self.power_surfaces.iter_mut())
                .chain(self.polkit_surfaces.iter_mut())
                .chain(self.theme_surfaces.iter_mut())
                .chain(self.dashboard_surfaces.iter_mut())
                .chain(self.screenshot_surfaces.iter_mut())
                .chain(self.hover_surfaces.iter_mut())
                .chain(self.tray_menu_surfaces.iter_mut())
            {
                if entry.output.as_ref() == Some(&output) {
                    entry.output_logical_size = size;
                }
            }
        }
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
}

impl LayerShellHandler for App {
    fn closed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        layer: &LayerSurface,
    ) {
        self.toast_surfaces.retain(|entry| &entry.layer != layer);
        self.osd_surfaces.retain(|entry| &entry.layer != layer);
        self.power_surfaces.retain(|entry| &entry.layer != layer);
        self.polkit_surfaces.retain(|entry| &entry.layer != layer);
        self.theme_surfaces.retain(|entry| &entry.layer != layer);
        self.dashboard_surfaces.retain(|entry| &entry.layer != layer);
        self.screenshot_surfaces.retain(|entry| &entry.layer != layer);
        self.hover_surfaces.retain(|entry| &entry.layer != layer);
        self.tray_menu_surfaces.retain(|entry| &entry.layer != layer);
        self.surfaces.retain(|entry| &entry.layer != layer);
        warn!("layer surface closed by compositor");
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        let (width, height) = configure.new_size;
        debug!("layer configure {width}x{height}");

        if let Some(entry) = self
            .power_surfaces
            .iter_mut()
            .chain(self.polkit_surfaces.iter_mut())
            .chain(self.theme_surfaces.iter_mut())
            .chain(self.dashboard_surfaces.iter_mut())
            .chain(self.screenshot_surfaces.iter_mut())
            .find(|e| &e.layer == layer)
        {
            // Fullscreen: the compositor's configure is authoritative.
            let (width, height) = configure.new_size;
            entry.resize(
                &self.gpu,
                if width == 0 { 1920 } else { width },
                if height == 0 { 1080 } else { height },
            );
            entry.dirty = true;
            return;
        }

        if let Some(entry) = self
            .tray_menu_surfaces
            .iter_mut()
            .chain(self.hover_surfaces.iter_mut())
            .find(|e| &e.layer == layer)
        {
            // The compositor may grant a different size than requested; keep
            // whatever it gave so the panel is drawn to fit.
            let (width, height) = configure.new_size;
            entry.resize(
                &self.gpu,
                width.max(1),
                height.max(1),
            );
            entry.dirty = true;
            return;
        }

        if let Some(entry) = self.toast_surfaces.iter_mut().find(|e| &e.layer == layer) {
            let (width, height) = configure.new_size;
            entry.resize(
                &self.gpu,
                if width == 0 { 344 } else { width },
                if height == 0 { 480 } else { height },
            );
            entry.dirty = true;
            return;
        }

        if let Some(entry) = self.osd_surfaces.iter_mut().find(|e| &e.layer == layer) {
            let (width, height) = configure.new_size;
            entry.resize(
                &self.gpu,
                if width == 0 { 320 } else { width },
                if height == 0 { 94 } else { height },
            );
            entry.dirty = true;
            return;
        }

        if let Some(entry) = self.surfaces.iter_mut().find(|e| &e.layer == layer) {
            // A zero on an anchored axis means "you choose"; the rail always
            // takes the height the compositor gave us.
            // A zero on an axis means "you choose". Vertical rails choose their
            // width and take the compositor's height; horizontal bars do the
            // reverse.
            let config = &self.config.sidebar;
            let (width, height) = match config.axis() {
                crate::config::Axis::Vertical => (
                    if width == 0 { config.width.max(1) } else { width },
                    height,
                ),
                crate::config::Axis::Horizontal => (
                    width,
                    if height == 0 { config.height.max(1) } else { height },
                ),
            };
            entry.resize(&self.gpu, width, height);
            entry.dirty = true;
        }
    }
}
