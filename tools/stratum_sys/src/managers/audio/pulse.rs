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

use pulsectl::controllers::types::DeviceInfo;
use pulsectl::controllers::{DeviceControl, SinkController, SourceController};
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

/// The default device of a controller's kind, and its mute.
///
/// pulsectl 0.3.2 gets both wrong for sources: `get_default_device` looks the
/// default *sink*'s name up among the sources, so it fails for every source,
/// and `set_device_mute_by_index` mutes the *sink* with that index (PipeWire
/// gives a monitor source its sink's id, so that muted the speakers).
pub(crate) trait DefaultDevice: DeviceControl<DeviceInfo> {
    fn default_device(&mut self) -> Result<DeviceInfo, ControllerError>;
    fn set_mute(&mut self, index: u32, muted: bool);
}

impl DefaultDevice for SinkController {
    fn default_device(&mut self) -> Result<DeviceInfo, ControllerError> {
        self.get_default_device()
    }

    fn set_mute(&mut self, index: u32, muted: bool) {
        self.set_device_mute_by_index(index, muted);
    }
}

impl DefaultDevice for SourceController {
    fn default_device(&mut self) -> Result<DeviceInfo, ControllerError> {
        let name = self
            .get_server_info()?
            .default_source_name
            .ok_or_else(|| ControllerError::GetInfo("no default source".to_string()))?;
        self.get_device_by_name(&name)
    }

    fn set_mute(&mut self, index: u32, muted: bool) {
        let op = self.handler.introspect.set_source_mute_by_index(index, muted, None);
        self.handler.wait_for_operation(op).ok();
    }
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
