use std::env;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde_json::{json, Value};

pub fn emit(msg: &str) {
    println!("{}", msg);
}

pub fn emit_json(value: Value) {
    emit(&value.to_string());
}

pub fn debug(msg: &str) {
    eprintln!("__DEBUG__|{}", msg);
}

pub fn fail(msg: &str) -> ! {
    emit_json(json!({ "ok": false, "error": msg }));
    std::process::exit(0);
}

pub fn is_help_flag(value: &str) -> bool {
    matches!(value, "help" | "-h" | "--help")
}

pub fn emit_help(command: &str, usage: &str, subcommands: &[&str]) {
    let mut lines = Vec::new();
    lines.push(format!("{} help", command));
    lines.push(String::new());
    lines.push("Usage:".to_string());
    lines.push(format!("  {}", usage));

    if !subcommands.is_empty() {
        lines.push(String::new());
        lines.push("Commands:".to_string());
        for subcommand in subcommands {
            lines.push(format!("  - {}", subcommand));
        }
    }

    emit(&lines.join("\n"));
}

pub fn run_command_capture(program: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("failed to run {}: {}", program, e))?;

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

pub fn command_available(program: &str) -> bool {
    match Command::new(program)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(_) => true,
        Err(err) => !matches!(err.kind(), ErrorKind::NotFound),
    }
}

pub fn config_dir() -> PathBuf {
    if let Ok(path) = env::var("XDG_CONFIG_HOME") {
        if !path.trim().is_empty() {
            return PathBuf::from(path).join("stratum");
        }
    }

    let home = env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".config/stratum")
}
