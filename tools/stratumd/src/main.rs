use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::oneshot;

use tracing::{debug, error, info, warn};

mod logger;

use chrono::Datelike;
use serde_json::{json, Value};
use std::collections::HashMap;

mod managers {
    pub mod audio;
    pub mod battery;
    pub mod bluetooth;
    pub mod common;
    pub mod dashboard;
    pub mod music;
    pub mod net;
    pub mod polkit;
    pub mod wifi;
}

mod ipc;
mod state;

use state::{
    AudioState, BatteryState, BluetoothState, DashboardState, MusicState, NetState, PolkitState,
};

struct AuthSession {
    tx: oneshot::Sender<bool>,
    attempts: u32,
}

struct AppState {
    started_at: Instant,
    dashboard_watch: Mutex<Option<(i32, u32)>>,
    audio: AudioState,
    battery: BatteryState,
    net: NetState,
    bluetooth: BluetoothState,
    music: MusicState,
    dashboard: DashboardState,
    polkit: PolkitState,
    qs_pid_cache: Mutex<Option<u32>>,
    pending_auths: Mutex<HashMap<String, AuthSession>>,
}

fn socket_path() -> PathBuf {
    let runtime_dir = env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(runtime_dir).join("stratumd.sock")
}

fn unique_push(list: &mut Vec<PathBuf>, path: PathBuf) {
    if !list.iter().any(|p| p == &path) {
        list.push(path);
    }
}

fn binary_in_path(binary: &str) -> Option<PathBuf> {
    let path_var = env::var("PATH").ok()?;
    for segment in path_var.split(':') {
        if segment.is_empty() {
            continue;
        }
        let candidate = Path::new(segment).join(binary);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn binary_candidates(binary: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Some(path) = binary_in_path(binary) {
        unique_push(&mut candidates, path);
    }

    if let Ok(home) = env::var("HOME") {
        unique_push(
            &mut candidates,
            Path::new(&home)
                .join(".nix-profile")
                .join("bin")
                .join(binary),
        );
        unique_push(
            &mut candidates,
            Path::new(&home)
                .join(".local")
                .join("state")
                .join("nix")
                .join("profile")
                .join("bin")
                .join(binary),
        );
    }

    candidates
}

fn resolve_qs_binary() -> Option<PathBuf> {
    binary_candidates("qs").into_iter().find(|p| p.is_file())
}

fn resolve_shell_binary() -> Option<PathBuf> {
    for bin in ["qs", "quickshell"] {
        if let Some(path) = binary_candidates(bin).into_iter().find(|p| p.is_file()) {
            return Some(path);
        }
    }
    None
}

fn quickshell_already_running() -> bool {
    let Some(qs_bin) = resolve_qs_binary() else {
        return false;
    };

    let output = Command::new(qs_bin)
        .args(["list", "--all", "--json"])
        .output();

    let Ok(output) = output else {
        return false;
    };

    if !output.status.success() {
        return false;
    }

    let raw = String::from_utf8_lossy(&output.stdout);
    match serde_json::from_str::<Value>(&raw) {
        Ok(Value::Array(entries)) => !entries.is_empty(),
        _ => false,
    }
}

fn maybe_launch_quickshell() {
    if quickshell_already_running() {
        return;
    }

    let Some(shell_bin) = resolve_shell_binary() else {
        error!("[stratumd] [launch] Failed to launch quickshell: neither 'qs' nor 'quickshell' could be resolved");
        return;
    };

    let log_path = logger::get_shell_log_path();
    let log_file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path);

    let (stdout, stderr) = match log_file {
        Ok(f) => {
            info!(
                "[stratumd] [launch] Redirecting shell output to {}",
                log_path.display()
            );
            (Stdio::from(f.try_clone().unwrap()), Stdio::from(f))
        }
        Err(e) => {
            warn!(
                "[stratumd] [launch] Failed to open shell log file {}, using null: {}",
                log_path.display(),
                e
            );
            (Stdio::null(), Stdio::null())
        }
    };

    info!("[stratumd] [launch] Launching quickshell: {:?}", shell_bin);
    if let Err(err) = Command::new("setsid")
        .arg(shell_bin)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr)
        .spawn()
    {
        error!(
            "[stratumd] [launch] failed to launch quickshell (detached): {}",
            err
        );
    }
}

fn spawn_net_monitor(state: Arc<AppState>) {
    tokio::task::spawn_blocking(move || {
        info!("Starting network monitor");
        loop {
            let snapshot = managers::net::status();
            state.net.update(snapshot);
            thread::sleep(Duration::from_secs(3));
        }
    });
}

fn spawn_bluetooth_monitor(state: Arc<AppState>) {
    tokio::task::spawn_blocking(move || {
        info!("Starting bluetooth monitor");
        loop {
            let snapshot = managers::bluetooth::status();
            state.bluetooth.update(snapshot);
            thread::sleep(Duration::from_secs(3));
        }
    });
}

fn spawn_music_monitor(state: Arc<AppState>) {
    tokio::task::spawn_blocking(move || {
        info!("Starting music monitor");
        loop {
            let snapshot = managers::music::status();
            state.music.update(snapshot);
            thread::sleep(Duration::from_secs(2));
        }
    });
}

fn spawn_dashboard_monitor(state: Arc<AppState>) {
    tokio::task::spawn_blocking(move || {
        info!("Starting dashboard monitor");
        loop {
            let watched = state.dashboard_watch.lock().ok().and_then(|guard| *guard);

            let (center_year, center_month) = if let Some((y, m)) = watched {
                (y, m)
            } else {
                let (y, m, _) = managers::dashboard::current_date_parts();
                (y, m)
            };

            // Maintain a 7-month sliding window [-3..+3]
            for delta in -3..=3 {
                let (y, m) = managers::dashboard::adjust_month(center_year, center_month, delta);

                // Only refresh the center month's performance data every 2s
                // Side months (calendar only) don't need frequent refreshes
                if delta == 0 {
                    let snapshot = managers::dashboard::status(y, m as i32);
                    state.dashboard.update(snapshot);
                } else {
                    // Just ensure the calendar payload is cached
                    managers::dashboard::calendar_payload(y, m);
                }
            }

            // Sleep 2s between maintenance cycles
            thread::sleep(Duration::from_secs(2));
        }
    });
}

fn spawn_battery_monitor(state: Arc<AppState>) {
    tokio::task::spawn_blocking(move || {
        info!("Starting battery monitor");
        loop {
            let snapshot = managers::battery::status();
            state.battery.update(snapshot);
            thread::sleep(Duration::from_secs(5));
        }
    });
}

fn spawn_polkit_agent(state: Arc<AppState>) {
    tokio::spawn(async move {
        if let Err(e) = managers::polkit::register_agent(state).await {
            error!("[polkit] failed to register authentication agent: {}", e);
        } else {
            info!("[polkit] authentication agent registered successfully");
        }
    });
}

// --- Single broadcaster thread ---

fn spawn_broadcaster(state: Arc<AppState>) {
    tokio::task::spawn_blocking(move || {
        info!("Starting broadcaster thread");
        // Wait for Quickshell to become available before first push
        for _ in 0..20 {
            if ipc::newest_quickshell_pid().is_ok() {
                break;
            }
            thread::sleep(Duration::from_millis(500));
        }

        loop {
            let pid = {
                let cached = state.qs_pid_cache.lock().ok().and_then(|g| *g);
                match cached {
                    Some(pid) => pid,
                    None => match ipc::newest_quickshell_pid() {
                        Ok(pid) => {
                            if let Ok(mut cache) = state.qs_pid_cache.lock() {
                                *cache = Some(pid);
                            }
                            pid
                        }
                        Err(_) => {
                            thread::sleep(Duration::from_millis(500));
                            continue;
                        }
                    },
                }
            };

            let mut any_failed = false;

            if let Some(payload) = state.audio.take_if_dirty() {
                debug!("Broadcasting audio update");
                let payload_text = payload.to_string();
                if ipc::send_shell_ipc_with_pid(pid, "daemon", "audio", &[payload_text]).is_err() {
                    any_failed = true;
                }
            }

            if let Some(payload) = state.net.take_if_dirty() {
                debug!("Broadcasting network update");
                let payload_text = payload.to_string();
                if ipc::send_shell_ipc_with_pid(pid, "daemon", "wifi", &[payload_text]).is_err() {
                    any_failed = true;
                }
            }

            if let Some(payload) = state.bluetooth.take_if_dirty() {
                debug!("Broadcasting bluetooth update");
                let payload_text = payload.to_string();
                if ipc::send_shell_ipc_with_pid(pid, "daemon", "bluetooth", &[payload_text])
                    .is_err()
                {
                    any_failed = true;
                }
            }

            if let Some(payload) = state.music.take_if_dirty() {
                debug!("Broadcasting music update");
                let payload_text = payload.to_string();
                if ipc::send_shell_ipc_with_pid(pid, "daemon", "music", &[payload_text]).is_err() {
                    any_failed = true;
                }
            }

            if let Some(payload) = state.dashboard.take_if_dirty() {
                debug!("Broadcasting dashboard update");
                let payload_text = payload.to_string();
                if ipc::send_shell_ipc_with_pid(pid, "daemon", "dashboard", &[payload_text])
                    .is_err()
                {
                    any_failed = true;
                }
            }

            if let Some(payload) = state.battery.take_if_dirty() {
                debug!("Broadcasting battery update");
                let payload_text = payload.to_string();
                if ipc::send_shell_ipc_with_pid(pid, "daemon", "battery", &[payload_text]).is_err()
                {
                    any_failed = true;
                }
            }

            if let Some(payload) = state.polkit.take_if_dirty() {
                info!("[polkit] Broadcasting update to shell");
                let payload_text = payload.to_string();
                if ipc::send_shell_ipc_with_pid(pid, "daemon", "polkit", &[payload_text]).is_err() {
                    any_failed = true;
                }
            }

            // Invalidate PID cache on any IPC failure so we re-resolve next tick
            if any_failed {
                if let Ok(mut cache) = state.qs_pid_cache.lock() {
                    *cache = None;
                }
            }

            // Sleep 50ms between broadcast ticks for higher responsiveness (especially for Polkit)
            thread::sleep(Duration::from_millis(50));
        }
    });
}

// --- JSON-RPC ---

fn jsonrpc_success(id: Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    })
}

fn jsonrpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message,
        }
    })
}

fn param_string(params: Option<&Value>, key: &str) -> Result<String, String> {
    // We chain the Options. If any step returns None, the 'else' block triggers.
    let Some(value) = params.and_then(|p| p.get(key)).and_then(Value::as_str) else {
        return Err(format!("missing or invalid string params.{}", key));
    };

    // No .trim() here! We need the raw string for PAM.
    Ok(value.to_string())
}

fn param_i64(params: Option<&Value>, key: &str, default: i64) -> i64 {
    params
        .and_then(|p| p.get(key))
        .and_then(Value::as_i64)
        .unwrap_or(default)
}

fn param_f64(params: Option<&Value>, key: &str, default: f64) -> f64 {
    params
        .and_then(|p| p.get(key))
        .and_then(Value::as_f64)
        .unwrap_or(default)
}

fn param_value_or_default(params: Option<&Value>, key: &str, default: Value) -> Value {
    params.and_then(|p| p.get(key)).cloned().unwrap_or(default)
}

fn chrono_like_now() -> (i32, i32) {
    let now = std::time::SystemTime::now();
    let datetime: chrono::DateTime<chrono::Local> = now.into();
    (datetime.year(), datetime.month() as i32)
}

async fn handle_method(
    state: &AppState,
    method: &str,
    _params: Option<&Value>,
) -> Result<Value, String> {
    match method {
        "health.ping" => Ok(json!({
            "ok": true,
            "service": "stratumd",
            "version": env!("CARGO_PKG_VERSION"),
        })),
        "daemon.status" => Ok(json!({
            "ok": true,
            "service": "stratumd",
            "uptime_seconds": state.started_at.elapsed().as_secs(),
            "socket": socket_path(),
        })),

        // Read from in-memory state for status queries
        "battery.status" => Ok(json!({
            "ok": true,
            "battery": state.battery.snapshot(),
        })),
        "audio.status" => Ok(json!({
            "ok": true,
            "audio": state.audio.snapshot(),
        })),
        "audio.devices" => {
            Ok(tokio::task::spawn_blocking(|| {
                json!({
                    "ok": true,
                    "audio": managers::audio::devices(),
                })
            })
            .await
            .map_err(|e| e.to_string())?)
        }
        "audio.set_output" => {
            let target = param_string(_params, "target")?;
            Ok(tokio::task::spawn_blocking(move || managers::audio::set_output(&target))
                .await
                .map_err(|e| e.to_string())?)
        }
        "audio.set_input" => {
            let target = param_string(_params, "target")?;
            Ok(tokio::task::spawn_blocking(move || managers::audio::set_input(&target))
                .await
                .map_err(|e| e.to_string())?)
        }
        "audio.set_volume" => {
            let percent = param_i64(_params, "percent", 0);
            Ok(tokio::task::spawn_blocking(move || managers::audio::set_volume(percent))
                .await
                .map_err(|e| e.to_string())?)
        }
        "audio.eq_list_presets" => {
            let device = param_string(_params, "device")?;
            Ok(tokio::task::spawn_blocking(move || managers::audio::eq_list_presets(&device))
                .await
                .map_err(|e| e.to_string())?)
        }
        "audio.eq_apply_preset" => {
            let device = param_string(_params, "device")?;
            let preset_name = param_string(_params, "preset_name")?;
            Ok(tokio::task::spawn_blocking(move || {
                managers::audio::eq_apply_preset(&device, &preset_name)
            })
            .await
            .map_err(|e| e.to_string())?)
        }
        "audio.eq_apply_parametric" => {
            let device = param_string(_params, "device")?;
            let bands = param_value_or_default(_params, "bands", Value::Array(Vec::new()));
            let preamp_db = param_f64(_params, "preamp_db", 0.0);
            Ok(tokio::task::spawn_blocking(move || {
                managers::audio::eq_apply_parametric(&device, &bands, preamp_db)
            })
            .await
            .map_err(|e| e.to_string())?)
        }
        "audio.eq_save_preset_parametric" => {
            let device = param_string(_params, "device")?;
            let preset_name = param_string(_params, "preset_name")?;
            let bands = param_value_or_default(_params, "bands", Value::Array(Vec::new()));
            let preamp_db = param_f64(_params, "preamp_db", 0.0);
            Ok(tokio::task::spawn_blocking(move || {
                managers::audio::eq_save_preset_parametric(&device, &preset_name, &bands, preamp_db)
            })
            .await
            .map_err(|e| e.to_string())?)
        }
        "audio.eq_delete_preset" => {
            let device = param_string(_params, "device")?;
            let preset_name = param_string(_params, "preset_name")?;
            Ok(tokio::task::spawn_blocking(move || {
                managers::audio::eq_delete_preset(&device, &preset_name)
            })
            .await
            .map_err(|e| e.to_string())?)
        }
        "audio.media_seek" => {
            let position_sec = param_i64(_params, "position_sec", 0);
            Ok(tokio::task::spawn_blocking(move || managers::audio::media_seek(position_sec))
                .await
                .map_err(|e| e.to_string())?)
        }

        // Read from in-memory state
        "net.status" => Ok(json!({
            "ok": true,
            "net": state.net.snapshot(),
        })),
        "wifi.state" => {
            Ok(tokio::task::spawn_blocking(|| managers::wifi::state())
                .await
                .map_err(|e| e.to_string())?)
        }
        "wifi.device_status" => {
            Ok(tokio::task::spawn_blocking(|| managers::wifi::device_status())
                .await
                .map_err(|e| e.to_string())?)
        }
        "wifi.known_connections" => {
            Ok(tokio::task::spawn_blocking(|| managers::wifi::known_connections())
                .await
                .map_err(|e| e.to_string())?)
        }
        "wifi.list" => {
            Ok(tokio::task::spawn_blocking(|| managers::wifi::list())
                .await
                .map_err(|e| e.to_string())?)
        }
        "wifi.active_info" => {
            let device = param_string(_params, "device")?;
            Ok(tokio::task::spawn_blocking(move || managers::wifi::active_info(&device))
                .await
                .map_err(|e| e.to_string())?)
        }
        "wifi.connect" => {
            let ssid = param_string(_params, "ssid")?;
            let password = _params
                .and_then(|p| p.get("password"))
                .and_then(Value::as_str)
                .map(|v| v.to_string());
            Ok(tokio::task::spawn_blocking(move || {
                managers::wifi::connect(&ssid, password.as_deref())
            })
            .await
            .map_err(|e| e.to_string())?)
        }
        "wifi.disconnect" => {
            let device = param_string(_params, "device")?;
            Ok(tokio::task::spawn_blocking(move || managers::wifi::disconnect(&device))
                .await
                .map_err(|e| e.to_string())?)
        }
        "wifi.forget" => {
            let ssid = param_string(_params, "ssid")?;
            Ok(tokio::task::spawn_blocking(move || managers::wifi::forget(&ssid))
                .await
                .map_err(|e| e.to_string())?)
        }
        "wifi.toggle" => {
            let target = param_string(_params, "target")?;
            Ok(tokio::task::spawn_blocking(move || managers::wifi::toggle(&target))
                .await
                .map_err(|e| e.to_string())?)
        }

        // Read from in-memory state
        "bluetooth.status" => Ok(json!({
            "ok": true,
            "bluetooth": state.bluetooth.snapshot(),
        })),
        "bluetooth.state" => {
            Ok(tokio::task::spawn_blocking(|| managers::bluetooth::state())
                .await
                .map_err(|e| e.to_string())?)
        }
        "bluetooth.list" => {
            Ok(tokio::task::spawn_blocking(|| managers::bluetooth::list())
                .await
                .map_err(|e| e.to_string())?)
        }
        "bluetooth.pair" => {
            let mac = param_string(_params, "mac")?;
            Ok(tokio::task::spawn_blocking(move || managers::bluetooth::pair(&mac))
                .await
                .map_err(|e| e.to_string())?)
        }
        "bluetooth.connect" => {
            let mac = param_string(_params, "mac")?;
            Ok(tokio::task::spawn_blocking(move || managers::bluetooth::connect(&mac))
                .await
                .map_err(|e| e.to_string())?)
        }
        "bluetooth.disconnect" => {
            let mac = param_string(_params, "mac")?;
            Ok(tokio::task::spawn_blocking(move || managers::bluetooth::disconnect(&mac))
                .await
                .map_err(|e| e.to_string())?)
        }
        "bluetooth.forget" => {
            let mac = param_string(_params, "mac")?;
            Ok(tokio::task::spawn_blocking(move || managers::bluetooth::forget(&mac))
                .await
                .map_err(|e| e.to_string())?)
        }
        "bluetooth.trust" => {
            let mac = param_string(_params, "mac")?;
            Ok(tokio::task::spawn_blocking(move || managers::bluetooth::trust(&mac))
                .await
                .map_err(|e| e.to_string())?)
        }
        "bluetooth.untrust" => {
            let mac = param_string(_params, "mac")?;
            Ok(tokio::task::spawn_blocking(move || managers::bluetooth::untrust(&mac))
                .await
                .map_err(|e| e.to_string())?)
        }
        "bluetooth.power" => {
            let target = param_string(_params, "target")?;
            Ok(tokio::task::spawn_blocking(move || managers::bluetooth::power(&target))
                .await
                .map_err(|e| e.to_string())?)
        }
        "bluetooth.scan" => {
            Ok(tokio::task::spawn_blocking(|| managers::bluetooth::scan())
                .await
                .map_err(|e| e.to_string())?)
        }

        // Read from in-memory state
        "music.status" | "audio.media_info" => Ok(json!({
            "ok": true,
            "music": state.music.snapshot(),
        })),

        "dashboard.status" => {
            let now = chrono_like_now();
            let year = param_i64(_params, "year", now.0 as i64) as i32;
            let month = param_i64(_params, "month", now.1 as i64) as i32;
            Ok(tokio::task::spawn_blocking(move || {
                json!({
                    "ok": true,
                    "dashboard": managers::dashboard::status(year, month),
                })
            })
            .await
            .map_err(|e| e.to_string())?)
        }
        "dashboard.all" => {
            let year = param_i64(_params, "year", 0) as i32;
            let month = param_i64(_params, "month", 0) as i32;

            // Fast-track: if requesting current year/month, try to return cached state first
            let snapshot = state.dashboard.snapshot();
            if let Some(cal) = snapshot.get("calendar") {
                if cal.get("year").and_then(Value::as_i64) == Some(year as i64)
                    && cal.get("month").and_then(Value::as_i64) == Some(month as i64)
                {
                    return Ok(snapshot);
                }
            }

            Ok(tokio::task::spawn_blocking(move || managers::dashboard::status(year, month))
                .await
                .map_err(|e| e.to_string())?)
        }

        "dashboard.watch" => {
            let now = chrono_like_now();
            let year = param_i64(_params, "year", now.0 as i64) as i32;
            let month = param_i64(_params, "month", now.1 as i64).clamp(1, 12) as u32;
            let mut watch = state
                .dashboard_watch
                .lock()
                .map_err(|err| format!("failed to lock dashboard watch state: {}", err))?;
            *watch = Some((year, month));
            Ok(json!({
                "ok": true,
                "watched": {
                    "year": year,
                    "month": month,
                }
            }))
        }
        "dashboard.unwatch" => {
            let mut watch = state
                .dashboard_watch
                .lock()
                .map_err(|err| format!("failed to lock dashboard watch state: {}", err))?;
            *watch = None;
            Ok(json!({
                "ok": true,
                "watched": false,
            }))
        }
        "polkit.status" => Ok(json!({
            "ok": true,
            "polkit": state.polkit.snapshot(),
        })),
        "polkit.respond" => {
            let user = param_string(_params, "user")?;
            let password = param_string(_params, "password")?;
            let cookie = param_string(_params, "cookie").unwrap_or_default();

            info!("[polkit] received response for cookie: {}", cookie);

            let success =
                managers::polkit::verify_password_and_notify(&user, &password, &cookie).await;

            if let Ok(mut pending) = state.pending_auths.lock() {
                if let Some(session) = pending.get_mut(&cookie) {
                    if success {
                        info!("[polkit] auth success for cookie: {}", cookie);
                        let session = pending.remove(&cookie).unwrap();
                        let _ = session.tx.send(true);

                        // Broadcast final success
                        let final_payload = json!({
                            "polkit": {
                                "active": false,
                                "success": true,
                                "cookie": cookie.clone(),
                                "message": "Authentication successful."
                            }
                        });
                        state.polkit.update(final_payload["polkit"].clone());
                        ipc::broadcast_polkit_update(&state.qs_pid_cache, &final_payload);

                        return Ok(json!({"ok": true, "success": true}));
                    } else {
                        session.attempts += 1;
                        let remaining = 3_u32.saturating_sub(session.attempts);

                        if remaining > 0 {
                            info!(
                                "[polkit] auth failed for cookie: {}, {} attempts remaining",
                                cookie, remaining
                            );

                            // Broadcast intermediate failure
                            let retry_payload = json!({
                                "polkit": {
                                    "active": true,
                                    "success": false,
                                    "cookie": cookie.clone(),
                                    "message": format!("Incorrect password. {} attempts remaining.", remaining)
                                }
                            });
                            state.polkit.update(retry_payload["polkit"].clone());
                            ipc::broadcast_polkit_update(&state.qs_pid_cache, &retry_payload);

                            return Ok(
                                json!({"ok": true, "success": false, "retry": true, "remaining": remaining}),
                            );
                        } else {
                            info!("[polkit] auth failed (max attempts) for cookie: {}", cookie);
                            let session = pending.remove(&cookie).unwrap();
                            let _ = session.tx.send(false);

                            // Broadcast final failure
                            let final_payload = json!({
                                "polkit": {
                                    "active": false,
                                    "success": false,
                                    "cookie": cookie.clone(),
                                    "message": "Authentication failed. Too many attempts."
                                }
                            });
                            state.polkit.update(final_payload["polkit"].clone());
                            ipc::broadcast_polkit_update(&state.qs_pid_cache, &final_payload);

                            return Ok(json!({"ok": true, "success": false, "retry": false}));
                        }
                    }
                }
            }

            Err("no pending authentication session found for this cookie".to_string())
        }
        _ => Err(format!("unknown method '{}'", method)),
    }
}

async fn handle_client(mut stream: UnixStream, state: Arc<AppState>) -> Result<(), String> {
    let (reader_stream, mut writer) = stream.split();
    let mut reader = BufReader::new(reader_stream);

    loop {
        let mut line = String::new();
        let read = reader
            .read_line(&mut line)
            .await
            .map_err(|err| format!("failed to read client line: {}", err))?;

        if read == 0 {
            return Ok(());
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let request: Value = match serde_json::from_str(trimmed) {
            Ok(value) => value,
            Err(err) => {
                let response = jsonrpc_error(Value::Null, -32700, &format!("parse error: {}", err));
                let payload = format!("{}\n", response);
                let _ = writer.write_all(payload.as_bytes()).await;
                continue;
            }
        };

        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let params = request.get("params");

        let response = if method.is_empty() {
            jsonrpc_error(id, -32600, "missing method")
        } else {
            match handle_method(&state, method, params).await {
                Ok(result) => jsonrpc_success(id, result),
                Err(err) => jsonrpc_error(id, -32601, &err),
            }
        };

        let payload = format!("{}\n", response);
        writer
            .write_all(payload.as_bytes())
            .await
            .map_err(|err| format!("failed to write client response: {}", err))?;
    }
}

#[tokio::main]
async fn main() {
    logger::init();
    info!(
        "[stratumd] [launch] Starting stratumd v{}",
        env!("CARGO_PKG_VERSION")
    );

    maybe_launch_quickshell();

    let state = Arc::new(AppState {
        started_at: Instant::now(),
        dashboard_watch: Mutex::new(None),
        audio: AudioState::new(),
        net: NetState::new(),
        bluetooth: BluetoothState::new(),
        battery: BatteryState::new(),
        music: MusicState::new(),
        dashboard: DashboardState::new(),
        polkit: PolkitState::new(),
        qs_pid_cache: Mutex::new(None),
        pending_auths: Mutex::new(HashMap::new()),
    });
    // Initialize the RPC listener early so clients can connect even if
    // some managers (e.g. audio) take time during initialization.
    let path = socket_path();
    if path.exists() {
        let _ = std::fs::remove_file(&path);
    }

    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(err) => {
            error!("failed to bind socket at {}: {}", path.display(), err);
            std::process::exit(1);
        }
    };

    info!("Listening on unix socket: {}", path.display());


    // Initialize managers (restore state, etc)
    managers::audio::initialize();

    // Spawn per-domain monitor threads (poll system tools, update in-memory state)
    managers::audio::spawn_monitor(Arc::clone(&state));
    spawn_net_monitor(Arc::clone(&state));
    spawn_bluetooth_monitor(Arc::clone(&state));
    spawn_battery_monitor(Arc::clone(&state));
    spawn_music_monitor(Arc::clone(&state));
    spawn_dashboard_monitor(Arc::clone(&state));
    spawn_polkit_agent(Arc::clone(&state));

    // Single broadcaster: checks dirty flags and pushes to Quickshell via IPC
    spawn_broadcaster(Arc::clone(&state));

    // Accept loop remains as the main future for the tokio runtime
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let state = Arc::clone(&state);
                tokio::spawn(async move {
                    if let Err(err) = handle_client(stream, state).await {
                        error!("client handling error: {}", err);
                    }
                });
            }
            Err(err) => {
                error!("listener accept error: {}", err);
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
        }
    }
}
