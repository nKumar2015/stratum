//! `stratum_sys` — the system integration layer behind Stratum.
//!
//! Every subsystem the shell needs to observe or control lives here: audio
//! (PulseAudio/PipeWire, including the native parametric EQ), Bluetooth,
//! Wi-Fi/NetworkManager, battery, media (MPRIS), dashboard metrics (CPU,
//! memory, disks, GPUs), weather, calendar feeds, and the polkit
//! authentication agent.
//!
//! The crate is deliberately host-agnostic. It does not own an event loop, a
//! socket, or a specific UI. Managers that need to report changes call a
//! [`Publisher`] supplied by whoever embeds them: the native shell.
//!
//! ```no_run
//! use stratum_sys::{managers, Publisher};
//!
//! let publisher = Publisher::new(|channel, payload| {
//!     println!("{}: {}", channel, payload);
//! });
//! managers::audio::spawn_monitor(publisher);
//! ```

use serde_json::Value;
use std::sync::Arc;

pub mod managers;

/// Callback a host provides so managers can publish state changes.
type PublishFn = dyn Fn(&str, Value) + Send + Sync;

/// Handle managers use to report state changes to their host.
///
/// Cloneable and cheap to pass into worker threads. The host decides what to do
/// with each snapshot: update in-memory state, push it to a UI, or both.
#[derive(Clone)]
pub struct Publisher(Arc<PublishFn>);

impl Publisher {
    pub fn new<F>(f: F) -> Self
    where
        F: Fn(&str, Value) + Send + Sync + 'static,
    {
        Self(Arc::new(f))
    }

    /// Publish a snapshot on `channel` (`"audio"`, `"polkit"`).
    pub fn emit(&self, channel: &str, payload: Value) {
        (self.0)(channel, payload);
    }

    pub fn audio(&self, payload: Value) {
        self.emit("audio", payload);
    }

    pub fn polkit(&self, payload: Value) {
        self.emit("polkit", payload);
    }
}

/// The identity of the account the shell runs as, used by the polkit agent
/// when answering authorization requests.
pub(crate) fn current_uid() -> u32 {
    // SAFETY: `getuid` is always successful and has no preconditions.
    unsafe { libc::getuid() }
}
