//! The polkit dialog, toasts, dashboard, theme switcher and hover menus:
//! what their actions do, and when they repaint.

use super::*;

impl App {
    /// Run the polkit agent on its own runtime. The runtime's handle is kept so
    /// password checks (which spawn polkit's helper) can run on it too.
    ///
    /// Debug builds honour two test hooks. `STRATUM_POLKIT_SUBJECT_PID` makes
    /// the agent answer only that process's requests, so it can be tested
    /// while another agent holds the session's. `STRATUM_POLKIT_FIXTURE_NAME`
    /// skips polkitd entirely and serves the agent on the session bus under
    /// that name, for tests that send the requests themselves.
    pub(super) fn start_polkit_agent(&mut self, publisher: stratum_sys::Publisher) {
        use stratum_sys::managers::polkit::{self, AgentSubject, PolkitAgent, PolkitHost};

        let debug_env = |key: &str| {
            cfg!(debug_assertions)
                .then(|| std::env::var(key).ok())
                .flatten()
                .filter(|value| !value.is_empty())
        };
        let fixture = debug_env("STRATUM_POLKIT_FIXTURE_NAME");
        let subject = debug_env("STRATUM_POLKIT_SUBJECT_PID")
            .and_then(|pid| pid.parse().ok())
            .map(AgentSubject::Process)
            .unwrap_or(AgentSubject::Session);
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(err) => {
                tracing::error!("failed to start polkit runtime: {err}");
                return;
            }
        };
        let agent = Arc::new(PolkitAgent::new(PolkitHost::new(publisher)));
        self.polkit_agent = Some(Arc::clone(&agent));
        self.polkit_runtime = Some(runtime.handle().clone());
        std::thread::Builder::new()
            .name("stratum-polkit".into())
            .spawn(move || {
                if let Some(name) = fixture {
                    if let Err(err) = runtime.block_on(polkit::serve_fixture(agent, &name)) {
                        tracing::warn!("polkit fixture agent unavailable: {err}");
                    }
                    return;
                }
                if let Err(err) = runtime.block_on(polkit::register_agent_for(agent, subject)) {
                    // Most often another agent (the installed shell's) already holds the
                    // session. The runtime ends with this thread, which is
                    // fine: checks are spawned onto it only in answer to a
                    // request, and without a registration none can arrive.
                    tracing::warn!("polkit agent not registered ({subject:?}): {err}");
                }
            })
            .ok();
    }

    pub(super) fn start_polkit_events(&mut self, loop_handle: &calloop::LoopHandle<'static, App>) {
        let (tx, channel) = calloop::channel::channel();
        self.polkit_events = Some(tx);
        if let Err(err) = loop_handle.insert_source(channel, |event, _, app: &mut App| {
            if let calloop::channel::Event::Msg(event) = event {
                let event: PolkitEvent = event;
                info!("polkit verdict for {}: {:?}", event.cookie, event.verdict);
                if app.polkit_dialog.verdict(&event.cookie, event.verdict) {
                    app.mark_polkit_dirty();
                }
            }
        }) {
            warn!("failed to insert polkit event source: {err}");
        }
    }

    /// Carry out what a click or key on the toast stack asked for.
    pub(super) fn apply_toast_action(&mut self, action: crate::toasts::ToastAction) {
        use crate::toasts::ToastAction;
        match action {
            ToastAction::None => return,
            ToastAction::Redraw => {}
            // Reason 2 is "dismissed by the user".
            ToastAction::Dismiss(id) => {
                if let Some(server) = &self.notifications {
                    server.close(id, 2);
                }
            }
            ToastAction::Invoke(id, key) => {
                if let Some(server) = &self.notifications {
                    server.invoke(id, &key);
                }
            }
            ToastAction::Reply(id, text) => {
                if let Some(server) = &self.notifications {
                    server.reply(id, &text);
                }
            }
        }
        for entry in &mut self.toast_surfaces {
            entry.dirty = true;
        }
    }

    pub(super) fn mark_polkit_dirty(&mut self) {
        for entry in &mut self.polkit_surfaces {
            entry.dirty = true;
        }
    }

    /// Carry out what the dialog asked for.
    pub(super) fn apply_polkit_action(&mut self, action: crate::polkit_dialog::DialogAction) {
        use crate::polkit_dialog::{DialogAction, Verdict};
        use stratum_sys::managers::polkit::FailureOutcome;

        match action {
            DialogAction::None => return,
            DialogAction::Redraw => {}
            DialogAction::Dismiss { cookie } => {
                if let Some(agent) = &self.polkit_agent {
                    agent.dismiss(&cookie);
                }
            }
            DialogAction::Submit {
                cookie,
                user,
                password,
            } => {
                let (Some(agent), Some(runtime), Some(sender)) = (
                    self.polkit_agent.clone(),
                    self.polkit_runtime.clone(),
                    self.polkit_events.clone(),
                ) else {
                    return;
                };
                runtime.spawn(async move {
                    let ok = stratum_sys::managers::polkit::verify_password_and_notify(
                        &user,
                        password.expose(),
                        &cookie,
                    )
                    .await;
                    drop(password);
                    let verdict = if ok {
                        agent.resolve(&cookie, true);
                        Verdict::Accepted
                    } else {
                        match agent.record_failure(&cookie) {
                            FailureOutcome::Retry { remaining } => Verdict::Retry { remaining },
                            FailureOutcome::Exhausted | FailureOutcome::Unknown => Verdict::Exhausted,
                        }
                    };
                    let _ = sender.send(PolkitEvent { cookie, verdict });
                });
            }
        }
        self.mark_polkit_dirty();
    }

    pub(super) fn mark_dashboard_dirty(&mut self) {
        for entry in &mut self.dashboard_surfaces {
            entry.dirty = true;
        }
    }

    /// Carry out what the dashboard asked for.
    pub(super) fn apply_dashboard_action(&mut self, action: crate::dashboard::DashboardAction) {
        use crate::dashboard::DashboardAction;
        match action {
            DashboardAction::None => return,
            DashboardAction::Redraw => {}
            DashboardAction::Media { player, action } => {
                self.actions.push(crate::actions::SystemAction::Media { player, action });
            }
            DashboardAction::ToggleDnd => {
                let next = !self.state.dnd.get();
                self.state.dnd.set(next);
            }
            DashboardAction::Seek { player, seconds } => {
                self.actions.push(crate::actions::SystemAction::MediaSeek { player, seconds });
            }
            DashboardAction::MediaSet { player, property, value } => {
                self.actions.push(crate::actions::SystemAction::MediaSet { player, property, value });
            }
            DashboardAction::ChoosePlayer(player) => {
                self.actions.push(crate::actions::SystemAction::MusicRefresh(Some(player)));
            }
        }
        self.mark_dashboard_dirty();
    }

    pub(super) fn mark_themes_dirty(&mut self) {
        for entry in &mut self.theme_surfaces {
            entry.dirty = true;
        }
    }

    /// Feed the pointer to the hover-menu timers: the status icon it rests on
    /// (on a bar surface) and whether it is over the open menu. Called on
    /// pointer events and every frame while a timer runs.
    pub(super) fn update_hover(&mut self) {
        use crate::ui::hover_menu::OpenMenu;
        let m = &self.config.metrics.hover_menu;
        let (open_after, close_after) = (
            std::time::Duration::from_millis(m.open_ms),
            std::time::Duration::from_millis(m.close_ms),
        );
        let (icon, over_menu) = match &self.pointer {
            Some((surface, x, y)) => {
                let point = crate::render::display_list::Point::new(*x, *y);
                let icon = self
                    .surfaces
                    .iter()
                    .find(|e| e.layer.wl_surface() == surface)
                    .and_then(|bar| {
                        let (kind, rect) = self.sidebar.hover_target(&bar.output_name, point)?;
                        Some(OpenMenu {
                            kind,
                            output: bar.output_name.clone(),
                            anchor_x: rect.x + rect.width * 0.5,
                            anchor_y: rect.y + rect.height * 0.5,
                        })
                    });
                let over = self.hover_surfaces.iter().any(|e| e.layer.wl_surface() == surface);
                (icon, over)
            }
            None => (None, false),
        };
        // Nothing opens over the lock screen.
        let icon = icon.filter(|_| !self.lock.active());
        if self
            .hover_menus
            .intent
            .update(std::time::Instant::now(), icon, over_menu, open_after, close_after)
        {
            self.mark_hover_dirty();
        }
    }

    /// Close the tray menu once the pointer has been off both it and the icon
    /// it opened from for `tray_menu.close_ms`, as the QML tray did. Called on
    /// pointer events and every frame while the countdown runs.
    pub(super) fn update_tray_menu_hover(&mut self) {
        let Some(output) = self.tray_menu.target().map(str::to_string) else {
            self.tray_menu_left_at = None;
            return;
        };
        let cell = self.config.metrics.tray.cell;
        let item_y = self.tray_menu.placement().y;
        let over = self.pointer.as_ref().is_some_and(|(surface, _, y)| {
            let on_menu = self.tray_menu_surfaces.iter().any(|t| t.layer.wl_surface() == surface);
            let on_icon = self
                .surfaces
                .iter()
                .any(|b| b.layer.wl_surface() == surface && b.output_name == output)
                && (item_y..=item_y + cell).contains(y);
            on_menu || on_icon
        });
        if over {
            self.tray_menu_left_at = None;
            return;
        }
        let left = *self.tray_menu_left_at.get_or_insert_with(std::time::Instant::now);
        if left.elapsed() >= std::time::Duration::from_millis(self.config.metrics.tray_menu.close_ms) {
            self.actions.push(crate::actions::SystemAction::TrayMenuClose);
            self.tray_menu.close();
            self.tray_menu_geometry = None;
            self.tray_menu_left_at = None;
        }
    }

    pub(super) fn mark_hover_dirty(&mut self) {
        for entry in &mut self.hover_surfaces {
            entry.dirty = true;
        }
    }

    /// Where the hover menu goes along the rail on an output of
    /// `output_size`: centred on the icon, kept on screen. The compositor
    /// places it beside the rail (`surfaces::beside_rail`).
    pub(super) fn hover_placement(&self, menu: &crate::ui::hover_menu::OpenMenu, size: (f32, f32), output_size: (f32, f32)) -> i32 {
        let m = &self.config.metrics.hover_menu;
        let (w, h) = size;
        let (ow, oh) = output_size;
        let clamp = |v: f32, len: f32, total: f32| v.min(total - len - m.offset).max(m.offset);
        let along = match self.config.sidebar.position {
            SidebarPosition::Left | SidebarPosition::Right => clamp(menu.anchor_y - h * 0.5, h, oh),
            SidebarPosition::Top | SidebarPosition::Bottom => clamp(menu.anchor_x - w * 0.5, w, ow),
        };
        along.round() as i32
    }

    /// Carry out what a hover menu asked for.
    pub(super) fn apply_hover_events(&mut self, events: Vec<crate::ui::hover_menu::MenuEvent>) {
        use crate::ui::hover_menu::MenuEvent;
        for event in events {
            match event {
                MenuEvent::System(action) => self.actions.push(action),
                MenuEvent::Close => self.hover_menus.intent.close(),
                MenuEvent::OpenPage(page) => {
                    self.hover_menus.intent.close();
                    self.control_center.show(page);
                    self.mark_all_dirty();
                }
            }
        }
        self.mark_hover_dirty();
    }

    /// Carry out what the theme switcher asked for.
    pub(super) fn apply_switcher_action(&mut self, action: crate::theme_switcher::SwitcherAction) {
        use crate::theme_switcher::SwitcherAction;
        match action {
            SwitcherAction::None => return,
            SwitcherAction::Redraw => {}
            SwitcherAction::Apply(name) => {
                if self.switch_theme(&name) {
                    self.mark_all_dirty();
                }
            }
        }
        self.mark_themes_dirty();
    }

    /// Switch the active theme live. Returns false for an unknown name, which
    /// must not blank the shell.
    pub(super) fn switch_theme(&mut self, name: &str) -> bool {
        if !self.config.themes.contains_key(name) {
            return false;
        }
        let config_theme =
            self.theme_choice.as_ref().map_or(&self.config.theme, |s| &s.config_theme).clone();
        self.theme_choice = crate::theme_choice::pick(name, &config_theme);
        crate::theme_choice::store(self.theme_choice.as_ref());
        let mut updated = (*self.config).clone();
        updated.theme = name.to_string();
        self.config = Arc::new(updated);
        self.sidebar = Sidebar::new(Arc::clone(&self.config));
        // Keep the panel's page and open state across the swap.
        self.control_center.set_config(Arc::clone(&self.config));
        true
    }
}
