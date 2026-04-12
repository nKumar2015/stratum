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

#[derive(Clone, Debug)]
struct AudioDeviceRow {
    name: String,
    description: String,
    block: String,
}

const EQ_VIRTUAL_INPUT_SINK: &str = "effect_input.stratum_eq";
const EQ_VIRTUAL_OUTPUT_NODE: &str = "effect_output.stratum_eq";

fn parse_pactl_device_rows(
    output: &str,
    block_prefix: &str,
    skip_monitor_sources: bool,
) -> Vec<AudioDeviceRow> {
    let mut rows: Vec<AudioDeviceRow> = Vec::new();
    let mut name = String::new();
    let mut description = String::new();
    let mut block = String::new();
    let mut in_block = false;

    for line in output.lines() {
        if line.starts_with(block_prefix) {
            if !name.is_empty() && (!skip_monitor_sources || !name.ends_with(".monitor")) {
                rows.push(AudioDeviceRow {
                    name: name.clone(),
                    description: description.clone(),
                    block: block.clone(),
                });
            }

            name.clear();
            description.clear();
            block.clear();
            in_block = true;
        }

        if !in_block {
            continue;
        }

        if !block.is_empty() {
            block.push('\n');
        }
        block.push_str(line);

        let trimmed = line.trim_start();
        if let Some(value) = trimmed.strip_prefix("Name: ") {
            name = value.trim().to_string();
        } else if let Some(value) = trimmed.strip_prefix("Description: ") {
            description = value.trim().to_string();
        }
    }

    if !name.is_empty() && (!skip_monitor_sources || !name.ends_with(".monitor")) {
        rows.push(AudioDeviceRow {
            name,
            description,
            block,
        });
    }

    rows
}

fn is_eq_virtual_sink_name(name: &str) -> bool {
    let trimmed = name.trim();
    trimmed == EQ_VIRTUAL_INPUT_SINK || trimmed == EQ_VIRTUAL_OUTPUT_NODE
}

fn resolve_effective_default_sink(default_sink: &str) -> String {
    if default_sink.trim() != EQ_VIRTUAL_INPUT_SINK {
        return default_sink.to_string();
    }

    let Ok(info) = run_command_capture("pw-cli", &["info", EQ_VIRTUAL_OUTPUT_NODE]) else {
        return default_sink.to_string();
    };

    parse_pw_info_property(&info, "target.object").unwrap_or_else(|| default_sink.to_string())
}

fn parse_pw_info_property(output: &str, key: &str) -> Option<String> {
    let needle = format!("{} =", key);
    for line in output.lines() {
        let trimmed = line.trim_start();
        if let Some(value) = trimmed.strip_prefix(&needle) {
            let parsed = value.trim().trim_matches('"').to_string();
            if !parsed.is_empty() {
                return Some(parsed);
            }
        }
    }
    None
}

fn is_headphone_default_sink(default_sink: &str) -> String {
    if default_sink.trim().is_empty() {
        return "no".to_string();
    }

    let sink_output = match run_command_capture("pactl", &["list", "sinks"]) {
        Ok(output) => output,
        Err(_) => return "no".to_string(),
    };

    let sinks = parse_pactl_device_rows(&sink_output, "Sink #", false);
    let Some(default_row) = sinks.iter().find(|row| row.name == default_sink) else {
        return "no".to_string();
    };

    let block = default_row.block.to_lowercase();
    if block.contains("device.form_factor = \"headset\"")
        || block.contains("device.form_factor = \"headphone\"")
        || block.contains("device.icon_name = \"audio-headset")
        || block.contains("active port: headset")
        || block.contains("active port: headphone")
        || block.contains("api.bluez5.icon = \"audio-headset\"")
    {
        return "yes".to_string();
    }

    let probe = format!("{} {}", default_sink, default_row.description).to_lowercase();
    if probe.contains("bluez_output.")
        || probe.contains("headphone")
        || probe.contains("headset")
        || probe.contains("earbud")
        || probe.contains("earphone")
        || probe.contains("airpods")
        || probe.contains("buds")
    {
        "yes".to_string()
    } else {
        "no".to_string()
    }
}

pub fn status() -> Value {
    let default_sink_raw = run_command_capture("pactl", &["get-default-sink"]).unwrap_or_default();
    let default_sink = resolve_effective_default_sink(&default_sink_raw);
    let volume = run_command_capture("pactl", &["get-sink-volume", "@DEFAULT_SINK@"])
        .ok()
        .and_then(|out| extract_first_percent(&out))
        .unwrap_or_else(|| "0%".to_string());
    let mute = run_command_capture("pactl", &["get-sink-mute", "@DEFAULT_SINK@"])
        .ok()
        .and_then(|out| extract_mute_state(&out))
        .unwrap_or_else(|| "yes".to_string());
    let headphones = is_headphone_default_sink(&default_sink);

    json!({
        "default_sink": default_sink,
        "volume": volume,
        "mute": mute,
        "headphones": headphones,
    })
}

pub fn devices() -> Value {
    let default_sink_raw = run_command_capture("pactl", &["get-default-sink"]).unwrap_or_default();
    let default_sink = resolve_effective_default_sink(&default_sink_raw);
    let default_source = run_command_capture("pactl", &["get-default-source"]).unwrap_or_default();

    let volume = run_command_capture("pactl", &["get-sink-volume", "@DEFAULT_SINK@"])
        .ok()
        .and_then(|out| extract_first_percent(&out))
        .unwrap_or_else(|| "0%".to_string());
    let mute = run_command_capture("pactl", &["get-sink-mute", "@DEFAULT_SINK@"])
        .ok()
        .and_then(|out| extract_mute_state(&out))
        .unwrap_or_else(|| "yes".to_string());

    let sink_rows = run_command_capture("pactl", &["list", "sinks"])
        .map(|out| parse_pactl_device_rows(&out, "Sink #", false))
        .unwrap_or_default();
    let sinks = sink_rows
        .iter()
        .filter(|row| !is_eq_virtual_sink_name(&row.name))
        .map(|row| {
            json!({
                "name": row.name,
                "description": row.description,
            })
        })
        .collect::<Vec<_>>();

    let source_rows = run_command_capture("pactl", &["list", "sources"])
        .map(|out| parse_pactl_device_rows(&out, "Source #", true))
        .unwrap_or_default();
    let sources = source_rows
        .iter()
        .map(|row| {
            json!({
                "name": row.name,
                "description": row.description,
            })
        })
        .collect::<Vec<_>>();

    json!({
        "ok": true,
        "command": "audio",
        "subcommand": "status",
        "hover": true,
        "status": {
            "volume": volume,
            "mute": mute,
        },
        "default": {
            "sink": default_sink,
            "source": default_source,
        },
        "sinks": sinks,
        "sources": sources,
    })
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

pub fn eq_save_preset_parametric(
    device: &str,
    preset_name: &str,
    bands: &Value,
    preamp_db: f64,
) -> Value {
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
