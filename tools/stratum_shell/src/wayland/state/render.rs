//! Painting every surface that needs it, once per loop iteration.

use super::*;

use crate::config::Theme;

impl App {
    /// Redraw every dirty surface. Called after each loop iteration.
    pub fn render_pending(&mut self) -> Result<(), String> {
        self.maybe_reload_config();
        // Polled managers update state asynchronously; when anything changed,
        // redraw every surface even without pointer input.
        let state_changed = self.state.take_dirty();
        if state_changed {
            for entry in &mut self.surfaces {
                entry.dirty = true;
            }
        }
        // Advance the OSD and pick up value changes. Volume is pushed state;
        // brightness has no push source, so it is polled on a slow cadence.
        let now = std::time::Instant::now();
        let dt = now
            .duration_since(self.last_frame)
            .as_secs_f32()
            .min(0.1);
        self.last_frame = now;

        // First, so a failure on any other surface cannot keep it from drawing.
        self.render_lock(dt);
        // While locked, nothing else is shown, and the compositor stops
        // releasing the buffers of hidden surfaces: presenting to one blocks
        // in the swapchain acquire and freezes the whole loop, lock screen
        // included. Everything is repainted on unlock.
        if self.lock.phase() == crate::lock::Phase::Locked {
            return Ok(());
        }

        // Widget animations (the workspace strip's scroll, the clock's minute
        // hand) run on the frame clock. While anything is moving every surface
        // stays dirty, so the next loop iteration paints again.
        let animating = self.sidebar.tick(dt);
        if animating {
            for entry in &mut self.surfaces {
                entry.dirty = true;
            }
        }

        self.update_osd_and_toasts(state_changed, dt);
        self.render_settings_window(dt);

        let theme = self.config.active_theme();
        self.render_osd(&theme, dt)?;
        self.render_toasts(&theme, dt)?;
        self.render_power_menu(&theme, dt)?;
        self.render_polkit_dialog(&theme, dt, state_changed)?;
        self.render_hover_menu(&theme, dt, state_changed)?;
        self.render_dashboard(&theme, dt, state_changed)?;
        self.render_screenshot(&theme, dt)?;
        self.render_viewer(&theme, dt);
        self.render_theme_switcher(&theme, dt)?;
        self.render_tray_menu(&theme, dt)?;
        self.render_bars(&theme, dt)
    }

    /// Feed the OSD and the toasts what changed, and mark their surfaces for
    /// the frames their animations need.
    fn update_osd_and_toasts(&mut self, state_changed: bool, dt: f32) {
        self.osd.set_metrics(&self.config.metrics.osd);
        self.toasts.set_metrics(&self.config.metrics.toasts);
        self.osd.observe_audio(&self.state);
        if let Some(percent) = self.state.brightness.get() {
            self.osd.observe_brightness(percent);
        }

        // Toasts follow the notification list and expire on their own timers.
        // `take_dirty` already consumed the aggregate flag, so sync on any
        // state change rather than on the notification cell alone. While
        // do-not-disturb is on, existing cards still update and disappear with
        // their notifications, but no new card appears.
        if state_changed {
            self.toasts.set_muted(self.state.dnd.get());
            let list = self.state.notifications.get();
            self.toasts.sync(&list);
            // Save for the next run whenever the list or DND changed.
            if let (Some(saver), Some(server)) = (&self.notification_saver, &self.notifications) {
                let snapshot = crate::notification_store::Snapshot {
                    do_not_disturb: self.state.dnd.get(),
                    next_id: server.next_id(),
                    notifications: list,
                };
                if self.saved_notifications.as_ref() != Some(&snapshot) {
                    saver.save(snapshot.clone());
                    self.saved_notifications = Some(snapshot);
                }
            }
        }
        // A toast that times out only hides the card. The notification stays in
        // the centre until it is dismissed or cleared — expiry is not dismissal,
        // and closing it on the server here would make notifications vanish from
        // the page a few seconds after arriving.
        self.toasts.tick(dt);
        if self.toasts.animating() || !self.toasts.is_empty() {
            for entry in &mut self.toast_surfaces {
                entry.dirty = true;
            }
        }
        if self.osd.tick(dt) {
            for entry in &mut self.osd_surfaces {
                entry.dirty = true;
            }
        }
        if self.osd.visible() {
            for entry in &mut self.osd_surfaces {
                entry.dirty = true;
            }
        }
    }

    /// The control centre's window: created on demand, painted while open,
    /// destroyed once closed, and the actions its pages queued dispatched.
    fn render_settings_window(&mut self, dt: f32) {
        // The settings window is an ordinary toplevel: create it on demand and
        // redraw while it is open.
        if self.control_center.open {
            self.ensure_settings_window();
        }
        // The open/close animation needs frames of its own, independent of
        // state changes.
        let cc_animating = self.control_center.tick(dt);
        if cc_animating {
            if let Some(settings) = &mut self.settings {
                settings.dirty = true;
            }
        }

        // Once the close animation has finished, destroy the toplevel. Dropping
        // the window unmaps it; leaving it alive would keep the last frame on
        // screen forever, since nothing repaints a hidden surface.
        if !self.control_center.visible() {
            if let Some(window) = self.settings.take() {
                debug!("closing settings window");
                drop(window);
                crate::memory::release_freed();
            }
        }

        if self.control_center.visible() {
            let theme = self.config.active_theme();
            if let Some(mut window) = self.settings.take() {
                if window.configured {
                    // The window can be on any output; show the one it is on.
                    let output_name = self
                        .surfaces
                        .first()
                        .map(|s| s.output_name.clone())
                        .unwrap_or_default();
                    let input = crate::ui::InputState {
                        pointer: self
                            .pointer
                            .as_ref()
                            .filter(|(surface, _, _)| surface == window.window.wl_surface())
                            .map(|(_, x, y)| (*x, *y)),
                        // A press released before this paint still counts once.
                        pressed: self.pointer_pressed || std::mem::take(&mut self.settings_press),
                    };
                    let mut ctx = paint_ctx!(self, &theme, &output_name, dt);
                    if let Err(err) = window.render(
                        &self.gpu,
                        self.display,
                        &mut ctx,
                        &mut self.control_center,
                        &input,
                    ) {
                        tracing::error!("settings window render failed: {err}");
                    }
                }
                self.settings = Some(window);
            }
            // Dispatch whatever the panel asked for. The worker executes them
            // off-thread and their effects arrive as normal state updates.
            for action in self.control_center.take_actions() {
                match action {
                    crate::actions::SystemAction::Dismiss(id) => {
                        if let Some(server) = &self.notifications {
                            server.close(id, 2);
                        }
                    }
                    crate::actions::SystemAction::DismissAll => self.dismiss_all_notifications(),
                    crate::actions::SystemAction::InvokeNotification(id, key) => {
                        if let Some(server) = &self.notifications {
                            server.invoke(id, &key);
                        }
                    }
                    crate::actions::SystemAction::SetTheme(name) => {
                        // Theme is shell-owned state, not a system action.
                        if self.switch_theme(&name) {
                            self.mark_all_dirty();
                        }
                    }
                    other => self.actions.push(other),
                }
            }
        }
    }

    /// The OSD. It shares the sidebar's paint context but draws its own
    /// display list, since it has nothing to do with the rail layout.
    fn render_osd(&mut self, theme: &Theme, dt: f32) -> Result<(), String> {
        // Once it has faded out, detach the buffer: a mapped surface with a
        // transparent buffer can still composite as black on some compositors,
        // which is the "OSD stays black" report.
        let osd_visible = self.osd.visible();
        for entry in &mut self.osd_surfaces {
            if !osd_visible {
                entry.unmap();
                continue;
            }
            if !entry.ready() || !entry.dirty {
                continue;
            }
            let output_name = entry.output_name.clone();
            let mut ctx = paint_ctx!(self, theme, &output_name, dt);
            let rect = entry.logical_rect();
            let mut list = crate::render::display_list::DisplayList::new(Color::TRANSPARENT);
            self.osd.paint(&mut ctx, &mut list, rect);
            entry.render_list(&self.gpu, self.display, &list, &mut self.text)?;
            entry.dirty = false;
        }
        Ok(())
    }

    /// The toast stack. The stack keeps its card layouts for clicks and hover.
    /// An empty stack unmaps, for the same reason as the OSD. The surface
    /// takes keyboard focus only while a reply field on it has focus.
    fn render_toasts(&mut self, theme: &Theme, dt: f32) -> Result<(), String> {
        let toasts_visible = !self.toasts.is_empty();
        let toast_keyboard = self.toasts.keyboard_output().map(str::to_string);
        for entry in &mut self.toast_surfaces {
            if !toasts_visible {
                entry.unmap();
                continue;
            }
            if !entry.ready() {
                continue;
            }
            if entry.dirty {
                let output_name = entry.output_name.clone();
                let mut ctx = paint_ctx!(self, theme, &output_name, dt);
                let rect = entry.logical_rect();
                let mut list = crate::render::display_list::DisplayList::new(Color::TRANSPARENT);
                let bounds = self.toasts.paint(&mut ctx, &mut list, rect);

                // The surface spans the full screen height so the stack can
                // grow, but only the cards themselves should take clicks.
                // Everything else must fall through to the window underneath.
                // This has to be set before the commit that `render_list`
                // performs, or the region only takes effect one frame late.
                entry.set_input_region(&self.compositor_state, bounds);
                entry.render_list(&self.gpu, self.display, &list, &mut self.text)?;
                entry.dirty = false;
            }
            // As with the power menu, focus is requested only once a buffer is
            // attached.
            let wants_keys = toast_keyboard.as_deref() == Some(entry.output_name.as_str());
            if entry.mapped {
                entry.set_keyboard_exclusive(wants_keys);
            }
        }
        Ok(())
    }

    /// The power menu. Only the output it opened on is painted and given
    /// keyboard focus; the rest stay unmapped and inert. Repaints happen on
    /// the state changes that matter (open/close, selection change), not
    /// every frame: forcing a redraw here made the labels shimmer.
    fn render_power_menu(&mut self, theme: &Theme, dt: f32) -> Result<(), String> {
        self.power_menu.set_metrics(&self.config.metrics.power_menu);
        let power_target = self.power_menu.target().map(str::to_string);
        for entry in &mut self.power_surfaces {
            let active = power_target
                .as_deref()
                .map(|name| name == entry.output_name)
                .unwrap_or(false);
            if !active {
                // Unmapping also drops keyboard focus, so keys return to the
                // window.
                entry.unmap();
                continue;
            }
            if !entry.ready() {
                continue;
            }
            if entry.dirty {
                tracing::debug!(
                    "power menu paint output={} selection={}",
                    entry.output_name,
                    self.power_menu.selection()
                );
                let output_name = entry.output_name.clone();
                let mut ctx = paint_ctx!(self, theme, &output_name, dt);
                let rect = entry.logical_rect();
                let mut list = crate::render::display_list::DisplayList::new(Color::TRANSPARENT);
                self.power_menu.paint(&mut ctx, &mut list, rect);
                entry.render_list(&self.gpu, self.display, &list, &mut self.text)?;
                entry.dirty = false;
            }
            // Request focus only once the surface has a buffer attached. Asking
            // before the first commit after a re-map is why keys stopped
            // arriving the second time the menu opened.
            if entry.mapped {
                entry.set_keyboard_exclusive(true);
            }
        }
        Ok(())
    }

    /// The polkit dialog: follows the agent's published request, on the output
    /// that was focused when the request arrived. Same mapping and focus
    /// rules as the power menu.
    fn render_polkit_dialog(&mut self, theme: &Theme, dt: f32, state_changed: bool) -> Result<(), String> {
        if state_changed {
            let focused = self.focused_output();
            let payload = self.state.polkit.get();
            if self.polkit_dialog.sync(&payload, &focused) {
                self.mark_polkit_dirty();
            }
        }
        self.polkit_dialog.set_metrics(&self.config.metrics.polkit);
        let polkit_target = self.polkit_dialog.target().map(str::to_string);
        for entry in &mut self.polkit_surfaces {
            let active = polkit_target.as_deref() == Some(entry.output_name.as_str());
            if !active {
                entry.unmap();
                continue;
            }
            if !entry.ready() {
                continue;
            }
            if entry.dirty {
                let output_name = entry.output_name.clone();
                let mut ctx = paint_ctx!(self, theme, &output_name, dt);
                let rect = entry.logical_rect();
                let mut list = crate::render::display_list::DisplayList::new(Color::TRANSPARENT);
                self.polkit_dialog.paint(&mut ctx, &mut list, rect);
                entry.render_list(&self.gpu, self.display, &list, &mut self.text)?;
                entry.dirty = false;
            }
            if entry.mapped {
                entry.set_keyboard_exclusive(true);
            }
        }
        Ok(())
    }

    /// The hover menu. Painted once per change; the painted height sizes the
    /// surface, and a size change waits for the configure before drawing.
    fn render_hover_menu(&mut self, theme: &Theme, dt: f32, state_changed: bool) -> Result<(), String> {
        if self.hover_menus.intent.pending() {
            self.update_hover();
        }
        if self.tray_menu_left_at.is_some() {
            self.update_tray_menu_hover();
        }
        if state_changed {
            self.mark_hover_dirty();
        }
        self.hover_menus.tick(dt);
        let open_hover = self.hover_menus.intent.open().cloned();
        let mut hover_events = Vec::new();
        for i in 0..self.hover_surfaces.len() {
            let entry = &self.hover_surfaces[i];
            let Some(menu) = open_hover.as_ref().filter(|o| o.output == entry.output_name) else {
                self.hover_surfaces[i].unmap();
                continue;
            };
            let applied = self.hover_geometry.as_ref().filter(|g| g.2 == entry.output_name).cloned();
            if applied.is_some() && (!entry.dirty || !self.hover_surfaces[i].ready()) {
                continue;
            }
            let entry = &self.hover_surfaces[i];
            let input = crate::ui::InputState {
                pointer: self
                    .pointer
                    .as_ref()
                    .filter(|(surface, _, _)| surface == entry.layer.wl_surface())
                    .map(|(_, x, y)| (*x, *y)),
                pressed: self.pointer_pressed,
            };
            let output_name = entry.output_name.clone();
            let output_size = entry.output_logical_size;
            let clicked = std::mem::take(&mut self.hover_clicked);
            let mut ctx = paint_ctx!(self, theme, &output_name, dt);
            let mut list = crate::render::display_list::DisplayList::new(Color::TRANSPARENT);
            let (height, events) = self.hover_menus.paint(
                &mut ctx,
                &mut list,
                menu.kind,
                crate::render::display_list::Point::new(0.0, 0.0),
                &input,
                clicked,
            );
            hover_events.extend(events);
            let width = self.config.metrics.hover_menu.width;
            let height = height.ceil().max(1.0);
            let placement = self.hover_placement(menu, (width, height), output_size);
            let geometry = (height as u32, placement, output_name);
            let entry = &mut self.hover_surfaces[i];
            if Some(&geometry) != applied.as_ref() {
                let gap = self.config.metrics.hover_menu.offset.round() as i32;
                let (anchor, (top, right, bottom, left)) =
                    crate::wayland::surfaces::beside_rail(self.config.sidebar.position, placement, gap);
                entry.layer.set_size(width as u32, geometry.0);
                entry.layer.set_anchor(anchor);
                entry.layer.set_margin(top, right, bottom, left);
                entry.layer.commit();
                entry.configured = false;
                entry.awaiting_configure = true;
                entry.dirty = true;
                self.hover_geometry = Some(geometry);
                continue;
            }
            entry.render_list(&self.gpu, self.display, &list, &mut self.text)?;
            entry.dirty = false;
        }
        if open_hover.is_none() {
            self.hover_geometry = None;
        }
        if !hover_events.is_empty() {
            self.apply_hover_events(hover_events);
        }
        Ok(())
    }

    /// The dashboard: its sampler publishes only while it is open, like the
    /// QML watch/unwatch. Its Focus timer runs, and ends, while it is closed.
    fn render_dashboard(&mut self, theme: &Theme, dt: f32, state_changed: bool) -> Result<(), String> {
        self.dashboard.set_config(&self.config.metrics.dashboard, &self.config.dashboard);
        let feed = &self.state.dashboard_feed;
        feed.configure(crate::dashboard::feed::Settings::from_config(&self.config.dashboard));
        feed.set_open(self.dashboard.is_open());
        feed.set_player(self.dashboard.player());
        let now = std::time::Instant::now();
        if let Some(ended) = self.dashboard.tick_timer(now) {
            let (summary, body) = ended.message();
            self.actions.push(crate::actions::SystemAction::Notify {
                app: "Stratum",
                icon: "alarm-symbolic",
                urgency: 1,
                summary,
                body,
            });
        }
        if self.dashboard.is_open() && (state_changed || self.dashboard.animating() || self.dashboard.timer_changed(now)) {
            self.mark_dashboard_dirty();
        }
        let dash_target = self.dashboard.target().map(str::to_string);
        let dash_snapshot = self.state.dashboard.get();
        let dash_music = self.state.music.get();
        for entry in &mut self.dashboard_surfaces {
            if dash_target.as_deref() != Some(entry.output_name.as_str()) {
                entry.unmap();
                continue;
            }
            if !entry.ready() {
                continue;
            }
            if entry.dirty {
                let output_name = entry.output_name.clone();
                let mut ctx = paint_ctx!(self, theme, &output_name, dt);
                let rect = entry.logical_rect();
                let mut list = crate::render::display_list::DisplayList::new(Color::TRANSPARENT);
                self.dashboard.paint(&mut ctx, &mut list, rect, &dash_snapshot, &dash_music);
                entry.render_list(&self.gpu, self.display, &list, &mut self.text)?;
                entry.dirty = false;
            }
            if entry.mapped {
                entry.set_keyboard_exclusive(true);
            }
        }
        Ok(())
    }

    /// The screenshot overlay: every output whose frame is frozen. Only the
    /// output it started on takes the keyboard, for Escape.
    fn render_screenshot(&mut self, theme: &Theme, dt: f32) -> Result<(), String> {
        self.screenshot.set_metrics(&self.config.metrics.screenshot);
        let origins: Vec<(i32, i32)> = self
            .screenshot_surfaces
            .iter()
            .map(|e| self.output_origin(&e.output_name))
            .collect();
        let shot_origin = self.screenshot.origin().to_string();
        for (entry, origin) in self.screenshot_surfaces.iter_mut().zip(origins) {
            if !self.screenshot.shows(&entry.output_name) {
                entry.unmap();
                // A frozen frame is tens of megabytes of GPU memory.
                if let Some(renderer) = &mut entry.renderer {
                    renderer.release_backdrop();
                }
                continue;
            }
            if !entry.ready() {
                continue;
            }
            if entry.dirty {
                let output_name = entry.output_name.clone();
                let mut ctx = paint_ctx!(self, theme, &output_name, dt);
                let rect = entry.logical_rect();
                let mut list = crate::render::display_list::DisplayList::new(Color::TRANSPARENT);
                self.screenshot.paint(&mut ctx, &mut list, rect, &output_name, origin);
                entry.render_list(&self.gpu, self.display, &list, &mut self.text)?;
                entry.dirty = false;
            }
            if entry.mapped {
                entry.set_keyboard_exclusive(entry.output_name == shot_origin);
            }
        }
        Ok(())
    }

    /// The screenshot viewer window. A failure is logged rather than
    /// returned, as for the settings window.
    fn render_viewer(&mut self, theme: &Theme, dt: f32) {
        if self.viewer.is_open() {
            self.viewer.configure(&self.config.metrics.screenshot_viewer, &self.config.screenshot);
            if self.viewer.tick() {
                self.mark_viewer_dirty();
            }
        }
        if let Some(window) = &mut self.viewer_window {
            if window.configured && window.dirty {
                let output_name = self.surfaces.first().map(|s| s.output_name.clone()).unwrap_or_default();
                let mut ctx = paint_ctx!(self, theme, &output_name, dt);
                let mut list = crate::render::display_list::DisplayList::new(theme.bg_main);
                self.viewer.paint(&mut ctx, &mut list, window.logical_rect());
                if let Err(err) = window.render_list(&self.gpu, self.display, &list, &mut self.text) {
                    tracing::error!("screenshot viewer render failed: {err}");
                }
            }
        }
    }

    /// The theme switcher, on the output it opened on.
    fn render_theme_switcher(&mut self, theme: &Theme, dt: f32) -> Result<(), String> {
        self.theme_switcher.set_metrics(&self.config.metrics.theme_switcher);
        let theme_target = self.theme_switcher.target().map(str::to_string);
        let theme_names = self.theme_names();
        let current_theme = self.config.theme.clone();
        for entry in &mut self.theme_surfaces {
            if theme_target.as_deref() != Some(entry.output_name.as_str()) {
                entry.unmap();
                continue;
            }
            if !entry.ready() {
                continue;
            }
            if entry.dirty {
                let output_name = entry.output_name.clone();
                let mut ctx = paint_ctx!(self, theme, &output_name, dt);
                let rect = entry.logical_rect();
                let mut list = crate::render::display_list::DisplayList::new(Color::TRANSPARENT);
                self.theme_switcher
                    .paint(&mut ctx, &mut list, rect, &theme_names, &current_theme);
                entry.render_list(&self.gpu, self.display, &list, &mut self.text)?;
                entry.dirty = false;
            }
            if entry.mapped {
                entry.set_keyboard_exclusive(true);
            }
        }
        Ok(())
    }

    /// The tray context menu. It is sized to its contents, so the layer's size
    /// and margin are set from the published menu whenever that changes; the
    /// surface is unmapped when no menu is open.
    fn render_tray_menu(&mut self, theme: &Theme, dt: f32) -> Result<(), String> {
        self.tray_menu.set_metrics(&self.config.metrics.tray_menu);
        let open_menu = self.state.tray_menu.get();
        let menu_target = self.tray_menu.target().map(str::to_string);
        let label_style = crate::text::TextStyle {
            family: std::sync::Arc::from(theme.font.as_str()),
            size: self.config.metrics.tray_menu.text,
            weight: self.config.metrics.typography.weight_regular,
            color: Color::WHITE,
        };
        let text = &mut self.text;
        let menu_size = self
            .tray_menu
            .size(open_menu.as_ref(), |label| text.measure(label, &label_style).0);
        let placement = self.tray_menu.placement();
        let geometry = menu_size.map(|(width, height)| {
            (
                (width.round() as u32, height.round() as u32),
                placement,
                menu_target.clone(),
            )
        });
        if geometry != self.tray_menu_geometry {
            if let Some(((width, height), _placement, _)) = &geometry {
                let (width, height) = (*width, *height);
                for surface in &mut self.tray_menu_surfaces {
                    // Placement is per output: the monitor the menu opens on
                    // decides where its edge is, and applying one output's
                    // placement to every surface put the menu off screen on the
                    // other monitor. It opens level with its icon, moved back
                    // as far as it must to end on the output: from an icon low
                    // on the rail it ran off the bottom.
                    let extent = surface
                        .output
                        .as_ref()
                        .and_then(|o| self.output_state.info(o))
                        .and_then(|info| info.logical_size);
                    let along = self.tray_menu.placement().y.round() as i32;
                    let along = match (extent, self.config.sidebar.axis()) {
                        (Some((_, h)), crate::config::Axis::Vertical) => along.min(h - height as i32).max(0),
                        (Some((w, _)), crate::config::Axis::Horizontal) => along.min(w - width as i32).max(0),
                        (None, _) => along,
                    };
                    let (anchor, (top, right, bottom, left)) =
                        crate::wayland::surfaces::beside_rail(self.config.sidebar.position, along, 0);
                    surface.layer.set_size(width, height);
                    surface.layer.set_anchor(anchor);
                    surface.layer.set_margin(top, right, bottom, left);
                    surface.layer.commit();
                    // Changing size or margins starts a new configure cycle.
                    // Attaching a buffer before that configure arrives is a
                    // fatal `layerSurface was not configured` protocol error, so
                    // wait for it rather than painting this frame.
                    surface.configured = false;
                    surface.awaiting_configure = true;
                    surface.dirty = true;
                }
            }
            self.tray_menu_geometry = geometry;
        }
        for entry in &mut self.tray_menu_surfaces {
            let active = menu_target
                .as_deref()
                .map(|name| name == entry.output_name)
                .unwrap_or(false)
                && menu_size.is_some();
            if !active {
                entry.unmap();
                continue;
            }
            if !entry.ready() {
                continue;
            }
            // Only repaint on a change. Without this the surface redrew on every
            // loop iteration, which made the text shimmer as glyphs re-rasterized
            // at slightly different subpixel positions.
            if !entry.dirty {
                continue;
            }
            let output_name = entry.output_name.clone();
            let mut ctx = paint_ctx!(self, theme, &output_name, dt);
            let rect = entry.logical_rect();
            let pointer = self
                .pointer
                .as_ref()
                .filter(|(surface, _, _)| surface == entry.layer.wl_surface())
                .map(|(_, x, y)| (*x, *y));
            let mut list = crate::render::display_list::DisplayList::new(Color::TRANSPARENT);
            if let Some(menu) = &open_menu {
                self.tray_menu.paint(&mut ctx, &mut list, rect, menu, pointer);
            }
            entry.render_list(&self.gpu, self.display, &list, &mut self.text)?;
            entry.dirty = false;
        }
        Ok(())
    }

    /// The bar on every output.
    fn render_bars(&mut self, theme: &Theme, dt: f32) -> Result<(), String> {
        for entry in &mut self.surfaces {
            if !entry.dirty || !entry.configured {
                continue;
            }
            let input = crate::ui::InputState {
                pointer: self
                    .pointer
                    .as_ref()
                    .filter(|(surface, _, _)| surface == entry.layer.wl_surface())
                    .map(|(_, x, y)| (*x, *y)),
                pressed: self.pointer_pressed,
            };
            // Clone the output name: the context borrows it while `entry` is
            // borrowed mutably for rendering.
            let output_name = entry.output_name.clone();
            let mut ctx = paint_ctx!(self, theme, &output_name, dt);
            entry.render_with(
                &self.gpu,
                self.display,
                &self.compositor_state,
                &mut ctx,
                &mut self.sidebar,
                &input,
            )?;
            entry.dirty = false;
        }
        Ok(())
    }
}
