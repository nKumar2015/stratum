//! Wi-Fi through NetworkManager.
//!
//! These used to shell out to `stratum-cli`, which itself shells out to
//! `nmcli`. Two process hops per query made the UI visibly lag behind a click;
//! talking to `nmcli` directly removes the outer hop and returns in about
//! 20 ms, which is fast enough to refresh on the interaction itself rather than
//! waiting for a poll tick.

use serde_json::{json, Value};

use crate::managers::common::run_command_capture;

/// Run `nmcli` and split its terse (`-t`) output into fields.
fn nmcli(args: &[&str]) -> Result<String, String> {
    run_command_capture("nmcli", args)
}

/// Split one line of `nmcli -t` output, honouring backslash escapes.
fn split_fields(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut escaped = false;
    for ch in line.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == ':' {
            fields.push(std::mem::take(&mut current));
        } else {
            current.push(ch);
        }
    }
    fields.push(current);
    fields
}

/// Radio state: `enabled`, `disabled`, or the raw NetworkManager string.
pub fn state() -> Value {
    let output = match nmcli(&["-t", "-f", "WIFI", "general", "status"]) {
        Ok(output) => output,
        Err(err) => {
            return json!({
                "ok": false,
                "error": err,
                "state": "unknown",
            });
        }
    };
    let state = output.lines().next().unwrap_or("").trim().to_string();
    json!({
        "ok": true,
        "state": state,
        "enabled": state == "enabled",
    })
}

/// Wi-Fi access points, strongest first.
///
/// `rescan` asks NetworkManager to refresh the cache first. That costs a few
/// hundred milliseconds, so callers pass `false` for the frequent refreshes and
/// `true` only when the user explicitly asked to scan.
pub fn list(rescan: bool) -> Value {
    let mut args = vec![
        "-t",
        "-f",
        "IN-USE,SSID,SIGNAL,SECURITY",
        "dev",
        "wifi",
        "list",
    ];
    if rescan {
        args.extend_from_slice(&["--rescan", "yes"]);
    }

    let output = match nmcli(&args) {
        Ok(output) => output,
        Err(err) => {
            return json!({ "ok": false, "error": err, "networks": [] });
        }
    };

    let mut networks = Vec::new();
    for line in output.lines() {
        let fields = split_fields(line);
        if fields.len() < 4 {
            continue;
        }
        let ssid = fields[1].trim().to_string();
        if ssid.is_empty() {
            continue;
        }
        let signal = fields[2].trim().parse::<i64>().unwrap_or(0);
        networks.push(json!({
            "in_use": fields[0].trim() == "*",
            "ssid": ssid,
            "signal": signal,
            "security": fields[3].trim(),
        }));
    }
    networks.sort_by_key(|network| {
        std::cmp::Reverse(network["signal"].as_i64().unwrap_or(0))
    });

    json!({
        "ok": true,
        "networks": networks,
    })
}

/// Active connections, one entry per connected device.
pub fn device_status() -> Value {
    let output = match nmcli(&["-t", "-f", "DEVICE,TYPE,STATE,CONNECTION", "dev", "status"]) {
        Ok(output) => output,
        Err(err) => {
            return json!({ "ok": false, "error": err, "devices": [] });
        }
    };
    let mut devices = Vec::new();
    for line in output.lines() {
        let fields = split_fields(line);
        if fields.len() < 4 {
            continue;
        }
        devices.push(json!({
            "device": fields[0].trim(),
            "type": fields[1].trim(),
            "state": fields[2].trim(),
            "connection": fields[3].trim(),
        }));
    }
    json!({ "ok": true, "devices": devices })
}

pub fn known_connections() -> Value {
    let output = match nmcli(&["-t", "-f", "NAME,TYPE", "connection", "show"]) {
        Ok(output) => output,
        Err(err) => {
            return json!({ "ok": false, "error": err, "connections": [] });
        }
    };
    let mut connections = Vec::new();
    for line in output.lines() {
        let fields = split_fields(line);
        if fields.len() < 2 {
            continue;
        }
        connections.push(json!({
            "name": fields[0].trim(),
            "type": fields[1].trim(),
        }));
    }
    json!({ "ok": true, "connections": connections })
}

pub fn active_info(device: &str) -> Value {
    let output = match nmcli(&["-t", "-f", "IP4.ADDRESS,IP4.GATEWAY", "dev", "show", device]) {
        Ok(output) => output,
        Err(err) => {
            return json!({ "ok": false, "error": err });
        }
    };
    let mut address = String::new();
    let mut gateway = String::new();
    for line in output.lines() {
        let fields = split_fields(line);
        if fields.len() < 2 {
            continue;
        }
        match fields[0].trim() {
            "IP4.ADDRESS[1]" => address = fields[1].trim().to_string(),
            "IP4.GATEWAY" => gateway = fields[1].trim().to_string(),
            _ => {}
        }
    }
    json!({ "ok": true, "ip4_address": address, "ip4_gateway": gateway })
}

pub fn connect(ssid: &str, password: Option<&str>) -> Value {
    let mut args = vec!["dev", "wifi", "connect", ssid];
    if let Some(password) = password.filter(|p| !p.trim().is_empty()) {
        args.extend_from_slice(&["password", password]);
    }
    match nmcli(&args) {
        Ok(message) => json!({ "ok": true, "ssid": ssid, "message": message }),
        Err(err) => json!({ "ok": false, "error": err }),
    }
}

pub fn disconnect(device: &str) -> Value {
    match nmcli(&["dev", "disconnect", device]) {
        Ok(message) => json!({ "ok": true, "device": device, "message": message }),
        Err(err) => json!({ "ok": false, "error": err }),
    }
}

pub fn forget(ssid: &str) -> Value {
    match nmcli(&["connection", "delete", "id", ssid]) {
        Ok(message) => json!({ "ok": true, "ssid": ssid, "message": message }),
        Err(err) => json!({ "ok": false, "error": err }),
    }
}

pub fn toggle(target: &str) -> Value {
    match nmcli(&["radio", "wifi", target]) {
        Ok(message) => json!({ "ok": true, "target": target, "message": message }),
        Err(err) => json!({ "ok": false, "error": err }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_escaped_fields() {
        assert_eq!(split_fields("a:b:c"), vec!["a", "b", "c"]);
        // `nmcli` escapes a literal colon as `\:`.
        assert_eq!(split_fields("a:b\\:c:d"), vec!["a", "b:c", "d"]);
        assert_eq!(split_fields("only"), vec!["only"]);
    }
}
