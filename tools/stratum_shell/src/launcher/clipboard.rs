//! The launcher's clipboard mode over the shell's own history
//! (`crate::clipboard::store`). Restoring hands an entry's file to
//! `wl-copy`; thumbnails are decoded from the files off the event loop.

use std::path::{Path, PathBuf};

use crate::clipboard::store::Store;

/// One history entry as the list shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    pub id: u64,
    pub mime: String,
    /// The text on one line; empty for an image.
    pub preview: String,
    pub image: Option<ImageInfo>,
    pub path: PathBuf,
    /// Bytes, and when it was copied (seconds since the epoch).
    pub len: u64,
    pub created: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImageInfo {
    pub format: String,
    pub size: Option<(u32, u32)>,
}

/// The history, newest first.
pub fn list(store: &Store) -> Vec<Clip> {
    store
        .entries()
        .iter()
        .map(|e| Clip {
            id: e.id,
            mime: e.mime.clone(),
            preview: e.preview.clone(),
            image: (!crate::clipboard::is_text(&e.mime)).then(|| ImageInfo {
                format: e.mime.trim_start_matches("image/").to_string(),
                size: e.size,
            }),
            path: store.path(e.id),
            len: e.len,
            created: e.created,
        })
        .collect()
}

/// How long ago `then` was, as the preview says it.
pub fn ago(now: u64, then: u64) -> String {
    let seconds = now.saturating_sub(then);
    let plural = |n: u64, unit: &str| format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" });
    match seconds {
        0..=59 => "Just now".to_string(),
        60..=3599 => plural(seconds / 60, "minute"),
        3600..=86_399 => plural(seconds / 3600, "hour"),
        86_400..=172_799 => "Yesterday".to_string(),
        _ => plural(seconds / 86_400, "day"),
    }
}

/// A byte count for people.
pub fn bytes(len: u64) -> String {
    match len {
        0..=1023 => format!("{len} B"),
        1024..=1_048_575 => format!("{:.1} KB", len as f64 / 1024.0),
        _ => format!("{:.1} MB", len as f64 / 1_048_576.0),
    }
}

/// The start of a text entry, at most `max` bytes, and whether there was
/// more.
pub fn read_text(path: &Path, max: u64) -> (String, bool) {
    use std::io::Read;
    let mut data = Vec::new();
    let read = std::fs::File::open(path).and_then(|f| f.take(max + 1).read_to_end(&mut data));
    if read.is_err() {
        return (String::new(), false);
    }
    let more = data.len() as u64 > max;
    data.truncate(max as usize);
    (String::from_utf8_lossy(&data).into_owned(), more)
}

/// An image entry fitted into `width`×`height` pixels for the preview.
/// Blocking: call it off the event loop.
pub fn preview(path: &Path, width: u32, height: u32) -> Option<crate::icons::IconImage> {
    let image = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.decode().ok()?;
    let image = if image.width() > width || image.height() > height {
        image.thumbnail(width, height)
    } else {
        image
    };
    premultiplied(image.to_rgba8())
}

fn premultiplied(image: image::RgbaImage) -> Option<crate::icons::IconImage> {
    let (width, height) = image.dimensions();
    let mut pixels = image.into_raw();
    for px in pixels.chunks_exact_mut(4) {
        let a = u16::from(px[3]);
        for c in &mut px[..3] {
            *c = (u16::from(*c) * a / 255) as u8;
        }
    }
    Some(crate::icons::IconImage { width, height, pixels: pixels.into() })
}

/// An image entry fitted into `size`² pixels, premultiplied, for its row.
/// Blocking: call it off the event loop.
pub fn thumbnail(path: &Path, size: u32) -> Option<crate::icons::IconImage> {
    // The files are named by id, so the format comes from the content.
    let image = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.decode().ok()?;
    premultiplied(image.thumbnail(size, size).to_rgba8())
}

/// The command that puts `clip` back on the clipboard, as the type it was
/// copied in (text as UTF-8). Values reach the script as `$1`/`$2`, never
/// inside it.
pub fn restore_command(clip: &Clip) -> Vec<String> {
    let mime = if crate::clipboard::is_text(&clip.mime) { crate::clipboard::TEXT } else { clip.mime.as_str() };
    let path = clip.path.to_string_lossy();
    ["sh", "-c", r#"exec wl-copy --type "$1" < "$2""#, "sh", mime, &path].map(String::from).to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_passes_values_as_arguments() {
        let clip = Clip {
            id: 3,
            mime: "UTF8_STRING".into(),
            preview: "x".into(),
            image: None,
            path: PathBuf::from("/s/clipboard/3"),
            len: 1,
            created: 0,
        };
        let command = restore_command(&clip);
        assert_eq!(&command[4..], ["text/plain;charset=utf-8", "/s/clipboard/3"], "text goes back as UTF-8");
        assert!(!command[2].contains("/s/"), "never interpolated");
    }

    #[test]
    fn times_and_sizes_read_naturally() {
        assert_eq!(ago(100, 90), "Just now");
        assert_eq!(ago(1000, 880), "2 minutes ago");
        assert_eq!(ago(3600, 0), "1 hour ago");
        assert_eq!(ago(100_000, 0), "Yesterday");
        assert_eq!(ago(10 * 86_400, 0), "10 days ago");
        assert_eq!(bytes(821), "821 B");
        assert_eq!(bytes(1536), "1.5 KB");
        assert_eq!(bytes(3 * 1_048_576), "3.0 MB");
    }
}
