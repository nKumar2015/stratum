use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

pub struct AudioState {
    data: Mutex<Value>,
    pub dirty: AtomicBool,
}

impl AudioState {
    pub fn new() -> Self {
        Self {
            data: Mutex::new(Value::Null),
            dirty: AtomicBool::new(false),
        }
    }

    pub fn update(&self, value: Value) {
        let mut data = self.data.lock().unwrap();
        if *data != value {
            *data = value;
            self.dirty.store(true, Ordering::Release);
        }
    }

    pub fn snapshot(&self) -> Value {
        self.data.lock().unwrap().clone()
    }

    pub fn take_if_dirty(&self) -> Option<Value> {
        if self.dirty.swap(false, Ordering::AcqRel) {
            Some(json!({"audio": self.snapshot()}))
        } else {
            None
        }
    }
}

pub struct NetState {
    data: Mutex<Value>,
    pub dirty: AtomicBool,
}

impl NetState {
    pub fn new() -> Self {
        Self {
            data: Mutex::new(Value::Null),
            dirty: AtomicBool::new(false),
        }
    }

    pub fn update(&self, value: Value) {
        let mut data = self.data.lock().unwrap();
        if *data != value {
            *data = value;
            self.dirty.store(true, Ordering::Release);
        }
    }

    pub fn snapshot(&self) -> Value {
        self.data.lock().unwrap().clone()
    }

    pub fn take_if_dirty(&self) -> Option<Value> {
        if self.dirty.swap(false, Ordering::AcqRel) {
            Some(json!({"net": self.snapshot()}))
        } else {
            None
        }
    }
}

pub struct BluetoothState {
    data: Mutex<Value>,
    pub dirty: AtomicBool,
}

impl BluetoothState {
    pub fn new() -> Self {
        Self {
            data: Mutex::new(Value::Null),
            dirty: AtomicBool::new(false),
        }
    }

    pub fn update(&self, value: Value) {
        let mut data = self.data.lock().unwrap();
        if *data != value {
            *data = value;
            self.dirty.store(true, Ordering::Release);
        }
    }

    pub fn snapshot(&self) -> Value {
        self.data.lock().unwrap().clone()
    }

    pub fn take_if_dirty(&self) -> Option<Value> {
        if self.dirty.swap(false, Ordering::AcqRel) {
            Some(json!({"bluetooth": self.snapshot()}))
        } else {
            None
        }
    }
}

pub struct MusicState {
    data: Mutex<Value>,
    pub dirty: AtomicBool,
}

impl MusicState {
    pub fn new() -> Self {
        Self {
            data: Mutex::new(Value::Null),
            dirty: AtomicBool::new(false),
        }
    }

    pub fn update(&self, value: Value) {
        let mut data = self.data.lock().unwrap();
        if *data != value {
            *data = value;
            self.dirty.store(true, Ordering::Release);
        }
    }

    pub fn snapshot(&self) -> Value {
        self.data.lock().unwrap().clone()
    }

    pub fn take_if_dirty(&self) -> Option<Value> {
        if self.dirty.swap(false, Ordering::AcqRel) {
            Some(json!({"music": self.snapshot()}))
        } else {
            None
        }
    }
}

pub struct DashboardState {
    data: Mutex<Value>,
    pub dirty: AtomicBool,
}

impl DashboardState {
    pub fn new() -> Self {
        Self {
            data: Mutex::new(Value::Null),
            dirty: AtomicBool::new(false),
        }
    }

    pub fn update(&self, value: Value) {
        let mut data = self.data.lock().unwrap();
        if *data != value {
            *data = value;
            self.dirty.store(true, Ordering::Release);
        }
    }

    pub fn snapshot(&self) -> Value {
        self.data.lock().unwrap().clone()
    }

    pub fn take_if_dirty(&self) -> Option<Value> {
        if self.dirty.swap(false, Ordering::AcqRel) {
            Some(json!({"dashboard": self.snapshot()}))
        } else {
            None
        }
    }
}
