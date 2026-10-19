//! Stratum's native desktop shell.
//!
//! This is the replacement for the Quickshell/QML implementation. It owns the
//! Wayland connection, renders with wgpu on Vulkan, and — from a later
//! milestone — hosts the system managers directly instead of talking to
//! `stratumd` over a socket.
//!
//! M1 scope: bring up a single layer-shell surface per output and prove the
//! Vulkan surface handoff works. The window is a flat background plus one
//! rounded rectangle; no widgets or state yet.

#[allow(dead_code)]
#[cfg(test)]
mod glyph_probe;
#[allow(dead_code)]
mod actions;
mod config;
mod hyprland;
mod notifications;
mod osd;
mod toasts;
#[allow(dead_code)]
mod icons;
mod render;
mod sys_state;
mod text;
mod ui;
mod wayland;

use std::process::ExitCode;

use tracing::{error, info};

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "stratum_shell=info,wgpu=warn,naga=warn".into()),
        )
        .init();

    info!("stratum-shell starting");

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
