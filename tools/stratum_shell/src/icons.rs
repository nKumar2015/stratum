//! Icon-theme lookup and rasterization.
//!
//! We implement the freedesktop icon lookup ourselves rather than pulling in a
//! toolkit: parse `index.theme`, walk the theme's size directories in the
//! order the spec defines, and rasterize SVG (via `resvg`) or PNG (via
//! `image`) into premultiplied RGBA that the renderer uploads as a texture.
//!
//! Results are cached by `(name, size, scale)`; icon rendering is one of the
//! few places the shell can afford to be lazy because it only ever asks for a
//! handful of names.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A decoded icon ready to upload.
#[derive(Clone, Debug)]
pub struct IconImage {
    pub width: u32,
    pub height: u32,
    /// Premultiplied RGBA8.
    pub pixels: Arc<[u8]>,
}

#[derive(Clone)]
struct ThemeEntry {
    /// Every base directory holding this theme, in search order. A theme is
    /// the union of all of them (hicolor alone is spread over every package
    /// that installs an icon), so the first one found is not the theme.
    roots: Vec<PathBuf>,
    /// Size -> subdirectories, as declared by `index.theme`.
    directories: Vec<String>,
}

pub struct IconTheme {
    themes: HashMap<String, ThemeEntry>,
    /// Search order, most specific first, ending in `hicolor`.
    search: Vec<String>,
    cache: HashMap<(String, u32, u32), Option<IconImage>>,
    /// Built on the first app-icon lookup, so a shell that never asks pays
    /// nothing for reading every desktop entry.
    desktop: Option<crate::desktop::DesktopIndex>,
}

impl IconTheme {
    /// Discover themes from the standard XDG data directories.
    pub fn load(preferred: &[&str]) -> Self {
        let roots: Vec<PathBuf> = data_dirs().into_iter().map(|dir| dir.join("icons")).chain(
            std::env::var("HOME").ok().map(|home| PathBuf::from(home).join(".icons")),
        ).collect();
        let themes = Self::discover(&roots);

        // Prefer explicitly requested themes, then hicolor, then all others.
        let mut search: Vec<String> = preferred.iter().map(|s| s.to_string()).collect();
        if !themes.contains_key("hicolor") {
            // Most systems ship it; if absent we simply have no icons.
        }
        search.push("hicolor".to_string());

        Self {
            themes,
            search,
            cache: HashMap::new(),
            desktop: None,
        }
    }

    /// The icon for an application, by window class or app name: through its
    /// desktop entry, then the name itself, then the generic executable icon.
    pub fn load_app_icon(&mut self, class: &str, size: u32, scale: u32) -> Option<IconImage> {
        let candidates = self
            .desktop
            .get_or_insert_with(crate::desktop::DesktopIndex::load)
            .icon_candidates(class);
        candidates
            .iter()
            .find_map(|name| self.load_icon(name, size, scale))
    }

    /// Every theme under the icon `roots`, merging a theme's directories
    /// across roots.
    fn discover(roots: &[PathBuf]) -> HashMap<String, ThemeEntry> {
        let mut themes: HashMap<String, ThemeEntry> = HashMap::new();
        for root in roots {
            let Ok(entries) = std::fs::read_dir(root) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                let Some(found) = Self::parse_theme(path) else {
                    continue;
                };
                match themes.get_mut(&name) {
                    Some(theme) => {
                        theme.roots.extend(found.roots);
                        for dir in found.directories {
                            if !theme.directories.contains(&dir) {
                                theme.directories.push(dir);
                            }
                        }
                    }
                    None => {
                        themes.insert(name, found);
                    }
                }
            }
        }
        themes
    }

    fn parse_theme(root: PathBuf) -> Option<ThemeEntry> {
        // `index.theme` lives at the theme root; sub-themes may not have one.
        let index = root.join("index.theme");
        let mut directories = Vec::new();
        if let Ok(text) = std::fs::read_to_string(&index) {
            for line in text.lines() {
                let line = line.trim();
                if let Some(rest) = line.strip_prefix("Directories=") {
                    directories.extend(rest.split(',').map(|s| s.trim().to_string()));
                }
            }
        }
        if directories.is_empty() {
            // Fall back to whatever size directories physically exist.
            if let Ok(entries) = std::fs::read_dir(&root) {
                for entry in entries.flatten() {
                    if entry.path().is_dir() {
                        directories.push(entry.file_name().to_string_lossy().to_string());
                    }
                }
            }
        }
        if directories.is_empty() {
            return None;
        }
        Some(ThemeEntry { roots: vec![root], directories })
    }

    /// Look up and rasterize `name` at `size` logical pixels on a `scale`x
    /// output. Returns `None` when the icon is not found.
    pub fn load_icon(&mut self, name: &str, size: u32, scale: u32) -> Option<IconImage> {
        let physical = size.saturating_mul(scale.max(1));
        let key = (name.to_string(), physical, scale);
        if let Some(hit) = self.cache.get(&key) {
            return hit.clone();
        }

        let result = self.find_and_render(name, physical);
        self.cache.insert(key, result.clone());
        result
    }

    fn find_and_render(&self, name: &str, size: u32) -> Option<IconImage> {
        // Handle absolute paths and `file://` references directly.
        let name = name.strip_prefix("file://").unwrap_or(name);
        if name.starts_with('/') {
            return render_file(Path::new(name), size);
        }

        for theme_name in &self.search {
            let Some(theme) = self.themes.get(theme_name) else {
                continue;
            };
            // Prefer the closest declared size bucket, then any other.
            // Some themes keep icons directly in the root, tried last.
            let dirs = sorted_directories(&theme.directories, size).into_iter().chain([String::new()]);
            for dir in dirs {
                for root in &theme.roots {
                    for ext in ["svg", "png"] {
                        let candidate = root.join(&dir).join(format!("{name}.{ext}"));
                        if candidate.is_file() {
                            if let Some(image) = render_file(&candidate, size) {
                                return Some(image);
                            }
                        }
                    }
                }
            }
        }
        None
    }
}

/// XDG data directories, most specific first, without duplicates.
///
/// `XDG_DATA_DIRS` alone is not enough: a shell started from a devshell or a
/// systemd unit may not inherit the session's value, and on NixOS the user's
/// packages (icon themes, desktop entries) live in profile directories that
/// only the session's value names. Those are always searched.
pub fn data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let home = std::env::var("HOME").ok().map(PathBuf::from);
    match std::env::var("XDG_DATA_HOME") {
        Ok(dir) if !dir.is_empty() => dirs.push(PathBuf::from(dir)),
        _ => dirs.extend(home.as_ref().map(|h| h.join(".local/share"))),
    }
    if let Ok(list) = std::env::var("XDG_DATA_DIRS") {
        dirs.extend(list.split(':').filter(|d| !d.is_empty()).map(PathBuf::from));
    }
    if let Ok(user) = std::env::var("USER") {
        dirs.push(PathBuf::from(format!("/etc/profiles/per-user/{user}/share")));
    }
    dirs.extend(home.map(|h| h.join(".nix-profile/share")));
    dirs.push(PathBuf::from("/run/current-system/sw/share"));
    dirs.push(PathBuf::from("/usr/local/share"));
    dirs.push(PathBuf::from("/usr/share"));
    let mut seen = std::collections::HashSet::new();
    dirs.retain(|dir| seen.insert(dir.clone()));
    dirs
}

/// Sort directory names (`48x48/apps`, `scalable/apps`, …) by how close their
/// declared size is to `want`, keeping non-numeric buckets last.
fn sorted_directories(dirs: &[String], want: u32) -> Vec<String> {
    let mut scored: Vec<(u64, String)> = dirs
        .iter()
        .map(|dir| {
            // Size buckets are `NxN/...`, `N/...`, or named (`scalable`, `symbolic`).
            let first = dir.split('/').next().unwrap_or(dir);
            let digits: String = first.chars().take_while(|c| c.is_ascii_digit()).collect();
            let size = digits.parse::<u32>().ok();
            let score = match size {
                Some(s) => (s as i64 - want as i64).unsigned_abs(),
                // Named buckets still work, just after exact-size ones.
                None => u64::MAX / 2,
            };
            (score, dir.clone())
        })
        .collect();
    scored.sort_by_key(|(score, _)| *score);
    scored.into_iter().map(|(_, dir)| dir).collect()
}

/// `icon` turned 90° counter-clockwise, to sit in a run that reads upward.
pub fn rotate_ccw(icon: &IconImage) -> IconImage {
    let (w, h) = (icon.width as usize, icon.height as usize);
    let mut pixels = vec![0u8; w * h * 4];
    // The rotated image is h wide and w tall; its (x, y) is the source's
    // (w - 1 - y, x).
    for y in 0..w {
        for x in 0..h {
            let from = (x * w + (w - 1 - y)) * 4;
            let to = (y * h + x) * 4;
            pixels[to..to + 4].copy_from_slice(&icon.pixels[from..from + 4]);
        }
    }
    IconImage { width: icon.height, height: icon.width, pixels: Arc::from(pixels) }
}

fn render_file(path: &Path, size: u32) -> Option<IconImage> {
    let data = std::fs::read(path).ok()?;
    match path.extension().and_then(|e| e.to_str()) {
        Some("svg") => render_svg(&data, size),
        Some("png") => render_png(&data, size),
        _ => None,
    }
}

fn render_svg(data: &[u8], size: u32) -> Option<IconImage> {
    let tree = resvg::usvg::Tree::from_data(data, &resvg::usvg::Options::default()).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size.max(1), size.max(1))?;
    let source = tree.size();
    let scale = (size as f32 / source.width().max(1.0))
        .min(size as f32 / source.height().max(1.0));
    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Some(IconImage {
        width: pixmap.width(),
        height: pixmap.height(),
        pixels: Arc::from(pixmap.data().to_vec()),
    })
}

fn render_png(data: &[u8], size: u32) -> Option<IconImage> {
    let image = image::load_from_memory(data).ok()?;
    let size = size.max(1);
    // Fit inside the square, keeping the aspect ratio, and centre it: a
    // notification's picture is often not square, and stretching it showed.
    let fitted = if image.width() == size && image.height() == size {
        image.to_rgba8()
    } else {
        image
            .resize(size, size, image::imageops::FilterType::Lanczos3)
            .to_rgba8()
    };
    let mut canvas = image::RgbaImage::new(size, size);
    image::imageops::overlay(
        &mut canvas,
        &fitted,
        ((size - fitted.width()) / 2) as i64,
        ((size - fitted.height()) / 2) as i64,
    );

    // tiny-skia and image both use straight alpha; premultiply for compositing.
    let mut pixels = canvas.into_raw();
    for px in pixels.chunks_exact_mut(4) {
        let alpha = px[3] as u32;
        px[0] = ((px[0] as u32 * alpha) / 255) as u8;
        px[1] = ((px[1] as u32 * alpha) / 255) as u8;
        px[2] = ((px[2] as u32 * alpha) / 255) as u8;
    }
    Some(IconImage {
        width: size,
        height: size,
        pixels: Arc::from(pixels),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_sorting_prefers_exact_size() {
        let dirs = vec![
            "16x16/apps".to_string(),
            "48x48/apps".to_string(),
            "scalable/apps".to_string(),
            "32x32/apps".to_string(),
        ];
        let sorted = sorted_directories(&dirs, 48);
        assert_eq!(sorted[0], "48x48/apps");
        assert_eq!(sorted[1], "32x32/apps");
    }

    #[test]
    fn svg_icons_render_without_resvg_default_features() {
        // Gradients, groups and transforms are what icon themes use; resvg
        // is built without text or raster-image support.
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16">
            <defs><linearGradient id="g"><stop offset="0" stop-color="#f00"/>
            <stop offset="1" stop-color="#00f"/></linearGradient></defs>
            <g transform="translate(2 2)"><rect width="12" height="12" fill="url(#g)"/></g>
            </svg>"##;
        let icon = render_svg(svg, 16).expect("renders");
        let at = |x: usize, y: usize| &icon.pixels[(y * 16 + x) * 4..(y * 16 + x) * 4 + 4];
        assert_eq!(at(0, 0)[3], 0, "outside the rect is clear");
        assert!(at(3, 8)[0] > 200 && at(3, 8)[3] == 255, "red end of the gradient");
        assert!(at(12, 8)[2] > 200, "blue end of the gradient");
    }

    #[test]
    fn rotating_counter_clockwise_moves_the_top_right_to_the_top_left() {
        // 2 wide, 1 tall: red then blue.
        let icon = IconImage { width: 2, height: 1, pixels: Arc::from(vec![255, 0, 0, 255, 0, 0, 255, 255]) };
        let turned = rotate_ccw(&icon);
        assert_eq!((turned.width, turned.height), (1, 2));
        assert_eq!(&turned.pixels[..], &[0, 0, 255, 255, 255, 0, 0, 255], "blue on top, red below");
    }

    #[test]
    fn a_theme_spans_every_root_that_holds_it() {
        // A package's own hicolor (first in XDG_DATA_DIRS) must not hide the
        // profile's hicolor, which is where the browser's icon was.
        let base = std::env::temp_dir().join(format!("stratum-icons-{}", std::process::id()));
        let first = base.join("pkg/icons");
        let second = base.join("profile/icons");
        for (root, dirs) in [(&first, "16x16/apps"), (&second, "16x16/apps,48x48/apps")] {
            let theme = root.join("hicolor");
            for dir in dirs.split(',') {
                std::fs::create_dir_all(theme.join(dir)).unwrap();
            }
            std::fs::write(theme.join("index.theme"), format!("[Icon Theme]\nDirectories={dirs}\n")).unwrap();
        }
        let png = second.join("hicolor/48x48/apps/browser.png");
        image::RgbaImage::from_pixel(48, 48, image::Rgba([255, 0, 0, 255])).save(&png).unwrap();

        let themes = IconTheme::discover(&[first.clone(), second.clone()]);
        let mut theme = IconTheme { themes, search: vec!["hicolor".into()], cache: HashMap::new(), desktop: None };
        assert_eq!(theme.themes["hicolor"].roots, vec![first.join("hicolor"), second.join("hicolor")]);
        assert!(theme.themes["hicolor"].directories.contains(&"48x48/apps".to_string()));
        assert!(theme.load_icon("browser", 48, 1).is_some());
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn missing_icon_returns_none_and_caches() {
        let mut theme = IconTheme::load(&["DefinitelyNotATheme"]);
        assert!(theme.load_icon("this-icon-does-not-exist", 24, 1).is_none());
        // Second call takes the cache path and still returns None.
        assert!(theme.load_icon("this-icon-does-not-exist", 24, 1).is_none());
    }
}
