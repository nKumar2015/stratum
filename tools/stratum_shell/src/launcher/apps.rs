//! Applications for the launcher: desktop entries, their `Exec` lines, and
//! how often each has been launched.
//!
//! Entries are read when the launcher opens and dropped when it closes, so an
//! app installed meanwhile shows up and nothing stays resident. Earlier XDG
//! data directories win for the same desktop ID, as the spec orders them.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// One launchable application.
#[derive(Clone, Debug, PartialEq)]
pub struct App {
    /// The desktop ID (`org.gnome.Nautilus.desktop`), which keys usage.
    pub id: String,
    pub name: String,
    pub generic: Option<String>,
    pub comment: Option<String>,
    pub keywords: Vec<String>,
    pub icon: Option<String>,
    /// The command, field codes already expanded.
    pub argv: Vec<String>,
    pub terminal: bool,
}

/// Every application the menu should list, sorted by name.
pub fn load() -> Vec<App> {
    let mut by_id: HashMap<String, Option<App>> = HashMap::new();
    for dir in crate::icons::data_dirs() {
        let root = dir.join("applications");
        let mut files = Vec::new();
        collect(&root, &mut files);
        for path in files {
            let Some(id) = desktop_id(&root, &path) else {
                continue;
            };
            // A hidden entry in an earlier directory still hides later ones.
            by_id.entry(id.clone()).or_insert_with(|| {
                let text = std::fs::read_to_string(&path).ok()?;
                parse(&id, &text)
            });
        }
    }
    let mut apps: Vec<App> = by_id.into_values().flatten().collect();
    apps.sort_by_cached_key(|a| a.name.to_lowercase());
    apps
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("desktop") {
            out.push(path);
        }
    }
}

/// `applications/foo/bar.desktop` is `foo-bar.desktop`.
fn desktop_id(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let parts: Vec<_> = relative.iter().filter_map(|p| p.to_str()).collect();
    Some(parts.join("-"))
}

/// The `[Desktop Entry]` group of a desktop file, if it is an application
/// to list: not hidden, not `NoDisplay`, shown on any desktop, with a name
/// and a command.
pub fn parse(id: &str, text: &str) -> Option<App> {
    let mut group = false;
    let mut fields: BTreeMap<&str, &str> = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            group = line == "[Desktop Entry]";
            continue;
        }
        if !group || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            // Localised keys (`Name[de]`) are skipped: the shell is not
            // localised either.
            fields.entry(key.trim()).or_insert(value.trim());
        }
    }
    let flag = |key: &str| fields.get(key).is_some_and(|v| *v == "true");
    if fields.get("Type").copied().unwrap_or("Application") != "Application" || flag("Hidden") || flag("NoDisplay") {
        return None;
    }
    // Entries limited to other desktops (`OnlyShowIn=KDE;`).
    if fields.get("OnlyShowIn").is_some_and(|v| !v.split(';').any(|d| d.eq_ignore_ascii_case("Hyprland"))) {
        return None;
    }
    let name = unescape(fields.get("Name")?);
    let icon = fields.get("Icon").map(|i| i.to_string()).filter(|i| !i.is_empty());
    let argv = expand(&split_exec(fields.get("Exec")?)?, &name, icon.as_deref());
    if argv.is_empty() {
        return None;
    }
    let list = |key: &str| -> Vec<String> {
        fields.get(key).map_or_else(Vec::new, |v| v.split(';').filter(|s| !s.is_empty()).map(unescape).collect())
    };
    Some(App {
        id: id.to_string(),
        name,
        generic: fields.get("GenericName").map(|s| unescape(s)),
        comment: fields.get("Comment").map(|s| unescape(s)),
        keywords: list("Keywords"),
        icon,
        argv,
        terminal: flag("Terminal"),
    })
}

/// The string escapes of a desktop entry value (`\s \n \t \r \\`).
fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// Split an `Exec` value into arguments: separated by spaces, with
/// double-quoted arguments in which `\"`, `` \` ``, `\$` and `\\` are
/// escapes. `None` for an unterminated quote.
pub fn split_exec(exec: &str) -> Option<Vec<String>> {
    let exec = unescape_exec_level(exec);
    let mut args = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut chars = exec.chars();
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' => {
                if started {
                    args.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            '"' => {
                started = true;
                loop {
                    match chars.next()? {
                        '"' => break,
                        '\\' => match chars.next()? {
                            e @ ('"' | '`' | '$' | '\\') => current.push(e),
                            other => {
                                current.push('\\');
                                current.push(other);
                            }
                        },
                        other => current.push(other),
                    }
                }
            }
            other => {
                started = true;
                current.push(other);
            }
        }
    }
    if started {
        args.push(current);
    }
    Some(args)
}

/// The general string escapes apply to `Exec` before its own quoting, but
/// only `\\` matters there: it is how a literal backslash inside quotes is
/// written (`\\\\`).
fn unescape_exec_level(exec: &str) -> String {
    exec.replace("\\\\", "\\")
}

/// Expand field codes: no file or URL is passed, so `%f %F %u %U` (and the
/// deprecated `%d %D %n %N %v %m`) vanish, `%c` is the name, `%i` the icon
/// flag, `%k` nothing, and `%%` a percent sign.
pub fn expand(args: &[String], name: &str, icon: Option<&str>) -> Vec<String> {
    let mut out = Vec::new();
    for arg in args {
        match arg.as_str() {
            "%f" | "%F" | "%u" | "%U" | "%d" | "%D" | "%n" | "%N" | "%v" | "%m" | "%k" => {}
            "%i" => {
                if let Some(icon) = icon {
                    out.extend(["--icon".to_string(), icon.to_string()]);
                }
            }
            _ => {
                let mut expanded = String::new();
                let mut chars = arg.chars();
                while let Some(c) = chars.next() {
                    if c != '%' {
                        expanded.push(c);
                        continue;
                    }
                    match chars.next() {
                        Some('%') => expanded.push('%'),
                        Some('c') => expanded.push_str(name),
                        // Any other code inside an argument is dropped.
                        _ => {}
                    }
                }
                out.push(expanded);
            }
        }
    }
    out
}

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
    fn exec_quoting_and_field_codes() {
        let args = split_exec(r#"env "FOO=a b" app --flag=%u "quo\"te" %F"#).unwrap();
        assert_eq!(args, ["env", "FOO=a b", "app", "--flag=%u", "quo\"te", "%F"]);
        assert_eq!(expand(&args, "App", None), ["env", "FOO=a b", "app", "--flag=", "quo\"te"]);
        let args = split_exec("app %i --title=%c 100%%").unwrap();
        assert_eq!(expand(&args, "My App", Some("my-icon")), ["app", "--icon", "my-icon", "--title=My App", "100%"]);
        assert_eq!(split_exec(r#"app "unterminated"#), None);
    }

    #[test]
    fn parses_an_application_entry() {
        let text = "[Desktop Entry]\nType=Application\nName=Files\nName[de]=Dateien\nGenericName=File Manager\nKeywords=folder;explorer;\nIcon=org.gnome.Nautilus\nExec=nautilus --new-window %U\nTerminal=false\n\n[Desktop Action new]\nName=Other\nExec=other\n";
        let app = parse("org.gnome.Nautilus.desktop", text).unwrap();
        assert_eq!(app.name, "Files");
        assert_eq!(app.generic.as_deref(), Some("File Manager"));
        assert_eq!(app.keywords, ["folder", "explorer"]);
        assert_eq!(app.argv, ["nautilus", "--new-window"]);
        assert!(!app.terminal);
    }

    #[test]
    fn skips_what_should_not_be_listed() {
        let base = "[Desktop Entry]\nName=X\nExec=x\n";
        assert!(parse("x.desktop", base).is_some());
        assert!(parse("x.desktop", &format!("{base}NoDisplay=true\n")).is_none());
        assert!(parse("x.desktop", &format!("{base}Hidden=true\n")).is_none());
        assert!(parse("x.desktop", &format!("{base}OnlyShowIn=KDE;\n")).is_none());
        assert!(parse("x.desktop", &format!("{base}OnlyShowIn=KDE;Hyprland;\n")).is_some());
        assert!(parse("x.desktop", "[Desktop Entry]\nType=Link\nName=X\nURL=x\n").is_none());
        assert!(parse("x.desktop", "[Desktop Entry]\nName=X\n").is_none(), "no Exec");
    }

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
