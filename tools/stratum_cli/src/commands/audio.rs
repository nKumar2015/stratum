use std::process::{Command, Stdio};
use serde_json::{json, Value};

use crate::common::{command_available, emit_help, emit_json, fail, is_help_flag};
use crate::daemon_client::daemon_call;

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


fn cmd_set_output(sink: &str) {
    match daemon_call("audio.set_output", json!({"target": sink})) {
        Ok(res) => {
            if let Some(result) = res.get("result") {
                emit_json(result.clone());
            } else {
                fail("failed to set output via daemon");
            }
        }
        Err(err) => fail(&err),
    }
}

fn cmd_equalizer_list_presets(device_id: &str) {
    match daemon_call("audio.eq_list_presets", json!({"device": device_id})) {
        Ok(res) => {
            if let Some(result) = res.get("result") {
                emit_json(result.clone());
            } else {
                fail("failed to list presets via daemon");
            }
        }
        Err(err) => fail(&err),
    }
}

fn cmd_equalizer_apply_preset(device_id: &str, preset_name: &str) {
    match daemon_call("audio.eq_apply_preset", json!({"device": device_id, "preset_name": preset_name})) {
        Ok(res) => {
            if let Some(result) = res.get("result") {
                emit_json(result.clone());
            } else {
                fail("failed to apply preset via daemon");
            }
        }
        Err(err) => fail(&err),
    }
}

fn cmd_equalizer_get_current(_device_id: &str) {
    fail("get-current is deprecated; use list-presets or status");
}

fn cmd_equalizer_capabilities() {
    match daemon_call("audio.eq_list_presets", json!({"device": "@DEFAULT_SINK@"})) {
        Ok(res) => {
            if let Some(result) = res.get("result") {
                if let Some(caps) = result.get("capabilities") {
                    emit_json(caps.clone());
                } else {
                    fail("missing capabilities in daemon response");
                }
            } else {
                fail("failed to get capabilities via daemon");
            }
        }
        Err(err) => fail(&err),
    }
}

fn cmd_equalizer_apply_parametric(device_id: &str, payload_str: &str) {
    let bands = serde_json::from_str::<Value>(payload_str).unwrap_or(Value::Null);
    match daemon_call("audio.eq_apply_parametric", json!({"device": device_id, "bands": bands})) {
        Ok(res) => {
            if let Some(result) = res.get("result") {
                emit_json(result.clone());
            } else {
                fail("failed to apply parametric EQ via daemon");
            }
        }
        Err(err) => fail(&err),
    }
}

fn cmd_equalizer_save_preset(device_id: &str, preset_name: &str, bands_str: &str) {
    let bands: Vec<i32> = bands_str
        .split(',')
        .filter_map(|s| s.trim().parse::<i32>().ok())
        .collect();

    match daemon_call("audio.eq_save_preset_parametric", json!({"device": device_id, "preset_name": preset_name, "bands": bands})) {
        Ok(res) => {
            if let Some(result) = res.get("result") {
                emit_json(result.clone());
            } else {
                fail("failed to save preset via daemon");
            }
        }
        Err(err) => fail(&err),
    }
}

fn cmd_equalizer_save_preset_parametric(device_id: &str, preset_name: &str, payload_str: &str) {
    let payload = serde_json::from_str::<Value>(payload_str).unwrap_or(Value::Null);
    let bands = payload.get("bands").cloned().unwrap_or(Value::Null);
    let preamp_db = payload.get("preamp_db").and_then(|v| v.as_f64()).unwrap_or(0.0);

    match daemon_call("audio.eq_save_preset_parametric", json!({"device": device_id, "preset_name": preset_name, "bands": bands, "preamp_db": preamp_db})) {
        Ok(res) => {
            if let Some(result) = res.get("result") {
                emit_json(result.clone());
            } else {
                fail("failed to save parametric preset via daemon");
            }
        }
        Err(err) => fail(&err),
    }
}

fn cmd_equalizer_delete_preset(device_id: &str, preset_name: &str) {
    match daemon_call("audio.eq_delete_preset", json!({"device": device_id, "preset_name": preset_name})) {
        Ok(res) => {
            if let Some(result) = res.get("result") {
                emit_json(result.clone());
            } else {
                fail("failed to delete preset via daemon");
            }
        }
        Err(err) => fail(&err),
    }
}

fn cmd_media_info() {
    match daemon_call("audio.media_info", json!({})) {
        Ok(res) => {
            if let Some(result) = res.get("result") {
                emit_json(result.clone());
            } else {
                fail("failed to get media info via daemon");
            }
        }
        Err(err) => fail(&err),
    }
}

fn cmd_media_seek(seconds_str: &str) {
    let seconds = seconds_str.parse::<i64>().unwrap_or(0);
    match daemon_call("audio.media_seek", json!({"position_sec": seconds})) {
        Ok(res) => {
            if let Some(result) = res.get("result") {
                emit_json(result.clone());
            } else {
                fail("failed to seek via daemon");
            }
        }
        Err(err) => fail(&err),
    }
}

fn cmd_media_seek_relative(offset_str: &str) {
    let offset = offset_str.parse::<i64>().unwrap_or(0);
    match daemon_call("audio.media_seek", json!({"offset_sec": offset})) {
        Ok(res) => {
            if let Some(result) = res.get("result") {
                emit_json(result.clone());
            } else {
                fail("failed to seek relative via daemon");
            }
        }
        Err(err) => fail(&err),
    }
}

fn has_hover_flag(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "--hover")
}
fn cmd_status(hover: bool) {
    if !hover {
        match daemon_call("audio.status", json!({})) {
            Ok(response) => {
                if let Some(result) = response.get("result") {
                    if let Some(audio) = result.get("audio") {
                        emit_json(json!({
                            "ok": true,
                            "command": "audio",
                            "subcommand": "status",
                            "hover": false,
                            "volume": audio.get("volume").and_then(|v| v.as_str()).unwrap_or("0%"),
                            "mute": audio.get("mute").and_then(|v| v.as_str()).unwrap_or("yes"),
                            "headphones": audio.get("headphones").and_then(|v| v.as_str()).unwrap_or("no"),
                        }));
                        return;
                    }
                }
            }
            Err(_) => {}
        }
    } else {
        match daemon_call("audio.devices", json!({})) {
            Ok(response) => {
                if let Some(result) = response.get("result") {
                    if let Some(audio) = result.get("audio") {
                        emit_json(audio.clone());
                        return;
                    }
                }
            }
            Err(_) => {}
        }
    }

    fail("failed to get audio status from daemon");
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
            cmd_set_output(sink);
        }
        "set-input" => {
            let source = args.get(1).map(String::as_str).unwrap_or("");
            match daemon_call("audio.set_input", json!({"target": source})) {
                Ok(res) => {
                    if let Some(result) = res.get("result") {
                        emit_json(result.clone());
                    } else {
                        fail("failed to set input via daemon");
                    }
                }
                Err(err) => fail(&err),
            }
        }
        "set-volume" => {
            let volume_arg = args.get(1).map(String::as_str).unwrap_or("");
            let volume = volume_arg.parse::<i64>().unwrap_or(0);
            match daemon_call("audio.set_volume", json!({"percent": volume})) {
                Ok(res) => {
                    if let Some(result) = res.get("result") {
                        emit_json(result.clone());
                    } else {
                        fail("failed to set volume via daemon");
                    }
                }
                Err(err) => fail(&err),
            }
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
