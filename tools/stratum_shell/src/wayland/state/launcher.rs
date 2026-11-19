//! The launcher's surfaces: a box-sized overlay on the output it opened on,
//! and behind it a transparent catcher over the whole output that closes it
//! when clicked outside. The catcher is a single-pixel buffer stretched by
//! `wp_viewporter`, so a click outside costs no output-sized swapchain.

use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;

use super::idle::Quiet;
use super::*;
use crate::launcher::LauncherAction;

/// The transparent layer that takes clicks outside the launcher.
pub(super) struct Catcher {
    layer: LayerSurface,
    viewport: WpViewport,
    buffer: WlBuffer,
}

impl Drop for Catcher {
    fn drop(&mut self) {
        self.viewport.destroy();
        self.buffer.destroy();
    }
}

impl App {
    /// Open on `output`, reading what it searches.
    pub(super) fn open_launcher(&mut self, output: &str) {
        let windows = crate::hyprland::clients();
        let commands = self.launcher_commands();
        self.launcher.open_on(output, windows, commands);
        // Sized at every open, so a config reload applies.
        let (width, height) = crate::launcher::Launcher::size(&self.config.metrics.launcher, self.config.launcher.rows);
        let margin = self.config.metrics.launcher.margin_top as i32;
        if let Some(entry) = self.launcher_surfaces.iter_mut().find(|e| e.output_name == output) {
            entry.layer.set_size(width, height);
            entry.layer.set_margin(margin, 0, 0, 0);
        }
        self.launcher_catcher = self.make_catcher(output);
        self.mark_launcher_dirty();
    }

    /// After the launcher closed: drop the catcher and the icons it loaded.
    fn launcher_closed(&mut self) {
        self.launcher.close();
        // The box is unmapped now, before the catcher goes, not at the next
        // render: removing the catcher makes wlroots send every layer a
        // configure, and a box unmapped after one arrived would ack it late,
        // which sway treats as fatal ("wrong configure serial").
        for entry in &mut self.launcher_surfaces {
            entry.unmap();
        }
        self.launcher_catcher = None;
        let icon = self.config.metrics.launcher.icon as u32;
        self.icons.forget(icon * 2);
        self.mark_launcher_dirty();
    }

    pub(super) fn close_launcher(&mut self) {
        if self.launcher.is_open() || self.launcher_catcher.is_some() {
            self.launcher_closed();
        }
    }

    pub(super) fn mark_launcher_dirty(&mut self) {
        for entry in &mut self.launcher_surfaces {
            entry.dirty = true;
        }
    }

    pub(super) fn apply_launcher_action(&mut self, action: LauncherAction) {
        match action {
            LauncherAction::None => {}
            LauncherAction::Redraw => self.mark_launcher_dirty(),
            LauncherAction::Close => self.launcher_closed(),
            LauncherAction::Launch { argv, terminal } => {
                self.launcher_closed();
                let config = &self.config.launcher;
                let mut command = config.launch_prefix.clone();
                if terminal {
                    command.extend(config.terminal.iter().cloned());
                }
                command.extend(argv);
                info!("launching {:?}", command);
                self.actions.push(crate::actions::SystemAction::Spawn(command));
            }
            LauncherAction::Copy(text) => {
                self.launcher_closed();
                self.actions.push(crate::actions::SystemAction::Spawn(vec!["wl-copy".into(), text]));
            }
            LauncherAction::FocusWindow(address) => {
                self.launcher_closed();
                if !crate::hyprland::focus_window(&address) {
                    warn!("could not focus window {address}");
                }
            }
            LauncherAction::Ipc(call) => {
                self.launcher_closed();
                if let Err(err) = self.execute_ipc(&call) {
                    warn!("launcher: {} {} failed: {}", call.target, call.function, err.message());
                }
            }
        }
    }

    /// The shell's own actions, offered by name.
    fn launcher_commands(&self) -> Vec<crate::launcher::Command> {
        use crate::launcher::Command;
        let call = |target: &str, function: &str, args: &[&str]| crate::ipc::IpcCall {
            target: target.into(),
            function: function.into(),
            args: args.iter().map(|a| a.to_string()).collect(),
        };
        let mut commands = vec![
            Command { title: "Lock screen".into(), glyph: "\u{f033e}", call: call("lockscreen", "lock", &[]) },
            Command { title: "Power menu".into(), glyph: "\u{f0425}", call: call("powermenu", "toggle", &[]) },
            Command { title: "Screenshot".into(), glyph: "\u{f0104}", call: call("screenshot", "start", &[]) },
            Command { title: "Dashboard".into(), glyph: "\u{f056e}", call: call("dashboard", "open", &[]) },
            Command { title: "Theme switcher".into(), glyph: "\u{f03d8}", call: call("theme", "open", &[]) },
            Command {
                title: "Do not disturb".into(),
                glyph: "\u{f009b}",
                call: call("notifications", "toggleDnd", &[]),
            },
            Command { title: "Stay awake".into(), glyph: "\u{f0176}", call: call("idle", "toggle", &[]) },
        ];
        for page in crate::ui::control_center::Page::ALL {
            commands.push(Command {
                title: format!("Settings: {}", page.title()),
                glyph: page.glyph(),
                call: call("settings", "open", &[page.title()]),
            });
        }
        for theme in self.theme_names() {
            commands.push(Command { title: format!("Theme: {theme}"), glyph: "\u{f03d8}", call: call("theme", "set", &[&theme]) });
        }
        commands
    }

    /// A transparent layer over `output`, under the launcher, that takes
    /// the clicks outside it.
    fn make_catcher(&mut self, output: &str) -> Option<Catcher> {
        let qh = self.queue_handle.clone();
        let wl_output = self.launcher_surfaces.iter().find(|e| e.output_name == output)?.output.clone()?;
        let (pixels, viewporter) = self.idle_globals.pixels()?;
        let layer = self.layer_shell.create_layer_surface(
            &qh,
            self.compositor_state.create_surface(&qh),
            // Under the launcher's overlay layer, over windows.
            Layer::Top,
            Some("stratum-launcher-catcher"),
            Some(&wl_output),
        );
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        let viewport = viewporter.get_viewport(layer.wl_surface(), &qh, Quiet);
        let buffer = pixels.create_u32_rgba_buffer(0, 0, 0, 0, &qh, Quiet);
        layer.commit();
        Some(Catcher { layer, viewport, buffer })
    }

    /// A configure for the catcher. Returns false if `layer` is not it.
    pub(super) fn configure_catcher(&mut self, layer: &LayerSurface, width: u32, height: u32) -> bool {
        let Some(catcher) = self.launcher_catcher.as_ref().filter(|c| &c.layer == layer) else {
            return false;
        };
        catcher.viewport.set_destination(width.max(1) as i32, height.max(1) as i32);
        let wl = catcher.layer.wl_surface();
        wl.attach(Some(&catcher.buffer), 0, 0);
        wl.damage_buffer(0, 0, 1, 1);
        wl.commit();
        true
    }

    /// Whether `surface` is the catcher.
    pub(super) fn is_catcher(&self, surface: &wl_surface::WlSurface) -> bool {
        self.launcher_catcher.as_ref().is_some_and(|c| c.layer.wl_surface() == surface)
    }

    pub(super) fn catcher_closed(&mut self, layer: &LayerSurface) {
        if self.launcher_catcher.as_ref().is_some_and(|c| &c.layer == layer) {
            self.close_launcher();
        }
    }

    pub(super) fn render_launcher(&mut self, theme: &crate::config::Theme, dt: f32) -> Result<(), String> {
        let target = self.launcher.target().map(str::to_string);
        let rows = self.config.launcher.rows;
        for entry in &mut self.launcher_surfaces {
            if target.as_deref() != Some(entry.output_name.as_str()) {
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
                self.launcher.paint(&mut ctx, &mut list, rect, rows);
                entry.render_list(&self.gpu, self.display, &list, &mut self.text)?;
                entry.dirty = false;
            }
            if entry.mapped {
                entry.set_keyboard_exclusive(true);
            }
        }
        Ok(())
    }
}
