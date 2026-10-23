//! Seat, pointer and keyboard input.

use super::*;


impl SeatHandler for App {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer {
            match self.seat_state.get_pointer(qh, &seat) {
                Ok(pointer) => {
                    info!("pointer capability acquired");
                    self.cursor_shape_device =
                        self.cursor_shapes.as_ref().map(|m| m.get_shape_device(&pointer, qh));
                }
                Err(err) => tracing::warn!("failed to create pointer: {err}"),
            }
        }
        if capability == Capability::Keyboard {
            match self.seat_state.get_keyboard(qh, &seat, None) {
                Ok(_keyboard) => info!("keyboard capability acquired"),
                Err(err) => tracing::warn!("failed to create keyboard: {err}"),
            }
        }
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        _: Capability,
    ) {
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl PointerHandler for App {
    fn pointer_frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _pointer: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        // The lock screen has nothing to click, and nothing behind it may be
        // clicked.
        if self.lock.active() {
            self.pointer = None;
            self.pointer_pressed = false;
            return;
        }
        let mut clicked_at: Option<(wl_surface::WlSurface, f32, f32, crate::ui::MouseButton)> = None;
        let mut scroll_at: Option<(wl_surface::WlSurface, f32, f32, f32)> = None;
        let mut changed = false;

        for event in events {
            let (x, y) = (event.position.0 as f32, event.position.1 as f32);
            // The screenshot overlay needs every press, motion and release in
            // order (a drag is all three), not the frame's summary below.
            let shot_output = self
                .screenshot_surfaces
                .iter()
                .find(|e| e.layer.wl_surface() == &event.surface)
                .map(|e| e.output_name.clone());
            if let Some(output) = shot_output {
                self.screenshot_pointer(&output, &event.kind, x, y);
                continue;
            }
            // The viewer draws with every motion, not once per frame.
            if self.viewer_window.as_ref().is_some_and(|w| w.window.wl_surface() == &event.surface) {
                if let (PointerEventKind::Enter { serial }, Some(device)) =
                    (&event.kind, &self.cursor_shape_device)
                {
                    device.set_shape(
                        *serial,
                        smithay_client_toolkit::reexports::protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::Shape::Crosshair,
                    );
                }
                self.viewer_pointer(&event.kind, x, y);
                continue;
            }
            // The tray menu repaints only when marked, so its hover highlight
            // needs every move, enter and leave over it.
            if let Some(menu) = self
                .tray_menu_surfaces
                .iter_mut()
                .find(|t| t.layer.wl_surface() == &event.surface)
            {
                if matches!(
                    event.kind,
                    PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } | PointerEventKind::Leave { .. }
                ) {
                    menu.dirty = true;
                }
            }
            // Once any surface sets a shape, every surface must: compositors
            // keep the last image across enters, so the screenshot crosshair
            // would otherwise stay on the bar.
            if let (PointerEventKind::Enter { serial }, Some(device)) =
                (&event.kind, &self.cursor_shape_device)
            {
                device.set_shape(
                    *serial,
                    smithay_client_toolkit::reexports::protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::Shape::Default,
                );
            }
            match event.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    self.pointer = Some((event.surface.clone(), x, y));
                    changed = true;
                }
                PointerEventKind::Leave { .. } => {
                    self.pointer = None;
                    self.pointer_pressed = false;
                    changed = true;
                }
                PointerEventKind::Press { button, .. } => {
                    let mapped = match button {
                        BTN_LEFT => Some(crate::ui::MouseButton::Left),
                        BTN_RIGHT => Some(crate::ui::MouseButton::Right),
                        BTN_MIDDLE => Some(crate::ui::MouseButton::Middle),
                        _ => None,
                    };
                    if let Some(mapped) = mapped {
                        if mapped == crate::ui::MouseButton::Left {
                            self.pointer_pressed = true;
                        }
                        clicked_at = Some((event.surface.clone(), x, y, mapped));
                        changed = true;
                    }
                }
                PointerEventKind::Release { button, .. } => {
                    if button == BTN_LEFT {
                        self.pointer_pressed = false;
                        changed = true;
                    }
                }
                PointerEventKind::Axis { vertical, .. } => {
                    // Wheel input scrolls list pages. `value120` is the
                    // high-resolution step count on modern compositors;
                    // `discrete` is the fallback for older ones.
                    let step = if vertical.value120 != 0 {
                        vertical.value120 as f32
                    } else if vertical.discrete != 0 {
                        vertical.discrete as f32 * 120.0
                    } else {
                        vertical.absolute as f32
                    };
                    if step != 0.0 {
                        scroll_at = Some((event.surface.clone(), x, y, step));
                    }
                }
            }
        }

        if let Some((surface, x, y, button)) = clicked_at {
            tracing::trace!("click {button:?} at ({x:.1},{y:.1})");
            let input = crate::ui::InputState {
                pointer: Some((x, y)),
                pressed: true,
            };
            let point = crate::render::display_list::Point::new(x, y);

            // The settings window handles its own clicks; the bar only acts
            // when the window is not the target.
            let in_settings = self
                .settings
                .as_ref()
                .map(|w| w.window.wl_surface() == &surface)
                .unwrap_or(false);

            // Toasts are on their own surface; a click dismisses the card.
            let on_toast = self
                .toast_surfaces
                .iter()
                .any(|t| t.layer.wl_surface() == &surface);
            let on_polkit = self
                .polkit_surfaces
                .iter()
                .any(|p| p.layer.wl_surface() == &surface);
            if on_polkit {
                let action = self.polkit_dialog.on_click(point);
                self.apply_polkit_action(action);
                return;
            }
            // The hover menu is immediate-mode: the press is handed to its
            // next paint, which knows where its controls are.
            if self.hover_surfaces.iter().any(|t| t.layer.wl_surface() == &surface) {
                self.hover_clicked = true;
                self.mark_hover_dirty();
                return;
            }
            if self.dashboard_surfaces.iter().any(|t| t.layer.wl_surface() == &surface) {
                let music = self.state.music.get();
                let action = self.dashboard.on_click(point, &music);
                self.apply_dashboard_action(action);
                return;
            }
            if self.theme_surfaces.iter().any(|t| t.layer.wl_surface() == &surface) {
                let themes = self.theme_names();
                let action = self.theme_switcher.on_click(point, &themes);
                self.apply_switcher_action(action);
                return;
            }
            // The power menu owns the pointer while open; a tile click runs it.
            let on_power = self
                .power_surfaces
                .iter()
                .any(|p| p.layer.wl_surface() == &surface);
            // A click inside the open tray menu either runs an entry or
            // dismisses the menu.
            let tray_menu_width = self
                .tray_menu_surfaces
                .iter()
                .find(|t| t.layer.wl_surface() == &surface)
                .map(|t| t.logical_rect().width);
            if let Some(width) = tray_menu_width {
                let menu = self.state.tray_menu.get();
                if let Some(menu) = menu {
                    if let Some(id) = self.tray_menu.on_click(point, &menu, width) {
                        self.actions.push(crate::actions::SystemAction::TrayMenuEvent {
                            service: menu.service,
                            path: menu.path,
                            id,
                        });
                    } else {
                        self.actions.push(crate::actions::SystemAction::TrayMenuClose);
                    }
                }
                self.tray_menu.close();
                self.tray_menu_geometry = None;
                changed = true;
            } else if on_power {
                if let Some(entry) = self
                    .power_surfaces
                    .iter()
                    .find(|p| p.layer.wl_surface() == &surface)
                {
                    let rect = entry.logical_rect();
                    if let Some(argv) = self.power_menu.on_click(point, rect) {
                        self.actions.push(crate::actions::SystemAction::Spawn(argv));
                    }
                }
                changed = true;
            } else if on_toast {
                let output = self
                    .toast_surfaces
                    .iter()
                    .find(|t| t.layer.wl_surface() == &surface)
                    .map(|t| t.output_name.clone())
                    .unwrap_or_default();
                let action = self.toasts.on_click(point, &output);
                self.apply_toast_action(action);
                changed = true;
            } else if in_settings {
                if let Some(window) = self.settings.as_ref() {
                    let rect = crate::render::display_list::Rect {
                        x: 0.0,
                        y: 0.0,
                        width: window.width as f32,
                        height: window.height as f32,
                        radius: 0.0,
                        color: crate::render::display_list::Color::TRANSPARENT,
                    };
                    if let Some(action) = self.control_center.on_click(point, rect, &input) {
                        self.apply_action(action);
                    }
                    if button == crate::ui::MouseButton::Left {
                        self.settings_press = true;
                    }
                    // Every click in the window needs a redraw: controls are
                    // immediate-mode and read the pointer state on the next
                    // frame, so without this the slider and buttons never
                    // observe a press.
                    if let Some(settings) = &mut self.settings {
                        settings.dirty = true;
                    }
                    changed = true;
                }
            } else {
                // Clicking a status icon opens its full page, so its hover
                // menu goes, as in the QML shell.
                self.hover_menus.intent.close();
                // A click elsewhere dismisses an open tray menu, the way
                // clicking away from a popup does in the QML shell.
                if self.tray_menu.target().is_some() {
                    self.actions.push(crate::actions::SystemAction::TrayMenuClose);
                    self.tray_menu.close();
                    self.tray_menu_geometry = None;
                }
                // The widget tree is shared across outputs, so the click has to
                // be attributed to the surface it landed on: a workspace click
                // on the left monitor must move that monitor.
                let output_name = self
                    .surfaces
                    .iter()
                    .find(|entry| entry.layer.wl_surface() == &surface)
                    .map(|entry| entry.output_name.clone())
                    .or_else(|| self.surfaces.first().map(|e| e.output_name.clone()))
                    .unwrap_or_default();
                if let Some(action) = self.sidebar.on_click(point, &output_name, button) {
                    self.apply_action(action);
                    changed = true;
                }
            }
        }

        if let Some((surface, x, y, delta)) = scroll_at {
            let in_settings = self
                .settings
                .as_ref()
                .map(|w| w.window.wl_surface() == &surface)
                .unwrap_or(false);
            if in_settings {
                self.control_center.on_scroll(
                    crate::render::display_list::Point::new(x, y),
                    delta,
                );
                if let Some(settings) = &mut self.settings {
                    settings.dirty = true;
                }
                changed = true;
            } else if let Some(output_name) = self
                .surfaces
                .iter()
                .find(|entry| entry.layer.wl_surface() == &surface)
                .map(|entry| entry.output_name.clone())
            {
                // The bar: the workspace strip switches workspace.
                let point = crate::render::display_list::Point::new(x, y);
                if let Some(action) = self.sidebar.on_scroll(point, &output_name, delta) {
                    self.apply_action(action);
                    changed = true;
                }
            }
        }

        // The card under the pointer is highlighted and does not expire.
        let over_toast = self.pointer.as_ref().and_then(|(surface, x, y)| {
            self.toast_surfaces
                .iter()
                .any(|t| t.layer.wl_surface() == surface)
                .then(|| crate::render::display_list::Point::new(*x, *y))
        });
        if self.toasts.set_pointer(over_toast) {
            for entry in &mut self.toast_surfaces {
                entry.dirty = true;
            }
        }
        // Hover menus: their timers follow the pointer, and the open menu
        // repaints as the pointer moves over it (row highlights, a dragged
        // slider).
        self.update_hover();
        self.update_tray_menu_hover();
        let over_hover = self.pointer.as_ref().is_some_and(|(surface, _, _)| {
            self.hover_surfaces.iter().any(|t| t.layer.wl_surface() == surface)
        });
        if over_hover || self.pointer_pressed {
            self.mark_hover_dirty();
        }

        // Hovering a theme row selects it, as in the QML switcher.
        let over_themes = self.pointer.as_ref().and_then(|(surface, x, y)| {
            self.theme_surfaces
                .iter()
                .any(|t| t.layer.wl_surface() == surface)
                .then(|| crate::render::display_list::Point::new(*x, *y))
        });
        if over_themes.is_some_and(|p| self.theme_switcher.on_hover(p)) {
            self.mark_themes_dirty();
        }

        if changed {
            for entry in &mut self.surfaces {
                entry.dirty = true;
            }
        }
    }
}

impl ActivationHandler for App {
    type RequestUdata = ();

    fn new_token(&mut self, token: String, data: &RequestData<()>) {
        // Apply the token to the window that asked for it (the settings
        // window or the screenshot viewer) so the compositor focuses it.
        if let (Some(activation), Some(surface)) = (&self.activation, &data.surface) {
            activation.activate::<App>(surface, token);
        }
    }
}

impl KeyboardHandler for App {
    fn enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wayland_client::protocol::wl_keyboard::WlKeyboard,
        surface: &wl_surface::WlSurface,
        _serial: u32,
        _raw: &[u32],
        _keysyms: &[Keysym],
    ) {
        self.keyboard_focus = Some(surface.clone());
    }

    fn leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wayland_client::protocol::wl_keyboard::WlKeyboard,
        surface: &wl_surface::WlSurface,
        _serial: u32,
    ) {
        if self.keyboard_focus.as_ref() == Some(surface) {
            self.keyboard_focus = None;
        }
    }

    fn press_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wayland_client::protocol::wl_keyboard::WlKeyboard,
        _serial: u32,
        event: KeyboardEvent,
    ) {
        // While locked every key belongs to the lock screen, whatever else is
        // open.
        if self.lock.active() {
            match self
                .lock
                .key(event.keysym, event.utf8.as_deref(), self.modifiers.ctrl)
            {
                crate::lock::KeyOutcome::Ignored => {}
                crate::lock::KeyOutcome::Changed => self.mark_lock_dirty(),
                crate::lock::KeyOutcome::Submit(password) => {
                    self.mark_lock_dirty();
                    let (Some(auth), Some(sender)) = (&self.lock_auth, self.lock_events.clone())
                    else {
                        return;
                    };
                    let service = auth.service.clone();
                    let user = auth.user.clone();
                    let confdir = crate::pam::test_confdir();
                    // PAM blocks, and pam_unix sleeps after a failure, so the
                    // check must not run on the UI thread.
                    std::thread::spawn(move || {
                        let result =
                            crate::pam::authenticate(&service, &user, password, confdir.as_deref());
                        let _ = sender.send(LockEvent::Auth(result));
                    });
                }
            }
            return;
        }

        // An open polkit dialog has exclusive focus; a program is waiting on it.
        if self.polkit_dialog.is_open() {
            let action = self
                .polkit_dialog
                .key(event.keysym, event.utf8.as_deref(), self.modifiers.ctrl);
            self.apply_polkit_action(action);
            return;
        }

        // A focused toast reply field holds the keyboard until sent or
        // abandoned.
        if self.toasts.wants_keys() {
            let action = self
                .toasts
                .key(event.keysym, event.utf8.as_deref(), self.modifiers.ctrl);
            self.apply_toast_action(action);
            return;
        }

        if self.screenshot.is_open() {
            if event.keysym == Keysym::Escape {
                self.screenshot.close();
                self.mark_screenshot_dirty();
            }
            return;
        }

        let viewer_focused = match (&self.viewer_window, &self.keyboard_focus) {
            (Some(window), Some(focus)) => window.window.wl_surface() == focus,
            _ => false,
        };
        if viewer_focused {
            let action = self.viewer.key(event.keysym, self.modifiers.ctrl);
            self.apply_viewer_action(action);
            return;
        }

        if self.dashboard.is_open() {
            let music = self.state.music.get();
            let action = self.dashboard.key(event.keysym, event.utf8.as_deref(), self.modifiers, &music);
            self.apply_dashboard_action(action);
            return;
        }

        if self.theme_switcher.is_open() {
            let themes = self.theme_names();
            let action = self.theme_switcher.key(
                event.keysym,
                event.utf8.as_deref(),
                self.modifiers.ctrl,
                &themes,
            );
            self.apply_switcher_action(action);
            return;
        }

        // The power menu owns the keyboard while it is open, so its keys are
        // handled first and nothing else sees them.
        if self.power_menu.is_open() {
            let Some(action) = self.power_menu.key_action(event.keysym) else {
                return;
            };
            tracing::debug!("power menu key {:?} -> {:?}", event.keysym, action);
            if let Some(argv) = self.power_menu.apply(action) {
                self.actions.push(crate::actions::SystemAction::Spawn(argv));
            }
            for entry in &mut self.power_surfaces {
                entry.dirty = true;
            }
            return;
        }

        // A text field in the settings window (the EQ's preset name) takes
        // typing while the window has focus, Escape included.
        let settings_focused = match (&self.settings, &self.keyboard_focus) {
            (Some(window), Some(focus)) => window.window.wl_surface() == focus,
            _ => false,
        };
        if settings_focused && self.control_center.wants_keys() {
            self.control_center
                .key(event.keysym, event.utf8.as_deref(), self.modifiers.ctrl);
            if let Some(settings) = &mut self.settings {
                settings.dirty = true;
            }
            return;
        }

        // Escape closes the settings window, which is how an ordinary window
        // behaves and what the QML panels did.
        if event.keysym == Keysym::Escape && self.control_center.open {
            self.control_center.close();
            for entry in &mut self.surfaces {
                entry.dirty = true;
            }
        }
    }

    fn repeat_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wayland_client::protocol::wl_keyboard::WlKeyboard,
        _serial: u32,
        _event: KeyboardEvent,
    ) {
        // Navigation keys do not need key repeat; ignoring it keeps held arrows
        // from racing across the menu.
    }

    fn release_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wayland_client::protocol::wl_keyboard::WlKeyboard,
        _serial: u32,
        _event: KeyboardEvent,
    ) {
    }

    fn update_modifiers(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wayland_client::protocol::wl_keyboard::WlKeyboard,
        _serial: u32,
        modifiers: Modifiers,
        _raw_modifiers: RawModifiers,
        _layout: u32,
    ) {
        self.modifiers = modifiers;
    }
}
