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

impl<T: Clone + PartialEq> Cell<T> {
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

    /// Store `value`, reporting whether it actually changed.
    ///
    /// Publishers fire on every underlying event, which for PipeWire can be
    /// many times a second. Treating an identical snapshot as a change would
    /// repaint every surface each time for no reason, so equality is checked
    /// before the dirty flag is set.
    pub fn set(&self, value: T) -> bool {
        let Ok(mut guard) = self.value.lock() else {
            return false;
        };
        if *guard == value {
            return false;
        }
        *guard = value;
        self.dirty.store(true, Ordering::Release);
        true
    }

    /// True once per change.
    pub fn take_dirty(&self) -> bool {
        self.dirty.swap(false, Ordering::AcqRel)
    }
}

/// The outcome of a tracked action: its ticket, and the error if it failed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActionResult {
    pub ticket: u64,
    pub error: Option<String>,
}

/// Everything the sidebar needs.
pub struct SystemState {
    pub audio: Cell<serde_json::Value>,
    pub net: Cell<serde_json::Value>,
    /// Radio state from `wifi.state`; `net` only lists connections.
    pub wifi: Cell<serde_json::Value>,
    /// Visible access points, strongest first. Refreshed on demand and by the
    /// poller while the radio is enabled.
    pub wifi_networks: Cell<serde_json::Value>,
    /// The active Wi-Fi connection and the saved networks
    /// (`wifi::details`). Refreshed by Network page actions only: it costs
    /// several `nmcli` runs, too many for the poller.
    pub wifi_details: Cell<serde_json::Value>,
    pub bluetooth: Cell<serde_json::Value>,
    pub battery: Cell<serde_json::Value>,
    pub music: Cell<serde_json::Value>,
    /// Resource samples and their history, published by the dashboard's
    /// sampler while the dashboard is open.
    pub dashboard: Cell<crate::dashboard::feed::Snapshot>,
    /// Tells that sampler whether the dashboard is open and how to sample.
    pub dashboard_feed: Arc<crate::dashboard::feed::Feed>,
    /// The forecast, fetched while the dashboard is open.
    pub weather: Cell<crate::dashboard::weather::WeatherState>,
    pub weather_feed: Arc<crate::dashboard::weather::WeatherFeed>,
    pub polkit: Cell<serde_json::Value>,
    /// The equalizer for the current output: its presets, the active one, the
    /// backend's limits, and the result of the last EQ action
    /// (`{ ..eq_list_presets, "status", "ok" }`). Written by the action worker.
    pub eq: Cell<serde_json::Value>,
    /// Compositor state (workspaces, active window), refreshed by the event
    /// watcher rather than polled.
    pub hypr: Cell<crate::hyprland::HyprState>,
    /// System tray items, refreshed by the tray watcher.
    pub tray: Cell<Vec<crate::tray::TrayItem>>,
    /// The open tray context menu, if any. The shell fetches it on right-click
    /// and publishes it here so the overlay can paint it.
    pub tray_menu: Cell<Option<crate::tray::TrayMenu>>,
    /// Backlight percentage, polled because there is no push source. `None`
    /// until the first reading, so the OSD never takes a placeholder as its
    /// baseline.
    pub brightness: Cell<Option<i64>>,
    /// Live notifications, newest last, published by the notification server.
    pub notifications: Cell<Vec<crate::notifications::Notification>>,
    /// Do-not-disturb: notifications still collect, but no toasts appear.
    pub dnd: Cell<bool>,
    /// How the last tracked action ended (`SystemAction::Tracked`), so the
    /// page that issued it can report the outcome.
    pub action_result: Cell<ActionResult>,
    /// Called after any snapshot changes, so a sleeping event loop wakes up and
    /// repaints instead of waiting for its poll timeout.
    waker: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    /// The running tray, so the action worker can call it. Set once at startup
    /// when the tray is enabled.
    tray_handle: Mutex<Option<Arc<crate::tray::Tray>>>,
}

impl SystemState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            audio: Cell::new(serde_json::Value::Null),
            net: Cell::new(serde_json::Value::Null),
            wifi: Cell::new(serde_json::Value::Null),
            wifi_networks: Cell::new(serde_json::Value::Null),
            wifi_details: Cell::new(serde_json::Value::Null),
            bluetooth: Cell::new(serde_json::Value::Null),
            battery: Cell::new(serde_json::Value::Null),
            music: Cell::new(serde_json::Value::Null),
            dashboard: Cell::default(),
            dashboard_feed: Arc::default(),
            weather: Cell::default(),
            weather_feed: Arc::default(),
            polkit: Cell::new(serde_json::Value::Null),
            eq: Cell::new(serde_json::Value::Null),
            hypr: Cell::new(crate::hyprland::HyprState::default()),
            tray: Cell::new(Vec::new()),
            tray_menu: Cell::new(None),
            brightness: Cell::new(None),
            notifications: Cell::new(Vec::new()),
            dnd: Cell::new(false),
            action_result: Cell::new(ActionResult::default()),
            waker: Mutex::new(None),
            tray_handle: Mutex::new(None),
        })
    }

    /// Install the running tray so actions can reach it.
    pub fn set_tray_handle(&self, tray: Arc<crate::tray::Tray>) {
        if let Ok(mut slot) = self.tray_handle.lock() {
            *slot = Some(tray);
        }
    }

    pub fn tray_handle(&self) -> Option<Arc<crate::tray::Tray>> {
        self.tray_handle.lock().ok().and_then(|slot| slot.clone())
    }

    /// Install the callback that pokes the event loop. Set once at startup.
    pub fn set_waker(&self, waker: Arc<dyn Fn() + Send + Sync>) {
        if let Ok(mut slot) = self.waker.lock() {
            *slot = Some(waker);
        }
    }

    /// Wake the UI thread if anything is waiting on state.
    pub fn notify(&self) {
        let waker = self.waker.lock().ok().and_then(|slot| slot.clone());
        if let Some(waker) = waker {
            waker();
        }
    }

    /// True when any domain changed since the last check.
    pub fn take_dirty(&self) -> bool {
        self.audio.take_dirty()
            | self.net.take_dirty()
            | self.wifi.take_dirty()
            | self.wifi_networks.take_dirty()
            | self.wifi_details.take_dirty()
            | self.bluetooth.take_dirty()
            | self.battery.take_dirty()
            | self.music.take_dirty()
            | self.dashboard.take_dirty()
            | self.weather.take_dirty()
            | self.polkit.take_dirty()
            | self.eq.take_dirty()
            | self.hypr.take_dirty()
            | self.tray.take_dirty()
            | self.tray_menu.take_dirty()
            | self.notifications.take_dirty()
            | self.dnd.take_dirty()
            | self.action_result.take_dirty()
    }

    /// A publisher that feeds every manager snapshot into this state.
    pub fn publisher(state: &Arc<Self>) -> Publisher {
        let state = Arc::clone(state);
        Publisher::new(move |channel, payload| {
            let changed = match channel {
                "audio" => state.audio.set(payload),
                "net" => state.net.set(payload),
                "wifi" => state.wifi.set(payload),
                "wifi_networks" => state.wifi_networks.set(payload),
                "bluetooth" => state.bluetooth.set(payload),
                "battery" => state.battery.set(payload),
                "music" => state.music.set(payload),
                "polkit" => state.polkit.set(payload),
                other => {
                    tracing::warn!("unhandled state channel: {other}");
                    false
                }
            };
            // Only wake the UI when something actually changed: the audio
            // monitor publishes on every PipeWire event, most of which carry
            // identical state.
            if changed {
                state.notify();
            }
        })
    }
}

impl<T: Clone + PartialEq + Default> Default for Cell<T> {
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
        assert!(cell.set(2), "a real change is reported");
        assert!(cell.take_dirty());
        assert_eq!(cell.get(), 2);

        // Re-publishing the same snapshot must not count as a change.
        assert!(!cell.set(2));
        assert!(!cell.take_dirty(), "identical values stay clean");
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
