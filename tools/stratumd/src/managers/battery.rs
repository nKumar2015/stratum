use crate::managers::common::run_command_capture;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

fn run_capture_optional(program: &str, args: &[&str]) -> String {
    run_command_capture(program, args).unwrap_or_default()
}

fn format_duration(total_seconds: i64) -> String {
    if total_seconds <= 0 {
        return "Unknown".to_string();
    }

    let h = total_seconds / 3600;
    let m = (total_seconds % 3600) / 60;

    if h > 0 {
        format!("{}h {:02}m", h, m)
    } else {
        format!("{}m", m)
    }
}

fn read_battery_device() -> String {
    let output = run_capture_optional("upower", &["-e"]);
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.contains("battery") || trimmed.contains("BAT") {
            return trimmed.to_string();
        }
    }
    String::new()
}

fn parse_upower_field(upower_info: &str, field: &str) -> String {
    for line in upower_info.lines() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with(field) {
            continue;
        }
        if let Some((_, value)) = trimmed.split_once(':') {
            return value.trim().to_string();
        }
    }
    String::new()
}

fn power_supply_bat_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    let base = Path::new("/sys/class/power_supply");
    if let Ok(entries) = fs::read_dir(base) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path.file_name().and_then(|v| v.to_str()).unwrap_or("");
            if name.starts_with("BAT") {
                dirs.push(path);
            }
        }
    }
    dirs.sort();
    dirs
}

fn read_battery_percentage(device: &str) -> i64 {
    if !device.is_empty() {
        let info = run_capture_optional("upower", &["-i", device]);
        let value = parse_upower_field(&info, "percentage");
        let pct_text = value.replace('%', "").trim().to_string();
        if let Ok(pct) = pct_text.parse::<i64>() {
            return pct;
        }
    }

    for dir in power_supply_bat_dirs() {
        let cap = dir.join("capacity");
        if let Ok(text) = fs::read_to_string(cap) {
            if let Ok(value) = text.trim().parse::<i64>() {
                return value;
            }
        }
    }
    0
}

fn read_battery_state(device: &str) -> String {
    let mut state = String::new();

    if !device.is_empty() {
        let info = run_capture_optional("upower", &["-i", device]);
        state = parse_upower_field(&info, "state").to_lowercase();
    }

    if state.is_empty() {
        for dir in power_supply_bat_dirs() {
            let status = dir.join("status");
            if let Ok(text) = fs::read_to_string(status) {
                let trimmed = text.trim().to_lowercase();
                if !trimmed.is_empty() {
                    state = trimmed;
                    break;
                }
            }
        }
    }

    if state == "not charging" {
        state = "pending-charge".to_string();
    }

    if state.is_empty() {
        "unknown".to_string()
    } else {
        state
    }
}

fn parse_duration_seconds(duration: &str) -> i64 {
    let mut parts = duration.split_whitespace();
    let value = parts.next().and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
    let unit = parts.next().unwrap_or("").to_lowercase();

    if value <= 0.0 {
        return 0;
    }

    if unit.contains("hour") {
        (value * 3600.0) as i64
    } else if unit.contains("minute") {
        (value * 60.0) as i64
    } else if unit.contains("second") {
        value as i64
    } else {
        0
    }
}

fn read_projected_seconds(device: &str, state: &str) -> i64 {
    if device.is_empty() {
        return 0;
    }

    let info = run_capture_optional("upower", &["-i", device]);
    if info.is_empty() {
        return 0;
    }

    match state {
        "charging" => parse_duration_seconds(&parse_upower_field(&info, "time to full")),
        "discharging" => parse_duration_seconds(&parse_upower_field(&info, "time to empty")),
        _ => 0,
    }
}

fn read_screen_on_time() -> String {
    if let Ok(text) = fs::read_to_string("/proc/uptime") {
        if let Some(first) = text.split_whitespace().next() {
            if let Ok(seconds) = first.parse::<f64>() {
                return format_duration(seconds as i64);
            }
        }
    }
    "Unknown".to_string()
}

fn read_active_profile() -> String {
    let path = Path::new("/sys/firmware/acpi/platform_profile");
    if let Ok(text) = fs::read_to_string(path) {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    "unknown".to_string()
}

fn read_charging_info(device: &str, state: &str) -> String {
    if state == "pending-charge" {
        return "Plugged in, not charging".to_string();
    }

    if state != "charging" || device.is_empty() {
        return String::new();
    }

    let info = run_capture_optional("upower", &["-i", device]);
    let rate = parse_upower_field(&info, "energy-rate");
    if !rate.is_empty() {
        return rate;
    }

    "Connected to charger".to_string()
}

pub fn status() -> Value {
    let device = read_battery_device();
    let pct = read_battery_percentage(&device).clamp(0, 100);
    let state = read_battery_state(&device);
    let projected_sec = read_projected_seconds(&device, &state);
    let projected_text = format_duration(projected_sec);
    let screen_on_time = read_screen_on_time();
    let profile = read_active_profile();
    let charging_info = read_charging_info(&device, &state);

    json!({
        "battery": {
            "pct": pct,
            "state": state,
            "projected_text": projected_text,
            "screen_on_time": screen_on_time,
        },
        "charging_info": charging_info,
        "profile": profile,
    })
}
