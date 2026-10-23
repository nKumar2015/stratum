//! Actions the UI asks the shell to perform.
//!
//! Widgets are painted on the UI thread and must never block, so actions are
//! queued here and executed by a worker. Results flow back into
//! [`crate::sys_state::SystemState`] through the normal publishers, which means
//! the UI updates the same way for a user action and a background change.
//!
//! Every action refreshes the domain it touched before returning, so the next
//! frame reflects the change without waiting for the poller. That is what makes
//! a toggle feel immediate rather than "eventually correct".

use std::collections::VecDeque;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;

use stratum_sys::managers;

use crate::sys_state::SystemState;

/// One user-initiated operation.
#[derive(Clone, Debug)]
pub enum SystemAction {
    /// Set output volume to an absolute percentage (0..=150).
    SetVolume(i64),
    /// Mute or unmute the default sink.
    SetMuted(bool),
    /// Set the default source's volume to an absolute percentage (0..=150).
    SetInputVolume(i64),
    /// Mute or unmute the default source.
    SetInputMuted(bool),
    /// Switch the default sink.
    SetOutput(String),
    /// Switch the default source.
    SetInput(String),
    /// Power the Bluetooth adapter on or off.
    BluetoothPower(bool),
    /// Connect or disconnect a Bluetooth device by address.
    BluetoothConnect { address: String, connect: bool },
    /// Any other operation on a Bluetooth device.
    BluetoothDevice { address: String, op: BluetoothOp },
    /// Run `action` and publish how it ended under `ticket`
    /// (`SystemState::action_result`), for pages that report outcomes.
    Tracked { ticket: u64, action: Box<SystemAction> },
    /// Start a Bluetooth scan.
    BluetoothScan,
    /// Enable or disable the Wi-Fi radio.
    WifiToggle(bool),
    /// Connect to a network by SSID.
    WifiConnect { ssid: String, password: Option<String> },
    /// Disconnect the active connection on a device.
    WifiDisconnect(String),
    /// Delete a saved connection by name.
    WifiForget(String),
    /// Re-read the Wi-Fi access-point list. `rescan` asks NetworkManager to
    /// probe the airwaves first, which is slow; the fast path only re-reads the
    /// cache.
    WifiRefresh { rescan: bool },
    /// Re-read Bluetooth devices.
    BluetoothRefresh,
    /// Set the platform power profile.
    SetPowerProfile(String),
    /// Run a shell command (power menu entries, lock, and so on).
    Spawn(Vec<String>),
    /// Close one notification by id.
    Dismiss(u32),
    /// Close every notification.
    DismissAll,
    /// Run one of a notification's actions by key.
    InvokeNotification(u32, String),
    /// Load the equalizer presets for an output.
    EqRefresh { device: String },
    /// Apply a stored preset to an output.
    EqApplyPreset { device: String, name: String },
    /// Apply bands that are not (yet) a preset. `bands` is the parametric
    /// array `stratum_sys` parses: `{frequency_hz, gain_db, q, filter_type,
    /// enabled}` objects.
    EqApply {
        device: String,
        bands: serde_json::Value,
        preamp_db: f64,
    },
    /// Save bands as a named custom preset.
    EqSave {
        device: String,
        name: String,
        bands: serde_json::Value,
        preamp_db: f64,
    },
    /// Delete a custom preset.
    EqDelete { device: String, name: String },
    /// A media control for an MPRIS player: `previous`, `play-pause`, `next`.
    Media { player: String, action: String },
    /// Seek an MPRIS player to an absolute position.
    MediaSeek { player: String, seconds: i64 },
    /// Set a player's `shuffle` (`Toggle`, `On`, `Off`) or `loop` (`None`,
    /// `Track`, `Playlist`).
    MediaSet { player: String, property: &'static str, value: String },
    /// Re-read now playing for this player (the dashboard's pick).
    MusicRefresh(Option<String>),
    /// Switch the active theme by name.
    SetTheme(String),
    /// Left-click a tray item.
    TrayActivate { service: String, path: String },
    /// Middle-click a tray item.
    TraySecondaryActivate { service: String, path: String },
    /// Fetch a tray item's DBusMenu and publish it.
    TrayMenuOpen { service: String, path: String },
    /// Invoke one entry of the open tray menu.
    TrayMenuEvent {
        service: String,
        path: String,
        id: i32,
    },
    /// Hide the tray menu.
    TrayMenuClose,
    /// Post a desktop notification, through whichever server holds the
    /// session's (this shell's own, usually).
    Notify { summary: String, body: String },
}

/// What `SystemAction::BluetoothDevice` does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BluetoothOp {
    Pair,
    Forget,
    Connect,
    Disconnect,
    Trust,
    Untrust,
}

/// A fresh ticket for `SystemAction::Tracked`.
pub fn next_ticket() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// A queue the UI writes to and a worker drains.
pub struct ActionQueue {
    tx: Sender<SystemAction>,
    rx: Mutex<Option<Receiver<SystemAction>>>,
}

impl ActionQueue {
    pub fn new() -> Arc<Self> {
        let (tx, rx) = channel();
        Arc::new(Self {
            tx,
            rx: Mutex::new(Some(rx)),
        })
    }

    /// Queue an action. Never blocks and never fails visibly: if the worker is
    /// gone the action is dropped, which is the right outcome during shutdown.
    pub fn push(&self, action: SystemAction) {
        let _ = self.tx.send(action);
    }

    /// Start the worker thread that executes actions in order.
    ///
    /// Actions run one at a time on a dedicated thread because several of them
    /// (Bluetooth, Wi-Fi) are slow and must not stall the UI. The worker writes
    /// straight into [`SystemState`], so a completed action is visible to the
    /// next paint.
    pub fn start_worker(self: &Arc<Self>, state: Arc<SystemState>) {
        let Some(rx) = self.rx.lock().ok().and_then(|mut slot| slot.take()) else {
            return;
        };
        thread::Builder::new()
            .name("stratum-actions".into())
            .spawn(move || {
                let mut backlog = VecDeque::new();
                while let Some(action) = next_action(&rx, &mut backlog) {
                    let (ticket, action) = match action {
                        SystemAction::Tracked { ticket, action } => (Some(ticket), *action),
                        action => (None, action),
                    };
                    // A Wi-Fi connect waits for the whole activation, DHCP
                    // included, which can take tens of seconds; the actions
                    // queued behind it (a volume change) must not wait too.
                    if matches!(action, SystemAction::WifiConnect { .. }) {
                        let state = Arc::clone(&state);
                        let spawned = thread::Builder::new()
                            .name("stratum-wifi-connect".into())
                            .spawn(move || run(ticket, &action, &state));
                        if let Err(err) = spawned {
                            tracing::warn!("action failed: {err}");
                        }
                        continue;
                    }
                    run(ticket, &action, &state);
                }
            })
            .ok();
    }
}

/// The next action worth running, or `None` once the queue has closed.
///
/// A slider drag queues a volume change on every frame, and each one costs
/// several PulseAudio round trips, so running them all left the real volume
/// seconds behind the slider. Only the newest pending value of each kind
/// matters: an older one is dropped when a newer one waits behind it.
fn next_action(rx: &Receiver<SystemAction>, backlog: &mut VecDeque<SystemAction>) -> Option<SystemAction> {
    loop {
        if backlog.is_empty() {
            backlog.push_back(rx.recv().ok()?);
        }
        backlog.extend(rx.try_iter());
        let action = backlog.pop_front()?;
        if !backlog.iter().any(|later| supersedes(later, &action)) {
            return Some(action);
        }
    }
}

/// Whether `later` makes `earlier` pointless to run. Tracked actions are
/// always run, since a page waits for their outcome.
fn supersedes(later: &SystemAction, earlier: &SystemAction) -> bool {
    matches!(
        (later, earlier),
        (SystemAction::SetVolume(_), SystemAction::SetVolume(_))
            | (SystemAction::SetInputVolume(_), SystemAction::SetInputVolume(_))
    )
}

/// Execute one action and publish how it ended.
fn run(ticket: Option<u64>, action: &SystemAction, state: &SystemState) {
    let result = execute(action, state);
    if let Err(err) = &result {
        tracing::warn!("action failed: {err}");
    }
    if let Some(ticket) = ticket {
        state.action_result.set(crate::sys_state::ActionResult {
            ticket,
            error: result.err(),
        });
    }
    // Wake the UI even on failure: the attempt may have changed
    // something (a refused connect still reorders the list).
    state.notify();
}

impl Default for ActionQueue {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self {
            tx,
            rx: Mutex::new(Some(rx)),
        }
    }
}

/// Refresh the audio snapshot after a change.
fn refresh_audio(state: &SystemState) {
    if let Some(snapshot) = managers::audio::snapshot() {
        state.audio.set(snapshot);
    }
}

/// Refresh network connection state and, when the radio is up, the access-point
/// list as well.
fn refresh_network(state: &SystemState, rescan: bool) {
    state.net.set(managers::net::status());
    let wifi = managers::wifi::state();
    let enabled = wifi["enabled"].as_bool().unwrap_or(false);
    state.wifi.set(wifi);
    state.wifi_details.set(managers::wifi::details());
    if enabled {
        state.wifi_networks.set(managers::wifi::list(rescan));
    } else {
        // A disabled radio has no list; clearing it avoids showing stale
        // networks that cannot be joined.
        state
            .wifi_networks
            .set(serde_json::json!({ "ok": true, "networks": [] }));
    }
    state.notify();
}

/// Publish the output's presets, with the outcome of the action that just ran
/// (`result` and the message to show when it succeeded).
fn publish_eq(state: &SystemState, device: &str, outcome: Option<(&serde_json::Value, &str)>) {
    let mut snapshot = managers::audio::eq_list_presets(device);
    let (ok, status) = match outcome {
        None => (true, String::new()),
        Some((result, done)) => {
            let ok = result["ok"].as_bool() == Some(true);
            let status = if ok {
                done.to_string()
            } else {
                result["error"].as_str().unwrap_or("EQ action failed").to_string()
            };
            (ok, status)
        }
    };
    snapshot["ok"] = serde_json::json!(ok);
    snapshot["status"] = serde_json::json!(status);
    state.eq.set(snapshot);
    state.notify();
}

fn refresh_bluetooth(state: &SystemState) {
    state.bluetooth.set(managers::bluetooth::status_with_all());
    state.notify();
}

fn execute(action: &SystemAction, state: &SystemState) -> Result<(), String> {
    let result = match action {
        SystemAction::SetVolume(percent) => managers::audio::set_volume(*percent),
        SystemAction::SetMuted(muted) => managers::audio::set_mute(*muted),
        SystemAction::SetInputVolume(percent) => managers::audio::set_input_volume(*percent),
        SystemAction::SetInputMuted(muted) => managers::audio::set_input_mute(*muted),
        SystemAction::SetOutput(target) => managers::audio::set_output(target),
        SystemAction::SetInput(target) => managers::audio::set_input(target),
        SystemAction::BluetoothPower(on) => {
            let result = managers::bluetooth::power(if *on { "on" } else { "off" });
            refresh_bluetooth(state);
            result
        }
        SystemAction::BluetoothConnect { address, connect } => {
            let result = if *connect {
                managers::bluetooth::connect(address)
            } else {
                managers::bluetooth::disconnect(address)
            };
            refresh_bluetooth(state);
            result
        }
        SystemAction::BluetoothDevice { address, op } => {
            use managers::bluetooth;
            let result = match op {
                BluetoothOp::Pair => bluetooth::pair(address),
                BluetoothOp::Forget => bluetooth::forget(address),
                BluetoothOp::Connect => bluetooth::connect(address),
                BluetoothOp::Disconnect => bluetooth::disconnect(address),
                BluetoothOp::Trust => bluetooth::trust(address),
                BluetoothOp::Untrust => bluetooth::untrust(address),
            };
            refresh_bluetooth(state);
            result
        }
        SystemAction::Tracked { action, .. } => {
            // Unwrapped by the worker; a nested one just runs.
            return execute(action, state);
        }
        SystemAction::BluetoothScan => {
            let result = managers::bluetooth::scan();
            refresh_bluetooth(state);
            result
        }
        SystemAction::BluetoothRefresh => {
            refresh_bluetooth(state);
            serde_json::json!({ "ok": true })
        }
        SystemAction::WifiToggle(on) => {
            let result = managers::wifi::toggle(if *on { "on" } else { "off" });
            // A radio toggle changes both the radio state and the list; refresh
            // before returning so the panel flips on this frame.
            refresh_network(state, false);
            result
        }
        SystemAction::WifiConnect { ssid, password } => {
            let result = managers::wifi::connect(ssid, password.as_deref());
            refresh_network(state, false);
            result
        }
        SystemAction::WifiDisconnect(device) => {
            let result = managers::wifi::disconnect(device);
            refresh_network(state, false);
            result
        }
        SystemAction::WifiForget(name) => {
            let result = managers::wifi::forget(name);
            refresh_network(state, false);
            result
        }
        SystemAction::WifiRefresh { rescan } => {
            refresh_network(state, *rescan);
            serde_json::json!({ "ok": true })
        }
        SystemAction::SetPowerProfile(profile) => {
            // Writes the ACPI platform profile directly; the CLI does the same.
            std::fs::write("/sys/firmware/acpi/platform_profile", profile)
                .map_err(|err| format!("failed to set power profile: {err}"))?;
            state.battery.set(managers::battery::status());
            serde_json::json!({ "ok": true })
        }
        SystemAction::Dismiss(_)
        | SystemAction::DismissAll
        | SystemAction::InvokeNotification(..)
        | SystemAction::SetTheme(_) => {
            // Handled by the shell, which owns the notification server; the
            // worker has nothing to do.
            return Ok(());
        }
        SystemAction::TrayActivate { service, path }
        | SystemAction::TraySecondaryActivate { service, path } => {
            let tray = state
                .tray_handle()
                .ok_or_else(|| "tray watcher is not running".to_string())?;
            let result = if matches!(action, SystemAction::TrayActivate { .. }) {
                tray.activate(service, path)
            } else {
                tray.secondary_activate(service, path)
            };
            match result {
                Ok(()) => serde_json::json!({ "ok": true }),
                Err(err) => serde_json::json!({ "ok": false, "error": err }),
            }
        }
        SystemAction::TrayMenuOpen { service, path } => {
            let tray = state
                .tray_handle()
                .ok_or_else(|| "tray watcher is not running".to_string())?;
            match tray.menu(service, path) {
                Ok(entries) => {
                    state.tray_menu.set(Some(crate::tray::TrayMenu {
                        service: service.clone(),
                        path: path.clone(),
                        entries,
                    }));
                    state.notify();
                    serde_json::json!({ "ok": true })
                }
                Err(err) => {
                    // A menu that fails to load must still dismiss, or the
                    // overlay stays on screen with stale content.
                    state.tray_menu.set(None);
                    state.notify();
                    serde_json::json!({ "ok": false, "error": err })
                }
            }
        }
        SystemAction::TrayMenuEvent { service, path, id } => {
            let tray = state
                .tray_handle()
                .ok_or_else(|| "tray watcher is not running".to_string())?;
            let result = tray.menu_event(service, path, *id);
            // Any click closes the menu, the way the QML did.
            state.tray_menu.set(None);
            state.notify();
            match result {
                Ok(()) => serde_json::json!({ "ok": true }),
                Err(err) => serde_json::json!({ "ok": false, "error": err }),
            }
        }
        SystemAction::TrayMenuClose => {
            state.tray_menu.set(None);
            state.notify();
            serde_json::json!({ "ok": true })
        }
        SystemAction::EqRefresh { device } => {
            publish_eq(state, device, None);
            serde_json::json!({ "ok": true })
        }
        SystemAction::EqApplyPreset { device, name } => {
            let result = managers::audio::eq_apply_preset(device, name);
            publish_eq(state, device, Some((&result, &format!("Applied \u{201c}{name}\u{201d}"))));
            result
        }
        SystemAction::EqApply {
            device,
            bands,
            preamp_db,
        } => {
            let result = managers::audio::eq_apply_parametric(device, bands, *preamp_db);
            publish_eq(state, device, Some((&result, "Applied")));
            result
        }
        SystemAction::EqSave { device, name, bands, preamp_db } => {
            let result = managers::audio::eq_save_preset_parametric(device, name, bands, *preamp_db);
            publish_eq(state, device, Some((&result, &format!("Saved \u{201c}{name}\u{201d}"))));
            result
        }
        SystemAction::EqDelete { device, name } => {
            let result = managers::audio::eq_delete_preset(device, name);
            publish_eq(state, device, Some((&result, &format!("Deleted \u{201c}{name}\u{201d}"))));
            result
        }
        SystemAction::Media { player, action } => {
            // The QML shell's MusicProvider sent the same playerctl commands.
            let status = std::process::Command::new("playerctl")
                .args(["-p", player, action])
                .status()
                .map_err(|err| format!("failed to run playerctl: {err}"))?;
            if !status.success() {
                return Err(format!("playerctl {action} failed for {player}"));
            }
            // Show the new state without waiting for the next poll, for the
            // player acted on rather than whichever plays.
            state.music.set(managers::music::status_for(Some(player)));
            state.notify();
            serde_json::json!({ "ok": true })
        }
        SystemAction::MediaSet { player, property, value } => {
            let status = std::process::Command::new("playerctl")
                .args(["-p", player.as_str(), property, value.as_str()])
                .status()
                .map_err(|err| format!("failed to run playerctl: {err}"))?;
            if !status.success() {
                return Err(format!("playerctl {property} {value} failed for {player}"));
            }
            state.music.set(managers::music::status_for(Some(player)));
            state.notify();
            serde_json::json!({ "ok": true })
        }
        SystemAction::MusicRefresh(player) => {
            state.music.set(managers::music::status_for(player.as_deref()));
            state.notify();
            serde_json::json!({ "ok": true })
        }
        SystemAction::MediaSeek { player, seconds } => {
            // `playerctl position` takes seconds, as the QML seek did.
            let position = (*seconds).max(0).to_string();
            let status = std::process::Command::new("playerctl")
                .args(["-p", player.as_str(), "position", position.as_str()])
                .status()
                .map_err(|err| format!("failed to run playerctl: {err}"))?;
            if !status.success() {
                return Err(format!("playerctl seek failed for {player}"));
            }
            state.music.set(managers::music::status_for(Some(player)));
            state.notify();
            serde_json::json!({ "ok": true })
        }
        SystemAction::Spawn(argv) => {
            let Some((program, args)) = argv.split_first() else {
                return Err("empty command".into());
            };
            std::process::Command::new(program)
                .args(args)
                .spawn()
                .map_err(|err| format!("failed to spawn {program}: {err}"))?;
            serde_json::json!({ "ok": true })
        }
        SystemAction::Notify { summary, body } => {
            // Over the bus even to this shell's own server, so DND, the
            // toast and the notification centre treat it like any other.
            let bus = zbus::blocking::Connection::session().map_err(|err| format!("no session bus: {err}"))?;
            let hints = std::collections::HashMap::from([("urgency", zbus::zvariant::Value::U8(1))]);
            bus.call_method(
                Some("org.freedesktop.Notifications"),
                "/org/freedesktop/Notifications",
                Some("org.freedesktop.Notifications"),
                "Notify",
                &("Stratum", 0u32, "alarm-symbolic", summary, body, Vec::<&str>::new(), hints, -1i32),
            )
            .map_err(|err| format!("notify failed: {err}"))?;
            serde_json::json!({ "ok": true })
        }
    };

    // Audio changes come back through the monitor, but that can take a frame or
    // two; read the value we just set so the UI never lags the user.
    if matches!(
        action,
        SystemAction::SetVolume(_)
            | SystemAction::SetMuted(_)
            | SystemAction::SetInputVolume(_)
            | SystemAction::SetInputMuted(_)
            | SystemAction::SetOutput(_)
            | SystemAction::SetInput(_)
    ) {
        refresh_audio(state);
    }

    if result.get("ok").and_then(|v| v.as_bool()) == Some(false) {
        return Err(result
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("unknown error")
            .to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_accepts_actions_without_a_worker() {
        let queue = ActionQueue::new();
        // No worker started: pushing must not panic or block.
        queue.push(SystemAction::SetVolume(50));
        queue.push(SystemAction::Spawn(vec!["true".into()]));
    }

    #[test]
    fn only_the_newest_pending_volume_runs() {
        let (tx, rx) = channel();
        for action in [
            SystemAction::SetVolume(10),
            SystemAction::SetInputVolume(5),
            SystemAction::SetVolume(20),
            SystemAction::SetMuted(true),
            SystemAction::SetVolume(30),
            SystemAction::SetInputVolume(6),
        ] {
            tx.send(action).unwrap();
        }
        drop(tx);
        let mut backlog = VecDeque::new();
        let mut ran = Vec::new();
        while let Some(action) = next_action(&rx, &mut backlog) {
            ran.push(format!("{action:?}"));
        }
        assert_eq!(ran, ["SetMuted(true)", "SetVolume(30)", "SetInputVolume(6)"]);
    }

    #[test]
    fn spawn_of_missing_program_reports_an_error() {
        let state = SystemState::new();
        let err = execute(
            &SystemAction::Spawn(vec!["/nonexistent/definitely-not-here".into()]),
            &state,
        );
        assert!(err.is_err());
    }
}
