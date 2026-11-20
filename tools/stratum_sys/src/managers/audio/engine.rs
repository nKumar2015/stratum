use super::{config, native_eq, parser, pulse, routes};
use tracing::{info, warn};

use crate::managers::common::{command_available, run_command_capture};
use pulsectl::controllers::{AppControl, DeviceControl};
use serde_json::{json, Value};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Debug)]
pub(crate) struct EqApplyResult {
    pub(crate) applied: bool,
    pub(crate) _dry_run: bool,
    pub(crate) engine: String,
    pub(crate) _resolved_device: String,
    pub(crate) status: String,
}

static EQ_TARGET_SINK: Mutex<Option<String>> = Mutex::new(None);
static GLOBAL_EQ_LOCK: Mutex<()> = Mutex::new(());
pub(crate) static DEVICE_CACHE: Mutex<Option<Value>> = Mutex::new(None);

static PW_CLI_AVAILABLE: OnceLock<bool> = OnceLock::new();

pub(crate) fn warm_pw_cli_availability_cache() {
    let _ = pw_cli_available();
}

pub(crate) fn pw_cli_available() -> bool {
    *PW_CLI_AVAILABLE.get_or_init(|| command_available("pw-cli"))
}

pub(crate) fn apply_eq_bands(
    device_id: &str,
    bands: &[config::EqBand],
    preamp_db: f64,
) -> Result<EqApplyResult, String> {
    config::validate_parametric_bands(bands)?;

    if native_eq::is_available() || pw_cli_available() {
        return apply_eq_bands_pipewire(device_id, bands, preamp_db);
    }

    Err("no supported audio processing engine found (requires pw-cli or native PipeWire)".to_string())
}

fn apply_eq_bands_pipewire(
    device_id: &str,
    bands: &[config::EqBand],
    preamp_db: f64,
) -> Result<EqApplyResult, String> {
    if !pw_cli_available() {
        return Err("pw-cli not found - EQ not applied".to_string());
    }

    let resolved_device = if device_id == "@DEFAULT_SINK@" {
        let def = pulse::default_sink_name().unwrap_or_else(|| device_id.to_string());
        parser::resolve_effective_default_sink(def.trim())
    } else {
        device_id.to_string()
    };

    let _lifecycle_guard = GLOBAL_EQ_LOCK.lock().unwrap();
    // Only the stage behind the sink is rebuilt: apps keep their sink.
    destroy_node_named(config::EQ_STAGE_NODE);
    std::thread::sleep(std::time::Duration::from_millis(150));

    if is_eq_virtual_sink_name(&resolved_device) || resolved_device.is_empty() {
        warn!(
            "resolved_device for EQ is invalid ('{}'), aborting load",
            resolved_device
        );
        return Err("invalid hardware target for EQ".to_string());
    }

    info!(
        "loading EQ module targeting device: {}",
        resolved_device
    );

    ensure_main_sink()?;
    let quote = |text: &str| text.replace('\\', "\\\\").replace('"', "\\\"");
    let module_args = format!(
        "{{ node.description = \"Stratum Parametric EQ stage\" media.name = \"Stratum Parametric EQ stage\" filter.graph = {graph} capture.props = {{ node.name = \"{stage}\" node.passive = true node.autoconnect = false audio.channels = 2 audio.position = [ FL FR ] }} playback.props = {{ node.name = \"{output}\" node.passive = true node.autoconnect = false audio.channels = 2 audio.position = [ FL FR ] target.object = \"{target}\" }} }}",
        graph = filter_graph(bands, preamp_db),
        stage = config::EQ_STAGE_NODE,
        output = config::EQ_VIRTUAL_OUTPUT_NODE,
        target = quote(&resolved_device),
    );
    load_eq_module(&module_args)?;
    wait_for_node(config::EQ_STAGE_NODE);

    let current_default = pulse::default_sink_name().unwrap_or_default();
    if current_default.trim() != config::EQ_VIRTUAL_INPUT_SINK {
        let _ = pulse::with_sink(|ctrl| ctrl.set_default_device(config::EQ_VIRTUAL_INPUT_SINK));
    }
    move_active_streams_to_eq(config::EQ_VIRTUAL_INPUT_SINK);

    let target = resolved_device.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(400));
        link_nodes(config::EQ_BRIDGE_NODE, config::EQ_STAGE_NODE);
        purge_eq_output_links();
        info!("ensuring manual link: EQ -> {}", target);
        ensure_eq_output_links(&target);
    });

    if let Ok(mut target) = EQ_TARGET_SINK.lock() {
        *target = Some(resolved_device.clone());
    }

    // An EQ of the device's own, for apps sent to it, takes the new curve too.
    routes::retune(&resolved_device, bands, preamp_db);

    Ok(EqApplyResult {
        applied: true,
        _dry_run: false,
        engine: "pipewire-filter-chain".to_string(),
        _resolved_device: resolved_device,
        status: format!(
            "applied {} enabled bands via PipeWire filter-chain virtual sink",
            bands.iter().filter(|b| b.enabled).count()
        ),
    })
}

/// The main EQ's sink, which apps play into: a loopback made once and kept,
/// in front of the EQ stage that is rebuilt whenever the curve or the
/// output changes. A stream that asked not to be moved (Electron apps that
/// chose an output) is ended when the sink it asked for goes, which every
/// output change used to do when the sink was the EQ itself.
fn ensure_main_sink() -> Result<(), String> {
    if find_node_id_by_name(config::EQ_VIRTUAL_INPUT_SINK).is_some() {
        return Ok(());
    }
    let args = format!(
        "{{ node.description = \"Stratum Parametric EQ\" media.name = \"Stratum Parametric EQ\" capture.props = {{ node.name = \"{sink}\" media.class = Audio/Sink audio.channels = 2 audio.position = [ FL FR ] }} playback.props = {{ node.name = \"{bridge}\" node.passive = true node.autoconnect = false audio.channels = 2 audio.position = [ FL FR ] }} }}",
        sink = config::EQ_VIRTUAL_INPUT_SINK,
        bridge = config::EQ_BRIDGE_NODE,
    );
    if native_eq::is_available() {
        native_eq::load_module("libpipewire-module-loopback", &args)?;
    } else {
        load_with_pw_cli("libpipewire-module-loopback", &args)?;
    }
    if !wait_for_node(config::EQ_VIRTUAL_INPUT_SINK) {
        return Err("the EQ's sink did not appear".to_string());
    }
    info!("made the EQ's sink");
    Ok(())
}

/// Wait up to 3 s for the node `name`. Returns whether it came.
fn wait_for_node(name: &str) -> bool {
    for _ in 0..30 {
        if find_node_id_by_name(name).is_some() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    false
}

/// Link node `from`'s outputs to node `to`'s inputs, channel to channel.
pub(crate) fn link_nodes(from: &str, to: &str) {
    let (Some(out_node), Some(in_node)) = (find_node_id_by_name(from), find_node_id_by_name(to)) else {
        warn!("cannot link {from} to {to}: one is missing");
        return;
    };
    let inputs = native_eq::input_ports(in_node);
    for port in native_eq::output_ports(out_node) {
        let channel = port.rsplit('_').next().unwrap_or("");
        if let Some(input) = inputs.iter().find(|i| i.rsplit('_').next() == Some(channel)) {
            if let Err(err) = native_eq::create_link(&out_node.to_string(), &port, &in_node.to_string(), input) {
                warn!("could not link {from} to {to}: {err}");
            }
        }
    }
}

/// The filter graph for `bands` after `preamp_db`.
pub(crate) fn filter_graph(bands: &[config::EqBand], preamp_db: f64) -> String {
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
    builds_filter_graph_string(&filter_specs)
}

/// The filter-chain module's arguments for an EQ with `bands` and
/// `preamp_db`, taking audio in as the sink `input` and playing it as
/// `output` into the device `target`. `extra` is added to the sink's
/// properties.
pub(crate) fn eq_module_args(
    bands: &[config::EqBand],
    preamp_db: f64,
    input: &str,
    output: &str,
    description: &str,
    target: &str,
    extra: &str,
) -> String {
    let graph = filter_graph(bands, preamp_db);
    let quote = |text: &str| text.replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        "{{ node.description = \"{description}\" media.name = \"{description}\" filter.graph = {graph} capture.props = {{ node.name = \"{input}\" media.class = Audio/Sink audio.channels = 2 audio.position = [ FL FR ] target.object = \"{target}\" {extra} }} playback.props = {{ node.name = \"{output}\" node.passive = true node.autoconnect = false audio.channels = 2 audio.position = [ FL FR ] target.object = \"{target}\" }} }}",
        description = quote(description),
        input = quote(input),
        output = quote(output),
        target = quote(target),
    )
}

/// Load a filter-chain module, natively or through `pw-cli`.
pub(crate) fn load_eq_module(module_args: &str) -> Result<(), String> {
    // Try native PipeWire API first.
    if native_eq::is_available() {
        match native_eq::load_filter_chain(module_args) {
            Ok(id) => {
                info!(
                    "native: loaded filter-chain module (id={})",
                    id
                );
                return Ok(());
            }
            Err(e) => {
                warn!(
                    "native module load failed, falling back to pw-cli: {}",
                    e
                );
            }
        }
    }

    load_with_pw_cli("libpipewire-module-filter-chain", module_args)
}

/// Load a module through `pw-cli`.
fn load_with_pw_cli(module: &str, module_args: &str) -> Result<(), String> {
    if !pw_cli_available() {
        return Err("neither native PipeWire nor pw-cli available".to_string());
    }

    let mut child = Command::new("pw-cli")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| format!("failed to spawn pw-cli: {}", err))?;

    if let Some(mut stdin) = child.stdin.take() {
        let cmd = format!("load-module {} {}\n", module, module_args);
        use std::io::Write;
        stdin.write_all(cmd.as_bytes())
            .map_err(|err| format!("failed to write to pw-cli stdin: {}", err))?;
        stdin.flush()
            .map_err(|err| format!("failed to flush pw-cli stdin: {}", err))?;
    }

    let status = child
        .wait()
        .map_err(|err| format!("failed to wait for pw-cli: {}", err))?;
    if !status.success() {
        return Err(format!("pw-cli exited with {} after loading EQ module", status));
    }

    Ok(())
}

/// Remove the EQ, its stage and its sink.
pub(crate) fn destroy_eq_module() {
    destroy_node_named(config::EQ_STAGE_NODE);
    destroy_node_named(config::EQ_VIRTUAL_INPUT_SINK);

    if let Ok(mut target) = EQ_TARGET_SINK.lock() {
        *target = None;
    }
}

/// Destroy the node named `name`, which takes its filter-chain module down
/// with it.
pub(crate) fn destroy_node_named(name: &str) {
    if let Some(id) = find_node_id_by_name(name) {
        // Try native destroy first.
        if native_eq::is_available() {
            if native_eq::destroy_global(id).is_ok() {
                info!("native: destroyed EQ module (id={})", id);
            } else {
                // Fallback to pw-cli.
                let _ = run_command_capture("pw-cli", &["destroy", &id.to_string()]);
            }
        } else {
            let _ = run_command_capture("pw-cli", &["destroy", &id.to_string()]);
        }
    }
}

pub(crate) fn current_eq_target_sink() -> Option<String> {
    EQ_TARGET_SINK.lock().ok().and_then(|target| target.clone())
}


pub(crate) fn find_node_id_by_name(node_name: &str) -> Option<u32> {
    if native_eq::is_available() {
        if let Some(id) = native_eq::find_node_by_name(node_name) {
            return Some(id);
        }
    }

    if !pw_cli_available() {
        return None;
    }

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
    if native_eq::is_available() {
        let links = find_node_id_by_name(config::EQ_VIRTUAL_OUTPUT_NODE)
            .map(native_eq::links_from_node)
            .unwrap_or_default();
        for link in &links {
            info!(
                "native: purging EQ link id={} ({}:{} -> {}:{})",
                link.id, link.output_node, link.output_port, link.input_node, link.input_port
            );
            let _ = native_eq::destroy_global(link.id);
        }
        if !links.is_empty() {
            return;
        }
    }

    let output = match Command::new("pw-link").arg("-l").output() {
        Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
        Err(_) => return,
    };

    let mut current_source = String::new();
    for line in output.lines() {
        let trimmed = line.trim();
        if (line.starts_with(' ') || line.starts_with('\t')) && !trimmed.is_empty() {
            if (current_source == "effect_output.stratum_eq:output_FL" || current_source == "effect_output.stratum_eq:output_FR") && trimmed.starts_with("|-> ") {
                let target_port = trimmed.replacen("|-> ", "", 1).trim().to_string();
                info!("purging incorrect EQ link: {} -> {}", current_source, target_port);
                let _ = Command::new("pw-link").args(["-d", &current_source, &target_port]).status();
            }
        } else if !trimmed.is_empty() {
            current_source = trimmed.trim_end_matches(':').to_string();
        }
    }
}

fn ensure_eq_output_links(target: &str) {
    if native_eq::is_available() {
        let fl_ok = native_eq::create_link(
            "effect_output.stratum_eq",
            "output_FL",
            target,
            "playback_FL",
        );
        let fr_ok = native_eq::create_link(
            "effect_output.stratum_eq",
            "output_FR",
            target,
            "playback_FR",
        );
        if fl_ok.is_ok() && fr_ok.is_ok() {
            return;
        }
        warn!("native link creation failed, falling back to pw-link");
    }

    let _ = run_command_capture(
        "pw-link",
        &[
            "effect_output.stratum_eq:output_FL",
            &format!("{}:playback_FL", target),
        ],
    );
    let _ = run_command_capture(
        "pw-link",
        &[
            "effect_output.stratum_eq:output_FR",
            &format!("{}:playback_FR", target),
        ],
    );
}

/// Move every app into `target_sink`, except those sent to a device of their
/// own (`routes`), which stay where they were put.
fn move_active_streams_to_eq(target_sink: &str) {
    let pinned = routes::input_sinks();
    let _ = pulse::with_sink(|ctrl| {
        let pinned: Vec<u32> = ctrl
            .list_devices()?
            .into_iter()
            .filter(|d| d.name.as_deref().is_some_and(|n| pinned.iter().any(|p| p == n)))
            .map(|d| d.index)
            .collect();
        for app in ctrl.list_applications()? {
            if !pinned.contains(&app.connection_id) {
                let _ = ctrl.move_app_by_name(app.index, target_sink);
            }
        }
        Ok(())
    });
}

fn map_filter_type_for_pipewire(filter_type: &str) -> &'static str {
    match config::normalized_filter_type(filter_type).as_str() {
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

/// The limits a parametric curve must stay within.
pub(crate) fn eq_capabilities_json() -> Value {
    json!({
        "parametric": {
            "max_bands": config::EQ_MAX_BANDS,
            "gain_range_db": [config::EQ_MIN_GAIN_DB, config::EQ_MAX_GAIN_DB],
            "freq_range_hz": [config::EQ_MIN_FREQ_HZ, config::EQ_MAX_FREQ_HZ],
            "q_range": [config::EQ_MIN_Q, config::EQ_MAX_Q],
        }
    })
}

pub(crate) fn auto_apply_preset_for_device(device_id: &str) -> Value {
    let config = config::load_eq_config();

    let preset_name = config
        .device_last_preset
        .get(device_id)
        .or_else(|| config.device_last_preset.get("@DEFAULT_SINK@"))
        .map(|s| s.as_str())
        .unwrap_or("Flat");

    super::eq_apply_preset(device_id, preset_name)
}

/// Whether `name` is one of the EQs' own nodes, the main one's or a
/// device's (`routes`), rather than a device.
pub(crate) fn is_eq_virtual_sink_name(name: &str) -> bool {
    let trimmed = name.trim();
    [config::EQ_VIRTUAL_INPUT_SINK, config::EQ_VIRTUAL_OUTPUT_NODE, config::EQ_BRIDGE_NODE, config::EQ_STAGE_NODE]
        .contains(&trimmed)
        || routes::is_route_node(trimmed)
}
