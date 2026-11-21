//! The sidebar's places: home and the XDG user folders, the user's GTK
//! bookmarks (which Thunar and Nautilus share), mounted drives, and the
//! file system root.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub name: String,
    pub path: PathBuf,
    /// An icon name.
    pub icon: &'static str,
    /// The filesystem a Drives row is, from udisks. An unmounted one has
    /// no path yet: a click mounts it.
    pub drive: Option<DriveRef>,
}

/// Which udisks filesystem, and on which drive, a place is.
#[derive(Clone, Debug, PartialEq)]
pub struct DriveRef {
    pub volume: String,
    pub drive: String,
    pub drive_name: String,
    pub mounted: bool,
    /// Encrypted and not unlocked: a click asks for its passphrase.
    pub locked: bool,
    /// It can be ejected: a removable drive.
    pub removable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Places,
    Bookmarks,
    Drives,
}

pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home_dir().join(".config"))
}

/// `XDG_*_DIR="$HOME/..."` lines from `user-dirs.dirs`.
pub fn parse_user_dirs(text: &str, home: &Path) -> Vec<(String, PathBuf)> {
    text.lines()
        .filter_map(|line| {
            let (key, value) = line.trim().split_once('=')?;
            let name = key.strip_prefix("XDG_")?.strip_suffix("_DIR")?;
            let value = value.trim().trim_matches('"');
            let path = match value.strip_prefix("$HOME") {
                Some(rest) => home.join(rest.trim_start_matches('/')),
                None => PathBuf::from(value),
            };
            // A user dir set to $HOME itself means "none".
            (path != home).then(|| (name.to_string(), path))
        })
        .collect()
}

/// `file:///path Label` lines from the GTK bookmarks file.
pub fn parse_bookmarks(text: &str) -> Vec<Place> {
    text.lines()
        .filter_map(|line| {
            let (uri, label) = line.split_once(' ').map_or((line, None), |(u, l)| (u, Some(l)));
            let path = stratum_ui::uri::to_path(uri.trim())?;
            let name = label
                .map(str::to_string)
                .or_else(|| path.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_else(|| path.display().to_string());
            Some(Place { name, path, icon: "folder", drive: None })
        })
        .collect()
}

/// The GTK bookmarks file, which Thunar and Nautilus share.
pub fn bookmarks_file() -> PathBuf {
    config_home().join("gtk-3.0/bookmarks")
}

/// Whether a bookmarks line names `path` (labelled or not).
fn names(line: &str, path: &Path) -> bool {
    let uri = line.split_once(' ').map_or(line, |(u, _)| u).trim();
    stratum_ui::uri::to_path(uri).is_some_and(|p| p == path)
}

pub fn is_bookmarked(text: &str, path: &Path) -> bool {
    text.lines().any(|l| names(l, path))
}

/// The bookmarks text with `path` added at the end, unless it is there.
pub fn with_bookmark(text: &str, path: &Path) -> String {
    if is_bookmarked(text, path) {
        return text.to_string();
    }
    let mut out = text.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&stratum_ui::uri::from_path(path));
    out.push('\n');
    out
}

/// The bookmarks text without `path`; every other line kept as written.
pub fn without_bookmark(text: &str, path: &Path) -> String {
    text.lines().filter(|l| !names(l, path)).map(|l| format!("{l}\n")).collect()
}

/// Rewrite the bookmarks file through `change`. A file that is a link
/// (one Home Manager made, say) is not replaced: the change belongs in
/// whatever made it.
fn change_bookmarks(change: impl FnOnce(&str) -> String) -> Result<(), String> {
    let file = bookmarks_file();
    if std::fs::symlink_metadata(&file).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(format!("{} is a link, managed elsewhere", file.display()));
    }
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    let changed = change(&text);
    if changed == text {
        return Ok(());
    }
    let dir = file.parent().ok_or("no folder for the bookmarks")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    // Written beside it and renamed over it, so a crash never leaves half.
    let temp = dir.join(format!(".bookmarks-{}", std::process::id()));
    std::fs::write(&temp, changed).and_then(|()| std::fs::rename(&temp, &file)).map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        e.to_string()
    })
}

pub fn add_bookmark(path: &Path) -> Result<(), String> {
    change_bookmarks(|text| with_bookmark(text, path))
}

pub fn remove_bookmark(path: &Path) -> Result<(), String> {
    change_bookmarks(|text| without_bookmark(text, path))
}

/// Removable and user mounts from `/proc/self/mounts`: under
/// `/run/media/$USER`, `/media` or `/mnt`.
pub fn parse_mounts(text: &str, user: &str) -> Vec<Place> {
    let media = format!("/run/media/{user}/");
    text.lines()
        .filter_map(|line| {
            let mount = line.split_whitespace().nth(1)?;
            // Spaces in mount points are written as \040.
            let mount = mount.replace("\\040", " ");
            let ours = mount.starts_with(&media) || mount.starts_with("/media/") || mount.starts_with("/mnt/");
            ours.then(|| {
                let path = PathBuf::from(&mount);
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(mount);
                Place { name, path, icon: "drive-removable-media", drive: None }
            })
        })
        .collect()
}

/// Every place, by section.
pub fn load() -> Vec<(Section, Place)> {
    let home = home_dir();
    let mut out = vec![(Section::Places, Place { name: "Home".into(), path: home.clone(), icon: "user-home", drive: None })];
    let dirs = std::fs::read_to_string(config_home().join("user-dirs.dirs")).unwrap_or_default();
    let dirs = parse_user_dirs(&dirs, &home);
    for (key, label, icon) in [
        ("DESKTOP", "Desktop", "user-desktop"),
        ("DOCUMENTS", "Documents", "folder-documents"),
        ("DOWNLOAD", "Downloads", "folder-download"),
        ("MUSIC", "Music", "folder-music"),
        ("PICTURES", "Pictures", "folder-pictures"),
        ("VIDEOS", "Videos", "folder-videos"),
    ] {
        let path = dirs.iter().find(|(k, _)| k == key).map(|(_, p)| p.clone()).unwrap_or_else(|| home.join(label));
        if path.is_dir() {
            out.push((Section::Places, Place { name: label.into(), path, icon, drive: None }));
        }
    }
    out.push((Section::Places, Place { name: "Trash".into(), path: crate::trash::files_dir(), icon: "user-trash", drive: None }));
    let bookmarks = std::fs::read_to_string(bookmarks_file()).unwrap_or_default();
    out.extend(parse_bookmarks(&bookmarks).into_iter().map(|p| (Section::Bookmarks, p)));
    let mounts = std::fs::read_to_string("/proc/self/mounts").unwrap_or_default();
    let user = std::env::var("USER").unwrap_or_default();
    out.extend(parse_mounts(&mounts, &user).into_iter().map(|p| (Section::Drives, p)));
    out.push((Section::Drives, Place { name: "File system".into(), path: PathBuf::from("/"), icon: "drive-harddisk", drive: None }));
    out
}

/// The places with udisks' drives laid over the Drives section: a mounted
/// filesystem's row learns which drive it is (so it can be ejected), and
/// unmounted ones (and locked, encrypted ones) are added before "File
/// system".
pub fn with_drives(mut places: Vec<(Section, Place)>, drives: &[stratum_drives::Drive]) -> Vec<(Section, Place)> {
    let root = places.iter().position(|(s, p)| *s == Section::Drives && p.path == Path::new("/"));
    let mut added = Vec::new();
    for drive in drives {
        for volume in &drive.volumes {
            let mounted = volume.mounted().map(Path::to_path_buf);
            let reference = DriveRef {
                volume: volume.object.clone(),
                drive: drive.object.clone(),
                drive_name: drive.name.clone(),
                mounted: mounted.is_some(),
                locked: volume.locked,
                removable: drive.removable,
            };
            let existing = mounted
                .as_ref()
                .and_then(|m| places.iter_mut().find(|(s, p)| *s == Section::Drives && &p.path == m));
            match existing {
                Some((_, place)) => place.drive = Some(reference),
                None => added.push((
                    Section::Drives,
                    Place {
                        name: volume.name(),
                        path: mounted.unwrap_or_default(),
                        icon: if drive.removable { "drive-removable-media" } else { "drive-harddisk" },
                        drive: Some(reference),
                    },
                )),
            }
        }
    }
    let at = root.unwrap_or(places.len());
    places.splice(at..at, added);
    places
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_dirs() {
        let text = "# comment\nXDG_DESKTOP_DIR=\"$HOME/Desktop\"\nXDG_MUSIC_DIR=\"/data/music\"\nXDG_TEMPLATES_DIR=\"$HOME/\"\n";
        let dirs = parse_user_dirs(text, Path::new("/home/me"));
        assert_eq!(dirs, [("DESKTOP".to_string(), PathBuf::from("/home/me/Desktop")), ("MUSIC".to_string(), PathBuf::from("/data/music"))]);
    }

    #[test]
    fn bookmarks_are_added_once_and_removed_alone() {
        let text = "file:///home/me/My%20Code Code\nsmb://server/x\nfile:///srv/share";
        let added = with_bookmark(text, Path::new("/home/me/new dir"));
        assert_eq!(added, "file:///home/me/My%20Code Code\nsmb://server/x\nfile:///srv/share\nfile:///home/me/new%20dir\n");
        assert_eq!(with_bookmark(&added, Path::new("/home/me/new dir")), added, "not twice");
        assert_eq!(with_bookmark("", Path::new("/a")), "file:///a\n");
        // A labelled one goes with its label; the rest stay as written.
        let removed = without_bookmark(&added, Path::new("/home/me/My Code"));
        assert_eq!(removed, "smb://server/x\nfile:///srv/share\nfile:///home/me/new%20dir\n");
        assert!(is_bookmarked(&removed, Path::new("/srv/share")));
        assert!(!is_bookmarked(&removed, Path::new("/home/me/My Code")));
    }

    #[test]
    fn bookmarks_and_mounts() {
        let marks = parse_bookmarks("file:///home/me/My%20Code Code\nfile:///srv/share\nsmb://server/x\n");
        assert_eq!(marks.len(), 2);
        assert_eq!((marks[0].name.as_str(), marks[0].path.as_path()), ("Code", Path::new("/home/me/My Code")));
        assert_eq!(marks[1].name, "share");
        let mounts = "/dev/nvme0n1p2 / ext4 rw 0 0\n/dev/sdb1 /run/media/me/USB\\040Stick vfat rw 0 0\n/dev/sdc1 /run/media/other/X vfat rw 0 0\n";
        let drives = parse_mounts(mounts, "me");
        assert_eq!(drives.len(), 1);
        assert_eq!(drives[0].name, "USB Stick");
    }

    #[test]
    fn drives_join_the_drives_section() {
        use stratum_drives::{Drive, Volume};
        let volume = |object: &str, label: &str, mounted: Option<&str>| Volume {
            object: object.into(),
            device: String::new(),
            label: label.into(),
            fs_type: "exfat".into(),
            size: 8_000_000_000,
            mount_points: mounted.map(PathBuf::from).into_iter().collect(),
            read_only: false,
            system: false,
            locked: false,
            backing: None,
        };
        let mut locked = volume("sdb3", "", None);
        locked.locked = true;
        let drive = Drive {
            object: "stick".into(),
            name: "SanDisk Cruzer".into(),
            removable: true,
            can_power_off: true,
            ejectable: false,
            optical: false,
            bus: "usb".into(),
            size: 16_000_000_000,
            volumes: vec![volume("sdb1", "STICK", Some("/run/media/me/STICK")), volume("sdb2", "", None), locked],
        };
        let places = vec![
            (Section::Places, Place { name: "Home".into(), path: "/home/me".into(), icon: "user-home", drive: None }),
            (Section::Drives, Place { name: "STICK".into(), path: "/run/media/me/STICK".into(), icon: "drive-removable-media", drive: None }),
            (Section::Drives, Place { name: "File system".into(), path: "/".into(), icon: "drive-harddisk", drive: None }),
        ];
        let merged = with_drives(places, &[drive]);
        let names: Vec<_> = merged.iter().map(|(_, p)| p.name.as_str()).collect();
        assert_eq!(names, ["Home", "STICK", "8.0 GB Volume", "8.0 GB Volume", "File system"]);
        assert!(merged[3].1.drive.as_ref().is_some_and(|d| d.locked && !d.mounted));
        let stick = merged[1].1.drive.as_ref().expect("the mounted row knows its drive");
        assert!(stick.mounted && stick.removable);
        assert_eq!(stick.drive, "stick");
        let unmounted = &merged[2].1;
        assert_eq!(unmounted.path, PathBuf::new());
        assert_eq!(unmounted.drive.as_ref().map(|d| (d.volume.as_str(), d.mounted)), Some(("sdb2", false)));
    }
}
