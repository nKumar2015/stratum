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
    /// Every mapped window's global rect, workspace, and focus order, for the
    /// screenshot overlay's window picking.
    pub client_rects: Vec<ClientRect>,
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

/// A window's place on screen, in global layout coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ClientRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub workspace: i32,
    /// 0 is the focused window; higher was focused longer ago.
    pub focus_history: i64,
}

impl HyprState {
    /// The window at global `(x, y)` on one of the given workspaces: the most
    /// recently focused when several overlap (a floating window over a tiled
    /// one).
    pub fn window_at(&self, x: i32, y: i32, workspaces: &[i32]) -> Option<ClientRect> {
        self.client_rects
            .iter()
            .filter(|c| workspaces.contains(&c.workspace))
            .filter(|c| x >= c.x && x < c.x + c.width && y >= c.y && y < c.y + c.height)
            .min_by_key(|c| if c.focus_history < 0 { i64::MAX } else { c.focus_history })
            .copied()
    }
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

/// Whether a Hyprland instance is reachable.
pub fn available() -> bool {
    instance_dir().is_some()
}

/// Every monitor, disabled ones included, as `hyprctl -j monitors all`.
pub fn monitors_json() -> Option<String> {
    query("j/monitors all")
}

/// Run Lua in the compositor (`hyprctl eval`). Hyprland answers `ok`, or
/// the error.
pub fn eval(code: &str) -> Result<(), String> {
    let reply = query(&format!("eval {code}")).ok_or("Hyprland is not reachable")?;
    let reply = reply.trim();
    if reply == "ok" {
        Ok(())
    } else {
        Err(reply.to_string())
    }
}

/// Reload Hyprland's config, which also drops every rule sent with `eval`.
pub fn reload() -> bool {
    query("reload").is_some_and(|reply| reply.trim() == "ok")
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

/// Focus the window at `address` (`0x…`).
pub fn focus_window(address: &str) -> bool {
    // Only hex digits reach the Lua call.
    let hex = address.trim_start_matches("0x");
    if hex.is_empty() || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return false;
    }
    dispatch(&format!("hl.dsp.focus({{ window = \"address:0x{hex}\" }})"))
}

/// The monitors, workspaces and windows, as the overview and the window
/// switcher show them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Desktop {
    pub monitors: Vec<Monitor>,
    /// Ordinary workspaces (not special ones), by id.
    pub workspaces: Vec<Workspace>,
    /// Mapped windows on ordinary workspaces, most recently focused first.
    pub windows: Vec<Window>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Monitor {
    pub name: String,
    /// The monitor's place and size in the layout, in logical pixels.
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub active_workspace: i32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Workspace {
    pub id: i32,
    pub name: String,
    pub monitor: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Window {
    pub address: u64,
    pub class: String,
    pub title: String,
    pub workspace: i32,
    /// In global layout coordinates, logical pixels.
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub floating: bool,
    pub fullscreen: bool,
}

impl Desktop {
    /// The workspaces on `monitor`, by id.
    pub fn workspaces_on(&self, monitor: &str) -> Vec<&Workspace> {
        self.workspaces.iter().filter(|w| w.monitor == monitor).collect()
    }

    pub fn monitor(&self, name: &str) -> Option<&Monitor> {
        self.monitors.iter().find(|m| m.name == name)
    }
}

/// Read the monitors, workspaces and windows. `None` without Hyprland.
pub fn desktop() -> Option<Desktop> {
    let monitors = query("j/monitors")?;
    let workspaces = query("j/workspaces")?;
    let clients = query("j/clients")?;
    Some(parse_desktop(&monitors, &workspaces, &clients))
}

fn parse_desktop(monitors: &str, workspaces: &str, clients: &str) -> Desktop {
    let list = |text: &str| match serde_json::from_str::<serde_json::Value>(text) {
        Ok(serde_json::Value::Array(list)) => list,
        _ => Vec::new(),
    };
    let int = |v: &serde_json::Value| v.as_i64().unwrap_or(0) as i32;
    let text = |v: &serde_json::Value| v.as_str().unwrap_or_default().to_string();
    let monitors = list(monitors)
        .iter()
        .filter(|m| m["disabled"].as_bool() != Some(true))
        .map(|m| {
            let scale = m["scale"].as_f64().filter(|s| *s > 0.0).unwrap_or(1.0);
            let (mut w, mut h) = (m["width"].as_f64().unwrap_or(0.0), m["height"].as_f64().unwrap_or(0.0));
            // A rotated monitor's mode is its unrotated size.
            if int(&m["transform"]) % 2 == 1 {
                std::mem::swap(&mut w, &mut h);
            }
            Monitor {
                name: text(&m["name"]),
                x: int(&m["x"]),
                y: int(&m["y"]),
                width: (w / scale).round() as i32,
                height: (h / scale).round() as i32,
                active_workspace: int(&m["activeWorkspace"]["id"]),
            }
        })
        .collect();
    let mut workspaces: Vec<Workspace> = list(workspaces)
        .iter()
        .map(|w| Workspace { id: int(&w["id"]), name: text(&w["name"]), monitor: text(&w["monitor"]) })
        .filter(|w| w.id > 0)
        .collect();
    workspaces.sort_by_key(|w| w.id);
    let mut windows: Vec<(i64, Window)> = list(clients)
        .iter()
        .filter(|c| c["mapped"].as_bool() != Some(false) && c["hidden"].as_bool() != Some(true))
        .filter_map(|c| {
            let address = u64::from_str_radix(c["address"].as_str()?.trim_start_matches("0x"), 16).ok()?;
            let workspace = int(&c["workspace"]["id"]);
            if workspace <= 0 {
                return None;
            }
            let class = Some(text(&c["class"])).filter(|c| !c.is_empty()).unwrap_or_else(|| text(&c["initialClass"]));
            let window = Window {
                address,
                class,
                title: text(&c["title"]),
                workspace,
                x: int(&c["at"][0]),
                y: int(&c["at"][1]),
                width: int(&c["size"][0]),
                height: int(&c["size"][1]),
                floating: c["floating"].as_bool() == Some(true),
                // 0 none, 1 maximised, 2 fullscreen (older: a bool).
                fullscreen: c["fullscreen"].as_i64().map_or(c["fullscreen"].as_bool() == Some(true), |f| f >= 2),
            };
            Some((c["focusHistoryID"].as_i64().filter(|f| *f >= 0).unwrap_or(i64::MAX), window))
        })
        .collect();
    windows.sort_by_key(|(history, _)| *history);
    Desktop { monitors, workspaces, windows: windows.into_iter().map(|(_, w)| w).collect() }
}

/// Move the window at `address` to workspace `id`, staying where we are.
pub fn move_window(address: u64, id: i32) -> bool {
    dispatch(&format!("hl.dsp.window.move({{ workspace = \"{id}\", follow = false, window = \"address:0x{address:x}\" }})"))
}

/// The mapped windows, most recently focused first, for the launcher.
pub fn clients() -> Vec<crate::launcher::Window> {
    let Some(text) = query("j/clients") else {
        return Vec::new();
    };
    parse_clients(&text)
}

fn parse_clients(text: &str) -> Vec<crate::launcher::Window> {
    let Ok(serde_json::Value::Array(list)) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    let mut windows: Vec<(i64, crate::launcher::Window)> = list
        .iter()
        .filter(|c| c["mapped"].as_bool() != Some(false) && c["hidden"].as_bool() != Some(true))
        .filter_map(|c| {
            Some((
                c["focusHistoryID"].as_i64().unwrap_or(i64::MAX),
                crate::launcher::Window {
                    address: c["address"].as_str()?.to_string(),
                    class: c["class"].as_str().unwrap_or_default().to_string(),
                    title: c["title"].as_str().unwrap_or_default().to_string(),
                    workspace: c["workspace"]["name"].as_str().unwrap_or_default().to_string(),
                },
            ))
        })
        .collect();
    windows.sort_by_key(|(history, _)| *history);
    windows.into_iter().map(|(_, w)| w).collect()
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
                        let pair = |key: &str| {
                            let v = &client[key];
                            (v[0].as_i64().unwrap_or(0) as i32, v[1].as_i64().unwrap_or(0) as i32)
                        };
                        let ((x, y), (width, height)) = (pair("at"), pair("size"));
                        if width > 0 && height > 0 && client["hidden"].as_bool() != Some(true) {
                            state.client_rects.push(ClientRect {
                                x,
                                y,
                                width,
                                height,
                                workspace: id,
                                focus_history: client["focusHistoryID"].as_i64().unwrap_or(i64::MAX),
                            });
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
        // Only these affect what the bar shows.
        const EVENTS: &[&str] = &[
            "workspace",
            "workspacev2",
            "focusedmon",
            "focusedmonv2",
            "activewindow",
            "activewindowv2",
            "openwindow",
            "closewindow",
            "movewindow",
        ];
        watch("hyprland-events", EVENTS, std::time::Duration::from_millis(150), on_change);
    }
}

/// Watch for monitors coming and going and for config reloads, which drop
/// the rules the shell applied. A hotplug arrives as a burst, so events are
/// spaced out as for the bar's.
pub fn spawn_monitor_watcher<F>(on_event: F)
where
    F: Fn() + Send + 'static,
{
    const EVENTS: &[&str] = &["monitoradded", "monitorremoved", "configreloaded"];
    watch("hyprland-monitors", EVENTS, std::time::Duration::from_millis(300), on_event);
}

/// Spawn a thread that reads the event socket and calls `on_event` for each
/// of `events`, at most once per `min_interval`. Bursts are not dropped:
/// the thread waits out the remainder of the interval, and events that
/// arrive meanwhile stay buffered in the socket. Reconnects if the
/// compositor restarts.
fn watch<F>(name: &str, events: &'static [&'static str], min_interval: std::time::Duration, on_event: F)
where
    F: Fn() + Send + 'static,
{
    let Some(dir) = instance_dir() else {
        tracing::warn!("Hyprland instance directory not found; {name} will not be watched");
        return;
    };
    let socket = dir.join(".socket2.sock");

    std::thread::Builder::new()
        .name(name.into())
        .spawn(move || loop {
            match UnixStream::connect(&socket) {
                Ok(stream) => {
                    let reader = BufReader::new(stream);
                    let mut last = std::time::Instant::now() - min_interval;
                    for line in reader.lines() {
                        let Ok(line) = line else { break };
                        let event = line.split(">>").next().unwrap_or_default();
                        if !events.contains(&event) {
                            continue;
                        }
                        if let Some(remaining) = min_interval.checked_sub(last.elapsed()) {
                            std::thread::sleep(remaining);
                        }
                        last = std::time::Instant::now();
                        on_event();
                    }
                }
                Err(err) => {
                    tracing::debug!("Hyprland event socket unavailable: {err}");
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        })
        .ok();
}

#[cfg(test)]
mod tests {

    #[test]
    fn clients_are_listed_by_focus_history() {
        let text = r#"[
            {"address":"0xb","class":"kitty","title":"zsh","workspace":{"name":"2"},"focusHistoryID":1,"mapped":true,"hidden":false},
            {"address":"0xa","class":"zen","title":"Docs","workspace":{"name":"1"},"focusHistoryID":0,"mapped":true,"hidden":false},
            {"address":"0xc","class":"x","title":"hidden","workspace":{"name":"1"},"focusHistoryID":2,"mapped":true,"hidden":true}
        ]"#;
        let windows = parse_clients(text);
        assert_eq!(windows.iter().map(|w| w.address.as_str()).collect::<Vec<_>>(), ["0xa", "0xb"]);
        assert_eq!(windows[1].workspace, "2");
    }

    #[test]
    fn the_desktop_is_read_in_logical_pixels_and_focus_order() {
        let monitors = r#"[
            {"name":"eDP-1","x":0,"y":440,"width":2560,"height":1600,"scale":1.6,"transform":0,"activeWorkspace":{"id":1}},
            {"name":"HDMI-A-1","x":-1440,"y":0,"width":2560,"height":1440,"scale":1,"transform":1,"activeWorkspace":{"id":11}},
            {"name":"OFF","disabled":true}
        ]"#;
        let workspaces = r#"[
            {"id":11,"name":"11","monitor":"HDMI-A-1"},
            {"id":-98,"name":"special:magic","monitor":"eDP-1"},
            {"id":1,"name":"1","monitor":"eDP-1"}
        ]"#;
        let clients = r#"[
            {"address":"0xb","class":"","initialClass":"zen","title":"Docs","workspace":{"id":1},"at":[10,460],"size":[800,600],"floating":true,"fullscreen":0,"focusHistoryID":1,"mapped":true,"hidden":false},
            {"address":"0xa","class":"kitty","title":"zsh","workspace":{"id":11},"at":[0,0],"size":[1440,2560],"floating":false,"fullscreen":2,"focusHistoryID":0,"mapped":true,"hidden":false},
            {"address":"0xc","class":"scratch","title":"","workspace":{"id":-98},"at":[0,0],"size":[1,1],"focusHistoryID":2,"mapped":true,"hidden":false}
        ]"#;
        let desktop = parse_desktop(monitors, workspaces, clients);
        assert_eq!(desktop.monitors.len(), 2);
        let edp = desktop.monitor("eDP-1").unwrap();
        assert_eq!((edp.width, edp.height, edp.active_workspace), (1600, 1000, 1));
        let hdmi = desktop.monitor("HDMI-A-1").unwrap();
        assert_eq!((hdmi.width, hdmi.height), (1440, 2560), "rotated");
        assert_eq!(desktop.workspaces.iter().map(|w| w.id).collect::<Vec<_>>(), [1, 11]);
        assert_eq!(desktop.windows.iter().map(|w| w.address).collect::<Vec<_>>(), [0xa, 0xb]);
        assert!(desktop.windows[0].fullscreen && !desktop.windows[0].floating);
        assert_eq!(desktop.windows[1].class, "zen");
        assert!(desktop.windows[1].floating);
    }

    #[test]
    fn a_window_address_must_be_hex() {
        assert!(!focus_window("0x12\" }) os.execute(\"x"));
        assert!(!focus_window(""));
    }
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
    fn window_at_prefers_the_most_recently_focused_and_ignores_other_workspaces() {
        let rect = |x, y, w, h, ws, fh| ClientRect { x, y, width: w, height: h, workspace: ws, focus_history: fh };
        let state = HyprState {
            client_rects: vec![
                rect(0, 0, 1000, 1000, 1, 2),    // tiled, under
                rect(100, 100, 300, 300, 1, 0),  // floating, focused, on top
                rect(0, 0, 1000, 1000, 5, 1),    // another workspace
            ],
            ..Default::default()
        };
        assert_eq!(state.window_at(150, 150, &[1]).map(|c| c.width), Some(300));
        assert_eq!(state.window_at(800, 800, &[1]).map(|c| c.width), Some(1000));
        assert_eq!(state.window_at(150, 150, &[7]), None);
        assert_eq!(state.window_at(1000, 10, &[1]), None, "right edge is exclusive");
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
