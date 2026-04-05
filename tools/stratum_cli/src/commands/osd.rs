use std::process::Command;

use serde_json::json;

use crate::common::{command_available, emit_help, emit_json, fail, is_help_flag, run_command_capture};

fn print_help() {
    emit_help(
        "osd",
        "stratum-cli osd <volume|brightness>",
        &["volume", "brightness"],
    );
}

fn clamp(value: i64, min: i64, max: i64) -> i64 {
    value.max(min).min(max)
}

fn parse_first_percent(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'%' {
                let number = text[start..i].parse::<i64>().ok()?;
                return Some(number);
            }
            continue;
        }
        i += 1;
    }

    None
}

fn parse_mute_state(text: &str) -> String {
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let _key = parts.next();
        if let Some(value) = parts.next() {
            return value.trim().to_lowercase();
        }
    }

    "yes".to_string()
}

fn cmd_volume() {
    if !command_available("pactl") {
        fail("pactl not found");
    }

    let volume_output = run_command_capture("pactl", &["get-sink-volume", "@DEFAULT_SINK@"]) 
        .unwrap_or_else(|e| fail(&e));
    let mute_output = run_command_capture("pactl", &["get-sink-mute", "@DEFAULT_SINK@"]) 
        .unwrap_or_else(|e| fail(&e));

    let volume = parse_first_percent(&volume_output).unwrap_or(0);
    let volume = clamp(volume, 0, 150);
    let mute = parse_mute_state(&mute_output);

    emit_json(json!({
        "ok": true,
        "command": "osd",
        "subcommand": "volume",
        "value": volume,
        "mute": mute,
    }));
}

fn run_program_capture_optional(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn parse_i64_text(text: &str) -> Option<i64> {
    text.trim().parse::<i64>().ok()
}

fn parse_f64_rounded(text: &str) -> Option<i64> {
    let value = text.trim().parse::<f64>().ok()?;
    Some((value + 0.5).floor() as i64)
}

fn get_brightness_percent() -> Option<i64> {
    let current = run_program_capture_optional("brightnessctl", &["get"]).and_then(|v| parse_i64_text(&v));
    let max = run_program_capture_optional("brightnessctl", &["max"]).and_then(|v| parse_i64_text(&v));
    if let (Some(current), Some(max)) = (current, max) {
        if max > 0 {
            let pct = ((current as f64 * 100.0) / max as f64 + 0.5).floor() as i64;
            return Some(clamp(pct, 0, 100));
        }
    }

    if let Some(value) = run_program_capture_optional("light", &["-G"]).and_then(|v| parse_f64_rounded(&v)) {
        return Some(clamp(value, 0, 100));
    }

    if let Some(value) = run_program_capture_optional("brillo", &["-G"]).and_then(|v| parse_f64_rounded(&v)) {
        return Some(clamp(value, 0, 100));
    }

    None
}

fn cmd_brightness() {
    let Some(value) = get_brightness_percent() else {
        fail("no brightness backend");
    };

    emit_json(json!({
        "ok": true,
        "command": "osd",
        "subcommand": "brightness",
        "value": value,
    }));
}

pub fn handle(args: &[String]) {
    let subcommand = args.first().map(String::as_str).unwrap_or("");
    if is_help_flag(subcommand) {
        print_help();
        return;
    }

    match subcommand {
        "volume" => cmd_volume(),
        "brightness" => cmd_brightness(),
        _ => fail("unknown osd command"),
    }
}
