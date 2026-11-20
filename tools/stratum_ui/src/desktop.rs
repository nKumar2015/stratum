//! Desktop entries, for finding an application's icon from its window class.
//!
//! Window classes rarely equal icon names (Zen's class is not its icon's
//! name; JetBrains IDEs report `jetbrains-idea`), so the icon is taken from the
//! application's desktop entry: the one whose `StartupWMClass` matches the
//! class, else the one whose file name does. The index is built once, from the
//! `applications` directory of every XDG data directory.

use std::collections::HashMap;
use std::path::Path;

#[derive(Default)]
pub struct DesktopIndex {
    /// Lowercased `StartupWMClass` -> `Icon`.
    by_wm_class: HashMap<String, String>,
    /// Lowercased file stem (`org.gnome.Nautilus`) -> `Icon`.
    by_stem: HashMap<String, String>,
}

impl DesktopIndex {
    pub fn load() -> Self {
        let mut index = Self::default();
        // Earlier directories win, as the XDG spec orders them.
        for dir in crate::icons::data_dirs() {
            let Ok(entries) = std::fs::read_dir(dir.join("applications")) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("desktop") {
                    index.add(&path);
                }
            }
        }
        index
    }

    fn add(&mut self, path: &Path) {
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        let Some((icon, wm_class)) = parse(&text) else {
            return;
        };
        if let Some(class) = wm_class {
            self.by_wm_class.entry(class.to_lowercase()).or_insert_with(|| icon.clone());
        }
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            self.by_stem.entry(stem.to_lowercase()).or_insert(icon);
        }
    }

    /// Icon names to try for a window class, best first. The class itself is
    /// always a candidate (many apps name their icon after it), and the
    /// generic executable icon is the last resort, as the QML title used.
    pub fn icon_candidates(&self, class: &str) -> Vec<String> {
        let class = class.trim().to_lowercase();
        let mut out = Vec::new();
        if !class.is_empty() {
            out.extend(self.by_wm_class.get(&class).cloned());
            out.extend(self.by_stem.get(&class).cloned());
            // Reverse-DNS classes (`org.kde.dolphin`) often have a stem-named
            // entry under the last component too.
            if let Some(last) = class.rsplit('.').next().filter(|l| *l != class) {
                out.extend(self.by_stem.get(last).cloned());
            }
            out.push(class);
        }
        out.push("application-x-executable".to_string());
        let mut seen = std::collections::HashSet::new();
        out.retain(|name| seen.insert(name.clone()));
        out
    }
}

/// `Icon` and `StartupWMClass` from the `[Desktop Entry]` group. Entries
/// without an icon, and hidden ones, are skipped.
fn parse(text: &str) -> Option<(String, Option<String>)> {
    let mut in_entry = false;
    let mut icon = None;
    let mut wm_class = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "Icon" => icon = Some(value.trim().to_string()),
            "StartupWMClass" => wm_class = Some(value.trim().to_string()),
            "Hidden" | "NoDisplay" if value.trim() == "true" => {
                // NoDisplay entries still name the app's icon; only Hidden
                // means "deleted".
                if key.trim() == "Hidden" {
                    return None;
                }
            }
            _ => {}
        }
    }
    icon.filter(|i| !i.is_empty()).map(|icon| (icon, wm_class))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_desktop_entry_group_only() {
        let text = "[Desktop Entry]\nName=Zen\nIcon=zen-twilight\nStartupWMClass=zen\n\n[Desktop Action new]\nIcon=other\n";
        assert_eq!(parse(text), Some(("zen-twilight".into(), Some("zen".into()))));
        assert_eq!(parse("[Desktop Entry]\nName=x\n"), None, "no icon, no entry");
        assert_eq!(parse("[Desktop Entry]\nIcon=x\nHidden=true\n"), None);
    }

    #[test]
    fn candidates_prefer_wm_class_then_stem_then_class() {
        let mut index = DesktopIndex::default();
        index.by_wm_class.insert("zen".into(), "zen-twilight".into());
        index.by_stem.insert("kitty".into(), "kitty".into());
        index.by_stem.insert("dolphin".into(), "system-file-manager".into());
        assert_eq!(index.icon_candidates("Zen"), vec!["zen-twilight", "zen", "application-x-executable"]);
        assert_eq!(index.icon_candidates("kitty"), vec!["kitty", "application-x-executable"]);
        assert_eq!(
            index.icon_candidates("org.kde.dolphin"),
            vec!["system-file-manager", "org.kde.dolphin", "application-x-executable"]
        );
        assert_eq!(index.icon_candidates(""), vec!["application-x-executable"]);
    }
}
