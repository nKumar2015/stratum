//! Applications from desktop entries: their names, icons, commands and the
//! MIME types they open. The shell's launcher lists them; the file manager
//! opens files with them.
//!
//! Earlier XDG data directories win for the same desktop ID, as the spec
//! orders them.

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
    /// The `Exec` arguments before field codes are expanded, for
    /// [`App::command`].
    pub exec: Vec<String>,
    /// The MIME types it opens (`MimeType`).
    pub mime_types: Vec<String>,
    /// `NoDisplay`: not listed in menus, but still an app that opens files.
    pub hidden: bool,
}

impl App {
    /// The command opening `files`: `%f` takes the first path, `%F` all of
    /// them, `%u`/`%U` the same as `file://` URIs. An `Exec` without a file
    /// code gets the paths appended, as the spec says to.
    pub fn command(&self, files: &[std::path::PathBuf]) -> Vec<String> {
        let uri = |p: &std::path::PathBuf| crate::uri::from_path(p);
        let path = |p: &std::path::PathBuf| p.to_string_lossy().into_owned();
        let mut out = Vec::new();
        let mut took = false;
        for arg in &self.exec {
            match arg.as_str() {
                "%f" => {
                    out.extend(files.first().map(path));
                    took = true;
                }
                "%F" => {
                    out.extend(files.iter().map(path));
                    took = true;
                }
                "%u" => {
                    out.extend(files.first().map(uri));
                    took = true;
                }
                "%U" => {
                    out.extend(files.iter().map(uri));
                    took = true;
                }
                _ => out.extend(expand(std::slice::from_ref(arg), &self.name, self.icon.as_deref())),
            }
        }
        if !took {
            out.extend(files.iter().map(path));
        }
        out
    }
}

/// Every application the menu should list, sorted by name.
pub fn load() -> Vec<App> {
    let mut apps = load_all();
    apps.retain(|a| !a.hidden);
    apps
}

/// Every application, those `NoDisplay` hides from menus too (they still
/// open files), sorted by name.
pub fn load_all() -> Vec<App> {
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

/// The `[Desktop Entry]` group of a desktop file, if it is an application:
/// not `Hidden`, shown on any desktop, with a name and a command.
/// `NoDisplay` ones are kept, marked [`App::hidden`].
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
    if fields.get("Type").copied().unwrap_or("Application") != "Application" || flag("Hidden") {
        return None;
    }
    // Entries limited to other desktops (`OnlyShowIn=KDE;`).
    if fields.get("OnlyShowIn").is_some_and(|v| !v.split(';').any(|d| d.eq_ignore_ascii_case("Hyprland"))) {
        return None;
    }
    let name = unescape(fields.get("Name")?);
    let icon = fields.get("Icon").map(|i| i.to_string()).filter(|i| !i.is_empty());
    let exec = split_exec(fields.get("Exec")?)?;
    let argv = expand(&exec, &name, icon.as_deref());
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
        mime_types: list("MimeType"),
        hidden: flag("NoDisplay"),
        exec,
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
        assert!(parse("x.desktop", &format!("{base}NoDisplay=true\n")).unwrap().hidden, "kept, for opening files");
        assert!(parse("x.desktop", &format!("{base}Hidden=true\n")).is_none());
        assert!(parse("x.desktop", &format!("{base}OnlyShowIn=KDE;\n")).is_none());
        assert!(parse("x.desktop", &format!("{base}OnlyShowIn=KDE;Hyprland;\n")).is_some());
        assert!(parse("x.desktop", "[Desktop Entry]\nType=Link\nName=X\nURL=x\n").is_none());
        assert!(parse("x.desktop", "[Desktop Entry]\nName=X\n").is_none(), "no Exec");
    }

    #[test]
    fn commands_take_files() {
        let text = "[Desktop Entry]\nName=Viewer\nExec=viewer --open %U\nMimeType=image/png;image/jpeg;\n";
        let app = parse("viewer.desktop", text).unwrap();
        assert_eq!(app.mime_types, ["image/png", "image/jpeg"]);
        let files = [std::path::PathBuf::from("/tmp/a b.png"), std::path::PathBuf::from("/tmp/c.png")];
        assert_eq!(app.command(&files), ["viewer", "--open", "file:///tmp/a%20b.png", "file:///tmp/c.png"]);
        let single = parse("e.desktop", "[Desktop Entry]\nName=E\nExec=editor %f\n").unwrap();
        assert_eq!(single.command(&files), ["editor", "/tmp/a b.png"]);
        let bare = parse("b.desktop", "[Desktop Entry]\nName=B\nExec=bare\n").unwrap();
        assert_eq!(bare.command(&files[..1]), ["bare", "/tmp/a b.png"]);
    }
}
