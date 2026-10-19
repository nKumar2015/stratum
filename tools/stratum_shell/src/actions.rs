//! Actions the UI asks the shell to perform.
//!
//! Widgets are painted on the UI thread and must never block, so actions are
//! queued here and executed by a worker. Results flow back into
//! [`crate::sys_state::SystemState`] through the normal publishers, which means
//! the UI updates the same way for a user action and a background change.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Mutex;
use std::sync::Arc;
use std::thread;

use stratum_sys::managers;

/// One user-initiated operation.
#[derive(Clone, Debug)]
pub enum SystemAction {
    /// Set output volume to an absolute percentage (0..=150).
    SetVolume(i64),
    /// Toggle mute on the default sink. `muted` is the current state, which
    /// decides whether this unmutes or mutes.
    ToggleMute { muted: bool },
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
    /// Set the platform power profile.
    SetPowerProfile(String),
    /// Run a shell command (power menu entries, lock, and so on).
    Spawn(Vec<String>),
    /// Close one notification by id.
    Dismiss(u32),
    /// Close every notification.
    DismissAll,
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
    /// (Bluetooth, Wi-Fi) are slow and must not stall the UI.
    pub fn start_worker(self: &Arc<Self>) {
        let Some(rx) = self.rx.lock().ok().and_then(|mut slot| slot.take()) else {
            return;
        };
        // Take a weak reference so the worker does not keep the queue alive.
        thread::Builder::new()
            .name("stratum-actions".into())
            .spawn(move || {
                for action in rx {
                    if let Err(err) = execute(&action) {
                        tracing::warn!("action failed: {err}");
                    }
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

/// Execute one action. Returns an error message for logging.
fn execute(action: &SystemAction) -> Result<(), String> {
    let result = match action {
        SystemAction::SetVolume(percent) => managers::audio::set_volume(*percent),
        SystemAction::ToggleMute { muted } => {
            // Absolute mute-switching lives in the manager's engine; until it
            // exposes a toggle, drive it through volume so the UI stays honest.
            if *muted {
                managers::audio::set_volume(50)
            } else {
                managers::audio::set_volume(0)
            }
        }
        SystemAction::SetOutput(target) => managers::audio::set_output(target),
        SystemAction::SetInput(target) => managers::audio::set_input(target),
        SystemAction::BluetoothPower(on) => {
            managers::bluetooth::power(if *on { "on" } else { "off" })
        }
        SystemAction::BluetoothConnect { address, connect } => {
            if *connect {
                managers::bluetooth::connect(address)
            } else {
                managers::bluetooth::disconnect(address)
            }
        }
        SystemAction::BluetoothScan => managers::bluetooth::scan(),
        SystemAction::WifiToggle(on) => {
            managers::wifi::toggle(if *on { "on" } else { "off" })
        }
        SystemAction::WifiConnect { ssid, password } => {
            managers::wifi::connect(ssid, password.as_deref())
        }
        SystemAction::WifiDisconnect(device) => managers::wifi::disconnect(device),
        SystemAction::SetPowerProfile(profile) => {
            // Writes the ACPI platform profile directly; the CLI does the same.
            std::fs::write("/sys/firmware/acpi/platform_profile", profile)
                .map_err(|err| format!("failed to set power profile: {err}"))?;
            serde_json::json!({ "ok": true })
        }
        SystemAction::Dismiss(_) | SystemAction::DismissAll => {
            // Handled by the shell, which owns the notification server; the
            // worker has nothing to do.
            return Ok(());
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
        let err = execute(&SystemAction::Spawn(vec![
            "/nonexistent/definitely-not-here".into(),
        ]));
        assert!(err.is_err());
    }
}
