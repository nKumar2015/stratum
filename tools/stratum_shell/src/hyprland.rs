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
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HyprState {
    /// Monitor name -> active workspace id.
    pub active: std::collections::HashMap<String, i32>,
    /// Workspace ids with at least one window.
    pub occupied: Vec<i32>,
    /// Active window title, if any.
    pub title: Option<String>,
    /// Monitor name -> the window its bar shows: the most recently focused
    /// window on that monitor's active workspace. Each monitor shows its own
    /// window, as the QML title did, not the globally focused one.
    pub windows: std::collections::HashMap<String, WindowInfo>,
    /// Monitor the compositor currently has focused.
    ///
    /// Popups open here rather than on an arbitrary output, matching the QML
    /// shell's `popupMonitorName` routing.
    pub focused_monitor: Option<String>,
}

/// A window as the bar shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WindowInfo {
    pub title: String,
    /// Window class (app id), for the icon.
    pub class: String,
}

/// One client from `j/clients`, reduced to what window selection needs.
#[derive(Clone, Debug)]
struct Client {
    workspace: i32,
    /// 0 is the focused window; higher was focused longer ago.
    focus_history: i64,
    info: WindowInfo,
}

/// Pick each monitor's window: the most recently focused client on its active
/// workspace. That is the focused window when it is there, which is the rule
/// the QML title used.
fn pick_windows(
    active: &std::collections::HashMap<String, i32>,
    clients: &[Client],
) -> std::collections::HashMap<String, WindowInfo> {
    active
        .iter()
        .filter_map(|(monitor, workspace)| {
            clients
                .iter()
                .filter(|c| c.workspace == *workspace)
                .min_by_key(|c| if c.focus_history < 0 { i64::MAX } else { c.focus_history })
                .map(|c| (monitor.clone(), c.info.clone()))
        })
        .collect()
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

/// Send a dispatch command to the compositor.
pub fn dispatch(command: &str) -> bool {
    query(&format!("dispatch {}", command)).is_some()
}

/// Focus a workspace by id, creating it if necessary.
///
/// Hyprland 0.56 routes dispatch through Lua, so the argument is a Lua call
/// expression rather than the older `workspace <id>` shorthand.
pub fn focus_workspace(id: i32) -> bool {
    dispatch(&format!("hl.dsp.focus({{ workspace = \"{id}\" }})"))
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
                            if monitor["focused"].as_bool() == Some(true) {
                                state.focused_monitor = Some(name.clone());
                            }
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
                    let mut windows = Vec::new();
                    for client in list {
                        let id = client["workspace"]["id"].as_i64().unwrap_or(0) as i32;
                        if id > 0 && !ids.contains(&id) {
                            ids.push(id);
                        }
                        if client["mapped"].as_bool() == Some(false) {
                            continue;
                        }
                        let text = |key: &str| client[key].as_str().unwrap_or_default().to_string();
                        let class = Some(text("class"))
                            .filter(|c| !c.is_empty())
                            .unwrap_or_else(|| text("initialClass"));
                        windows.push(Client {
                            workspace: id,
                            focus_history: client["focusHistoryID"].as_i64().unwrap_or(i64::MAX),
                            info: WindowInfo {
                                title: text("title"),
                                class,
                            },
                        });
                    }
                    state.occupied = ids;
                    state.windows = pick_windows(&state.active, &windows);
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
    ///
    /// Events are coalesced: applications that animate their window title (a
    /// terminal's spinner, a browser's loading indicator) emit `activewindow`
    /// several times a second, and each one would otherwise cost three socket
    /// queries plus a full repaint.
    pub fn spawn_watcher<F>(on_change: F)
    where
        F: Fn() + Send + 'static,
    {
        /// Minimum spacing between state reads.
        const MIN_INTERVAL: std::time::Duration = std::time::Duration::from_millis(150);

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
                            let mut last = std::time::Instant::now()
                                - MIN_INTERVAL;
                            for line in reader.lines() {
                                let Ok(line) = line else { break };
                                let event = line.split(">>").next().unwrap_or_default();
                                // Only these affect what the bar shows.
                                if !matches!(
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
                                    continue;
                                }
                                // Coalesce bursts: wait out the remainder of the
                                // cooldown instead of dropping the event, so a
                                // rapid workspace switch still updates. Events
                                // that arrive meanwhile stay buffered in the
                                // socket and are read after the wait.
                                if let Some(remaining) = MIN_INTERVAL.checked_sub(last.elapsed()) {
                                    std::thread::sleep(remaining);
                                }
                                last = std::time::Instant::now();
                                on_change();
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

    fn client(workspace: i32, focus_history: i64, title: &str) -> Client {
        Client {
            workspace,
            focus_history,
            info: WindowInfo {
                title: title.into(),
                class: title.to_lowercase(),
            },
        }
    }

    #[test]
    fn each_monitor_shows_its_own_workspace_window() {
        let active = [("eDP-2".to_string(), 1), ("HDMI-A-1".to_string(), 11)]
            .into_iter()
            .collect();
        let clients = [
            client(11, 0, "Kitty"),   // focused, on the external monitor
            client(1, 3, "Firefox"),  // older focus on the laptop's workspace
            client(1, 1, "Zen"),      // more recent on the laptop's workspace
            client(5, 2, "Hidden"),   // not on any active workspace
        ];
        let windows = pick_windows(&active, &clients);
        assert_eq!(windows["HDMI-A-1"].title, "Kitty");
        assert_eq!(windows["eDP-2"].title, "Zen", "most recently focused wins");
        assert_eq!(windows.len(), 2);
    }

    #[test]
    fn an_empty_workspace_has_no_window() {
        let active = [("eDP-2".to_string(), 4)].into_iter().collect();
        assert!(pick_windows(&active, &[client(1, 0, "Kitty")]).is_empty());
    }

    #[test]
    fn reads_state_without_a_compositor() {
        // No Hyprland in the test environment: this must not panic, and must
        // produce empty state rather than garbage.
        let state = HyprState::read();
        assert!(state.occupied.iter().all(|id| *id > 0));
    }
}
