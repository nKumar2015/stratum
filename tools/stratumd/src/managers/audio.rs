use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio, Child};
use std::sync::Mutex;
use std::io::Write;
use std::time::{Duration, Instant};
use lazy_static::lazy_static;
use serde::{Serialize, Deserialize};
use serde_json::{json, Value};

use crate::managers::common::{run_command_capture, run_capture_optional, command_available};

fn config_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".config").join("stratum")
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
const EQ_VIRTUAL_INPUT_SINK: &str = "effect_input.stratum_eq";
const EQ_VIRTUAL_OUTPUT_NODE: &str = "effect_output.stratum_eq";
const EQ_SUPPORTED_FILTER_TYPES: [&str; 6] = [
    "peaking",
    "low_shelf",
    "high_shelf",
    "low_pass",
    "high_pass",
    "band_pass",
];

const EQ_DEFAULT_FREQUENCIES: [f64; 10] = [
    31.0, 62.0, 125.0, 250.0, 500.0, 1_000.0, 2_000.0, 4_000.0, 8_000.0, 16_000.0,
];

// EQ band structure (parametric)
#[derive(Clone, Debug, Serialize, Deserialize)]
struct EqBand {
    frequency_hz: f64,
    gain_db: f64,
    q: f64,
    filter_type: String,
    enabled: bool,
}

// EQ Preset structure (per output)
#[derive(Clone, Debug, Serialize, Deserialize)]
struct EqPreset {
    name: String,
    device_id: String,
    bands: Vec<EqBand>,
    preamp_db: f64,
    is_builtin: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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
            "supported": wpctl_available && pw_cli_available,
            "apply_mode": if wpctl_available && pw_cli_available { "pipewire-filter-chain" } else { "dry-run" },
            "max_bands": EQ_MAX_BANDS,
            "gain_range_db": [EQ_MIN_GAIN_DB, EQ_MAX_GAIN_DB],
            "freq_range_hz": [EQ_MIN_FREQ_HZ, EQ_MAX_FREQ_HZ],
            "q_range": [EQ_MIN_Q, EQ_MAX_Q],
            "supported_filter_types": EQ_SUPPORTED_FILTER_TYPES,
        }
    })
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

fn default_eq_presets() -> Vec<EqPreset> {
    vec![
        EqPreset {
            name: "Flat".to_string(),
            device_id: "@DEFAULT_SINK@".to_string(),
            bands: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
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
            bands: [6, 4, 2, 0, -2, -1, 0, 1, 2, 3]
                .iter()
                .enumerate()
                .map(|(index, gain)| legacy_band(index, *gain))
                .collect(),
            preamp_db: 0.0,
            is_builtin: true,
        },
    ]
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

fn save_eq_config(config_data: &EqConfig) -> Result<(), String> {
    let file_path = eq_config_file();
    if let Err(err) = fs::create_dir_all(file_path.parent().unwrap_or_else(|| Path::new("."))) {
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

fn parse_pw_info_property(info: &str, property: &str) -> Option<String> {
    for line in info.lines() {
        let trimmed = line.trim();
        if (trimmed.starts_with(property) || (trimmed.starts_with('*') && trimmed.contains(property)))
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

fn resolve_effective_default_sink(default_sink: &str) -> String {
    let raw = default_sink.trim();
    if raw.is_empty() {
        return String::new();
    }

    if raw != EQ_VIRTUAL_INPUT_SINK {
        return raw.to_string();
    }

    // If we are currently on the EQ sink, find out where its output is pointing
    let Ok(info) = run_command_capture("pw-cli", &["info", EQ_VIRTUAL_OUTPUT_NODE]) else {
        return raw.to_string();
    };

    let resolved = parse_pw_info_property(&info, "target.object").unwrap_or_else(|| raw.to_string());
    
    // Safety check: if it resolved back to the virtual sink (rare/buggy state), don't return it
    if resolved == EQ_VIRTUAL_INPUT_SINK || resolved == EQ_VIRTUAL_OUTPUT_NODE {
        return String::new(); // Caller should handle empty as "unknown/lost"
    }

    resolved
}

fn find_node_id_by_name(node_name: &str) -> Option<u32> {
    let output = run_command_capture("pw-cli", &["ls", "Node"]).ok()?;
    let mut current_id: Option<u32> = None;

    for line in output.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("id ") {
            let id_part = trimmed
                .trim_start_matches("id ")
                .split(',')
                .next()
                .unwrap_or("")
                .trim();
            current_id = id_part.parse::<u32>().ok();
            continue;
        }

        if trimmed.starts_with("node.name =") {
            let name = trimmed
                .trim_start_matches("node.name =")
                .trim()
                .trim_matches('"');
            if name == node_name {
                return current_id;
            }
        }
    }

    None
}

fn purge_eq_output_links() {
    let output = match Command::new("pw-link").arg("-l").output() {
        Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
        Err(_) => return,
    };

    let mut current_source = String::new();
    for line in output.lines() {
        let trimmed = line.trim();
        if (line.starts_with(' ') || line.starts_with('\t')) && !trimmed.is_empty() {
             if (current_source == "effect_output.stratum_eq:output_FL" || current_source == "effect_output.stratum_eq:output_FR") && trimmed.starts_with("|->") {
                 let target_port = trimmed.replacen("|->", "", 1).trim().to_string();
                 println!("[audio] [info] purging incorrect EQ link: {} -> {}", current_source, target_port);
                 let _ = Command::new("pw-link").args(&["-d", &current_source, &target_port]).status();
             }
        } else if !trimmed.is_empty() {
            current_source = trimmed.trim_end_matches(':').to_string();
        }
    }
}

fn list_sink_names() -> Vec<String> {
    let sink_output = match run_command_capture("pactl", &["list", "sinks"]) {
        Ok(output) => output,
        Err(_) => return Vec::new(),
    };

    parse_pactl_device_rows(&sink_output, "Sink #", false)
        .into_iter()
        .map(|row| row.name)
        .collect()
}

fn try_disconnect_link(output_port: &str, input_port: &str) {
    let _ = Command::new("pw-link")
        .args(["-d", output_port, input_port])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn relink_static_eq_output_to_sink(sink: &str) -> Result<(), String> {
    if find_node_id_by_name(EQ_VIRTUAL_INPUT_SINK).is_none()
        || find_node_id_by_name(EQ_VIRTUAL_OUTPUT_NODE).is_none()
    {
        return Err("static EQ sink is not available".to_string());
    }

    let sink_names = list_sink_names();
    if !sink_names.iter().any(|name| name == sink) {
        return Err(format!("sink '{}' not found", sink));
    }

    for candidate in &sink_names {
        for channel in ["FL", "FR"] {
            let output_port = format!("{}:output_{}", EQ_VIRTUAL_OUTPUT_NODE, channel);
            let input_port = format!("{}:playback_{}", candidate, channel);
            try_disconnect_link(&output_port, &input_port);
        }
    }

    for channel in ["FL", "FR"] {
        let output_port = format!("{}:output_{}", EQ_VIRTUAL_OUTPUT_NODE, channel);
        let input_port = format!("{}:playback_{}", sink, channel);
        run_command_capture("pw-link", &[&output_port, &input_port])?;
    }

    run_command_capture("wpctl", &["set-default", EQ_VIRTUAL_INPUT_SINK])?;
    Ok(())
}

lazy_static! {
    static ref EQ_PROCESS: Mutex<Option<Child>> = Mutex::new(None);
    static ref LAST_SYNCED_STATE: Mutex<(String, Instant)> = Mutex::new((String::new(), Instant::now()));
    static ref IS_RESTORING: Mutex<bool> = Mutex::new(false);
}

fn spawn_eq_module(module_args: &str) -> std::io::Result<()> {
    // 1. Terminate existing process if any
    destroy_eq_module();

    // 2. Spawn pw-cli
    let mut child = Command::new("pw-cli")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    // 3. Send load-module command
    if let Some(mut stdin) = child.stdin.take() {
        let cmd = format!("load-module libpipewire-module-filter-chain {}\n", module_args);
        stdin.write_all(cmd.as_bytes())?;
        stdin.flush()?;
        
        // IMPORTANT: We keep CHILD's stdin open? 
        // Actually, pw-cli reads one command and then waits for more.
        // We want it to stay alive. We'll store the stdin handle or just the child.
        // If we drop the handle, pw-cli might exit. Let's keep the child and don't exit.
    }

    let mut lock = EQ_PROCESS.lock().unwrap();
    *lock = Some(child);
    
    Ok(())
}

fn destroy_eq_module() {
    let mut lock = EQ_PROCESS.lock().unwrap();
    if let Some(mut child) = lock.take() {
        let _ = child.kill();
        let _ = child.wait();
    }

    // Fallback: search for any stray nodes by name and destroy them via one-shot pw-cli
    if let Some(id) = find_node_id_by_name(EQ_VIRTUAL_INPUT_SINK) {
        let _ = run_command_capture("pw-cli", &["destroy", &id.to_string()]);
    }
}

fn move_active_streams_to_eq(target_sink: &str) {
    let Ok(output) = run_command_capture("pactl", &["list", "short", "sink-inputs"]) else {
        return;
    };

    for line in output.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 1 {
            let id = parts[0];
            let _ = Command::new("pactl")
                .args(["move-sink-input", id, target_sink])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}

fn map_filter_type_for_pipewire(filter_type: &str) -> &'static str {
    match normalized_filter_type(filter_type).as_str() {
        "peaking" => "bq_peaking",
        "low_shelf" => "bq_lowshelf",
        "high_shelf" => "bq_highshelf",
        "low_pass" => "bq_lowpass",
        "high_pass" => "bq_highpass",
        "band_pass" => "bq_bandpass",
        _ => "bq_peaking",
    }
}

fn builds_filter_graph_string(filter_specs: &[String]) -> String {
    format!(
        "{{ nodes = [ {{ type = builtin name = eq label = param_eq config = {{ filters = [ {} ] }} }} ] inputs = [ \"eq:In 1\" \"eq:In 2\" ] outputs = [ \"eq:Out 1\" \"eq:Out 2\" ] }}",
        filter_specs.join(" ")
    )
}

fn apply_eq_bands_pipewire(device_id: &str, bands: &[EqBand], preamp_db: f64) -> Result<EqApplyResult, String> {
    if !command_available("pw-cli") {
        return Err("pw-cli not found - EQ not applied".to_string());
    }

    let resolved_device = if device_id == "@DEFAULT_SINK@" {
        let def = run_command_capture("pactl", &["get-default-sink"]).unwrap_or_else(|_| device_id.to_string());
        resolve_effective_default_sink(def.trim())
    } else {
        device_id.to_string()
    };

    let mut filter_specs = Vec::new();
    if preamp_db.abs() > 0.01 {
        filter_specs.push(format!("{{ type = bq_peaking freq = 1000.0 gain = {:.4} q = 0.707 }}", preamp_db));
    }
    for band in bands.iter().filter(|b| b.enabled) {
        filter_specs.push(format!(
            "{{ type = {} freq = {:.4} gain = {:.4} q = {:.4} }}",
            map_filter_type_for_pipewire(&band.filter_type),
            band.frequency_hz,
            band.gain_db,
            band.q
        ));
    }
    if filter_specs.is_empty() {
        filter_specs.push("{ type = bq_peaking freq = 1000.0 gain = 0.0 q = 0.707 }".to_string());
    }

    // Since we are in the daemon and owning the dynamic instance, 
    // we use the reliable "destroy and recreate" method to ensure changes are applied.
    destroy_eq_module();
    std::thread::sleep(std::time::Duration::from_millis(150));

    let graph = builds_filter_graph_string(&filter_specs);
    
    // Ensure we are NOT linking to the virtual sink itself (recursion/silence risk)
    if is_eq_virtual_sink_name(&resolved_device) || resolved_device.is_empty() {
        println!("[audio] [warn] resolved_device for EQ is invalid ('{}'), aborting load", resolved_device);
        return Err("invalid hardware target for EQ".to_string());
    }

    println!("[audio] [info] loading EQ module targeting device: {}", resolved_device);

    let module_args = format!(
        "{{ node.description = \"Stratum Parametric EQ\" media.name = \"Stratum Parametric EQ\" filter.graph = {} capture.props = {{ node.name = \"effect_input.stratum_eq\" media.class = Audio/Sink audio.channels = 2 audio.position = [ FL FR ] target.object = \"{}\" }} playback.props = {{ node.name = \"effect_output.stratum_eq\" node.passive = true node.autoconnect = false audio.channels = 2 audio.position = [ FL FR ] target.object = \"{}\" }} }}",
        graph,
        resolved_device.replace('\\', "\\\\").replace('"', "\\\""),
        resolved_device.replace('\\', "\\\\").replace('"', "\\\"")
    );

    spawn_eq_module(&module_args).map_err(|err| format!("failed to spawn persistent EQ process ($pw-cli): {}", err))?;

    let mut created_id = None;
    for _ in 0..30 {
        created_id = find_node_id_by_name(EQ_VIRTUAL_INPUT_SINK);
        if created_id.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    // Only set default sink if it's not already the EQ sink (prevents notification loops)
    let current_default = run_command_capture("pactl", &["get-default-sink"]).unwrap_or_default();
    if current_default.trim() != EQ_VIRTUAL_INPUT_SINK {
        let _ = run_command_capture("pactl", &["set-default-sink", EQ_VIRTUAL_INPUT_SINK]);
    }
    move_active_streams_to_eq(EQ_VIRTUAL_INPUT_SINK);

    // Manual link fallback: disconnect garbage and explicitly connect to hardware
    let target = resolved_device.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(400));
        purge_eq_output_links();
        println!("[audio] [info] ensuring manual link: EQ -> {}", target);
        let _ = run_command_capture("pw-link", &["effect_output.stratum_eq:output_FL", &format!("{}:playback_FL", target)]);
        let _ = run_command_capture("pw-link", &["effect_output.stratum_eq:output_FR", &format!("{}:playback_FR", target)]);
    });

    Ok(EqApplyResult {
        applied: true,
        dry_run: false,
        engine: "pipewire-filter-chain".to_string(),
        resolved_device,
        status: format!(
            "applied {} enabled bands via PipeWire filter-chain virtual sink",
            bands.iter().filter(|b| b.enabled).count()
        ),
    })
}

fn apply_eq_bands(device_id: &str, bands: &[EqBand], preamp_db: f64) -> Result<EqApplyResult, String> {
    validate_parametric_bands(bands)?;

    if command_available("pw-cli") {
        return apply_eq_bands_pipewire(device_id, bands, preamp_db);
    }

    Err("no supported audio processing engine found (requires pw-cli)".to_string())
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
        .filter(|row| !is_eq_virtual_sink_name(&row.name))
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
            "sink": default_sink.clone(),
            "source": default_source,
            "resolved_hardware_sink": resolve_effective_default_sink(&default_sink),
        },
        "sinks": sinks,
        "sources": sources,
        "active_preset": load_eq_config().device_last_preset.get(&resolve_effective_default_sink(&default_sink)),
    })
}

pub fn set_output(target: &str) -> Value {
    let target = target.trim();
    if target.is_empty() {
        return json!({"ok": false, "error": "missing target sink"});
    }

    if is_eq_virtual_sink_name(target) {
        let _ = run_command_capture("pactl", &["set-default-sink", target]);
        return json!({"ok": true, "sink": target});
    }

    // Per-device restoration:
    // When manually switching outputs, we restore the specific profile for that hardware.
    println!("[audio] [info] set_output: restored device EQ profile for target: {}", target);
    let res = auto_apply_preset_for_device(target);
    
    // Update internal state so the monitor doesn't try to re-apply it immediately.
    if let Ok(mut lock) = LAST_SYNCED_STATE.lock() {
        *lock = (target.to_string(), Instant::now());
    }

    if res.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        json!({
            "ok": true,
            "sink": target,
            "routed_via_eq": true,
            "preset": res.get("preset"),
            "message": "output changed and device profile restored",
        })
    } else {
        // Fallback to basic relinking if something went wrong with the preset apply
        let _ = relink_static_eq_output_to_sink(target);
        json!({
            "ok": true,
            "sink": target,
            "routed_via_eq": false,
            "warning": res.get("error"),
        })
    }
}

pub fn set_input(target: &str) -> Value {
    let _ = run_command_capture("pactl", &["set-default-source", target]);
    json!({"ok": true, "source": target})
}

pub fn set_volume(percent: i64) -> Value {
    let p = percent.clamp(0, 150).to_string();
    let _ = run_command_capture("pactl", &["set-sink-volume", "@DEFAULT_SINK@", &format!("{}%", p)]);
    json!({"ok": true, "volume": p})
}

pub fn eq_list_presets(device_id: &str) -> Value {
    let config = load_eq_config();
    let presets: Vec<_> = config
        .presets
        .iter()
        .filter(|p| p.device_id == device_id || p.device_id == "@DEFAULT_SINK@")
        .map(|p| {
            json!({
                "name": p.name,
                "is_builtin": p.is_builtin,
                "bands": legacy_gains_from_bands(&p.bands),
                "parametric_bands": p.bands.iter().map(band_to_json).collect::<Vec<_>>(),
                "preamp_db": p.preamp_db,
            })
        })
        .collect();

    let active_preset = config
        .device_last_preset
        .get(device_id)
        .cloned()
        .or_else(|| config.device_last_preset.get("@DEFAULT_SINK@").cloned())
        .unwrap_or_else(|| "Flat".to_string());

    json!({
        "ok": true,
        "device_id": device_id,
        "active_preset": active_preset,
        "capabilities": eq_capabilities_json(),
        "presets": presets,
    })
}

pub fn eq_apply_preset(device_id: &str, preset_name: &str) -> Value {
    let mut config = load_eq_config();
    let preset = config
        .presets
        .iter()
        .find(|p| p.name == preset_name && (p.device_id == device_id || p.device_id == "@DEFAULT_SINK@"))
        .cloned();

    if let Some(preset) = preset {
        let apply_result = apply_eq_bands(device_id, &preset.bands, preset.preamp_db);

        config.device_last_preset.insert(device_id.to_string(), preset_name.to_string());
        let _ = save_eq_config(&config);

        match apply_result {
            Ok(res) => json!({
                "ok": true,
                "device_id": device_id,
                "preset": preset_name,
                "applied": res.applied,
                "engine": res.engine,
                "status": res.status,
                "parametric_bands": preset.bands,
                "preamp_db": preset.preamp_db,
            }),
            Err(err) => json!({
                "ok": false,
                "error": err,
            }),
        }
    } else {
        json!({"ok": false, "error": format!("preset '{}' not found", preset_name)})
    }
}

pub fn eq_apply_parametric(device_id: &str, bands: &Value, preamp_db: f64) -> Value {
    let Some((parsed_bands, _)) = parse_eq_bands(bands) else {
        return json!({"ok": false, "error": "failed to parse parametric bands"});
    };

    let apply_result = apply_eq_bands(device_id, &parsed_bands, preamp_db);

    match apply_result {
        Ok(res) => json!({
            "ok": true,
            "device_id": device_id,
            "preset": "Custom",
            "applied": res.applied,
            "engine": res.engine,
            "status": res.status,
        }),
        Err(err) => json!({
            "ok": false,
            "error": err,
        }),
    }
}

pub fn eq_save_preset_parametric(
    device_id: &str,
    preset_name: &str,
    bands: &Value,
    preamp_db: f64,
) -> Value {
    let Some((parsed_bands, _)) = parse_eq_bands(bands) else {
        return json!({"ok": false, "error": "failed to parse parametric bands"});
    };

    if let Err(err) = validate_parametric_bands(&parsed_bands) {
        return json!({"ok": false, "error": err});
    }

    let mut config = load_eq_config();
    config.presets.retain(|p| !(p.name == preset_name && (p.device_id == device_id || p.device_id == "@DEFAULT_SINK@") && !p.is_builtin));

    config.presets.push(EqPreset {
        name: preset_name.to_string(),
        device_id: device_id.to_string(),
        bands: parsed_bands,
        preamp_db,
        is_builtin: false,
    });

    config.device_last_preset.insert(device_id.to_string(), preset_name.to_string());
    if let Err(err) = save_eq_config(&config) {
        return json!({"ok": false, "error": err});
    }

    json!({"ok": true, "preset": preset_name, "saved": true})
}

pub fn eq_delete_preset(device_id: &str, preset_name: &str) -> Value {
    let mut config = load_eq_config();
    let original_len = config.presets.len();
    config.presets.retain(|p| !(p.name == preset_name && (p.device_id == device_id || p.device_id == "@DEFAULT_SINK@") && !p.is_builtin));

    if config.presets.len() == original_len {
        return json!({"ok": false, "error": format!("preset '{}' not found", preset_name)});
    }

    if let Err(err) = save_eq_config(&config) {
        return json!({"ok": false, "error": err});
    }

    json!({"ok": true, "deleted": true})
}

pub fn media_seek(position_sec: i64) -> Value {
    let player = run_capture_optional("playerctl", &["-l"]).lines().next().unwrap_or_default().to_string();
    if player.is_empty() {
        return json!({"ok": false, "error": "no active media player"});
    }

    let seek_micro = (position_sec * 1_000_000).to_string();
    let _ = run_command_capture("playerctl", &["-p", &player, "position", &seek_micro]);
    json!({"ok": true, "player": player})
}

pub fn notify_default_sink_changed(new_sink_raw: &str) {
    if *IS_RESTORING.lock().unwrap() {
        return;
    }

    let effective = resolve_effective_default_sink(new_sink_raw);
    if is_eq_virtual_sink_name(&effective) || effective.is_empty() {
        return;
    }

    let mut lock = LAST_SYNCED_STATE.lock().unwrap();
    let (last_sink, last_time) = &*lock;
    
    // Throttle: don't auto-restore more than once every 3 seconds to avoid loops/flapping
    if last_time.elapsed() < Duration::from_secs(3) {
        return;
    }

    if *last_sink != effective {
        println!("[audio] [info] hardware output changed to {}, restoring device EQ profile", effective);
        *lock = (effective.clone(), Instant::now());
        
        // Asynchronously restore preset to avoid blocking the monitor loop
        let target = effective.clone();
        std::thread::spawn(move || {
            {
                let mut guard = IS_RESTORING.lock().unwrap();
                *guard = true;
            }
            
            let res = auto_apply_preset_for_device(&target);
            
            if !res.get("ok").and_then(Value::as_bool).unwrap_or(false) {
                 println!("[audio] [error] auto-restore failed for {}: {:?}", target, res.get("error"));
            }
            
            {
                let mut guard = IS_RESTORING.lock().unwrap();
                *guard = false;
            }
        });
    }
}

pub fn auto_apply_preset_for_device(device_id: &str) -> Value {
    let config = load_eq_config();
    
    // 1. Check for specific device preset
    // 2. Fall back to @DEFAULT_SINK@ preset
    // 3. Fall back to "Flat"
    let preset_name = config.device_last_preset.get(device_id)
        .or_else(|| config.device_last_preset.get("@DEFAULT_SINK@"))
        .map(|s| s.as_str())
        .unwrap_or("Flat");
        
    eq_apply_preset(device_id, preset_name)
}

pub fn initialize() {
    let default_sink_raw = run_command_capture("pactl", &["get-default-sink"]).unwrap_or_default();
    let effective = resolve_effective_default_sink(default_sink_raw.trim());
    
    println!("[audio] [info] initializing audio manager with effective sink: {}", effective);
    
    if let Ok(mut lock) = LAST_SYNCED_STATE.lock() {
        *lock = (effective.clone(), Instant::now());
    }

    if !is_eq_virtual_sink_name(&effective) && !effective.is_empty() {
        let _ = auto_apply_preset_for_device(&effective);
    }
}
