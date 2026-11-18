//! Themes made in the Theme page, one file each.
//!
//! The config's `[themes.*]` is usually a read-only Nix store link, so a
//! theme made in the settings window is saved to
//! `$XDG_STATE_HOME/stratum/themes/<name>.toml` instead: the same keys as a
//! `[themes.<name>]` table, at the top level, named by the file. They are
//! merged into the configuration's themes at start and on every reload. A
//! built-in (config) theme of the same name wins, so a stray file can never
//! change a theme the config defines.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::config::{Config, ThemeConfig};

/// The longest name a custom theme may have.
pub const MAX_NAME: usize = 32;

/// The directory that holds the theme files.
pub fn dir() -> Option<PathBuf> {
    crate::paths::state_file("themes")
}

/// Every readable theme in the directory. A file that does not parse, or
/// whose colours do not, is logged and skipped: one broken file must not
/// cost the user the others, or the shell.
pub fn load_all() -> BTreeMap<String, ThemeConfig> {
    dir().map(|dir| load_from(&dir)).unwrap_or_default()
}

fn load_from(dir: &Path) -> BTreeMap<String, ThemeConfig> {
    let mut themes = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return themes;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if check_syntax(name).is_err() {
            tracing::warn!("ignoring custom theme {}: not a valid theme name", path.display());
            continue;
        }
        match std::fs::read_to_string(&path)
            .map_err(|err| err.to_string())
            .and_then(|text| parse(&text))
        {
            Ok(theme) => {
                themes.insert(name.to_string(), theme);
            }
            Err(err) => tracing::warn!("ignoring custom theme {}: {err}", path.display()),
        }
    }
    themes
}

/// A theme file's contents, checked as the config checks its themes.
fn parse(text: &str) -> Result<ThemeConfig, String> {
    let theme: ThemeConfig = toml::from_str(text).map_err(|err| err.to_string())?;
    theme.to_theme()?;
    Ok(theme)
}

/// The file's text for `theme`.
fn serialize(theme: &ThemeConfig) -> Result<String, String> {
    toml::to_string(theme).map_err(|err| err.to_string())
}

/// Save `theme` as `name`.
pub fn save(name: &str, theme: &ThemeConfig) -> Result<(), String> {
    check_syntax(name).map_err(str::to_string)?;
    let dir = dir().ok_or("no state directory ($HOME is unset)")?;
    let path = dir.join(format!("{name}.toml"));
    let text = serialize(theme)?;
    crate::paths::write_atomic(&path, text).map_err(|err| format!("{}: {err}", path.display()))
}

/// Remove the file of the theme `name`. One that is already gone is fine.
pub fn delete(name: &str) -> Result<(), String> {
    check_syntax(name).map_err(str::to_string)?;
    let dir = dir().ok_or("no state directory ($HOME is unset)")?;
    let path = dir.join(format!("{name}.toml"));
    match std::fs::remove_file(&path) {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(format!("{}: {err}", path.display())),
        _ => Ok(()),
    }
}

/// Whether `name` is usable as a theme and a file name: lowercase letters,
/// digits, `-` and `_`, at most [`MAX_NAME`] characters.
pub fn check_syntax(name: &str) -> Result<(), &'static str> {
    if name.is_empty() {
        return Err("Give the theme a name");
    }
    if name.chars().count() > MAX_NAME {
        return Err("Names are at most 32 characters");
    }
    if !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_') {
        return Err("Use lowercase letters, digits, - and _");
    }
    Ok(())
}

/// Whether a custom theme may be saved as `name`: the syntax holds, no
/// built-in theme has it, and no other custom theme does. `original` is the
/// name the theme being edited was saved under, which it may keep.
pub fn check_name(
    name: &str,
    themes: &BTreeMap<String, ThemeConfig>,
    custom: &BTreeSet<String>,
    original: Option<&str>,
) -> Result<(), &'static str> {
    check_syntax(name)?;
    if Some(name) == original {
        return Ok(());
    }
    if custom.contains(name) {
        return Err("Another custom theme has this name");
    }
    if themes.contains_key(name) {
        return Err("A built-in theme has this name");
    }
    Ok(())
}

/// Add the saved custom themes to `config`. Returns the names added.
pub fn merge_into(config: &mut Config) -> BTreeSet<String> {
    merge(config, load_all())
}

fn merge(config: &mut Config, custom: BTreeMap<String, ThemeConfig>) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for (name, theme) in custom {
        if config.themes.contains_key(&name) {
            tracing::warn!("custom theme `{name}` has the name of a configured theme; the configured one is kept");
            continue;
        }
        config.themes.insert(name.clone(), theme);
        names.insert(name);
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn carbon() -> ThemeConfig {
        Config::test_defaults().themes["carbon"].clone()
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("stratum-custom-themes-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn names_must_be_file_safe() {
        assert!(check_syntax("my-theme_2").is_ok());
        assert!(check_syntax("").is_err());
        assert!(check_syntax("My Theme").is_err());
        assert!(check_syntax("../escape").is_err());
        assert!(check_syntax(&"a".repeat(MAX_NAME + 1)).is_err());
    }

    #[test]
    fn a_name_may_not_shadow_another_theme_but_may_keep_its_own() {
        let config = Config::test_defaults();
        let custom: BTreeSet<String> = ["mine".to_string()].into();
        let mut themes = config.themes.clone();
        themes.insert("mine".into(), carbon());
        assert!(check_name("carbon", &themes, &custom, None).is_err());
        assert!(check_name("mine", &themes, &custom, None).is_err());
        assert!(check_name("mine", &themes, &custom, Some("mine")).is_ok());
        assert!(check_name("other", &themes, &custom, Some("mine")).is_ok());
    }

    #[test]
    fn a_theme_round_trips_through_its_file() {
        let theme = carbon();
        let text = serialize(&theme).unwrap();
        assert!(!text.contains("[themes"), "the file holds the theme's keys at the top level");
        let back = parse(&text).unwrap();
        assert_eq!(serialize(&back).unwrap(), text);
    }

    #[test]
    fn configured_themes_win_over_custom_ones() {
        let mut config = Config::test_defaults();
        let mut changed = carbon();
        changed.primary = "#ff0000".into();
        let before = config.themes["carbon"].primary.clone();
        let custom = BTreeMap::from([("carbon".to_string(), changed.clone()), ("mine".to_string(), changed)]);
        let names = merge(&mut config, custom);
        assert_eq!(names, BTreeSet::from(["mine".to_string()]));
        assert_eq!(config.themes["carbon"].primary, before);
        assert!(config.themes.contains_key("mine"));
    }

    #[test]
    fn broken_files_are_skipped() {
        let dir = scratch("broken");
        std::fs::write(dir.join("good.toml"), serialize(&carbon()).unwrap()).unwrap();
        std::fs::write(dir.join("bad.toml"), "primary = 3").unwrap();
        let mut bad_colour = carbon();
        bad_colour.primary = "blue".into();
        std::fs::write(dir.join("colour.toml"), serialize(&bad_colour).unwrap()).unwrap();
        std::fs::write(dir.join("Bad Name.toml"), serialize(&carbon()).unwrap()).unwrap();
        std::fs::write(dir.join("left.json.tmp"), "junk").unwrap();
        let themes = load_from(&dir);
        assert_eq!(themes.keys().collect::<Vec<_>>(), ["good"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
