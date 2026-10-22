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

/// The connected Wi-Fi device from `device_status()`: `(device, connection,
/// state)`. Only a state that starts with "connected" counts; the QML menu
/// matched "connected" anywhere, which "disconnected" also contains.
fn pick_active(devices: &Value) -> Option<(String, String, String)> {
    devices["devices"].as_array()?.iter().find_map(|row| {
        let field = |k: &str| row[k].as_str().unwrap_or("").trim().to_string();
        let state = field("state");
        (field("type").eq_ignore_ascii_case("wifi") && state.to_ascii_lowercase().starts_with("connected"))
            .then(|| (field("device"), field("connection"), state))
    })
}

/// Names of saved Wi-Fi connections from `known_connections()`.
fn known_wifi(connections: &Value) -> Vec<String> {
    connections["connections"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter(|r| matches!(r["type"].as_str(), Some("802-11-wireless" | "wifi")))
                .filter_map(|r| r["name"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// What the Network page shows beyond the access-point list: the active
/// connection (device, SSID, state, IPv4 address and gateway) and the saved
/// Wi-Fi networks, which connect without asking for a password.
pub fn details() -> Value {
    let active = pick_active(&device_status()).map(|(device, ssid, state)| {
        let info = active_info(&device);
        json!({
            "device": device,
            "ssid": ssid,
            "state": state,
            "ip": info["ip4_address"].as_str().unwrap_or(""),
            "gateway": info["ip4_gateway"].as_str().unwrap_or(""),
        })
    });
    json!({
        "ok": true,
        "active": active,
        "known": known_wifi(&known_connections()),
    })
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

    #[test]
    fn the_active_device_is_a_connected_wifi_one() {
        let devices = json!({ "devices": [
            { "device": "wlan1", "type": "wifi", "state": "disconnected", "connection": "--" },
            { "device": "eth0", "type": "ethernet", "state": "connected", "connection": "Wired" },
            { "device": "wlan0", "type": "wifi", "state": "connected", "connection": "Home" },
        ]});
        assert_eq!(pick_active(&devices), Some(("wlan0".into(), "Home".into(), "connected".into())));
        let none = json!({ "devices": [{ "device": "wlan1", "type": "wifi", "state": "disconnected", "connection": "--" }]});
        assert_eq!(pick_active(&none), None, "\"disconnected\" contains \"connected\"");
    }

    #[test]
    fn known_networks_are_saved_wifi_connections() {
        let connections = json!({ "connections": [
            { "name": "Home", "type": "802-11-wireless" },
            { "name": "Wired", "type": "802-3-ethernet" },
        ]});
        assert_eq!(known_wifi(&connections), vec!["Home".to_string()]);
    }
}
