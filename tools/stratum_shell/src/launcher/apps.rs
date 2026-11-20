//! Applications for the launcher: desktop entries, their `Exec` lines, and
//! how often each has been launched.
//!
//! Entries are read when the launcher opens and dropped when it closes, so an
//! app installed meanwhile shows up and nothing stays resident. Earlier XDG
//! data directories win for the same desktop ID, as the spec orders them.

use std::collections::BTreeMap;
use std::path::PathBuf;

pub use stratum_ui::apps::{load, App};

/// How often each app was launched, and when last: the ranking for an empty
/// query, and a boost for a typed one.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Usage {
    #[serde(flatten)]
    entries: BTreeMap<String, UseCount>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct UseCount {
    pub count: u32,
    /// Seconds since the epoch.
    pub last: u64,
}

impl Usage {
    fn path() -> Option<PathBuf> {
        crate::paths::state_file("launcher.json")
    }

    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(path) = Self::path() else {
            return;
        };
        match serde_json::to_string(self) {
            Ok(text) => {
                if let Err(err) = crate::paths::write_atomic(&path, text) {
                    tracing::warn!("could not save {}: {err}", path.display());
                }
            }
            Err(err) => tracing::warn!("could not encode launcher usage: {err}"),
        }
    }

    pub fn record(&mut self, id: &str, now: u64) {
        let entry = self.entries.entry(id.to_string()).or_default();
        entry.count = entry.count.saturating_add(1);
        entry.last = now;
    }

    /// A ranking weight: launches, halved for every two weeks since the last.
    pub fn weight(&self, id: &str, now: u64) -> f32 {
        let Some(entry) = self.entries.get(id) else {
            return 0.0;
        };
        let weeks = now.saturating_sub(entry.last) as f32 / (14.0 * 86_400.0);
        entry.count as f32 * 0.5f32.powf(weeks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_decays() {
        let mut usage = Usage::default();
        usage.record("a.desktop", 0);
        usage.record("a.desktop", 0);
        assert_eq!(usage.weight("a.desktop", 0), 2.0);
        assert_eq!(usage.weight("a.desktop", 14 * 86_400), 1.0);
        assert_eq!(usage.weight("b.desktop", 0), 0.0);
        let text = serde_json::to_string(&usage).unwrap();
        assert_eq!(serde_json::from_str::<Usage>(&text).unwrap(), usage);
    }
}
