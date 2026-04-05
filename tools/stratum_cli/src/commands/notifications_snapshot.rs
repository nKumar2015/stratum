use std::env;
use std::fs;
use std::path::PathBuf;

use crate::common::{emit, emit_help, fail, is_help_flag};

fn print_help() {
    emit_help(
        "notifications-snapshot",
        "stratum-cli notifications-snapshot <snapshot-save|snapshot-load> [value]",
        &["snapshot-save <urlencoded_json>", "snapshot-load"],
    );
}

fn state_dir() -> PathBuf {
    if let Ok(path) = env::var("XDG_STATE_HOME") {
        if !path.trim().is_empty() {
            return PathBuf::from(path).join("quickshell");
        }
    }

    let home = env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".local/state/quickshell")
}

fn snapshot_file() -> PathBuf {
    state_dir().join("notifications-snapshot.json")
}

fn ensure_state_dir() {
    let dir = state_dir();
    fs::create_dir_all(&dir).unwrap_or_else(|e| fail(&format!("failed to create state dir: {}", e)));

    let file = snapshot_file();
    if !file.exists() {
        fs::write(&file, "").unwrap_or_else(|e| fail(&format!("failed to create snapshot file: {}", e)));
    }
}

fn decode_urlencoded(input: &str) -> String {
    let plus_as_space = input.replace('+', " ");
    match urlencoding::decode(&plus_as_space) {
        Ok(value) => value.into_owned(),
        Err(_) => String::new(),
    }
}

fn cmd_snapshot_save(encoded: &str) {
    ensure_state_dir();

    if encoded.is_empty() {
        return;
    }

    let decoded = decode_urlencoded(encoded);
    if decoded.is_empty() {
        return;
    }

    let file = snapshot_file();
    fs::write(file, format!("{}\n", decoded))
        .unwrap_or_else(|e| fail(&format!("failed to write snapshot: {}", e)));
}

fn cmd_snapshot_load() {
    ensure_state_dir();

    let file = snapshot_file();
    let content = fs::read_to_string(file).unwrap_or_default();
    if content.trim().is_empty() {
        return;
    }

    emit(content.trim_end());
}

pub fn handle(args: &[String]) {
    let subcommand = args.first().map(String::as_str).unwrap_or("");
    if is_help_flag(subcommand) {
        print_help();
        return;
    }

    match subcommand {
        "snapshot-save" => {
            let encoded = args.get(1).map(String::as_str).unwrap_or("");
            cmd_snapshot_save(encoded);
        }
        "snapshot-load" => cmd_snapshot_load(),
        _ => fail("usage: stratum-cli notifications-snapshot <snapshot-save|snapshot-load> [value]"),
    }
}
