//! The theme picked live, kept across reloads and restarts.
//!
//! The config file's `theme` is often a read-only Nix store link, so a theme
//! picked in the switcher, the Display page or `stratum-cli theme set` is
//! saved to `$XDG_STATE_HOME/stratum/theme.json` instead, together with the
//! file's `theme` at the time. The pick overrides the file only while the
//! file still says that: changing `theme` in the config is the newer choice,
//! and it wins and clears the saved one.

use std::path::PathBuf;

use serde_json::{json, Value};

/// A theme picked live, over the config's `theme` when it was picked.
#[derive(Clone, Debug, PartialEq)]
pub struct Saved {
    pub theme: String,
    pub config_theme: String,
}

/// `$XDG_STATE_HOME`, else `~/.local/state`.
pub fn state_dir() -> Option<PathBuf> {
    match std::env::var("XDG_STATE_HOME") {
        Ok(dir) if !dir.trim().is_empty() => Some(PathBuf::from(dir)),
        _ => std::env::var("HOME").ok().map(|h| PathBuf::from(h).join(".local/state")),
    }
}

fn path() -> Option<PathBuf> {
    state_dir().map(|d| d.join("stratum/theme.json"))
}

pub fn load() -> Option<Saved> {
    let path = path()?;
    let text = std::fs::read_to_string(&path).ok()?;
    let parsed = serde_json::from_str::<Value>(&text).ok().and_then(|v| from_json(&v));
    if parsed.is_none() {
        tracing::warn!("ignoring unreadable {}", path.display());
    }
    parsed
}

fn from_json(value: &Value) -> Option<Saved> {
    Some(Saved {
        theme: value.get("theme")?.as_str()?.to_string(),
        config_theme: value.get("config_theme")?.as_str()?.to_string(),
    })
}

/// Save `saved`, or remove the file for `None`. Failures are logged: losing
/// the pick at the next start is not worth stopping the switch for.
pub fn store(saved: Option<&Saved>) {
    let Some(path) = path() else { return };
    let result = match saved {
        None => match std::fs::remove_file(&path) {
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other,
        },
        Some(saved) => {
            let text = json!({ "theme": saved.theme, "config_theme": saved.config_theme }).to_string();
            // Written beside and renamed, so a crash cannot leave half a file.
            let tmp = path.with_extension("json.tmp");
            path.parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(&tmp, text))
                .and_then(|()| std::fs::rename(&tmp, &path))
        }
    };
    if let Err(err) = result {
        tracing::warn!("could not save the theme choice to {}: {err}", path.display());
    }
}

/// The saved pick if it still applies to a config whose `theme` is
/// `config_theme`: the file has not changed its theme since, and the picked
/// theme still exists.
pub fn still_applies(saved: Option<Saved>, config_theme: &str, exists: impl Fn(&str) -> bool) -> Option<Saved> {
    saved.filter(|s| s.config_theme == config_theme && exists(&s.theme))
}

/// Apply `saved` to a freshly loaded `config` and return it if it still
/// applies; the caller clears the file when a pick it had is gone.
pub fn apply(config: &mut crate::config::Config, saved: Option<Saved>) -> Option<Saved> {
    let kept = still_applies(saved, &config.theme, |t| config.themes.contains_key(t));
    if let Some(kept) = &kept {
        config.theme = kept.theme.clone();
    }
    kept
}

/// What to save after switching to `theme` live: nothing when that is the
/// config's own theme.
pub fn pick(theme: &str, config_theme: &str) -> Option<Saved> {
    (theme != config_theme).then(|| Saved { theme: theme.to_string(), config_theme: config_theme.to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved(theme: &str, over: &str) -> Saved {
        Saved { theme: theme.into(), config_theme: over.into() }
    }

    #[test]
    fn a_pick_applies_while_the_config_keeps_its_theme() {
        let s = saved("gruvbox", "carbon");
        assert_eq!(still_applies(Some(s.clone()), "carbon", |_| true), Some(s));
    }

    #[test]
    fn changing_the_configs_theme_overrides_the_pick() {
        assert_eq!(still_applies(Some(saved("gruvbox", "carbon")), "rosepine", |_| true), None);
    }

    #[test]
    fn a_pick_of_a_removed_theme_is_dropped() {
        assert_eq!(still_applies(Some(saved("gone", "carbon")), "carbon", |t| t != "gone"), None);
    }

    #[test]
    fn picking_the_configs_own_theme_saves_nothing() {
        assert_eq!(pick("carbon", "carbon"), None);
        assert_eq!(pick("gruvbox", "carbon"), Some(saved("gruvbox", "carbon")));
    }

    #[test]
    fn apply_sets_the_picked_theme() {
        let mut config = crate::config::Config::test_defaults();
        let file_theme = config.theme.clone();
        let other = config.themes.keys().find(|t| **t != file_theme).unwrap().clone();
        let kept = apply(&mut config, Some(saved(&other, &file_theme)));
        assert_eq!(config.theme, other);
        assert_eq!(kept.map(|s| s.config_theme), Some(file_theme));
    }

    #[test]
    fn the_file_round_trips() {
        let s = saved("gruvbox", "carbon");
        let value = json!({ "theme": s.theme, "config_theme": s.config_theme });
        assert_eq!(from_json(&value), Some(s));
        assert_eq!(from_json(&json!({ "theme": "gruvbox" })), None);
    }
}
