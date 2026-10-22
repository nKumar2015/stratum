mod commands;
mod common;
mod daemon_client;

use std::env;
use std::process::Command;

use serde_json::{json, Value};
use commands::{audio, battery, bluetooth, dashboard, net, notifications_snapshot, osd, portal_save_file, screenshot_menu, screenshot_post, theme, wifi};
use common::{emit_help, emit_json, fail, is_help_flag, run_command_capture};

const ROOT_COMMANDS: [&str; 17] = [
    "audio",
    "battery",
    "bluetooth",
    "dashboard",
    "lockscreen",
    "net",
    "notifications",
    "notifications-snapshot",
    "osd",
    "portal-save-file",
    "powermenu",
    "screenshot",
    "screenshot-menu",
    "screenshot-post",
    "settings",
    "theme",
    "wifi",
];

fn print_help() {
    emit_help(
        "stratum-cli",
        "stratum-cli <command|target> [args] | stratum-cli help [command|target]",
        &ROOT_COMMANDS,
    );
}

fn print_dashboard_help() {
    emit_help(
        "dashboard",
        "stratum-cli dashboard <all|calendar|music|performance|open|close|toggle> [args]",
        &[
            "all [year month]",
            "calendar [year month]",
            "music",
            "performance",
            "open",
            "close",
            "toggle",
        ],
    );
}

fn print_theme_help() {
    emit_help(
        "theme",
        "stratum-cli theme <list|open|close|toggle|set> [args]",
        &[
            "list",
            "open",
            "close",
            "toggle",
            "set <themeName>",
        ],
    );
}

fn ipc_functions(target: &str) -> Option<&'static [&'static str]> {
    match target {
        "dashboard" => Some(&["open", "close", "toggle"]),
        "lockscreen" => Some(&["lock"]),
        "notifications" => Some(&["open", "close", "toggle", "clear", "toggleDnd"]),
        "powermenu" => Some(&["toggle"]),
        "screenshot" => Some(&["start"]),
        "settings" => Some(&["open [page]", "close", "toggle [page]"]),
        "theme" => Some(&["open", "close", "toggle", "set"]),
        _ => None,
    }
}

/// Targets only the native shell serves. Quickshell has no equivalent, so a
/// failed call is reported as is instead of being retried through `qs ipc`.
fn native_only(target: &str) -> bool {
    target == "settings"
}

/// Whether `function` is one of `functions`, ignoring the `[args]` hints that
/// some entries carry for help output.
fn has_function(functions: &[&str], function: &str) -> bool {
    functions
        .iter()
        .any(|entry| entry.split_whitespace().next() == Some(function))
}

fn print_ipc_target_help(target: &str) {
    let Some(functions) = ipc_functions(target) else {
        fail(&format!("unknown target: {}", target));
    };

    let usage = format!("stratum-cli {} <function> [args]", target);
    emit_help(target, &usage, functions);
}

fn newest_quickshell_pid() -> Result<i64, String> {
    let list_raw = run_command_capture("qs", &["list", "--all", "--json"])?;
    let parsed: Value =
        serde_json::from_str(&list_raw).map_err(|e| format!("failed to parse qs list output: {}", e))?;
    let list = parsed
        .as_array()
        .ok_or_else(|| "qs list returned non-array json".to_string())?;

    let mut selected: Option<(String, i64)> = None;

    for entry in list {
        let Some(obj) = entry.as_object() else {
            continue;
        };

        let pid = obj
            .get("pid")
            .and_then(|v| {
                v.as_i64()
                    .or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok()))
            })
            .unwrap_or(-1);

        if pid <= 0 {
            continue;
        }

        let launch_time = obj
            .get("launch_time")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        match &selected {
            None => selected = Some((launch_time, pid)),
            Some((best_launch, best_pid)) => {
                if launch_time > *best_launch || (launch_time == *best_launch && pid > *best_pid) {
                    selected = Some((launch_time, pid));
                }
            }
        }
    }

    selected
        .map(|(_, pid)| pid)
        .ok_or_else(|| "no running quickshell instances found".to_string())
}

fn run_ipc_call(target: &str, function: &str, args: &[String]) -> Result<i64, String> {
    let Some(functions) = ipc_functions(target) else {
        return Err(format!("unknown target: {}", target));
    };

    if !has_function(functions, function) {
        return Err(format!(
            "unknown {} function: {} (expected one of: {})",
            target,
            function,
            functions.join(", ")
        ));
    }

    // Prefer the native shell. It reports `unsupported` for surfaces that have
    // not been ported yet, which is the cue to fall through to Quickshell while
    // both shells run side by side. A missing socket means the native shell is
    // not running at all.
    match daemon_client::shell_call(target, function, args) {
        Ok(response) if response["ok"].as_bool() == Some(true) => {
            let pid = response["pid"].as_i64().unwrap_or(std::process::id() as i64);
            Ok(pid)
        }
        Ok(response)
            if response["code"].as_str() == Some("unsupported") && !native_only(target) =>
        {
            run_ipc_call_quickshell(target, function, args)
        }
        Ok(response) => {
            let detail = response["error"]
                .as_str()
                .unwrap_or("shell rejected the request");
            Err(format!("shell ipc failed: {}", detail))
        }
        Err(err) if native_only(target) => Err(format!("native shell unavailable ({err})")),
        Err(err) => {
            // No native shell listening (missing socket, or a stale file left
            // by a previous run). Fall back to Quickshell; if that is not
            // running either, report the native error as the more useful one.
            run_ipc_call_quickshell(target, function, args)
                .map_err(|fallback| format!("native shell unavailable ({err}); {fallback}"))
        }
    }
}

/// The pre-native path: ask the newest Quickshell instance.
fn run_ipc_call_quickshell(target: &str, function: &str, args: &[String]) -> Result<i64, String> {
    let pid = newest_quickshell_pid().map_err(|err| {
        format!("no native shell IPC socket and no running Quickshell instance ({err})")
    })?;
    let pid_text = pid.to_string();

    let mut command_args = vec![
        "ipc".to_string(),
        "--pid".to_string(),
        pid_text,
        "call".to_string(),
        target.to_string(),
        function.to_string(),
    ];
    command_args.extend(args.iter().cloned());

    let output = Command::new("qs")
        .args(&command_args)
        .output()
        .map_err(|e| format!("failed to run qs ipc: {}", e))?;

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

        return Err(format!("qs ipc failed: {}", detail));
    }

    Ok(pid)
}

fn dispatch_ipc_target(target: &str, args: &[String]) -> bool {
    if ipc_functions(target).is_none() {
        return false;
    }

    let function = args.first().map(String::as_str).unwrap_or("");
    if function.is_empty() || is_help_flag(function) {
        print_ipc_target_help(target);
        return true;
    }

    let pid = run_ipc_call(target, function, args.get(1..).unwrap_or(&[])).unwrap_or_else(|e| fail(&e));

    emit_json(json!({
        "ok": true,
        "command": "ipc",
        "target": target,
        "function": function,
        "pid": pid,
    }));

    true
}

fn dispatch(command: &str, args: &[String]) -> bool {
    if command == "dashboard" {
        let first = args.first().map(String::as_str).unwrap_or("");

        if first.is_empty() {
            dashboard::handle(args);
            return true;
        }

        if is_help_flag(first) {
            print_dashboard_help();
            return true;
        }

        if matches!(first, "open" | "close" | "toggle") {
            return dispatch_ipc_target("dashboard", args);
        }

        dashboard::handle(args);
        return true;
    }

    if command == "theme" {
        let first = args.first().map(String::as_str).unwrap_or("");

        if first.is_empty() {
            print_theme_help();
            return true;
        }

        if is_help_flag(first) {
            print_theme_help();
            return true;
        }

        if matches!(first, "list") {
            theme::handle(args);
            return true;
        }

        if matches!(first, "open" | "close" | "toggle" | "set") {
            return dispatch_ipc_target("theme", args);
        }

        print_theme_help();
        return true;
    }

    if dispatch_ipc_target(command, args) {
        return true;
    }

    match command {
        "audio" => {
            audio::handle(args);
            true
        }
        "battery" => {
            battery::handle(args);
            true
        }
        "bluetooth" => {
            bluetooth::handle(args);
            true
        }
        "net" => {
            net::handle(args);
            true
        }
        "notifications-snapshot" => {
            notifications_snapshot::handle(args);
            true
        }
        "osd" => {
            osd::handle(args);
            true
        }
        "wifi" => {
            wifi::handle(args);
            true
        }
        "portal-save-file" => {
            portal_save_file::handle(args);
            true
        }
        "screenshot-menu" => {
            screenshot_menu::handle(args);
            true
        }
        "screenshot-post" => {
            screenshot_post::handle(args);
            true
        }
        _ => false,
    }
}

fn main() {
    let mut args = env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() {
        print_help();
        return;
    }

    let command = args.remove(0);
    if is_help_flag(&command) {
        if args.is_empty() {
            print_help();
            return;
        }

        let requested = args.remove(0);
        let help_args = vec!["--help".to_string()];
        if !dispatch(&requested, &help_args) {
            fail(&format!("unknown command: {}", requested));
        }
        return;
    }

    if !dispatch(&command, &args) {
        fail(&format!("unknown command: {}", command));
    }
}
