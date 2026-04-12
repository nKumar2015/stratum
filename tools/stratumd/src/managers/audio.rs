use serde_json::{json, Value};

use crate::managers::common::{run_command_capture, run_stratum_cli_json};

fn extract_first_percent(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'%' {
                return Some(text[start..=i].to_string());
            }
            continue;
        }
        i += 1;
    }
    None
}

fn extract_mute_state(text: &str) -> Option<String> {
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let _key = parts.next();
        if let Some(value) = parts.next() {
            return Some(value.trim().to_lowercase());
        }
    }
    None
}

pub fn status() -> Value {
    let default_sink = run_command_capture("pactl", &["get-default-sink"]).unwrap_or_default();
    let volume = run_command_capture("pactl", &["get-sink-volume", "@DEFAULT_SINK@"]) 
        .ok()
        .and_then(|out| extract_first_percent(&out))
        .unwrap_or_else(|| "0%".to_string());
    let mute = run_command_capture("pactl", &["get-sink-mute", "@DEFAULT_SINK@"]) 
        .ok()
        .and_then(|out| extract_mute_state(&out))
        .unwrap_or_else(|| "yes".to_string());

    json!({
        "default_sink": default_sink,
        "volume": volume,
        "mute": mute,
    })
}

pub fn devices() -> Value {
    run_stratum_cli_json(&["audio", "status", "--hover"])
}

pub fn set_output(target: &str) -> Value {
    run_stratum_cli_json(&["audio", "set-output", target])
}

pub fn set_input(target: &str) -> Value {
    run_stratum_cli_json(&["audio", "set-input", target])
}

pub fn set_volume(percent: i64) -> Value {
    let p = percent.clamp(0, 150).to_string();
    run_stratum_cli_json(&["audio", "set-volume", &p])
}

pub fn eq_list_presets(device: &str) -> Value {
    run_stratum_cli_json(&["audio", "equalizer", "list-presets", device])
}

pub fn eq_apply_preset(device: &str, preset_name: &str) -> Value {
    run_stratum_cli_json(&["audio", "equalizer", "apply-preset", device, preset_name])
}

pub fn eq_apply_parametric(device: &str, bands: &Value, preamp_db: f64) -> Value {
    let payload = json!({
        "bands": bands,
        "preamp_db": preamp_db,
    })
    .to_string();
    run_stratum_cli_json(&["audio", "equalizer", "apply-parametric", device, &payload])
}

pub fn eq_save_preset_parametric(device: &str, preset_name: &str, bands: &Value, preamp_db: f64) -> Value {
    let payload = json!({
        "bands": bands,
        "preamp_db": preamp_db,
    })
    .to_string();
    run_stratum_cli_json(&[
        "audio",
        "equalizer",
        "save-preset-parametric",
        device,
        preset_name,
        &payload,
    ])
}

pub fn eq_delete_preset(device: &str, preset_name: &str) -> Value {
    run_stratum_cli_json(&["audio", "equalizer", "delete-preset", device, preset_name])
}

pub fn media_seek(position_sec: i64) -> Value {
    let pos = position_sec.max(0).to_string();
    run_stratum_cli_json(&["audio", "media", "seek", &pos])
}
