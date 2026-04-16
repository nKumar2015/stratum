use super::{engine, parser};

use crate::AppState;
use lazy_static::lazy_static;
use serde_json::json;
use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::error;
lazy_static! {
    pub static ref LAST_SYNCED_STATE: Mutex<(String, Instant)> =
        Mutex::new((String::new(), Instant::now()));
    static ref IS_RESTORING: Mutex<bool> = Mutex::new(false);
}
use crate::managers::common::run_command_capture;

pub(crate) async fn run_monitor(state: Arc<AppState>) {
    let mut interval = tokio::time::interval(Duration::from_secs(2));
    let mut iteration = 0;

    loop {
        interval.tick().await;

        let snapshot = match tokio::task::spawn_blocking(status).await {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => {
                error!(
                    "[stratumd] [audio:monitor] Audio status fetch failed due to err: \n\t{}",
                    e
                );
                continue;
            }
            Err(e) => {
                error!(
                    "[stratumd] [audio:monitor] Audio status thread panicked due to err: \n\t{}",
                    e
                );
                continue;
            }
        };

        // refactor to use dbus signal (?)
        if let Some(sink) = snapshot.get("default_sink").and_then(|s| s.as_str()) {
            notify_default_sink_changed(sink);
        }

        state.audio.update(snapshot);

        iteration += 2;
        if iteration >= 10 {
            let _ = tokio::task::spawn_blocking(|| {
                super::refresh_device_cache();
            })
            .await;
            iteration = 0;
        }
    }
}

fn notify_default_sink_changed(new_sink_raw: &str) {
    if *IS_RESTORING.lock().unwrap() {
        return;
    }

    let effective = parser::resolve_effective_default_sink(new_sink_raw);
    if engine::is_eq_virtual_sink_name(&effective) || effective.is_empty() {
        return;
    }

    let mut lock = LAST_SYNCED_STATE.lock().unwrap();
    let (last_sink, last_time) = &*lock;

    // Throttle: don't auto-restore more than once every 3 seconds to avoid loops/flapping
    if last_time.elapsed() < Duration::from_secs(3) {
        return;
    }

    if *last_sink != effective {
        println!(
            "[audio] [info] hardware output changed to {}, restoring device EQ profile",
            effective
        );
        *lock = (effective.clone(), Instant::now());

        // Asynchronously restore preset to avoid blocking the monitor loop
        let target = effective.clone();
        std::thread::spawn(move || {
            {
                let mut guard = IS_RESTORING.lock().unwrap();
                *guard = true;
            }

            let res = engine::auto_apply_preset_for_device(&target);

            if !res.get("ok").and_then(Value::as_bool).unwrap_or(false) {
                println!(
                    "[audio] [error] auto-restore failed for {}: {:?}",
                    target,
                    res.get("error")
                );
            }

            {
                let mut guard = IS_RESTORING.lock().unwrap();
                *guard = false;
            }
        });
    }
}

pub fn status() -> Result<Value, String> {
    let get_err = |cmd: &str, e: String| format!("Failed to run {}: \n\t{}", cmd, e);

    let default_sink_raw = run_command_capture("pactl", &["get-default-sink"])
        .map_err(|e| get_err("pactl get-default-sink", e))?;

    let default_sink = parser::resolve_effective_default_sink(&default_sink_raw);

    let default_source = run_command_capture("pactl", &["get-default-source"])
        .map_err(|e| get_err("pactl get-default-source", e))?;

    let vol_string = run_command_capture("pactl", &["get-sink-volume", "@DEFAULT_SINK@"])
        .map_err(|e| get_err("pactl get-sink-volume", e))?;
    let volume = parser::extract_first_percent(&vol_string)
        .ok_or_else(|| "Could not parse volume percentage".to_string())?;

    let mute_string = run_command_capture("pactl", &["get-sink-mute", "@DEFAULT_SINK@"])
        .map_err(|e| get_err("pactl get-sink-mute", e))?;
    let mute = parser::extract_first_percent(&mute_string)
        .ok_or_else(|| "Could not parse mute state".to_string())?;

    let headphones = parser::is_headphone_default_sink(&default_sink);

    let (sinks, sources) = {
        let cache = engine::DEVICE_CACHE
            .lock()
            .map_err(|_| "Poisned lock on DEVICE_CACHE".to_string())?;

        if let Some(val) = &*cache {
            (
                val.get("sinks").cloned().unwrap_or_else(|| json!([])),
                val.get("sources").cloned().unwrap_or_else(|| json!([])),
            )
        } else {
            (json!([]), json!([]))
        }
    };

    Ok(json!({
        "default_sink": default_sink,
        "default_source": default_source,
        "volume": volume,
        "mute": mute,
        "headphones": headphones,
        "sinks": sinks,
        "sources": sources,
    }))
}

pub(crate) fn update_last_synced_state(sink_name: &str) {
    if let Ok(mut lock) = LAST_SYNCED_STATE.lock() {
        *lock = (sink_name.to_string(), Instant::now());
    }
}
