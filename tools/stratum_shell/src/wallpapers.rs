//! Wallpapers: the images in `[wallpapers] dir`, the one shown, and the
//! links from themes to them.
//!
//! The wallpaper shown is `[wallpapers] current`, a symlink that hyprpaper
//! and the lock screen both read. Applying one points that link at the image
//! (only when it is a link, or missing: a real file is the user's and is left
//! alone) and tells hyprpaper over `hyprctl hyprpaper wallpaper`.
//!
//! A theme may be linked to a wallpaper (`$XDG_STATE_HOME/stratum/
//! theme-wallpapers.json`, by theme name; none by default), which is applied
//! whenever that theme is switched to.
//!
//! A worker thread decodes thumbnails and colour samples for the Theme page,
//! only when asked, and applies wallpapers in order, so quick picks in the
//! editor land on the last one.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::SystemTime;

use crate::sys_state::SystemState;

/// Edge of the square colour sample kept for theme generation.
pub const SAMPLE_EDGE: u32 = 48;

/// One image in the wallpaper directory.
#[derive(Clone, Debug)]
pub struct Wallpaper {
    pub path: PathBuf,
    /// File name without the extension.
    pub name: String,
    modified: Option<SystemTime>,
    /// Premultiplied RGBA, `thumb_size`, cropped to the thumbnail's shape.
    pub thumb: Arc<[u8]>,
    pub thumb_size: (u32, u32),
    /// `SAMPLE_EDGE`² pixels of the whole image, for generating a theme.
    pub sample: Arc<[[u8; 3]]>,
}

/// Two decodes of the same file at the same time are the same.
impl PartialEq for Wallpaper {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path && self.modified == other.modified
    }
}

/// A wallpaper decoded large, for the theme switcher's slices.
#[derive(Clone, Debug)]
pub struct Preview {
    /// Identifies the pixels (the file and its modification time), for the
    /// renderer's texture cache.
    pub key: u64,
    pub width: u32,
    pub height: u32,
    /// Premultiplied (opaque) RGBA8.
    pub pixels: Arc<[u8]>,
}

impl PartialEq for Preview {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

/// What the Theme page and the theme switcher show.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WallpaperState {
    /// The wallpapers decoded so far, by name; more arrive while loading.
    pub items: Vec<Wallpaper>,
    pub loading: bool,
    /// The image `current` points at.
    pub current: Option<PathBuf>,
    /// What went wrong last.
    pub error: Option<String>,
}

/// Large decodes the theme switcher asked for, by path; empty while it is
/// closed, since each is several megabytes. Published by a thread of their
/// own, so that decoding never delays applying a wallpaper.
pub type Previews = BTreeMap<PathBuf, Preview>;

impl WallpaperState {
    pub fn find(&self, path: &Path) -> Option<&Wallpaper> {
        self.items.iter().find(|w| w.path == path)
    }
}

/// What the worker is asked to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// Decode the directory's images (only those new or changed).
    Load,
    /// Show this image.
    Apply(PathBuf),
    /// Decode exactly these images large (keeping those already decoded).
    Previews(Vec<PathBuf>),
    /// Forget every large decode.
    DropPreviews,
}

/// Where the worker finds things.
#[derive(Clone, Debug)]
pub struct Places {
    pub dir: Option<PathBuf>,
    pub current: Option<PathBuf>,
    pub thumb_size: (u32, u32),
    /// The long edge of a preview, in pixels.
    pub preview_px: u32,
}

impl Places {
    pub fn new(config: &crate::config::WallpapersConfig, preview_px: u32) -> Self {
        let width = config.thumb_px.max(16);
        Self {
            dir: crate::lock::expand_home(&config.dir),
            current: crate::lock::expand_home(&config.current),
            thumb_size: (width, (width as f32 * 10.0 / 16.0).round() as u32),
            preview_px: preview_px.max(16),
        }
    }
}

/// Start the worker.
pub fn start(state: Arc<SystemState>, places: Places) -> Sender<Command> {
    let (tx, rx) = std::sync::mpsc::channel::<Command>();
    std::thread::Builder::new()
        .name("stratum-wallpapers".into())
        .spawn(move || {
            let mut snapshot = WallpaperState { current: current_target(&places), ..Default::default() };
            let publish = |snapshot: &WallpaperState| {
                if state.wallpapers.set(snapshot.clone()) {
                    state.notify();
                }
            };
            publish(&snapshot);
            let (preview_tx, preview_rx) = std::sync::mpsc::channel::<Command>();
            let preview_state = Arc::clone(&state);
            let long_edge = places.preview_px;
            std::thread::Builder::new()
                .name("stratum-wallpaper-previews".into())
                .spawn(move || run_previews(&preview_state, &preview_rx, long_edge))
                .ok();
            // Previews go straight to their own thread, however much is queued.
            let route = |command: Command| match command {
                Command::Previews(_) | Command::DropPreviews => {
                    let _ = preview_tx.send(command);
                    None
                }
                other => Some(other),
            };
            while let Ok(command) = rx.recv() {
                let Some(mut command) = route(command) else { continue };
                // Of several applies queued, only the last matters.
                while let Ok(next) = rx.try_recv() {
                    let Some(next) = route(next) else { continue };
                    match (&command, &next) {
                        (Command::Apply(_), Command::Apply(_)) => command = next,
                        _ => {
                            run(&mut snapshot, &places, command, &publish);
                            command = next;
                        }
                    }
                }
                run(&mut snapshot, &places, command, &publish);
            }
        })
        .ok();
    tx
}

/// The preview thread: decodes what the switcher asks for, dropping a
/// request as soon as a newer one arrives (the switcher closed, or opened
/// again).
fn run_previews(state: &SystemState, rx: &std::sync::mpsc::Receiver<Command>, long_edge: u32) {
    let mut previews = Previews::new();
    let publish = |previews: &Previews| {
        if state.wallpaper_previews.set(previews.clone()) {
            state.notify();
        }
    };
    let mut pending = None;
    loop {
        let mut command = match pending.take() {
            Some(command) => command,
            None => match rx.recv() {
                Ok(command) => command,
                Err(_) => return,
            },
        };
        while let Ok(next) = rx.try_recv() {
            command = next;
        }
        match command {
            Command::Previews(paths) => {
                previews.retain(|path, _| paths.contains(path));
                publish(&previews);
                for path in paths {
                    if let Ok(next) = rx.try_recv() {
                        pending = Some(next);
                        break;
                    }
                    let key = preview_key(&path);
                    if previews.get(&path).is_some_and(|p| p.key == key) {
                        continue;
                    }
                    match decode_preview(&path, key, long_edge) {
                        Ok(preview) => {
                            previews.insert(path, preview);
                            publish(&previews);
                        }
                        Err(err) => tracing::warn!("wallpaper {}: {err}", path.display()),
                    }
                }
            }
            Command::DropPreviews => {
                previews.clear();
                publish(&previews);
            }
            Command::Load | Command::Apply(_) => {}
        }
    }
}

fn run(snapshot: &mut WallpaperState, places: &Places, command: Command, publish: &impl Fn(&WallpaperState)) {
    match command {
        Command::Load => {
            let Some(dir) = &places.dir else { return };
            let files = list(dir, places.current.as_deref());
            // Keep what is unchanged; decode the rest, publishing as each
            // arrives so the page fills in.
            snapshot.items.retain(|w| files.iter().any(|(p, m)| *p == w.path && *m == w.modified));
            snapshot.loading = true;
            publish(snapshot);
            for (path, modified) in files {
                if snapshot.items.iter().any(|w| w.path == path) {
                    continue;
                }
                match decode(&path, modified, places.thumb_size) {
                    Ok(wallpaper) => {
                        snapshot.items.push(wallpaper);
                        snapshot.items.sort_by(|a, b| a.name.cmp(&b.name));
                        publish(snapshot);
                    }
                    Err(err) => tracing::warn!("wallpaper {}: {err}", path.display()),
                }
            }
            snapshot.loading = false;
            snapshot.current = current_target(places);
        }
        // Handled by the preview thread.
        Command::Previews(_) | Command::DropPreviews => return,
        Command::Apply(path) => {
            match apply(&path, places.current.as_deref()) {
                Ok(()) => snapshot.error = None,
                Err(err) => {
                    tracing::error!("could not set the wallpaper: {err}");
                    snapshot.error = Some(err);
                }
            }
            snapshot.current = current_target(places).or(Some(path));
        }
    }
    publish(snapshot);
}

/// The images in `dir`, sorted, without `current` (a link to one of them).
pub fn list(dir: &Path, current: Option<&Path>) -> Vec<(PathBuf, Option<SystemTime>)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<(PathBuf, Option<SystemTime>)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| Some(p.as_path()) != current)
        .filter(|p| {
            let ext = p.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
            matches!(ext.as_deref(), Some("png" | "jpg" | "jpeg"))
        })
        .filter(|p| !p.is_symlink())
        .map(|p| {
            let modified = std::fs::metadata(&p).and_then(|m| m.modified()).ok();
            (p, modified)
        })
        .collect();
    files.sort();
    files
}

/// The image the `current` link points at.
fn current_target(places: &Places) -> Option<PathBuf> {
    std::fs::canonicalize(places.current.as_ref()?).ok()
}

/// A thumbnail cropped to `size`'s shape, and a colour sample.
fn decode(path: &Path, modified: Option<SystemTime>, size: (u32, u32)) -> Result<Wallpaper, String> {
    let image = image::open(path).map_err(|err| err.to_string())?.into_rgba8();
    let (w, h) = image.dimensions();
    if w == 0 || h == 0 {
        return Err("empty image".into());
    }
    // Centre crop to the thumbnail's aspect, then scale.
    let aspect = size.0 as f32 / size.1 as f32;
    let (cw, ch) = if w as f32 / h as f32 > aspect {
        (((h as f32) * aspect).round() as u32, h)
    } else {
        (w, ((w as f32) / aspect).round() as u32)
    };
    let crop = image::imageops::crop_imm(&image, (w - cw) / 2, (h - ch) / 2, cw.max(1), ch.max(1));
    let mut thumb = image::imageops::thumbnail(&*crop, size.0, size.1).into_raw();
    for px in thumb.chunks_exact_mut(4) {
        // Opaque, so premultiplied is the same.
        px[3] = 255;
    }
    let sample: Vec<[u8; 3]> = image::imageops::thumbnail(&image, SAMPLE_EDGE, SAMPLE_EDGE)
        .pixels()
        .map(|p| [p[0], p[1], p[2]])
        .collect();
    Ok(Wallpaper {
        path: path.to_path_buf(),
        name: path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string(),
        modified,
        thumb: Arc::from(thumb),
        thumb_size: size,
        sample: Arc::from(sample),
    })
}

/// A key for `path` as it is now: a changed file gets a new one.
fn preview_key(path: &Path) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    std::fs::metadata(path).and_then(|m| m.modified()).ok().hash(&mut hasher);
    hasher.finish()
}

/// `path` scaled to fit `long_edge` pixels on its longer side.
fn decode_preview(path: &Path, key: u64, long_edge: u32) -> Result<Preview, String> {
    let image = image::open(path).map_err(|err| err.to_string())?.into_rgba8();
    let (w, h) = image.dimensions();
    if w == 0 || h == 0 {
        return Err("empty image".into());
    }
    let fit = (long_edge as f32 / w.max(h) as f32).min(1.0);
    let (pw, ph) = (((w as f32 * fit).round() as u32).max(1), ((h as f32 * fit).round() as u32).max(1));
    let mut pixels = if (pw, ph) == (w, h) {
        image.into_raw()
    } else {
        image::imageops::thumbnail(&image, pw, ph).into_raw()
    };
    for px in pixels.chunks_exact_mut(4) {
        px[3] = 255;
    }
    Ok(Preview { key, width: pw, height: ph, pixels: Arc::from(pixels) })
}

/// Show `path`: point the `current` link at it, then tell hyprpaper.
fn apply(path: &Path, current: Option<&Path>) -> Result<(), String> {
    if !path.is_file() {
        return Err(format!("{} is not a file", path.display()));
    }
    if let Some(link) = current {
        relink(link, path)?;
    }
    let output = std::process::Command::new("hyprctl")
        .args(["hyprpaper", "wallpaper", &format!(",{}", path.display())])
        .output()
        .map_err(|err| format!("hyprctl: {err}"))?;
    if !output.status.success() {
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return Err(format!("hyprpaper: {text}"));
    }
    Ok(())
}

/// Point the symlink `link` at `target`, atomically. A real file there is
/// the user's, not ours: it is left alone.
fn relink(link: &Path, target: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(link) {
        Ok(meta) if !meta.file_type().is_symlink() => {
            tracing::warn!("{} is not a symlink; leaving it alone", link.display());
            return Ok(());
        }
        _ => {}
    }
    let tmp = link.with_extension("stratum-tmp");
    let _ = std::fs::remove_file(&tmp);
    std::os::unix::fs::symlink(target, &tmp).map_err(|err| format!("{}: {err}", tmp.display()))?;
    std::fs::rename(&tmp, link).map_err(|err| format!("{}: {err}", link.display()))
}

/// Theme name -> linked wallpaper.
pub type Links = BTreeMap<String, PathBuf>;

fn links_path() -> Option<PathBuf> {
    crate::paths::state_file("theme-wallpapers.json")
}

pub fn load_links() -> Links {
    let Some(path) = links_path() else { return Links::new() };
    let Ok(text) = std::fs::read_to_string(&path) else { return Links::new() };
    serde_json::from_str(&text).unwrap_or_else(|err| {
        tracing::warn!("ignoring unreadable {}: {err}", path.display());
        Links::new()
    })
}

pub fn store_links(links: &Links) {
    let Some(path) = links_path() else { return };
    let text = serde_json::to_string_pretty(links).unwrap_or_default();
    if let Err(err) = crate::paths::write_atomic(&path, text) {
        tracing::warn!("could not save theme wallpapers to {}: {err}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("stratum-wallpapers-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn png(path: &Path, w: u32, h: u32, rgb: [u8; 3]) {
        image::RgbImage::from_pixel(w, h, image::Rgb(rgb)).save(path).unwrap();
    }

    #[test]
    fn listing_takes_images_but_not_the_current_link() {
        let dir = scratch("list");
        png(&dir.join("b.png"), 4, 4, [0, 0, 0]);
        png(&dir.join("a.jpg"), 4, 4, [0, 0, 0]);
        std::fs::write(dir.join("notes.txt"), "x").unwrap();
        let current = dir.join("current_wp.png");
        std::os::unix::fs::symlink(dir.join("b.png"), &current).unwrap();
        let names: Vec<_> = list(&dir, Some(&current)).into_iter().map(|(p, _)| p.file_name().unwrap().to_owned()).collect();
        assert_eq!(names, ["a.jpg", "b.png"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_thumbnail_is_cropped_to_its_shape_and_sampled() {
        let dir = scratch("decode");
        let path = dir.join("wide.png");
        png(&path, 400, 100, [200, 40, 40]);
        let w = decode(&path, None, (32, 20)).unwrap();
        assert_eq!(w.thumb.len(), 32 * 20 * 4);
        assert_eq!(w.sample.len(), (SAMPLE_EDGE * SAMPLE_EDGE) as usize);
        assert_eq!(w.sample[0], [200, 40, 40]);
        assert_eq!(w.name, "wide");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_preview_fits_its_long_edge_and_keeps_its_shape() {
        let dir = scratch("preview");
        let path = dir.join("tall.png");
        png(&path, 300, 600, [10, 20, 30]);
        let p = decode_preview(&path, preview_key(&path), 100).unwrap();
        assert_eq!((p.width, p.height), (50, 100));
        assert_eq!(p.pixels.len(), 50 * 100 * 4);
        // Never scaled up.
        let small = decode_preview(&path, 0, 4000).unwrap();
        assert_eq!((small.width, small.height), (300, 600));
        assert_eq!(preview_key(&path), preview_key(&path));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn relinking_moves_a_link_but_never_replaces_a_file() {
        let dir = scratch("relink");
        let (a, b) = (dir.join("a.png"), dir.join("b.png"));
        png(&a, 2, 2, [0, 0, 0]);
        png(&b, 2, 2, [0, 0, 0]);
        let link = dir.join("current_wp.png");
        std::os::unix::fs::symlink(&a, &link).unwrap();
        relink(&link, &b).unwrap();
        assert_eq!(std::fs::read_link(&link).unwrap(), b);
        // A missing link is created.
        let fresh = dir.join("fresh.png");
        relink(&fresh, &a).unwrap();
        assert_eq!(std::fs::read_link(&fresh).unwrap(), a);
        // A real file stays.
        relink(&a, &b).unwrap();
        assert!(!a.is_symlink());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
