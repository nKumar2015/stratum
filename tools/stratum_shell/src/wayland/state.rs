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
use smithay_client_toolkit::session_lock::{
    SessionLock, SessionLockHandler, SessionLockState, SessionLockSurface,
    SessionLockSurfaceConfigure,
};
use smithay_client_toolkit::{delegate_dispatch2, delegate_registry};
use tracing::{debug, info, warn};
use wayland_client::protocol::{wl_output, wl_pointer, wl_seat, wl_surface};
use wayland_client::{Connection, QueueHandle};

use super::lock_surface::LockSurface;
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

/// The paint context for one surface. A macro, not a method: it borrows only
/// the fields it names, so the caller can still borrow the surface it paints.
macro_rules! paint_ctx {
    ($app:expr, $theme:expr, $output:expr, $dt:expr) => {
        crate::ui::PaintCtx {
            theme: $theme,
            appearance: &$app.config.appearance,
            metrics: &$app.config.metrics,
            text: &mut $app.text,
            icons: &mut $app.icons,
            state: &$app.state,
            themes: &$app.config.themes,
            output_name: $output,
            axis: $app.config.sidebar.axis(),
            dt: $dt,
        }
    };
}

mod input;
mod ipc;
mod lock;
mod monitors;
mod outputs;
mod overlays;
mod render;
mod screenshot;
mod settings_window;

use settings_window::SettingsWindow;

/// Results the lock screen's worker threads deliver to the event loop.
enum LockEvent {
    /// A PAM check finished.
    Auth(Result<(), crate::pam::AuthError>),
    /// Artwork finished decoding for this spec.
    Artwork(crate::lock::ArtworkSpec, crate::lock::Artwork),
}

/// What a screenshot worker finished.
enum ScreenshotEvent {
    /// `output`'s frozen frame, RGBA8.
    Frozen { output: String, width: u32, height: u32, pixels: Vec<u8> },
    FreezeFailed { error: String },
    /// The capture is at `path`, decoded for the viewer.
    Captured { path: String, mode: crate::screenshot::Mode, image: crate::screenshot_viewer::Image },
    CaptureFailed { error: String },
    /// A viewer action finished.
    Posted(Result<crate::screenshot_viewer::PostAction, String>),
    /// The portal answered Save As: a path, or `None` for a cancel.
    SaveAsChosen(Result<Option<String>, String>),
}

/// The answer to a polkit password, delivered from the agent's runtime.
struct PolkitEvent {
    cookie: String,
    verdict: crate::polkit_dialog::Verdict,
}

/// The PAM service and user the lock checks against, resolved when locking.
struct LockAuth {
    service: String,
    user: String,
}

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
    /// Saves the notification list and do-not-disturb for the next run, and
    /// the last snapshot sent, so unchanged state is not rewritten.
    notification_saver: Option<crate::notification_store::Saver>,
    saved_notifications: Option<crate::notification_store::Snapshot>,
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
    /// The lock screen. `session_lock` is only ever released through
    /// `SessionLock::unlock` after a successful PAM check, or after the
    /// compositor ends the lock itself: dropping it while locked is a protocol
    /// error, and the compositor then keeps the session locked.
    lock: crate::lock::LockScreen,
    session_lock_state: SessionLockState,
    session_lock: Option<SessionLock>,
    lock_surfaces: Vec<LockSurface>,
    lock_auth: Option<LockAuth>,
    /// Decoded lock artwork, and the spec of the newest decode requested.
    lock_art: crate::lock::Artwork,
    lock_art_spec: Option<crate::lock::ArtworkSpec>,
    lock_events: Option<calloop::channel::Sender<LockEvent>>,
    /// The polkit dialog, its per-output overlays, and the agent behind it.
    /// `polkit_runtime` runs the agent; password checks are spawned onto it.
    polkit_dialog: crate::polkit_dialog::PolkitDialog,
    polkit_surfaces: Vec<SurfaceEntry>,
    polkit_agent: Option<Arc<stratum_sys::managers::polkit::PolkitAgent>>,
    polkit_runtime: Option<tokio::runtime::Handle>,
    polkit_events: Option<calloop::channel::Sender<PolkitEvent>>,
    /// The theme switcher overlay and its per-output surfaces.
    theme_switcher: crate::theme_switcher::ThemeSwitcher,
    theme_surfaces: Vec<SurfaceEntry>,
    /// The dashboard overlay and its per-output surfaces. Its data comes
    /// from `SystemState::dashboard_feed`.
    dashboard: crate::dashboard::Dashboard,
    dashboard_surfaces: Vec<SurfaceEntry>,
    /// The screenshot overlay, its per-output surfaces, and the channel its
    /// `grim` workers answer on.
    screenshot: crate::screenshot::Screenshot,
    screenshot_surfaces: Vec<SurfaceEntry>,
    screenshot_events: Option<calloop::channel::Sender<ScreenshotEvent>>,
    /// The viewer a capture opens in, and its window while open.
    viewer: crate::screenshot_viewer::ScreenshotViewer,
    viewer_window: Option<super::viewer_window::ViewerWindow>,
    /// Hover menus beside the status icons, one overlay surface per output.
    hover_menus: crate::ui::hover_menu::HoverMenus,
    hover_surfaces: Vec<SurfaceEntry>,
    /// Size and placement last applied to the hover surface, and the output.
    hover_geometry: Option<(u32, i32, String)>,
    /// A press landed on the hover menu since its last paint.
    hover_clicked: bool,
    /// When the pointer left both the open tray menu and its icon.
    tray_menu_left_at: Option<std::time::Instant>,
    /// A press landed on the settings window since its last paint. Its pages
    /// find clicks as a pressed edge between paints, so a press and release
    /// arriving together (a quick tap) would otherwise never be seen.
    settings_press: bool,
    /// Last frame time, for animation.
    last_frame: std::time::Instant,
    queue_handle: QueueHandle<App>,
    xdg_shell: XdgShell,
    activation: Option<ActivationState>,
    /// `wp_cursor_shape_v1`, and its device for the pointer once there is one.
    cursor_shapes: Option<smithay_client_toolkit::seat::pointer::cursor_shape::CursorShapeManager>,
    cursor_shape_device: Option<
        smithay_client_toolkit::reexports::protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::WpCursorShapeDeviceV1,
    >,
    /// The control-centre window, created on first open.
    settings: Option<SettingsWindow>,
    /// Surface the pointer is currently over, and the last known position in
    /// that surface's logical coordinates.
    pointer: Option<(wl_surface::WlSurface, f32, f32)>,
    pointer_pressed: bool,
    /// Keyboard modifiers, updated by the compositor.
    modifiers: Modifiers,
    /// The surface holding keyboard focus, for routing keys to the settings
    /// window's text fields only when it is the one being typed into.
    keyboard_focus: Option<wl_surface::WlSurface>,
    /// Where the config comes from, and the user file's stamp at load.
    config_source: crate::config::ConfigSource,
    config_stamp: Option<(std::path::PathBuf, std::time::SystemTime)>,
    last_reload_check: std::time::Instant,
    /// The theme picked live, while it overrides the config's `theme`.
    theme_choice: Option<crate::theme_choice::Saved>,
    /// Poke used by worker threads to wake the event loop out of its poll.
    /// Held so the ping source's channel stays open for the shell's lifetime.
    _wake: Option<calloop::ping::Ping>,
    /// The IPC listener. Held so its socket file lives exactly as long as the
    /// shell; dropping it unlinks the socket.
    _ipc: Option<crate::ipc::Server>,
}

impl App {
    /// Create an `App` and a calloop loop with the Wayland source installed.
    pub fn new() -> Result<(Self, calloop::EventLoop<'static, App>, Connection), String> {
        let conn = Connection::connect_to_env()
            .map_err(|err| format!("failed to connect to Wayland: {err}"))?;
        info!("connected to Wayland");

        let display = display_ptr(&conn)?;

        let config_source = crate::config::ConfigSource::locate().map_err(|err| err.to_string())?;
        let config_stamp = config_source.user_stamp();
        let mut config = match config_source.load() {
            Ok(config) => config,
            // A broken user file must not leave the session without a bar.
            Err(err) => {
                tracing::error!("{err}\nstarting with the defaults alone");
                config_source.load_defaults().map_err(|err| err.to_string())?
            }
        };
        let saved_theme = crate::theme_choice::load();
        let had_saved_theme = saved_theme.is_some();
        let theme_choice = crate::theme_choice::apply(&mut config, saved_theme);
        if had_saved_theme && theme_choice.is_none() {
            crate::theme_choice::store(None);
        }
        info!(
            "loaded configuration: {} with {}",
            config_source.defaults.display(),
            match (&config_source.user, &config_stamp) {
                (Some(user), Some(_)) => user.display().to_string(),
                _ => "no user file".to_string(),
            }
        );
        let gpu = pollster::block_on(Gpu::new(config.render.gpu))?;
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
        // Optional: the screenshot overlay shows a crosshair cursor with it.
        let cursor_shapes =
            smithay_client_toolkit::seat::pointer::cursor_shape::CursorShapeManager::bind(&globals, &qh).ok();
        // Optional too: without ext-session-lock, `lockscreen lock` answers
        // unsupported and the CLI leaves locking to another client.
        let session_lock_state = SessionLockState::new(&globals, &qh);
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
            notification_saver: None,
            saved_notifications: None,
            tray: None,
            osd: crate::osd::Osd::new(&config_for_cc.metrics.osd),
            osd_surfaces: Vec::new(),
            toasts: crate::toasts::Toasts::new(&config_for_cc.metrics.toasts),
            toast_surfaces: Vec::new(),
            power_menu: crate::power_menu::PowerMenu::new(&config_for_cc.metrics.power_menu),
            power_surfaces: Vec::new(),
            tray_menu: crate::tray_menu::TrayMenuOverlay::new(&config_for_cc.metrics.tray_menu),
            tray_menu_surfaces: Vec::new(),
            tray_menu_geometry: None,
            lock: crate::lock::LockScreen::new(&config_for_cc.metrics.lock),
            session_lock_state,
            session_lock: None,
            lock_surfaces: Vec::new(),
            lock_auth: None,
            lock_art: crate::lock::Artwork::default(),
            lock_art_spec: None,
            lock_events: None,
            polkit_dialog: crate::polkit_dialog::PolkitDialog::new(&config_for_cc.metrics.polkit),
            polkit_surfaces: Vec::new(),
            polkit_agent: None,
            polkit_runtime: None,
            polkit_events: None,
            theme_switcher: crate::theme_switcher::ThemeSwitcher::new(&config_for_cc.metrics.theme_switcher),
            theme_surfaces: Vec::new(),
            dashboard: crate::dashboard::Dashboard::new(&config_for_cc.metrics.dashboard, &config_for_cc.dashboard),
            dashboard_surfaces: Vec::new(),
            screenshot: crate::screenshot::Screenshot::new(&config_for_cc.metrics.screenshot),
            screenshot_surfaces: Vec::new(),
            screenshot_events: None,
            viewer: crate::screenshot_viewer::ScreenshotViewer::new(
                &config_for_cc.metrics.screenshot_viewer,
                &config_for_cc.screenshot,
            ),
            viewer_window: None,
            hover_menus: crate::ui::hover_menu::HoverMenus::new(),
            hover_surfaces: Vec::new(),
            hover_geometry: None,
            hover_clicked: false,
            tray_menu_left_at: None,
            settings_press: false,
            last_frame: std::time::Instant::now(),
            queue_handle: qh.clone(),
            xdg_shell,
            activation,
            cursor_shapes,
            cursor_shape_device: None,
            settings: None,
            pointer: None,
            pointer_pressed: false,
            modifiers: Modifiers::default(),
            keyboard_focus: None,
            config_source,
            config_stamp,
            last_reload_check: std::time::Instant::now(),
            theme_choice,
            _wake: None,
            _ipc: None,
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
        app._wake = Some(ping);

        app.start_ipc(&loop_handle);
        app.start_lock_events(&loop_handle);
        app.start_screenshot_events(&loop_handle);
        app.start_polkit_events(&loop_handle);
        app.create_surfaces(&qh);
        app.start_monitors();
        // Decode the lock artwork now, off-thread, so the wallpaper is already
        // there the first time the screen locks.
        app.load_lock_art();

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
            || self.dashboard.animating()
        {
            return std::time::Duration::from_millis(16);
        }
        // A hover-menu timer is running; it needs to fire close to on time,
        // not on the next one-second tick.
        if self.hover_menus.intent.pending() || self.tray_menu_left_at.is_some() {
            return std::time::Duration::from_millis(50);
        }
        // The clock changes once a minute; a one-second wakeup is enough to
        // catch it without spinning. A running timer wakes when its face
        // changes, or (unseen) when it ends.
        let idle = std::time::Duration::from_secs(1);
        let timer = self.dashboard.timer_wake(std::time::Instant::now());
        timer.map_or(idle, |wait| wait.min(idle))
    }

    /// `output`'s top-left in global layout coordinates, which `grim` and
    /// window geometry use.
    fn output_origin(&self, output_name: &str) -> (i32, i32) {
        self.screenshot_surfaces
            .iter()
            .find(|e| e.output_name == output_name)
            .and_then(|e| e.output.as_ref())
            .and_then(|o| self.output_state.info(o))
            .and_then(|info| info.logical_position)
            .unwrap_or((0, 0))
    }

    /// `output`'s size in logical pixels, or `(0, 0)` while unknown.
    fn output_logical_size(&self, output_name: &str) -> (i32, i32) {
        self.screenshot_surfaces
            .iter()
            .find(|e| e.output_name == output_name)
            .and_then(|e| e.output.as_ref())
            .and_then(|o| self.output_state.info(o))
            .and_then(|info| info.logical_size)
            .unwrap_or((0, 0))
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

    /// Theme names in display order (the config's map is sorted).
    fn theme_names(&self) -> Vec<String> {
        self.config.themes.keys().cloned().collect()
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
