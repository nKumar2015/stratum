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
            DashboardAction::OpenUrl(url) => {
                self.actions.push(crate::actions::SystemAction::Spawn(vec!["xdg-open".into(), url]));
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
            SwitcherAction::Preview(name) => {
                let base = self.theme_preview.as_ref().unwrap_or(&self.config);
                if let Some(theme) = base.themes.get(&name).cloned() {
                    self.preview_theme(name, theme);
                }
            }
            SwitcherAction::PreviewWallpaper(name) => {
                let linked = self.wallpaper_links.get(&name).cloned();
                self.preview_wallpaper(linked);
            }
            SwitcherAction::Apply(name) => {
                if self.switch_theme(&name) {
                    self.mark_all_dirty();
                }
                self.drop_wallpaper_previews();
            }
            SwitcherAction::Cancel => {
                self.end_theme_preview();
                self.drop_wallpaper_previews();
            }
        }
        self.mark_themes_dirty();
    }

    /// Open the theme switcher on `output`. A theme being edited is put
    /// back first: the switcher previews over the theme really in use. The
    /// linked wallpapers are decoded large for its slices.
    pub(super) fn open_theme_switcher(&mut self, output: &str) {
        self.end_theme_preview();
        let themes = self.theme_names();
        let current = self.config.theme.clone();
        self.theme_switcher.open_on(output, &themes, &current);
        let mut linked: Vec<_> = self.wallpaper_links.values().cloned().collect();
        linked.sort();
        linked.dedup();
        if let Some(tx) = &self.wallpaper_tx {
            let _ = tx.send(crate::wallpapers::Command::Previews(linked));
        }
        self.mark_themes_dirty();
    }

    /// The switcher's large wallpapers are megabytes each: free them as soon
    /// as it closes.
    fn drop_wallpaper_previews(&mut self) {
        if let Some(tx) = &self.wallpaper_tx {
            let _ = tx.send(crate::wallpapers::Command::DropPreviews);
        }
    }

    /// Switch the active theme live. Returns false for an unknown name, which
    /// must not blank the shell.
    pub(super) fn switch_theme(&mut self, name: &str) -> bool {
        // A switch from anywhere ends a live preview: the draft must not be
        // carried into the config the switch keeps.
        self.end_theme_preview();
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
        // A theme linked to a wallpaper brings it along.
        if let Some(wallpaper) = self.wallpaper_links.get(name).cloned() {
            self.show_wallpaper(wallpaper);
        }
        true
    }

    /// Show `wallpaper`, unless it already is.
    ///
    /// Compared with what was last *asked* for, not what the worker last
    /// published: a restore that is queued but not yet shown would
    /// otherwise make showing the wallpaper it replaces look redundant, and
    /// the restore would win.
    fn show_wallpaper(&mut self, wallpaper: std::path::PathBuf) {
        let shown = self.wallpaper_requested.clone().or_else(|| self.state.wallpapers.get().current);
        if shown.as_ref() == Some(&wallpaper) {
            return;
        }
        if let Some(tx) = &self.wallpaper_tx {
            let _ = tx.send(crate::wallpapers::Command::Apply(wallpaper.clone()));
            self.wallpaper_requested = Some(wallpaper);
        }
    }

    /// Link `theme` to `wallpaper`, or unlink it. Linking the theme shown
    /// shows the wallpaper now, as switching to it would.
    pub(super) fn link_wallpaper(&mut self, theme: &str, wallpaper: Option<std::path::PathBuf>) {
        match &wallpaper {
            Some(path) => self.wallpaper_links.insert(theme.to_string(), path.clone()),
            None => self.wallpaper_links.remove(theme),
        };
        crate::wallpapers::store_links(&self.wallpaper_links);
        self.state.theme_wallpapers.set(self.wallpaper_links.clone());
        if let Some(path) = wallpaper.filter(|_| self.config.theme == theme) {
            self.show_wallpaper(path);
        }
    }

    /// Show a wallpaper while a theme is edited (`None`: the one from
    /// before). The one from before is remembered the first time.
    pub(super) fn preview_wallpaper(&mut self, wallpaper: Option<std::path::PathBuf>) {
        if self.wallpaper_before.is_none() {
            let shown = self.wallpaper_requested.clone().or_else(|| self.state.wallpapers.get().current);
            self.wallpaper_before = Some(shown);
        }
        let target = wallpaper.or_else(|| self.wallpaper_before.clone().flatten());
        if let Some(path) = target {
            self.show_wallpaper(path);
        }
    }

    /// Show `theme` live under `name` without saving it. The config before
    /// the first preview is kept, and every preview is applied over it, so
    /// ending the preview restores it exactly.
    pub(super) fn preview_theme(&mut self, name: String, theme: crate::config::ThemeConfig) {
        let base = Arc::clone(self.theme_preview.get_or_insert_with(|| Arc::clone(&self.config)));
        let mut updated = (*base).clone();
        updated.themes.insert(name.clone(), theme);
        updated.theme = name;
        self.replace_theme_config(updated);
    }

    /// Go back to the theme shown before the preview, if one is showing.
    pub(super) fn end_theme_preview(&mut self) {
        if let Some(Some(before)) = self.wallpaper_before.take() {
            self.show_wallpaper(before);
        }
        if let Some(base) = self.theme_preview.take() {
            self.config = base;
            self.control_center.set_config(Arc::clone(&self.config));
            self.mark_all_dirty();
            self.mark_themes_dirty();
        }
    }

    /// Save a custom theme to its file and switch to it. A failed write
    /// keeps the preview, so the edit is not lost.
    pub(super) fn save_custom_theme(
        &mut self,
        name: String,
        theme: crate::config::ThemeConfig,
        replaces: Option<String>,
        wallpaper: Option<std::path::PathBuf>,
    ) {
        let base = self.theme_preview.take().unwrap_or_else(|| Arc::clone(&self.config));
        let checked = crate::custom_themes::check_name(&name, &base.themes, &self.custom_themes, replaces.as_deref())
            .map_err(str::to_string)
            .and_then(|()| crate::custom_themes::save(&name, &theme));
        if let Err(err) = checked {
            tracing::error!("could not save theme `{name}`: {err}");
            if !Arc::ptr_eq(&base, &self.config) {
                self.theme_preview = Some(base);
            }
            return;
        }
        let mut updated = (*base).clone();
        if let Some(old) = replaces.filter(|old| *old != name) {
            if let Err(err) = crate::custom_themes::delete(&old) {
                tracing::warn!("could not remove the renamed theme `{old}`: {err}");
            }
            updated.themes.remove(&old);
            self.custom_themes.remove(&old);
            self.wallpaper_links.remove(&old);
        }
        updated.themes.insert(name.clone(), theme);
        self.custom_themes.insert(name.clone());
        self.config = Arc::new(updated);
        // The wallpaper previewed stays: it is the theme's now, or the
        // edit chose none and it is simply what is shown.
        self.wallpaper_before = None;
        self.link_wallpaper(&name, wallpaper);
        if self.switch_theme(&name) {
            self.mark_all_dirty();
        }
        self.mark_themes_dirty();
    }

    /// Delete a custom theme. When it was showing, fall back to the
    /// config's own theme, else the first one.
    pub(super) fn delete_custom_theme(&mut self, name: &str) {
        if !self.custom_themes.contains(name) {
            return;
        }
        self.end_theme_preview();
        if let Err(err) = crate::custom_themes::delete(name) {
            tracing::error!("could not delete theme `{name}`: {err}");
            return;
        }
        self.custom_themes.remove(name);
        self.link_wallpaper(name, None);
        let mut updated = (*self.config).clone();
        updated.themes.remove(name);
        let was_active = updated.theme == name;
        self.replace_theme_config(updated);
        if was_active {
            let config_theme = self.theme_choice.as_ref().map(|s| s.config_theme.clone());
            let fallback = config_theme
                .filter(|t| self.config.themes.contains_key(t))
                .or_else(|| self.config.themes.keys().next().cloned());
            if let Some(fallback) = fallback {
                self.switch_theme(&fallback);
            }
        }
        self.mark_all_dirty();
        self.mark_themes_dirty();
    }

    /// Use `config`, which differs from the current one only in its themes.
    /// The bar holds no theme state, so it is not rebuilt.
    fn replace_theme_config(&mut self, config: crate::config::Config) {
        self.config = Arc::new(config);
        self.control_center.set_config(Arc::clone(&self.config));
        self.mark_all_dirty();
        self.mark_themes_dirty();
    }
}
