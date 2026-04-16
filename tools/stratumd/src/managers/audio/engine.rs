use super::{config, parser};

use crate::managers::common::{command_available, run_command_capture};
use lazy_static::lazy_static;
use serde_json::{json, Value};
use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

#[derive(Clone, Debug)]
pub(crate) struct EqApplyResult {
    pub(crate) applied: bool,
    pub(crate) _dry_run: bool,
    pub(crate) engine: String,
    pub(crate) _resolved_device: String,
    pub(crate) status: String,
}

lazy_static! {
    static ref EQ_PROCESS: Mutex<Option<Child>> = Mutex::new(None);
    static ref GLOBAL_EQ_LOCK: Mutex<()> = Mutex::new(());
    pub(crate) static ref DEVICE_CACHE: Mutex<Option<Value>> = Mutex::new(None);
}

pub(crate) fn apply_eq_bands(
    device_id: &str,
    bands: &[config::EqBand],
    preamp_db: f64,
) -> Result<EqApplyResult, String> {
    config::validate_parametric_bands(bands)?;

    if command_available("pw-cli") {
        return apply_eq_bands_pipewire(device_id, bands, preamp_db);
    }

    Err("no supported audio processing engine found (requires pw-cli)".to_string())
}

fn apply_eq_bands_pipewire(
    device_id: &str,
    bands: &[config::EqBand],
    preamp_db: f64,
) -> Result<EqApplyResult, String> {
    if !command_available("pw-cli") {
        return Err("pw-cli not found - EQ not applied".to_string());
    }

    let resolved_device = if device_id == "@DEFAULT_SINK@" {
        let def = run_command_capture("pactl", &["get-default-sink"])
            .unwrap_or_else(|_| device_id.to_string());
        parser::resolve_effective_default_sink(def.trim())
    } else {
        device_id.to_string()
    };

    let mut filter_specs = Vec::new();
    if preamp_db.abs() > 0.01 {
        filter_specs.push(format!(
            "{{ type = bq_peaking freq = 1000.0 gain = {:.4} q = 0.707 }}",
            preamp_db
        ));
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

    let _lifecycle_guard = GLOBAL_EQ_LOCK.lock().unwrap();
    destroy_eq_module();
    std::thread::sleep(std::time::Duration::from_millis(150));

    let graph = builds_filter_graph_string(&filter_specs);

    if is_eq_virtual_sink_name(&resolved_device) || resolved_device.is_empty() {
        println!(
            "[audio] [warn] resolved_device for EQ is invalid ('{}'), aborting load",
            resolved_device
        );
        return Err("invalid hardware target for EQ".to_string());
    }

    println!(
        "[audio] [info] loading EQ module targeting device: {}",
        resolved_device
    );

    let module_args = format!(
        "{{ node.description = \"Stratum Parametric EQ\" media.name = \"Stratum Parametric EQ\" filter.graph = {} capture.props = {{ node.name = \"effect_input.stratum_eq\" media.class = Audio/Sink audio.channels = 2 audio.position = [ FL FR ] target.object = \"{}\" }} playback.props = {{ node.name = \"effect_output.stratum_eq\" node.passive = true node.autoconnect = false audio.channels = 2 audio.position = [ FL FR ] target.object = \"{}\" }} }}",
        graph,
        resolved_device.replace('\\', "\\\\").replace('"', "\\\""),
        resolved_device.replace('\\', "\\\\").replace('"', "\\\"")
    );

    spawn_eq_module(&module_args)
        .map_err(|err| format!("failed to spawn persistent EQ process ($pw-cli): {}", err))?;

    for _ in 0..30 {
        if find_node_id_by_name(config::EQ_VIRTUAL_INPUT_SINK).is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    let current_default = run_command_capture("pactl", &["get-default-sink"]).unwrap_or_default();
    if current_default.trim() != config::EQ_VIRTUAL_INPUT_SINK {
        let _ = run_command_capture(
            "pactl",
            &["set-default-sink", config::EQ_VIRTUAL_INPUT_SINK],
        );
    }
    move_active_streams_to_eq(config::EQ_VIRTUAL_INPUT_SINK);

    let target = resolved_device.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(400));
        purge_eq_output_links();
        println!("[audio] [info] ensuring manual link: EQ -> {}", target);
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
    });

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

fn spawn_eq_module(module_args: &str) -> std::io::Result<()> {
    destroy_eq_module();

    let mut child = Command::new("pw-cli")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        let cmd = format!(
            "load-module libpipewire-module-filter-chain {}\n",
            module_args
        );
        stdin.write_all(cmd.as_bytes())?;
        stdin.flush()?;
    }

    let mut lock = EQ_PROCESS.lock().unwrap();
    *lock = Some(child);

    Ok(())
}

pub(crate) fn destroy_eq_module() {
    let mut lock = EQ_PROCESS.lock().unwrap();
    if let Some(mut child) = lock.take() {
        let _ = child.kill();
        let _ = child.wait();
    }

    if let Some(id) = find_node_id_by_name(config::EQ_VIRTUAL_INPUT_SINK) {
        let _ = run_command_capture("pw-cli", &["destroy", &id.to_string()]);
    }
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
            if (current_source == "effect_output.stratum_eq:output_FL"
                || current_source == "effect_output.stratum_eq:output_FR")
                && trimmed.starts_with("|->")
            {
                let target_port = trimmed.replacen("|->", "", 1).trim().to_string();
                println!(
                    "[audio] [info] purging incorrect EQ link: {} -> {}",
                    current_source, target_port
                );
                let _ = Command::new("pw-link")
                    .args(["-d", &current_source, &target_port])
                    .status();
            }
        } else if !trimmed.is_empty() {
            current_source = trimmed.trim_end_matches(':').to_string();
        }
    }
}

fn move_active_streams_to_eq(target_sink: &str) {
    let Ok(output) = run_command_capture("pactl", &["list", "short", "sink-inputs"]) else {
        return;
    };

    for line in output.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if !parts.is_empty() {
            let id = parts[0];
            let _ = Command::new("pactl")
                .args(["move-sink-input", id, target_sink])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}

fn builds_filter_graph_string(filter_specs: &[String]) -> String {
    format!(
        "{{ nodes = [ {{ type = builtin name = eq label = param_eq config = {{ filters = [ {} ] }} }} ] inputs = [ \"eq:In 1\" \"eq:In 2\" ] outputs = [ \"eq:Out 1\" \"eq:Out 2\" ] }}",
        filter_specs.join(" ")
    )
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

pub(crate) fn eq_capabilities_json() -> Value {
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
            "max_bands": config::EQ_MAX_BANDS,
            "gain_range_db": [config::EQ_MIN_GAIN_DB, config::EQ_MAX_GAIN_DB],
            "freq_range_hz": [config::EQ_MIN_FREQ_HZ, config::EQ_MAX_FREQ_HZ],
            "q_range": [config::EQ_MIN_Q, config::EQ_MAX_Q],
            "supported_filter_types": config::EQ_SUPPORTED_FILTER_TYPES,
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

pub(crate) fn is_eq_virtual_sink_name(name: &str) -> bool {
    let trimmed = name.trim();
    trimmed == config::EQ_VIRTUAL_INPUT_SINK || trimmed == config::EQ_VIRTUAL_OUTPUT_NODE
}
