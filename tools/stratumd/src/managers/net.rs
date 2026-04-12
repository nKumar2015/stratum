use serde_json::{json, Value};
use std::fs;
use crate::managers::common::run_command_capture;

fn split_nmcli_fields(line: &str, expected_fields: usize) -> Vec<String> {
    if expected_fields == 0 {
        return vec![line.to_string()];
    }

    let mut fields = Vec::new();
    let mut current = String::new();
    let mut escaped = false;

    for ch in line.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }

        if ch == '\\' {
            escaped = true;
            continue;
        }

        if ch == ':' && fields.len() < expected_fields - 1 {
            fields.push(current);
            current = String::new();
            continue;
        }

        current.push(ch);
    }

    fields.push(current);
    fields
}

fn first_nmcli_value(output: &str, prefix: &str, strip_cidr: bool) -> String {
    for line in output.lines() {
        if line.starts_with(prefix) {
            let cols = split_nmcli_fields(line, 2);
            if cols.len() >= 2 {
                let trimmed = cols[1].trim();
                if strip_cidr {
                    return trimmed.split('/').next().unwrap_or("").trim().to_string();
                }
                return trimmed.to_string();
            }
        }
    }
    String::new()
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

pub fn status() -> Value {
    let interfaces = list_interfaces();
    let mut connections = Vec::new();
    let mut primary_state = "none";
    let mut primary_signal = 0;

    for interface in &interfaces {
        let is_up = is_up_interface(interface);
        if !is_up {
            continue;
        }

        let is_wifi = interface.starts_with('w');
        let is_eth = interface.starts_with('e');

        if !is_wifi && !is_eth {
            continue;
        }

        let dev_type = if is_wifi { "wifi" } else { "ethernet" };
        
        let show_output = run_command_capture("nmcli", &["-t", "-f", "GENERAL.CONNECTION,IP4.ADDRESS,IP4.GATEWAY", "dev", "show", interface]).unwrap_or_default();
        let connection = first_nmcli_value(&show_output, "GENERAL.CONNECTION", false);
        
        if connection.is_empty() || connection == "--" {
            continue;
        }

        let ip = first_nmcli_value(&show_output, "IP4.ADDRESS", true);
        let gateway = first_nmcli_value(&show_output, "IP4.GATEWAY", false);

        if primary_state == "none" || primary_state == "wifi" && is_eth {
            primary_state = dev_type;
        }

        if is_wifi {
            let quality = parse_wireless_quality(interface).unwrap_or(0);
            let pct = (quality * 100) / 70;
            if primary_state == "wifi" && pct > primary_signal {
                primary_signal = pct;
            }

            connections.push(json!({
                "type": "wifi",
                "device": interface,
                "connection": connection,
                "signal": pct.to_string(),
                "ip_address": ip,
                "gateway": gateway,
            }));
        } else {
            connections.push(json!({
                "type": "ethernet",
                "device": interface,
                "connection": connection,
                "signal": serde_json::Value::Null,
                "ip_address": ip,
                "gateway": gateway,
            }));
        }
    }

    if primary_state == "wifi" {
        json!({
            "ok": true,
            "state": "wifi",
            "signal_pct": primary_signal,
            "connections": connections,
        })
    } else {
        json!({
            "ok": true,
            "state": primary_state,
            "connections": connections,
        })
    }
}
