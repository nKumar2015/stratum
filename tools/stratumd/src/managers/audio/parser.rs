use super::{config, engine};
use pulsectl::controllers::{DeviceControl, SinkController, SourceController};
use serde_json::{json, Value};

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

pub(crate) fn resolve_effective_default_sink(default_sink: &str) -> String {
    let raw = default_sink.trim();
    if raw.is_empty() {
        return String::new();
    }

    if raw != config::EQ_VIRTUAL_INPUT_SINK {
        return raw.to_string();
    }

    let resolved = engine::current_eq_target_sink().unwrap_or_default();

    // If this daemon instance does not have an EQ target in memory yet, keep value unknown.
    if resolved.is_empty() {
        return String::new();
    }

    resolved
}

pub(crate) fn is_headphone_default_sink(default_sink: &str) -> String {
    if default_sink.trim().is_empty() {
        return "no".to_string();
    }

    let mut sink_ctrl = match SinkController::create() {
        Ok(ctrl) => ctrl,
        Err(_) => return "no".to_string(),
    };

    let sinks = match sink_ctrl.list_devices() {
        Ok(list) => list,
        Err(_) => return "no".to_string(),
    };

    let Some(default_row) = sinks
        .iter()
        .find(|row| row.name.as_deref().unwrap_or_default() == default_sink)
    else {
        return "no".to_string();
    };

    let name = default_row.name.as_deref().unwrap_or_default();
    let description = default_row.description.as_deref().unwrap_or_default();
    let active_port = default_row
        .active_port
        .as_ref()
        .and_then(|p| p.description.as_deref())
        .unwrap_or_default();

    let probe = format!("{} {} {}", name, description, active_port).to_lowercase();
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
    let mut sink_ctrl = match SinkController::create() {
        Ok(ctrl) => ctrl,
        Err(_) => {
            return json!({"ok": false, "error": "failed to connect sink controller"});
        }
    };
    let mut source_ctrl = match SourceController::create() {
        Ok(ctrl) => ctrl,
        Err(_) => {
            return json!({"ok": false, "error": "failed to connect source controller"});
        }
    };

    let server_info = match sink_ctrl.get_server_info() {
        Ok(info) => info,
        Err(_) => return json!({"ok": false, "error": "failed to query server info"}),
    };

    let default_sink_raw = server_info.default_sink_name.unwrap_or_default();
    let default_source = server_info.default_source_name.unwrap_or_default();
    let default_sink = resolve_effective_default_sink(&default_sink_raw);

    let sink_devices = sink_ctrl.list_devices().unwrap_or_default();
    let default_sink_dev = sink_devices
        .iter()
        .find(|dev| dev.name.as_deref().unwrap_or_default() == default_sink_raw)
        .cloned();

    let volume = default_sink_dev
        .as_ref()
        .and_then(|dev| extract_first_percent(&dev.volume.print()))
        .unwrap_or_else(|| "0%".to_string());

    let mute = if default_sink_dev.as_ref().map(|dev| dev.mute).unwrap_or(true) {
        "yes".to_string()
    } else {
        "no".to_string()
    };

    let sinks = sink_devices
        .iter()
        .filter_map(|dev| {
            let name = dev.name.clone().unwrap_or_default();
            if name.is_empty()
                || name == config::EQ_VIRTUAL_INPUT_SINK
                || name == config::EQ_VIRTUAL_OUTPUT_NODE
            {
                return None;
            }
            let description = dev.description.clone().unwrap_or_else(|| name.clone());
            Some(json!({
                "name": name,
                "description": description,
            }))
        })
        .collect::<Vec<_>>();

    let source_devices = source_ctrl.list_devices().unwrap_or_default();
    let sources = source_devices
        .iter()
        .filter_map(|dev| {
            let name = dev.name.clone().unwrap_or_default();
            if name.is_empty()
                || name.ends_with(".monitor")
                || name == config::EQ_VIRTUAL_INPUT_SINK
                || name == config::EQ_VIRTUAL_OUTPUT_NODE
            {
                return None;
            }
            let description = dev.description.clone().unwrap_or_else(|| name.clone());
            Some(json!({
                "name": name,
                "description": description,
            }))
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
