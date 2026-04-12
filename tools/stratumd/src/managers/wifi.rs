use serde_json::Value;

use crate::managers::common::{run_stratum_cli_json, run_stratum_cli_json_owned};

pub fn state() -> Value {
    run_stratum_cli_json(&["wifi", "state"])
}

pub fn device_status() -> Value {
    run_stratum_cli_json(&["wifi", "device-status"])
}

pub fn known_connections() -> Value {
    run_stratum_cli_json(&["wifi", "known-connections"])
}

pub fn list() -> Value {
    run_stratum_cli_json(&["wifi", "list"])
}

pub fn active_info(device: &str) -> Value {
    run_stratum_cli_json(&["wifi", "active-info", device])
}

pub fn connect(ssid: &str, password: Option<&str>) -> Value {
    let mut args = vec!["wifi".to_string(), "connect".to_string(), ssid.to_string()];
    if let Some(pass) = password {
        if !pass.trim().is_empty() {
            args.push(pass.to_string());
        }
    }
    run_stratum_cli_json_owned(&args)
}

pub fn disconnect(device: &str) -> Value {
    run_stratum_cli_json(&["wifi", "disconnect", device])
}

pub fn forget(ssid: &str) -> Value {
    run_stratum_cli_json(&["wifi", "forget", ssid])
}

pub fn toggle(target: &str) -> Value {
    run_stratum_cli_json(&["wifi", "toggle", target])
}
