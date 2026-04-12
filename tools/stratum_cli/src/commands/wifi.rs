use serde_json::json;

use crate::common::{command_available, emit_help, emit_json, fail, is_help_flag, run_command_capture};

fn print_help() {
    emit_help(
        "wifi",
        "stratum-cli wifi <subcommand> [args]",
        &[
            "state [--hover]",
            "device-status",
            "known-connections",
            "list",
            "active-info <device>",
            "connect <ssid> [password]",
            "disconnect <device>",
            "forget <ssid>",
            "toggle <on|off>",
        ],
    );
}

fn nmcli_capture(args: &[&str]) -> String {
    run_command_capture("nmcli", args).unwrap_or_else(|e| fail(&e))
}

fn split_nmcli_fields(line: &str, expected_fields: usize) -> Vec<String> {
    if expected_fields == 0 {
        return vec![line.to_string()];
    }

    let mut fields = Vec::new();
    let mut current = String::new();
    let mut escaped = false;

    for ch in line.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }

        if ch == '\\' {
            escaped = true;
            continue;
        }

        if ch == ':' && fields.len() < expected_fields - 1 {
            fields.push(current);
            current = String::new();
            continue;
        }

        current.push(ch);
    }

    fields.push(current);
    fields
}

fn first_nmcli_value(output: &str, strip_cidr: bool) -> String {
    for line in output.lines() {
        let cols = split_nmcli_fields(line, 2);
        if cols.len() < 2 {
            continue;
        }

        let trimmed = cols[1].trim();
        if strip_cidr {
            return trimmed.split('/').next().unwrap_or("").trim().to_string();
        }
        return trimmed.to_string();
    }

    String::new()
}

fn connected_wifi_signal(device: &str) -> String {
    let output = nmcli_capture(&[
        "-t",
        "-f",
        "IN-USE,SSID,SIGNAL",
        "dev",
        "wifi",
        "list",
        "ifname",
        device,
    ]);

    for line in output.lines() {
        if !line.starts_with('*') {
            continue;
        }
        let cols: Vec<&str> = line.splitn(3, ':').collect();
        if cols.len() >= 3 {
            return cols[2].trim().to_string();
        }
    }

    String::new()
}

fn has_hover_flag(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "--hover")
}

fn cmd_state(hover: bool) {
    if !hover {
        let output = nmcli_capture(&["-t", "-f", "WIFI", "general", "status"]);
        let state = output.lines().next().unwrap_or("").trim();
        emit_json(json!({
            "ok": true,
            "command": "wifi",
            "subcommand": "state",
            "hover": false,
            "state": state,
        }));
        return;
    }

    if let Ok(response) = crate::daemon_client::daemon_call("net.status", serde_json::json!({})) {
        if let Some(result) = response.get("result") {
            if let Some(net_json) = result.get("net") {
                if let Some(connections) = net_json.get("connections") {
                    emit_json(json!({
                        "ok": true,
                        "command": "wifi",
                        "subcommand": "state",
                        "hover": true,
                        "connections": connections,
                    }));
                    return;
                }
            }
        }
    }

    let device_rows = nmcli_capture(&["-t", "-f", "DEVICE,TYPE,STATE,CONNECTION", "dev"]);
    let mut connections = Vec::new();

    for line in device_rows.lines() {
        let cols = split_nmcli_fields(line, 4);
        if cols.len() < 4 {
            continue;
        }

        let device = cols[0].trim();
        let dev_type = cols[1].trim();
        let state = cols[2].trim();
        let connection = cols[3].trim();

        if state != "connected" {
            continue;
        }

        let ip = first_nmcli_value(
            &nmcli_capture(&["-t", "-f", "IP4.ADDRESS", "dev", "show", device]),
            true,
        );
        let gateway = first_nmcli_value(
            &nmcli_capture(&["-t", "-f", "IP4.GATEWAY", "dev", "show", device]),
            false,
        );

        if dev_type == "ethernet" || dev_type == "bridge" {
            connections.push(json!({
                "type": "ethernet",
                "device": device,
                "connection": connection,
                "signal": serde_json::Value::Null,
                "ip_address": ip,
                "gateway": gateway,
            }));
        } else if dev_type == "wifi" {
            let signal = connected_wifi_signal(device);
            connections.push(json!({
                "type": "wifi",
                "device": device,
                "connection": connection,
                "signal": signal,
                "ip_address": ip,
                "gateway": gateway,
            }));
        }
    }

    emit_json(json!({
        "ok": true,
        "command": "wifi",
        "subcommand": "state",
        "hover": true,
        "connections": connections,
    }));
}

pub fn handle(args: &[String]) {
    if !command_available("nmcli") {
        fail("nmcli not found");
    }

    let command = args.first().map(String::as_str).unwrap_or("");
    if is_help_flag(command) {
        print_help();
        return;
    }

    match command {
        "state" => cmd_state(has_hover_flag(args.get(1..).unwrap_or(&[]))),
        "device-status" => {
            let output = nmcli_capture(&["-t", "-f", "DEVICE,TYPE,STATE,CONNECTION", "dev", "status"]);
            let mut devices = Vec::new();
            for line in output.lines() {
                let cols = split_nmcli_fields(line, 4);
                if cols.len() < 4 {
                    continue;
                }
                devices.push(json!({
                    "device": cols[0].trim(),
                    "type": cols[1].trim(),
                    "state": cols[2].trim(),
                    "connection": cols[3].trim(),
                }));
            }
            emit_json(json!({
                "ok": true,
                "command": "wifi",
                "subcommand": "device-status",
                "devices": devices,
            }));
        }
        "known-connections" => {
            let output = nmcli_capture(&["-t", "-f", "NAME,TYPE", "connection", "show"]);
            let mut connections = Vec::new();
            for line in output.lines() {
                let cols = split_nmcli_fields(line, 2);
                if cols.len() < 2 {
                    continue;
                }
                connections.push(json!({
                    "name": cols[0].trim(),
                    "type": cols[1].trim(),
                }));
            }
            emit_json(json!({
                "ok": true,
                "command": "wifi",
                "subcommand": "known-connections",
                "connections": connections,
            }));
        }
        "list" => {
            let output = nmcli_capture(&[
                "-t",
                "-f",
                "IN-USE,SSID,SIGNAL,SECURITY",
                "dev",
                "wifi",
                "list",
                "--rescan",
                "auto",
            ]);
            let mut networks = Vec::new();
            for line in output.lines() {
                let cols = split_nmcli_fields(line, 4);
                if cols.len() < 4 {
                    continue;
                }
                let signal = cols[2].trim().parse::<i64>().unwrap_or(0);
                networks.push(json!({
                    "in_use": cols[0].trim(),
                    "ssid": cols[1].trim(),
                    "signal": signal,
                    "security": cols[3].trim(),
                }));
            }
            emit_json(json!({
                "ok": true,
                "command": "wifi",
                "subcommand": "list",
                "networks": networks,
            }));
        }
        "active-info" => {
            let device = args.get(1).map(String::as_str).unwrap_or("");
            let output = nmcli_capture(&["-t", "-f", "IP4.ADDRESS,IP4.GATEWAY", "dev", "show", device]);

            let mut ip4_address = String::new();
            let mut ip4_gateway = String::new();
            for line in output.lines() {
                let cols = split_nmcli_fields(line, 2);
                if cols.len() < 2 {
                    continue;
                }
                let key = cols[0].trim();
                let value = cols[1].trim();
                if key == "IP4.ADDRESS[1]" {
                    ip4_address = value.to_string();
                } else if key == "IP4.GATEWAY" {
                    ip4_gateway = value.to_string();
                }
            }

            emit_json(json!({
                "ok": true,
                "command": "wifi",
                "subcommand": "active-info",
                "ip4_address": ip4_address,
                "ip4_gateway": ip4_gateway,
            }));
        }
        "connect" => {
            let ssid = args.get(1).map(String::as_str).unwrap_or("");
            let password = args.get(2).map(String::as_str).unwrap_or("");
            let message = if !password.is_empty() {
                run_command_capture("nmcli", &["dev", "wifi", "connect", ssid, "password", password])
            } else {
                run_command_capture("nmcli", &["dev", "wifi", "connect", ssid])
            }
            .unwrap_or_else(|e| fail(&e));

            emit_json(json!({
                "ok": true,
                "command": "wifi",
                "subcommand": "connect",
                "ssid": ssid,
                "message": message,
            }));
        }
        "disconnect" => {
            let device = args.get(1).map(String::as_str).unwrap_or("");
            let message =
                run_command_capture("nmcli", &["dev", "disconnect", device]).unwrap_or_else(|e| fail(&e));
            emit_json(json!({
                "ok": true,
                "command": "wifi",
                "subcommand": "disconnect",
                "device": device,
                "message": message,
            }));
        }
        "forget" => {
            let ssid = args.get(1).map(String::as_str).unwrap_or("");
            let message =
                run_command_capture("nmcli", &["connection", "delete", "id", ssid]).unwrap_or_else(|e| fail(&e));
            emit_json(json!({
                "ok": true,
                "command": "wifi",
                "subcommand": "forget",
                "ssid": ssid,
                "message": message,
            }));
        }
        "toggle" => {
            let target = args.get(1).map(String::as_str).unwrap_or("");
            let message =
                run_command_capture("nmcli", &["radio", "wifi", target]).unwrap_or_else(|e| fail(&e));
            emit_json(json!({
                "ok": true,
                "command": "wifi",
                "subcommand": "toggle",
                "target": target,
                "message": message,
            }));
        }
        _ => fail("unknown wifi command"),
    }
}
