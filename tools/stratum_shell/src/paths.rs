//! Where the shell keeps its files, and how it writes them.

use std::path::{Path, PathBuf};

/// `name` in the shell's state directory: `$XDG_STATE_HOME/stratum`, else
/// `~/.local/state/stratum`.
pub fn state_file(name: &str) -> Option<PathBuf> {
    let base = match std::env::var("XDG_STATE_HOME") {
        Ok(dir) if !dir.trim().is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(std::env::var("HOME").ok()?).join(".local/state"),
    };
    Some(base.join("stratum").join(name))
}

/// `$XDG_RUNTIME_DIR`, else `/tmp`.
pub fn runtime_dir() -> PathBuf {
    std::env::var("XDG_RUNTIME_DIR").map_or_else(|_| PathBuf::from("/tmp"), PathBuf::from)
}

/// Write `contents` to `path`, creating its directory. Written beside and
/// renamed, so a crash mid-write cannot leave half a file.
pub fn write_atomic(path: &Path, contents: impl AsRef<[u8]>) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)
}
