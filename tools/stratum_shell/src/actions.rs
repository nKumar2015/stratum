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
    /// Switch the default sink.
    SetOutput(String),
    /// Switch the default source.
    SetInput(String),
    /// Power the Bluetooth adapter on or off.
    BluetoothPower(bool),
    /// Connect or disconnect a Bluetooth device by address.
    BluetoothConnect { address: String, connect: bool },
    /// Start a Bluetooth scan.
    BluetoothScan,
    /// Enable or disable the Wi-Fi radio.
    WifiToggle(bool),
    /// Connect to a network by SSID.
    WifiConnect { ssid: String, password: Option<String> },
    /// Disconnect the active connection on a device.
    WifiDisconnect(String),
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
                for action in rx {
                    if let Err(err) = execute(&action, &state) {
                        tracing::warn!("action failed: {err}");
                    }
                    // Wake the UI even on failure: the attempt may have changed
                    // something (a refused connect still reorders the list).
                    state.notify();
                }
            })
            .ok();
    }
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

fn refresh_bluetooth(state: &SystemState) {
    state.bluetooth.set(managers::bluetooth::status());
    state.notify();
}

fn execute(action: &SystemAction, state: &SystemState) -> Result<(), String> {
    let result = match action {
        SystemAction::SetVolume(percent) => managers::audio::set_volume(*percent),
        SystemAction::SetMuted(muted) => managers::audio::set_mute(*muted),
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
        SystemAction::Dismiss(_) | SystemAction::DismissAll | SystemAction::SetTheme(_) => {
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
    };

    // Audio changes come back through the monitor, but that can take a frame or
    // two; read the value we just set so the UI never lags the user.
    if matches!(
        action,
        SystemAction::SetVolume(_)
            | SystemAction::SetMuted(_)
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
    fn spawn_of_missing_program_reports_an_error() {
        let state = SystemState::new();
        let err = execute(
            &SystemAction::Spawn(vec!["/nonexistent/definitely-not-here".into()]),
            &state,
        );
        assert!(err.is_err());
    }
}
