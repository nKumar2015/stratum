use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use tracing::{debug, error, info, warn};

mod logger;

use chrono::Datelike;
use serde_json::{json, Value};

mod managers {
    pub mod audio;
    pub mod bluetooth;
    pub mod common;
    pub mod dashboard;
    pub mod music;
    pub mod net;
    pub mod wifi;
    pub mod battery;
    pub mod polkit;
}

mod state;

use state::{AudioState, BatteryState, BluetoothState, DashboardState, MusicState, NetState, PolkitState};

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
    qs_pid_cache: Mutex<Option<i64>>,
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
        if segment.trim().is_empty() {
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

fn newest_quickshell_pid() -> Result<i64, String> {
    let qs_bin = resolve_qs_binary().ok_or_else(|| "qs binary not found".to_string())?;
    let list_raw = Command::new(qs_bin)
        .args(["list", "--all", "--json"])
        .output()
        .map_err(|err| format!("failed to run qs list: {}", err))?;

    if !list_raw.status.success() {
        let stderr = String::from_utf8_lossy(&list_raw.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&list_raw.stdout).trim().to_string();
        let detail = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("exit code {}", list_raw.status)
        };
        return Err(format!("qs list failed: {}", detail));
    }

    let parsed: Value = serde_json::from_slice(&list_raw.stdout)
        .map_err(|err| format!("failed to parse qs list output: {}", err))?;
    let list = parsed
        .as_array()
        .ok_or_else(|| "qs list returned non-array json".to_string())?;

    let mut selected: Option<(String, i64)> = None;

    for entry in list {
        let Some(obj) = entry.as_object() else {
            continue;
        };

        let pid = obj
            .get("pid")
            .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok())))
            .unwrap_or(-1);

        if pid <= 0 {
            continue;
        }

        let launch_time = obj
            .get("launch_time")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        match &selected {
            None => selected = Some((launch_time, pid)),
            Some((best_launch, best_pid)) => {
                if launch_time > *best_launch || (launch_time == *best_launch && pid > *best_pid) {
                    selected = Some((launch_time, pid));
                }
            }
        }
    }

    selected
        .map(|(_, pid)| pid)
        .ok_or_else(|| "no running quickshell instances found".to_string())
}

fn send_shell_ipc_with_pid(
    pid: i64,
    target: &str,
    function: &str,
    args: &[String],
) -> Result<(), String> {
    debug!("IPC call: target={}, function={}, args={:?}", target, function, args);
    let qs_bin = resolve_qs_binary().ok_or_else(|| "qs binary not found".to_string())?;

    let mut command_args = vec![
        "ipc".to_string(),
        "--pid".to_string(),
        pid.to_string(),
        "call".to_string(),
        target.to_string(),
        function.to_string(),
    ];
    command_args.extend(args.iter().cloned());

    let output = Command::new(qs_bin)
        .args(&command_args)
        .output()
        .map_err(|err| format!("failed to run qs ipc: {}", err))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("exit code {}", output.status)
        };
        return Err(format!("qs ipc failed: {}", detail));
    }

    Ok(())
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
        error!("failed to launch quickshell: neither 'qs' nor 'quickshell' could be resolved");
        return;
    };

    let log_path = logger::get_shell_log_path();
    let log_file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path);

    let (stdout, stderr) = match log_file {
        Ok(f) => {
            info!("Redirecting shell output to {}", log_path.display());
            (Stdio::from(f.try_clone().unwrap()), Stdio::from(f))
        }
        Err(e) => {
            warn!("Failed to open shell log file {}, using null: {}", log_path.display(), e);
            (Stdio::null(), Stdio::null())
        }
    };

    info!("Launching quickshell: {:?}", shell_bin);
    if let Err(err) = Command::new("setsid")
        .arg(shell_bin)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr)
        .spawn()
    {
        error!("failed to launch quickshell (detached): {}", err);
    }
}

// --- Monitor threads ---

fn spawn_audio_monitor(state: Arc<AppState>) {
    thread::spawn(move || {
        info!("Starting audio monitor");
        let mut iteration = 0;
        // Initial refresh
        managers::audio::refresh_device_cache();
        
        loop {
            let snapshot = managers::audio::status();
            
            // Auto-restore EQ when the hardware output shifts
            if let Some(sink) = snapshot.get("default_sink").and_then(Value::as_str) {
                managers::audio::notify_default_sink_changed(sink);
            }

            state.audio.update(snapshot);
            
            // Refresh device list every 10 seconds (5 * 2s)
            iteration += 1;
            if iteration >= 5 {
                managers::audio::refresh_device_cache();
                iteration = 0;
            }
            
            thread::sleep(Duration::from_secs(2));
        }
    });
}

fn spawn_net_monitor(state: Arc<AppState>) {
    thread::spawn(move || {
        info!("Starting network monitor");
        loop {
            let snapshot = managers::net::status();
            state.net.update(snapshot);
            thread::sleep(Duration::from_secs(3));
        }
    });
}

fn spawn_bluetooth_monitor(state: Arc<AppState>) {
    thread::spawn(move || {
        info!("Starting bluetooth monitor");
        loop {
            let snapshot = managers::bluetooth::status();
            state.bluetooth.update(snapshot);
            thread::sleep(Duration::from_secs(3));
        }
    });
}

fn spawn_music_monitor(state: Arc<AppState>) {
    thread::spawn(move || {
        info!("Starting music monitor");
        loop {
            let snapshot = managers::music::status();
            state.music.update(snapshot);
            thread::sleep(Duration::from_secs(2));
        }
    });
}

fn spawn_dashboard_monitor(state: Arc<AppState>) {
    thread::spawn(move || {
        info!("Starting dashboard monitor");
        loop {
            let watched = state
                .dashboard_watch
                .lock()
                .ok()
                .and_then(|guard| *guard);

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
    thread::spawn(move || {
        info!("Starting battery monitor");
        loop {
            let snapshot = managers::battery::status();
            state.battery.update(snapshot);
            thread::sleep(Duration::from_secs(5));
        }
    });
}

fn spawn_polkit_agent(state: Arc<AppState>) {
    thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
            
        rt.block_on(async {
            if let Err(e) = managers::polkit::register_agent(state).await {
                error!("[polkit] failed to register authentication agent: {}", e);
                warn!("[polkit] hint: ensure the Polkit service is running (e.g., 'services.polkit.enable = true' on NixOS)");
            } else {
                info!("[polkit] authentication agent registered successfully");
            }
        });
    });
}

// --- Single broadcaster thread ---

fn spawn_broadcaster(state: Arc<AppState>) {
    thread::spawn(move || {
        info!("Starting broadcaster thread");
        // Wait for Quickshell to become available before first push
        for _ in 0..20 {
            if newest_quickshell_pid().is_ok() {
                break;
            }
            thread::sleep(Duration::from_millis(500));
        }

        loop {
            let pid = {
                let cached = state.qs_pid_cache.lock().ok().and_then(|g| *g);
                match cached {
                    Some(pid) => pid,
                    None => match newest_quickshell_pid() {
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
                if send_shell_ipc_with_pid(pid, "daemon", "audio", &[payload_text]).is_err() {
                    any_failed = true;
                }
            }

            if let Some(payload) = state.net.take_if_dirty() {
                debug!("Broadcasting network update");
                let payload_text = payload.to_string();
                if send_shell_ipc_with_pid(pid, "daemon", "wifi", &[payload_text]).is_err() {
                    any_failed = true;
                }
            }

            if let Some(payload) = state.bluetooth.take_if_dirty() {
                debug!("Broadcasting bluetooth update");
                let payload_text = payload.to_string();
                if send_shell_ipc_with_pid(pid, "daemon", "bluetooth", &[payload_text]).is_err() {
                    any_failed = true;
                }
            }

            if let Some(payload) = state.music.take_if_dirty() {
                debug!("Broadcasting music update");
                let payload_text = payload.to_string();
                if send_shell_ipc_with_pid(pid, "daemon", "music", &[payload_text]).is_err() {
                    any_failed = true;
                }
            }

            if let Some(payload) = state.dashboard.take_if_dirty() {
                debug!("Broadcasting dashboard update");
                let payload_text = payload.to_string();
                if send_shell_ipc_with_pid(pid, "daemon", "dashboard", &[payload_text]).is_err() {
                    any_failed = true;
                }
            }

            if let Some(payload) = state.battery.take_if_dirty() {
                debug!("Broadcasting battery update");
                let payload_text = payload.to_string();
                if send_shell_ipc_with_pid(pid, "daemon", "battery", &[payload_text]).is_err() {
                    any_failed = true;
                }
            }

            if let Some(payload) = state.polkit.take_if_dirty() {
                debug!("Broadcasting polkit update");
                let payload_text = payload.to_string();
                if send_shell_ipc_with_pid(pid, "daemon", "polkit", &[payload_text]).is_err() {
                    any_failed = true;
                }
            }

            // Invalidate PID cache on any IPC failure so we re-resolve next tick
            if any_failed {
                if let Ok(mut cache) = state.qs_pid_cache.lock() {
                    *cache = None;
                }
            }

            thread::sleep(Duration::from_millis(200));
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
    let Some(params) = params else {
        return Err(format!("missing params.{}", key));
    };
    let Some(value) = params.get(key).and_then(Value::as_str) else {
        return Err(format!("missing string params.{}", key));
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("empty params.{}", key));
    }
    Ok(trimmed.to_string())
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
    params
        .and_then(|p| p.get(key))
        .cloned()
        .unwrap_or(default)
}

fn chrono_like_now() -> (i32, i32) {
    let now = std::time::SystemTime::now();
    let datetime: chrono::DateTime<chrono::Local> = now.into();
    (datetime.year(), datetime.month() as i32)
}

fn handle_method(state: &AppState, method: &str, _params: Option<&Value>) -> Result<Value, String> {
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
        "audio.devices" => Ok(json!({
            "ok": true,
            "audio": managers::audio::devices(),
        })),
        "audio.set_output" => {
            let target = param_string(_params, "target")?;
            Ok(managers::audio::set_output(&target))
        }
        "audio.set_input" => {
            let target = param_string(_params, "target")?;
            Ok(managers::audio::set_input(&target))
        }
        "audio.set_volume" => {
            let percent = param_i64(_params, "percent", 0);
            Ok(managers::audio::set_volume(percent))
        }
        "audio.eq_list_presets" => {
            let device = param_string(_params, "device")?;
            Ok(managers::audio::eq_list_presets(&device))
        }
        "audio.eq_apply_preset" => {
            let device = param_string(_params, "device")?;
            let preset_name = param_string(_params, "preset_name")?;
            Ok(managers::audio::eq_apply_preset(&device, &preset_name))
        }
        "audio.eq_apply_parametric" => {
            let device = param_string(_params, "device")?;
            let bands = param_value_or_default(_params, "bands", Value::Array(Vec::new()));
            let preamp_db = param_f64(_params, "preamp_db", 0.0);
            Ok(managers::audio::eq_apply_parametric(&device, &bands, preamp_db))
        }
        "audio.eq_save_preset_parametric" => {
            let device = param_string(_params, "device")?;
            let preset_name = param_string(_params, "preset_name")?;
            let bands = param_value_or_default(_params, "bands", Value::Array(Vec::new()));
            let preamp_db = param_f64(_params, "preamp_db", 0.0);
            Ok(managers::audio::eq_save_preset_parametric(
                &device,
                &preset_name,
                &bands,
                preamp_db,
            ))
        }
        "audio.eq_delete_preset" => {
            let device = param_string(_params, "device")?;
            let preset_name = param_string(_params, "preset_name")?;
            Ok(managers::audio::eq_delete_preset(&device, &preset_name))
        }
        "audio.media_seek" => {
            let position_sec = param_i64(_params, "position_sec", 0);
            Ok(managers::audio::media_seek(position_sec))
        }

        // Read from in-memory state
        "net.status" => Ok(json!({
            "ok": true,
            "net": state.net.snapshot(),
        })),
        "wifi.state" => Ok(managers::wifi::state()),
        "wifi.device_status" => Ok(managers::wifi::device_status()),
        "wifi.known_connections" => Ok(managers::wifi::known_connections()),
        "wifi.list" => Ok(managers::wifi::list()),
        "wifi.active_info" => {
            let device = param_string(_params, "device")?;
            Ok(managers::wifi::active_info(&device))
        }
        "wifi.connect" => {
            let ssid = param_string(_params, "ssid")?;
            let password = _params
                .and_then(|p| p.get("password"))
                .and_then(Value::as_str)
                .map(|v| v.to_string());
            Ok(managers::wifi::connect(&ssid, password.as_deref()))
        }
        "wifi.disconnect" => {
            let device = param_string(_params, "device")?;
            Ok(managers::wifi::disconnect(&device))
        }
        "wifi.forget" => {
            let ssid = param_string(_params, "ssid")?;
            Ok(managers::wifi::forget(&ssid))
        }
        "wifi.toggle" => {
            let target = param_string(_params, "target")?;
            Ok(managers::wifi::toggle(&target))
        }

        // Read from in-memory state
        "bluetooth.status" => Ok(json!({
            "ok": true,
            "bluetooth": state.bluetooth.snapshot(),
        })),
        "bluetooth.state" => Ok(managers::bluetooth::state()),
        "bluetooth.list" => Ok(managers::bluetooth::list()),
        "bluetooth.pair" => {
            let mac = param_string(_params, "mac")?;
            Ok(managers::bluetooth::pair(&mac))
        }
        "bluetooth.connect" => {
            let mac = param_string(_params, "mac")?;
            Ok(managers::bluetooth::connect(&mac))
        }
        "bluetooth.disconnect" => {
            let mac = param_string(_params, "mac")?;
            Ok(managers::bluetooth::disconnect(&mac))
        }
        "bluetooth.forget" => {
            let mac = param_string(_params, "mac")?;
            Ok(managers::bluetooth::forget(&mac))
        }
        "bluetooth.trust" => {
            let mac = param_string(_params, "mac")?;
            Ok(managers::bluetooth::trust(&mac))
        }
        "bluetooth.untrust" => {
            let mac = param_string(_params, "mac")?;
            Ok(managers::bluetooth::untrust(&mac))
        }
        "bluetooth.power" => {
            let target = param_string(_params, "target")?;
            Ok(managers::bluetooth::power(&target))
        }
        "bluetooth.scan" => Ok(managers::bluetooth::scan()),

        // Read from in-memory state
        "music.status" | "audio.media_info" => Ok(json!({
            "ok": true,
            "music": state.music.snapshot(),
        })),

        "dashboard.status" => {
            let now = chrono_like_now();
            let year = param_i64(_params, "year", now.0 as i64) as i32;
            let month = param_i64(_params, "month", now.1 as i64) as i32;
            Ok(json!({
                "ok": true,
                "dashboard": managers::dashboard::status(year, month),
            }))
        }
        "dashboard.all" => {
            let year = param_i64(_params, "year", 0) as i32;
            let month = param_i64(_params, "month", 0) as i32;

            // Fast-track: if requesting current year/month, try to return cached state first
            let snapshot = state.dashboard.snapshot();
            if let Some(cal) = snapshot.get("calendar") {
                if cal.get("year").and_then(Value::as_i64) == Some(year as i64) &&
                   cal.get("month").and_then(Value::as_i64) == Some(month as i64) {
                    return Ok(snapshot);
                }
            }

            Ok(managers::dashboard::status(year, month))
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
            let success = managers::polkit::verify_password(&user, &password);
            
            if success && !cookie.is_empty() {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| e.to_string())?;
                let _ = rt.block_on(async {
                    managers::polkit::notify_response(cookie, true).await
                });
            }
            
            Ok(json!({
                "ok": true,
                "success": success,
            }))
        }
        _ => Err(format!("unknown method '{}'", method)),
    }
}

fn handle_client(mut stream: UnixStream, state: Arc<AppState>) -> Result<(), String> {
    let reader_stream = stream
        .try_clone()
        .map_err(|err| format!("failed to clone client stream: {}", err))?;
    let mut reader = BufReader::new(reader_stream);

    loop {
        let mut line = String::new();
        let read = reader
            .read_line(&mut line)
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
                let response =
                    jsonrpc_error(Value::Null, -32700, &format!("parse error: {}", err));
                let payload = format!("{}\n", response);
                let _ = stream.write_all(payload.as_bytes());
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
            match handle_method(&state, method, params) {
                Ok(result) => jsonrpc_success(id, result),
                Err(err) => jsonrpc_error(id, -32601, &err),
            }
        };

        let payload = format!("{}\n", response);
        stream
            .write_all(payload.as_bytes())
            .map_err(|err| format!("failed to write client response: {}", err))?;
    }
}

fn main() {
    logger::init();
    info!("Starting stratumd v{}", env!("CARGO_PKG_VERSION"));
    
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
    });

    // Initialize managers (restore state, etc)
    managers::audio::initialize();

    // Spawn per-domain monitor threads (poll system tools, update in-memory state)
    spawn_audio_monitor(Arc::clone(&state));
    spawn_net_monitor(Arc::clone(&state));
    spawn_bluetooth_monitor(Arc::clone(&state));
    spawn_battery_monitor(Arc::clone(&state));
    spawn_music_monitor(Arc::clone(&state));
    spawn_dashboard_monitor(Arc::clone(&state));
    spawn_polkit_agent(Arc::clone(&state));

    // Single broadcaster: checks dirty flags and pushes to Quickshell via IPC
    spawn_broadcaster(Arc::clone(&state));

    let path = socket_path();
    if path.exists() {
        let _ = fs::remove_file(&path);
    }

    let listener = UnixListener::bind(&path).unwrap_or_else(|err| {
        error!("failed to bind socket at {}: {}", path.display(), err);
        std::process::exit(1);
    });

    info!("Listening on unix socket: {}", path.display());

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let state = Arc::clone(&state);
                std::thread::spawn(move || {
                    if let Err(err) = handle_client(stream, state) {
                        error!("client handling error: {}", err);
                    }
                });
            }
            Err(err) => {
                error!("listener accept error: {}", err);
            }
        }
    }
}
