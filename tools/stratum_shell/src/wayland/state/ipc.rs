//! The IPC socket's requests, carried out on the event loop.

use super::*;

impl App {
    /// Start the IPC listener and route requests through calloop.
    ///
    /// The listener runs on its own thread because a client may connect at any
    /// time. Requests that need UI state — opening a panel, switching the theme
    /// — are delivered to the event loop through a calloop channel, so they are
    /// handled on the same thread as every other state change.
    pub(super) fn start_ipc(&mut self, loop_handle: &calloop::LoopHandle<'static, App>) {
        let (tx, channel) = calloop::channel::channel();
        self._ipc = crate::ipc::start(tx);
        if let Err(err) = loop_handle.insert_source(channel, |event, _, app: &mut App| {
            match event {
                calloop::channel::Event::Msg(request) => app.handle_ipc(request),
                // The listener thread is gone; nothing more can arrive.
                calloop::channel::Event::Closed => {}
            }
        }) {
            warn!("failed to insert ipc source: {err}");
        }
    }

    /// Execute one validated IPC call and answer the client.
    /// Dismiss every notification, as the user closing each one.
    pub(super) fn dismiss_all_notifications(&self) {
        if let Some(server) = &self.notifications {
            let ids: Vec<u32> = self.state.notifications.get().iter().map(|n| n.id).collect();
            for id in ids {
                server.close(id, 2);
            }
        }
    }

    fn handle_ipc(&mut self, request: crate::ipc::Request) {
        let result = self.execute_ipc(&request.call);
        if result.is_err() {
            tracing::debug!(
                "ipc {} {} failed: {:?}",
                request.call.target,
                request.call.function,
                result
            );
        }
        // If the client has already gone away the send fails; that is its
        // problem, not ours, and must not stop the shell.
        let _ = request.reply.send(result);
    }

    fn execute_ipc(&mut self, call: &IpcCall) -> Result<(), IpcError> {
        use crate::ipc::IpcError;

        match (call.target.as_str(), call.function.as_str()) {
            // The notifications centre is the Notifications page of the
            // settings window.
            ("notifications", "open") => {
                self.control_center.show(crate::ui::control_center::Page::Notifications);
            }
            ("notifications", "close") => self.control_center.close(),
            ("notifications", "toggle") => {
                self.control_center.toggle(crate::ui::control_center::Page::Notifications)
            }
            ("notifications", "clear") => self.dismiss_all_notifications(),
            ("notifications", "toggleDnd") => {
                let next = !self.state.dnd.get();
                self.state.dnd.set(next);
                self.state.notify();
            }
            ("theme", "set") => {
                // The parser guarantees exactly one non-empty argument.
                let name = call
                    .args
                    .first()
                    .ok_or_else(|| IpcError::BadArgs("missing theme name".into()))?;
                if !self.switch_theme(name) {
                    return Err(IpcError::BadArgs(format!("unknown theme: {name}")));
                }
            }
            ("theme", function @ ("open" | "close" | "toggle")) => {
                // Opens where the user is looking, like the other overlays.
                let output = self.focused_output();
                let themes = self.theme_names();
                let current = self.config.theme.clone();
                match function {
                    "open" if !self.theme_switcher.is_open() => {
                        self.theme_switcher.open_on(&output, &themes, &current)
                    }
                    "close" => self.theme_switcher.close(),
                    "toggle" => self.theme_switcher.toggle(&output, &themes, &current),
                    _ => {}
                }
                self.mark_themes_dirty();
            }
            ("dashboard", function @ ("open" | "close" | "toggle")) => {
                // The parser allows one argument; only the config knows the
                // tabs.
                let tab = match call.args.first() {
                    Some(name) => Some(self.config.dashboard.tab_index(name).ok_or_else(|| {
                        let names: Vec<_> = self.config.dashboard.tabs.iter().map(|t| t.name.as_str()).collect();
                        IpcError::BadArgs(format!("unknown dashboard tab: {name} (tabs: {})", names.join(", ")))
                    })?),
                    None => None,
                };
                let output = self.focused_output();
                match function {
                    "open" => self.dashboard.open_on(&output, tab),
                    "close" => self.dashboard.close(),
                    "toggle" => self.dashboard.toggle(&output, tab),
                    _ => {}
                }
                self.mark_dashboard_dirty();
            }
            ("todo", "add") => {
                // The parser guarantees at least one argument; the words of
                // an unquoted `todo add buy milk` arrive one each.
                if !self.dashboard.add_todo(&call.args.join(" ")) {
                    return Err(IpcError::BadArgs("a to-do needs some text".into()));
                }
                self.mark_dashboard_dirty();
            }
            ("timer", function) => {
                self.dashboard
                    .timer_command(function, call.args.first().map(String::as_str))
                    .map_err(IpcError::BadArgs)?;
                self.mark_dashboard_dirty();
            }
            ("settings", function @ ("open" | "close" | "toggle")) => {
                use crate::ui::control_center::Page;
                // The parser has already rejected unknown page names. Without
                // one, the window reopens on the page it last showed.
                let named = call.args.first().and_then(|name| Page::from_name(name));
                let page = named.unwrap_or(self.control_center.page);
                match function {
                    "open" => self.control_center.show(page),
                    "close" => self.control_center.close(),
                    "toggle" if named.is_none() && self.control_center.open => {
                        self.control_center.close()
                    }
                    "toggle" => self.control_center.toggle(page),
                    _ => {}
                }
            }
            ("screenshot", "start") => self.start_screenshot(),
            ("lockscreen", "lock") => self.begin_lock()?,
            ("powermenu", "toggle") => {
                // The menu opens on the focused monitor, not on whichever is
                // first: the QML shell routed popups the same way.
                let output = self.focused_output();
                self.power_menu.toggle(&output);
                for entry in &mut self.power_surfaces {
                    entry.dirty = true;
                }
            }
            _ => return Err(IpcError::Unsupported),
        }

        self.mark_all_dirty();
        Ok(())
    }
}
