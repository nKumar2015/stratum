use pulsectl::controllers::types::DeviceInfo;
use pulsectl::controllers::DeviceControl;
use pulsectl::ControllerError;
use serde_json::{json, Value};
use std::time::Instant;
use tracing::{info, warn};

pub(crate) mod config;
pub(crate) mod engine;
pub(crate) mod monitor;
pub(crate) mod native_eq;
pub(crate) mod parser;
pub(crate) mod pulse;

use crate::Publisher;

pub fn spawn_monitor(publisher: Publisher) {
    tokio::spawn(async move {
        info!("starting audio monitor");
        monitor::run_monitor(publisher).await;
    });
}

pub fn initialize() {
    // Initialize native PipeWire backend (non-fatal if it fails; pw-cli fallback is used).
    if let Err(e) = native_eq::init() {
        warn!("native PipeWire init failed (will use pw-cli fallback): {}", e);
    }
    engine::warm_pw_cli_availability_cache();
    cleanup_orphans();
    let default_sink_raw = pulse::default_sink_name().unwrap_or_default();
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

fn cleanup_orphans() {
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

/// The snapshot shape the monitor publishes, produced synchronously.
///
/// The UI uses this after a user action so the change is visible on the next
/// frame instead of waiting for the PipeWire event loop to notice.
pub fn snapshot() -> Option<Value> {
    monitor::current_status()
}

pub fn set_volume(percent: i64) -> Value {
    let result = pulse::with_sink(|ctrl| set_default_volume(ctrl, percent));
    volume_result(result, "sink")
}

/// Set the default source (microphone) to an absolute percentage.
pub fn set_input_volume(percent: i64) -> Value {
    let result = pulse::with_source(|ctrl| set_default_volume(ctrl, percent));
    volume_result(result, "source")
}

fn volume_result(result: Result<i64, ControllerError>, kind: &str) -> Value {
    refresh_device_cache();
    match result {
        Ok(volume) => json!({"ok": true, "volume": volume}),
        Err(err) => json!({"ok": false, "error": format!("failed to set {} volume: {}", kind, err)}),
    }
}

fn set_default_volume<C: DeviceControl<DeviceInfo>>(ctrl: &mut C, percent: i64) -> Result<i64, ControllerError> {
    let target = percent.clamp(0, 150) as f64;
    let default_device = ctrl.get_default_device()?;

    let current = parser::extract_first_percent(&default_device.volume.print())
        .and_then(|v| v.trim_end_matches('%').parse::<f64>().ok())
        .unwrap_or(0.0);

    let delta = (target - current).abs() / 100.0;
    if target > current {
        ctrl.increase_device_volume_by_percent(default_device.index, delta);
    } else if target < current {
        ctrl.decrease_device_volume_by_percent(default_device.index, delta);
    }
    Ok(target.round() as i64)
}

/// Mute or unmute the default sink.
///
/// Volume is deliberately left alone: muting used to be implemented as
/// "set volume to 0", which destroyed the level the user had set. The sink's
/// own mute flag is the right control and is what `pactl`/`wpctl` use too.
pub fn set_mute(muted: bool) -> Value {
    let result = pulse::with_sink(|ctrl| set_default_mute(ctrl, muted));
    mute_result(result, "sink", muted)
}

/// Mute or unmute the default source (microphone).
pub fn set_input_mute(muted: bool) -> Value {
    let result = pulse::with_source(|ctrl| set_default_mute(ctrl, muted));
    mute_result(result, "source", muted)
}

fn mute_result(result: Result<(), ControllerError>, kind: &str, muted: bool) -> Value {
    refresh_device_cache();
    match result {
        Ok(()) => json!({"ok": true, "mute": if muted { "yes" } else { "no" }}),
        Err(err) => json!({"ok": false, "error": format!("failed to mute default {}: {}", kind, err)}),
    }
}

fn set_default_mute<C: DeviceControl<DeviceInfo>>(ctrl: &mut C, muted: bool) -> Result<(), ControllerError> {
    let default_device = ctrl.get_default_device()?;
    // `set_device_mute_by_index` returns `()`, so a failure to look the device
    // up beforehand is the only signal that the device is unreachable.
    ctrl.get_device_by_index(default_device.index)?;
    ctrl.set_device_mute_by_index(default_device.index, muted);
    Ok(())
}

pub fn set_input(target: &str) -> Value {
    match pulse::with_source(|ctrl| ctrl.set_default_device(target)) {
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
    if let Ok(current_device) = pulse::with_sink(|ctrl| ctrl.get_default_device()) {
        if current_device.name.as_deref() == Some(target) {
            return json!({"ok": true, "sink": target, "note": "already on target device"});
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
        match pulse::with_sink(|ctrl| ctrl.set_default_device(target)) {
            Ok(true) => {
                res = json!({
                    "ok": true,
                    "sink": target,
                    "warning": "output switched but EQ apply failed",
                    "eq_error": res.get("error").cloned().unwrap_or_else(|| Value::String("unknown error".to_string())),
                });
            }
            Ok(false) => {
                res = json!({
                    "ok": false,
                    "error": format!("failed to switch sink and apply EQ: sink '{}' rejected", target),
                    "eq_error": res.get("error").cloned().unwrap_or_else(|| Value::String("unknown error".to_string())),
                });
            }
            Err(err) => {
                res = json!({
                    "ok": false,
                    "error": format!("failed to switch sink and apply EQ: {}", err),
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
