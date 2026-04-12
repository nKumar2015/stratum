use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Instant;

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
}

struct AppState {
    started_at: Instant,
}

fn socket_path() -> PathBuf {
    let runtime_dir = env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(runtime_dir).join("stratumd.sock")
}

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
        "audio.status" => Ok(json!({
            "ok": true,
            "audio": managers::audio::status(),
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
            Ok(managers::audio::eq_save_preset_parametric(&device, &preset_name, &bands, preamp_db))
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
        "net.status" => Ok(json!({
            "ok": true,
            "net": managers::net::status(),
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
        "bluetooth.status" => Ok(json!({
            "ok": true,
            "bluetooth": managers::bluetooth::status(),
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
        "music.status" => Ok(json!({
            "ok": true,
            "music": managers::music::status(),
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
        _ => Err(format!("unknown method '{}'", method)),
    }
}

fn chrono_like_now() -> (i32, i32) {
    let now = std::time::SystemTime::now();
    let datetime: chrono::DateTime<chrono::Local> = now.into();
    (datetime.year(), datetime.month() as i32)
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
            Path::new(&home).join(".nix-profile").join("bin").join(binary),
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
        eprintln!("failed to launch quickshell: neither 'qs' nor 'quickshell' could be resolved");
        return;
    };

    if let Err(err) = Command::new(shell_bin)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        eprintln!("failed to launch quickshell: {}", err);
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
                let response = jsonrpc_error(Value::Null, -32700, &format!("parse error: {}", err));
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
    maybe_launch_quickshell();

    let path = socket_path();
    if path.exists() {
        let _ = fs::remove_file(&path);
    }

    let listener = UnixListener::bind(&path).unwrap_or_else(|err| {
        eprintln!("failed to bind socket at {}: {}", path.display(), err);
        std::process::exit(1);
    });

    let state = Arc::new(AppState {
        started_at: Instant::now(),
    });

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let state = Arc::clone(&state);
                std::thread::spawn(move || {
                    if let Err(err) = handle_client(stream, state) {
                        eprintln!("client handling error: {}", err);
                    }
                });
            }
            Err(err) => {
                eprintln!("listener accept error: {}", err);
            }
        }
    }
}
