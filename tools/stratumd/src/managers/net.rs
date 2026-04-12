use serde_json::{json, Value};
use std::fs;

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

    // Check for ethernet first
    for interface in &interfaces {
        if !interface.starts_with('e') {
            continue;
        }
        if is_up_interface(interface) {
            return json!({
                "ok": true,
                "state": "ethernet",
            });
        }
    }

    // Check for wifi
    for interface in &interfaces {
        if !interface.starts_with('w') {
            continue;
        }
        if !is_up_interface(interface) {
            continue;
        }

        let Some(quality) = parse_wireless_quality(interface) else {
            continue;
        };

        let pct = (quality * 100) / 70;
        return json!({
            "ok": true,
            "state": "wifi",
            "signal_pct": pct,
        });
    }

    json!({
        "ok": true,
        "state": "none",
    })
}
