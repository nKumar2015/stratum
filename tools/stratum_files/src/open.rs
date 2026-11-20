//! Which apps open a file: the defaults and associations in `mimeapps.list`
//! (the user's first, then the system's), then every app whose entry says
//! it opens the type or a type it is a kind of. Opening runs the app in a
//! session of its own, so closing the file manager leaves it running.

use std::path::PathBuf;

use stratum_ui::apps::App;

use crate::mime::MimeDb;

/// A `mimeapps.list`'s groups that matter here.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MimeApps {
    pub defaults: Vec<(String, Vec<String>)>,
    pub added: Vec<(String, Vec<String>)>,
    pub removed: Vec<(String, Vec<String>)>,
}

pub fn parse_mimeapps(text: &str) -> MimeApps {
    let mut out = MimeApps::default();
    let mut group = "";
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            group = line;
            continue;
        }
        let Some((mime, ids)) = line.split_once('=') else { continue };
        let ids: Vec<String> = ids.split(';').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect();
        let entry = (mime.trim().to_string(), ids);
        match group {
            "[Default Applications]" => out.defaults.push(entry),
            "[Added Associations]" => out.added.push(entry),
            "[Removed Associations]" => out.removed.push(entry),
            _ => {}
        }
    }
    out
}

/// Every `mimeapps.list`, most important first.
fn mimeapps_files() -> Vec<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
    let mut files = vec![config.join("mimeapps.list")];
    let config_dirs = std::env::var("XDG_CONFIG_DIRS").unwrap_or_else(|_| "/etc/xdg".into());
    files.extend(config_dirs.split(':').filter(|d| !d.is_empty()).map(|d| PathBuf::from(d).join("mimeapps.list")));
    files.extend(stratum_ui::icons::data_dirs().into_iter().map(|d| d.join("applications/mimeapps.list")));
    files
}

/// The apps for `mime`, the default first.
pub fn apps_for(mime: &str, db: &MimeDb, apps: &[App], lists: &[MimeApps]) -> Vec<App> {
    let types = db.ancestors(mime);
    let removed = |id: &str, t: &str| lists.iter().any(|l| l.removed.iter().any(|(m, ids)| m == t && ids.iter().any(|i| i == id)));
    let by_id = |id: &str| apps.iter().find(|a| a.id == id);
    let mut out: Vec<App> = Vec::new();
    let mut push = |app: &App| {
        if !out.iter().any(|a| a.id == app.id) {
            out.push(app.clone());
        }
    };
    for t in &types {
        for list in lists {
            for (m, ids) in list.defaults.iter().chain(&list.added) {
                if m == t {
                    ids.iter().filter(|id| !removed(id, t)).filter_map(|id| by_id(id)).for_each(&mut push);
                }
            }
        }
        for app in apps.iter().filter(|a| a.mime_types.iter().any(|m| m == t) && !removed(&a.id, t)) {
            push(app);
        }
    }
    out
}

/// Every app and association, read now: an app installed since shows up.
pub fn load() -> (Vec<App>, Vec<MimeApps>) {
    let lists = mimeapps_files().iter().filter_map(|f| std::fs::read_to_string(f).ok()).map(|t| parse_mimeapps(&t)).collect();
    (stratum_ui::apps::load_all(), lists)
}

/// Run `argv` in a session of its own, waited for on a thread so it never
/// lingers as a zombie.
pub fn spawn(argv: &[String]) -> Result<(), String> {
    use std::os::unix::process::CommandExt;
    let (program, args) = argv.split_first().ok_or("no command")?;
    let mut command = std::process::Command::new(program);
    command.args(args).stdin(std::process::Stdio::null());
    // SAFETY: setsid is async-signal-safe.
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = command.spawn().map_err(|e| format!("{program}: {e}"))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Open `files` with `app`, through `terminal` when it runs in one.
pub fn open_with(app: &App, files: &[PathBuf], terminal: &[String]) -> Result<(), String> {
    let mut argv = app.command(files);
    if app.terminal {
        argv.splice(0..0, terminal.iter().cloned());
    }
    spawn(&argv)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str, mimes: &[&str]) -> App {
        stratum_ui::apps::parse(id, &format!("[Desktop Entry]\nName={id}\nExec={id} %f\nMimeType={};\n", mimes.join(";"))).unwrap()
    }

    #[test]
    fn defaults_then_associations_then_parents() {
        let mut db = MimeDb::default();
        db.add_parents("text/rust text/plain\n");
        db.index();
        let apps = vec![app("vim.desktop", &["text/plain"]), app("code.desktop", &["text/rust"]), app("gedit.desktop", &["text/plain"])];
        let list = parse_mimeapps("[Default Applications]\ntext/plain=gedit.desktop;\n[Removed Associations]\ntext/plain=vim.desktop;\n");
        let ids: Vec<String> = apps_for("text/rust", &db, &apps, &[list]).into_iter().map(|a| a.id).collect();
        assert_eq!(ids, ["code.desktop", "gedit.desktop"], "the type's own app, then the parent's default; vim removed");
    }
}
