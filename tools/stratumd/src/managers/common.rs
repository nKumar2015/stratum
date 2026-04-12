use serde_json::{json, Value};
use std::process::Command;

pub fn run_command_capture(program: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|err| format!("failed to run {}: {}", program, err))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("exit code {}", output.status)
        };
        return Err(format!("{} failed: {}", program, detail));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn run_stratum_cli_json(args: &[&str]) -> Value {
    let mut cmd = vec!["stratum-cli"];
    cmd.extend_from_slice(args);

    let out = match run_command_capture(cmd[0], &cmd[1..]) {
        Ok(output) => output,
        Err(err) => {
            return json!({
                "ok": false,
                "error": err,
            });
        }
    };

    match serde_json::from_str::<Value>(&out) {
        Ok(v) => v,
        Err(err) => json!({
            "ok": false,
            "error": format!("invalid stratum-cli JSON: {}", err),
            "raw": out,
        }),
    }
}

pub fn run_stratum_cli_json_owned(args: &[String]) -> Value {
    let mut cmd: Vec<String> = vec!["stratum-cli".to_string()];
    cmd.extend(args.iter().cloned());

    let argv: Vec<&str> = cmd.iter().map(String::as_str).collect();
    let out = match run_command_capture(argv[0], &argv[1..]) {
        Ok(output) => output,
        Err(err) => {
            return json!({
                "ok": false,
                "error": err,
            });
        }
    };

    match serde_json::from_str::<Value>(&out) {
        Ok(v) => v,
        Err(err) => json!({
            "ok": false,
            "error": format!("invalid stratum-cli JSON: {}", err),
            "raw": out,
        }),
    }
}
