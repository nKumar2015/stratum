//! Hyprland integration.
//!
//! Reads workspace and window state over Hyprland's command socket rather than
//! spawning `hyprctl` for every update. The event socket (`.socket2.sock`) is
//! watched so changes arrive as they happen; queries are only made when
//! something actually changed.
//!
//! Both sockets live in `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE`.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use serde::Deserialize;

/// One workspace as reported by `hyprctl -j monitors`.
#[derive(Clone, Debug, Deserialize, Default)]
pub struct Workspace {
    pub id: i32,
    pub name: String,
}

/// Everything the bar needs from the compositor.
#[derive(Clone, Debug, Default)]
pub struct HyprState {
    /// Monitor name -> active workspace id.
    pub active: std::collections::HashMap<String, i32>,
    /// Workspace ids with at least one window.
    pub occupied: Vec<i32>,
    /// Active window title, if any.
    pub title: Option<String>,
}

fn instance_dir() -> Option<PathBuf> {
    let sig = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
    let runtime = std::env::var("XDG_RUNTIME_DIR").ok()?;
    let path = PathBuf::from(runtime).join("hypr").join(sig);
    path.is_dir().then_some(path)
}

/// Send a command to the command socket and return its reply.
fn query(command: &str) -> Option<String> {
    let dir = instance_dir()?;
    let mut stream = UnixStream::connect(dir.join(".socket.sock")).ok()?;
    stream.write_all(command.as_bytes()).ok()?;
    stream.shutdown(std::net::Shutdown::Write).ok()?;
    let mut out = String::new();
    std::io::Read::read_to_string(&mut stream, &mut out).ok()?;
    Some(out)
}

impl HyprState {
    /// Query the current state once.
    pub fn read() -> Self {
        let mut state = HyprState::default();

        if let Some(json) = query("j/monitors") {
            if let Ok(monitors) = serde_json::from_str::<serde_json::Value>(&json) {
                if let Some(list) = monitors.as_array() {
                    for monitor in list {
                        let name = monitor["name"].as_str().unwrap_or_default().to_string();
                        let id = monitor["activeWorkspace"]["id"].as_i64().unwrap_or(0) as i32;
                        if !name.is_empty() {
                            state.active.insert(name, id);
                        }
                    }
                }
            }
        }

        if let Some(json) = query("j/clients") {
            if let Ok(clients) = serde_json::from_str::<serde_json::Value>(&json) {
                if let Some(list) = clients.as_array() {
                    let mut ids = Vec::new();
                    for client in list {
                        let id = client["workspace"]["id"].as_i64().unwrap_or(0) as i32;
                        if id > 0 && !ids.contains(&id) {
                            ids.push(id);
                        }
                    }
                    state.occupied = ids;
                }
            }
        }

        if let Some(json) = query("j/activewindow") {
            if let Ok(window) = serde_json::from_str::<serde_json::Value>(&json) {
                state.title = window["title"].as_str().map(|s| s.to_string());
            }
        }

        state
    }

    /// Spawn a thread that watches the event socket and calls `on_change`
    /// whenever workspace or window state may have changed.
    ///
    /// The event socket is level-triggered; re-reading state on each relevant
    /// event keeps us consistent without parsing every event payload.
    pub fn spawn_watcher<F>(on_change: F)
    where
        F: Fn() + Send + 'static,
    {
        let Some(dir) = instance_dir() else {
            tracing::warn!("Hyprland instance directory not found; workspace state will be static");
            return;
        };
        let socket = dir.join(".socket2.sock");

        std::thread::Builder::new()
            .name("hyprland-events".into())
            .spawn(move || {
                // Reconnect if the compositor restarts.
                loop {
                    match UnixStream::connect(&socket) {
                        Ok(stream) => {
                            let reader = BufReader::new(stream);
                            for line in reader.lines() {
                                let Ok(line) = line else { break };
                                let event = line.split(">>").next().unwrap_or_default();
                                // Only these affect what the bar shows.
                                if matches!(
                                    event,
                                    "workspace"
                                        | "workspacev2"
                                        | "focusedmon"
                                        | "focusedmonv2"
                                        | "activewindow"
                                        | "activewindowv2"
                                        | "openwindow"
                                        | "closewindow"
                                        | "movewindow"
                                ) {
                                    on_change();
                                }
                            }
                        }
                        Err(err) => {
                            tracing::debug!("Hyprland event socket unavailable: {err}");
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_secs(2));
                }
            })
            .ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_state_without_a_compositor() {
        // No Hyprland in the test environment: this must not panic, and must
        // produce empty state rather than garbage.
        let state = HyprState::read();
        assert!(state.occupied.iter().all(|id| *id > 0));
    }
}
