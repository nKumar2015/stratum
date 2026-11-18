//! The thread that applies monitor layouts to Hyprland.
//!
//! The Display page sends [`Command`]s here; this thread talks to the
//! compositor and publishes [`DisplaysState`] into `SystemState::displays`.
//!
//! An applied layout is on trial until it is kept: if Keep is not sent
//! within `[displays] confirm_s` (the screen may be black and the window
//! out of reach), the layout from before is put back. The countdown runs
//! here rather than in the page, so it ends on time even when the settings
//! window is closed or unpainted.
//!
//! A kept layout is saved for the set of monitors connected
//! (`displays::Saved`) and applied again at start, after a config reload
//! (which drops `eval`'d rules) and whenever that set is plugged back in.
//! Applying only sends what differs, which matters: disabling a monitor
//! makes Hyprland report it removed, and that event comes back here.

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::displays::{self, Monitor, Settings};
use crate::sys_state::SystemState;

/// What the Display page asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// Read the monitors again.
    Refresh,
    /// Apply these settings (by connector name) on trial.
    Apply(Vec<(String, Settings)>),
    /// Keep the layout on trial, and save it.
    Keep,
    /// Put back the layout from before the trial.
    Revert,
    /// Apply the saved layout for the monitors connected, if any.
    Reapply,
    /// Forget the saved layout and go back to Hyprland's config.
    Forget,
}

/// What the Display page shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DisplaysState {
    /// Whether Hyprland is reachable; the page has nothing to offer without.
    pub available: bool,
    pub monitors: Vec<Monitor>,
    /// Whether a layout is saved for this set of monitors.
    pub saved: bool,
    /// When the layout on trial reverts, while one is.
    pub trial_until: Option<Instant>,
    /// What went wrong last, to show until the next success.
    pub error: Option<String>,
}

struct Trial {
    previous: Vec<(String, Settings)>,
    applied: Vec<(String, Settings)>,
    until: Instant,
}

/// Start the thread. Monitor events from Hyprland are turned into
/// [`Command::Reapply`], and the saved layout is applied once at start.
pub fn start(state: Arc<SystemState>, confirm: Duration) -> Sender<Command> {
    let (tx, rx) = std::sync::mpsc::channel();
    if crate::hyprland::available() {
        let events = tx.clone();
        crate::hyprland::spawn_monitor_watcher(move || {
            let _ = events.send(Command::Reapply);
        });
    }
    let _ = tx.send(Command::Reapply);
    std::thread::Builder::new()
        .name("stratum-displays".into())
        .spawn(move || run(rx, state, confirm))
        .ok();
    tx
}

fn run(rx: Receiver<Command>, state: Arc<SystemState>, confirm: Duration) {
    let mut trial: Option<Trial> = None;
    let mut error: Option<String> = None;
    loop {
        let command = match trial.as_ref().map(|t| t.until.saturating_duration_since(Instant::now())) {
            Some(left) => match rx.recv_timeout(left) {
                Ok(command) => command,
                Err(RecvTimeoutError::Timeout) => Command::Revert,
                Err(RecvTimeoutError::Disconnected) => return,
            },
            None => match rx.recv() {
                Ok(command) => command,
                Err(_) => return,
            },
        };
        let monitors = read();
        match command {
            Command::Refresh => {}
            Command::Apply(mut changes) => {
                // Put back whatever a previous trial changed first, so the
                // layout to revert to is the kept one.
                let monitors = match trial.take() {
                    Some(old) => {
                        restore(old.previous, &mut error);
                        read()
                    }
                    None => monitors,
                };
                let previous: Vec<(String, Settings)> =
                    monitors.iter().map(|m| (m.name.clone(), m.current)).collect();
                displays::apply_order(&mut changes);
                error = None;
                send(&changes, &mut error);
                trial = Some(Trial { previous, applied: changes, until: Instant::now() + confirm });
            }
            Command::Keep => {
                if let Some(kept) = trial.take() {
                    keep(&kept, &monitors, &mut error);
                }
            }
            Command::Revert => {
                if let Some(old) = trial.take() {
                    restore(old.previous, &mut error);
                }
            }
            Command::Reapply => {
                // A trial is the user's to end; an event mid-trial (the
                // disabled monitor reported removed) must not undo it.
                if trial.is_none() {
                    let plan = displays::load().plan(&monitors);
                    if !plan.is_empty() {
                        tracing::info!("applying the saved layout to {} monitor(s)", plan.len());
                        send(&plan, &mut error);
                    }
                }
            }
            Command::Forget => {
                trial = None;
                let mut saved = displays::load();
                if saved.forget(&monitors) {
                    if let Err(err) = displays::store(&saved) {
                        error = Some(format!("Could not save: {err}"));
                    }
                }
                // The config's own rules come back with a reload.
                if !crate::hyprland::reload() {
                    error = Some("Hyprland did not reload its config".into());
                }
            }
        }
        publish(&state, &trial, &error);
    }
}

fn read() -> Vec<Monitor> {
    crate::hyprland::monitors_json().map(|json| displays::parse_monitors(&json)).unwrap_or_default()
}

/// Send each rule, recording the first failure, then wait for Hyprland to
/// report them. `eval` answers before the monitors are reconfigured, and a
/// move raises no event, so reading straight away would publish the old
/// layout and keep it. Hyprland may adjust what it was asked (a scale), so
/// the wait is bounded rather than exact.
fn send(changes: &[(String, Settings)], error: &mut Option<String>) {
    for (name, settings) in changes {
        let code = settings.lua_rule(name);
        if let Err(err) = crate::hyprland::eval(&code) {
            tracing::error!("applying {name} failed: {err}");
            error.get_or_insert_with(|| format!("{name}: {err}"));
        }
    }
    const STEP: Duration = Duration::from_millis(50);
    const LIMIT: Duration = Duration::from_millis(1500);
    let start = Instant::now();
    while start.elapsed() < LIMIT {
        let now = read();
        let applied = changes.iter().all(|(name, wanted)| {
            now.iter().find(|m| m.name == *name).is_none_or(|m| !wanted.differs(&m.current))
        });
        if applied {
            return;
        }
        std::thread::sleep(STEP);
    }
}

/// Put back `previous`: every monitor still connected, so none that the
/// config places `auto` is left where the trial's layout pushed it.
fn restore(mut previous: Vec<(String, Settings)>, error: &mut Option<String>) {
    let now = read();
    previous.retain(|(name, _)| now.iter().any(|m| m.name == *name));
    displays::apply_order(&mut previous);
    send(&previous, error);
}

/// Save the layout on trial for the monitors connected. Enabled monitors
/// are saved as Hyprland reports them (it may have adjusted a scale), so
/// applying the saved layout later finds nothing to change; disabled ones
/// as asked, since Hyprland reports nothing useful for them.
fn keep(trial: &Trial, monitors: &[Monitor], error: &mut Option<String>) {
    let settings: Vec<(String, Settings)> = monitors
        .iter()
        .map(|m| {
            let asked = trial.applied.iter().find(|(n, _)| *n == m.name).map(|(_, s)| *s);
            let s = match asked {
                Some(asked) if !asked.enabled => asked,
                _ => m.current,
            };
            (m.name.clone(), s)
        })
        .collect();
    let mut saved = displays::load();
    saved.remember(monitors, &settings);
    if let Err(err) = displays::store(&saved) {
        tracing::error!("could not save the display layout: {err}");
        *error = Some(format!("Could not save: {err}"));
    }
}

fn publish(state: &SystemState, trial: &Option<Trial>, error: &Option<String>) {
    let monitors = read();
    let saved = displays::load().profile(&monitors).is_some();
    let next = DisplaysState {
        available: crate::hyprland::available(),
        monitors,
        saved,
        trial_until: trial.as_ref().map(|t| t.until),
        error: error.clone(),
    };
    if state.displays.set(next) {
        state.notify();
    }
}
