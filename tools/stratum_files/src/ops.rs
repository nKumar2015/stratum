//! Copying, moving and deleting, on a thread of their own with progress.
//! Nothing is overwritten: a name already taken gets ` (copy)`, ` (copy 2)`
//! and so on. A move tries a rename first and copies (then deletes) only
//! across file systems.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Copy,
    Move,
}

/// How a running operation is doing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
    pub current: String,
    pub finished: bool,
    /// What went wrong, file by file.
    pub errors: Vec<String>,
}

/// `name` with ` (copy)` or ` (copy N)` before its extension, the first not
/// taken in `dir`.
pub fn free_name(dir: &Path, name: &str) -> String {
    if !dir.join(name).exists() {
        return name.to_string();
    }
    // `archive.tar.gz` keeps `.tar.gz`; a dot file keeps its name whole.
    let split = match name.find('.') {
        Some(0) | None => name.len(),
        Some(i) => i,
    };
    let (stem, ext) = name.split_at(split);
    std::iter::once(format!("{stem} (copy){ext}"))
        .chain((2..).map(|i| format!("{stem} (copy {i}){ext}")))
        .find(|n| !dir.join(n).exists())
        .expect("some name is free")
}

/// Bytes under `path`, links not followed.
pub fn size_of(path: &Path) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(path) else { return 0 };
    if meta.is_dir() {
        std::fs::read_dir(path).map(|it| it.flatten().map(|e| size_of(&e.path())).sum()).unwrap_or(0)
    } else {
        meta.len()
    }
}

/// Delete `path` for good, a directory with everything in it.
pub fn remove(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(err) => Err(err),
    }
}

/// Run a copy or move of `sources` into `dest` on a thread, reporting
/// through `report` (called often; it should be cheap). `cancel` stops it
/// between files.
pub fn start(
    kind: Kind,
    sources: Vec<PathBuf>,
    dest: PathBuf,
    cancel: Arc<AtomicBool>,
    report: impl Fn(&Progress) + Send + 'static,
) {
    std::thread::spawn(move || {
        let mut progress = Progress { total: sources.iter().map(|s| size_of(s)).sum(), ..Progress::default() };
        report(&progress);
        for source in &sources {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let Some(name) = source.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
            // A folder into itself would never end.
            if dest.starts_with(source) {
                progress.errors.push(format!("{name}: cannot go into itself"));
                continue;
            }
            // Moving within the same folder changes nothing.
            if kind == Kind::Move && source.parent() == Some(dest.as_path()) {
                continue;
            }
            let target = dest.join(free_name(&dest, &name));
            progress.current = name.clone();
            let result = match kind {
                Kind::Move => match std::fs::rename(source, &target) {
                    Ok(()) => {
                        progress.done += size_of(&target);
                        Ok(())
                    }
                    Err(err) if err.raw_os_error() == Some(libc::EXDEV) => {
                        copy_tree(source, &target, &mut progress, &cancel, &report).and_then(|()| remove(source))
                    }
                    Err(err) => Err(err),
                },
                Kind::Copy => copy_tree(source, &target, &mut progress, &cancel, &report),
            };
            if let Err(err) = result {
                progress.errors.push(format!("{name}: {err}"));
            }
            report(&progress);
        }
        progress.finished = true;
        progress.current.clear();
        report(&progress);
    });
}

fn copy_tree(
    from: &Path,
    to: &Path,
    progress: &mut Progress,
    cancel: &AtomicBool,
    report: &impl Fn(&Progress),
) -> std::io::Result<()> {
    if cancel.load(Ordering::Relaxed) {
        return Ok(());
    }
    let meta = std::fs::symlink_metadata(from)?;
    if meta.file_type().is_symlink() {
        std::os::unix::fs::symlink(std::fs::read_link(from)?, to)?;
    } else if meta.is_dir() {
        std::fs::create_dir(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_tree(&entry.path(), &to.join(entry.file_name()), progress, cancel, report)?;
        }
        std::fs::set_permissions(to, meta.permissions())?;
    } else {
        // In chunks, so a large file reports as it goes.
        use std::io::{Read, Write};
        let mut input = std::fs::File::open(from)?;
        let mut output = std::fs::File::create(to)?;
        let mut buffer = vec![0u8; 1 << 20];
        loop {
            if cancel.load(Ordering::Relaxed) {
                drop(output);
                let _ = std::fs::remove_file(to);
                return Ok(());
            }
            let n = input.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            output.write_all(&buffer[..n])?;
            progress.done += n as u64;
            report(progress);
        }
        std::fs::set_permissions(to, meta.permissions())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("stratum-files-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn taken_names_get_copy_suffixes() {
        let dir = scratch("names");
        std::fs::write(dir.join("a.tar.gz"), "").unwrap();
        assert_eq!(free_name(&dir, "a.tar.gz"), "a (copy).tar.gz");
        std::fs::write(dir.join("a (copy).tar.gz"), "").unwrap();
        assert_eq!(free_name(&dir, "a.tar.gz"), "a (copy 2).tar.gz");
        std::fs::write(dir.join(".bashrc"), "").unwrap();
        assert_eq!(free_name(&dir, ".bashrc"), ".bashrc (copy)");
        assert_eq!(free_name(&dir, "new"), "new");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn copies_trees_and_moves() {
        let dir = scratch("copy");
        let src = dir.join("src");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("sub/f.txt"), "hello").unwrap();
        std::os::unix::fs::symlink("sub/f.txt", src.join("link")).unwrap();
        let dest = dir.join("dest");
        std::fs::create_dir(&dest).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        start(Kind::Copy, vec![src.clone()], dest.clone(), Arc::default(), move |p| {
            let _ = tx.send(p.clone());
        });
        let last = rx.iter().find(|p| p.finished).unwrap();
        assert!(last.errors.is_empty(), "{:?}", last.errors);
        assert_eq!(last.done, 5);
        assert_eq!(std::fs::read_to_string(dest.join("src/sub/f.txt")).unwrap(), "hello");
        assert!(std::fs::symlink_metadata(dest.join("src/link")).unwrap().file_type().is_symlink());

        // Again: no overwriting.
        let (tx, rx) = std::sync::mpsc::channel();
        start(Kind::Move, vec![src.clone()], dest.clone(), Arc::default(), move |p| {
            let _ = tx.send(p.clone());
        });
        assert!(rx.iter().find(|p| p.finished).unwrap().errors.is_empty());
        assert!(!src.exists());
        assert!(dest.join("src (copy)/sub/f.txt").exists());

        // Into itself.
        let (tx, rx) = std::sync::mpsc::channel();
        start(Kind::Copy, vec![dest.clone()], dest.join("src"), Arc::default(), move |p| {
            let _ = tx.send(p.clone());
        });
        assert_eq!(rx.iter().find(|p| p.finished).unwrap().errors.len(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
