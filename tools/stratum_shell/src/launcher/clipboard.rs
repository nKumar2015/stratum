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
        })
        .collect()
}

/// An image entry fitted into `size`² pixels, premultiplied, for its row.
/// Blocking: call it off the event loop.
pub fn thumbnail(path: &Path, size: u32) -> Option<crate::icons::IconImage> {
    // The files are named by id, so the format comes from the content.
    let image = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.decode().ok()?;
    let image = image.thumbnail(size, size).to_rgba8();
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
        };
        let command = restore_command(&clip);
        assert_eq!(&command[4..], ["text/plain;charset=utf-8", "/s/clipboard/3"], "text goes back as UTF-8");
        assert!(!command[2].contains("/s/"), "never interpolated");
    }
}
