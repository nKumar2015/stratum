mod common;
mod shell_client;

use std::env;

use serde_json::json;
use common::{emit_help, emit_json, fail, is_help_flag};

const ROOT_COMMANDS: [&str; 9] = [
    "dashboard",
    "lockscreen",
    "notifications",
    "powermenu",
    "screenshot",
    "settings",
    "theme",
    "timer",
    "todo",
];

fn print_help() {
    emit_help(
        "stratum-cli",
        "stratum-cli <command|target> [args] | stratum-cli help [command|target]",
        &ROOT_COMMANDS,
    );
}

fn ipc_functions(target: &str) -> Option<&'static [&'static str]> {
    match target {
        "dashboard" => Some(&["open [tab]", "close", "toggle [tab]"]),
        "lockscreen" => Some(&["lock"]),
        "notifications" => Some(&["open", "close", "toggle", "clear", "toggleDnd"]),
        "powermenu" => Some(&["toggle"]),
        "screenshot" => Some(&["start"]),
        "settings" => Some(&["open [page]", "close", "toggle [page]"]),
        "theme" => Some(&["open", "close", "toggle", "set <themeName>"]),
        "todo" => Some(&["add <text>"]),
        "timer" => Some(&["start [25m|90s|1h30m|pomodoro|stopwatch]", "pause", "toggle", "stop"]),
        _ => None,
    }
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

fn run_ipc_call(target: &str, function: &str, args: &[String]) -> Result<(), String> {
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

    // The only call the shell answers `unsupported` is a lock it will not
    // take (no usable PAM service), which is reported as it is.
    match shell_client::shell_call(target, function, args) {
        Ok(response) if response["ok"].as_bool() == Some(true) => Ok(()),
        Ok(response) => {
            let detail = response["error"]
                .as_str()
                .unwrap_or("shell rejected the request");
            Err(format!("shell ipc failed: {}", detail))
        }
        Err(err) => Err(format!("native shell unavailable ({err})")),
    }
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

    run_ipc_call(target, function, args.get(1..).unwrap_or(&[])).unwrap_or_else(|e| fail(&e));

    emit_json(json!({
        "ok": true,
        "command": "ipc",
        "target": target,
        "function": function,
    }));

    true
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
        if !dispatch_ipc_target(&requested, &help_args) {
            fail(&format!("unknown command: {}", requested));
        }
        return;
    }

    if !dispatch_ipc_target(&command, &args) {
        fail(&format!("unknown command: {}", command));
    }
}
