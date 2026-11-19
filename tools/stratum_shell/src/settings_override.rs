//! Settings changed in the settings window, kept over the config.
//!
//! The config file is usually a read-only Nix store link, so a value changed
//! on a page (idle timeouts, night light) is saved to
//! `$XDG_STATE_HOME/stratum/settings.json` instead, by its dotted path, with
//! the config's value at that path when it was changed. It applies over the
//! config only while the config still says that: editing the key in the
//! config is the newer choice, which wins and drops the override, as
//! `theme_choice` does for the theme.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Overrides {
    #[serde(flatten)]
    entries: BTreeMap<String, Override>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Override {
    value: toml::Value,
    /// The config's value when it was overridden; `None` if it had none.
    base: Option<toml::Value>,
}

fn path() -> Option<PathBuf> {
    crate::paths::state_file("settings.json")
}

impl Overrides {
    pub fn load() -> Self {
        path()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(path) = path() else { return };
        let result = serde_json::to_vec_pretty(self)
            .map_err(std::io::Error::other)
            .and_then(|bytes| crate::paths::write_atomic(&path, bytes));
        if let Err(err) = result {
            tracing::warn!("could not save settings to {}: {err}", path.display());
        }
    }

    /// Set `path` to `value`, over `base`, the config's value there. Setting
    /// it back to `base` removes the override.
    pub fn set(&mut self, path: &str, value: toml::Value, base: Option<toml::Value>) {
        if base.as_ref() == Some(&value) {
            self.entries.remove(path);
        } else {
            self.entries.insert(path.to_string(), Override { value, base });
        }
    }

    /// Apply over `table` (the config without overrides). An override whose
    /// base no longer matches the config is dropped; returns whether any
    /// was, so the caller can save.
    pub fn apply(&mut self, table: &mut toml::Table) -> bool {
        let before = self.entries.len();
        self.entries.retain(|path, o| get(table, path) == o.base.as_ref());
        for (path, o) in &self.entries {
            set(table, path, o.value.clone());
        }
        self.entries.len() != before
    }
}

/// The value at a dotted `path` in `table`.
pub fn get<'a>(table: &'a toml::Table, path: &str) -> Option<&'a toml::Value> {
    let mut parts = path.split('.');
    let mut value = table.get(parts.next()?)?;
    for part in parts {
        value = value.as_table()?.get(part)?;
    }
    Some(value)
}

fn set(table: &mut toml::Table, path: &str, value: toml::Value) {
    let mut parts: Vec<&str> = path.split('.').collect();
    let Some(last) = parts.pop() else { return };
    let mut current = table;
    for part in parts {
        let next = current.entry(part.to_string()).or_insert_with(|| toml::Value::Table(toml::Table::new()));
        let Some(next) = next.as_table_mut() else { return };
        current = next;
    }
    current.insert(last.to_string(), value);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(text: &str) -> toml::Table {
        toml::from_str(text).unwrap()
    }

    #[test]
    fn an_override_applies_while_the_config_is_unchanged() {
        let mut overrides = Overrides::default();
        overrides.set("idle.dim_s", toml::Value::Integer(60), Some(toml::Value::Integer(240)));
        let mut config = table("[idle]\ndim_s = 240\nlock_s = 300\n");
        assert!(!overrides.apply(&mut config));
        assert_eq!(get(&config, "idle.dim_s"), Some(&toml::Value::Integer(60)));
        assert_eq!(get(&config, "idle.lock_s"), Some(&toml::Value::Integer(300)), "others untouched");
    }

    #[test]
    fn editing_the_config_wins() {
        let mut overrides = Overrides::default();
        overrides.set("idle.dim_s", toml::Value::Integer(60), Some(toml::Value::Integer(240)));
        let mut config = table("[idle]\ndim_s = 120\n");
        assert!(overrides.apply(&mut config), "dropped");
        assert_eq!(get(&config, "idle.dim_s"), Some(&toml::Value::Integer(120)));
        assert!(overrides.entries.is_empty());
    }

    #[test]
    fn setting_the_config_value_back_removes_the_override() {
        let mut overrides = Overrides::default();
        overrides.set("night_light.enabled", toml::Value::Boolean(true), Some(toml::Value::Boolean(false)));
        overrides.set("night_light.enabled", toml::Value::Boolean(false), Some(toml::Value::Boolean(false)));
        assert!(overrides.entries.is_empty());
    }

    #[test]
    fn a_missing_key_is_created_and_survives_json() {
        let mut overrides = Overrides::default();
        overrides.set("night_light.latitude", toml::Value::Float(37.2), None);
        let mut config = table("[night_light]\nenabled = true\n");
        overrides.apply(&mut config);
        assert_eq!(get(&config, "night_light.latitude"), Some(&toml::Value::Float(37.2)));
        let text = serde_json::to_string(&overrides).unwrap();
        assert_eq!(serde_json::from_str::<Overrides>(&text).unwrap(), overrides);
    }
}
