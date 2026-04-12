use std::fs;

use serde_json::json;

use crate::common::{emit_help, emit_json, is_help_flag};

fn print_help() {
    emit_help("net", "stratum-cli net [check]", &["check"]);
}

fn is_up_interface(name: &str) -> bool {
    let operstate_path = format!("/sys/class/net/{}/operstate", name);
    fs::read_to_string(operstate_path)
        .map(|value| value.trim() == "up")
        .unwrap_or(false)
}

fn list_interfaces() -> Vec<String> {
    let mut names = Vec::new();
    if let Ok(entries) = fs::read_dir("/sys/class/net") {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.is_empty() {
                names.push(name);
            }
        }
    }
    names.sort();
    names
}

fn parse_wireless_quality(device: &str) -> Option<i32> {
    let data = fs::read_to_string("/proc/net/wireless").ok()?;
    for line in data.lines() {
        let tokens = line.split_whitespace().collect::<Vec<_>>();
        if tokens.len() < 3 {
            continue;
        }

        if tokens[0].trim_end_matches(':') != device {
            continue;
        }

        let quality_text = tokens[2].trim_end_matches('.');
        let quality = quality_text.parse::<i32>().ok()?;
        return Some(quality);
    }

    None
}

pub fn handle(args: &[String]) {
    let subcommand = args.first().map(String::as_str).unwrap_or("check");
    if is_help_flag(subcommand) {
        print_help();
        return;
    }

    if subcommand != "check" {
        emit_json(json!({
            "ok": false,
            "error": "unknown net command",
        }));
        return;
    }

    // Try daemon first for instant cached response
    if let Ok(response) = crate::daemon_client::daemon_call("net.status", serde_json::json!({})) {
        if let Some(result) = response.get("result") {
            if let Some(net) = result.get("net") {
                let state = net.get("state").and_then(|v| v.as_str()).unwrap_or("none");
                if state == "wifi" {
                    let signal_pct = net.get("signal_pct").and_then(|v| v.as_i64()).unwrap_or(0);
                    emit_json(json!({
                        "ok": true,
                        "command": "net",
                        "subcommand": "check",
                        "state": "wifi",
                        "signal_pct": signal_pct,
                    }));
                } else {
                    emit_json(json!({
                        "ok": true,
                        "command": "net",
                        "subcommand": "check",
                        "state": state,
                    }));
                }
                return;
            }
        }
    }

    // Fallback to direct sysfs reads

    let interfaces = list_interfaces();

    for interface in &interfaces {
        if !interface.starts_with('e') {
            continue;
        }
        if is_up_interface(interface) {
            emit_json(json!({
                "ok": true,
                "command": "net",
                "subcommand": "check",
                "state": "ethernet",
            }));
            return;
        }
    }

    for interface in &interfaces {
        if !(interface.starts_with('w')) {
            continue;
        }
        if !is_up_interface(interface) {
            continue;
        }

        let Some(quality) = parse_wireless_quality(interface) else {
            continue;
        };

        let pct = (quality * 100) / 70;
        emit_json(json!({
            "ok": true,
            "command": "net",
            "subcommand": "check",
            "state": "wifi",
            "signal_pct": pct,
        }));
        return;
    }

    emit_json(json!({
        "ok": true,
        "command": "net",
        "subcommand": "check",
        "state": "none",
    }));
}
