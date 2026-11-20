//! A directory's entries: reading them, sorting (folders first, names in
//! natural order), filtering, and the sizes and dates the views show.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::mime::MimeDb;

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    /// A directory, or a link to one.
    pub is_dir: bool,
    pub is_link: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub mime: String,
}

impl Entry {
    pub fn hidden(&self) -> bool {
        self.name.starts_with('.') || self.name.ends_with('~')
    }
}

/// Read `dir`. Entries whose metadata cannot be read (a dangling link) are
/// kept as files of size 0.
pub fn list(dir: &Path, mime: &MimeDb) -> std::io::Result<Vec<Entry>> {
    let mut out = Vec::new();
    for item in std::fs::read_dir(dir)? {
        let Ok(item) = item else { continue };
        let path = item.path();
        let name = item.file_name().to_string_lossy().into_owned();
        let link = item.file_type().map(|t| t.is_symlink()).unwrap_or(false);
        let meta = std::fs::metadata(&path).or_else(|_| std::fs::symlink_metadata(&path)).ok();
        let is_dir = meta.as_ref().is_some_and(|m| m.is_dir());
        out.push(Entry {
            mime: mime.of(&path, is_dir),
            name,
            is_dir,
            is_link: link,
            size: meta.as_ref().map_or(0, |m| if m.is_dir() { 0 } else { m.len() }),
            modified: meta.and_then(|m| m.modified().ok()),
            path,
        });
    }
    Ok(out)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SortKey {
    #[default]
    Name,
    Size,
    Modified,
    Type,
}

/// Folders first, then by `key`, ties broken by name.
pub fn sort(entries: &mut [Entry], key: SortKey, descending: bool) {
    entries.sort_by(|a, b| {
        let folders = b.is_dir.cmp(&a.is_dir);
        let by_key = match key {
            SortKey::Name => Ordering::Equal,
            SortKey::Size => a.size.cmp(&b.size),
            SortKey::Modified => a.modified.cmp(&b.modified),
            SortKey::Type => a.mime.cmp(&b.mime),
        };
        let by_key = by_key.then_with(|| natural(&a.name, &b.name));
        folders.then(if descending { by_key.reverse() } else { by_key })
    });
}

/// Names as people order them: case-insensitive, with runs of digits
/// compared as numbers (`file2` before `file10`).
pub fn natural(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars<'_>>| {
                    let mut digits = String::new();
                    while let Some(c) = it.peek().filter(|c| c.is_ascii_digit()) {
                        digits.push(*c);
                        it.next();
                    }
                    digits
                };
                let (x, y) = (take(&mut a), take(&mut b));
                let (tx, ty) = (x.trim_start_matches('0'), y.trim_start_matches('0'));
                let order = tx.len().cmp(&ty.len()).then_with(|| tx.cmp(ty)).then_with(|| x.len().cmp(&y.len()));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                let order = x.to_lowercase().cmp(y.to_lowercase());
                if order != Ordering::Equal {
                    return order;
                }
                a.next();
                b.next();
            }
        }
    }
}

/// The entries to show: hidden ones only when asked, and those whose name
/// holds `query` (case-insensitive) when there is one.
pub fn visible<'a>(entries: &'a [Entry], show_hidden: bool, query: &str) -> Vec<&'a Entry> {
    let query = query.trim().to_lowercase();
    entries
        .iter()
        .filter(|e| show_hidden || !e.hidden())
        .filter(|e| query.is_empty() || e.name.to_lowercase().contains(&query))
        .collect()
}

/// A size for people: `512 B`, `1.5 KB`, `3.2 GB`.
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 { format!("{bytes} B") } else { format!("{value:.1} {}", UNITS[unit]) }
}

/// When a file changed: the time today, `Yesterday`, the day this year,
/// else the date.
pub fn format_time(time: SystemTime, now: chrono::DateTime<chrono::Local>) -> String {
    let time: chrono::DateTime<chrono::Local> = time.into();
    let (day, today) = (time.date_naive(), now.date_naive());
    if day == today {
        time.format("%H:%M").to_string()
    } else if today.pred_opt() == Some(day) {
        format!("Yesterday {}", time.format("%H:%M"))
    } else if chrono::Datelike::year(&day) == chrono::Datelike::year(&today) {
        time.format("%-d %b").to_string()
    } else {
        time.format("%-d %b %Y").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, is_dir: bool, size: u64) -> Entry {
        Entry { name: name.into(), path: PathBuf::from(name), is_dir, is_link: false, size, modified: None, mime: String::new() }
    }

    #[test]
    fn natural_order() {
        let mut names = vec!["file10", "File2", "file1", "a", "file02"];
        names.sort_by(|a, b| natural(a, b));
        assert_eq!(names, ["a", "file1", "File2", "file02", "file10"]);
    }

    #[test]
    fn folders_first_then_the_key() {
        let mut list = vec![entry("b.txt", false, 5), entry("Zdir", true, 0), entry("a.txt", false, 50), entry("adir", true, 0)];
        sort(&mut list, SortKey::Name, false);
        assert_eq!(list.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["adir", "Zdir", "a.txt", "b.txt"]);
        sort(&mut list, SortKey::Size, true);
        assert_eq!(list.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["Zdir", "adir", "a.txt", "b.txt"]);
    }

    #[test]
    fn hidden_and_search() {
        let list = vec![entry(".git", true, 0), entry("notes.txt", false, 1), entry("backup~", false, 1), entry("Notes.md", false, 1)];
        assert_eq!(visible(&list, false, "").len(), 2);
        assert_eq!(visible(&list, true, "").len(), 4);
        assert_eq!(visible(&list, false, "NOTES").len(), 2);
    }

    #[test]
    fn sizes() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(5 * 1024 * 1024 * 1024), "5.0 GB");
    }
}
