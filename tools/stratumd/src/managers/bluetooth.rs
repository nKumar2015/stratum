use serde_json::Value;

use crate::managers::common::run_stratum_cli_json;

pub fn status() -> Value {
    run_stratum_cli_json(&["bluetooth", "check"])
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
