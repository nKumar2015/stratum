use std::process::{Command, Stdio};

use serde_json::json;

use crate::common::{command_available, emit_help, emit_json, fail, is_help_flag, run_command_capture};

fn print_help() {
    emit_help(
        "audio",
        "stratum-cli audio <status|set-output|set-input|set-volume|open-control> [args]",
        &[
            "status [--hover]",
            "set-output <sink>",
            "set-input <source>",
            "set-volume <0-150>",
            "open-control",
        ],
    );
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

fn parse_pactl_device_rows(output: &str, block_prefix: &str, skip_monitor_sources: bool) -> Vec<AudioDeviceRow> {
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

fn has_hover_flag(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "--hover")
}

fn cmd_status(hover: bool) {
    let volume = run_command_capture("pactl", &["get-sink-volume", "@DEFAULT_SINK@"])
        .ok()
        .and_then(|out| extract_first_percent(&out))
        .unwrap_or_else(|| "0%".to_string());
    let mute = run_command_capture("pactl", &["get-sink-mute", "@DEFAULT_SINK@"])
        .ok()
        .and_then(|out| extract_mute_state(&out))
        .unwrap_or_else(|| "yes".to_string());

    if !hover {
        let default_sink = run_command_capture("pactl", &["get-default-sink"]).unwrap_or_default();
        let headphones = is_headphone_default_sink(&default_sink);
        emit_json(json!({
            "ok": true,
            "command": "audio",
            "subcommand": "status",
            "hover": false,
            "volume": volume,
            "mute": mute,
            "headphones": headphones,
        }));
        return;
    }

    let default_sink = run_command_capture("pactl", &["get-default-sink"]).unwrap_or_default();
    let default_source = run_command_capture("pactl", &["get-default-source"]).unwrap_or_default();

    let sink_rows = run_command_capture("pactl", &["list", "sinks"])
        .map(|out| parse_pactl_device_rows(&out, "Sink #", false))
        .unwrap_or_default();
    let sinks = sink_rows
        .iter()
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
        .map(|row| {
            json!({
                "name": row.name,
                "description": row.description,
            })
        })
        .collect::<Vec<_>>();

    emit_json(json!({
        "ok": true,
        "command": "audio",
        "subcommand": "status",
        "hover": true,
        "status": {
            "volume": volume,
            "mute": mute,
        },
        "default": {
            "sink": default_sink,
            "source": default_source,
        },
        "sinks": sinks,
        "sources": sources,
    }));
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
            let result =
                run_command_capture("pactl", &["set-default-sink", sink]).unwrap_or_else(|e| fail(&e));
            emit_json(json!({
                "ok": true,
                "command": "audio",
                "subcommand": "set-output",
                "message": result,
            }));
        }
        "set-input" => {
            let source = args.get(1).map(String::as_str).unwrap_or("");
            let result =
                run_command_capture("pactl", &["set-default-source", source]).unwrap_or_else(|e| fail(&e));
            emit_json(json!({
                "ok": true,
                "command": "audio",
                "subcommand": "set-input",
                "message": result,
            }));
        }
        "set-volume" => {
            let volume_arg = args.get(1).map(String::as_str).unwrap_or("");
            if volume_arg.is_empty() || !volume_arg.bytes().all(|byte| byte.is_ascii_digit()) {
                fail("invalid volume");
            }

            let mut volume = volume_arg.parse::<i32>().unwrap_or(0);
            volume = volume.clamp(0, 150);
            let volume_text = format!("{}%", volume);
            let result = run_command_capture("pactl", &["set-sink-volume", "@DEFAULT_SINK@", &volume_text])
                .unwrap_or_else(|e| fail(&e));
            emit_json(json!({
                "ok": true,
                "command": "audio",
                "subcommand": "set-volume",
                "volume": volume,
                "message": result,
            }));
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
        _ => fail("unknown audio command"),
    }
}
