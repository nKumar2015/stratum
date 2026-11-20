//! File types from shared-mime-info: the glob patterns that name a type
//! from a file name (`mime/globs2`), the types each one is a kind of
//! (`mime/subclasses`, so a Rust file opens in a text editor), and the
//! generic icons (`mime/generic-icons`). Content sniffing is not done: a
//! file without a known name is `application/octet-stream`, or
//! `text/plain` when it is short and reads as text.

use std::collections::HashMap;
use std::path::Path;

pub const DIRECTORY: &str = "inode/directory";
pub const UNKNOWN: &str = "application/octet-stream";

#[derive(Clone, Debug)]
struct Glob {
    weight: u32,
    pattern: String,
    mime: String,
    case_sensitive: bool,
}

#[derive(Clone, Debug, Default)]
pub struct MimeDb {
    globs: Vec<Glob>,
    /// Literal `*.ext` patterns, by lowercase extension, best first.
    by_extension: HashMap<String, Vec<usize>>,
    parents: HashMap<String, Vec<String>>,
    icons: HashMap<String, String>,
}

impl MimeDb {
    /// Read every data directory's database; earlier directories win.
    pub fn load() -> Self {
        let mut db = MimeDb::default();
        for dir in stratum_ui::icons::data_dirs().iter().rev() {
            let read = |name: &str| std::fs::read_to_string(dir.join("mime").join(name)).unwrap_or_default();
            db.add_globs(&read("globs2"));
            db.add_parents(&read("subclasses"));
            db.add_icons(&read("generic-icons"));
        }
        db.index();
        db
    }

    /// `weight:type:pattern[:flags]` lines.
    pub fn add_globs(&mut self, text: &str) {
        for line in text.lines().filter(|l| !l.starts_with('#')) {
            let mut parts = line.splitn(4, ':');
            let (Some(weight), Some(mime), Some(pattern)) = (parts.next(), parts.next(), parts.next()) else { continue };
            let case_sensitive = parts.next().is_some_and(|flags| flags.split(',').any(|f| f == "cs"));
            self.globs.push(Glob {
                weight: weight.parse().unwrap_or(50),
                pattern: pattern.to_string(),
                mime: mime.to_string(),
                case_sensitive,
            });
        }
    }

    pub fn add_parents(&mut self, text: &str) {
        for line in text.lines() {
            if let Some((child, parent)) = line.split_once(' ') {
                self.parents.entry(child.to_string()).or_default().push(parent.to_string());
            }
        }
    }

    pub fn add_icons(&mut self, text: &str) {
        for line in text.lines() {
            if let Some((mime, icon)) = line.split_once(':') {
                self.icons.insert(mime.to_string(), icon.to_string());
            }
        }
    }

    /// Index the literal extension patterns, heaviest and longest first.
    pub fn index(&mut self) {
        self.by_extension.clear();
        for (i, glob) in self.globs.iter().enumerate() {
            if let Some(ext) = glob.pattern.strip_prefix("*.").filter(|e| !e.contains(['*', '?', '['])) {
                self.by_extension.entry(ext.to_lowercase()).or_default().push(i);
            }
        }
        let globs = &self.globs;
        for list in self.by_extension.values_mut() {
            list.sort_by_key(|&i| std::cmp::Reverse(globs[i].weight));
        }
    }

    /// The type a file name says, if any pattern matches it.
    pub fn guess(&self, name: &str) -> Option<&str> {
        let lower = name.to_lowercase();
        // The longest extension first: `archive.tar.gz` is a tarball.
        let mut dot = 0;
        while let Some(i) = lower[dot..].find('.') {
            let ext = &lower[dot + i + 1..];
            if let Some(found) = self.by_extension.get(ext) {
                let glob = found
                    .iter()
                    .map(|&g| &self.globs[g])
                    .find(|g| !g.case_sensitive || name.ends_with(&g.pattern[1..]));
                if let Some(glob) = glob {
                    return Some(&glob.mime);
                }
            }
            dot += i + 1;
        }
        // The other patterns (`Makefile`, `README*`), heaviest first.
        self.globs
            .iter()
            .filter(|g| !g.pattern.starts_with("*.") || g.pattern[2..].contains(['*', '?', '[']))
            .filter(|g| if g.case_sensitive { glob_match(&g.pattern, name) } else { glob_match(&g.pattern.to_lowercase(), &lower) })
            .max_by_key(|g| (g.weight, g.pattern.len()))
            .map(|g| g.mime.as_str())
    }

    /// A file's type: a directory, its name's, or a guess from its start.
    pub fn of(&self, path: &Path, is_dir: bool) -> String {
        if is_dir {
            return DIRECTORY.into();
        }
        let name = path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
        if let Some(mime) = self.guess(&name) {
            return mime.to_string();
        }
        if looks_like_text(path) { "text/plain".into() } else { UNKNOWN.into() }
    }

    /// `mime` and every type it is a kind of, nearest first.
    pub fn ancestors(&self, mime: &str) -> Vec<String> {
        let mut out = vec![mime.to_string()];
        let mut i = 0;
        while i < out.len() {
            for parent in self.parents.get(&out[i]).into_iter().flatten() {
                if !out.contains(parent) {
                    out.push(parent.clone());
                }
            }
            i += 1;
        }
        // Every text file is a kind of text/plain, and every file a stream of
        // bytes, even where the database does not say so.
        if mime.starts_with("text/") && !out.iter().any(|m| m == "text/plain") {
            out.push("text/plain".into());
        }
        if mime != DIRECTORY && !out.iter().any(|m| m == UNKNOWN) {
            out.push(UNKNOWN.into());
        }
        out
    }

    /// Icon names to try for `mime`, best first.
    pub fn icon_names(&self, mime: &str) -> Vec<String> {
        if mime == DIRECTORY {
            return vec!["folder".into()];
        }
        let mut names = vec![mime.replace('/', "-")];
        if let Some(generic) = self.icons.get(mime) {
            names.push(generic.clone());
        }
        let media = mime.split('/').next().unwrap_or("");
        names.push(format!("{media}-x-generic"));
        names.push("text-x-generic".into());
        names
    }
}

/// `*`, `?` and `[...]` matching, as fnmatch.
pub fn glob_match(pattern: &str, name: &str) -> bool {
    fn go(p: &[char], n: &[char]) -> bool {
        match p.first() {
            None => n.is_empty(),
            Some('*') => (0..=n.len()).any(|i| go(&p[1..], &n[i..])),
            Some('?') => !n.is_empty() && go(&p[1..], &n[1..]),
            Some('[') => {
                let Some(end) = p.iter().position(|c| *c == ']') else { return false };
                let Some(c) = n.first() else { return false };
                let set = &p[1..end];
                let (negate, set) = match set.first() {
                    Some('!') | Some('^') => (true, &set[1..]),
                    _ => (false, set),
                };
                let mut hit = false;
                let mut i = 0;
                while i < set.len() {
                    if i + 2 < set.len() && set[i + 1] == '-' {
                        hit |= (set[i]..=set[i + 2]).contains(c);
                        i += 3;
                    } else {
                        hit |= set[i] == *c;
                        i += 1;
                    }
                }
                hit != negate && go(&p[end + 1..], &n[1..])
            }
            Some(c) => n.first() == Some(c) && go(&p[1..], &n[1..]),
        }
    }
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    go(&p, &n)
}

/// A short file whose first 1 KB is valid UTF-8 without NULs.
fn looks_like_text(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(path) else { return false };
    let mut head = [0u8; 1024];
    let Ok(n) = file.read(&mut head) else { return false };
    if n == 0 {
        return true;
    }
    let head = &head[..n];
    // A multibyte character cut at the end is still text.
    let valid = match std::str::from_utf8(head) {
        Ok(_) => true,
        Err(err) => err.error_len().is_none() && err.valid_up_to() + 4 > n,
    };
    valid && !head.contains(&0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> MimeDb {
        let mut db = MimeDb::default();
        db.add_globs(
            "# comment\n50:application/gzip:*.gz\n55:application/x-compressed-tar:*.tar.gz\n50:image/png:*.png\n\
             50:text/rust:*.rs\n10:text/x-readme:README*\n50:text/x-makefile:Makefile\n50:text/x-c:*.C:cs\n50:text/x-csrc:*.c\n",
        );
        db.add_parents("text/rust text/plain\napplication/x-compressed-tar application/gzip\n");
        db.add_icons("application/gzip:package-x-generic\n");
        db.index();
        db
    }

    #[test]
    fn names_give_types() {
        let db = db();
        assert_eq!(db.guess("photo.PNG"), Some("image/png"), "case-insensitive");
        assert_eq!(db.guess("backup.tar.gz"), Some("application/x-compressed-tar"), "the longest extension");
        assert_eq!(db.guess("notes.gz"), Some("application/gzip"));
        assert_eq!(db.guess("README.md"), Some("text/x-readme"));
        assert_eq!(db.guess("Makefile"), Some("text/x-makefile"));
        assert_eq!(db.guess("main.C"), Some("text/x-c"), "case-sensitive pattern");
        assert_eq!(db.guess("main.c"), Some("text/x-csrc"));
        assert_eq!(db.guess("noextension"), None);
    }

    #[test]
    fn types_have_ancestors_and_icons() {
        let db = db();
        assert_eq!(db.ancestors("text/rust"), ["text/rust", "text/plain", UNKNOWN]);
        assert_eq!(db.ancestors("text/x-makefile"), ["text/x-makefile", "text/plain", UNKNOWN]);
        assert_eq!(db.icon_names("application/gzip"), ["application-gzip", "package-x-generic", "application-x-generic", "text-x-generic"]);
        assert_eq!(db.icon_names(DIRECTORY), ["folder"]);
    }

    #[test]
    fn globs() {
        assert!(glob_match("*.[ch]", "a.c"));
        assert!(!glob_match("*.[!ch]", "a.c"));
        assert!(glob_match("file?.txt", "file1.txt"));
        assert!(!glob_match("file?.txt", "file10.txt"));
    }
}
