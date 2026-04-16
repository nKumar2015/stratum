use std::env;
use std::fs;
use std::path::PathBuf;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

pub fn init() {
    let state_dir = if let Ok(xdg_state) = env::var("XDG_STATE_HOME") {
        PathBuf::from(xdg_state).join("stratum")
    } else if let Ok(home) = env::var("HOME") {
        PathBuf::from(home)
            .join(".local")
            .join("state")
            .join("stratum")
    } else {
        PathBuf::from("/tmp/stratum")
    };

    if let Err(e) = fs::create_dir_all(&state_dir) {
        eprintln!(
            "Failed to create log directory {}: {}",
            state_dir.display(),
            e
        );
        return;
    }

    // Daily rotation: stratumd.log.2024-04-14 etc.
    let file_appender = tracing_appender::rolling::daily(&state_dir, "stratumd.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    // We need to keep the guard alive for the duration of the program.
    // However, since we're using a global subscriber, we often justLeak it or
    // use a static if we really need to shut down cleanly. For a daemon,
    // leaks are often acceptable for the logger guard at the global level.
    // Better: return it to main.

    // Subscriber with file and stderr output
    let subscriber = tracing_subscriber::registry()
        .with(EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()))
        .with(fmt::layer().with_writer(std::io::stderr))
        .with(fmt::layer().with_writer(non_blocking).with_ansi(false));

    tracing::subscriber::set_global_default(subscriber).expect("Failed to set tracing subscriber");

    Box::leak(Box::new(_guard));

    tracing::info!(
        "Logging initialized. Target directory: {}",
        state_dir.display()
    );
}

pub fn get_shell_log_path() -> PathBuf {
    let state_dir = if let Ok(xdg_state) = env::var("XDG_STATE_HOME") {
        PathBuf::from(xdg_state).join("stratum")
    } else if let Ok(home) = env::var("HOME") {
        PathBuf::from(home)
            .join(".local")
            .join("state")
            .join("stratum")
    } else {
        PathBuf::from("/tmp/stratum")
    };
    state_dir.join("shell.log")
}
