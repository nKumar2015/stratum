//! Clipboard entries on disk: one file per entry, named by its id, and an
//! index of them newest first, in `$XDG_STATE_HOME/stratum/clipboard/`.
//! The directory is private (0700, files 0600): a clipboard holds
//! passwords that were pasted, tokens and private messages.
//!
//! Only the index stays in memory (a short preview per entry); the data is
//! read from its file when it is restored or shown as a thumbnail.

use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Preview length kept in the index.
const PREVIEW_CHARS: usize = 160;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub id: u64,
    pub hash: u64,
    pub mime: String,
    pub len: u64,
    /// Seconds since the epoch.
    pub created: u64,
    /// The text, on one line; empty for an image.
    pub preview: String,
    /// An image's size, when its header could be read.
    pub size: Option<(u32, u32)>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Index {
    next_id: u64,
    /// Newest first.
    entries: Vec<Entry>,
}

pub struct Store {
    dir: PathBuf,
    index: Index,
    max_entries: usize,
}

impl Store {
    /// The shell's store, at `$XDG_STATE_HOME/stratum/clipboard`.
    pub fn default_dir() -> Option<PathBuf> {
        crate::paths::state_file("clipboard")
    }

    /// Open the store in `dir`, keeping at most `max_entries`. An index that
    /// cannot be read starts empty; entries whose file is gone are dropped.
    pub fn open(dir: &Path, max_entries: usize) -> Self {
        let mut index: Index = std::fs::read(dir.join("index.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        index.entries.retain(|e| dir.join(e.id.to_string()).is_file());
        let mut store = Self { dir: dir.to_path_buf(), index, max_entries: max_entries.max(1) };
        store.trim();
        store
    }

    pub fn entries(&self) -> &[Entry] {
        &self.index.entries
    }

    pub fn path(&self, id: u64) -> PathBuf {
        self.dir.join(id.to_string())
    }

    #[cfg(test)]
    pub fn get(&self, id: u64) -> Option<&Entry> {
        self.index.entries.iter().find(|e| e.id == id)
    }

    pub fn set_max_entries(&mut self, max: usize) {
        self.max_entries = max.max(1);
        self.trim();
        let _ = self.save();
    }

    /// Record a copy. A copy already in the history moves to the top
    /// instead of being stored twice. Returns its id.
    pub fn add(&mut self, mime: &str, data: &[u8], now: u64) -> std::io::Result<u64> {
        let hash = super::hash(data);
        let len = data.len() as u64;
        if let Some(pos) = self.index.entries.iter().position(|e| e.hash == hash && e.len == len && e.mime == mime) {
            let mut entry = self.index.entries.remove(pos);
            entry.created = now;
            let id = entry.id;
            self.index.entries.insert(0, entry);
            self.save()?;
            return Ok(id);
        }
        self.ensure_dir()?;
        let id = self.index.next_id;
        self.index.next_id += 1;
        write_private(&self.path(id), data)?;
        let (preview, size) = if super::is_text(mime) {
            (super::preview(data, PREVIEW_CHARS), None)
        } else {
            let size = image::ImageReader::new(std::io::Cursor::new(data))
                .with_guessed_format()
                .ok()
                .and_then(|r| r.into_dimensions().ok());
            (String::new(), size)
        };
        self.index.entries.insert(0, Entry { id, hash, mime: mime.to_string(), len, created: now, preview, size });
        self.trim();
        self.save()?;
        Ok(id)
    }

    pub fn delete(&mut self, id: u64) -> std::io::Result<()> {
        self.index.entries.retain(|e| e.id != id);
        let _ = std::fs::remove_file(self.path(id));
        self.save()
    }

    /// Forget everything.
    pub fn clear(&mut self) -> std::io::Result<()> {
        for entry in std::mem::take(&mut self.index.entries) {
            let _ = std::fs::remove_file(self.path(entry.id));
        }
        self.save()
    }

    fn trim(&mut self) {
        while self.index.entries.len() > self.max_entries {
            if let Some(old) = self.index.entries.pop() {
                let _ = std::fs::remove_file(self.path(old.id));
            }
        }
    }

    fn ensure_dir(&self) -> std::io::Result<()> {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&self.dir)
    }

    fn save(&self) -> std::io::Result<()> {
        self.ensure_dir()?;
        let bytes = serde_json::to_vec(&self.index).map_err(std::io::Error::other)?;
        let tmp = self.dir.join("index.json.tmp");
        write_private(&tmp, &bytes)?;
        std::fs::rename(tmp, self.dir.join("index.json"))
    }
}

/// Write `data` to a new file readable by the user alone.
fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    file.write_all(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("stratum-clip-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn adds_newest_first_and_dedupes_by_content() {
        let dir = scratch("dedupe");
        let mut store = Store::open(&dir, 10);
        let a = store.add("text/plain;charset=utf-8", b"first", 1).unwrap();
        let b = store.add("text/plain;charset=utf-8", b"second", 2).unwrap();
        assert_eq!(store.entries().iter().map(|e| e.id).collect::<Vec<_>>(), [b, a]);
        assert_eq!(store.add("text/plain;charset=utf-8", b"first", 3).unwrap(), a, "a repeat moves up");
        assert_eq!(store.entries().iter().map(|e| e.id).collect::<Vec<_>>(), [a, b]);
        assert_eq!(std::fs::read(store.path(a)).unwrap(), b"first");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn keeps_at_most_max_entries_and_survives_reopening() {
        let dir = scratch("trim");
        let mut store = Store::open(&dir, 2);
        let oldest = store.add("text/plain", b"1", 1).unwrap();
        store.add("text/plain", b"2", 2).unwrap();
        store.add("text/plain", b"3", 3).unwrap();
        assert_eq!(store.entries().len(), 2);
        assert!(!store.path(oldest).exists(), "a trimmed entry's file is removed");
        let reopened = Store::open(&dir, 2);
        assert_eq!(reopened.entries(), store.entries());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn files_are_private_and_deletes_remove_them() {
        let dir = scratch("private");
        let mut store = Store::open(&dir, 5);
        let id = store.add("text/plain", b"secret-ish", 1).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&store.path(id)), 0o600);
        assert_eq!(mode(&dir.join("index.json")), 0o600);
        store.delete(id).unwrap();
        assert!(store.entries().is_empty() && !store.path(id).exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn images_keep_their_size_and_no_text() {
        let dir = scratch("image");
        let mut store = Store::open(&dir, 5);
        let mut png = Vec::new();
        image::RgbaImage::new(7, 3).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let id = store.add("image/png", &png, 1).unwrap();
        let entry = store.get(id).unwrap();
        assert_eq!((entry.size, entry.preview.as_str()), (Some((7, 3)), ""));
        let _ = std::fs::remove_dir_all(dir);
    }
}
