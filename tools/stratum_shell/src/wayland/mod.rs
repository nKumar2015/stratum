//! Wayland plumbing for the shell.
//!
//! SCTK handles the protocol details; this module owns our application state:
//! the globals we bind, the layer surfaces we create, and the mapping from
//! surface to renderer.

mod lock_surface;
mod surfaces;
mod state;
mod viewer_window;

pub use state::App;

use std::ptr::NonNull;
use tracing::{error, info};

/// Run the shell until the compositor closes the connection.
pub fn run() -> Result<(), String> {
    let (mut app, mut event_loop, conn) = App::new()?;

    // Dispatch manually rather than with `EventLoop::run`, because the idle
    // timeout has to change per frame: animations need ~60 Hz, while a steady
    // shell should block for a second at a time to keep idle CPU near zero.
    let outcome = loop {
        let timeout = app.frame_timeout();
        if let Err(err) = event_loop.dispatch(timeout, &mut app) {
            // A Wayland connection that has gone away is a normal exit; report
            // anything else.
            break Err(format!("event loop failed: {err}"));
        }
        if let Err(err) = app.render_pending() {
            // Presentation failures are usually transient (resize, disabled
            // output). Log and keep the loop alive so a monitor hotplug does
            // not take the shell down.
            error!("render failed: {err}");
        }
        if app.should_exit {
            break Ok(());
        }
    };

    // Drop order is load-bearing. Every surface owns a Vulkan swapchain built
    // from a raw `wl_surface`, and destroying it calls back into libwayland.
    // If the `Connection` were still the last binding to fall out of scope it
    // would be dropped *before* the app, so the swapchains would tear down
    // against a closed display and segfault inside the driver.
    drop(app);
    drop(event_loop);
    drop(conn);

    outcome
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
