use super::config;
use crate::managers::common::run_command_capture;
use serde_json::{json, Value};

#[derive(Clone, Debug)]
struct AudioDeviceRow {
    name: String,
    description: String,
    block: String,
}

pub(crate) fn extract_first_percent(text: &str) -> Option<String> {
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

fn parse_pw_info_property(info: &str, property: &str) -> Option<String> {
    for line in info.lines() {
        let trimmed = line.trim();
        if (trimmed.starts_with(property)
            || (trimmed.starts_with('*') && trimmed.contains(property)))
            && trimmed.contains('=')
        {
            let parts: Vec<&str> = trimmed.split('=').collect();
            if parts.len() > 1 {
                return Some(parts[1].trim().trim_matches('"').to_string());
            }
        }
    }
    None
}

pub(crate) fn resolve_effective_default_sink(default_sink: &str) -> String {
    let raw = default_sink.trim();
    if raw.is_empty() {
        return String::new();
    }

    if raw != config::EQ_VIRTUAL_INPUT_SINK {
        return raw.to_string();
    }

    // If we are currently on the EQ sink, find out where its output is pointing
    let Ok(info) = run_command_capture("pw-cli", &["info", config::EQ_VIRTUAL_OUTPUT_NODE]) else {
        return raw.to_string();
    };

    let resolved =
        parse_pw_info_property(&info, "target.object").unwrap_or_else(|| raw.to_string());

    // Safety check: if it resolved back to the virtual sink (rare/buggy state), don't return it
    if resolved == config::EQ_VIRTUAL_INPUT_SINK || resolved == config::EQ_VIRTUAL_OUTPUT_NODE {
        return String::new(); // Caller should handle empty as "unknown/lost"
    }

    resolved
}

pub(crate) fn is_headphone_default_sink(default_sink: &str) -> String {
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

pub(crate) fn parse_eq_bands(value: &Value) -> Option<(Vec<config::EqBand>, bool)> {
    let bands_arr = value.as_array()?;
    if bands_arr.is_empty() {
        return Some((Vec::new(), false));
    }

    if bands_arr.iter().all(Value::is_number) {
        let bands = bands_arr
            .iter()
            .enumerate()
            .filter_map(|(index, raw_gain)| {
                raw_gain
                    .as_i64()
                    .map(|g| config::legacy_band(index, g as i32))
            })
            .collect::<Vec<_>>();
        return Some((bands, true));
    }

    let mut bands = Vec::new();
    for (index, raw_band) in bands_arr.iter().enumerate() {
        let Some(band_obj) = raw_band.as_object() else {
            continue;
        };

        let fallback_freq = config::EQ_DEFAULT_FREQUENCIES
            .get(index)
            .copied()
            .unwrap_or(1000.0);
        let frequency_hz = band_obj
            .get("frequency_hz")
            .and_then(Value::as_f64)
            .or_else(|| band_obj.get("frequency").and_then(Value::as_f64))
            .unwrap_or(fallback_freq);
        let gain_db = band_obj
            .get("gain_db")
            .and_then(Value::as_f64)
            .or_else(|| band_obj.get("gain").and_then(Value::as_f64))
            .unwrap_or(0.0);
        let q = band_obj
            .get("q")
            .and_then(Value::as_f64)
            .unwrap_or(config::EQ_DEFAULT_Q);
        let filter_type = band_obj
            .get("filter_type")
            .and_then(Value::as_str)
            .unwrap_or("peaking")
            .to_string();
        let enabled = band_obj
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true);

        bands.push(config::clamp_band(config::EqBand {
            frequency_hz,
            gain_db,
            q,
            filter_type,
            enabled,
        }));
    }

    Some((bands, false))
}

pub(crate) fn fetch_current_audio_status() -> Value {
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
        .filter(|row| {
            row.name != config::EQ_VIRTUAL_INPUT_SINK && row.name != config::EQ_VIRTUAL_OUTPUT_NODE
        })
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
        .filter(|row| {
            row.name != config::EQ_VIRTUAL_INPUT_SINK && row.name != config::EQ_VIRTUAL_OUTPUT_NODE
        })
        .map(|row| {
            json!({
                "name": row.name,
                "description": row.description,
            })
        })
        .collect::<Vec<_>>();

    json!({
        "ok": true,
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
