use crate::managers::common::{run_capture_optional, run_command_capture};
use pulsectl::controllers::{DeviceControl, SinkController, SourceController};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Instant;
use tracing::{info, warn};

pub(crate) mod config;
pub(crate) mod engine;
pub(crate) mod monitor;
pub(crate) mod native_eq;
pub(crate) mod parser;

use crate::AppState;

pub fn spawn_monitor(state: Arc<AppState>) {
    tokio::spawn(async move {
        info!("starting audio monitor");
        monitor::run_monitor(state).await;
    });
}

pub fn initialize() {
    // Initialize native PipeWire backend (non-fatal if it fails; pw-cli fallback is used).
    if let Err(e) = native_eq::init() {
        warn!("native PipeWire init failed (will use pw-cli fallback): {}", e);
    }
    engine::warm_pw_cli_availability_cache();
    cleanup_orphans();
    let default_sink_raw = SinkController::create()
        .ok()
        .and_then(|mut ctrl| ctrl.get_server_info().ok())
        .and_then(|info| info.default_sink_name)
        .unwrap_or_default();
    let effective = parser::resolve_effective_default_sink(default_sink_raw.trim());

    info!(
        "initializing audio manager with effective sink: {}",
        effective
    );

    if let Ok(mut lock) = monitor::LAST_SYNCED_STATE.lock() {
        *lock = (effective.clone(), Instant::now());
    }

    if !engine::is_eq_virtual_sink_name(&effective) && !effective.is_empty() {
        monitor::update_last_synced_state(&effective);
    }

    if !engine::is_eq_virtual_sink_name(&effective) && !effective.is_empty() {
        let _ = engine::auto_apply_preset_for_device(&effective);
    }
}

pub fn cleanup_orphans() {
    info!("cleaning up lingering EQ graph objects");
    engine::destroy_eq_module();
}

pub(crate) fn refresh_device_cache() -> Value {
    let result = parser::fetch_current_audio_status();

    if let Ok(mut cache) = engine::DEVICE_CACHE.lock() {
        *cache = Some(result.clone());
    }
    result
}

pub fn set_volume(percent: i64) -> Value {
    let target = percent.clamp(0, 150) as f64;
    let mut sink_ctrl = match SinkController::create() {
        Ok(ctrl) => ctrl,
        Err(err) => {
            return json!({"ok": false, "error": format!("sink controller unavailable: {}", err)});
        }
    };

    let default_device = match sink_ctrl.get_default_device() {
        Ok(device) => device,
        Err(err) => {
            return json!({"ok": false, "error": format!("failed to query default sink: {}", err)});
        }
    };

    let current = parser::extract_first_percent(&default_device.volume.print())
        .and_then(|v| v.trim_end_matches('%').parse::<f64>().ok())
        .unwrap_or(0.0);

    let delta = (target - current).abs() / 100.0;
    if target > current {
        sink_ctrl.increase_device_volume_by_percent(default_device.index, delta);
    } else if target < current {
        sink_ctrl.decrease_device_volume_by_percent(default_device.index, delta);
    }

    refresh_device_cache();
    serde_json::json!({"ok": true, "volume": target.round() as i64})
}

pub fn set_input(target: &str) -> Value {
    let mut source_ctrl = match SourceController::create() {
        Ok(ctrl) => ctrl,
        Err(err) => {
            return json!({"ok": false, "error": format!("source controller unavailable: {}", err)});
        }
    };

    match source_ctrl.set_default_device(target) {
        Ok(true) => {}
        Ok(false) => {
            return json!({"ok": false, "error": format!("source '{}' not accepted", target)});
        }
        Err(err) => {
            return json!({"ok": false, "error": format!("failed to set default source: {}", err)});
        }
    }

    refresh_device_cache();
    json!({"ok": true, "source": target})
}

pub fn set_output(target: &str) -> Value {
    let target = target.trim();
    if target.is_empty() {
        return serde_json::json!({"ok": false, "error": "missing target sink"});
    }

    // Skip if we're already on this device
    if let Ok(mut ctrl) = SinkController::create() {
        if let Ok(current_device) = ctrl.get_default_device() {
            if let Some(current_name) = &current_device.name {
                if current_name == target {
                    return json!({"ok": true, "sink": target, "note": "already on target device"});
                }
            }
        }
    }

    // Prevent monitor from triggering auto-restore while we're switching output
    // Keep the lock active for 3+ seconds to let PulseAudio settle
    {
        let mut guard = monitor::IS_RESTORING.lock().unwrap();
        *guard = true;
    }

    info!("switching output to: {}", target);
    let mut res = engine::auto_apply_preset_for_device(target);
    if !res.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        match SinkController::create()
            .ok()
            .map(|mut ctrl| ctrl.set_default_device(target))
        {
            Some(Ok(true)) => {
                res = json!({
                    "ok": true,
                    "sink": target,
                    "warning": "output switched but EQ apply failed",
                    "eq_error": res.get("error").cloned().unwrap_or_else(|| Value::String("unknown error".to_string())),
                });
            }
            Some(Ok(false)) => {
                res = json!({
                    "ok": false,
                    "error": format!("failed to switch sink and apply EQ: sink '{}' rejected", target),
                    "eq_error": res.get("error").cloned().unwrap_or_else(|| Value::String("unknown error".to_string())),
                });
            }
            Some(Err(err)) => {
                res = json!({
                    "ok": false,
                    "error": format!("failed to switch sink and apply EQ: {}", err),
                    "eq_error": res.get("error").cloned().unwrap_or_else(|| Value::String("unknown error".to_string())),
                });
            }
            None => {
                res = json!({
                    "ok": false,
                    "error": "failed to switch sink and apply EQ: sink controller unavailable",
                    "eq_error": res.get("error").cloned().unwrap_or_else(|| Value::String("unknown error".to_string())),
                });
            }
        }
    }

    if res.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        monitor::update_last_synced_state(target);
    }

    // Re-enable auto-restore after 3 seconds to let PulseAudio fully settle
    let _ = std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(3));
        if let Ok(mut guard) = monitor::IS_RESTORING.lock() {
            *guard = false;
        }
    });

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

        match apply_result {
            Ok(res) => {
                config
                    .device_last_preset
                    .insert(device_id.to_string(), preset_name.to_string());
                let _ = config::save_eq_config(&config);
                json!({
                    "ok": true,
                    "device_id": device_id,
                    "preset": preset_name,
                    "applied": res.applied,
                    "engine": res.engine,
                    "status": res.status,
                    "parametric_bands": preset.bands,
                    "preamp_db": preset.preamp_db,
                })
            }
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
    // Always read live state for device listings so UI views cannot regress to stale cache.
    // The refresh helper keeps DEVICE_CACHE in sync for other call sites.
    refresh_device_cache()
}
