//! Stratum's native desktop shell.
//!
//! It replaced the Quickshell/QML shell. One process owns the
//! Wayland connection, renders with wgpu on Vulkan, and hosts the system
//! managers (`stratum_sys`) directly.

mod actions;
mod config;
mod dashboard;
mod desktop;
mod hyprland;
mod ipc;
mod notification_store;
mod notifications;
mod lock;
mod memory;
mod osd;
mod pam;
mod paths;
mod polkit_dialog;
mod screenshot;
mod screenshot_viewer;
mod theme_choice;
mod theme_switcher;
mod power_menu;
mod toasts;
mod tray;
mod tray_menu;
mod icons;
mod render;
mod sys_state;
mod text;
mod ui;
mod wayland;

use std::process::ExitCode;

use tracing::{error, info};

fn main() -> ExitCode {
    // `Targets` reads the same `target=level` directives as `EnvFilter`
    // without its span and regex matching, which cost ~250 KB of binary.
    use tracing_subscriber::prelude::*;
    let filter = std::env::var("RUST_LOG")
        .ok()
        .and_then(|value| value.parse::<tracing_subscriber::filter::Targets>().ok())
        .unwrap_or_else(|| {
            tracing_subscriber::filter::Targets::new()
                .with_target("stratum_shell", tracing::Level::INFO)
                .with_target("wgpu", tracing::Level::WARN)
                .with_target("naga", tracing::Level::WARN)
        });
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(filter)
        .init();

    info!("stratum-shell starting");
    drop_missing_temp_dirs();
    memory::tune_allocator();

    match wayland::run() {
        Ok(()) => {
            info!("stratum-shell exited");
            ExitCode::SUCCESS
        }
        Err(err) => {
            error!("stratum-shell failed: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Unset a temp directory variable that names a directory which is gone,
/// so the programs the shell runs fall back to `/tmp`. A `nix develop`
/// environment imported into the systemd user manager leaves `TMPDIR` at
/// a deleted `/tmp/nix-shell-*`, and `wl-copy` then fails (exit 1) without
/// saying why. Called before any thread starts.
fn drop_missing_temp_dirs() {
    for name in ["TMPDIR", "TMP", "TEMP", "TEMPDIR"] {
        if let Some(dir) = std::env::var_os(name).filter(|dir| !std::path::Path::new(dir).is_dir()) {
            tracing::warn!("{name} names a missing directory ({}); unsetting it", dir.to_string_lossy());
            std::env::remove_var(name);
        }
    }
}
