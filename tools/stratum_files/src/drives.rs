//! Drives from udisks for the sidebar: unmounted filesystems as well as
//! mounted ones, mounted on a click, and unmounted or ejected from a row's
//! menu.
//!
//! Listing drives starts udisksd if it is not running yet (the shell waits
//! for a removable drive before it does). Everything here runs on threads
//! of its own: a mount may wait for a polkit password, and udisks itself
//! may take a moment to start.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use stratum_drives::{Drive, Udisks};
use stratum_ui::app::Waker;

/// How an operation ended, for the window to act on.
pub enum Event {
    /// Mounted, and asked to be opened.
    Mounted(PathBuf),
    /// Unmounted; a window showing a folder in `PathBuf` leaves it.
    Unmounted(PathBuf),
    Ejected(String),
    Failed(String),
    /// The passphrase was not accepted; the dialog stays open.
    UnlockFailed(String),
}

#[derive(Default)]
struct Shared {
    udisks: Option<Udisks>,
    /// The latest list, until the window takes it.
    drives: Option<Vec<Drive>>,
    events: Vec<Event>,
}

pub struct Drives {
    shared: Arc<Mutex<Shared>>,
    waker: Waker,
}

impl Drives {
    /// Connect and watch on a thread of its own.
    pub fn start(waker: Waker) -> Self {
        let shared: Arc<Mutex<Shared>> = Arc::default();
        let drives = Self { shared: Arc::clone(&shared), waker: waker.clone() };
        let spawned = std::thread::Builder::new().name("stratum-udisks".into()).spawn(move || {
            let udisks = match Udisks::connect() {
                Ok(udisks) => udisks,
                Err(err) => {
                    tracing::warn!("drives: no system bus: {err}");
                    return;
                }
            };
            let publish = |list: Vec<Drive>| {
                if let Ok(mut slot) = shared.lock() {
                    slot.drives = Some(list);
                }
                waker.wake();
                true
            };
            match udisks.drives() {
                Ok(list) => {
                    if let Ok(mut slot) = shared.lock() {
                        slot.udisks = Some(udisks.clone());
                    }
                    publish(list);
                }
                Err(err) => {
                    tracing::info!("drives: udisks is unavailable, so only mounted drives show: {err}");
                    return;
                }
            }
            if let Err(err) = udisks.watch(Duration::from_millis(150), publish) {
                tracing::warn!("drives: stopped watching: {err}");
            }
        });
        if let Err(err) = spawned {
            tracing::warn!("drives: failed to start: {err}");
        }
        drives
    }

    /// A new list of drives, if there is one, and the operations that
    /// ended since the last call.
    pub fn take(&self) -> (Option<Vec<Drive>>, Vec<Event>) {
        match self.shared.lock() {
            Ok(mut slot) => (slot.drives.take(), std::mem::take(&mut slot.events)),
            Err(_) => (None, Vec::new()),
        }
    }

    fn run(&self, op: impl FnOnce(&Udisks) -> Result<Event, String> + Send + 'static) {
        let shared = Arc::clone(&self.shared);
        let waker = self.waker.clone();
        let spawned = std::thread::Builder::new().name("stratum-drive-op".into()).spawn(move || {
            let udisks = shared.lock().ok().and_then(|s| s.udisks.clone());
            let event = match udisks {
                Some(udisks) => op(&udisks).unwrap_or_else(Event::Failed),
                None => Event::Failed("udisks is not running".into()),
            };
            if let Ok(mut slot) = shared.lock() {
                slot.events.push(event);
            }
            waker.wake();
        });
        if let Err(err) = spawned {
            tracing::warn!("drives: failed to start an operation: {err}");
        }
    }

    pub fn mount(&self, volume: String, name: String) {
        self.run(move |u| u.mount(&volume).map(Event::Mounted).map_err(|e| format!("Could not mount {name}: {e}")));
    }

    /// Unlock an encrypted volume, then mount the filesystem inside.
    pub fn unlock(&self, volume: String, passphrase: stratum_ui::secret::Secret, name: String) {
        self.run(move |u| {
            let inner = u.unlock(&volume, passphrase.expose()).map_err(|e| format!("Could not unlock {name}: {e}"));
            drop(passphrase);
            match inner {
                Ok(inner) => u.mount(&inner).map(Event::Mounted).map_err(|e| format!("Could not mount {name}: {e}")),
                Err(err) => Ok(Event::UnlockFailed(err)),
            }
        });
    }

    pub fn unmount(&self, volume: String, name: String, at: PathBuf) {
        self.run(move |u| u.unmount(&volume).map(|()| Event::Unmounted(at)).map_err(|e| format!("Could not unmount {name}: {e}")));
    }

    pub fn eject(&self, drive: String, name: String) {
        self.run(move |u| u.eject(&drive).map(|()| Event::Ejected(name.clone())).map_err(|e| format!("Could not eject {name}: {e}")));
    }
}
