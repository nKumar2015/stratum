//! The freedesktop trash. A trash directory holds `files/` (what was
//! deleted) and `info/<name>.trashinfo` (where it came from and when), so
//! any file manager can list and restore it.
//!
//! Files on the home's file system go to `$XDG_DATA_HOME/Trash`. A file
//! that cannot be renamed there (another drive) goes to its own file
//! system's top-directory trash, as the spec's "top directories" section
//! says: `$topdir/.Trash/$uid` when an administrator made a sticky
//! `.Trash`, else `$topdir/.Trash-$uid`. Their info records the path
//! relative to `$topdir`, so a drive mounted elsewhere still restores.
//! The Trash view lists every trash on every mounted file system.

use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Path, PathBuf};

/// One trash directory.
#[derive(Clone, Debug, PartialEq)]
pub struct Trash {
    /// The directory holding `files/` and `info/`.
    pub root: PathBuf,
    /// The top directory a top-directory trash belongs to, which its
    /// relative `Path=` lines are joined to; `None` for the home trash,
    /// whose paths are absolute.
    pub topdir: Option<PathBuf>,
}

impl Trash {
    pub fn home() -> Self {
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share"));
        Self { root: data.join("Trash"), topdir: None }
    }

    pub fn files(&self) -> PathBuf {
        self.root.join("files")
    }

    fn info(&self) -> PathBuf {
        self.root.join("info")
    }

    fn info_path(&self, name: &str) -> PathBuf {
        self.info().join(format!("{name}.trashinfo"))
    }

    /// The trash an item in some trash's `files/` belongs to.
    pub fn of(item: &Path) -> Option<Self> {
        let files = item.parent()?;
        if files.file_name()? != "files" {
            return None;
        }
        let root = files.parent()?.to_path_buf();
        if root == Self::home().root {
            return Some(Self::home());
        }
        let topdir = topdir_of_trash(&root, uid())?;
        Some(Self { root, topdir: Some(topdir) })
    }

    /// A name in this trash for `name` that is not taken: `name`,
    /// `name.2`, ...
    fn free_name(&self, name: &str) -> String {
        let taken = |n: &str| self.info_path(n).exists() || self.files().join(n).exists();
        if !taken(name) {
            return name.to_string();
        }
        (2..).map(|i| format!("{name}.{i}")).find(|n| !taken(n)).expect("some name is free")
    }

    /// The path recorded for `original`: absolute in the home trash,
    /// relative to the top directory in a top-directory trash.
    fn recorded(&self, original: &Path) -> PathBuf {
        match &self.topdir {
            Some(top) => original.strip_prefix(top).map(Path::to_path_buf).unwrap_or_else(|_| original.to_path_buf()),
            None => original.to_path_buf(),
        }
    }

    /// Where an item came from, resolving a relative record.
    fn resolve(&self, recorded: PathBuf) -> PathBuf {
        match &self.topdir {
            Some(top) if recorded.is_relative() => top.join(recorded),
            _ => recorded,
        }
    }

    /// Move `original` (absolute) into this trash, returning where it
    /// now is.
    fn put(&self, original: &Path) -> Result<PathBuf, TrashError> {
        let name = original.file_name().ok_or_else(|| TrashError::Io("no file name".into()))?.to_string_lossy().into_owned();
        for dir in [self.files(), self.info()] {
            std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir).map_err(io)?;
        }
        let name = self.free_name(&name);
        let info_path = self.info_path(&name);
        // The info first, created exclusively, as the spec asks: it
        // reserves the name.
        let mut info = std::fs::OpenOptions::new().write(true).create_new(true).open(&info_path).map_err(io)?;
        info.write_all(info_text(&self.recorded(original), chrono::Local::now().naive_local()).as_bytes()).map_err(io)?;
        let to = self.files().join(&name);
        match std::fs::rename(original, &to) {
            Ok(()) => Ok(to),
            Err(err) => {
                let _ = std::fs::remove_file(&info_path);
                if err.raw_os_error() == Some(libc::EXDEV) { Err(TrashError::OtherDevice) } else { Err(io(err)) }
            }
        }
    }
}

fn io(e: std::io::Error) -> TrashError {
    TrashError::Io(e.to_string())
}

fn uid() -> u32 {
    // SAFETY: getuid cannot fail.
    unsafe { libc::getuid() }
}

/// The home trash's file list: the Trash place, which the view shows as
/// every trash merged.
pub fn files_dir() -> PathBuf {
    Trash::home().files()
}

/// Whether `path` is the Trash place (the view shows restore and empty
/// there).
pub fn is_trash(path: &Path) -> bool {
    path == files_dir()
}

/// The top directory a top-directory trash at `root` serves:
/// `$topdir/.Trash-$uid` or `$topdir/.Trash/$uid`.
fn topdir_of_trash(root: &Path, uid: u32) -> Option<PathBuf> {
    let name = root.file_name()?.to_str()?;
    if name == format!(".Trash-{uid}") {
        return root.parent().map(Path::to_path_buf);
    }
    let admin = root.parent()?;
    (name == uid.to_string() && admin.file_name()? == ".Trash").then(|| admin.parent().map(Path::to_path_buf))?
}

/// The mount point `dir` is under: the highest ancestor on the same
/// device. `dev` reads a path's device (`None` ends the walk).
pub fn mount_point(dir: &Path, dev: impl Fn(&Path) -> Option<u64>) -> PathBuf {
    let Some(own) = dev(dir) else { return dir.to_path_buf() };
    let mut top = dir;
    while let Some(parent) = top.parent() {
        if dev(parent) != Some(own) {
            break;
        }
        top = parent;
    }
    top.to_path_buf()
}

/// A directory we may keep a trash in: a real directory (not a link to
/// one) that is ours.
fn usable_dir(path: &Path, uid: u32) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir() && m.uid() == uid)
}

/// The trash for files under `topdir`, made if missing: the
/// administrator's `.Trash/$uid` when `.Trash` is a sticky directory and
/// not a link, else our own `.Trash-$uid`. `None` when neither can be
/// used (a read-only drive, or one where someone else owns the name).
pub fn topdir_trash(topdir: &Path, uid: u32) -> Option<Trash> {
    let make = |path: &Path| {
        let _ = std::fs::DirBuilder::new().mode(0o700).create(path);
        usable_dir(path, uid)
    };
    let admin = topdir.join(".Trash");
    let sticky = std::fs::symlink_metadata(&admin).is_ok_and(|m| m.is_dir() && m.mode() & libc::S_ISVTX != 0);
    let mine = admin.join(uid.to_string());
    if sticky && make(&mine) {
        return Some(Trash { root: mine, topdir: Some(topdir.to_path_buf()) });
    }
    let own = topdir.join(format!(".Trash-{uid}"));
    make(&own).then(|| Trash { root: own, topdir: Some(topdir.to_path_buf()) })
}

#[derive(Clone, Debug, PartialEq)]
pub enum TrashError {
    /// On a drive with no usable trash.
    OtherDevice,
    Io(String),
}

impl std::fmt::Display for TrashError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TrashError::OtherDevice => f.write_str("its drive has no trash that can be used"),
            TrashError::Io(err) => f.write_str(err),
        }
    }
}

/// The `.trashinfo` text for a file deleted from `original` at `when`.
pub fn info_text(original: &Path, when: chrono::NaiveDateTime) -> String {
    let path = stratum_ui::uri::encode(std::os::unix::ffi::OsStrExt::as_bytes(original.as_os_str()));
    format!("[Trash Info]\nPath={path}\nDeletionDate={}\n", when.format("%Y-%m-%dT%H:%M:%S"))
}

/// Move `path` to the trash: the home trash, or its own drive's when it
/// is on another. Returns where it now is, inside a trash's `files/`.
pub fn trash(path: &Path) -> Result<PathBuf, TrashError> {
    let original = std::path::absolute(path).map_err(io)?;
    match Trash::home().put(&original) {
        Err(TrashError::OtherDevice) => {}
        done => return done,
    }
    // The real folder, not one reached through a link: walking up a
    // linked path would find a "top" inside the drive, where the Trash
    // view never looks. The file itself may be a link, so it keeps its
    // name.
    let (Some(parent), Some(name)) = (original.parent(), original.file_name()) else {
        return Err(TrashError::OtherDevice);
    };
    let parent = std::fs::canonicalize(parent).map_err(io)?;
    let topdir = mount_point(&parent, |p| std::fs::metadata(p).ok().map(|m| m.dev()));
    topdir_trash(&topdir, uid()).ok_or(TrashError::OtherDevice)?.put(&parent.join(name))
}

/// Where a trashed item (its path in a trash's `files/`) came from.
pub fn original(item: &Path) -> Option<PathBuf> {
    let trash = Trash::of(item)?;
    let text = std::fs::read_to_string(trash.info_path(&item.file_name()?.to_string_lossy())).ok()?;
    Some(trash.resolve(parse_info(&text)?))
}

pub fn parse_info(text: &str) -> Option<PathBuf> {
    let value = text.lines().find_map(|l| l.strip_prefix("Path="))?;
    let bytes = stratum_ui::uri::decode(value)?;
    Some(PathBuf::from(<std::ffi::OsString as std::os::unix::ffi::OsStringExt>::from_vec(bytes)))
}

/// Put a trashed item back where it was. Refused if something is there
/// now.
pub fn restore(item: &Path) -> Result<PathBuf, String> {
    let trash = Trash::of(item).ok_or("it is not in a trash")?;
    let to = original(item).ok_or("its origin is not recorded")?;
    if to.symlink_metadata().is_ok() {
        return Err(format!("{} exists", to.display()));
    }
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::rename(item, &to).map_err(|e| e.to_string())?;
    if let Some(name) = item.file_name() {
        let _ = std::fs::remove_file(trash.info_path(&name.to_string_lossy()));
    }
    Ok(to)
}

/// Delete a trashed item for good.
pub fn delete(item: &Path) -> std::io::Result<()> {
    crate::ops::remove(item)?;
    if let (Some(trash), Some(name)) = (Trash::of(item), item.file_name()) {
        let _ = std::fs::remove_file(trash.info_path(&name.to_string_lossy()));
    }
    Ok(())
}

/// File systems that never hold a trash, which listing skips: kernel
/// interfaces, and `autofs`, whose mount points mount a drive (or wait on
/// a server) when looked into.
const NO_TRASH: &[&str] = &[
    "autofs", "binfmt_misc", "bpf", "cgroup", "cgroup2", "configfs", "debugfs", "devpts", "devtmpfs", "efivarfs", "fusectl", "hugetlbfs",
    "mqueue", "nsfs", "proc", "pstore", "ramfs", "securityfs", "sysfs", "tracefs",
];

/// Mount points from `/proc/self/mounts` text that may hold a trash.
pub fn mount_points(text: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = text
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let mount = fields.nth(1)?;
            let kind = fields.next()?;
            (!NO_TRASH.contains(&kind)).then(|| PathBuf::from(unescape_mount(mount)))
        })
        .collect();
    out.dedup();
    out
}

/// `/proc/self/mounts` writes space, tab, newline and backslash in octal.
fn unescape_mount(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let octal = bytes.get(i + 1..i + 4).filter(|d| bytes[i] == b'\\' && d.iter().all(|c| (b'0'..=b'7').contains(c)));
        match octal {
            Some(d) => {
                out.push((d[0] - b'0') * 64 + (d[1] - b'0') * 8 + (d[2] - b'0'));
                i += 4;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Every trash that exists: the home one, then each mounted file
/// system's top-directory trashes. Nothing is created.
pub fn all() -> Vec<Trash> {
    let uid = uid();
    let mut out = vec![Trash::home()];
    let mounts = std::fs::read_to_string("/proc/self/mounts").unwrap_or_default();
    for top in mount_points(&mounts) {
        for root in [top.join(".Trash").join(uid.to_string()), top.join(format!(".Trash-{uid}"))] {
            if usable_dir(&root, uid) && !out.iter().any(|t| t.root == root) {
                out.push(Trash { root, topdir: Some(top.clone()) });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("stratum-trash-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn info_round_trips() {
        let when = chrono::NaiveDate::from_ymd_opt(2026, 11, 20).unwrap().and_hms_opt(9, 5, 3).unwrap();
        let text = info_text(Path::new("/home/me/a b#.txt"), when);
        assert_eq!(text, "[Trash Info]\nPath=/home/me/a%20b%23.txt\nDeletionDate=2026-11-20T09:05:03\n");
        assert_eq!(parse_info(&text).as_deref(), Some(Path::new("/home/me/a b#.txt")));
    }

    #[test]
    fn the_mount_point_is_the_highest_folder_on_the_same_device() {
        // /run/media/me/USB and below are device 2; everything else 1.
        let dev = |p: &Path| Some(if p.starts_with("/run/media/me/USB") { 2 } else { 1 });
        assert_eq!(mount_point(Path::new("/run/media/me/USB/photos/2026"), dev), Path::new("/run/media/me/USB"));
        assert_eq!(mount_point(Path::new("/home/me/Documents"), dev), Path::new("/"));
        // An unreadable parent ends the walk at the last readable folder.
        let dev = |p: &Path| (p != Path::new("/srv")).then_some(3);
        assert_eq!(mount_point(Path::new("/srv/data/x"), dev), Path::new("/srv/data"));
    }

    #[test]
    fn top_directory_records_are_relative() {
        let trash = Trash { root: "/mnt/usb/.Trash-1000".into(), topdir: Some("/mnt/usb".into()) };
        let recorded = trash.recorded(Path::new("/mnt/usb/photos/a.jpg"));
        assert_eq!(recorded, Path::new("photos/a.jpg"));
        assert_eq!(trash.resolve(recorded), Path::new("/mnt/usb/photos/a.jpg"));
        // An absolute record (another file manager may write one) stays.
        assert_eq!(trash.resolve("/elsewhere/b".into()), Path::new("/elsewhere/b"));
        let home = Trash { root: "/home/me/.local/share/Trash".into(), topdir: None };
        assert_eq!(home.recorded(Path::new("/home/me/a")), Path::new("/home/me/a"));
    }

    #[test]
    fn a_trash_knows_its_top_directory() {
        assert_eq!(topdir_of_trash(Path::new("/mnt/usb/.Trash-1000"), 1000).as_deref(), Some(Path::new("/mnt/usb")));
        assert_eq!(topdir_of_trash(Path::new("/mnt/usb/.Trash/1000"), 1000).as_deref(), Some(Path::new("/mnt/usb")));
        assert_eq!(topdir_of_trash(Path::new("/mnt/usb/.Trash-1001"), 1000), None);
        assert_eq!(topdir_of_trash(Path::new("/mnt/usb/stuff/1000"), 1000), None);
    }

    #[test]
    fn the_administrator_trash_is_used_only_when_sticky() {
        use std::os::unix::fs::PermissionsExt;
        let uid = uid();
        let top = scratch("admin");
        std::fs::create_dir(top.join(".Trash")).unwrap();
        // Not sticky: our own .Trash-$uid instead.
        let trash = topdir_trash(&top, uid).unwrap();
        assert_eq!(trash.root, top.join(format!(".Trash-{uid}")));
        let mode = std::fs::metadata(&trash.root).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
        std::fs::set_permissions(top.join(".Trash"), std::fs::Permissions::from_mode(0o1777)).unwrap();
        let trash = topdir_trash(&top, uid).unwrap();
        assert_eq!(trash.root, top.join(".Trash").join(uid.to_string()));
        assert_eq!(trash.topdir.as_deref(), Some(top.as_path()));
        // A link named .Trash-$uid is never followed.
        let other = scratch("link");
        std::os::unix::fs::symlink(&top, other.join(format!(".Trash-{uid}"))).unwrap();
        assert_eq!(topdir_trash(&other, uid), None);
        let _ = std::fs::remove_dir_all(&top);
        let _ = std::fs::remove_dir_all(&other);
    }

    #[test]
    fn put_restore_and_delete_in_a_top_directory_trash() {
        let top = scratch("put");
        std::fs::create_dir_all(top.join("photos")).unwrap();
        std::fs::write(top.join("photos/a.jpg"), b"a").unwrap();
        let trash = topdir_trash(&top, uid()).unwrap();
        let item = trash.put(&top.join("photos/a.jpg")).unwrap();
        assert_eq!(item, trash.files().join("a.jpg"));
        let info = std::fs::read_to_string(trash.info_path("a.jpg")).unwrap();
        assert!(info.contains("\nPath=photos/a.jpg\n"), "{info}");
        assert_eq!(Trash::of(&item), Some(trash.clone()));
        assert_eq!(original(&item).as_deref(), Some(top.join("photos/a.jpg").as_path()));
        // A second file of the same name gets its own.
        std::fs::write(top.join("photos/a.jpg"), b"b").unwrap();
        let second = trash.put(&top.join("photos/a.jpg")).unwrap();
        assert_eq!(second, trash.files().join("a.jpg.2"));
        // Restoring over a file that came back is refused.
        std::fs::write(top.join("photos/a.jpg"), b"c").unwrap();
        assert!(restore(&item).is_err());
        std::fs::remove_file(top.join("photos/a.jpg")).unwrap();
        assert_eq!(restore(&item).unwrap(), top.join("photos/a.jpg"));
        assert!(!trash.info_path("a.jpg").exists());
        delete(&second).unwrap();
        assert!(!second.exists() && !trash.info_path("a.jpg.2").exists());
        let _ = std::fs::remove_dir_all(&top);
    }

    #[test]
    fn listing_skips_kernel_file_systems() {
        let text = "proc /proc proc rw 0 0\n/dev/sda1 / btrfs rw 0 0\nsystemd-1 /mnt/nas autofs rw 0 0\n//nas/x /mnt/nas cifs rw 0 0\n/dev/sdb1 /run/media/me/My\\040Stick vfat rw 0 0\n";
        assert_eq!(mount_points(text), [PathBuf::from("/"), "/mnt/nas".into(), "/run/media/me/My Stick".into()]);
        assert_eq!(unescape_mount("a\\134b\\011c"), "a\\b\tc");
    }
}
