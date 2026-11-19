//! Idle: the compositor's idle notifications, the dim layer, turning the
//! screens off, and logind's sleep and lock requests. The policy is
//! `crate::idle`; this is the plumbing.
//!
//! The dim draws no frames. Each output gets an overlay layer holding a
//! single-pixel buffer that `wp_viewporter` stretches over it, so dimming
//! costs a few protocol objects instead of an output-sized swapchain (60 MB
//! at 2560x1440). Without either protocol the shell just does not dim.

use smithay_client_toolkit::dispatch2::Dispatch2;
use wayland_client::globals::GlobalList;
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notification_v1::{
    self, ExtIdleNotificationV1,
};
use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notifier_v1::ExtIdleNotifierV1;
use wayland_protocols::wp::single_pixel_buffer::v1::client::wp_single_pixel_buffer_manager_v1::WpSinglePixelBufferManagerV1;
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;
use wayland_protocols::wp::viewporter::client::wp_viewporter::WpViewporter;
use wayland_protocols_wlr::output_power_management::v1::client::zwlr_output_power_manager_v1::ZwlrOutputPowerManagerV1;
use wayland_protocols_wlr::output_power_management::v1::client::zwlr_output_power_v1::{
    self, ZwlrOutputPowerV1,
};

use super::*;
use crate::idle::{Effect, Stage};

/// The globals idling uses. Each is optional: a compositor without one loses
/// only that feature.
pub(super) struct IdleGlobals {
    notifier: Option<ExtIdleNotifierV1>,
    pixels: Option<WpSinglePixelBufferManagerV1>,
    viewporter: Option<WpViewporter>,
    power: Option<ZwlrOutputPowerManagerV1>,
}

impl IdleGlobals {
    pub(super) fn bind(globals: &GlobalList, qh: &QueueHandle<App>) -> Self {
        let this = Self {
            notifier: globals.bind(qh, 1..=1, Quiet).ok(),
            pixels: globals.bind(qh, 1..=1, Quiet).ok(),
            viewporter: globals.bind(qh, 1..=1, Quiet).ok(),
            power: globals.bind(qh, 1..=1, Quiet).ok(),
        };
        if this.notifier.is_none() {
            warn!("the compositor has no ext-idle-notify-v1: nothing happens when idle");
        }
        if this.pixels.is_none() || this.viewporter.is_none() {
            info!("no single-pixel buffers or viewporter: idle will not dim");
        }
        if this.power.is_none() {
            info!("no wlr-output-power-management: idle will not turn the screens off");
        }
        this
    }
}

/// The dim layer on every output while dimmed.
pub(super) struct Dim {
    started: std::time::Instant,
    /// Premultiplied 1×1 buffers by level, made as the fade reaches them and
    /// shared by every output.
    buffers: Vec<Option<WlBuffer>>,
    surfaces: Vec<DimSurface>,
}

impl Drop for Dim {
    fn drop(&mut self) {
        // Surfaces first, so no buffer is destroyed while attached.
        self.surfaces.clear();
        for buffer in self.buffers.drain(..).flatten() {
            buffer.destroy();
        }
    }
}

struct DimSurface {
    layer: LayerSurface,
    viewport: WpViewport,
    /// The configured logical size; `None` until the first configure.
    size: Option<(u32, u32)>,
    /// The level last attached.
    level: Option<u8>,
}

impl Drop for DimSurface {
    fn drop(&mut self) {
        self.viewport.destroy();
    }
}

impl App {
    /// Ask for one idle notification per enabled stage, replacing any from
    /// before. Called at start, when a seat appears, and when `[idle]`
    /// changes.
    pub(super) fn arm_idle(&mut self) {
        for notification in self.idle_notifications.drain(..) {
            notification.destroy();
        }
        let effects = self.idle.reset();
        self.apply_idle_effects(effects);
        if !self.config.idle.enabled {
            return;
        }
        let Some(notifier) = &self.idle_globals.notifier else {
            return;
        };
        let Some(seat) = self.seat_state.seats().next() else {
            return;
        };
        for (stage, timeout_ms) in crate::idle::timeouts(&self.config.idle) {
            self.idle_notifications.push(notifier.get_idle_notification(
                timeout_ms,
                &seat,
                &self.queue_handle,
                StageData(stage),
            ));
        }
        debug!("idle armed: {:?}", crate::idle::timeouts(&self.config.idle));
    }

    fn idle_stage(&mut self, stage: Stage, idled: bool) {
        let effect = if idled {
            info!("idle: {stage:?} timeout reached");
            let battery = self.state.battery.get();
            let on_battery = battery["battery"]["present"].as_bool() == Some(true)
                && battery["battery"]["state"].as_str() == Some("discharging");
            self.idle.idled(stage, on_battery, self.config.idle.suspend_on_ac)
        } else {
            self.idle.resumed(stage)
        };
        if let Some(effect) = effect {
            self.apply_idle_effects([effect]);
        }
    }

    fn apply_idle_effects(&mut self, effects: impl IntoIterator<Item = Effect>) {
        for effect in effects {
            match effect {
                Effect::DimOn => self.start_dim(),
                Effect::DimOff => self.dim = None,
                // A refusal is logged by `begin_lock`.
                Effect::Lock => {
                    let _ = self.begin_lock();
                }
                Effect::DpmsOff => self.set_screens_on(false),
                Effect::DpmsOn => self.set_screens_on(true),
                Effect::Suspend => crate::logind::suspend(self.config.idle.suspend_while_playing),
            }
        }
    }

    /// Stay awake, or stop (the sidebar icon and `stratum-cli idle`).
    pub(super) fn set_idle_inhibited(&mut self, inhibited: bool) {
        let effects = self.idle.set_inhibited(inhibited);
        self.apply_idle_effects(effects);
        if self.state.idle_inhibit.set(inhibited) {
            info!("idle {}", if inhibited { "held off" } else { "allowed" });
        }
        self.mark_all_dirty();
    }

    pub(super) fn idle_inhibited(&self) -> bool {
        self.idle.inhibited()
    }

    fn start_dim(&mut self) {
        let (Some(_), Some(viewporter)) = (&self.idle_globals.pixels, &self.idle_globals.viewporter) else {
            return;
        };
        // Under the lock screen the dim would never be seen.
        if self.lock.active() {
            return;
        }
        let qh = self.queue_handle.clone();
        let mut surfaces = Vec::new();
        for output in self.output_state.outputs() {
            let wl = self.compositor_state.create_surface(&qh);
            let layer = self.layer_shell.create_layer_surface(
                &qh,
                wl,
                Layer::Overlay,
                Some("stratum-dim"),
                Some(&output),
            );
            layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
            layer.set_exclusive_zone(-1);
            layer.set_keyboard_interactivity(KeyboardInteractivity::None);
            // An empty input region: a click goes to the window underneath,
            // and the input itself ends the dim.
            if let Ok(region) = smithay_client_toolkit::compositor::Region::new(&self.compositor_state) {
                layer.wl_surface().set_input_region(Some(region.wl_region()));
            }
            let viewport = viewporter.get_viewport(layer.wl_surface(), &qh, Quiet);
            layer.commit();
            surfaces.push(DimSurface { layer, viewport, size: None, level: None });
        }
        self.dim = Some(Dim {
            started: std::time::Instant::now(),
            buffers: vec![None; usize::from(crate::idle::DIM_LEVELS) + 1],
            surfaces,
        });
    }

    /// A configure for one of the dim layers. Returns false if `layer` is
    /// not one.
    pub(super) fn configure_dim(&mut self, layer: &LayerSurface, width: u32, height: u32) -> bool {
        let Some(surface) = self
            .dim
            .as_mut()
            .and_then(|dim| dim.surfaces.iter_mut().find(|s| &s.layer == layer))
        else {
            return false;
        };
        let (width, height) = (width.max(1), height.max(1));
        surface.viewport.set_destination(width as i32, height as i32);
        surface.size = Some((width, height));
        // Attach again on the next tick, with the new destination.
        surface.level = None;
        true
    }

    /// The compositor closed a dim layer (its output went away).
    pub(super) fn dim_closed(&mut self, layer: &LayerSurface) {
        if let Some(dim) = &mut self.dim {
            dim.surfaces.retain(|s| &s.layer != layer);
        }
    }

    /// Advance the dim's fade. Commits only when its level changes.
    pub(super) fn tick_idle(&mut self) {
        let Some(pixels) = &self.idle_globals.pixels else {
            return;
        };
        let Some(dim) = &mut self.dim else {
            return;
        };
        let config = &self.config.idle;
        let alpha = crate::idle::dim_alpha(
            dim.started.elapsed(),
            std::time::Duration::from_millis(u64::from(config.dim_fade_ms)),
            config.dim_opacity,
        );
        let level = crate::idle::dim_level(alpha);
        if dim.surfaces.iter().all(|s| s.size.is_none() || s.level == Some(level)) {
            return;
        }
        let buffer = dim.buffers[usize::from(level)]
            .get_or_insert_with(|| {
                let color = crate::config::parse_color(&config.dim_color).unwrap_or(Color::TRANSPARENT);
                let alpha = f32::from(level) / f32::from(crate::idle::DIM_LEVELS);
                // Premultiplied, each channel a fraction of u32::MAX.
                let channel = |value: f32| (f64::from(value.clamp(0.0, 1.0)) * f64::from(u32::MAX)) as u32;
                pixels.create_u32_rgba_buffer(
                    channel(color.r * alpha),
                    channel(color.g * alpha),
                    channel(color.b * alpha),
                    channel(alpha),
                    &self.queue_handle,
                    Quiet,
                )
            })
            .clone();
        for surface in &mut dim.surfaces {
            if surface.size.is_none() || surface.level == Some(level) {
                continue;
            }
            let wl = surface.layer.wl_surface();
            wl.attach(Some(&buffer), 0, 0);
            wl.damage_buffer(0, 0, 1, 1);
            wl.commit();
            surface.level = Some(level);
        }
    }

    /// Whether the dim is still fading in, which needs frames.
    pub(super) fn dim_fading(&self) -> bool {
        self.dim.as_ref().is_some_and(|dim| {
            dim.started.elapsed() < std::time::Duration::from_millis(u64::from(self.config.idle.dim_fade_ms))
        })
    }

    /// Turn every output off, or back on.
    fn set_screens_on(&mut self, on: bool) {
        if on {
            for power in self.output_power.drain(..) {
                power.set_mode(zwlr_output_power_v1::Mode::On);
                power.destroy();
            }
            return;
        }
        let Some(manager) = &self.idle_globals.power else {
            return;
        };
        for output in self.output_state.outputs() {
            let name = self
                .output_state
                .info(&output)
                .and_then(|info| info.name)
                .unwrap_or_default();
            let power = manager.get_output_power(&output, &self.queue_handle, PowerData(name));
            power.set_mode(zwlr_output_power_v1::Mode::Off);
            self.output_power.push(power);
        }
    }

    /// Listen to logind for sleep and `loginctl lock-session`.
    pub(super) fn start_logind(&mut self, loop_handle: &calloop::LoopHandle<'static, App>) {
        let config = &self.config.idle;
        if !config.enabled || !(config.lock_before_sleep || config.lock_on_loginctl) {
            return;
        }
        let (tx, channel) = calloop::channel::channel();
        if let Err(err) = loop_handle.insert_source(channel, |event, _, app: &mut App| {
            if let calloop::channel::Event::Msg(event) = event {
                app.handle_logind(event);
            }
        }) {
            warn!("failed to insert logind source: {err}");
            return;
        }
        self.logind = Some(crate::logind::Logind::start(tx, config.lock_before_sleep, config.lock_on_loginctl));
    }

    fn handle_logind(&mut self, event: crate::logind::Event) {
        match event {
            crate::logind::Event::PrepareForSleep => {
                info!("the system is going to sleep; locking first");
                match self.begin_lock() {
                    Ok(()) => self.sleep_pending = true,
                    // Refused (and logged by `begin_lock`): nothing to wait for.
                    Err(_) => {
                        if let Some(logind) = &self.logind {
                            logind.release_sleep();
                        }
                    }
                }
                // Already locked and drawn: no frame is coming to release it.
                self.release_sleep_when_locked();
            }
            crate::logind::Event::LockRequested => {
                info!("logind asked the session to lock");
                let _ = self.begin_lock();
            }
        }
    }

    /// Let a held-back sleep go ahead once every lock surface has drawn, or
    /// once the lock has ended without (the compositor refused it).
    pub(super) fn release_sleep_when_locked(&mut self) {
        if !self.sleep_pending {
            return;
        }
        let drawn = self.lock.phase() == crate::lock::Phase::Locked
            && self.lock_surfaces.iter().all(|s| s.configured && !s.dirty);
        if drawn || !self.lock.active() {
            self.sleep_pending = false;
            if let Some(logind) = &self.logind {
                logind.release_sleep();
            }
        }
    }
}

/// User data for objects whose events need nothing: the globals, viewports
/// and the dim's buffers (single-pixel buffers never need releasing).
pub(super) struct Quiet;

macro_rules! quiet {
    ($($interface:ty),*) => {$(
        impl Dispatch2<$interface, App> for Quiet {
            fn event(
                &self,
                _: &mut App,
                _: &$interface,
                _: <$interface as wayland_client::Proxy>::Event,
                _: &Connection,
                _: &QueueHandle<App>,
            ) {
            }
        }
    )*};
}

quiet!(
    ExtIdleNotifierV1,
    WpSinglePixelBufferManagerV1,
    WpViewporter,
    WpViewport,
    WlBuffer,
    ZwlrOutputPowerManagerV1
);

/// Which stage an idle notification is for.
pub(super) struct StageData(Stage);

impl Dispatch2<ExtIdleNotificationV1, App> for StageData {
    fn event(
        &self,
        app: &mut App,
        _: &ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
        match event {
            ext_idle_notification_v1::Event::Idled => app.idle_stage(self.0, true),
            ext_idle_notification_v1::Event::Resumed => app.idle_stage(self.0, false),
            _ => {}
        }
    }
}

/// The output a power control is for, for the log.
pub(super) struct PowerData(String);

impl Dispatch2<ZwlrOutputPowerV1, App> for PowerData {
    fn event(
        &self,
        _: &mut App,
        _: &ZwlrOutputPowerV1,
        event: zwlr_output_power_v1::Event,
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
        match event {
            zwlr_output_power_v1::Event::Mode { mode } => debug!("output {} power: {mode:?}", self.0),
            zwlr_output_power_v1::Event::Failed => warn!("could not change the power of output {}", self.0),
            _ => {}
        }
    }
}
