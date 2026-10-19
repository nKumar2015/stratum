//! The shell's in-process state.
//!
//! This is the single-process replacement for the daemon's `AppState`: the
//! managers in `stratum_sys` publish into these cells and widgets read from
//! them. There is no IPC between the UI and system state.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use stratum_sys::Publisher;

/// One observable value, with a change flag the renderer can consult.
///
/// Widgets read `get()` while painting; the shell's render loop asks
/// `take_dirty()` to decide whether a redraw is needed.
pub struct Cell<T> {
    value: Mutex<T>,
    dirty: AtomicBool,
}

impl<T: Clone> Cell<T> {
    pub fn new(value: T) -> Self {
        Self {
            value: Mutex::new(value),
            dirty: AtomicBool::new(true),
        }
    }

    pub fn get(&self) -> T {
        self.value
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_else(|poisoned| poisoned.into_inner().clone())
    }

    pub fn set(&self, value: T) {
        if let Ok(mut guard) = self.value.lock() {
            *guard = value;
            self.dirty.store(true, Ordering::Release);
        }
    }

    /// True once per change.
    pub fn take_dirty(&self) -> bool {
        self.dirty.swap(false, Ordering::AcqRel)
    }
}

/// Everything the sidebar needs.
pub struct SystemState {
    pub audio: Cell<serde_json::Value>,
    pub net: Cell<serde_json::Value>,
    /// Radio state from `wifi.state`; `net` only lists connections.
    pub wifi: Cell<serde_json::Value>,
    pub bluetooth: Cell<serde_json::Value>,
    pub battery: Cell<serde_json::Value>,
    pub music: Cell<serde_json::Value>,
    pub dashboard: Cell<serde_json::Value>,
    pub polkit: Cell<serde_json::Value>,
    /// Compositor state (workspaces, active window), refreshed by the event
    /// watcher rather than polled.
    pub hypr: Cell<crate::hyprland::HyprState>,
    /// System tray items, refreshed by the tray watcher.
    pub tray: Cell<Vec<crate::tray::TrayItem>>,
    /// Backlight percentage, polled because there is no push source.
    pub brightness: Cell<i64>,
    /// Live notifications, newest last, published by the notification server.
    pub notifications: Cell<Vec<crate::notifications::Notification>>,
}

impl SystemState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            audio: Cell::new(serde_json::Value::Null),
            net: Cell::new(serde_json::Value::Null),
            wifi: Cell::new(serde_json::Value::Null),
            bluetooth: Cell::new(serde_json::Value::Null),
            battery: Cell::new(serde_json::Value::Null),
            music: Cell::new(serde_json::Value::Null),
            dashboard: Cell::new(serde_json::Value::Null),
            polkit: Cell::new(serde_json::Value::Null),
            hypr: Cell::new(crate::hyprland::HyprState::default()),
            tray: Cell::new(Vec::new()),
            brightness: Cell::new(0),
            notifications: Cell::new(Vec::new()),
        })
    }

    /// True when any domain changed since the last check.
    pub fn take_dirty(&self) -> bool {
        self.audio.take_dirty()
            | self.net.take_dirty()
            | self.wifi.take_dirty()
            | self.bluetooth.take_dirty()
            | self.battery.take_dirty()
            | self.music.take_dirty()
            | self.dashboard.take_dirty()
            | self.polkit.take_dirty()
            | self.hypr.take_dirty()
            | self.notifications.take_dirty()
    }

    /// A publisher that feeds every manager snapshot into this state.
    pub fn publisher(state: &Arc<Self>) -> Publisher {
        let state = Arc::clone(state);
        Publisher::new(move |channel, payload| match channel {
            "audio" => state.audio.set(payload),
            "net" => state.net.set(payload),
            "wifi" => state.wifi.set(payload),
            "bluetooth" => state.bluetooth.set(payload),
            "battery" => state.battery.set(payload),
            "music" => state.music.set(payload),
            "dashboard" => state.dashboard.set(payload),
            "polkit" => state.polkit.set(payload),
            other => tracing::warn!("unhandled state channel: {other}"),
        })
    }
}

impl<T: Clone + Default> Default for Cell<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_reports_one_dirty_edge_per_change() {
        let cell = Cell::new(1u32);
        assert!(cell.take_dirty(), "starts dirty");
        assert!(!cell.take_dirty(), "cleared after first read");
        cell.set(2);
        assert!(cell.take_dirty());
        assert_eq!(cell.get(), 2);
    }

    #[test]
    fn publisher_routes_by_channel() {
        let state = SystemState::new();
        let publisher = SystemState::publisher(&state);
        publisher.emit("battery", serde_json::json!({ "percent": 42 }));
        assert_eq!(state.battery.get()["percent"], 42);
        assert!(state.take_dirty());
    }
}
