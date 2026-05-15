use serde_json::{json, Value};
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::Duration;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

pub fn run_command_with_timeout(
    program: &str,
    args: &[&str],
    timeout: Duration,
) -> Result<String, String> {
    let mut child = Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("failed to spawn {}: {}", program, err))?;

    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout = String::new();
                if let Some(mut out) = child.stdout.take() {
                    let _ = out.read_to_string(&mut stdout);
                }
                let mut stderr = String::new();
                if let Some(mut err) = child.stderr.take() {
                    let _ = err.read_to_string(&mut stderr);
                }

                if !status.success() {
                    let detail = if !stderr.trim().is_empty() {
                        stderr.trim().to_string()
                    } else if !stdout.trim().is_empty() {
                        stdout.trim().to_string()
                    } else {
                        format!("exit code {}", status)
                    };
                    return Err(format!("{} failed: {}", program, detail));
                }

                return Ok(stdout.trim().to_string());
            }
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "{} timed out after {}s",
                        program,
                        timeout.as_secs()
                    ));
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(err) => {
                let _ = child.kill();
                return Err(format!("{} wait failed: {}", program, err));
            }
        }
    }
}

pub fn run_command_capture(program: &str, args: &[&str]) -> Result<String, String> {
    run_command_with_timeout(program, args, DEFAULT_TIMEOUT)
}

pub fn run_capture_optional(program: &str, args: &[&str]) -> String {
    run_command_capture(program, args).unwrap_or_default()
}

pub fn command_available(program: &str) -> bool {
    match Command::new(program)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(_) => true,
        Err(err) => !matches!(err.kind(), std::io::ErrorKind::NotFound),
    }
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
