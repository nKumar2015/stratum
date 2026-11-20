//! The overview and the window switcher: opening them, the pictures of
//! windows they show, and carrying out what they ask for.
//!
//! Both read the desktop from Hyprland when they open, and the overview
//! again whenever the bar's Hyprland state changes while it is open. Only
//! one is open at a time; they share the capture thread and the pictures,
//! which are dropped when either closes.

use smithay_client_toolkit::seat::pointer::{PointerEventKind, BTN_LEFT};

use super::*;
use crate::overview::OverviewAction;
use crate::window_capture::{Captured, Watch, WindowCapture};
use crate::window_switcher::{SwitcherAction, WindowSwitcher};

/// What the overview's helper threads finish.
pub(super) enum WindowsEvent {
    /// The wallpaper behind the workspaces, decoded for the session it was
    /// asked for.
    Wallpaper(u64, crate::wallpapers::Preview),
}

impl App {
    /// Bind the window export and route its pictures, and the overview's
    /// wallpaper, into the event loop.
    pub(super) fn start_window_capture(
        &mut self,
        conn: &Connection,
        globals: &wayland_client::globals::GlobalList,
        loop_handle: &calloop::LoopHandle<'static, App>,
    ) {
        let (tx, channel) = calloop::channel::channel::<Captured>();
        if let Err(err) = loop_handle.insert_source(channel, |event, _, app: &mut App| {
            if let calloop::channel::Event::Msg(captured) = event {
                app.window_captured(captured);
            }
        }) {
            warn!("failed to insert window capture source: {err}");
            return;
        }
        self.window_capture = WindowCapture::bind(conn, globals, tx);
        if self.window_capture.is_none() {
            info!("no hyprland_toplevel_export_manager_v1: windows are shown by their icons");
        }
        let (tx, channel) = calloop::channel::channel::<WindowsEvent>();
        if let Err(err) = loop_handle.insert_source(channel, |event, _, app: &mut App| {
            if let calloop::channel::Event::Msg(WindowsEvent::Wallpaper(session, preview)) = event {
                if app.overview.is_open() && session == app.overview_session {
                    app.overview_wallpaper = Some(preview);
                    app.mark_overview_dirty();
                }
            }
        }) {
            warn!("failed to insert overview event source: {err}");
        }
        self.windows_events = Some(tx);
    }

    fn window_captured(&mut self, captured: Captured) {
        let wanted = (self.overview.is_open() || self.window_switcher.is_open())
            && self.watched.contains(&captured.address);
        if !wanted {
            return;
        }
        self.thumbs.insert(captured.address, captured.thumb);
        self.mark_overview_dirty();
        self.mark_window_switcher_dirty();
    }

    /// Ask for pictures of `windows`.
    fn watch_windows(&mut self, windows: Vec<u64>, long_edge: u32, refresh_ms: u32) {
        self.thumbs.retain(|address, _| windows.contains(address));
        self.watched = windows.clone();
        if let Some(capture) = &mut self.window_capture {
            let refresh = (refresh_ms > 0).then(|| std::time::Duration::from_millis(refresh_ms as u64));
            capture.watch(Watch { windows, long_edge, refresh });
        }
    }

    /// Stop the pictures and free them.
    fn forget_windows(&mut self) {
        if let Some(capture) = &mut self.window_capture {
            capture.stop();
        }
        self.watched.clear();
        self.thumbs = std::collections::HashMap::new();
        crate::memory::release_freed();
    }

    pub(super) fn mark_overview_dirty(&mut self) {
        for entry in &mut self.overview_surfaces {
            entry.dirty = true;
        }
    }

    pub(super) fn mark_window_switcher_dirty(&mut self) {
        for entry in &mut self.switcher_surfaces {
            entry.dirty = true;
        }
    }

    /// Open the overview on `output`.
    pub(super) fn open_overview(&mut self, output: &str) {
        if self.window_switcher.is_open() {
            self.close_window_switcher();
        }
        let Some(desktop) = crate::hyprland::desktop() else {
            warn!("the overview needs Hyprland");
            return;
        };
        self.overview_session += 1;
        self.overview_hypr = Some(self.state.hypr.get());
        self.overview.open_on(output, desktop);
        let config = &self.config.overview;
        let (long_edge, refresh) = (config.thumbnail_px, config.refresh_ms);
        self.watch_windows(self.overview.windows(), long_edge, refresh);
        if self.config.overview.wallpaper {
            self.load_overview_wallpaper(output);
        }
        self.mark_overview_dirty();
    }

    /// Decode the wallpaper at about a workspace's size, on a thread.
    fn load_overview_wallpaper(&mut self, output: &str) {
        let path = self
            .state
            .wallpapers
            .get()
            .current
            .or_else(|| crate::lock::expand_home(&self.config.wallpapers.current));
        let (Some(path), Some(sender)) = (path, self.windows_events.clone()) else {
            return;
        };
        // A workspace is never wider than half the output, at the output's
        // scale.
        let (width, height) = self.output_logical_size(output);
        let scale = self
            .overview_surfaces
            .iter()
            .find(|e| e.output_name == output)
            .map_or(1, |e| e.scale.max(1));
        let long_edge = ((width.max(height) as u32 * scale as u32) / 2).clamp(256, 2048);
        let session = self.overview_session;
        let _ = std::thread::Builder::new().name("stratum-overview-wp".into()).spawn(move || {
            match crate::wallpapers::decode_preview(&path, crate::wallpapers::preview_key(&path), long_edge) {
                Ok(preview) => {
                    let _ = sender.send(WindowsEvent::Wallpaper(session, preview));
                }
                Err(err) => tracing::debug!("overview wallpaper {}: {err}", path.display()),
            }
        });
    }

    pub(super) fn close_overview(&mut self) {
        if !self.overview.is_open() {
            return;
        }
        self.overview.close();
        // Unmapped now rather than at the next render: a focus dispatched
        // next should not race the overlay's keyboard.
        for entry in &mut self.overview_surfaces {
            entry.unmap();
        }
        self.overview_wallpaper = None;
        self.overview_hypr = None;
        self.forget_windows();
    }

    pub(super) fn apply_overview_action(&mut self, action: OverviewAction) {
        match action {
            OverviewAction::None => return,
            OverviewAction::Redraw => {}
            OverviewAction::Close => self.close_overview(),
            OverviewAction::FocusWindow(address) => {
                self.close_overview();
                self.settle();
                if !crate::hyprland::focus_window(&format!("0x{address:x}")) {
                    warn!("could not focus window 0x{address:x}");
                }
            }
            OverviewAction::FocusWorkspace(id) => {
                self.close_overview();
                self.settle();
                crate::hyprland::focus_workspace(id);
            }
            OverviewAction::MoveWindow { address, workspace } => {
                if !crate::hyprland::move_window(address, workspace) {
                    warn!("could not move window 0x{address:x} to workspace {workspace}");
                }
                self.refresh_overview();
            }
        }
        self.mark_overview_dirty();
    }

    /// Read the desktop again while the overview is open.
    fn refresh_overview(&mut self) {
        if let Some(desktop) = crate::hyprland::desktop() {
            self.overview.set_desktop(desktop);
            let config = &self.config.overview;
            let (long_edge, refresh) = (config.thumbnail_px, config.refresh_ms);
            self.watch_windows(self.overview.windows(), long_edge, refresh);
            self.mark_overview_dirty();
        }
    }

    /// The overview's pointer: every press, motion and release in order,
    /// since a drag is all three.
    pub(super) fn overview_pointer(&mut self, kind: &PointerEventKind, x: f32, y: f32) {
        let point = crate::render::display_list::Point::new(x, y);
        let action = match kind {
            PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => self.overview.pointer_motion(point),
            PointerEventKind::Leave { .. } => self.overview.pointer_leave(),
            PointerEventKind::Press { button, .. } if *button == BTN_LEFT => self.overview.pointer_press(point),
            PointerEventKind::Release { button, .. } if *button == BTN_LEFT => self.overview.pointer_release(point),
            _ => OverviewAction::None,
        };
        self.apply_overview_action(action);
    }

    /// `switcher next|prev`: open on the previous window (or the oldest),
    /// or move on once open.
    pub(super) fn window_switcher_step(&mut self, forward: bool) {
        if self.window_switcher.is_open() {
            let action = self.window_switcher.step(if forward { 1 } else { -1 });
            self.apply_window_switcher_action(action);
            return;
        }
        if self.overview.is_open() {
            self.close_overview();
        }
        let Some(desktop) = crate::hyprland::desktop() else {
            warn!("the window switcher needs Hyprland");
            return;
        };
        let output = self.focused_output();
        let output_width = self.output_logical_size(&output).0 as f32;
        let count = desktop.windows.len();
        self.window_switcher.open_on(&output, desktop.windows, forward, output_width);
        if !self.window_switcher.is_open() {
            return;
        }
        let (width, height) = WindowSwitcher::size(&self.config.metrics.switcher, count, output_width);
        if let Some(entry) = self.switcher_surfaces.iter_mut().find(|e| e.output_name == output) {
            entry.layer.set_size(width, height);
        }
        let config = &self.config.switcher;
        let (long_edge, refresh) = (config.thumbnail_px, config.refresh_ms);
        self.watch_windows(self.window_switcher.addresses(), long_edge, refresh);
        self.mark_window_switcher_dirty();
    }

    pub(super) fn close_window_switcher(&mut self) {
        if !self.window_switcher.is_open() {
            return;
        }
        self.window_switcher.close();
        for entry in &mut self.switcher_surfaces {
            entry.unmap();
        }
        self.forget_windows();
    }

    pub(super) fn apply_window_switcher_action(&mut self, action: SwitcherAction) {
        match action {
            SwitcherAction::None => return,
            SwitcherAction::Redraw => {}
            SwitcherAction::Close => self.close_window_switcher(),
            SwitcherAction::Focus(address) => {
                debug!("window switcher: going to 0x{address:x}");
                self.close_window_switcher();
                self.settle();
                if !crate::hyprland::focus_window(&format!("0x{address:x}")) {
                    warn!("could not focus window 0x{address:x}");
                }
            }
        }
        self.mark_window_switcher_dirty();
    }

    /// The modifiers changed while the switcher may hold the keyboard.
    pub(super) fn window_switcher_modifiers(&mut self, modifiers: &Modifiers) {
        let held = modifiers.logo || modifiers.alt || modifiers.ctrl;
        debug!("window switcher: modifiers {modifiers:?}");
        let action = self.window_switcher.modifiers(held);
        self.apply_window_switcher_action(action);
    }

    /// The switcher's surface got the keyboard, with `keys` already down.
    /// A modifier let go before that is seen here, not in a modifiers event.
    pub(super) fn window_switcher_entered(&mut self, keys: &[Keysym]) {
        debug!("window switcher: keyboard entered with {keys:?}");
        let action = self.window_switcher.entered(keys);
        self.apply_window_switcher_action(action);
    }

    /// A key was let go while the switcher is open.
    pub(super) fn window_switcher_released(&mut self, keysym: Keysym) {
        let action = self.window_switcher.released(keysym);
        self.apply_window_switcher_action(action);
    }

    pub(super) fn tick_windows(&mut self, dt: f32) {
        self.overview.set_metrics(&self.config.metrics.overview);
        self.window_switcher.set_metrics(&self.config.metrics.switcher);
        if self.overview.is_open() {
            let animating = self.overview.animating();
            self.overview.tick(dt);
            if animating {
                self.mark_overview_dirty();
            }
            // The compositor reported a change: read the desktop again.
            let hypr = self.state.hypr.get();
            if self.overview_hypr.as_ref() != Some(&hypr) {
                self.overview_hypr = Some(hypr);
                self.refresh_overview();
            }
        }
        if self.window_switcher.is_open() {
            let animating = self.window_switcher.animating();
            self.window_switcher.tick(dt);
            if animating {
                self.mark_window_switcher_dirty();
            }
        }
    }

    pub(super) fn render_overview(&mut self, theme: &crate::config::Theme, dt: f32) -> Result<(), String> {
        let target = self.overview.target().map(str::to_string);
        for entry in &mut self.overview_surfaces {
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
                self.overview.paint(&mut ctx, &mut list, rect, &self.thumbs, self.overview_wallpaper.as_ref());
                entry.render_list(&self.gpu, self.display, &list, &mut self.text)?;
                entry.dirty = false;
            }
            if entry.mapped {
                entry.set_keyboard_exclusive(true);
            }
        }
        Ok(())
    }

    pub(super) fn render_window_switcher(&mut self, theme: &crate::config::Theme, dt: f32) -> Result<(), String> {
        let target = self.window_switcher.target().map(str::to_string);
        for entry in &mut self.switcher_surfaces {
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
                self.window_switcher.paint(&mut ctx, &mut list, rect, &self.thumbs);
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
