use serde_json::json;
use serde_json::Value;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

/// Snapshot cell for one state domain.
///
/// Managers publish into these; the broadcaster drains the dirty flag and
/// pushes the snapshot to the shell. Comparing before storing means an
/// unchanged poll costs nothing.
pub struct StateCell {
    data: Mutex<Value>,
    pub dirty: AtomicBool,
}

impl StateCell {
    pub fn new() -> Self {
        Self {
            data: Mutex::new(Value::Null),
            dirty: AtomicBool::new(false),
        }
    }

    pub fn update(&self, value: Value) {
        let Ok(mut data) = self.data.lock() else {
            return;
        };
        if *data != value {
            *data = value;
            self.dirty
                .store(true, std::sync::atomic::Ordering::Release);
        }
    }

    pub fn snapshot(&self) -> Value {
        self.data
            .lock()
            .map(|data| data.clone())
            .unwrap_or(Value::Null)
    }

    /// Return the snapshot once per change, wrapped under `key`.
    pub fn take_if_dirty(&self, key: &str) -> Option<Value> {
        if self.dirty.swap(false, std::sync::atomic::Ordering::AcqRel) {
            Some(json!({ key: self.snapshot() }))
        } else {
            None
        }
    }
}

impl Default for StateCell {
    fn default() -> Self {
        Self::new()
    }
}

pub type AudioState = StateCell;
pub type NetState = StateCell;
pub type BluetoothState = StateCell;
pub type MusicState = StateCell;
pub type DashboardState = StateCell;
pub type BatteryState = StateCell;
pub type PolkitState = StateCell;
