use serde_json::{json, Value};

pub fn emit(msg: &str) {
    println!("{}", msg);
}

pub fn emit_json(value: Value) {
    emit(&value.to_string());
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
