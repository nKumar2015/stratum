//! Thumbnails: made here for the pictures `image` and resvg decode, by the
//! thumbnailers installed on the system for everything else (videos,
//! PDFs, fonts: the freedesktop thumbnailer spec's `.thumbnailer` files),
//! and kept in the freedesktop thumbnail cache,
//! `$XDG_CACHE_HOME/thumbnails/<size>/<md5 of the URI>.png`, which other
//! file managers and apps read and fill too. A cached thumbnail is used
//! only while its `Thumb::MTime` matches the file's.

use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use stratum_ui::icons::IconImage;

/// How long a thumbnailer may take before it is stopped.
const TIMEOUT: Duration = Duration::from_secs(10);

/// Types decoded here rather than by a thumbnailer.
const IN_PROCESS: &[&str] = &["image/png", "image/jpeg", "image/webp", "image/gif", "image/bmp", "image/svg+xml"];

pub fn in_process(mime: &str) -> bool {
    IN_PROCESS.contains(&mime)
}

// ---- MD5 (RFC 1321), for the cache's file names ----------------------

pub fn md5(data: &[u8]) -> [u8; 16] {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23,
        4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    // K[i] = floor(abs(sin(i + 1)) * 2^32).
    let k: [u32; 64] = std::array::from_fn(|i| ((i as f64 + 1.0).sin().abs() * 4_294_967_296.0) as u32);
    let mut state: [u32; 4] = [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&((data.len() as u64).wrapping_mul(8)).to_le_bytes());
    for block in message.chunks_exact(64) {
        let m: [u32; 16] = std::array::from_fn(|i| u32::from_le_bytes(block[i * 4..i * 4 + 4].try_into().expect("four bytes")));
        let [mut a, mut b, mut c, mut d] = state;
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(S[i]));
        }
        for (s, v) in state.iter_mut().zip([a, b, c, d]) {
            *s = s.wrapping_add(v);
        }
    }
    let mut out = [0u8; 16];
    for (i, word) in state.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    out
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ---- The cache ---------------------------------------------------------

/// The cache's size folders and their sizes; a thumbnail is made at the
/// smallest that is at least what is asked.
const CLASSES: [(&str, u32); 4] = [("normal", 128), ("large", 256), ("x-large", 512), ("xx-large", 1024)];

pub fn class_for(px: u32) -> (&'static str, u32) {
    CLASSES.iter().copied().find(|&(_, size)| size >= px).unwrap_or(CLASSES[3])
}

pub fn cache_dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".cache"))
        .join("thumbnails")
}

/// The cache file for a URI in a size folder.
pub fn cache_file(uri: &str, class: &str) -> PathBuf {
    cache_dir().join(class).join(format!("{}.png", hex(&md5(uri.as_bytes()))))
}

/// Whether thumbnails of `path` are kept: not for the cache's own files,
/// nor for temporary ones.
pub fn cacheable(path: &Path) -> bool {
    !path.starts_with(cache_dir()) && !path.starts_with(std::env::temp_dir())
}

fn mtime_secs(time: SystemTime) -> u64 {
    time.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// A cached thumbnail, if it is of this URI at this modification time.
pub fn read_cache(file: &Path, uri: &str, mtime: SystemTime) -> Option<image::RgbaImage> {
    let bytes = std::fs::read(file).ok()?;
    let reader = png::Decoder::new(std::io::Cursor::new(&bytes)).read_info().ok()?;
    let info = reader.info();
    let text = |key: &str| {
        info.uncompressed_latin1_text
            .iter()
            .find(|c| c.keyword == key)
            .map(|c| c.text.clone())
            .or_else(|| info.utf8_text.iter().find(|c| c.keyword == key).and_then(|c| c.get_text().ok()))
    };
    if text("Thumb::URI")? != uri || text("Thumb::MTime")?.parse::<u64>().ok()? != mtime_secs(mtime) {
        return None;
    }
    Some(image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).ok()?.into_rgba8())
}

/// Keep a thumbnail in the cache, as the spec asks: written beside its
/// place and renamed into it, readable by the user alone.
pub fn write_cache(file: &Path, uri: &str, mtime: SystemTime, size: u64, image: &image::RgbaImage) -> std::io::Result<()> {
    let dir = file.parent().ok_or_else(|| std::io::Error::other("no folder"))?;
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let temp = dir.join(format!(".stratum-{}-{}.png", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed)));
    let result = (|| {
        let out = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&temp)?;
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(out), image.width(), image.height());
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let other = |e: png::EncodingError| std::io::Error::other(e.to_string());
        encoder.add_text_chunk("Thumb::URI".into(), uri.into()).map_err(other)?;
        encoder.add_text_chunk("Thumb::MTime".into(), mtime_secs(mtime).to_string()).map_err(other)?;
        encoder.add_text_chunk("Thumb::Size".into(), size.to_string()).map_err(other)?;
        encoder.add_text_chunk("Software".into(), "stratum-files".into()).map_err(other)?;
        let mut writer = encoder.write_header().map_err(other)?;
        writer.write_image_data(image.as_raw()).map_err(other)?;
        writer.finish().map_err(other)?;
        std::fs::rename(&temp, file)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

// ---- Thumbnailers ----------------------------------------------------

/// A `.thumbnailer` file: the command and the types it makes thumbnails
/// of.
#[derive(Clone, Debug, PartialEq)]
pub struct Thumbnailer {
    pub exec: Vec<String>,
    pub mimes: Vec<String>,
    /// The program that must be installed, if it says.
    pub try_exec: Option<String>,
}

pub fn parse_thumbnailer(text: &str) -> Option<Thumbnailer> {
    let mut in_entry = false;
    let (mut exec, mut mimes, mut try_exec) = (None, Vec::new(), None);
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_entry = line == "[Thumbnailer Entry]";
            continue;
        }
        let Some((key, value)) = line.split_once('=').filter(|_| in_entry) else { continue };
        match key.trim() {
            "Exec" => exec = stratum_ui::apps::split_exec(value.trim()),
            "TryExec" => try_exec = Some(value.trim().to_string()),
            "MimeType" => mimes = value.split(';').map(str::trim).filter(|m| !m.is_empty()).map(str::to_string).collect(),
            _ => {}
        }
    }
    let exec = exec.filter(|e| !e.is_empty())?;
    (!mimes.is_empty()).then_some(Thumbnailer { exec, mimes, try_exec })
}

/// Fill in `%i` (the file), `%u` (its URI), `%o` (the output), `%s` (the
/// size) and `%%`.
pub fn expand(exec: &[String], input: &Path, output: &Path, size: u32) -> Vec<String> {
    exec.iter()
        .map(|arg| {
            let mut out = String::new();
            let mut chars = arg.chars();
            while let Some(c) = chars.next() {
                if c != '%' {
                    out.push(c);
                    continue;
                }
                match chars.next() {
                    Some('i') => out.push_str(&input.to_string_lossy()),
                    Some('u') => out.push_str(&stratum_ui::uri::from_path(input)),
                    Some('o') => out.push_str(&output.to_string_lossy()),
                    Some('s') => out.push_str(&size.to_string()),
                    Some('%') => out.push('%'),
                    Some(other) => {
                        out.push('%');
                        out.push(other);
                    }
                    None => out.push('%'),
                }
            }
            out
        })
        .collect()
}

fn on_path(program: &str) -> bool {
    if program.contains('/') {
        return Path::new(program).is_file();
    }
    std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
}

/// The thumbnailers installed: every data directory's `thumbnailers/`,
/// a file name in an earlier directory hiding the same in a later one,
/// and only those whose program is there.
pub fn load_thumbnailers() -> Vec<Thumbnailer> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for dir in stratum_ui::icons::data_dirs() {
        let Ok(entries) = std::fs::read_dir(dir.join("thumbnailers")) else { continue };
        let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "thumbnailer")).collect();
        files.sort();
        for file in files {
            let Some(name) = file.file_name().map(|n| n.to_owned()) else { continue };
            if !seen.insert(name) {
                continue;
            }
            let Some(thumbnailer) = std::fs::read_to_string(&file).ok().and_then(|t| parse_thumbnailer(&t)) else { continue };
            let program = thumbnailer.try_exec.as_deref().unwrap_or(&thumbnailer.exec[0]);
            if on_path(program) {
                out.push(thumbnailer);
            }
        }
    }
    out
}

/// Run a thumbnailer for `input` at `size`, stopping it after `TIMEOUT`.
fn run(thumbnailer: &Thumbnailer, input: &Path, size: u32) -> Option<image::RgbaImage> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let output = std::env::temp_dir().join(format!("stratum-thumb-{}-{}.png", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed)));
    let args = expand(&thumbnailer.exec, input, &output, size);
    let mut child = Command::new(&args[0]).args(&args[1..]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().ok()?;
    let started = Instant::now();
    let finished = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if started.elapsed() < TIMEOUT => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                tracing::warn!("{} took too long on {}; stopped", args[0], input.display());
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
        }
    };
    let image = finished.then(|| image::open(&output).ok()).flatten().map(|i| i.into_rgba8());
    let _ = std::fs::remove_file(&output);
    image
}

// ---- Making them -------------------------------------------------------

/// An SVG drawn to fit `size`.
fn svg(path: &Path, size: u32) -> Option<image::RgbaImage> {
    let data = std::fs::read(path).ok()?;
    let tree = resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default()).ok()?;
    let source = tree.size();
    let scale = (size as f32 / source.width().max(1.0)).min(size as f32 / source.height().max(1.0));
    let (w, h) = ((source.width() * scale).round().max(1.0) as u32, (source.height() * scale).round().max(1.0) as u32);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h)?;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    // tiny-skia keeps premultiplied pixels; the cache holds straight ones.
    let pixels = pixmap.pixels().iter().flat_map(|p| {
        let c = p.demultiply();
        [c.red(), c.green(), c.blue(), c.alpha()]
    });
    image::RgbaImage::from_raw(w, h, pixels.collect())
}

/// A picture decoded here, scaled to fit `size`.
fn decode(path: &Path, mime: &str, size: u32) -> Option<image::RgbaImage> {
    if mime == "image/svg+xml" {
        return svg(path, size);
    }
    let image = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.decode().ok()?;
    let image = if image.width() > size || image.height() > size { image.thumbnail(size, size) } else { image };
    Some(image.into_rgba8())
}

/// What a thumbnail is made from and how.
pub struct Request<'a> {
    pub path: &'a Path,
    pub mime: &'a str,
    pub modified: Option<SystemTime>,
    pub bytes: u64,
    /// Pixels wanted on each side.
    pub px: u32,
    /// Pictures larger than this keep their icon (decoding them costs).
    pub max_bytes: u64,
    /// Run installed thumbnailers.
    pub external: bool,
}

/// Whether a thumbnail can be made for a file of this type and size.
pub fn can(mime: &str, bytes: u64, max_bytes: u64, external: bool, thumbnailers: &[Thumbnailer]) -> bool {
    if in_process(mime) {
        bytes <= max_bytes
    } else {
        external && thumbnailers.iter().any(|t| t.mimes.iter().any(|m| m == mime))
    }
}

/// A thumbnail at most `px` on a side: from the cache when it is fresh,
/// else made (here or by a thumbnailer) at the cache's size and kept.
/// Straight (not premultiplied) alpha.
pub fn make(request: &Request<'_>, thumbnailers: &[Thumbnailer]) -> Option<image::RgbaImage> {
    let uri = stratum_ui::uri::from_path(request.path);
    let (class, class_px) = class_for(request.px);
    let cache = request.modified.filter(|_| cacheable(request.path)).map(|mtime| (cache_file(&uri, class), mtime));
    let fit = |image: image::RgbaImage| {
        if image.width() > request.px || image.height() > request.px {
            image::DynamicImage::ImageRgba8(image).thumbnail(request.px, request.px).into_rgba8()
        } else {
            image
        }
    };
    if let Some((file, mtime)) = &cache {
        if let Some(found) = read_cache(file, &uri, *mtime) {
            return Some(fit(found));
        }
    }
    let made = if in_process(request.mime) {
        (request.bytes <= request.max_bytes).then(|| decode(request.path, request.mime, class_px)).flatten()
    } else if request.external {
        thumbnailers.iter().filter(|t| t.mimes.iter().any(|m| m == request.mime)).find_map(|t| run(t, request.path, class_px))
    } else {
        None
    }?;
    // Kept at the cache's size, never larger: a thumbnailer may ignore %s.
    let made = if made.width() > class_px || made.height() > class_px {
        image::DynamicImage::ImageRgba8(made).thumbnail(class_px, class_px).into_rgba8()
    } else {
        made
    };
    if let Some((file, mtime)) = &cache {
        if let Err(err) = write_cache(file, &uri, *mtime, request.bytes, &made) {
            tracing::debug!("thumbnail not cached: {err}");
        }
    }
    Some(fit(made))
}

/// Straight RGBA to the renderer's premultiplied image.
pub fn premultiplied(image: image::RgbaImage) -> IconImage {
    let (width, height) = image.dimensions();
    let mut pixels = image.into_raw();
    for px in pixels.chunks_exact_mut(4) {
        let a = u16::from(px[3]);
        for c in &mut px[..3] {
            *c = (u16::from(*c) * a / 255) as u8;
        }
    }
    IconImage { width, height, pixels: pixels.into() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_matches_rfc_1321() {
        assert_eq!(hex(&md5(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex(&md5(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(hex(&md5(b"message digest")), "f96b697d7cb7938d525a2f31aaf161d0");
        let long = b"12345678901234567890123456789012345678901234567890123456789012345678901234567890";
        assert_eq!(hex(&md5(long)), "57edf4a22be3c955ac49da2e2107b67a");
    }

    #[test]
    fn the_cache_file_is_named_by_the_uri() {
        // The spec's own example.
        let file = cache_file("file:///home/jens/photos/me.png", "normal");
        assert!(file.ends_with("thumbnails/normal/c6ee772d9e49320e97ec29a7eb5b1697.png"), "{}", file.display());
        assert_eq!(class_for(40), ("normal", 128));
        assert_eq!(class_for(200), ("large", 256));
        assert_eq!(class_for(5000), ("xx-large", 1024));
    }

    #[test]
    fn a_thumbnailer_file_is_read_and_expanded() {
        let text = "[Thumbnailer Entry]\nTryExec=ffmpegthumbnailer\nExec=ffmpegthumbnailer -i %i -o %o -s %s -f\nMimeType=video/mp4;video/webm;\n\n[Other]\nExec=nope\n";
        let t = parse_thumbnailer(text).unwrap();
        assert_eq!(t.try_exec.as_deref(), Some("ffmpegthumbnailer"));
        assert_eq!(t.mimes, ["video/mp4", "video/webm"]);
        let args = expand(&t.exec, Path::new("/v/a b.mp4"), Path::new("/tmp/o.png"), 256);
        assert_eq!(args, ["ffmpegthumbnailer", "-i", "/v/a b.mp4", "-o", "/tmp/o.png", "-s", "256", "-f"]);
        assert_eq!(expand(&["%u".into(), "100%%".into()], Path::new("/a b"), Path::new("/o"), 1), ["file:///a%20b", "100%"]);
        assert_eq!(parse_thumbnailer("[Thumbnailer Entry]\nExec=x %i\n"), None, "no types");
    }

    #[test]
    fn which_files_get_thumbnails() {
        let video = Thumbnailer { exec: vec!["t".into()], mimes: vec!["video/mp4".into()], try_exec: None };
        assert!(can("image/webp", 10, 100, false, &[]));
        assert!(!can("image/png", 1000, 100, false, &[]), "too large to decode");
        assert!(!can("video/mp4", 10, 100, false, std::slice::from_ref(&video)), "thumbnailers are off");
        assert!(can("video/mp4", 1 << 30, 100, true, std::slice::from_ref(&video)), "no size limit for a thumbnailer");
        assert!(!can("application/pdf", 10, 100, true, &[video]));
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("stratum-thumbs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_cache_round_trips_and_goes_stale() {
        let dir = scratch("cache");
        let file = dir.join("normal/x.png");
        let when = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let image = image::RgbaImage::from_pixel(4, 2, image::Rgba([10, 20, 30, 40]));
        write_cache(&file, "file:///a.png", when, 99, &image).unwrap();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(read_cache(&file, "file:///a.png", when).unwrap(), image);
        assert!(read_cache(&file, "file:///b.png", when).is_none(), "another file's");
        assert!(read_cache(&file, "file:///a.png", when + Duration::from_secs(1)).is_none(), "changed since");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_thumbnailer_runs_and_one_that_fails_gives_nothing() {
        let dir = scratch("run");
        let picture = dir.join("p.png");
        image::RgbaImage::from_pixel(8, 8, image::Rgba([1, 2, 3, 255])).save(&picture).unwrap();
        // A stand-in that copies a picture to %o; and one that fails.
        let good = Thumbnailer { exec: vec!["cp".into(), picture.to_string_lossy().into_owned(), "%o".into()], mimes: vec![], try_exec: None };
        assert_eq!(run(&good, Path::new("/whatever"), 128).unwrap().dimensions(), (8, 8));
        let bad = Thumbnailer { exec: vec!["false".into()], mimes: vec![], try_exec: None };
        assert!(run(&bad, Path::new("/whatever"), 128).is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_svg_keeps_its_shape() {
        let dir = scratch("svg");
        let file = dir.join("a.svg");
        std::fs::write(&file, r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100"><rect width="200" height="100" fill="#ff0000"/></svg>"##).unwrap();
        let image = svg(&file, 64).unwrap();
        assert_eq!(image.dimensions(), (64, 32));
        assert_eq!(image.get_pixel(10, 10).0, [255, 0, 0, 255]);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
