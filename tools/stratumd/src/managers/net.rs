use serde_json::Value;

use crate::managers::common::run_stratum_cli_json;

pub fn status() -> Value {
    run_stratum_cli_json(&["net", "check"])
}
