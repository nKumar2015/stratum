//! Wayland plumbing for the shell.
//!
//! SCTK handles the protocol details; this module owns our application state:
//! the globals we bind, the layer surfaces we create, and the mapping from
//! surface to renderer. M1 creates one anchored layer surface per output.

mod surfaces;
mod state;

pub use state::App;

use std::ptr::NonNull;
use std::time::Duration;

use tracing::{error, info};

/// Run the shell until the compositor closes the connection.
pub fn run() -> Result<(), String> {
    let (mut app, mut event_loop, _conn) = App::new()?;

    // A bounded timeout keeps the loop responsive to redraw requests without
    // spinning: with no work the loop blocks here instead of burning CPU.
    event_loop
        .run(Duration::from_secs(1), &mut app, |app| {
            if let Err(err) = app.render_pending() {
                // Presentation failures are usually transient (resize, disabled
                // output). Log and keep the loop alive so a monitor hotplug
                // does not take the shell down.
                error!("render failed: {err}");
            }
        })
        .map_err(|err| format!("event loop failed: {err}"))?;

    Ok(())
}

/// Raw `wl_display` pointer for the Vulkan surface handoff.
pub(crate) fn display_ptr(
    conn: &wayland_client::Connection,
) -> Result<NonNull<std::ffi::c_void>, String> {
    let ptr = conn.backend().display_ptr();
    let non_null = NonNull::new(ptr.cast::<std::ffi::c_void>())
        .ok_or_else(|| "wl_display pointer was null".to_string())?;
    info!("wayland display acquired");
    Ok(non_null)
}
