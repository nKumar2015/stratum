use crate::common::emit_help;

pub fn handle(args: &[String]) {
    let command = args.first().map(String::as_str).unwrap_or("");

    match command {
        "list" => list_themes(),
        "" => print_help(),
        "help" => print_help(),
        "--help" | "-h" => print_help(),
        _ => print_help(),
    }
}

fn print_help() {
    emit_help(
        "theme",
        "stratum-cli theme <subcommand|function>",
        &[
            "list",
            "open",
            "close",
            "toggle",
            "set <themeName>",
        ],
    );
}

fn list_themes() {
    let themes = ["Carbon", "Gruvbox", "RosePine", "TokyoNight"];
    println!("Available themes:");
    for (i, theme) in themes.iter().enumerate() {
        println!("  {}. {}", i + 1, theme);
    }
}
