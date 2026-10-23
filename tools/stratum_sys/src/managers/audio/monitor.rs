use super::{engine, parser};

use crate::Publisher;
use lazy_static::lazy_static;
use pipewire::keys;
use pipewire::registry::GlobalObject;
use pipewire::types::ObjectType;
use serde_json::json;
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tracing::{error, info, warn};

lazy_static! {
    pub static ref LAST_SYNCED_STATE: Mutex<(String, Instant)> =
        Mutex::new((String::new(), Instant::now()));
    pub static ref IS_RESTORING: Mutex<bool> = Mutex::new(false);
}

pub(crate) async fn run_monitor(publisher: Publisher) {
    if let Some(snapshot) = fetch_current_status() {
        publisher.audio(snapshot.clone());
    }

    let event_publisher = publisher.clone();
    let event_result =
        tokio::task::spawn_blocking(move || run_pipewire_event_loop(event_publisher)).await;

    match event_result {
        Ok(Ok(())) => {
            warn!("pipewire event loop exited unexpectedly; falling back to polling");
        }
        Ok(Err(err)) => {
            warn!("failed to start pipewire event loop: {}; falling back to polling", err);
        }
        Err(err) => {
            warn!("pipewire event worker join error: {}; falling back to polling", err);
        }
    }

    info!("polling PipeWire/Pulse state (fallback mode)");

    loop {
        if let Some(snapshot) = fetch_current_status() {
            if let Some(sink) = snapshot.get("default_sink").and_then(|s| s.as_str()) {
                notify_default_sink_changed(sink);
            }
            publisher.audio(snapshot);
        }

        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// How long PipeWire events are gathered before one refresh. A volume key or a
/// slider drag sends a burst of them, and each refresh opens new Pulse
/// connections.
const REFRESH_DELAY: Duration = Duration::from_millis(40);

/// A bound object whose changes are not registry events, with its listener.
/// The listener is declared first so that it is removed before the proxy.
/// Only held: dropping an entry stops following that object.
#[allow(dead_code)]
enum Watched {
    /// A sink or source: its `Props` param carries volume and mute.
    Node(pipewire::node::NodeListener, pipewire::node::Node),
    /// The `default` metadata: the default sink and source.
    Metadata(pipewire::metadata::MetadataListener, pipewire::metadata::Metadata),
}

fn run_pipewire_event_loop(publisher: Publisher) -> Result<(), String> {
    // Leaked: this thread runs the loop for the life of the process, and the
    // refresh timer, which borrows the loop, is armed from `'static` callbacks.
    let mainloop: &'static pipewire::main_loop::MainLoopRc = Box::leak(Box::new(
        pipewire::main_loop::MainLoopRc::new(None)
            .map_err(|err| format!("failed to create pipewire mainloop: {}", err))?,
    ));
    let context = pipewire::context::ContextRc::new(mainloop, None)
        .map_err(|err| format!("failed to create pipewire context: {}", err))?;
    let core = context
        .connect_rc(None)
        .map_err(|err| format!("failed to connect to pipewire core: {}", err))?;
    let registry = core
        .get_registry_rc()
        .map_err(|err| format!("failed to get pipewire registry: {}", err))?;

    info!("listening for PipeWire registry events");

    let timer_publisher = publisher.clone();
    let timer = Rc::new(
        mainloop
            .loop_()
            .add_timer(move |_| refresh_from_system(&timer_publisher)),
    );
    // Re-arming moves the refresh to the end of the burst.
    let schedule: Rc<dyn Fn()> = Rc::new(move || {
        let _ = timer.update_timer(Some(REFRESH_DELAY), None);
    });

    // Registry events say only that objects came or went. Volume, mute and
    // the default devices change on objects that already exist, so sinks,
    // sources and the default metadata are bound and followed too; without
    // that, a volume key changed nothing until some stream appeared.
    let watched = Rc::new(RefCell::new(HashMap::<u32, Watched>::new()));
    let relevant_ids = Rc::new(RefCell::new(HashSet::<u32>::new()));
    let relevant_ids_on_global = Rc::clone(&relevant_ids);
    let relevant_ids_on_remove = Rc::clone(&relevant_ids);
    let watched_on_global = Rc::clone(&watched);
    let watched_on_remove = Rc::clone(&watched);
    let schedule_on_global = Rc::clone(&schedule);
    let schedule_on_remove = Rc::clone(&schedule);
    let registry_weak = registry.downgrade();
    let _listener = registry
        .add_listener_local()
        .global(move |global| {
            if let Some(registry) = registry_weak.upgrade() {
                if let Some(entry) = watch(&registry, global, &schedule_on_global) {
                    watched_on_global.borrow_mut().insert(global.id, entry);
                }
            }
            if should_refresh_on_global(global) {
                relevant_ids_on_global.borrow_mut().insert(global.id);
                schedule_on_global();
            } else {
                // If this id is re-announced with non-audio props, stop tracking it.
                relevant_ids_on_global.borrow_mut().remove(&global.id);
            }
        })
        .global_remove(move |id| {
            watched_on_remove.borrow_mut().remove(&id);
            if relevant_ids_on_remove.borrow_mut().remove(&id) {
                schedule_on_remove();
            }
        })
        .register();

    // Seed state once after listener setup in case no immediate global event is emitted.
    refresh_from_system(&publisher);

    // Runs indefinitely and dispatches registry callbacks in this worker thread.
    mainloop.run();
    Ok(())
}

/// Bind `global` and follow its changes if it is a sink, a source, or the
/// `default` metadata.
fn watch(
    registry: &pipewire::registry::RegistryRc,
    global: &GlobalObject<&pipewire::spa::utils::dict::DictRef>,
    schedule: &Rc<dyn Fn()>,
) -> Option<Watched> {
    match global.type_ {
        ObjectType::Node
            if prop_starts_with(global, &keys::MEDIA_CLASS, "Audio/Sink")
                || prop_starts_with(global, &keys::MEDIA_CLASS, "Audio/Source") =>
        {
            let node: pipewire::node::Node = registry.bind(global).ok()?;
            let schedule = Rc::clone(schedule);
            let listener = node
                .add_listener_local()
                .param(move |_, kind, _, _, _| {
                    if kind == pipewire::spa::param::ParamType::Props {
                        schedule();
                    }
                })
                .register();
            node.subscribe_params(&[pipewire::spa::param::ParamType::Props]);
            Some(Watched::Node(listener, node))
        }
        ObjectType::Metadata if prop_value(global, "metadata.name") == Some("default") => {
            let metadata: pipewire::metadata::Metadata = registry.bind(global).ok()?;
            let schedule = Rc::clone(schedule);
            let listener = metadata
                .add_listener_local()
                .property(move |_, key, _, _| {
                    if key.is_none_or(|key| key.starts_with("default.audio.")) {
                        schedule();
                    }
                    0
                })
                .register();
            Some(Watched::Metadata(listener, metadata))
        }
        _ => None,
    }
}

fn should_refresh_on_global_type(kind: &ObjectType) -> bool {
    matches!(kind, ObjectType::Metadata | ObjectType::Device | ObjectType::Node | ObjectType::Port | ObjectType::ClientNode)
}

fn should_refresh_on_global(global: &GlobalObject<&pipewire::spa::utils::dict::DictRef>) -> bool {
    if !should_refresh_on_global_type(&global.type_) {
        return false;
    }

    match global.type_ {
        // Metadata includes default-node changes and policy metadata that can affect routing.
        ObjectType::Metadata => true,
        ObjectType::Device => {
            has_audio_media_class(global)
                || prop_contains(global, &keys::DEVICE_CLASS, "audio")
                || prop_contains(global, &keys::DEVICE_API, "alsa")
                || prop_contains(global, &keys::DEVICE_API, "bluez")
        }
        ObjectType::Node | ObjectType::Port | ObjectType::ClientNode => {
            has_audio_media_class(global)
                || prop_starts_with(global, &keys::NODE_NAME, "alsa_")
                || prop_starts_with(global, &keys::NODE_NAME, "bluez_")
                || prop_starts_with(global, &keys::NODE_NAME, "effect_")
        }
        _ => false,
    }
}

fn has_audio_media_class(global: &GlobalObject<&pipewire::spa::utils::dict::DictRef>) -> bool {
    prop_starts_with(global, &keys::MEDIA_CLASS, "Audio/")
}

fn prop_value<'a>(
    global: &'a GlobalObject<&pipewire::spa::utils::dict::DictRef>,
    key: &str,
) -> Option<&'a str> {
    global.props.and_then(|props| props.get(key))
}

fn prop_starts_with(
    global: &GlobalObject<&pipewire::spa::utils::dict::DictRef>,
    key: &str,
    needle: &str,
) -> bool {
    prop_value(global, key)
        .map(|value| value.starts_with(needle))
        .unwrap_or(false)
}

fn prop_contains(
    global: &GlobalObject<&pipewire::spa::utils::dict::DictRef>,
    key: &str,
    needle: &str,
) -> bool {
    prop_value(global, key)
        .map(|value| value.to_lowercase().contains(needle))
        .unwrap_or(false)
}

fn refresh_from_system(publisher: &Publisher) {
    let _ = super::refresh_device_cache();
    if let Some(snapshot) = fetch_current_status() {
        if let Some(sink) = snapshot.get("default_sink").and_then(|s| s.as_str()) {
            notify_default_sink_changed(sink);
        }
        publisher.audio(snapshot);
    }
}

/// The snapshot shape this module publishes, read synchronously.
///
/// Exposed so the UI can refresh immediately after it changes something,
/// rather than waiting for the PipeWire event loop to deliver the change.
pub(crate) fn current_status() -> Option<Value> {
    fetch_current_status()
}

fn fetch_current_status() -> Option<Value> {
    let fresh = parser::fetch_current_audio_status();
    if !fresh.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        return None;
    }

    let effective_sink = fresh
        .get("default")
        .and_then(|v| v.get("sink"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let default_source = fresh
        .get("default")
        .and_then(|v| v.get("source"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let volume = fresh
        .get("status")
        .and_then(|v| v.get("volume"))
        .and_then(Value::as_str)
        .unwrap_or("0%")
        .to_string();
    let mute = fresh
        .get("status")
        .and_then(|v| v.get("mute"))
        .and_then(Value::as_str)
        .unwrap_or("yes")
        .to_string();
    let status = |key: &str, fallback: &str| {
        fresh
            .get("status")
            .and_then(|v| v.get(key))
            .and_then(Value::as_str)
            .unwrap_or(fallback)
            .to_string()
    };
    let input_volume = status("input_volume", "0%");
    let input_mute = status("input_mute", "yes");
    let sinks = fresh.get("sinks").cloned().unwrap_or_else(|| json!([]));
    let sources = fresh.get("sources").cloned().unwrap_or_else(|| json!([]));

    let headphones = fresh
        .get("headphones")
        .and_then(Value::as_str)
        .unwrap_or("no")
        .to_string();

    Some(json!({
        "default_sink": effective_sink,
        "default_source": default_source,
        "volume": volume,
        "mute": mute,
        "input_volume": input_volume,
        "input_mute": input_mute,
        "headphones": headphones,
        "sinks": sinks,
        "sources": sources,
    }))
}

fn notify_default_sink_changed(new_sink_raw: &str) {
    if *IS_RESTORING.lock().unwrap() {
        return;
    }

    // Skip if the new sink IS the EQ virtual sink itself
    if engine::is_eq_virtual_sink_name(new_sink_raw) {
        return;
    }

    let effective = parser::resolve_effective_default_sink(new_sink_raw);
    if effective.is_empty() {
        return;
    }

    let mut lock = LAST_SYNCED_STATE.lock().unwrap();
    let (last_sink, last_time) = &*lock;

    // Throttle: don't auto-restore more than once every 3 seconds to avoid loops/flapping
    if last_time.elapsed() < Duration::from_secs(3) {
        return;
    }

    if *last_sink != effective {
        info!(
            "hardware output changed to {}, restoring device EQ profile",
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
                error!(
                    "auto-restore failed for {}: {:?}",
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

pub(crate) fn update_last_synced_state(sink_name: &str) {
    if let Ok(mut lock) = LAST_SYNCED_STATE.lock() {
        *lock = (sink_name.to_string(), Instant::now());
    }
}
