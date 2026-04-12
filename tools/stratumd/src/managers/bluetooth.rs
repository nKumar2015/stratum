use serde_json::{json, Value};
use std::time::Duration;

use crate::managers::common::{run_program_combined, run_stratum_cli_json};

const BT_TIMEOUT: Duration = Duration::from_secs(4);

fn parse_bool_field(info: &str, key: &str, default: &str) -> String {
    for line in info.lines() {
        if let Some(value) = line.trim().strip_prefix(key) {
            return value.trim().to_lowercase();
        }
    }
    default.to_string()
}

pub fn status() -> Value {
    let (_ok, show_output) = run_program_combined("bluetoothctl", &["show"], None, BT_TIMEOUT);
    let powered = parse_bool_field(&show_output, "Powered:", "no");
    if powered != "yes" {
        return json!({
            "ok": true,
            "state": "off",
        });
    }

    let (_ok_connected, connected_output) =
        run_program_combined("bluetoothctl", &["devices", "Connected"], None, BT_TIMEOUT);
    let has_connected = connected_output
        .lines()
        .any(|line| !line.trim().is_empty());

    let scanning = parse_bool_field(&show_output, "Discovering:", "no");

    json!({
        "ok": true,
        "state": if has_connected { "connected" } else { "on" },
        "scanning": scanning,
    })
}

pub fn state() -> Value {
    run_stratum_cli_json(&["bluetooth", "state"])
}

pub fn list() -> Value {
    run_stratum_cli_json(&["bluetooth", "list"])
}

pub fn pair(mac: &str) -> Value {
    run_stratum_cli_json(&["bluetooth", "pair", mac])
}

pub fn connect(mac: &str) -> Value {
    run_stratum_cli_json(&["bluetooth", "connect", mac])
}

pub fn disconnect(mac: &str) -> Value {
    run_stratum_cli_json(&["bluetooth", "disconnect", mac])
}

pub fn forget(mac: &str) -> Value {
    run_stratum_cli_json(&["bluetooth", "forget", mac])
}

pub fn trust(mac: &str) -> Value {
    run_stratum_cli_json(&["bluetooth", "trust", mac])
}

pub fn untrust(mac: &str) -> Value {
    run_stratum_cli_json(&["bluetooth", "untrust", mac])
}

pub fn power(target: &str) -> Value {
    run_stratum_cli_json(&["bluetooth", "power", target])
}

pub fn scan() -> Value {
    run_stratum_cli_json(&["bluetooth", "scan"])
}
