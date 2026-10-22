//! Application state and the SCTK handler implementations.
//!
//! One `App` owns the connection, the bound globals, and one layer surface per
//! output. It runs inside a calloop `EventLoop`; Wayland events are dispatched
//! by `calloop-wayland-source`, and anything that needs redrawing marks the
//! surface dirty for the next loop iteration.

use std::ptr::NonNull;

use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::registry_handlers;
use smithay_client_toolkit::seat::pointer::{
    PointerEvent, PointerEventKind, PointerHandler, BTN_LEFT, BTN_MIDDLE, BTN_RIGHT,
};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::seat::keyboard::{
    KeyEvent as KeyboardEvent, KeyboardHandler, Modifiers, RawModifiers,
};
use smithay_client_toolkit::seat::keyboard::Keysym;
use smithay_client_toolkit::activation::{ActivationHandler, ActivationState, RequestData};
use smithay_client_toolkit::shell::xdg::window::{Window, WindowConfigure, WindowDecorations, WindowHandler};
use smithay_client_toolkit::shell::xdg::XdgShell;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::{delegate_dispatch2, delegate_registry};
use tracing::{debug, info, warn};
use wayland_client::protocol::{wl_output, wl_pointer, wl_seat, wl_surface};
use wayland_client::{Connection, QueueHandle};

use super::surfaces::SurfaceEntry;
use super::display_ptr;
use crate::config::{Config, SidebarPosition};
use crate::ipc::{IpcCall, IpcError};
use crate::render::display_list::Color;
use crate::render::Gpu;
use crate::sys_state::SystemState;
use crate::text::TextEngine;
use crate::ui::sidebar::Sidebar;

use std::sync::Arc;

/// Fallback rail width, used before the configuration is read.
const DEFAULT_SIDEBAR_WIDTH: u32 = 40;

pub struct App {
    registry_state: RegistryState,
    output_state: OutputState,
    compositor_state: CompositorState,
    layer_shell: LayerShell,

    gpu: Gpu,
    display: NonNull<std::ffi::c_void>,
    surfaces: Vec<SurfaceEntry>,
    text: TextEngine,
    icons: crate::icons::IconTheme,
    config: Arc<Config>,
    state: Arc<SystemState>,
    sidebar: Sidebar,
    seat_state: SeatState,
    control_center: crate::ui::control_center::ControlCenter,
    actions: Arc<crate::actions::ActionQueue>,
    notifications: Option<Arc<crate::notifications::NotificationServer>>,
    tray: Option<Arc<crate::tray::Tray>>,
    osd: crate::osd::Osd,
    /// OSD overlay surfaces, one per output.
    osd_surfaces: Vec<SurfaceEntry>,
    toasts: crate::toasts::Toasts,
    toast_surfaces: Vec<SurfaceEntry>,
    /// The power menu overlay and its per-output surfaces.
    power_menu: crate::power_menu::PowerMenu,
    power_surfaces: Vec<SurfaceEntry>,
    /// A tray item's context menu, and its per-output surfaces.
    tray_menu: crate::tray_menu::TrayMenuOverlay,
    tray_menu_surfaces: Vec<SurfaceEntry>,
    /// Last applied menu geometry, so the layer is only reconfigured on change.
    /// Placement is part of the key: two menus with the same row count still
    /// open beside different icons.
    tray_menu_geometry: Option<((u32, u32), crate::tray_menu::Placement, Option<String>)>,
    /// Toast rects from the last paint, for click-to-dismiss.
    toast_hits: Vec<(crate::render::display_list::Rect, u32)>,
    /// Last frame time, for animation.
    last_frame: std::time::Instant,
    queue_handle: QueueHandle<App>,
    xdg_shell: XdgShell,
    activation: Option<ActivationState>,
    /// The control-centre window, created on first open.
    settings: Option<SettingsWindow>,
    /// Surface the pointer is currently over, and the last known position in
    /// that surface's logical coordinates.
    pointer: Option<(wl_surface::WlSurface, f32, f32)>,
    pointer_pressed: bool,
    /// Keyboard modifiers, updated by the compositor.
    modifiers: Modifiers,
    /// Where the config came from, and its modification time at load.
    config_path: Option<std::path::PathBuf>,
    config_mtime: Option<std::time::SystemTime>,
    last_reload_check: std::time::Instant,
    /// Poke used by worker threads to wake the event loop out of its poll.
    /// Held so the ping source's channel stays open for the shell's lifetime.
    #[allow(dead_code)]
    wake: Option<calloop::ping::Ping>,
    /// The IPC listener. Held so its socket file lives exactly as long as the
    /// shell; dropping it unlinks the socket.
    #[allow(dead_code)]
    ipc: Option<crate::ipc::Server>,

    #[allow(dead_code)]
    pub should_exit: bool,
}

impl App {
    /// Create an `App` and a calloop loop with the Wayland source installed.
    pub fn new() -> Result<(Self, calloop::EventLoop<'static, App>, Connection), String> {
        let conn = Connection::connect_to_env()
            .map_err(|err| format!("failed to connect to Wayland: {err}"))?;
        info!("connected to Wayland");

        let display = display_ptr(&conn)?;
        let gpu = pollster::block_on(Gpu::new())?;

        let (config, path) = Config::load().map_err(|err| err.to_string())?;
        match &path {
            Some(path) => info!("loaded configuration from {}", path.display()),
            None => info!("no configuration file found; using defaults"),
        }
        let config_mtime = path
            .as_ref()
            .and_then(|p| std::fs::metadata(p).ok())
            .and_then(|m| m.modified().ok());
        let config = Arc::new(config);
        let state = SystemState::new();
        let sidebar = Sidebar::new(Arc::clone(&config));
        let config_for_cc = Arc::clone(&config);

        // Enumerates the globals and hands us the queue they were collected on.
        let (globals, event_queue) = wayland_client::globals::registry_queue_init::<App>(&conn)
            .map_err(|err| format!("failed to enumerate globals: {err}"))?;
        let qh = event_queue.handle();

        let event_loop: calloop::EventLoop<'static, App> =
            calloop::EventLoop::try_new().map_err(|err| format!("calloop init failed: {err}"))?;
        let loop_handle = event_loop.handle();

        let output_state = OutputState::new(&globals, &qh);
        let seat_state = SeatState::new(&globals, &qh);
        let compositor_state = CompositorState::bind(&globals, &qh)
            .map_err(|err| format!("compositor unavailable: {err}"))?;
        let layer_shell = LayerShell::bind(&globals, &qh)
            .map_err(|err| format!("wlr-layer-shell unavailable: {err}"))?;
        let xdg_shell = XdgShell::bind(&globals, &qh)
            .map_err(|err| format!("xdg-shell unavailable: {err}"))?;
        // Optional: lets us hand focus to the settings window when opened.
        let activation = ActivationState::bind(&globals, &qh).ok();
        let registry_state = RegistryState::new(&globals);

        let mut app = App {
            registry_state,
            output_state,
            compositor_state,
            layer_shell,
            gpu,
            display,
            surfaces: Vec::new(),
            text: TextEngine::new(),
            icons: crate::icons::IconTheme::load(&["Papirus", "Adwaita", "breeze"]),
            config,
            state,
            sidebar,
            seat_state,
            control_center: crate::ui::control_center::ControlCenter::new(Arc::clone(&config_for_cc)),
            actions: crate::actions::ActionQueue::new(),
            notifications: None,
            tray: None,
            osd: crate::osd::Osd::new(),
            osd_surfaces: Vec::new(),
            toasts: crate::toasts::Toasts::new(),
            toast_surfaces: Vec::new(),
            power_menu: crate::power_menu::PowerMenu::new(),
            power_surfaces: Vec::new(),
            tray_menu: crate::tray_menu::TrayMenuOverlay::new(),
            tray_menu_surfaces: Vec::new(),
            tray_menu_geometry: None,
            toast_hits: Vec::new(),
            last_frame: std::time::Instant::now(),
            queue_handle: qh.clone(),
            xdg_shell,
            activation,
            settings: None,
            pointer: None,
            pointer_pressed: false,
            modifiers: Modifiers::default(),
            config_path: path,
            config_mtime,
            last_reload_check: std::time::Instant::now(),
            wake: None,
            ipc: None,
            should_exit: false,
        };

        // Worker threads (actions, pollers, D-Bus) have no way to signal the
        // event loop. Install a ping source so a state change wakes the loop
        // immediately instead of waiting for the poll timeout — this is what
        // makes a click feel instant rather than up to a second late.
        let (ping, ping_source) = calloop::ping::make_ping()
            .map_err(|err| format!("failed to create wake pipe: {err}"))?;
        event_loop
            .handle()
            .insert_source(ping_source, move |(), _, app: &mut App| {
                for entry in &mut app.surfaces {
                    entry.dirty = true;
                }
                if app.control_center.visible() {
                    if let Some(settings) = &mut app.settings {
                        settings.dirty = true;
                    }
                }
            })
            .map_err(|err| format!("failed to insert wake source: {err}"))?;
        // The waker is shared: every publisher and the action worker notify
        // through it. Keep one clone on the App so it stays alive.
        let wake_for_state = ping.clone();
        app.state.set_waker(Arc::new(move || wake_for_state.ping()));
        app.wake = Some(ping);

        app.start_ipc(&loop_handle);
        app.create_surfaces(&qh);
        app.start_monitors();



        // Hand the queue to calloop, which dispatches whenever the socket is
        // readable. The App itself lives in the loop's shared data.
        let source = smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource::new(
            conn.clone(),
            event_queue,
        );
        source
            .insert(loop_handle)
            .map_err(|err| format!("failed to insert Wayland source: {err}"))?;

        Ok((app, event_loop, conn))
    }

    /// How long the event loop may block before the next frame.
    ///
    /// Animations need a steady cadence; an idle shell should sleep as long as
    /// possible. Asking for a new frame every 16 ms all the time would show up
    /// immediately in idle CPU, so the timeout follows what is actually moving.
    pub fn frame_timeout(&self) -> std::time::Duration {
        // A redraw already queued means we want to paint right away.
        if self.sidebar.animating()
            || self.osd.visible()
            || self.toasts.animating()
            || self.control_center.animating()
        {
            return std::time::Duration::from_millis(16);
        }
        // The clock changes once a minute; a one-second wakeup is enough to
        // catch it without spinning.
        std::time::Duration::from_secs(1)
    }

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

        // Widget animations (the workspace strip's scroll, the clock's minute
        // hand) run on the frame clock. While anything is moving every surface
        // stays dirty, so the next loop iteration paints again.
        let animating = self.sidebar.tick(dt);
        if animating {
            for entry in &mut self.surfaces {
                entry.dirty = true;
            }
        }

        self.osd.set_metrics(&self.config.metrics.osd);
        self.toasts.set_metrics(&self.config.metrics.toasts);
        self.osd.observe_audio(&self.state);
        self.osd.observe_brightness(self.state.brightness.get());

        // Toasts follow the notification list and expire on their own timers.
        // `take_dirty` above already consumed the aggregate flag, so sync on any
        // state change rather than on the notification cell alone. While
        // do-not-disturb is on, existing cards still update and disappear with
        // their notifications, but no new card appears.
        if state_changed {
            self.toasts.set_muted(self.state.dnd.get());
            let list = self.state.notifications.get();
            self.toasts.sync(&list);
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
                        pressed: self.pointer_pressed,
                    };
                    let mut ctx = crate::ui::PaintCtx {
                        theme: &theme,
                        appearance: &self.config.appearance,
                        metrics: &self.config.metrics,
                        text: &mut self.text,
                        icons: &mut self.icons,
                        state: &self.state,
                        themes: &self.config.themes,
                        output_name: &output_name,
                        axis: self.config.sidebar.axis(),
                        dt,
                    };
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
                    crate::actions::SystemAction::DismissAll => {
                        if let Some(server) = &self.notifications {
                            let ids: Vec<u32> = self
                                .state
                                .notifications
                                .get()
                                .iter()
                                .map(|n| n.id)
                                .collect();
                            for id in ids {
                                server.close(id, 2);
                            }
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

        let theme = self.config.active_theme();

        // OSD overlay. It shares the sidebar's paint context but draws its own
        // display list, since it has nothing to do with the rail layout.
        // Once it has faded out, detach the buffer: a mapped surface with a
        // transparent buffer can still composite as black on some compositors,
        // which is the "OSD stays black" report.
        let osd_visible = self.osd.visible();
        for entry in &mut self.osd_surfaces {
            if !osd_visible {
                entry.unmap();
                continue;
            }
            if !entry.dirty || !entry.configured {
                continue;
            }
            let output_name = entry.output_name.clone();
            let mut ctx = crate::ui::PaintCtx {
                theme: &theme,
                appearance: &self.config.appearance,
                metrics: &self.config.metrics,
                text: &mut self.text,
                icons: &mut self.icons,
                state: &self.state,
                themes: &self.config.themes,
                output_name: &output_name,
                axis: self.config.sidebar.axis(),
                dt,
            };
            let rect = entry.logical_rect();
            let mut list = crate::render::display_list::DisplayList::new(Color::TRANSPARENT);
            self.osd.paint(&mut ctx, &mut list, rect);
            entry.render_list(&self.gpu, self.display, &list, &mut self.text)?;
            entry.dirty = false;
        }

        // Toast stack. Hits are collected so a click on a card can dismiss it.
        // An empty stack unmaps, for the same reason as the OSD.
        self.toast_hits.clear();
        let toasts_visible = !self.toasts.is_empty();
        for entry in &mut self.toast_surfaces {
            if !toasts_visible {
                entry.unmap();
                continue;
            }
            if !entry.dirty || !entry.configured {
                continue;
            }
            let output_name = entry.output_name.clone();
            let mut ctx = crate::ui::PaintCtx {
                theme: &theme,
                appearance: &self.config.appearance,
                metrics: &self.config.metrics,
                text: &mut self.text,
                icons: &mut self.icons,
                state: &self.state,
                themes: &self.config.themes,
                output_name: &output_name,
                axis: self.config.sidebar.axis(),
                dt,
            };
            let rect = entry.logical_rect();
            let mut list = crate::render::display_list::DisplayList::new(Color::TRANSPARENT);
            let hits = self.toasts.paint(&mut ctx, &mut list, rect);

            // The surface spans the full screen height so the stack can grow,
            // but only the cards themselves should take clicks. Everything
            // else must fall through to the window underneath. This has to be
            // set before the commit that `render_list` performs, or the region
            // only takes effect one frame late.
            let bounds = hits
                .iter()
                .fold(None::<crate::render::display_list::Rect>, |acc, (card, _)| {
                    Some(match acc {
                        None => *card,
                        Some(acc) => crate::render::display_list::union(acc, *card),
                    })
                });
            entry.set_input_region(&self.compositor_state, bounds);

            // Only the surface the pointer is over can receive a click, so
            // merging the stacks is safe: at most one output is interactive.
            self.toast_hits.extend(hits);
            entry.render_list(&self.gpu, self.display, &list, &mut self.text)?;
            entry.dirty = false;
        }

        // Power menu. Only the output it opened on is painted and given
        // keyboard focus; the rest stay unmapped and inert. Repaints happen on
        // the state changes that matter (open/close, selection change), not
        // every frame: forcing a redraw here made the labels shimmer.
        self.power_menu.set_metrics(&self.config.metrics.power_menu);
        let power_target = self.power_menu.target().map(str::to_string);
        for entry in &mut self.power_surfaces {
            let active = power_target
                .as_deref()
                .map(|name| name == entry.output_name)
                .unwrap_or(false);
            if !active {
                // Drop focus before unmapping so keys return to the window.
                entry.set_keyboard_exclusive(false);
                entry.unmap();
                continue;
            }
            if !entry.configured {
                continue;
            }
            if entry.dirty {
                tracing::debug!(
                    "power menu paint output={} selection={}",
                    entry.output_name,
                    self.power_menu.selection()
                );
                let output_name = entry.output_name.clone();
                let mut ctx = crate::ui::PaintCtx {
                    theme: &theme,
                    appearance: &self.config.appearance,
                    metrics: &self.config.metrics,
                    text: &mut self.text,
                    icons: &mut self.icons,
                    state: &self.state,
                    themes: &self.config.themes,
                    output_name: &output_name,
                    axis: self.config.sidebar.axis(),
                    dt,
                };
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

        // Tray context menu. It is sized to its contents, so the layer's size
        // and margin are set from the published menu whenever that changes; the
        // surface is unmapped when no menu is open.
        self.tray_menu.set_metrics(&self.config.metrics.tray_menu);
        let open_menu = self.state.tray_menu.get();
        let menu_target = self.tray_menu.target().map(str::to_string);
        let menu_size = self.tray_menu.size(open_menu.as_ref());
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
                    // other monitor.
                    let output_width = surface.output_logical_size.0;
                    let placement = if output_width > 0.0 {
                        crate::tray_menu::Placement::beside_rail(
                            self.config.sidebar.position,
                            self.config.sidebar.width as f32,
                            output_width,
                            self.config.metrics.tray_menu.width,
                            self.tray_menu.placement().y,
                        )
                    } else {
                        self.tray_menu.placement()
                    };
                    surface.layer.set_size(width, height);
                    surface.layer.set_margin(
                        placement.y.round() as i32,
                        self.config.metrics.tray_menu.margin_right,
                        self.config.metrics.tray_menu.margin_bottom,
                        placement.x.round() as i32,
                    );
                    surface.layer.commit();
                    // Changing size or margins starts a new configure cycle.
                    // Attaching a buffer before that configure arrives is a
                    // fatal `layerSurface was not configured` protocol error, so
                    // wait for it rather than painting this frame.
                    surface.configured = false;
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
                entry.set_keyboard_exclusive(false);
                entry.unmap();
                continue;
            }
            if !entry.configured {
                continue;
            }
            // Only repaint on a change. Without this the surface redrew on every
            // loop iteration, which made the text shimmer as glyphs re-rasterized
            // at slightly different subpixel positions.
            if !entry.dirty {
                continue;
            }
            let output_name = entry.output_name.clone();
            let mut ctx = crate::ui::PaintCtx {
                theme: &theme,
                appearance: &self.config.appearance,
                metrics: &self.config.metrics,
                text: &mut self.text,
                icons: &mut self.icons,
                state: &self.state,
                themes: &self.config.themes,
                output_name: &output_name,
                axis: self.config.sidebar.axis(),
                dt,
            };
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
            let mut ctx = crate::ui::PaintCtx {
                theme: &theme,
                appearance: &self.config.appearance,
                metrics: &self.config.metrics,
                text: &mut self.text,
                icons: &mut self.icons,
                state: &self.state,
                themes: &self.config.themes,
                output_name: &output_name,
                axis: self.config.sidebar.axis(),
                dt,
            };
            entry.render_with(
                &self.gpu,
                self.display,
                &mut ctx,
                &mut self.sidebar,
                &input,
            )?;
            entry.dirty = false;
        }
        Ok(())
    }

    /// Start the IPC listener and route requests through calloop.
    ///
    /// The listener runs on its own thread because a client may connect at any
    /// time. Requests that need UI state — opening a panel, switching the theme
    /// — are delivered to the event loop through a calloop channel, so they are
    /// handled on the same thread as every other state change.
    fn start_ipc(&mut self, loop_handle: &calloop::LoopHandle<'static, App>) {
        let (tx, channel) = calloop::channel::channel();
        self.ipc = crate::ipc::start(tx);
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
        use crate::ipc::{is_supported, IpcError};

        if !is_supported(&call.target, &call.function) {
            return Err(IpcError::Unsupported);
        }

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
            ("notifications", "clear") => {
                if let Some(server) = &self.notifications {
                    let ids: Vec<u32> =
                        self.state.notifications.get().iter().map(|n| n.id).collect();
                    for id in ids {
                        server.close(id, 2);
                    }
                }
            }
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

    /// The output the compositor currently has focused, by name.
    ///
    /// Popups use this so they open where the user is looking rather than on an
    /// arbitrary monitor. When the compositor has not reported a focused
    /// workspace (no Hyprland, or before the first event), fall back to the
    /// first output so the overlay still has somewhere to appear.
    fn focused_output(&self) -> String {
        let hypr = self.state.hypr.get();
        let named = hypr.focused_monitor.filter(|name| {
            self.surfaces.iter().any(|entry| &entry.output_name == name)
        });
        named
            .or_else(|| self.surfaces.first().map(|e| e.output_name.clone()))
            .unwrap_or_default()
    }

    /// Logical width of the focused output, for popup placement.
    ///
    /// Resolved by name through the bar surfaces: taking the first output from
    /// `OutputState` picks an arbitrary monitor, which is wrong whenever the
    /// monitors differ in size.
    fn focused_output_width(&self) -> f32 {
        let focused = self.focused_output();
        self.surfaces
            .iter()
            .find(|entry| entry.output_name == focused)
            .map(|entry| entry.output_logical_size.0)
            .unwrap_or(0.0)
    }

    /// Redraw every surface after a state change the UI thread caused.
    fn mark_all_dirty(&mut self) {
        for entry in &mut self.surfaces {
            entry.dirty = true;
        }
        for entry in &mut self.osd_surfaces {
            entry.dirty = true;
        }
        for entry in &mut self.toast_surfaces {
            entry.dirty = true;
        }
        if let Some(settings) = &mut self.settings {
            settings.dirty = true;
        }
    }

    /// Switch the active theme live. Returns false for an unknown name, which
    /// must not blank the shell.
    fn switch_theme(&mut self, name: &str) -> bool {
        if !self.config.themes.contains_key(name) {
            return false;
        }
        let mut updated = (*self.config).clone();
        updated.theme = name.to_string();
        self.config = Arc::new(updated);
        self.sidebar = Sidebar::new(Arc::clone(&self.config));
        // Keep the panel's page and open state across the swap.
        self.control_center.set_config(Arc::clone(&self.config));
        true
    }

    /// Start the system managers and compositor watcher.
    ///
    /// The managers publish into `SystemState` from their own threads, so the
    /// UI thread only ever reads. Hyprland changes wake the render loop by
    /// marking every surface dirty.
    fn start_monitors(&mut self) {
        use stratum_sys::managers;

        let publisher = SystemState::publisher(&self.state);
        self.actions.start_worker(Arc::clone(&self.state));

        // The managers' async entry points expect a Tokio runtime. The shell's
        // UI thread runs calloop, so give them a small dedicated runtime.
        managers::audio::initialize();
        let audio_publisher = publisher.clone();
        std::thread::Builder::new()
            .name("stratum-runtime".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(err) => {
                        tracing::error!("failed to start manager runtime: {err}");
                        return;
                    }
                };
                runtime.block_on(async move {
                    managers::audio::spawn_monitor(audio_publisher);
                    // Keep the runtime alive for the shell's lifetime.
                    std::future::pending::<()>().await;
                });
            })
            .ok();

        // Safety-net poller. User-initiated changes refresh their own domain
        // synchronously (see `actions`), so this only has to catch changes made
        // outside the shell. A slow cadence is fine and keeps idle CPU low.
        let state = Arc::clone(&self.state);
        let poll_ms = self.config.behavior.poll_ms.max(250);
        std::thread::Builder::new()
            .name("stratum-poll".into())
            .spawn(move || loop {
                state.net.set(managers::net::status());
                let wifi = managers::wifi::state();
                let wifi_enabled = wifi["enabled"].as_bool().unwrap_or(false);
                state.wifi.set(wifi);
                if wifi_enabled {
                    // Re-read the cache only: forcing a rescan here would put
                    // radio work on a timer for no reason.
                    state.wifi_networks.set(managers::wifi::list(false));
                }
                state.bluetooth.set(managers::bluetooth::status());
                state.battery.set(managers::battery::status());
                state.notify();
                std::thread::sleep(std::time::Duration::from_millis(poll_ms));
            })
            .ok();

        // Own the tray watcher when configured to.
        if self.config.behavior.tray {
            let sink_state = Arc::clone(&self.state);
            let sink: crate::tray::TraySink =
                Arc::new(move |items| {
                    if sink_state.tray.set(items) {
                        sink_state.notify();
                    }
                });
            let tray = crate::tray::Tray::new(sink);
            self.tray = Some(Arc::clone(&tray));
            // The action worker uses this for activation and menu requests.
            self.state.set_tray_handle(Arc::clone(&tray));
            std::thread::Builder::new()
                .name("stratum-tray".into())
                .spawn(move || {
                    let runtime = match tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    {
                        Ok(runtime) => runtime,
                        Err(err) => {
                            tracing::error!("failed to start tray runtime: {err}");
                            return;
                        }
                    };
                    if let Err(err) = runtime.block_on(crate::tray::serve(tray)) {
                        tracing::warn!("tray watcher unavailable: {err}");
                    }
                })
                .ok();
        }

        // Own org.freedesktop.Notifications when configured to. Off by default
        // so another daemon (Quickshell) can keep it during migration.
        if self.config.behavior.notifications {
            let sink_state = Arc::clone(&self.state);
            let sink: crate::notifications::NotificationSink =
                Arc::new(move |list| {
                    if sink_state.notifications.set(list) {
                        sink_state.notify();
                    }
                });
            let server = crate::notifications::NotificationServer::new(sink);
            self.notifications = Some(Arc::clone(&server));
            std::thread::Builder::new()
                .name("stratum-notifications".into())
                .spawn(move || {
                    let runtime = match tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    {
                        Ok(runtime) => runtime,
                        Err(err) => {
                            tracing::error!("failed to start notification runtime: {err}");
                            return;
                        }
                    };
                    if let Err(err) = runtime.block_on(server.serve()) {
                        tracing::error!("notification server failed: {err}");
                    }
                })
                .ok();
        }

        // Brightness has no push source, so it is polled. `brightnessctl` is
        // two short-lived processes per reading; at 350 ms that was the largest
        // remaining idle cost (and it spawned a process just to discover there
        // is no backlight). Read the sysfs node directly when possible and fall
        // back to one polling cadence that is cheap enough to keep.
        let brightness_state = Arc::clone(&self.state);
        std::thread::Builder::new()
            .name("stratum-brightness".into())
            .spawn(move || {
                let mut last: Option<i64> = None;
                loop {
                    if let Some(percent) = read_brightness_percent() {
                        if last != Some(percent) {
                            last = Some(percent);
                            brightness_state.brightness.set(percent);
                            brightness_state.notify();
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(1000));
                }
            })
            .ok();

        // Hyprland is edge-driven: re-read on each relevant event.
        let state = Arc::clone(&self.state);
        crate::hyprland::HyprState::spawn_watcher(move || {
            state.hypr.set(crate::hyprland::HyprState::read());
            state.notify();
        });
        // Seed once so the first frame is correct.
        self.state.hypr.set(crate::hyprland::HyprState::read());
    }

    /// Reload the configuration when the file changed on disk.
    ///
    /// Checked at most once a second so a burst of editor writes does not
    /// trigger repeated reloads. A parse error leaves the old config in place
    /// and logs the problem, so a typo cannot blank a running shell.
    fn maybe_reload_config(&mut self) {
        if !self.config.behavior.reload_on_change {
            return;
        }
        let Some(path) = self.config_path.clone() else {
            return;
        };
        if self.last_reload_check.elapsed() < std::time::Duration::from_secs(1) {
            return;
        }
        self.last_reload_check = std::time::Instant::now();

        let mtime = std::fs::metadata(&path).ok().and_then(|m| m.modified().ok());
        if mtime.is_none() || mtime == self.config_mtime {
            return;
        }

        // Only touch files that changed; re-read through the normal loader so
        // validation and defaults behave identically to startup.
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) => {
                tracing::warn!("config reload failed to read {}: {err}", path.display());
                return;
            }
        };
        match toml::from_str::<Config>(&text) {
            Ok(config) => {
                info!("reloaded configuration from {}", path.display());
                let width_changed = config.sidebar.width != self.config.sidebar.width
                    || config.sidebar.position != self.config.sidebar.position;
                self.config = Arc::new(config);
                self.sidebar = Sidebar::new(Arc::clone(&self.config));
                self.config_mtime = mtime;
                if width_changed {
                    info!("sidebar geometry changed; restart to apply the new anchorage");
                }
                for entry in &mut self.surfaces {
                    entry.dirty = true;
                }
            }
            Err(err) => {
                tracing::error!(
                    "config reload rejected for {}:\n{err}\nkeeping the previous configuration",
                    path.display()
                );
            }
        }
    }

    fn create_surfaces(&mut self, qh: &QueueHandle<Self>) {
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

            // Placement and width come from the configuration.
            let config = &self.config.sidebar;
            let (anchor, size) = match config.position {
                SidebarPosition::Left => (
                    Anchor::TOP | Anchor::LEFT | Anchor::BOTTOM,
                    (config.width, 0),
                ),
                SidebarPosition::Right => (
                    Anchor::TOP | Anchor::RIGHT | Anchor::BOTTOM,
                    (config.width, 0),
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

            self.surfaces.push(
                SurfaceEntry::new(layer, scale, output_name.clone()).with_output(output.clone())
                    .with_output_logical_size(output_logical_size),
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
            osd_layer.set_margin(0, 0, 72, 0);
            osd_layer.set_size(320, 94);
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
            toast_layer.set_margin(12, 12, 12, 12);
            toast_layer.set_size(344, 0);
            toast_layer.commit();
            self.toast_surfaces.push(
                SurfaceEntry::new(toast_layer, scale, output_name.clone()).with_output(output.clone())
                    .with_output_logical_size(output_logical_size),
            );

            // Power menu: a fullscreen overlay that is only mapped for the
            // output it opened on. It is created with keyboard interactivity
            // `None` and switched to `Exclusive` while open, so it cannot steal
            // keys from a window when hidden.
            let power_wl = self.compositor_state.create_surface(qh);
            let power_layer = self.layer_shell.create_layer_surface(
                qh,
                power_wl,
                Layer::Overlay,
                Some("stratum-powermenu"),
                Some(output),
            );
            power_layer.set_anchor(
                Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
            );
            power_layer.set_exclusive_zone(-1);
            power_layer.set_keyboard_interactivity(KeyboardInteractivity::None);
            power_layer.commit();
            self.power_surfaces.push(
                SurfaceEntry::new(power_layer, scale, output_name.clone())
                    .with_output(output.clone())
                    .with_output_logical_size(output_logical_size),
            );

            // Tray context menu: a small overlay pinned just inside the rail.
            // It is sized to its contents, so it starts hidden and is mapped
            // only while a menu is open.
            let menu_wl = self.compositor_state.create_surface(qh);
            let menu_layer = self.layer_shell.create_layer_surface(
                qh,
                menu_wl,
                Layer::Overlay,
                Some("stratum-traymenu"),
                Some(output),
            );
            menu_layer.set_anchor(Anchor::TOP | Anchor::LEFT);
            menu_layer.set_exclusive_zone(-1);
            menu_layer.set_margin(
                self.config.metrics.tray_menu.margin_top,
                self.config.metrics.tray_menu.margin_right,
                self.config.metrics.tray_menu.margin_bottom,
                self.config.metrics.tray_menu.margin_left,
            );
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

/// The control centre as a real toplevel window.
///
/// Using xdg-shell rather than a layer surface means the compositor owns the
/// window: it gets normal focus, workspace placement, alt-tab, and stacking,
/// so it feels like part of the shell rather than an overlay.
/// Read the backlight percentage.
///
/// Prefers the sysfs `brightness`/`max_brightness` nodes, which cost a single
/// file read. `brightnessctl` is only used when those are unavailable, and the
/// result is cached for the process lifetime because a machine without a
/// backlight will never grow one and each probe spawns two processes.
///
/// Returns `None` when no backlight backend is available, which is normal on
/// desktops; the OSD then simply never shows brightness.
fn read_brightness_percent() -> Option<i64> {
    static TOOL_AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

    if let Some(percent) = read_brightness_sysfs() {
        return Some(percent);
    }
    if !*TOOL_AVAILABLE.get_or_init(brightnessctl_available) {
        return None;
    }
    read_brightnessctl()
}

/// Read `brightness`/`max_brightness` from the first `/sys/class/backlight`
/// device that exposes both.
fn read_brightness_sysfs() -> Option<i64> {
    let entries = std::fs::read_dir("/sys/class/backlight").ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let current = std::fs::read_to_string(path.join("brightness")).ok()?;
        let max = std::fs::read_to_string(path.join("max_brightness")).ok()?;
        let current = current.trim().parse::<i64>().ok()?;
        let max = max.trim().parse::<i64>().ok()?;
        if max > 0 {
            return Some(((current as f64 * 100.0) / max as f64).round() as i64);
        }
    }
    None
}

fn brightnessctl_available() -> bool {
    std::process::Command::new("brightnessctl")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn read_brightnessctl() -> Option<i64> {
    let current = std::process::Command::new("brightnessctl")
        .arg("get")
        .output()
        .ok()?;
    let max = std::process::Command::new("brightnessctl")
        .arg("max")
        .output()
        .ok()?;
    if !current.status.success() || !max.status.success() {
        return None;
    }
    let current = String::from_utf8_lossy(&current.stdout).trim().parse::<i64>().ok()?;
    let max = String::from_utf8_lossy(&max.stdout).trim().parse::<i64>().ok()?;
    if max <= 0 {
        return None;
    }
    Some(((current as f64 * 100.0) / max as f64).round() as i64)
}

struct SettingsWindow {
    /// Declared before `window` so the swapchain drops before the `wl_surface`
    /// it references. See `SurfaceEntry` for why the order matters.
    renderer: Option<crate::render::SurfaceRenderer>,
    window: Window,
    scale: i32,
    width: u32,
    height: u32,
    configured: bool,
    dirty: bool,
    /// Whether we have asked for focus in response to an open request.
    activated: bool,
    /// Output the window prefers to open on, used for per-output display info.
    output: Option<wl_output::WlOutput>,
}

impl SettingsWindow {
    fn new(
        window: Window,
        scale: i32,
        output: Option<wl_output::WlOutput>,
        metrics: &crate::ui::style::WindowMetrics,
    ) -> Self {
        Self {
            renderer: None,
            window,
            scale,
            width: metrics.width,
            height: metrics.height,
            configured: false,
            dirty: true,
            activated: false,
            output,
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

    fn render(
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
    fn ensure_settings_window(&mut self) {
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
        let (scale, output) = self
            .surfaces
            .iter()
            .max_by_key(|s| s.scale)
            .map(|s| (s.scale, s.output.clone()))
            .unwrap_or((1, None));
        let metrics = self.config.metrics.window.clone();
        self.settings = Some(SettingsWindow::new(window, scale, output, &metrics));
    }

    fn apply_action(&mut self, action: crate::ui::Action) {
        use crate::ui::Action;
        match action {
            Action::OpenPanel(page) => self.control_center.toggle(page),
            Action::ClosePanel => self.control_center.close(),
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
                // Place the menu beside the rail, on the side the rail is on.
                // The output's logical width comes from its info, not from the
                // bar (which is only `sidebar.width` across); using the bar's
                // width put a right-hand menu at the left edge of the screen.
                let width = bar
                    .map(|entry| entry.output_logical_size.0)
                    .filter(|width| *width > 0.0)
                    .unwrap_or_else(|| self.focused_output_width());
                let placement = crate::tray_menu::Placement::beside_rail(
                    self.config.sidebar.position,
                    self.config.sidebar.width as f32,
                    width,
                    self.config.metrics.tray_menu.width,
                    y,
                );
                self.tray_menu.open(&output, placement);
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
        }
        for entry in &mut self.surfaces {
            entry.dirty = true;
        }
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

        if let Some(entry) = self.power_surfaces.iter_mut().find(|e| &e.layer == layer) {
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

        if let Some(entry) = self.tray_menu_surfaces.iter_mut().find(|e| &e.layer == layer) {
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
                Ok(_pointer) => info!("pointer capability acquired"),
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
        let mut clicked_at: Option<(wl_surface::WlSurface, f32, f32, crate::ui::MouseButton)> = None;
        let mut scroll_at: Option<(wl_surface::WlSurface, f32, f32, f32)> = None;
        let mut changed = false;

        for event in events {
            let (x, y) = (event.position.0 as f32, event.position.1 as f32);
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
            // The power menu owns the pointer while open; a tile click runs it.
            let on_power = self
                .power_surfaces
                .iter()
                .any(|p| p.layer.wl_surface() == &surface);
            // A click inside the open tray menu either runs an entry or
            // dismisses the menu.
            let on_tray_menu = self
                .tray_menu_surfaces
                .iter()
                .any(|t| t.layer.wl_surface() == &surface);
            if on_tray_menu {
                let menu = self.state.tray_menu.get();
                if let Some(menu) = menu {
                    if let Some(id) = self.tray_menu.on_click(point, &menu) {
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
                if let Some((_, id)) = self
                    .toast_hits
                    .iter()
                    .find(|(rect, _)| rect.contains(point))
                    .copied()
                {
                    self.toasts.dismiss(id);
                    if let Some(server) = &self.notifications {
                        server.close(id, 2);
                    }
                }
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
                if let Some(action) =
                    self.sidebar
                        .on_click_with(point, &output_name, &input, button)
                {
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
            }
        }

        if changed {
            for entry in &mut self.surfaces {
                entry.dirty = true;
            }
        }
    }
}

impl WindowHandler for App {
    fn request_close(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _window: &Window,
    ) {
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

impl ActivationHandler for App {
    type RequestUdata = ();

    fn new_token(&mut self, token: String, _data: &RequestData<()>) {
        // Apply the token to the settings window so the compositor focuses it.
        if let (Some(activation), Some(settings)) = (&self.activation, &self.settings) {
            activation.activate::<App>(settings.window.wl_surface(), token);
        }
    }
}

impl KeyboardHandler for App {
    fn enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wayland_client::protocol::wl_keyboard::WlKeyboard,
        _surface: &wl_surface::WlSurface,
        _serial: u32,
        _raw: &[u32],
        _keysyms: &[Keysym],
    ) {
    }

    fn leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wayland_client::protocol::wl_keyboard::WlKeyboard,
        _surface: &wl_surface::WlSurface,
        _serial: u32,
    ) {
    }

    fn press_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wayland_client::protocol::wl_keyboard::WlKeyboard,
        _serial: u32,
        event: KeyboardEvent,
    ) {
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

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState, SeatState];
}

delegate_dispatch2!(App);
delegate_registry!(App);
