use std::env;
use std::fs;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::common::{command_available, emit_help, emit_json, fail, is_help_flag, run_command_capture};

const SLURP_BACKGROUND: &str = "#00000066";
const SLURP_BORDER: &str = "#7aa2f7ff";
const SLURP_SELECTION: &str = "#7aa2f744";
const SLURP_BOX: &str = "#101520dd";
const SLURP_BORDER_WIDTH: &str = "2";

fn print_help() {
    emit_help(
        "screenshot-menu",
        "stratum-cli screenshot-menu <subcommand> [args]",
        &[
            "capture [window|region|fullscreen]",
            "capture-geometry <x,y wxh> [mode]",
            "capture-fullscreen [mode] [geometry] [output_name]",
            "freeze-frame [geometry] [monitor_key] [output_name]",
            "active-monitor",
            "window-at <x> <y>",
        ],
    );
}

fn now_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn remove_file_if_exists(path: &str) {
    if path.is_empty() {
        return;
    }
    let _ = fs::remove_file(path);
}

fn file_non_empty(path: &str) -> bool {
    fs::metadata(path).map(|meta| meta.len() > 0).unwrap_or(false)
}

fn run_program_capture_with_stdin(program: &str, args: &[&str], stdin_text: &str) -> Result<String, String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to run {}: {}", program, e))?;

    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin
            .write_all(stdin_text.as_bytes())
            .map_err(|e| format!("failed to write {} stdin: {}", program, e))?;
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("failed to wait for {}: {}", program, e))?;

    if !output.status.success() {
        return Err(format!("{} failed", program));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn run_program_success(program: &str, args: &[&str]) -> Result<(), String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("failed to run {}: {}", program, e))?;

    if !output.status.success() {
        return Err(format!("{} failed", program));
    }

    Ok(())
}

fn make_temp_output() -> String {
    let runtime_dir = env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    let primary_pattern = format!("{}/quickshell-screenshot-XXXXXX.png", runtime_dir);

    if let Ok(path) = run_command_capture("mktemp", &[&primary_pattern]) {
        return path;
    }

    let fallback_pattern = "/tmp/quickshell-screenshot-XXXXXX.png";
    run_command_capture("mktemp", &[fallback_pattern]).unwrap_or_default()
}

fn sanitize_monitor_key(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' || ch == '-' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }

    if out.is_empty() {
        "default".to_string()
    } else {
        out
    }
}

fn freeze_output_path(monitor_key: &str) -> String {
    let runtime_dir = env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    let key = sanitize_monitor_key(monitor_key);
    format!("{}/quickshell-screenshot-freeze-{}.png", runtime_dir, key)
}

fn parse_i64(value: &Value) -> i64 {
    if let Some(num) = value.as_i64() {
        return num;
    }
    if let Some(num) = value.as_u64() {
        return num as i64;
    }
    if let Some(text) = value.as_str() {
        return text.parse::<i64>().unwrap_or(0);
    }
    0
}

fn resolve_window_geometry_at(px: i64, py: i64) -> Option<String> {
    if !command_available("hyprctl") {
        return None;
    }

    let clients_raw = run_command_capture("hyprctl", &["-j", "clients"]).ok()?;
    let clients = serde_json::from_str::<Value>(&clients_raw).ok()?;
    let list = clients.as_array()?;

    let mut candidates = Vec::new();

    for client in list {
        let mapped = client.get("mapped").and_then(Value::as_bool).unwrap_or(false);
        if !mapped {
            continue;
        }

        let hidden = client.get("hidden").and_then(Value::as_bool).unwrap_or(false);
        if hidden {
            continue;
        }

        let at = client.get("at").and_then(Value::as_array);
        let size = client.get("size").and_then(Value::as_array);
        let (Some(at), Some(size)) = (at, size) else {
            continue;
        };
        if at.len() < 2 || size.len() < 2 {
            continue;
        }

        let x = parse_i64(&at[0]);
        let y = parse_i64(&at[1]);
        let w = parse_i64(&size[0]);
        let h = parse_i64(&size[1]);
        if w <= 0 || h <= 0 {
            continue;
        }

        if !(px >= x && px < x + w && py >= y && py < y + h) {
            continue;
        }

        let z = client
            .get("focusHistoryID")
            .map(parse_i64)
            .unwrap_or(999_999);
        candidates.push((z, x, y, w, h));
    }

    candidates.sort_by_key(|row| row.0);
    let (_z, x, y, w, h) = candidates.first().copied()?;
    Some(format!("{},{} {}x{}", x, y, w, h))
}

fn window_boxes_hyprland() -> Vec<String> {
    if !command_available("hyprctl") {
        return Vec::new();
    }

    let clients_raw = match run_command_capture("hyprctl", &["-j", "clients"]) {
        Ok(raw) => raw,
        Err(_) => return Vec::new(),
    };
    let clients = match serde_json::from_str::<Value>(&clients_raw) {
        Ok(value) => value,
        Err(_) => return Vec::new(),
    };
    let Some(list) = clients.as_array() else {
        return Vec::new();
    };

    let mut rows = Vec::new();
    for client in list {
        let mapped = client.get("mapped").and_then(Value::as_bool).unwrap_or(false);
        if !mapped {
            continue;
        }

        let hidden = client.get("hidden").and_then(Value::as_bool).unwrap_or(false);
        if hidden {
            continue;
        }

        let at = client.get("at").and_then(Value::as_array);
        let size = client.get("size").and_then(Value::as_array);
        let (Some(at), Some(size)) = (at, size) else {
            continue;
        };
        if at.len() < 2 || size.len() < 2 {
            continue;
        }

        let x = parse_i64(&at[0]);
        let y = parse_i64(&at[1]);
        let w = parse_i64(&size[0]);
        let h = parse_i64(&size[1]);
        if w <= 0 || h <= 0 {
            continue;
        }

        let title = client
            .get("title")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .or_else(|| client.get("class").and_then(Value::as_str))
            .unwrap_or("Window")
            .replace('\n', " ");

        rows.push(format!("{},{} {}x{} {}", x, y, w, h, title));
    }

    rows
}

fn slurp_selection(args: &[&str], stdin_boxes: Option<&str>) -> Result<String, String> {
    if !command_available("slurp") {
        fail("slurp not found");
    }

    if let Some(input) = stdin_boxes {
        run_program_capture_with_stdin("slurp", args, input).map_err(|_| "selection cancelled".to_string())
    } else {
        run_command_capture("slurp", args).map_err(|_| "selection cancelled".to_string())
    }
}

fn pick_geometry(mode: &str) -> Result<String, String> {
    match mode {
        "fullscreen" => Ok(String::new()),
        "region" => slurp_selection(
            &[
                "-b",
                SLURP_BACKGROUND,
                "-c",
                SLURP_BORDER,
                "-s",
                SLURP_SELECTION,
                "-B",
                SLURP_BOX,
                "-w",
                SLURP_BORDER_WIDTH,
                "-f",
                "%x,%y %wx%h",
            ],
            None,
        ),
        "window" => {
            let boxes = window_boxes_hyprland();
            if !boxes.is_empty() {
                let joined = format!("{}\n", boxes.join("\n"));
                return slurp_selection(
                    &[
                        "-r",
                        "-b",
                        SLURP_BACKGROUND,
                        "-c",
                        SLURP_BORDER,
                        "-s",
                        SLURP_SELECTION,
                        "-B",
                        SLURP_BOX,
                        "-w",
                        SLURP_BORDER_WIDTH,
                        "-f",
                        "%x,%y %wx%h",
                    ],
                    Some(&joined),
                );
            }

            slurp_selection(
                &[
                    "-b",
                    SLURP_BACKGROUND,
                    "-c",
                    SLURP_BORDER,
                    "-s",
                    SLURP_SELECTION,
                    "-B",
                    SLURP_BOX,
                    "-w",
                    SLURP_BORDER_WIDTH,
                    "-f",
                    "%x,%y %wx%h",
                ],
                None,
            )
        }
        _ => fail(&format!("unknown mode: {}", mode)),
    }
}

fn ensure_grim() {
    if !command_available("grim") {
        fail("grim not found");
    }
}

fn emit_capture_ok(path: &str, mode: &str) {
    emit_json(json!({
        "ok": true,
        "command": "screenshot-menu",
        "path": path,
        "mode": mode,
        "ts": now_unix_seconds(),
    }));
}

fn cmd_capture(mode: &str) {
    ensure_grim();

    let out_file = make_temp_output();
    if out_file.is_empty() {
        fail("failed to allocate output file");
    }

    let geom = match pick_geometry(mode) {
        Ok(value) => value,
        Err(message) => {
            remove_file_if_exists(&out_file);
            fail(&message);
        }
    };

    let result = if mode == "fullscreen" {
        run_program_success("grim", &[&out_file])
    } else {
        if geom.is_empty() {
            remove_file_if_exists(&out_file);
            fail("selection cancelled");
        }
        run_program_success("grim", &["-g", &geom, &out_file])
    };

    if result.is_err() {
        remove_file_if_exists(&out_file);
        fail("grim capture failed");
    }

    if !file_non_empty(&out_file) {
        remove_file_if_exists(&out_file);
        fail("capture produced empty file");
    }

    emit_capture_ok(&out_file, mode);
}

fn cmd_capture_geometry(args: &[String]) {
    ensure_grim();

    let geometry = args.get(1).map(String::as_str).unwrap_or("");
    let mode = args.get(2).map(String::as_str).unwrap_or("region");

    if geometry.is_empty() {
        fail("missing geometry");
    }

    let out_file = make_temp_output();
    if out_file.is_empty() {
        fail("failed to allocate output file");
    }

    if run_program_success("grim", &["-g", geometry, &out_file]).is_err() {
        remove_file_if_exists(&out_file);
        fail("grim capture failed");
    }

    if !file_non_empty(&out_file) {
        remove_file_if_exists(&out_file);
        fail("capture produced empty file");
    }

    emit_capture_ok(&out_file, mode);
}

fn cmd_capture_fullscreen(args: &[String]) {
    ensure_grim();

    let mode = args.get(1).map(String::as_str).unwrap_or("fullscreen");
    let geometry = args.get(2).map(String::as_str).unwrap_or("");
    let output_name = args.get(3).map(String::as_str).unwrap_or("");

    let out_file = make_temp_output();
    if out_file.is_empty() {
        fail("failed to allocate output file");
    }

    let result = if !output_name.is_empty() {
        run_program_success("grim", &["-o", output_name, &out_file])
    } else if !geometry.is_empty() {
        run_program_success("grim", &["-g", geometry, &out_file])
    } else {
        run_program_success("grim", &[&out_file])
    };

    if result.is_err() {
        remove_file_if_exists(&out_file);
        fail("grim capture failed");
    }

    if !file_non_empty(&out_file) {
        remove_file_if_exists(&out_file);
        fail("capture produced empty file");
    }

    emit_capture_ok(&out_file, mode);
}

fn cmd_freeze_frame(args: &[String]) {
    ensure_grim();

    let geometry = args.get(1).map(String::as_str).unwrap_or("");
    let monitor_key = args.get(2).map(String::as_str).unwrap_or("default");
    let output_name = args.get(3).map(String::as_str).unwrap_or("");

    let out_file = freeze_output_path(monitor_key);

    let result = if !output_name.is_empty() {
        run_program_success("grim", &["-o", output_name, &out_file])
    } else if !geometry.is_empty() {
        run_program_success("grim", &["-g", geometry, &out_file])
    } else {
        run_program_success("grim", &[&out_file])
    };

    if result.is_err() {
        fail("failed to freeze screen");
    }

    if !file_non_empty(&out_file) {
        fail("freeze image is empty");
    }

    emit_json(json!({
        "ok": true,
        "command": "screenshot-menu",
        "subcommand": "freeze-frame",
        "path": out_file,
    }));
}

fn active_monitor_name() -> Option<String> {
    if !command_available("hyprctl") {
        return None;
    }

    if let Ok(active_workspace_raw) = run_command_capture("hyprctl", &["-j", "activeworkspace"]) {
        if let Ok(active_workspace) = serde_json::from_str::<Value>(&active_workspace_raw) {
            if let Some(name) = active_workspace.get("monitor").and_then(Value::as_str) {
                if !name.is_empty() {
                    return Some(name.to_string());
                }
            }
        }
    }

    if let Ok(monitors_raw) = run_command_capture("hyprctl", &["-j", "monitors"]) {
        if let Ok(monitors) = serde_json::from_str::<Value>(&monitors_raw) {
            if let Some(list) = monitors.as_array() {
                for monitor in list {
                    let focused = monitor.get("focused").and_then(Value::as_bool).unwrap_or(false);
                    if !focused {
                        continue;
                    }
                    if let Some(name) = monitor.get("name").and_then(Value::as_str) {
                        if !name.is_empty() {
                            return Some(name.to_string());
                        }
                    }
                }
            }
        }
    }

    None
}

fn cmd_active_monitor() {
    let monitor = active_monitor_name();
    emit_json(json!({
        "ok": true,
        "command": "screenshot-menu",
        "subcommand": "active-monitor",
        "monitor": monitor,
    }));
}

fn cmd_window_at(args: &[String]) {
    let px = args.get(1).and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
    let py = args.get(2).and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);

    let geometry = resolve_window_geometry_at(px, py);
    emit_json(json!({
        "ok": true,
        "command": "screenshot-menu",
        "subcommand": "window-at",
        "geometry": geometry,
    }));
}

pub fn handle(args: &[String]) {
    let subcommand = args.first().map(String::as_str).unwrap_or("");
    if is_help_flag(subcommand) {
        print_help();
        return;
    }

    match subcommand {
        "capture" => {
            let mode = args.get(1).map(String::as_str).unwrap_or("window");
            cmd_capture(mode);
        }
        "capture-geometry" => cmd_capture_geometry(args),
        "capture-fullscreen" => cmd_capture_fullscreen(args),
        "freeze-frame" => cmd_freeze_frame(args),
        "active-monitor" => cmd_active_monitor(),
        "window-at" => cmd_window_at(args),
        _ => fail("unknown command"),
    }
}
