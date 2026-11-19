//! Night light on the outputs, through `wlr-gamma-control-unstable-v1`.
//!
//! While it is warmer than daylight, every output holds a gamma control
//! carrying the ramps for the current colour; in daylight the controls are
//! destroyed, which gives the outputs back their own colours. The schedule
//! is evaluated every few seconds (the loop wakes at least once a second),
//! and the ramps are sent again only when the colour has moved by
//! `MIN_STEP` kelvin, so a 30-minute fade costs a handful of commits.
//!
//! Only one client may hold an output's gamma: if another does (gammastep,
//! hyprsunset), the control fails, the status says so, and the shell does
//! not retry until the settings change or night light is toggled. An
//! output without gamma support (a headless or nested one) fails the same
//! way; the protocol does not say which it was.

use std::io::{Seek, Write};
use std::os::fd::{AsFd, FromRawFd};

use smithay_client_toolkit::dispatch2::Dispatch2;
use wayland_client::globals::GlobalList;
use wayland_protocols_wlr::gamma_control::v1::client::zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1;
use wayland_protocols_wlr::gamma_control::v1::client::zwlr_gamma_control_v1::{self, ZwlrGammaControlV1};

use super::*;
use crate::night_light::{self as night, Status};

/// How far the colour moves before the ramps are sent again.
const MIN_STEP: u32 = 15;
/// How often the schedule is looked at.
const EVERY: std::time::Duration = std::time::Duration::from_secs(5);
/// How often the location is read again (the weather may find a new one).
const LOCATION_EVERY: std::time::Duration = std::time::Duration::from_secs(600);

struct Control {
    output: wl_output::WlOutput,
    name: String,
    control: ZwlrGammaControlV1,
    /// The ramp size, once the compositor has said it.
    size: Option<usize>,
    applied: Option<u32>,
}

#[derive(Default)]
pub(super) struct NightLight {
    manager: Option<ZwlrGammaControlManagerV1>,
    controls: Vec<Control>,
    /// Turned on or off by hand, until a Unix time (`i64::MAX`: until
    /// turned back).
    manual: Option<(bool, i64)>,
    location: Option<night::Location>,
    location_read: Option<std::time::Instant>,
    last: Option<std::time::Instant>,
    /// Outputs whose control failed (another program has their gamma),
    /// not retried until something changes.
    failed: std::collections::BTreeSet<String>,
}

impl App {
    pub(super) fn bind_night_light(&mut self, globals: &GlobalList) {
        self.night.manager = globals.bind(&self.queue_handle, 1..=1, GammaManagerData).ok();
        if self.night.manager.is_none() {
            info!("no wlr-gamma-control: no night light");
        }
    }

    /// Look again now: the settings changed or it was toggled.
    pub(super) fn night_light_changed(&mut self) {
        self.night.last = None;
        self.night.location_read = None;
        self.night.failed.clear();
    }

    /// `stratum-cli nightlight on|off|toggle|auto`, the bar's moon, and the
    /// Display page's buttons. On or off by hand lasts until the schedule
    /// would change anyway (or, with night light disabled, until undone).
    pub(super) fn night_light_command(&mut self, command: &str) {
        let now = chrono::Local::now();
        let config = &self.config.night_light;
        let (scheduled, next) = if config.enabled {
            night::strength(now, config, self.night.location)
        } else {
            (0.0, None)
        };
        let until = next.unwrap_or(i64::MAX);
        let on_now = self.state.night_light.get().on;
        self.night.manual = match command {
            "on" => Some((true, until)),
            "off" => Some((false, until)),
            "toggle" => Some((!on_now, until)),
            _ => None,
        };
        // Back on the schedule when the hand setting matches it.
        if self.night.manual.is_some_and(|(on, _)| on == (scheduled > 0.0)) && config.enabled {
            self.night.manual = None;
        }
        self.night_light_changed();
        self.tick_night_light();
        self.mark_all_dirty();
    }

    /// Bring the outputs to the colour the schedule asks for now.
    pub(super) fn tick_night_light(&mut self) {
        if self.night.last.is_some_and(|t| t.elapsed() < EVERY) {
            return;
        }
        self.night.last = Some(std::time::Instant::now());
        let config = self.config.night_light.clone();
        if self.night.location_read.is_none_or(|t| t.elapsed() > LOCATION_EVERY) {
            self.night.location = night::location(&config);
            self.night.location_read = Some(std::time::Instant::now());
        }
        let now = chrono::Local::now();
        let (scheduled, next) = if config.enabled {
            night::strength(now, &config, self.night.location)
        } else {
            (0.0, None)
        };
        if self.night.manual.is_some_and(|(_, until)| now.timestamp() >= until) {
            self.night.manual = None;
        }
        let strength = match self.night.manual {
            Some((on, _)) => f32::from(u8::from(on)),
            None => scheduled,
        };
        let kelvin = night::kelvin(strength, config.temperature);
        let problem = if self.night.manager.is_none() {
            Some("The compositor does not let the shell set screen colours".to_string())
        } else if !self.night.failed.is_empty() {
            let names: Vec<&str> = self.night.failed.iter().map(String::as_str).collect();
            Some(format!(
                "Cannot set the colours of {}: another program holds them, or the screen does not support it",
                names.join(", ")
            ))
        } else {
            None
        };
        let configured = (config.latitude.is_some() && config.longitude.is_some()).then_some(true);
        let status = Status {
            on: kelvin < night::DAY_KELVIN,
            kelvin,
            until: self.night.manual.map(|(_, until)| until).filter(|u| *u != i64::MAX).or(next),
            manual: self.night.manual.map(|(on, _)| on),
            location: self.night.location.map(|l| (l, configured.unwrap_or(false))),
            problem,
        };
        if self.state.night_light.set(status) {
            self.mark_all_dirty();
        }
        self.apply_night_light(kelvin);
    }

    fn apply_night_light(&mut self, kelvin: u32) {
        if kelvin >= night::DAY_KELVIN {
            // Destroying a control gives the output back its own colours.
            for control in self.night.controls.drain(..) {
                control.control.destroy();
            }
            return;
        }
        let Some(manager) = &self.night.manager else { return };
        // A control for every output that lacks one.
        for output in self.output_state.outputs() {
            let Some(name) = self.output_state.info(&output).and_then(|info| info.name) else { continue };
            if self.night.failed.contains(&name) || self.night.controls.iter().any(|c| c.output == output) {
                continue;
            }
            let control = manager.get_gamma_control(&output, &self.queue_handle, GammaData(name.clone()));
            self.night.controls.push(Control { output, name, control, size: None, applied: None });
        }
        for control in &mut self.night.controls {
            let Some(size) = control.size else { continue };
            if control.applied.is_some_and(|k| k.abs_diff(kelvin) < MIN_STEP) {
                continue;
            }
            match gamma_file(&night::ramps(size, kelvin)) {
                Ok(file) => {
                    control.control.set_gamma(file.as_fd());
                    debug!("night light: {} at {kelvin} K ({size} entries)", control.name);
                    control.applied = Some(kelvin);
                }
                Err(err) => warn!("night light: no buffer for the ramps: {err}"),
            }
        }
    }

    fn gamma_size(&mut self, name: &str, size: u32) {
        if let Some(control) = self.night.controls.iter_mut().find(|c| c.name == name) {
            control.size = Some(size as usize);
            control.applied = None;
        }
        // Send the ramps now rather than at the next look.
        self.night.last = None;
    }

    fn gamma_failed(&mut self, name: &str) {
        if let Some(pos) = self.night.controls.iter().position(|c| c.name == name) {
            let control = self.night.controls.remove(pos);
            control.control.destroy();
        }
        // An output that went away fails too; only a present one is a
        // conflict.
        let present = self
            .output_state
            .outputs()
            .any(|o| self.output_state.info(&o).and_then(|i| i.name).as_deref() == Some(name));
        if present {
            warn!("night light: cannot set the colours of {name} (another program holds them, or no gamma support)");
            self.night.failed.insert(name.to_string());
        }
        self.night.last = None;
    }
}

/// The ramps in a sealed-off memory file, at its start, as `set_gamma`
/// reads them.
fn gamma_file(ramps: &[u16]) -> std::io::Result<std::fs::File> {
    // SAFETY: a valid C string and flags; the descriptor is owned by the
    // File from here on.
    let fd = unsafe { libc::memfd_create(c"stratum-gamma".as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    file.write_all(bytemuck::cast_slice(ramps))?;
    file.rewind()?;
    Ok(file)
}

pub(super) struct GammaManagerData;

impl Dispatch2<ZwlrGammaControlManagerV1, App> for GammaManagerData {
    fn event(
        &self,
        _: &mut App,
        _: &ZwlrGammaControlManagerV1,
        _: <ZwlrGammaControlManagerV1 as wayland_client::Proxy>::Event,
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
    }
}

/// The output a gamma control is for.
pub(super) struct GammaData(String);

impl Dispatch2<ZwlrGammaControlV1, App> for GammaData {
    fn event(
        &self,
        app: &mut App,
        _: &ZwlrGammaControlV1,
        event: zwlr_gamma_control_v1::Event,
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
        match event {
            zwlr_gamma_control_v1::Event::GammaSize { size } => app.gamma_size(&self.0, size),
            zwlr_gamma_control_v1::Event::Failed => app.gamma_failed(&self.0),
            _ => {}
        }
    }
}
