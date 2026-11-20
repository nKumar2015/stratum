//! The freedesktop trash: `$XDG_DATA_HOME/Trash/files` holds what was
//! deleted and `info/<name>.trashinfo` where it came from and when, so any
//! file manager can list and restore it. Files on another file system
//! cannot be renamed into the home trash; those are refused rather than
//! copied (the caller offers to delete them for good instead).

use std::io::Write;
use std::path::{Path, PathBuf};

pub fn trash_dir() -> PathBuf {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share"));
    data.join("Trash")
}

pub fn files_dir() -> PathBuf {
    trash_dir().join("files")
}

fn info_dir() -> PathBuf {
    trash_dir().join("info")
}

/// Whether `path` is the trash's own file list (the view shows restore
/// and empty there).
pub fn is_trash(path: &Path) -> bool {
    path == files_dir()
}

#[derive(Clone, Debug, PartialEq)]
pub enum TrashError {
    /// On another file system than the home trash.
    OtherDevice,
    Io(String),
}

impl std::fmt::Display for TrashError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TrashError::OtherDevice => f.write_str("it is on another drive than the trash"),
            TrashError::Io(err) => f.write_str(err),
        }
    }
}

/// A name in the trash for `name` that is not taken: `name`, `name.2`, ...
fn free_name(name: &str) -> String {
    let taken = |n: &str| info_dir().join(format!("{n}.trashinfo")).exists() || files_dir().join(n).exists();
    if !taken(name) {
        return name.to_string();
    }
    (2..).map(|i| format!("{name}.{i}")).find(|n| !taken(n)).expect("some name is free")
}

/// The `.trashinfo` text for a file deleted from `original` at `when`.
pub fn info_text(original: &Path, when: chrono::NaiveDateTime) -> String {
    let path = stratum_ui::uri::encode(std::os::unix::ffi::OsStrExt::as_bytes(original.as_os_str()));
    format!("[Trash Info]\nPath={path}\nDeletionDate={}\n", when.format("%Y-%m-%dT%H:%M:%S"))
}

/// Move `path` to the trash.
pub fn trash(path: &Path) -> Result<(), TrashError> {
    let io = |e: std::io::Error| TrashError::Io(e.to_string());
    let original = std::path::absolute(path).map_err(io)?;
    let name = original.file_name().ok_or_else(|| TrashError::Io("no file name".into()))?.to_string_lossy().into_owned();
    std::fs::create_dir_all(files_dir()).map_err(io)?;
    std::fs::create_dir_all(info_dir()).map_err(io)?;
    let name = free_name(&name);
    let info_path = info_dir().join(format!("{name}.trashinfo"));
    // The info first, created exclusively, as the spec asks: it reserves
    // the name.
    let mut info = std::fs::OpenOptions::new().write(true).create_new(true).open(&info_path).map_err(io)?;
    info.write_all(info_text(&original, chrono::Local::now().naive_local()).as_bytes()).map_err(io)?;
    match std::fs::rename(&original, files_dir().join(&name)) {
        Ok(()) => Ok(()),
        Err(err) => {
            let _ = std::fs::remove_file(&info_path);
            if err.raw_os_error() == Some(libc::EXDEV) { Err(TrashError::OtherDevice) } else { Err(io(err)) }
        }
    }
}

/// Where a trashed file (by its name in `files/`) came from.
pub fn original(name: &str) -> Option<PathBuf> {
    let text = std::fs::read_to_string(info_dir().join(format!("{name}.trashinfo"))).ok()?;
    parse_info(&text)
}

pub fn parse_info(text: &str) -> Option<PathBuf> {
    let value = text.lines().find_map(|l| l.strip_prefix("Path="))?;
    let bytes = stratum_ui::uri::decode(value)?;
    Some(PathBuf::from(<std::ffi::OsString as std::os::unix::ffi::OsStringExt>::from_vec(bytes)))
}

/// Put a trashed file back where it was. Refused if something is there now.
pub fn restore(name: &str) -> Result<PathBuf, String> {
    let to = original(name).ok_or("its origin is not recorded")?;
    if to.exists() {
        return Err(format!("{} exists", to.display()));
    }
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::rename(files_dir().join(name), &to).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(info_dir().join(format!("{name}.trashinfo")));
    Ok(to)
}

/// Delete a trashed file for good.
pub fn delete(name: &str) -> std::io::Result<()> {
    crate::ops::remove(&files_dir().join(name))?;
    let _ = std::fs::remove_file(info_dir().join(format!("{name}.trashinfo")));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_round_trips() {
        let when = chrono::NaiveDate::from_ymd_opt(2026, 11, 20).unwrap().and_hms_opt(9, 5, 3).unwrap();
        let text = info_text(Path::new("/home/me/a b#.txt"), when);
        assert_eq!(text, "[Trash Info]\nPath=/home/me/a%20b%23.txt\nDeletionDate=2026-11-20T09:05:03\n");
        assert_eq!(parse_info(&text).as_deref(), Some(Path::new("/home/me/a b#.txt")));
    }
}
