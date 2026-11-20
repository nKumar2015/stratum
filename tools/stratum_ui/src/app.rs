//! A window of its own, for Stratum's apps: one xdg toplevel, drawn by the
//! shared renderer, with the keyboard (repeat included), the pointer, and
//! the theme and metrics the shell uses, reloaded live when the config or
//! the picked theme changes.
//!
//! An app implements [`App`]; [`run`] owns the connection and the loop. One
//! window per process: a dialog is its own process too.

use std::ptr::NonNull;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop::{self, EventLoop};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::registry_handlers;
use smithay_client_toolkit::seat::keyboard::{KeyEvent, KeyboardHandler, Modifiers, RawModifiers};
pub use smithay_client_toolkit::seat::keyboard::Keysym;
use smithay_client_toolkit::seat::pointer::cursor_shape::CursorShapeManager;
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind, PointerHandler, BTN_LEFT, BTN_RIGHT};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::xdg::window::{Window, WindowConfigure, WindowDecorations, WindowHandler};
use smithay_client_toolkit::shell::xdg::XdgShell;
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::{delegate_dispatch2, delegate_registry};
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::{wl_output, wl_pointer, wl_seat, wl_surface};
use wayland_client::{Connection, QueueHandle};

use crate::config::{AppearanceConfig, Config, ConfigSource, Theme};
use crate::icons::IconTheme;
use crate::render::display_list::{Color, DisplayList, Rect};
use crate::render::{Gpu, SurfaceRenderer};
use crate::style::Metrics;
use crate::text::TextEngine;
use crate::widgets::{InputState, Ui};

/// Two presses this close in time and space are a double click.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
const DOUBLE_CLICK_SLOP: f32 = 4.0;
/// How often the config and the picked theme are looked at.
const RELOAD_EVERY: Duration = Duration::from_secs(1);

/// What an app paints with: the shell's theme and metrics, and its own text
/// and icon engines.
pub struct AppUi {
    pub config: Arc<Config>,
    pub theme: Theme,
    pub text: TextEngine,
    pub icons: IconTheme,
}

impl Ui for AppUi {
    fn theme(&self) -> &Theme {
        &self.theme
    }
    fn appearance(&self) -> &AppearanceConfig {
        &self.config.appearance
    }
    fn metrics(&self) -> &Metrics {
        &self.config.metrics
    }
    fn text(&mut self) -> &mut TextEngine {
        &mut self.text
    }
    fn icons(&mut self) -> &mut IconTheme {
        &mut self.icons
    }
}

/// A key press (or repeat), with the modifiers held.
#[derive(Clone, Debug)]
pub struct Key {
    pub keysym: Keysym,
    /// What it types, if anything.
    pub text: Option<String>,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub repeat: bool,
}

/// The pointer for one paint: where it is, and what happened since the
/// last paint. Presses are latched until a paint has seen them.
#[derive(Clone, Debug, Default)]
pub struct Pointer {
    pub input: InputState,
    /// The primary button went down.
    pub clicked: bool,
    /// ... and it was the second click of a double click.
    pub double: bool,
    /// The secondary button went down (a context menu).
    pub secondary: bool,
    /// The primary button came up.
    pub released: bool,
    pub ctrl: bool,
    pub shift: bool,
}

/// Wakes the loop from another thread, so the app's [`App::poll`] runs and
/// the window repaints.
#[derive(Clone)]
pub struct Waker(calloop::ping::Ping);

impl Waker {
    pub fn wake(&self) {
        self.0.ping();
    }
}

/// What an app asks after a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Quit,
}

pub trait App {
    /// Called once, before the first paint, with a way for threads to wake
    /// the loop.
    fn start(&mut self, _waker: Waker) {}
    /// The window title, read after every paint.
    fn title(&self) -> String;
    /// Paint the window: `area` is all of it, in logical pixels.
    fn paint(&mut self, ui: &mut AppUi, list: &mut DisplayList, area: Rect, pointer: &Pointer);
    fn key(&mut self, ui: &mut AppUi, key: &Key) -> Flow;
    /// The wheel at `point`; `delta` is in logical pixels, positive moving
    /// the content up.
    fn scroll(&mut self, _point: (f32, f32), _delta: f32) {}
    /// Advance animations; returns whether any runs.
    fn tick(&mut self, _dt: f32) -> bool {
        false
    }
    /// Take what background threads finished; returns whether to repaint.
    fn poll(&mut self) -> bool {
        false
    }
    /// Whether to quit now (a dialog that was answered).
    fn done(&self) -> bool {
        false
    }
    /// The window's close button: returns whether to quit.
    fn close_requested(&mut self) -> bool {
        true
    }
}

/// The window to open.
pub struct Options {
    pub app_id: String,
    pub title: String,
    /// Logical size before the compositor says otherwise.
    pub size: (u32, u32),
    pub min_size: (u32, u32),
}

/// The defaults and the user's file, the settings window's overrides, the
/// user's custom themes and the theme picked live, as the shell loads them.
pub fn load_config(source: &ConfigSource) -> Result<Config, String> {
    let mut config = match source.load() {
        Ok(config) => config,
        Err(err) => {
            tracing::error!("{err}\nusing the defaults alone");
            source.load_defaults().map_err(|e| e.to_string())?
        }
    };
    crate::custom_themes::merge_into(&mut config);
    crate::theme_choice::apply(&mut config, crate::theme_choice::load());
    Ok(config)
}

/// The theme, falling back to the first valid one.
fn theme_of(config: &Config) -> Theme {
    config.active_theme()
}

/// What changes the look: the user's config file and the live theme pick.
fn look_stamp(source: &ConfigSource) -> (Option<(std::path::PathBuf, SystemTime)>, Option<SystemTime>, Option<SystemTime>) {
    let mtime = |name: &str| {
        crate::paths::state_file(name).and_then(|p| std::fs::metadata(p).ok()).and_then(|m| m.modified().ok())
    };
    let themes_dir = crate::paths::state_file("themes").and_then(|p| std::fs::metadata(p).ok()).and_then(|m| m.modified().ok());
    (source.user_stamp(), mtime("theme.json").max(mtime("settings.json")), themes_dir)
}

struct Runner<A: App + 'static> {
    app: A,
    ui: AppUi,
    source: ConfigSource,
    stamp: (Option<(std::path::PathBuf, SystemTime)>, Option<SystemTime>, Option<SystemTime>),
    last_reload: Instant,

    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    _compositor: CompositorState,
    cursor_shapes: Option<CursorShapeManager>,
    cursor_device: Option<
        smithay_client_toolkit::reexports::protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::WpCursorShapeDeviceV1,
    >,
    loop_handle: calloop::LoopHandle<'static, Runner<A>>,

    gpu: Gpu,
    display: NonNull<std::ffi::c_void>,
    /// Declared before `window`: the swapchain must drop before the surface
    /// it presents into.
    renderer: Option<SurfaceRenderer>,
    window: Window,
    size: (u32, u32),
    scale: i32,
    outputs: Vec<wl_output::WlOutput>,
    configured: bool,
    dirty: bool,
    title: String,

    pointer: Pointer,
    last_press: Option<(Instant, (f32, f32))>,
    modifiers: Modifiers,
    quit: bool,
    last_frame: Instant,
}

/// Open the window and run `app` until it quits or the window closes.
pub fn run<A: App + 'static>(app: A, options: Options) -> Result<(), String> {
    let conn = Connection::connect_to_env().map_err(|e| format!("no Wayland display: {e}"))?;
    let (globals, event_queue) = registry_queue_init::<Runner<A>>(&conn).map_err(|e| e.to_string())?;
    let qh = event_queue.handle();
    let mut event_loop: EventLoop<'static, Runner<A>> = EventLoop::try_new().map_err(|e| e.to_string())?;
    let loop_handle = event_loop.handle();

    let source = ConfigSource::locate().map_err(|e| e.to_string())?;
    let config = load_config(&source)?;
    let theme = theme_of(&config);
    let gpu = pollster::block_on(Gpu::new(config.render.gpu))?;
    let display = NonNull::new(conn.backend().display_ptr().cast::<std::ffi::c_void>()).ok_or("wl_display pointer was null")?;

    let compositor = CompositorState::bind(&globals, &qh).map_err(|e| format!("no wl_compositor: {e}"))?;
    let xdg_shell = XdgShell::bind(&globals, &qh).map_err(|e| format!("no xdg-shell: {e}"))?;
    let surface = compositor.create_surface(&qh);
    let window = xdg_shell.create_window(surface, WindowDecorations::RequestServer, &qh);
    window.set_title(options.title.clone());
    window.set_app_id(options.app_id.clone());
    window.set_min_size(Some(options.min_size));
    window.commit();

    let mut runner = Runner {
        app,
        ui: AppUi { config: Arc::new(config), theme, text: TextEngine::new(), icons: IconTheme::load(&["Papirus", "Adwaita", "breeze"]) },
        stamp: look_stamp(&source),
        source,
        last_reload: Instant::now(),
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        _compositor: compositor,
        cursor_shapes: CursorShapeManager::bind(&globals, &qh).ok(),
        cursor_device: None,
        loop_handle: loop_handle.clone(),
        gpu,
        display,
        renderer: None,
        window,
        size: options.size,
        scale: 1,
        outputs: Vec::new(),
        configured: false,
        dirty: true,
        title: options.title,
        pointer: Pointer::default(),
        last_press: None,
        modifiers: Modifiers::default(),
        quit: false,
        last_frame: Instant::now(),
    };

    let (ping, ping_source) = calloop::ping::make_ping().map_err(|e| e.to_string())?;
    loop_handle
        .insert_source(ping_source, |(), _, runner: &mut Runner<A>| {
            if runner.app.poll() {
                runner.dirty = true;
            }
        })
        .map_err(|e| e.to_string())?;
    runner.app.start(Waker(ping));
    WaylandSource::new(conn.clone(), event_queue).insert(loop_handle).map_err(|e| e.to_string())?;

    let outcome = loop {
        let animating = runner.app.tick(runner.last_frame.elapsed().as_secs_f32().min(0.1));
        runner.last_frame = Instant::now();
        if animating {
            runner.dirty = true;
        }
        let timeout = if animating { Duration::from_millis(16) } else { RELOAD_EVERY };
        if let Err(err) = event_loop.dispatch(timeout, &mut runner) {
            break Err(format!("event loop failed: {err}"));
        }
        if let Some(err) = conn.protocol_error() {
            break Err(format!("protocol error {} on {}@{}: {}", err.code, err.object_interface, err.object_id, err.message));
        }
        if runner.app.poll() {
            runner.dirty = true;
        }
        runner.reload_look();
        if let Err(err) = runner.paint() {
            tracing::error!("render failed: {err}");
        }
        if runner.quit || runner.app.done() {
            break Ok(());
        }
    };
    // The swapchain goes before the window and the connection.
    runner.renderer = None;
    drop(runner);
    drop(event_loop);
    drop(conn);
    outcome
}

impl<A: App + 'static> Runner<A> {
    fn physical(&self) -> (u32, u32) {
        let s = self.scale.max(1) as u32;
        (self.size.0 * s, self.size.1 * s)
    }

    /// Follow the config and the theme picked in the shell.
    fn reload_look(&mut self) {
        if self.last_reload.elapsed() < RELOAD_EVERY {
            return;
        }
        self.last_reload = Instant::now();
        let stamp = look_stamp(&self.source);
        if stamp == self.stamp {
            return;
        }
        self.stamp = stamp;
        match load_config(&self.source) {
            Ok(config) => {
                self.ui.theme = theme_of(&config);
                self.ui.config = Arc::new(config);
                self.dirty = true;
            }
            Err(err) => tracing::warn!("config reload failed: {err}"),
        }
    }

    fn paint(&mut self) -> Result<(), String> {
        if !self.configured || !self.dirty {
            return Ok(());
        }
        self.dirty = false;
        let physical = self.physical();
        if self.renderer.is_none() {
            self.renderer = Some(SurfaceRenderer::new(&self.gpu, self.display, self.window.wl_surface(), physical, self.scale as f32)?);
        }
        let area = Rect { x: 0.0, y: 0.0, width: self.size.0 as f32, height: self.size.1 as f32, radius: 0.0, color: Color::TRANSPARENT };
        let mut list = DisplayList::default();
        list.push_rect(Rect { color: self.ui.theme.bg_main, ..area });
        self.pointer.ctrl = self.modifiers.ctrl;
        self.pointer.shift = self.modifiers.shift;
        self.app.paint(&mut self.ui, &mut list, area, &self.pointer);
        // Edges are seen once.
        self.pointer.clicked = false;
        self.pointer.double = false;
        self.pointer.secondary = false;
        self.pointer.released = false;
        let renderer = self.renderer.as_mut().ok_or("renderer missing")?;
        renderer.submit(&self.gpu, &list, &mut self.ui.text)?;
        self.window.wl_surface().damage_buffer(0, 0, physical.0 as i32, physical.1 as i32);
        self.window.commit();
        let title = self.app.title();
        if title != self.title {
            self.window.set_title(title.clone());
            self.title = title;
        }
        Ok(())
    }

    fn rescale(&mut self) {
        let scale = self
            .outputs
            .iter()
            .filter_map(|o| self.output_state.info(o).map(|i| i.scale_factor))
            .max()
            .unwrap_or(1)
            .max(1);
        if scale != self.scale {
            self.scale = scale;
            self.window.wl_surface().set_buffer_scale(scale);
            let physical = self.physical();
            if let Some(renderer) = &mut self.renderer {
                renderer.resize(&self.gpu, physical, scale as f32);
            }
            self.dirty = true;
        }
    }

    fn key_event(&mut self, event: KeyEvent, repeat: bool) {
        let key = Key {
            keysym: event.keysym,
            text: event.utf8.filter(|t| !t.chars().any(char::is_control)),
            ctrl: self.modifiers.ctrl,
            shift: self.modifiers.shift,
            alt: self.modifiers.alt,
            repeat,
        };
        if self.app.key(&mut self.ui, &key) == Flow::Quit {
            self.quit = true;
        }
        self.dirty = true;
    }
}

impl<A: App + 'static> CompositorHandler for Runner<A> {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: i32) {
        self.rescale();
    }
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, output: &wl_output::WlOutput) {
        if !self.outputs.contains(output) {
            self.outputs.push(output.clone());
        }
        self.rescale();
    }
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, output: &wl_output::WlOutput) {
        self.outputs.retain(|o| o != output);
        self.rescale();
    }
}

impl<A: App + 'static> OutputHandler for Runner<A> {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        self.rescale();
    }
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, output: wl_output::WlOutput) {
        self.outputs.retain(|o| *o != output);
    }
}

impl<A: App + 'static> WindowHandler for Runner<A> {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window) {
        if self.app.close_requested() {
            self.quit = true;
        }
    }

    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window, configure: WindowConfigure, _serial: u32) {
        let width = configure.new_size.0.map(|v| v.get()).unwrap_or(self.size.0);
        let height = configure.new_size.1.map(|v| v.get()).unwrap_or(self.size.1);
        self.size = (width.max(1), height.max(1));
        self.window.wl_surface().set_buffer_scale(self.scale.max(1));
        let physical = self.physical();
        if let Some(renderer) = &mut self.renderer {
            renderer.resize(&self.gpu, physical, self.scale as f32);
        }
        self.configured = true;
        self.dirty = true;
    }
}

impl<A: App + 'static> SeatHandler for Runner<A> {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Pointer {
            if let Ok(pointer) = self.seat_state.get_pointer(qh, &seat) {
                self.cursor_device = self.cursor_shapes.as_ref().map(|m| m.get_shape_device(&pointer, qh));
            }
        }
        if capability == Capability::Keyboard {
            let repeat: smithay_client_toolkit::seat::keyboard::repeat::RepeatCallback<Self> =
                Box::new(|runner, _keyboard, event| runner.key_event(event, true));
            if let Err(err) = self.seat_state.get_keyboard_with_repeat(qh, &seat, None, self.loop_handle.clone(), repeat) {
                tracing::warn!("no keyboard: {err}");
            }
        }
    }
    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, _: Capability) {}
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl<A: App + 'static> KeyboardHandler for Runner<A> {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wayland_client::protocol::wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
    }
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wayland_client::protocol::wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32) {}
    fn press_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wayland_client::protocol::wl_keyboard::WlKeyboard, _: u32, event: KeyEvent) {
        self.key_event(event, false);
    }
    fn repeat_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wayland_client::protocol::wl_keyboard::WlKeyboard, _: u32, event: KeyEvent) {
        // Compositors on wl_keyboard v10 repeat keys themselves.
        self.key_event(event, true);
    }
    fn release_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wayland_client::protocol::wl_keyboard::WlKeyboard, _: u32, _: KeyEvent) {}
    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wayland_client::protocol::wl_keyboard::WlKeyboard,
        _: u32,
        modifiers: Modifiers,
        _: RawModifiers,
        _: u32,
    ) {
        self.modifiers = modifiers;
    }
}

impl<A: App + 'static> PointerHandler for Runner<A> {
    fn pointer_frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_pointer::WlPointer, events: &[PointerEvent]) {
        for event in events {
            let point = (event.position.0 as f32, event.position.1 as f32);
            match &event.kind {
                PointerEventKind::Enter { serial } => {
                    self.pointer.input.pointer = Some(point);
                    if let Some(device) = &self.cursor_device {
                        device.set_shape(
                            *serial,
                            smithay_client_toolkit::reexports::protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::Shape::Default,
                        );
                    }
                }
                PointerEventKind::Leave { .. } => self.pointer.input.pointer = None,
                PointerEventKind::Motion { .. } => self.pointer.input.pointer = Some(point),
                PointerEventKind::Press { button, .. } if *button == BTN_LEFT => {
                    self.pointer.input.pointer = Some(point);
                    self.pointer.input.pressed = true;
                    self.pointer.clicked = true;
                    let now = Instant::now();
                    let double = self.last_press.is_some_and(|(at, p)| {
                        now.duration_since(at) < DOUBLE_CLICK
                            && (p.0 - point.0).abs() < DOUBLE_CLICK_SLOP
                            && (p.1 - point.1).abs() < DOUBLE_CLICK_SLOP
                    });
                    self.pointer.double = double;
                    // A third click starts a new pair.
                    self.last_press = if double { None } else { Some((now, point)) };
                }
                PointerEventKind::Press { button, .. } if *button == BTN_RIGHT => {
                    self.pointer.input.pointer = Some(point);
                    self.pointer.secondary = true;
                }
                PointerEventKind::Release { button, .. } if *button == BTN_LEFT => {
                    self.pointer.input.pressed = false;
                    self.pointer.released = true;
                }
                PointerEventKind::Axis { vertical, .. } => {
                    let delta = if vertical.discrete != 0 { vertical.discrete as f32 * 48.0 } else { vertical.absolute as f32 };
                    if delta != 0.0 {
                        self.app.scroll(point, delta);
                    }
                }
                _ => {}
            }
            self.dirty = true;
        }
    }
}

impl<A: App + 'static> ProvidesRegistryState for Runner<A> {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

delegate_dispatch2!(@<A: App + 'static> Runner<A>);
delegate_registry!(@<A: App + 'static> Runner<A>);
