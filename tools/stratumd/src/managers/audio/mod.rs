use crate::managers::common::{run_capture_optional, run_command_capture};
use serde_json::{json, Value};
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;
use tracing::info;

pub(crate) mod config;
pub(crate) mod engine;
pub(crate) mod monitor;
pub(crate) mod parser;

use crate::AppState;

pub fn spawn_monitor(state: Arc<AppState>) {
    tokio::spawn(async move {
        info!("[stratumd] [audio:monitor] starting audio monitor");
        monitor::run_monitor(state).await;
    });
}

pub fn initialize() {
    cleanup_orphans();
    let default_sink_raw =
        crate::managers::common::run_command_capture("pactl", &["get-default-sink"])
            .unwrap_or_default();
    let effective = parser::resolve_effective_default_sink(default_sink_raw.trim());

    println!(
        "[audio] [info] initializing audio manager with effective sink: {}",
        effective
    );

    if let Ok(mut lock) = monitor::LAST_SYNCED_STATE.lock() {
        *lock = (effective.clone(), Instant::now());
    }

    if !engine::is_eq_virtual_sink_name(&effective) && !effective.is_empty() {
        let _ = engine::auto_apply_preset_for_device(&effective);
    }
}

pub fn cleanup_orphans() {
    println!("[audio] [info] cleaning up orphaned EQ processes");
    let _ = Command::new("pkill")
        .args(["-f", "pw-cli.*stratum_eq"])
        .status();
}

pub(crate) fn refresh_device_cache() -> Value {
    let result = parser::fetch_current_audio_status();

    if let Ok(mut cache) = engine::DEVICE_CACHE.lock() {
        *cache = Some(result.clone());
    }
    result
}

pub fn set_volume(percent: i64) -> Value {
    let p = percent.clamp(0, 150).to_string();
    let _ = crate::managers::common::run_command_capture(
        "pactl",
        &["set-sink-volume", "@DEFAULT_SINK@", &format!("{}%", p)],
    );
    refresh_device_cache();
    serde_json::json!({"ok": true, "volume": p})
}

pub fn set_input(target: &str) -> Value {
    let _ = run_command_capture("pactl", &["set-default-source", target]);
    refresh_device_cache();
    json!({"ok": true, "source": target})
}

pub fn set_output(target: &str) -> Value {
    let target = target.trim();
    if target.is_empty() {
        return serde_json::json!({"ok": false, "error": "missing target sink"});
    }

    info!("[stratumd] [audio] switching output to: {}", target);
    let res = engine::auto_apply_preset_for_device(target);

    monitor::update_last_synced_state(target);

    refresh_device_cache();
    res
}

pub fn eq_list_presets(device_id: &str) -> Value {
    let config = config::load_eq_config();
    let presets: Vec<_> = config
        .presets
        .iter()
        .filter(|p| p.device_id == device_id || p.device_id == "@DEFAULT_SINK@")
        .map(|p| {
            json!({
                "name": p.name,
                "is_builtin": p.is_builtin,
                "bands": config::legacy_gains_from_bands(&p.bands),
                "parametric_bands": p.bands.iter().map(config::band_to_json).collect::<Vec<_>>(),
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
        "capabilities": engine::eq_capabilities_json(),
        "presets": presets,
    })
}

pub fn eq_apply_preset(device_id: &str, preset_name: &str) -> Value {
    let mut config = config::load_eq_config();
    let preset = config
        .presets
        .iter()
        .find(|p| {
            p.name == preset_name && (p.device_id == device_id || p.device_id == "@DEFAULT_SINK@")
        })
        .cloned();

    if let Some(preset) = preset {
        let apply_result = engine::apply_eq_bands(device_id, &preset.bands, preset.preamp_db);

        config
            .device_last_preset
            .insert(device_id.to_string(), preset_name.to_string());
        let _ = config::save_eq_config(&config);

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
    let Some((parsed_bands, _)) = parser::parse_eq_bands(bands) else {
        return json!({"ok": false, "error": "failed to parse parametric bands"});
    };

    let apply_result = engine::apply_eq_bands(device_id, &parsed_bands, preamp_db);

    match apply_result {
        Ok(res) => json!({
            "ok": true,
            "device_id": device_id,
            "preset": "Custom",
            "applied": res.applied,
            "engine": res.engine,
            "status": res.status,
            "parametric_bands": parsed_bands,
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
    let Some((parsed_bands, _)) = parser::parse_eq_bands(bands) else {
        return json!({"ok": false, "error": "failed to parse parametric bands"});
    };

    if let Err(err) = config::validate_parametric_bands(&parsed_bands) {
        return json!({"ok": false, "error": err});
    }

    let mut config = config::load_eq_config();
    config.presets.retain(|p| {
        !(p.name == preset_name
            && (p.device_id == device_id || p.device_id == "@DEFAULT_SINK@")
            && !p.is_builtin)
    });

    config.presets.push(config::EqPreset {
        name: preset_name.to_string(),
        device_id: device_id.to_string(),
        bands: parsed_bands,
        preamp_db,
        is_builtin: false,
    });

    config
        .device_last_preset
        .insert(device_id.to_string(), preset_name.to_string());
    if let Err(err) = config::save_eq_config(&config) {
        return json!({"ok": false, "error": err});
    }

    json!({"ok": true, "preset": preset_name, "saved": true})
}

pub fn eq_delete_preset(device_id: &str, preset_name: &str) -> Value {
    let mut config = config::load_eq_config();
    let original_len = config.presets.len();
    config.presets.retain(|p| {
        !(p.name == preset_name
            && (p.device_id == device_id || p.device_id == "@DEFAULT_SINK@")
            && !p.is_builtin)
    });

    if config.presets.len() == original_len {
        return json!({"ok": false, "error": format!("preset '{}' not found", preset_name)});
    }

    if let Err(err) = config::save_eq_config(&config) {
        return json!({"ok": false, "error": err});
    }

    json!({"ok": true, "deleted": true})
}

pub fn media_seek(position_sec: i64) -> Value {
    let player = run_capture_optional("playerctl", &["-l"])
        .lines()
        .next()
        .unwrap_or_default()
        .to_string();
    if player.is_empty() {
        return json!({"ok": false, "error": "no active media player"});
    }

    let seek_micro = (position_sec * 1_000_000).to_string();
    let _ = run_command_capture("playerctl", &["-p", &player, "position", &seek_micro]);
    json!({"ok": true, "player": player})
}

pub fn devices() -> Value {
    let cache = engine::DEVICE_CACHE.lock().unwrap();
    if let Some(data) = &*cache {
        return data.clone();
    }

    // Fallback if cache is empty (usually only on first boot)
    drop(cache);
    refresh_device_cache()
}
