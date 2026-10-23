mod commands;
mod common;
mod daemon_client;

use std::env;

use serde_json::json;
use commands::{audio, battery, bluetooth, dashboard, net, notifications_snapshot, osd, portal_save_file, screenshot_menu, screenshot_post, theme, wifi};
use common::{emit_help, emit_json, fail, is_help_flag};

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

    // The native shell is the only shell. Quickshell used to take the calls it
    // answered `unsupported`; now the only such call is a lock the shell will
    // not take (no usable PAM service), which is reported as it is.
    match daemon_client::shell_call(target, function, args) {
        Ok(response) if response["ok"].as_bool() == Some(true) => {
            let pid = response["pid"].as_i64().unwrap_or(std::process::id() as i64);
            Ok(pid)
        }
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
