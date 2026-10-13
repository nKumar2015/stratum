use super::parser;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const EQ_CONFIG_VERSION: u32 = 2;
pub(crate) const EQ_DEFAULT_Q: f64 = 0.707;
pub(crate) const EQ_MIN_FREQ_HZ: f64 = 20.0;
pub(crate) const EQ_MAX_FREQ_HZ: f64 = 20_000.0;
pub(crate) const EQ_MIN_GAIN_DB: f64 = -24.0;
pub(crate) const EQ_MAX_GAIN_DB: f64 = 24.0;
pub(crate) const EQ_MIN_Q: f64 = 0.1;
pub(crate) const EQ_MAX_Q: f64 = 10.0;
pub(crate) const EQ_MAX_BANDS: usize = 24;
pub(crate) const EQ_VIRTUAL_INPUT_SINK: &str = "effect_input.stratum_eq";
pub(crate) const EQ_VIRTUAL_OUTPUT_NODE: &str = "effect_output.stratum_eq";
pub(crate) const EQ_SUPPORTED_FILTER_TYPES: [&str; 6] = [
    "peaking",
    "low_shelf",
    "high_shelf",
    "low_pass",
    "high_pass",
    "band_pass",
];

pub(crate) const EQ_DEFAULT_FREQUENCIES: [f64; 10] = [
    31.0, 62.0, 125.0, 250.0, 500.0, 1_000.0, 2_000.0, 4_000.0, 8_000.0, 16_000.0,
];

// EQ band structure (parametric)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct EqBand {
    pub(crate) frequency_hz: f64,
    pub(crate) gain_db: f64,
    pub(crate) q: f64,
    pub(crate) filter_type: String,
    pub(crate) enabled: bool,
}

// EQ Preset structure (per output)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct EqPreset {
    pub(crate) name: String,
    pub(crate) device_id: String,
    pub(crate) bands: Vec<EqBand>,
    pub(crate) preamp_db: f64,
    pub(crate) is_builtin: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct EqConfig {
    pub(crate) presets: Vec<EqPreset>,
    pub(crate) device_last_preset: HashMap<String, String>,
}

fn config_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".config").join("stratum")
}

fn eq_config_file() -> PathBuf {
    config_dir().join("audio-eq-presets.json")
}

pub(crate) fn clamp_band(mut band: EqBand) -> EqBand {
    band.frequency_hz = band.frequency_hz.clamp(EQ_MIN_FREQ_HZ, EQ_MAX_FREQ_HZ);
    band.gain_db = band.gain_db.clamp(EQ_MIN_GAIN_DB, EQ_MAX_GAIN_DB);
    band.q = band.q.clamp(EQ_MIN_Q, EQ_MAX_Q);
    if band.filter_type.trim().is_empty() {
        band.filter_type = "peaking".to_string();
    }
    band
}

pub(crate) fn legacy_band(index: usize, gain_db: i32) -> EqBand {
    let frequency_hz = EQ_DEFAULT_FREQUENCIES.get(index).copied().unwrap_or(1000.0);
    clamp_band(EqBand {
        frequency_hz,
        gain_db: gain_db as f64,
        q: EQ_DEFAULT_Q,
        filter_type: "peaking".to_string(),
        enabled: true,
    })
}

pub(crate) fn band_to_json(band: &EqBand) -> Value {
    json!({
        "frequency_hz": band.frequency_hz,
        "gain_db": band.gain_db,
        "q": band.q,
        "filter_type": band.filter_type,
        "enabled": band.enabled,
    })
}

pub(crate) fn legacy_gains_from_bands(bands: &[EqBand]) -> Vec<i32> {
    bands
        .iter()
        .map(|band| band.gain_db.round() as i32)
        .collect()
}

pub(crate) fn normalized_filter_type(raw: &str) -> String {
    raw.trim().to_lowercase().replace('-', "_")
}

fn is_supported_filter_type(raw: &str) -> bool {
    let normalized = normalized_filter_type(raw);
    EQ_SUPPORTED_FILTER_TYPES
        .iter()
        .any(|supported| *supported == normalized)
}

pub(crate) fn validate_parametric_bands(bands: &[EqBand]) -> Result<(), String> {
    if bands.is_empty() {
        return Err("parametric preset requires at least one band".to_string());
    }
    if bands.len() > EQ_MAX_BANDS {
        return Err(format!(
            "too many bands: {} (max {})",
            bands.len(),
            EQ_MAX_BANDS
        ));
    }

    for (index, band) in bands.iter().enumerate() {
        if !(EQ_MIN_FREQ_HZ..=EQ_MAX_FREQ_HZ).contains(&band.frequency_hz) {
            return Err(format!(
                "band {} frequency out of range: {} Hz ({}..={} expected)",
                index, band.frequency_hz, EQ_MIN_FREQ_HZ, EQ_MAX_FREQ_HZ
            ));
        }
        if !(EQ_MIN_GAIN_DB..=EQ_MAX_GAIN_DB).contains(&band.gain_db) {
            return Err(format!(
                "band {} gain out of range: {} dB ({}..={} expected)",
                index, band.gain_db, EQ_MIN_GAIN_DB, EQ_MAX_GAIN_DB
            ));
        }
        if !(EQ_MIN_Q..=EQ_MAX_Q).contains(&band.q) {
            return Err(format!(
                "band {} Q out of range: {} ({}..={} expected)",
                index, band.q, EQ_MIN_Q, EQ_MAX_Q
            ));
        }
        if !is_supported_filter_type(&band.filter_type) {
            return Err(format!(
                "band {} uses unsupported filter_type '{}'",
                index, band.filter_type
            ));
        }
    }

    Ok(())
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

pub(crate) fn load_eq_config() -> EqConfig {
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
                let mut migrated = data.get("version").and_then(Value::as_u64).unwrap_or(1)
                    < EQ_CONFIG_VERSION as u64;
                let mut presets = Vec::new();
                if let Some(presets_arr) = data.get("presets").and_then(|v| v.as_array()) {
                    for preset_val in presets_arr {
                        if let (Some(name), Some(device_id), Some(bands_arr)) = (
                            preset_val.get("name").and_then(|v| v.as_str()),
                            preset_val.get("device_id").and_then(|v| v.as_str()),
                            preset_val.get("bands"),
                        ) {
                            if let Some((bands, used_legacy)) = parser::parse_eq_bands(bands_arr) {
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
                                    is_builtin: preset_val
                                        .get("is_builtin")
                                        .and_then(|v| v.as_bool())
                                        .unwrap_or(false),
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

pub(crate) fn save_eq_config(config_data: &EqConfig) -> Result<(), String> {
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
