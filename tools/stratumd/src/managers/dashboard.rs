use serde_json::Value;

use crate::managers::common::run_stratum_cli_json;

pub fn status(year: i32, month: i32) -> Value {
    let year_s = year.to_string();
    let month_s = month.to_string();
    run_stratum_cli_json(&["dashboard", "all", &year_s, &month_s])
}
