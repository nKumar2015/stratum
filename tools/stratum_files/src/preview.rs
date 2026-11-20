//! The preview pane: what it shows of the selection, and the loading
//! behind it.
//!
//! A picture is decoded at the pane's size, text read up to 16 KB, and a
//! folder's size added up, all on one thread of the pane's own, and only
//! for what is selected now: a newer selection stops an older folder walk.
//! The pane holds one decoded picture at most, dropped when the selection
//! changes.

use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use stratum_ui::app::Waker;
use stratum_ui::icons::IconImage;

/// How much of a text file is shown.
pub const TEXT_BYTES: usize = 16 * 1024;
/// A folder walk stops after this many entries or this long, and says
/// "more than".
const WALK_ENTRIES: u64 = 200_000;
const WALK_TIME: Duration = Duration::from_secs(3);

/// What the pane's top shows.
#[derive(Clone)]
pub enum Visual {
    /// Being loaded.
    Pending,
    /// A picture: decoded at the pane's size, and its own size in pixels.
    Picture { image: IconImage, width: u32, height: u32 },
    Text { text: String, more: bool },
    /// The type's icon (a folder, or anything else).
    Icon,
}

/// A folder's contents, added up.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FolderSize {
    /// Entries directly inside it.
    pub items: u64,
    /// Bytes of every file below it, links not followed.
    pub bytes: u64,
    /// False when the walk was cut short.
    pub complete: bool,
}

/// What a request asks the thread for.
enum Job {
    Picture { path: PathBuf, px: u32 },
    Text(PathBuf),
    Folder(PathBuf),
}

enum Loaded {
    Visual(Visual),
    Folder(FolderSize),
}

/// The pane's state for the current selection.
pub struct Preview {
    /// What it is showing: the selected paths, in order.
    pub showing: Vec<PathBuf>,
    pub visual: Visual,
    pub folder: Option<FolderSize>,
    generation: Arc<AtomicU64>,
    tx: Option<std::sync::mpsc::Sender<(u64, Job)>>,
    done: Arc<Mutex<Vec<(u64, Loaded)>>>,
}

impl Default for Preview {
    fn default() -> Self {
        Self { showing: Vec::new(), visual: Visual::Icon, folder: None, generation: Arc::default(), tx: None, done: Arc::default() }
    }
}

/// How a selected file is previewed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Picture,
    Text,
    Folder,
    Icon,
}

impl Preview {
    /// Show `paths`, starting whatever has to be loaded. `kind` is how the
    /// single selected item is shown, `px` the picture's size in pixels.
    pub fn show(&mut self, paths: Vec<PathBuf>, kind: Kind, px: u32, waker: Option<&Waker>) {
        // A new generation: anything still loading for the last one is
        // dropped when it arrives, and a folder walk stops.
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        self.visual = Visual::Icon;
        self.folder = None;
        let job = match (paths.as_slice(), kind) {
            ([one], Kind::Picture) => Some(Job::Picture { path: one.clone(), px }),
            ([one], Kind::Text) => Some(Job::Text(one.clone())),
            ([one], Kind::Folder) => Some(Job::Folder(one.clone())),
            _ => None,
        };
        self.showing = paths;
        let Some(job) = job else { return };
        if !matches!(job, Job::Folder(_)) {
            self.visual = Visual::Pending;
        }
        let tx = self.tx.get_or_insert_with(|| spawn(Arc::clone(&self.generation), Arc::clone(&self.done), waker.cloned()));
        let _ = tx.send((generation, job));
    }

    /// Take what the thread finished. True when something changed.
    pub fn take(&mut self) -> bool {
        let done: Vec<_> = self.done.lock().map(|mut d| std::mem::take(&mut *d)).unwrap_or_default();
        let current = self.generation.load(Ordering::Relaxed);
        let mut any = false;
        for (generation, loaded) in done {
            if generation != current {
                continue;
            }
            any = true;
            match loaded {
                Loaded::Visual(visual) => self.visual = visual,
                Loaded::Folder(size) => self.folder = Some(size),
            }
        }
        any
    }

    /// Changes with every `show`: a texture key for the picture.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    /// Forget what is shown, so the next `show` reloads it (the folder
    /// changed on disk).
    pub fn forget(&mut self) {
        self.showing.clear();
    }

    /// Drop everything shown and stop any loading (the pane was hidden).
    pub fn clear(&mut self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.showing.clear();
        self.visual = Visual::Icon;
        self.folder = None;
    }
}

fn spawn(generation: Arc<AtomicU64>, done: Arc<Mutex<Vec<(u64, Loaded)>>>, waker: Option<Waker>) -> std::sync::mpsc::Sender<(u64, Job)> {
    let (tx, rx) = std::sync::mpsc::channel::<(u64, Job)>();
    let spawned = std::thread::Builder::new().name("stratum-preview".into()).spawn(move || {
        for (asked, job) in rx {
            // Only the newest request matters; skip the ones overtaken.
            if asked != generation.load(Ordering::Relaxed) {
                continue;
            }
            let loaded = match job {
                Job::Picture { path, px } => {
                    Loaded::Visual(picture(&path, px).map_or(Visual::Icon, |(image, width, height)| Visual::Picture { image, width, height }))
                }
                Job::Text(path) => Loaded::Visual(read_text(&path).map_or(Visual::Icon, |(text, more)| Visual::Text { text, more })),
                Job::Folder(path) => Loaded::Folder(folder_size(&path, || asked != generation.load(Ordering::Relaxed))),
            };
            if let Ok(mut done) = done.lock() {
                done.push((asked, loaded));
            }
            if let Some(waker) = &waker {
                waker.wake();
            }
        }
    });
    if let Err(err) = spawned {
        tracing::warn!("no preview thread: {err}");
    }
    tx
}

/// A picture decoded and scaled down to fit `px`, premultiplied, with its
/// own size.
pub fn picture(path: &Path, px: u32) -> Option<(IconImage, u32, u32)> {
    let image = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.decode().ok()?;
    let (width, height) = (image.width(), image.height());
    let image = if width > px || height > px { image.thumbnail(px, px) } else { image };
    let image = image.to_rgba8();
    let (w, h) = image.dimensions();
    let mut pixels = image.into_raw();
    for px in pixels.chunks_exact_mut(4) {
        let a = u16::from(px[3]);
        for c in &mut px[..3] {
            *c = (u16::from(*c) * a / 255) as u8;
        }
    }
    Some((IconImage { width: w, height: h, pixels: pixels.into() }, width, height))
}

/// The start of a text file, and whether there is more. `None` for one
/// that is not text after all (a NUL byte).
pub fn read_text(path: &Path) -> Option<(String, bool)> {
    use std::io::Read;
    let mut bytes = Vec::with_capacity(TEXT_BYTES + 1);
    std::fs::File::open(path).ok()?.take(TEXT_BYTES as u64 + 1).read_to_end(&mut bytes).ok()?;
    let more = bytes.len() > TEXT_BYTES;
    bytes.truncate(TEXT_BYTES);
    text_of(&bytes).map(|text| (text, more))
}

/// Bytes as text: a character cut at the end is dropped, other invalid
/// sequences shown as U+FFFD; anything with a NUL is not text.
pub fn text_of(bytes: &[u8]) -> Option<String> {
    if bytes.contains(&0) {
        return None;
    }
    let end = match std::str::from_utf8(bytes) {
        Ok(_) => bytes.len(),
        // Cut inside a character at the very end.
        Err(err) if err.error_len().is_none() => err.valid_up_to(),
        Err(_) => bytes.len(),
    };
    Some(String::from_utf8_lossy(&bytes[..end]).into_owned())
}

/// Add up a folder, giving up past `WALK_ENTRIES` entries, `WALK_TIME`,
/// or when `stop` says so.
pub fn folder_size(dir: &Path, stop: impl Fn() -> bool) -> FolderSize {
    let started = Instant::now();
    let mut size = FolderSize { items: std::fs::read_dir(dir).map(|it| it.count() as u64).unwrap_or(0), bytes: 0, complete: true };
    let mut seen = 0u64;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            seen += 1;
            if seen.is_multiple_of(1024) && (stop() || started.elapsed() > WALK_TIME) || seen > WALK_ENTRIES {
                size.complete = false;
                return size;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                stack.push(entry.path());
            } else {
                size.bytes += meta.len();
            }
        }
    }
    size
}

/// The comment shared-mime-info gives a type ("PNG image"), from its
/// `mime/<type>.xml` in the data directories; `None` when it has none.
pub fn comment(mime: &str) -> Option<String> {
    if mime.contains("..") {
        return None;
    }
    stratum_ui::icons::data_dirs().iter().find_map(|dir| {
        let xml = std::fs::read_to_string(dir.join("mime").join(format!("{mime}.xml"))).ok()?;
        parse_comment(&xml)
    })
}

/// The untranslated `<comment>` in a type's XML.
pub fn parse_comment(xml: &str) -> Option<String> {
    let start = xml.find("<comment>")? + "<comment>".len();
    let end = start + xml[start..].find("</comment>")?;
    let text = &xml[start..end];
    Some(text.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&"))
}

/// `rwxr-xr-x`, with set-id and sticky bits as `ls` writes them.
pub fn permissions(mode: u32) -> String {
    let bit = |mask: u32, ch: char| if mode & mask != 0 { ch } else { '-' };
    let exec = |x: u32, special: u32, on: char, off: char| match (mode & x != 0, mode & special != 0) {
        (true, true) => on,
        (false, true) => off,
        (true, false) => 'x',
        (false, false) => '-',
    };
    [
        bit(0o400, 'r'),
        bit(0o200, 'w'),
        exec(0o100, 0o4000, 's', 'S'),
        bit(0o040, 'r'),
        bit(0o020, 'w'),
        exec(0o010, 0o2000, 's', 'S'),
        bit(0o004, 'r'),
        bit(0o002, 'w'),
        exec(0o001, 0o1000, 't', 'T'),
    ]
    .iter()
    .collect()
}

/// A user's name, or their number when they have none.
pub fn user_name(uid: u32) -> String {
    let mut buffer = vec![0 as libc::c_char; 4096];
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut found: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: every pointer is to memory owned here, sized as passed.
    let ok = unsafe { libc::getpwuid_r(uid, &mut entry, buffer.as_mut_ptr(), buffer.len(), &mut found) } == 0;
    if ok && !found.is_null() {
        // SAFETY: getpwuid_r succeeded, so pw_name points into `buffer`.
        let name = unsafe { std::ffi::CStr::from_ptr(entry.pw_name) };
        return name.to_string_lossy().into_owned();
    }
    uid.to_string()
}

/// What is known of one item without reading it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facts {
    pub is_dir: bool,
    /// The type's comment, else the type.
    pub kind: String,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub mode: u32,
    pub owner: String,
    /// Where a link points.
    pub link: Option<PathBuf>,
}

impl Facts {
    pub fn read(path: &Path, mime: &str) -> Self {
        let link = std::fs::read_link(path).ok();
        let meta = std::fs::metadata(path).or_else(|_| std::fs::symlink_metadata(path)).ok();
        Facts {
            is_dir: meta.as_ref().is_some_and(|m| m.is_dir()),
            kind: comment(mime).unwrap_or_else(|| mime.to_string()),
            size: meta.as_ref().map_or(0, |m| m.len()),
            modified: meta.as_ref().and_then(|m| m.modified().ok()),
            mode: meta.as_ref().map_or(0, |m| m.permissions().mode()),
            owner: meta.as_ref().map(|m| user_name(m.uid())).unwrap_or_default(),
            link,
        }
    }
}

/// The pane's information rows for one item: the folder's size once
/// added up, a picture's pixels once decoded.
pub fn info_lines(facts: &Facts, folder: Option<FolderSize>, pixels: Option<(u32, u32)>, now: chrono::DateTime<chrono::Local>) -> Vec<(&'static str, String)> {
    use crate::entries::{format_size, format_time};
    let mut lines = vec![("Type", facts.kind.clone())];
    if facts.is_dir {
        match folder {
            Some(size) => {
                let more = if size.complete { "" } else { "more than " };
                lines.push(("Contains", crate::browser::count(size.items as usize, "item")));
                lines.push(("Size", format!("{more}{}", format_size(size.bytes))));
            }
            None => lines.push(("Size", "\u{2026}".into())),
        }
    } else {
        lines.push(("Size", format_size(facts.size)));
    }
    if let Some((w, h)) = pixels {
        lines.push(("Dimensions", format!("{w} \u{d7} {h}")));
    }
    if let Some(when) = facts.modified {
        lines.push(("Modified", format_time(when, now)));
    }
    lines.push(("Permissions", permissions(facts.mode)));
    if !facts.owner.is_empty() {
        lines.push(("Owner", facts.owner.clone()));
    }
    if let Some(target) = &facts.link {
        lines.push(("Link to", target.display().to_string()));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permissions_read_as_ls_writes_them() {
        assert_eq!(permissions(0o755), "rwxr-xr-x");
        assert_eq!(permissions(0o640), "rw-r-----");
        assert_eq!(permissions(0o4755), "rwsr-xr-x");
        assert_eq!(permissions(0o2644), "rw-r-Sr--");
        assert_eq!(permissions(0o1777), "rwxrwxrwt");
        assert_eq!(permissions(0o1776), "rwxrwxrwT");
    }

    #[test]
    fn the_untranslated_comment_is_taken() {
        let xml = r#"<mime-type type="text/x-c"><comment xml:lang="de">C-Quelltext</comment><comment>C source &amp; header</comment></mime-type>"#;
        // Translated ones come after the plain one in real files; a file
        // with a translation first still finds the plain one.
        assert_eq!(parse_comment(xml).as_deref(), Some("C source & header"));
        assert_eq!(parse_comment("<mime-type/>"), None);
    }

    #[test]
    fn text_is_cut_at_a_character_and_binaries_refused() {
        assert_eq!(text_of(b"hello").as_deref(), Some("hello"));
        // "é" is two bytes; cut after the first.
        assert_eq!(text_of(b"caf\xc3").as_deref(), Some("caf"));
        assert_eq!(text_of(b"a\xffb").as_deref(), Some("a\u{fffd}b"));
        assert_eq!(text_of(b"ELF\0\0"), None);
    }

    #[test]
    fn info_for_a_file_a_folder_and_a_link() {
        let now = chrono::Local::now();
        let file = Facts { kind: "PNG image".into(), size: 2048, mode: 0o644, owner: "me".into(), ..Facts::default() };
        let lines = info_lines(&file, None, Some((640, 480)), now);
        let labels: Vec<&str> = lines.iter().map(|(l, _)| *l).collect();
        assert_eq!(labels, ["Type", "Size", "Dimensions", "Permissions", "Owner"]);
        assert_eq!(lines[1].1, "2.0 KB");
        assert_eq!(lines[2].1, "640 \u{d7} 480");
        assert_eq!(lines[3].1, "rw-r--r--");

        let folder = Facts { is_dir: true, kind: "Folder".into(), mode: 0o755, ..Facts::default() };
        assert_eq!(info_lines(&folder, None, None, now)[1], ("Size", "\u{2026}".to_string()));
        let walked = FolderSize { items: 3, bytes: 5000, complete: false };
        let lines = info_lines(&folder, Some(walked), None, now);
        assert_eq!(lines[1], ("Contains", "3 items".to_string()));
        assert_eq!(lines[2], ("Size", "more than 4.9 KB".to_string()));

        let link = Facts { link: Some("/etc/target".into()), ..file };
        assert_eq!(info_lines(&link, None, None, now).last().unwrap(), &("Link to", "/etc/target".to_string()));
    }

    #[test]
    fn a_folder_is_added_up_and_a_walk_can_stop() {
        let dir = std::env::temp_dir().join(format!("stratum-preview-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a"), [0u8; 100]).unwrap();
        std::fs::write(dir.join("sub/b"), [0u8; 50]).unwrap();
        assert_eq!(folder_size(&dir, || false), FolderSize { items: 2, bytes: 150, complete: true });
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_large_picture_is_scaled_to_fit_and_keeps_its_size() {
        let path = std::env::temp_dir().join(format!("stratum-preview-{}.png", std::process::id()));
        image::RgbaImage::from_pixel(400, 200, image::Rgba([255, 0, 0, 128])).save(&path).unwrap();
        let (image, w, h) = picture(&path, 100).unwrap();
        assert_eq!((image.width, image.height, w, h), (100, 50, 400, 200));
        // Premultiplied: red at half alpha.
        assert_eq!(&image.pixels[..4], &[128, 0, 0, 128]);
        std::fs::remove_file(path).unwrap();
    }
}
