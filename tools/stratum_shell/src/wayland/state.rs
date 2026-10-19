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
    PointerEvent, PointerEventKind, PointerHandler, BTN_LEFT,
};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
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
    /// Where the config came from, and its modification time at load.
    config_path: Option<std::path::PathBuf>,
    config_mtime: Option<std::time::SystemTime>,
    last_reload_check: std::time::Instant,

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
            toast_hits: Vec::new(),
            last_frame: std::time::Instant::now(),
            queue_handle: qh.clone(),
            xdg_shell,
            activation,
            settings: None,
            pointer: None,
            pointer_pressed: false,
            config_path: path,
            config_mtime,
            last_reload_check: std::time::Instant::now(),
            should_exit: false,
        };

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
        self.osd.observe_audio(&self.state);
        self.osd.observe_brightness(self.state.brightness.get());

        // Toasts follow the notification list and expire on their own timers.
        // `take_dirty` above already consumed the aggregate flag, so sync on any
        // state change rather than on the notification cell alone.
        if state_changed {
            let list = self.state.notifications.get();
            self.toasts.sync(&list);
        }
        let expired = self.toasts.tick(dt);
        if !expired.is_empty() {
            if let Some(server) = &self.notifications {
                for id in expired {
                    server.close(id, 1);
                }
            }
        }
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
        if self.control_center.visible() {
            let theme = self.config.active_theme();
            if let Some(mut window) = self.settings.take() {
                if window.configured {
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
                        text: &mut self.text,
                        state: &self.state,
                        output_name: &output_name,
                        axis: self.config.sidebar.axis(),
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
                        if self.config.themes.contains_key(&name) {
                            let mut updated = (*self.config).clone();
                            updated.theme = name.clone();
                            self.config = Arc::new(updated);
                            self.sidebar = crate::ui::sidebar::Sidebar::new(Arc::clone(&self.config));
                            // Keep the panel's page and open state across the swap.
                            self.control_center.set_config(Arc::clone(&self.config));
                            for entry in &mut self.surfaces {
                                entry.dirty = true;
                            }
                            for entry in &mut self.osd_surfaces {
                                entry.dirty = true;
                            }
                            for entry in &mut self.toast_surfaces {
                                entry.dirty = true;
                            }
                        }
                    }
                    other => self.actions.push(other),
                }
            }
        }

        let theme = self.config.active_theme();
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
                text: &mut self.text,
                state: &self.state,
                output_name: &output_name,
                axis: self.config.sidebar.axis(),
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

    /// Start the system managers and compositor watcher.
    ///
    /// The managers publish into `SystemState` from their own threads, so the
    /// UI thread only ever reads. Hyprland changes wake the render loop by
    /// marking every surface dirty.
    fn start_monitors(&mut self) {
        use stratum_sys::managers;

        let publisher = SystemState::publisher(&self.state);
        self.actions.start_worker();

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

        let state = Arc::clone(&self.state);
        std::thread::Builder::new()
            .name("stratum-poll".into())
            .spawn(move || {
                let poll = std::time::Duration::from_secs(3);
                loop {
                    state.net.set(managers::net::status());
                    state.bluetooth.set(managers::bluetooth::status());
                    state.battery.set(managers::battery::status());
                    std::thread::sleep(poll);
                }
            })
            .ok();

        // Own the tray watcher when configured to.
        if self.config.behavior.tray {
            let sink_state = Arc::clone(&self.state);
            let sink: crate::tray::TraySink =
                Arc::new(move |items| sink_state.tray.set(items));
            let tray = crate::tray::Tray::new(sink);
            self.tray = Some(Arc::clone(&tray));
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
                Arc::new(move |list| sink_state.notifications.set(list));
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

        // Brightness has no push source; poll it slowly. The reading is stored
        // in SystemState so the OSD can react to changes.
        let brightness_state = Arc::clone(&self.state);
        std::thread::Builder::new()
            .name("stratum-brightness".into())
            .spawn(move || loop {
                if let Some(percent) = read_brightness_percent() {
                    brightness_state.brightness.set(percent);
                }
                std::thread::sleep(std::time::Duration::from_millis(350));
            })
            .ok();

        // Hyprland is edge-driven: re-read on each relevant event.
        let state = Arc::clone(&self.state);
        crate::hyprland::HyprState::spawn_watcher(move || {
            state.hypr.set(crate::hyprland::HyprState::read());
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

        for output in outputs {
            let info = self.output_state.info(&output);
            // SCTK reports an integer scale. For fractional outputs (Hyprland
            // reports 1.6) an integer buffer scale of 2 is correct for sharpness;
            // the compositor downsamples the buffer to the logical size.
            let scale = info.as_ref().map(|i| i.scale_factor).unwrap_or(1).max(1);
            let output_name = info.as_ref().map(|i| i.model.clone()).unwrap_or_default();

            let wl_surface = self.compositor_state.create_surface(qh);
            let layer = self.layer_shell.create_layer_surface(
                qh,
                wl_surface,
                Layer::Top,
                Some("stratum-sidebar"),
                Some(&output),
            );

            // Placement and width come from the configuration.
            let width = self.config.sidebar.width.max(1);
            let config = &self.config.sidebar;
            let vertical = config.is_vertical();
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
            let _ = vertical;
            layer.set_anchor(anchor);
            layer.set_exclusive_zone(match config.position {
                SidebarPosition::Top | SidebarPosition::Bottom => config.height as i32,
                _ => config.width as i32,
            });
            layer.set_margin(0, 0, 0, 0);
            layer.set_keyboard_interactivity(KeyboardInteractivity::None);
            layer.set_size(size.0, size.1);
            layer.commit();

            self.surfaces.push(SurfaceEntry::new(layer, scale, output_name.clone()));

            // OSD: an overlay-layer surface anchored bottom-centre with no
            // exclusive zone, so it floats above windows without reserving
            // space. It takes no input.
            let osd_wl = self.compositor_state.create_surface(qh);
            let osd_layer = self.layer_shell.create_layer_surface(
                qh,
                osd_wl,
                Layer::Overlay,
                Some("stratum-osd"),
                Some(&output),
            );
            osd_layer.set_anchor(Anchor::BOTTOM);
            osd_layer.set_exclusive_zone(-1);
            osd_layer.set_keyboard_interactivity(KeyboardInteractivity::None);
            osd_layer.set_margin(0, 0, 72, 0);
            osd_layer.set_size(320, 94);
            osd_layer.commit();
            self.osd_surfaces.push(SurfaceEntry::new(osd_layer, scale, output_name.clone()));

            // Toast stack: overlay layer, top-right, full height, no exclusive
            // zone. It ignores input so clicks reach the window underneath.
            let toast_wl = self.compositor_state.create_surface(qh);
            let toast_layer = self.layer_shell.create_layer_surface(
                qh,
                toast_wl,
                Layer::Overlay,
                Some("stratum-toasts"),
                Some(&output),
            );
            // A zero height requires anchoring both top and bottom; the toast
            // column then sizes itself between the margins.
            toast_layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::RIGHT);
            toast_layer.set_exclusive_zone(-1);
            toast_layer.set_keyboard_interactivity(KeyboardInteractivity::None);
            toast_layer.set_margin(12, 12, 12, 12);
            toast_layer.set_size(344, 0);
            toast_layer.commit();
            self.toast_surfaces.push(SurfaceEntry::new(toast_layer, scale, output_name));

            debug!("created layer surface on output (scale {scale}x)");
        }
    }
}

/// The control centre as a real toplevel window.
///
/// Using xdg-shell rather than a layer surface means the compositor owns the
/// window: it gets normal focus, workspace placement, alt-tab, and stacking,
/// so it feels like part of the shell rather than an overlay.
/// Read the backlight percentage via `brightnessctl`.
///
/// Returns `None` when no backlight backend is available, which is normal on
/// desktops; the OSD then simply never shows brightness.
fn read_brightness_percent() -> Option<i64> {
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
    window: Window,
    renderer: Option<crate::render::SurfaceRenderer>,
    scale: i32,
    width: u32,
    height: u32,
    configured: bool,
    dirty: bool,
    /// Whether we have asked for focus in response to an open request.
    activated: bool,
}

impl SettingsWindow {
    fn new(window: Window, scale: i32) -> Self {
        Self {
            window,
            renderer: None,
            scale,
            width: 520,
            height: 640,
            configured: false,
            dirty: true,
            activated: false,
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
        window.set_min_size(Some((360, 320)));
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
        let scale = self
            .surfaces
            .iter()
            .map(|s| s.scale)
            .max()
            .unwrap_or(1);
        self.settings = Some(SettingsWindow::new(window, scale));
    }

    fn apply_action(&mut self, action: crate::ui::Action) {
        use crate::ui::Action;
        match action {
            Action::OpenPanel(page) => self.control_center.toggle(page),
            Action::ClosePanel => self.control_center.close(),
            Action::ActivateTray(id) => {
                // Activation needs the tray's bus connection; hand it to the
                // running watcher through the action queue.
                tracing::debug!("tray activate requested: {id}");
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
        _output: wl_output::WlOutput,
    ) {
        // Outputs are enumerated at startup in M1; hotplug handling lands with
        // the sidebar work in M4.
        let _ = qh;
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
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
                Ok(pointer) => info!("pointer capability acquired"),
                Err(err) => tracing::warn!("failed to create pointer: {err}"),
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
        let mut clicked_at: Option<(wl_surface::WlSurface, f32, f32)> = None;
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
                    if button == BTN_LEFT {
                        self.pointer_pressed = true;
                        clicked_at = Some((event.surface.clone(), x, y));
                        changed = true;
                    }
                }
                PointerEventKind::Release { button, .. } => {
                    if button == BTN_LEFT {
                        self.pointer_pressed = false;
                        changed = true;
                    }
                }
                PointerEventKind::Axis { .. } => {}
            }
        }

        if let Some((surface, x, y)) = clicked_at {
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
            if on_toast {
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
                        changed = true;
                    }
                }
            } else if let Some(action) = self.sidebar.on_click(point, &input) {
                self.apply_action(action);
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
        let width = configure.new_size.0.map(|v| v.get()).unwrap_or(520);
        let height = configure.new_size.1.map(|v| v.get()).unwrap_or(640);
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

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState, SeatState];
}

delegate_dispatch2!(App);
delegate_registry!(App);
