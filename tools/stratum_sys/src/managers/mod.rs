//! System integration managers.
//!
//! Each submodule owns one domain and is safe to drive from a worker thread.
//! Managers never assume a particular host: state changes are reported through
//! [`crate::Publisher`] and queries return `serde_json::Value` snapshots.

pub mod audio;
pub mod battery;
pub mod bluetooth;
pub mod common;
pub mod dashboard;
pub mod music;
pub mod net;
pub mod polkit;
pub mod wifi;
