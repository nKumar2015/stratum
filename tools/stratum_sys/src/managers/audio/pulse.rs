//! One PulseAudio connection per thread, reused by every call on it.
//!
//! Opening a connection per call made a single slider step open five. The
//! connection cannot be shared between threads (pulsectl's `Handler` holds
//! `Rc`s), so each thread that touches audio (the action worker, the monitor)
//! keeps its own. Sink and source controllers are thin wrappers around the
//! same `Handler`, so one connection serves both.
//!
//! pulsectl drives a non-threaded mainloop only while a call runs, so an idle
//! connection costs nothing between calls.

use std::cell::RefCell;

use pulsectl::controllers::{SinkController, SourceController};
use pulsectl::{ControllerError, Handler};

thread_local! {
    static CONNECTION: RefCell<Option<Handler>> = const { RefCell::new(None) };
}

/// Run `f` against the default sink controller on this thread's connection.
pub(crate) fn with_sink<T>(
    mut f: impl FnMut(&mut SinkController) -> Result<T, ControllerError>,
) -> Result<T, ControllerError> {
    with_handler(|handler| {
        let mut ctrl = SinkController { handler };
        let result = f(&mut ctrl);
        (ctrl.handler, result)
    })
}

/// Run `f` against the source controller on this thread's connection.
pub(crate) fn with_source<T>(
    mut f: impl FnMut(&mut SourceController) -> Result<T, ControllerError>,
) -> Result<T, ControllerError> {
    with_handler(|handler| {
        let mut ctrl = SourceController { handler };
        let result = f(&mut ctrl);
        (ctrl.handler, result)
    })
}

/// The server's default sink name, as it reports it (possibly the EQ sink).
pub(crate) fn default_sink_name() -> Option<String> {
    with_sink(|ctrl| ctrl.get_server_info())
        .ok()
        .and_then(|info| info.default_sink_name)
}

fn with_handler<T>(
    mut f: impl FnMut(Handler) -> (Handler, Result<T, ControllerError>),
) -> Result<T, ControllerError> {
    // Taken out while in use, so a nested call opens a connection of its own
    // instead of panicking on the borrow.
    let cached = CONNECTION.with(|slot| slot.borrow_mut().take());
    let (handler, result) = match cached.map(&mut f) {
        Some((handler, Ok(value))) => (handler, Ok(value)),
        // A cached connection goes stale when the server restarts, and then
        // every call on it fails; retry once on a fresh one. Every caller's
        // calls are idempotent (absolute volumes, named defaults), so running
        // one twice is harmless.
        Some((_, Err(_))) | None => f(Handler::connect("stratum")?),
    };
    // A failed call may have left the connection broken, so only a
    // successful one is kept.
    if result.is_ok() {
        CONNECTION.with(|slot| *slot.borrow_mut() = Some(handler));
    }
    result
}
