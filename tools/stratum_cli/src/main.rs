mod commands;
mod common;

use std::env;

use commands::{audio, battery, bluetooth, dashboard, net, notifications_snapshot, osd, portal_save_file, screenshot_menu, screenshot_post, wifi};
use common::{emit_help, fail, is_help_flag};

const ROOT_COMMANDS: [&str; 11] = [
    "audio",
    "battery",
    "bluetooth",
    "dashboard",
    "net",
    "notifications-snapshot",
    "osd",
    "wifi",
    "portal-save-file",
    "screenshot-menu",
    "screenshot-post",
];

fn print_help() {
    emit_help(
        "stratum-cli",
        "stratum-cli <command> [args] | stratum-cli help [command]",
        &ROOT_COMMANDS,
    );
}

fn dispatch(command: &str, args: &[String]) -> bool {
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
        "dashboard" => {
            dashboard::handle(args);
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
