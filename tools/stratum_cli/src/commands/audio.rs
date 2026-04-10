use std::fs;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde_json::{json, Value};

use crate::common::{command_available, config_dir, emit_help, emit_json, fail, is_help_flag, run_command_capture};

fn print_help() {
    emit_help(
        "audio",
        "stratum-cli audio <status|set-output|set-input|set-volume|media|equalizer|open-control> [args]",
        &[
            "status [--hover]",
            "set-output <sink>",
            "set-input <source>",
            "set-volume <0-150>",
            "media <info|seek|seek-relative>",
            "media seek <seconds>",
            "media seek-relative <+/- offset_seconds>",
            "equalizer <list-presets|apply-preset|apply-parametric|save-preset|save-preset-parametric|get-current|delete-preset|capabilities>",
            "open-control",
        ],
    );
}

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

const EQ_CONFIG_VERSION: u32 = 2;
const EQ_DEFAULT_Q: f64 = 0.707;
const EQ_MIN_FREQ_HZ: f64 = 20.0;
const EQ_MAX_FREQ_HZ: f64 = 20_000.0;
const EQ_MIN_GAIN_DB: f64 = -24.0;
const EQ_MAX_GAIN_DB: f64 = 24.0;
const EQ_MIN_Q: f64 = 0.1;
const EQ_MAX_Q: f64 = 10.0;
const EQ_MAX_BANDS: usize = 24;
const EQ_SUPPORTED_FILTER_TYPES: [&str; 6] = [
    "peaking",
    "low_shelf",
    "high_shelf",
    "low_pass",
    "high_pass",
    "band_pass",
];

const EQ_DEFAULT_FREQUENCIES: [f64; 10] = [31.0, 62.0, 125.0, 250.0, 500.0, 1_000.0, 2_000.0, 4_000.0, 8_000.0, 16_000.0];

// EQ band structure (parametric)
#[derive(Clone, Debug)]
struct EqBand {
    frequency_hz: f64,
    gain_db: f64,
    q: f64,
    filter_type: String,
    enabled: bool,
}

// EQ Preset structure (per output)
#[derive(Clone, Debug)]
struct EqPreset {
    name: String,
    device_id: String,
    bands: Vec<EqBand>,
    preamp_db: f64,
    is_builtin: bool,
}

#[derive(Clone, Debug)]
struct EqConfig {
    presets: Vec<EqPreset>,
    device_last_preset: HashMap<String, String>,
}

#[derive(Clone, Debug)]
struct EqApplyResult {
    applied: bool,
    dry_run: bool,
    engine: String,
    resolved_device: String,
    status: String,
}

fn eq_config_file() -> PathBuf {
    config_dir().join("audio-eq-presets.json")
}

fn clamp_band(mut band: EqBand) -> EqBand {
    band.frequency_hz = band.frequency_hz.clamp(EQ_MIN_FREQ_HZ, EQ_MAX_FREQ_HZ);
    band.gain_db = band.gain_db.clamp(EQ_MIN_GAIN_DB, EQ_MAX_GAIN_DB);
    band.q = band.q.clamp(EQ_MIN_Q, EQ_MAX_Q);
    if band.filter_type.trim().is_empty() {
        band.filter_type = "peaking".to_string();
    }
    band
}

fn legacy_band(index: usize, gain_db: i32) -> EqBand {
    let frequency_hz = EQ_DEFAULT_FREQUENCIES.get(index).copied().unwrap_or(1000.0);
    clamp_band(EqBand {
        frequency_hz,
        gain_db: gain_db as f64,
        q: EQ_DEFAULT_Q,
        filter_type: "peaking".to_string(),
        enabled: true,
    })
}

fn band_to_json(band: &EqBand) -> Value {
    json!({
        "frequency_hz": band.frequency_hz,
        "gain_db": band.gain_db,
        "q": band.q,
        "filter_type": band.filter_type,
        "enabled": band.enabled,
    })
}

fn preset_legacy_gains(preset: &EqPreset) -> Vec<i32> {
    legacy_gains_from_bands(&preset.bands)
}

fn legacy_gains_from_bands(bands: &[EqBand]) -> Vec<i32> {
    bands
        .iter()
        .map(|band| band.gain_db.round() as i32)
        .collect()
}

fn normalized_filter_type(raw: &str) -> String {
    raw.trim().to_lowercase().replace('-', "_")
}

fn is_supported_filter_type(raw: &str) -> bool {
    let normalized = normalized_filter_type(raw);
    EQ_SUPPORTED_FILTER_TYPES
        .iter()
        .any(|supported| *supported == normalized)
}

fn validate_parametric_bands(bands: &[EqBand]) -> Result<(), String> {
    if bands.is_empty() {
        return Err("parametric preset requires at least one band".to_string());
    }
    if bands.len() > EQ_MAX_BANDS {
        return Err(format!("too many bands: {} (max {})", bands.len(), EQ_MAX_BANDS));
    }

    for (index, band) in bands.iter().enumerate() {
        if !(EQ_MIN_FREQ_HZ..=EQ_MAX_FREQ_HZ).contains(&band.frequency_hz) {
            return Err(format!(
                "band {} frequency out of range: {} Hz ({}..={} expected)",
                index,
                band.frequency_hz,
                EQ_MIN_FREQ_HZ,
                EQ_MAX_FREQ_HZ
            ));
        }
        if !(EQ_MIN_GAIN_DB..=EQ_MAX_GAIN_DB).contains(&band.gain_db) {
            return Err(format!(
                "band {} gain out of range: {} dB ({}..={} expected)",
                index,
                band.gain_db,
                EQ_MIN_GAIN_DB,
                EQ_MAX_GAIN_DB
            ));
        }
        if !(EQ_MIN_Q..=EQ_MAX_Q).contains(&band.q) {
            return Err(format!(
                "band {} Q out of range: {} ({}..={} expected)",
                index,
                band.q,
                EQ_MIN_Q,
                EQ_MAX_Q
            ));
        }
        if !is_supported_filter_type(&band.filter_type) {
            return Err(format!(
                "band {} uses unsupported filter_type '{}'",
                index,
                band.filter_type
            ));
        }
    }

    Ok(())
}

fn eq_capabilities_json() -> Value {
    let wpctl_available = command_available("wpctl");
    let pw_cli_available = command_available("pw-cli");
    let pactl_available = command_available("pactl");

    let wpctl_status_ok = if wpctl_available {
        run_command_capture("wpctl", &["status"]).is_ok()
    } else {
        false
    };

    json!({
        "engine": "pipewire-wireplumber",
        "tools": {
            "pactl": pactl_available,
            "wpctl": wpctl_available,
            "pw_cli": pw_cli_available,
            "wpctl_status_ok": wpctl_status_ok,
        },
        "parametric": {
            "supported": wpctl_available,
            "apply_mode": "dry-run",
            "max_bands": EQ_MAX_BANDS,
            "gain_range_db": [EQ_MIN_GAIN_DB, EQ_MAX_GAIN_DB],
            "freq_range_hz": [EQ_MIN_FREQ_HZ, EQ_MAX_FREQ_HZ],
            "q_range": [EQ_MIN_Q, EQ_MAX_Q],
            "supported_filter_types": EQ_SUPPORTED_FILTER_TYPES,
        }
    })
}

fn cmd_equalizer_capabilities() {
    emit_json(json!({
        "ok": true,
        "capabilities": eq_capabilities_json(),
    }));
}

fn parse_eq_bands(value: &Value) -> Option<(Vec<EqBand>, bool)> {
    let bands_arr = value.as_array()?;
    if bands_arr.is_empty() {
        return Some((Vec::new(), false));
    }

    // Legacy format: array of numbers.
    if bands_arr.iter().all(Value::is_number) {
        let bands = bands_arr
            .iter()
            .enumerate()
            .filter_map(|(index, raw_gain)| raw_gain.as_i64().map(|g| legacy_band(index, g as i32)))
            .collect::<Vec<_>>();
        return Some((bands, true));
    }

    // Parametric format: array of objects.
    let mut bands = Vec::new();
    for (index, raw_band) in bands_arr.iter().enumerate() {
        let Some(band_obj) = raw_band.as_object() else {
            continue;
        };

        let fallback_freq = EQ_DEFAULT_FREQUENCIES.get(index).copied().unwrap_or(1000.0);
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
            .unwrap_or(EQ_DEFAULT_Q);
        let filter_type = band_obj
            .get("filter_type")
            .and_then(Value::as_str)
            .unwrap_or("peaking")
            .to_string();
        let enabled = band_obj
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true);

        bands.push(clamp_band(EqBand {
            frequency_hz,
            gain_db,
            q,
            filter_type,
            enabled,
        }));
    }

    Some((bands, false))
}

fn load_eq_config() -> EqConfig {
    let file_path = eq_config_file();
    if !file_path.exists() {
        return EqConfig {
            presets: default_eq_presets(),
            device_last_preset: HashMap::new(),
        };
    }

    match fs::read_to_string(&file_path) {
        Ok(content) => match serde_json::from_str::<Value>(&content) {
            Ok(data) => {
                let mut migrated = data
                    .get("version")
                    .and_then(Value::as_u64)
                    .unwrap_or(1) < EQ_CONFIG_VERSION as u64;
                let mut presets = Vec::new();
                if let Some(presets_arr) = data.get("presets").and_then(|v| v.as_array()) {
                    for preset_val in presets_arr {
                        if let (Some(name), Some(device_id), Some(bands_arr)) = (
                            preset_val.get("name").and_then(|v| v.as_str()),
                            preset_val.get("device_id").and_then(|v| v.as_str()),
                            preset_val.get("bands"),
                        ) {
                            if let Some((bands, used_legacy)) = parse_eq_bands(bands_arr) {
                                if used_legacy {
                                    migrated = true;
                                }
                                presets.push(EqPreset {
                                    name: name.to_string(),
                                    device_id: device_id.to_string(),
                                    bands,
                                    preamp_db: preset_val
                                        .get("preamp_db")
                                        .and_then(Value::as_f64)
                                        .unwrap_or(0.0),
                                    is_builtin: preset_val.get("is_builtin").and_then(|v| v.as_bool()).unwrap_or(false),
                                });
                            }
                        }
                    }
                }

                let mut device_last_preset = HashMap::new();
                if let Some(last_map) = data.get("device_last_preset").and_then(Value::as_object) {
                    for (device, preset_name) in last_map {
                        if let Some(name) = preset_name.as_str() {
                            device_last_preset.insert(device.to_string(), name.to_string());
                        }
                    }
                }

                let mut config = EqConfig {
                    presets,
                    device_last_preset,
                };

                if config.presets.is_empty() {
                    config.presets = default_eq_presets();
                    migrated = true;
                }

                if migrated {
                    let _ = save_eq_config(&config);
                }

                config
            }
            Err(_) => EqConfig {
                presets: default_eq_presets(),
                device_last_preset: HashMap::new(),
            },
        },
        Err(_) => EqConfig {
            presets: default_eq_presets(),
            device_last_preset: HashMap::new(),
        },
    }
}

fn load_eq_presets() -> Vec<EqPreset> {
    load_eq_config().presets
}

fn default_eq_presets() -> Vec<EqPreset> {
    vec![
        EqPreset {
            name: "Flat".to_string(),
            device_id: "@DEFAULT_SINK@".to_string(),
            bands: vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
                .iter()
                .enumerate()
                .map(|(index, gain)| legacy_band(index, *gain))
                .collect(),
            preamp_db: 0.0,
            is_builtin: true,
        },
        EqPreset {
            name: "Bass Boost".to_string(),
            device_id: "@DEFAULT_SINK@".to_string(),
            bands: vec![6, 4, 2, 0, -2, -1, 0, 1, 2, 3]
                .iter()
                .enumerate()
                .map(|(index, gain)| legacy_band(index, *gain))
                .collect(),
            preamp_db: 0.0,
            is_builtin: true,
        },
        EqPreset {
            name: "Bright".to_string(),
            device_id: "@DEFAULT_SINK@".to_string(),
            bands: vec![0, -2, -1, 0, 1, 2, 3, 4, 3, 2]
                .iter()
                .enumerate()
                .map(|(index, gain)| legacy_band(index, *gain))
                .collect(),
            preamp_db: 0.0,
            is_builtin: true,
        },
        EqPreset {
            name: "Treble Boost".to_string(),
            device_id: "@DEFAULT_SINK@".to_string(),
            bands: vec![-2, -1, 0, 0, 0, 0, 2, 4, 6, 5]
                .iter()
                .enumerate()
                .map(|(index, gain)| legacy_band(index, *gain))
                .collect(),
            preamp_db: 0.0,
            is_builtin: true,
        },
    ]
}

fn save_eq_config(config_data: &EqConfig) -> Result<(), String> {
    let file_path = eq_config_file();
    if let Err(err) = fs::create_dir_all(file_path.parent().unwrap_or_else(|| std::path::Path::new("."))) {
        return Err(format!("failed to create config dir: {}", err));
    }

    let presets_json: Vec<Value> = config_data
        .presets
        .iter()
        .map(|p| {
            json!({
                "name": p.name,
                "device_id": p.device_id,
                "bands": p.bands.iter().map(band_to_json).collect::<Vec<_>>(),
                "preamp_db": p.preamp_db,
                "is_builtin": p.is_builtin,
            })
        })
        .collect();

    let config = json!({
        "version": EQ_CONFIG_VERSION,
        "presets": presets_json,
        "device_last_preset": config_data.device_last_preset,
    });

    fs::write(&file_path, config.to_string())
        .map_err(|err| format!("failed to write EQ config: {}", err))
}

fn parse_pactl_device_rows(output: &str, block_prefix: &str, skip_monitor_sources: bool) -> Vec<AudioDeviceRow> {
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

fn has_hover_flag(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "--hover")
}

fn run_capture_optional(program: &str, args: &[&str]) -> String {
    let output = match Command::new(program).args(args).output() {
        Ok(output) => output,
        Err(_) => return String::new(),
    };

    if !output.status.success() {
        return String::new();
    }

    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn parse_int_from_text(text: &str) -> i64 {
    text.trim()
        .parse::<i64>()
        .ok()
        .or_else(|| {
            text.trim()
                .parse::<f64>()
                .ok()
                .map(|value| value.floor() as i64)
        })
        .unwrap_or(0)
}

fn format_mmss(total_seconds: i64) -> String {
    if total_seconds < 0 {
        return "00:00".to_string();
    }

    let mm = total_seconds / 60;
    let ss = total_seconds % 60;
    format!("{mm:02}:{ss:02}")
}

fn get_active_player() -> String {
    if !command_available("playerctl") {
        return String::new();
    }

    let players_raw = run_capture_optional("playerctl", &["-l"]);
    let mut seen = std::collections::HashSet::new();
    let players = players_raw
        .lines()
        .filter_map(|line| {
            let name = line.trim();
            if name.is_empty() {
                return None;
            }
            if seen.insert(name.to_string()) {
                Some(name.to_string())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    for player in &players {
        let status = run_capture_optional("playerctl", &["-p", player, "status"]);
        if status == "Playing" {
            return player.clone();
        }
    }

    if let Some(player) = players.first() {
        return player.clone();
    }

    String::new()
}

fn cmd_media_info() {
    let player = get_active_player();
    if player.is_empty() {
        emit_json(json!({
            "ok": false,
            "error": "no active media player",
        }));
        return;
    }

    let status = run_capture_optional("playerctl", &["-p", &player, "status"]);
    let pos_raw = run_capture_optional("playerctl", &["-p", &player, "position"]);
    let pos_seconds = parse_int_from_text(&pos_raw);

    let len_raw = run_capture_optional(
        "playerctl",
        &["-p", &player, "metadata", "mpris:length"],
    );
    let len_micro = parse_int_from_text(&len_raw);
    let len_seconds = if len_micro > 0 { len_micro / 1_000_000 } else { 0 };

    emit_json(json!({
        "ok": true,
        "player": player,
        "status": status,
        "position_sec": pos_seconds,
        "length_sec": len_seconds,
        "position": format_mmss(pos_seconds),
        "length": format_mmss(len_seconds),
    }));
}

fn cmd_media_seek(seconds: &str) {
    let player = get_active_player();
    if player.is_empty() {
        emit_json(json!({
            "ok": false,
            "error": "no active media player",
        }));
        return;
    }

    let seek_sec = parse_int_from_text(seconds);
    if seek_sec < 0 {
        emit_json(json!({
            "ok": false,
            "error": "seek position cannot be negative",
        }));
        return;
    }

    let seek_micro = seek_sec * 1_000_000;
    let result = run_command_capture("playerctl", &["-p", &player, "position", &seek_micro.to_string()])
        .unwrap_or_else(|e| fail(&e));

    emit_json(json!({
        "ok": true,
        "player": player,
        "message": result,
    }));
}

fn cmd_media_seek_relative(offset: &str) {
    let player = get_active_player();
    if player.is_empty() {
        emit_json(json!({
            "ok": false,
            "error": "no active media player",
        }));
        return;
    }

    let offset_sec = parse_int_from_text(offset);
    let pos_raw = run_capture_optional("playerctl", &["-p", &player, "position"]);
    let current_sec = parse_int_from_text(&pos_raw);
    let new_sec = (current_sec + offset_sec).max(0);
    let seek_micro = new_sec * 1_000_000;

    let result = run_command_capture("playerctl", &["-p", &player, "position", &seek_micro.to_string()])
        .unwrap_or_else(|e| fail(&e));

    emit_json(json!({
        "ok": true,
        "player": player,
        "position_sec": new_sec,
        "message": result,
    }));
}

fn cmd_equalizer_list_presets(device_id: &str) {
    let presets = load_eq_presets();
    let device_presets: Vec<_> = presets
        .iter()
        .filter(|p| p.device_id == device_id || p.device_id == "@DEFAULT_SINK@")
        .map(|p| {
            json!({
                "name": p.name,
                "is_builtin": p.is_builtin,
                // Keep legacy field for compatibility with existing fixed-band UI.
                "bands": preset_legacy_gains(p),
                "parametric_bands": p.bands.iter().map(band_to_json).collect::<Vec<_>>(),
                "preamp_db": p.preamp_db,
            })
        })
        .collect();

    let config_data = load_eq_config();
    let active_preset = config_data
        .device_last_preset
        .get(device_id)
        .cloned()
        .or_else(|| config_data.device_last_preset.get("@DEFAULT_SINK@").cloned())
        .unwrap_or_else(|| "Flat".to_string());

    emit_json(json!({
        "ok": true,
        "device_id": device_id,
        "active_preset": active_preset,
        "capabilities": eq_capabilities_json(),
        "presets": device_presets,
        "count": device_presets.len(),
    }));
}

fn apply_eq_bands_wpctl(device_id: &str, bands: &[EqBand], preamp_db: f64) -> Result<EqApplyResult, String> {
    validate_parametric_bands(bands)?;

    if !command_available("wpctl") {
        return Err("wpctl not found - EQ not applied".to_string());
    }

    // Resolve @DEFAULT_SINK@ to actual device name
    let resolved_device = if device_id == "@DEFAULT_SINK@" {
        run_command_capture("pactl", &["get-default-sink"]).unwrap_or_else(|_| device_id.to_string())
    } else {
        device_id.to_string()
    };

    // Query wpctl status to find the sink
    let wp_status = run_command_capture("wpctl", &["status"])
        .unwrap_or_default();

    if !resolved_device.trim().is_empty() && !wp_status.contains(&resolved_device) {
        return Err(format!(
            "target sink '{}' was not found in wpctl status output",
            resolved_device
        ));
    }

    // Log the bands being applied for user feedback
    let bands_str = bands
        .iter()
        .filter(|band| band.enabled)
        .map(|band| {
            format!(
                "{}Hz:{}dB@Q{}({})",
                band.frequency_hz.round() as i32,
                band.gain_db,
                band.q,
                band.filter_type
            )
        })
        .collect::<Vec<_>>()
        .join(", ");

    Ok(EqApplyResult {
        applied: false,
        dry_run: true,
        engine: "pipewire-wireplumber".to_string(),
        resolved_device,
        status: format!(
            "validated preamp {} dB and {} enabled bands (dry-run): {}",
            preamp_db,
            bands.iter().filter(|b| b.enabled).count(),
            bands_str
        ),
    })
}

fn cmd_equalizer_apply_preset(device_id: &str, preset_name: &str) {
    let presets = load_eq_presets();
    let preset = presets
        .iter()
        .find(|p| p.name == preset_name && (p.device_id == device_id || p.device_id == "@DEFAULT_SINK@"));

    if let Some(preset) = preset {
        // Apply EQ bands via wpctl
        let apply_result = apply_eq_bands_wpctl(device_id, &preset.bands, preset.preamp_db);

        let mut config_data = load_eq_config();
        config_data
            .device_last_preset
            .insert(device_id.to_string(), preset_name.to_string());
        let _ = save_eq_config(&config_data);
        
        let (apply_ok, apply_payload, status_message) = match apply_result {
            Ok(result) => (true, json!({
                "applied": result.applied,
                "dry_run": result.dry_run,
                "engine": result.engine,
                "resolved_device": result.resolved_device,
                "status": result.status,
            }), result.status),
            Err(err) => (false, json!({
                "applied": false,
                "dry_run": true,
                "engine": "pipewire-wireplumber",
                "error": err,
            }), format!("warning: {}", err)),
        };

        emit_json(json!({
            "ok": true,
            "device_id": device_id,
            "preset": preset_name,
            "bands": preset_legacy_gains(preset),
            "parametric_bands": preset.bands.iter().map(band_to_json).collect::<Vec<_>>(),
            "preamp_db": preset.preamp_db,
            "bands_applied": preset.bands.len(),
            "apply_ok": apply_ok,
            "apply": apply_payload,
            "status": status_message,
        }));
    } else {
        emit_json(json!({
            "ok": false,
            "error": format!("preset '{}' not found", preset_name),
        }));
    }
}

fn cmd_equalizer_apply_parametric(device_id: &str, payload_str: &str) {
    let payload = match serde_json::from_str::<Value>(payload_str) {
        Ok(value) => value,
        Err(err) => {
            emit_json(json!({
                "ok": false,
                "error": format!("invalid parametric payload JSON: {}", err),
            }));
            return;
        }
    };

    let Some(payload_obj) = payload.as_object() else {
        emit_json(json!({
            "ok": false,
            "error": "parametric payload must be a JSON object",
        }));
        return;
    };

    let Some(raw_bands) = payload_obj.get("bands") else {
        emit_json(json!({
            "ok": false,
            "error": "parametric payload must include a 'bands' array",
        }));
        return;
    };

    let Some((bands, _used_legacy)) = parse_eq_bands(raw_bands) else {
        emit_json(json!({
            "ok": false,
            "error": "failed to parse parametric bands array",
        }));
        return;
    };

    if let Err(err) = validate_parametric_bands(&bands) {
        emit_json(json!({
            "ok": false,
            "error": err,
        }));
        return;
    }

    let preamp_db = payload_obj
        .get("preamp_db")
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
        .clamp(EQ_MIN_GAIN_DB, EQ_MAX_GAIN_DB);

    let apply_result = apply_eq_bands_wpctl(device_id, &bands, preamp_db);
    let (apply_ok, apply_payload, status_message) = match apply_result {
        Ok(result) => (
            true,
            json!({
                "applied": result.applied,
                "dry_run": result.dry_run,
                "engine": result.engine,
                "resolved_device": result.resolved_device,
                "status": result.status,
            }),
            result.status,
        ),
        Err(err) => (
            false,
            json!({
                "applied": false,
                "dry_run": true,
                "engine": "pipewire-wireplumber",
                "error": err,
            }),
            format!("warning: {}", err),
        ),
    };

    emit_json(json!({
        "ok": true,
        "device_id": device_id,
        "preset": "Custom",
        "bands": legacy_gains_from_bands(&bands),
        "parametric_bands": bands.iter().map(band_to_json).collect::<Vec<_>>(),
        "preamp_db": preamp_db,
        "bands_applied": bands.len(),
        "apply_ok": apply_ok,
        "apply": apply_payload,
        "status": status_message,
        "mode": "parametric-apply",
    }));
}

fn cmd_equalizer_save_preset(device_id: &str, preset_name: &str, bands_str: &str) {
    // Parse bands from comma-separated string (e.g., "0,2,4,-1,0,0,1,2,3,1")
    let bands: Vec<i32> = bands_str
        .split(',')
        .filter_map(|s| s.trim().parse::<i32>().ok())
        .take(10)
        .collect();

    if bands.len() != 10 {
        emit_json(json!({
            "ok": false,
            "error": "expected 10 EQ bands, comma-separated",
        }));
        return;
    }

    // Clamp legacy gains and convert to parametric bands.
    let bands: Vec<EqBand> = bands
        .iter()
        .enumerate()
        .map(|(index, b)| legacy_band(index, (*b).clamp(-12, 12)))
        .collect();

    let mut config_data = load_eq_config();
    let mut presets = config_data.presets;

    // Remove existing preset with same name for this device
    presets.retain(|p| !(p.name == preset_name && (p.device_id == device_id || p.device_id == "@DEFAULT_SINK@") && !p.is_builtin));

    // Add new preset
    presets.push(EqPreset {
        name: preset_name.to_string(),
        device_id: device_id.to_string(),
        bands,
        preamp_db: 0.0,
        is_builtin: false,
    });

    config_data.presets = presets;
    config_data
        .device_last_preset
        .insert(device_id.to_string(), preset_name.to_string());
    let _ = save_eq_config(&config_data);

    emit_json(json!({
        "ok": true,
        "device_id": device_id,
        "preset": preset_name,
        "saved": true,
    }));
}

fn cmd_equalizer_save_preset_parametric(device_id: &str, preset_name: &str, payload_str: &str) {
    let payload = match serde_json::from_str::<Value>(payload_str) {
        Ok(value) => value,
        Err(err) => {
            emit_json(json!({
                "ok": false,
                "error": format!("invalid parametric payload JSON: {}", err),
            }));
            return;
        }
    };

    let Some(payload_obj) = payload.as_object() else {
        emit_json(json!({
            "ok": false,
            "error": "parametric payload must be a JSON object",
        }));
        return;
    };

    let Some(raw_bands) = payload_obj.get("bands") else {
        emit_json(json!({
            "ok": false,
            "error": "parametric payload must include a 'bands' array",
        }));
        return;
    };

    let Some((bands, _used_legacy)) = parse_eq_bands(raw_bands) else {
        emit_json(json!({
            "ok": false,
            "error": "failed to parse parametric bands array",
        }));
        return;
    };

    if bands.is_empty() {
        emit_json(json!({
            "ok": false,
            "error": "parametric preset requires at least one band",
        }));
        return;
    }

    if let Err(err) = validate_parametric_bands(&bands) {
        emit_json(json!({
            "ok": false,
            "error": err,
        }));
        return;
    }

    let preamp_db = payload_obj
        .get("preamp_db")
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
        .clamp(EQ_MIN_GAIN_DB, EQ_MAX_GAIN_DB);

    let mut config_data = load_eq_config();
    let mut presets = config_data.presets;

    presets.retain(|p| !(p.name == preset_name && (p.device_id == device_id || p.device_id == "@DEFAULT_SINK@") && !p.is_builtin));

    presets.push(EqPreset {
        name: preset_name.to_string(),
        device_id: device_id.to_string(),
        bands,
        preamp_db,
        is_builtin: false,
    });

    config_data.presets = presets;
    config_data
        .device_last_preset
        .insert(device_id.to_string(), preset_name.to_string());
    let _ = save_eq_config(&config_data);

    emit_json(json!({
        "ok": true,
        "device_id": device_id,
        "preset": preset_name,
        "saved": true,
        "mode": "parametric",
    }));
}

fn cmd_equalizer_get_current(device_id: &str) {
    let config_data = load_eq_config();
    let current_name = config_data
        .device_last_preset
        .get(device_id)
        .cloned()
        .or_else(|| config_data.device_last_preset.get("@DEFAULT_SINK@").cloned())
        .unwrap_or_else(|| "Flat".to_string());

    let default_preset = config_data
        .presets
        .iter()
        .find(|p| p.name == current_name && (p.device_id == device_id || p.device_id == "@DEFAULT_SINK@"))
        .or_else(|| {
            config_data
                .presets
                .iter()
                .find(|p| p.name == "Flat" && (p.device_id == device_id || p.device_id == "@DEFAULT_SINK@"))
        })
        .cloned()
        .unwrap_or_else(|| EqPreset {
            name: "Flat".to_string(),
            device_id: device_id.to_string(),
            bands: vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
                .iter()
                .enumerate()
                .map(|(index, gain)| legacy_band(index, *gain))
                .collect(),
            preamp_db: 0.0,
            is_builtin: true,
        });

    emit_json(json!({
        "ok": true,
        "device_id": device_id,
        "current_preset": default_preset.name,
        "bands": preset_legacy_gains(&default_preset),
        "parametric_bands": default_preset.bands.iter().map(band_to_json).collect::<Vec<_>>(),
        "preamp_db": default_preset.preamp_db,
        "capabilities": eq_capabilities_json(),
    }));
}

fn cmd_equalizer_delete_preset(device_id: &str, preset_name: &str) {
    let mut config_data = load_eq_config();
    let mut presets = config_data.presets;

    // Find and remove the preset
    let original_len = presets.len();
    presets.retain(|p| !(p.name == preset_name && (p.device_id == device_id || p.device_id == "@DEFAULT_SINK@") && !p.is_builtin));

    if presets.len() == original_len {
        emit_json(json!({
            "ok": false,
            "error": "preset not found or cannot delete builtin presets",
        }));
        return;
    }

    config_data.presets = presets;
    if config_data
        .device_last_preset
        .get(device_id)
        .map(String::as_str)
        == Some(preset_name)
    {
        config_data.device_last_preset.remove(device_id);
    }
    let _ = save_eq_config(&config_data);

    emit_json(json!({
        "ok": true,
        "device_id": device_id,
        "preset": preset_name,
        "deleted": true,
    }));
}

fn cmd_status(hover: bool) {
    let volume = run_command_capture("pactl", &["get-sink-volume", "@DEFAULT_SINK@"])
        .ok()
        .and_then(|out| extract_first_percent(&out))
        .unwrap_or_else(|| "0%".to_string());
    let mute = run_command_capture("pactl", &["get-sink-mute", "@DEFAULT_SINK@"])
        .ok()
        .and_then(|out| extract_mute_state(&out))
        .unwrap_or_else(|| "yes".to_string());

    if !hover {
        let default_sink = run_command_capture("pactl", &["get-default-sink"]).unwrap_or_default();
        let headphones = is_headphone_default_sink(&default_sink);
        emit_json(json!({
            "ok": true,
            "command": "audio",
            "subcommand": "status",
            "hover": false,
            "volume": volume,
            "mute": mute,
            "headphones": headphones,
        }));
        return;
    }

    let default_sink = run_command_capture("pactl", &["get-default-sink"]).unwrap_or_default();
    let default_source = run_command_capture("pactl", &["get-default-source"]).unwrap_or_default();

    let sink_rows = run_command_capture("pactl", &["list", "sinks"])
        .map(|out| parse_pactl_device_rows(&out, "Sink #", false))
        .unwrap_or_default();
    let sinks = sink_rows
        .iter()
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

    emit_json(json!({
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
    }));
}

pub fn handle(args: &[String]) {
    if !command_available("pactl") {
        fail("pactl not found");
    }

    let command = args.first().map(String::as_str).unwrap_or("");
    if is_help_flag(command) {
        print_help();
        return;
    }

    match command {
        "status" => {
            let sub_args = args.get(1..).unwrap_or(&[]);
            cmd_status(has_hover_flag(sub_args));
        }
        "set-output" => {
            let sink = args.get(1).map(String::as_str).unwrap_or("");
            let result =
                run_command_capture("pactl", &["set-default-sink", sink]).unwrap_or_else(|e| fail(&e));
            emit_json(json!({
                "ok": true,
                "command": "audio",
                "subcommand": "set-output",
                "message": result,
            }));
        }
        "set-input" => {
            let source = args.get(1).map(String::as_str).unwrap_or("");
            let result =
                run_command_capture("pactl", &["set-default-source", source]).unwrap_or_else(|e| fail(&e));
            emit_json(json!({
                "ok": true,
                "command": "audio",
                "subcommand": "set-input",
                "message": result,
            }));
        }
        "set-volume" => {
            let volume_arg = args.get(1).map(String::as_str).unwrap_or("");
            if volume_arg.is_empty() || !volume_arg.bytes().all(|byte| byte.is_ascii_digit()) {
                fail("invalid volume");
            }

            let mut volume = volume_arg.parse::<i32>().unwrap_or(0);
            volume = volume.clamp(0, 150);
            let volume_text = format!("{}%", volume);
            let result = run_command_capture("pactl", &["set-sink-volume", "@DEFAULT_SINK@", &volume_text])
                .unwrap_or_else(|e| fail(&e));
            emit_json(json!({
                "ok": true,
                "command": "audio",
                "subcommand": "set-volume",
                "volume": volume,
                "message": result,
            }));
        }
        "open-control" => {
            if !command_available("pavucontrol") {
                fail("pavucontrol not found");
            }

            if let Err(err) = Command::new("pavucontrol")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                fail(&format!("failed to launch pavucontrol: {}", err));
            }

            emit_json(json!({
                "ok": true,
                "command": "audio",
                "subcommand": "open-control",
            }));
        }
        "media" => {
            let subcommand = args.get(1).map(String::as_str).unwrap_or("");
            match subcommand {
                "info" => cmd_media_info(),
                "seek" => {
                    let seconds = args.get(2).map(String::as_str).unwrap_or("");
                    cmd_media_seek(seconds);
                }
                "seek-relative" => {
                    let offset = args.get(2).map(String::as_str).unwrap_or("");
                    cmd_media_seek_relative(offset);
                }
                _ => fail("unknown media subcommand"),
            }
        }
        "equalizer" => {
            let subcommand = args.get(1).map(String::as_str).unwrap_or("");
            let device = args.get(2).map(String::as_str).unwrap_or("@DEFAULT_SINK@");
            match subcommand {
                "list-presets" => cmd_equalizer_list_presets(device),
                "get-current" => cmd_equalizer_get_current(device),
                "capabilities" => cmd_equalizer_capabilities(),
                "apply-preset" => {
                    let preset = args.get(3).map(String::as_str).unwrap_or("");
                    cmd_equalizer_apply_preset(device, preset);
                }
                "apply-parametric" => {
                    let payload = args.get(3).map(String::as_str).unwrap_or("");
                    cmd_equalizer_apply_parametric(device, payload);
                }
                "save-preset" => {
                    let preset = args.get(3).map(String::as_str).unwrap_or("");
                    let bands = args.get(4).map(String::as_str).unwrap_or("");
                    cmd_equalizer_save_preset(device, preset, bands);
                }
                "save-preset-parametric" => {
                    let preset = args.get(3).map(String::as_str).unwrap_or("");
                    let payload = args.get(4).map(String::as_str).unwrap_or("");
                    cmd_equalizer_save_preset_parametric(device, preset, payload);
                }
                "delete-preset" => {
                    let preset = args.get(3).map(String::as_str).unwrap_or("");
                    cmd_equalizer_delete_preset(device, preset);
                }
                _ => fail("unknown equalizer subcommand"),
            }
        }
        _ => fail("unknown audio command"),
    }
}
