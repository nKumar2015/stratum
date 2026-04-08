use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::json;

use crate::common::{command_available, emit_help, emit_json, fail, is_help_flag};

fn print_help() {
    emit_help(
        "bluetooth",
        "stratum-cli bluetooth <subcommand> [args]",
        &[
            "check",
            "state",
            "list [--hover]",
            "connect <mac> [--hover]",
            "disconnect <mac> [--hover]",
            "pair <mac> [--hover]",
            "trust <mac>",
            "untrust <mac>",
            "forget <mac>",
            "power <on|off>",
            "scan",
        ],
    );
}

fn run_program_combined(program: &str, args: &[&str], stdin_data: Option<&str>) -> (bool, String) {
    let mut command = Command::new(program);
    command.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());

    if stdin_data.is_some() {
        command.stdin(Stdio::piped());
    }

    let mut child = command
        .spawn()
        .unwrap_or_else(|e| fail(&format!("failed to run {}: {}", program, e)));

    if let Some(data) = stdin_data {
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(data.as_bytes())
                .unwrap_or_else(|e| fail(&format!("failed to write {} stdin: {}", program, e)));
        }
    }

    let output = child
        .wait_with_output()
        .unwrap_or_else(|e| fail(&format!("failed to wait for {}: {}", program, e)));

    let mut combined = String::new();
    combined.push_str(&String::from_utf8_lossy(&output.stdout));
    combined.push_str(&String::from_utf8_lossy(&output.stderr));

    (output.status.success(), combined.trim().to_string())
}

fn parse_bool_field(info: &str, key: &str, default: &str) -> String {
    for line in info.lines() {
        if let Some(value) = line.trim().strip_prefix(key) {
            return value.trim().to_lowercase();
        }
    }
    default.to_string()
}

fn parse_device_line(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    if !trimmed.starts_with("Device ") {
        return None;
    }

    let rest = trimmed.trim_start_matches("Device ").trim();
    let mut parts = rest.splitn(2, ' ');
    let mac = parts.next()?.trim();
    if mac.len() != 17 {
        return None;
    }

    let name = parts.next().unwrap_or("").trim().to_string();
    Some((mac.to_uppercase(), name))
}

fn first_keyword_line(text: &str) -> String {
    let keywords = ["successful", "failed", "error", "not available", "already"];

    for line in text.lines() {
        let lowered = line.to_lowercase();
        if keywords.iter().any(|needle| lowered.contains(needle)) {
            return line.trim().to_string();
        }
    }

    String::new()
}

fn message_indicates_failure(message: &str) -> bool {
    let lowered = message.to_lowercase();
    lowered.contains("failed") || lowered.contains("error") || lowered.contains("not available")
}

fn bluetooth_required() {
    if !command_available("bluetoothctl") {
        fail("bluetoothctl not found");
    }
}

fn cmd_state() {
    let (_ok, show_output) = run_program_combined("bluetoothctl", &["show"], None);
    let powered = parse_bool_field(&show_output, "Powered:", "no");
    emit_json(json!({
        "ok": true,
        "command": "bluetooth",
        "subcommand": "state",
        "powered": powered,
    }));
}

fn device_rows(include_paired: bool) -> Vec<(String, String)> {
    let (_ok_devices, devices_output) = run_program_combined("bluetoothctl", &["devices"], None);
    let (_ok_paired, paired_output) = if include_paired {
        run_program_combined("bluetoothctl", &["devices", "Paired"], None)
    } else {
        (true, String::new())
    };

    let mut seen = HashSet::new();
    let mut rows = Vec::new();

    for source in [devices_output, paired_output] {
        for line in source.lines() {
            let Some((mac, name)) = parse_device_line(line) else {
                continue;
            };
            if seen.insert(mac.clone()) {
                rows.push((mac, name));
            }
        }
    }

    rows
}

fn cmd_list(hover: bool) {
    let rows = if hover {
        let (_ok, paired_output) = run_program_combined("bluetoothctl", &["devices", "Paired"], None);
        let mut parsed = Vec::new();

        for line in paired_output.lines() {
            let Some((mac, name)) = parse_device_line(line) else {
                continue;
            };
            parsed.push((mac, name));
        }

        parsed
    } else {
        device_rows(true)
    };

    let mut devices = Vec::new();

    for (mac, name) in rows {
        let (_ok, info) = run_program_combined("bluetoothctl", &["info", &mac], None);
        if hover {
            devices.push(json!({
                "mac": mac,
                "name": if name.is_empty() { mac.clone() } else { name },
                "connected": parse_bool_field(&info, "Connected:", "no"),
            }));
        } else {
            devices.push(json!({
                "mac": mac,
                "name": if name.is_empty() { mac.clone() } else { name },
                "connected": parse_bool_field(&info, "Connected:", "no"),
                "trusted": parse_bool_field(&info, "Trusted:", "no"),
                "paired": parse_bool_field(&info, "Paired:", "no"),
            }));
        }
    }

    emit_json(json!({
        "ok": true,
        "command": "bluetooth",
        "subcommand": "list",
        "hover": hover,
        "devices": devices,
    }));
}

fn cmd_connect(mac: &str, hover: bool) {
    if hover {
        cmd_simple_action("connect", &["connect", mac], true);
        return;
    }

    let (_trust_ok, trust_output) = run_program_combined("bluetoothctl", &["trust", mac], None);

    let stdin_script = format!("agent on\ndefault-agent\nconnect {}\n", mac);
    let (_connect_ok, connect_output) = run_program_combined("timeout", &["12", "bluetoothctl"], Some(&stdin_script));

    let mut message_parts = Vec::new();
    if !trust_output.is_empty() {
        message_parts.push(trust_output);
    }

    let summary = first_keyword_line(&connect_output);
    if !summary.is_empty() {
        message_parts.push(summary);
    }

    let message = if message_parts.is_empty() {
        connect_output
    } else {
        message_parts.join("\n")
    };

    emit_json(json!({
        "ok": true,
        "command": "bluetooth",
        "subcommand": "connect",
        "hover": false,
        "output": message,
        "successful": !message_indicates_failure(&message),
    }));
}

fn cmd_pair(mac: &str, hover: bool) {
    let (_pair_ok, pair_output) = run_program_combined("bluetoothctl", &["pair", mac], None);
    let (_trust_ok, trust_output) = run_program_combined("bluetoothctl", &["trust", mac], None);

    let mut parts = Vec::new();
    if !pair_output.is_empty() {
        parts.push(pair_output);
    }
    if !trust_output.is_empty() {
        parts.push(trust_output);
    }
    let message = parts.join("\n");

    emit_json(json!({
        "ok": true,
        "command": "bluetooth",
        "subcommand": "pair",
        "hover": hover,
        "output": message,
        "successful": !message_indicates_failure(&message),
    }));
}

fn cmd_simple_action(subcommand: &str, args: &[&str], hover: bool) {
    let (_ok, output) = run_program_combined("bluetoothctl", args, None);
    emit_json(json!({
        "ok": true,
        "command": "bluetooth",
        "subcommand": subcommand,
        "hover": hover,
        "output": output,
        "successful": !message_indicates_failure(&output),
    }));
}

fn has_hover_flag(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "--hover")
}

fn first_positional_arg(args: &[String]) -> &str {
    args.iter()
        .map(String::as_str)
        .find(|value| *value != "--hover")
        .unwrap_or("")
}

fn rfkill_bluetooth_power() -> Option<String> {
    let base = Path::new("/sys/class/rfkill");
    let entries = fs::read_dir(base).ok()?;

    let mut found = false;
    let mut any_unblocked = false;

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let ty = fs::read_to_string(path.join("type")).ok()?;
        if ty.trim() != "bluetooth" {
            continue;
        }

        found = true;

        let soft = fs::read_to_string(path.join("soft")).unwrap_or_default();
        let hard = fs::read_to_string(path.join("hard")).unwrap_or_default();

        if soft.trim() == "0" && hard.trim() == "0" {
            any_unblocked = true;
            break;
        }
    }

    if !found {
        return None;
    }

    if any_unblocked {
        Some("on".to_string())
    } else {
        Some("off".to_string())
    }
}

fn cmd_check() {
    if command_available("bluetoothctl") {
        let (_ok, show_output) = run_program_combined("bluetoothctl", &["show"], None);
        let powered = parse_bool_field(&show_output, "Powered:", "no");
        if powered != "yes" {
            emit_json(json!({
                "ok": true,
                "command": "bluetooth",
                "subcommand": "check",
                "state": "off",
            }));
            return;
        }

        let (_ok_connected, connected_output) = run_program_combined("bluetoothctl", &["devices", "Connected"], None);
        let state = if connected_output.lines().any(|line| !line.trim().is_empty()) {
            "connected"
        } else {
            "on"
        };

        emit_json(json!({
            "ok": true,
            "command": "bluetooth",
            "subcommand": "check",
            "state": state,
        }));
        return;
    }

    let state = rfkill_bluetooth_power().unwrap_or_else(|| "none".to_string());
    emit_json(json!({
        "ok": true,
        "command": "bluetooth",
        "subcommand": "check",
        "state": state,
    }));
}

pub fn handle(args: &[String]) {
    let subcommand = args.first().map(String::as_str).unwrap_or("");
    if is_help_flag(subcommand) {
        print_help();
        return;
    }

    match subcommand {
        "check" => cmd_check(),
        "state" => {
            bluetooth_required();
            cmd_state();
        }
        "list" => {
            bluetooth_required();
            let sub_args = args.get(1..).unwrap_or(&[]);
            let hover = has_hover_flag(sub_args);
            cmd_list(hover);
        }
        "connect" => {
            bluetooth_required();
            let sub_args = args.get(1..).unwrap_or(&[]);
            let hover = has_hover_flag(sub_args);
            let mac = first_positional_arg(sub_args);
            cmd_connect(mac, hover);
        }
        "disconnect" => {
            bluetooth_required();
            let sub_args = args.get(1..).unwrap_or(&[]);
            let hover = has_hover_flag(sub_args);
            let mac = first_positional_arg(sub_args);
            cmd_simple_action("disconnect", &["disconnect", mac], hover);
        }
        "pair" => {
            bluetooth_required();
            let sub_args = args.get(1..).unwrap_or(&[]);
            let hover = has_hover_flag(sub_args);
            let mac = first_positional_arg(sub_args);
            cmd_pair(mac, hover);
        }
        "trust" => {
            bluetooth_required();
            let mac = args.get(1).map(String::as_str).unwrap_or("");
            cmd_simple_action("trust", &["trust", mac], false);
        }
        "untrust" => {
            bluetooth_required();
            let mac = args.get(1).map(String::as_str).unwrap_or("");
            cmd_simple_action("untrust", &["untrust", mac], false);
        }
        "forget" => {
            bluetooth_required();
            let mac = args.get(1).map(String::as_str).unwrap_or("");
            cmd_simple_action("forget", &["remove", mac], false);
        }
        "power" => {
            bluetooth_required();
            let target = args.get(1).map(String::as_str).unwrap_or("");
            cmd_simple_action("power", &["--timeout", "4", "power", target], false);
        }
        "scan" => {
            bluetooth_required();
            cmd_simple_action("scan", &["--timeout", "5", "scan", "on"], false);
        }
        _ => fail("unknown bluetooth command"),
    }
}
