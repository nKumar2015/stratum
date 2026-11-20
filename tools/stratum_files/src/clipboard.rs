//! Files on the clipboard, as `text/uri-list` through `wl-copy` and
//! `wl-paste`, so a copy in one window pastes in another (and into other
//! apps that take files). Whether it was a cut is kept beside it, in the
//! runtime directory: the next paste of the same list moves rather than
//! copies, and forgets it.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const URI_LIST: &str = "text/uri-list";

fn cut_marker() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?);
    Some(dir.join("stratum-files-cut"))
}

/// `text/uri-list` text: one URI per line, CRLF as RFC 2483 says.
pub fn encode(paths: &[PathBuf]) -> String {
    paths.iter().map(|p| stratum_ui::uri::from_path(p) + "\r\n").collect()
}

/// The local paths in a `text/uri-list` (or GNOME's copied-files list,
/// whose first line says `copy` or `cut`), and whether it was a cut.
pub fn decode(text: &str) -> (Vec<PathBuf>, bool) {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).peekable();
    let cut = match lines.peek() {
        Some(&"cut") => {
            lines.next();
            true
        }
        Some(&"copy") => {
            lines.next();
            false
        }
        _ => false,
    };
    (lines.filter_map(stratum_ui::uri::to_path).collect(), cut)
}

fn wl_copy(mime: &str, text: &str) -> Result<(), String> {
    let mut child = Command::new("wl-copy")
        .args(["--type", mime])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .map_err(|e| format!("wl-copy: {e}"))?;
    child.stdin.take().ok_or("wl-copy: no stdin")?.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    // wl-copy forks to serve the selection and exits.
    child.wait().map_err(|e| e.to_string())?;
    Ok(())
}

/// Put `paths` on the clipboard.
pub fn put(paths: &[PathBuf], cut: bool) -> Result<(), String> {
    let list = encode(paths);
    wl_copy(URI_LIST, &list)?;
    if let Some(marker) = cut_marker() {
        if cut {
            let _ = std::fs::write(marker, &list);
        } else {
            let _ = std::fs::remove_file(marker);
        }
    }
    Ok(())
}

pub fn put_text(text: &str) -> Result<(), String> {
    wl_copy("text/plain;charset=utf-8", text)
}

/// The files on the clipboard, and whether they were cut.
pub fn get() -> Result<(Vec<PathBuf>, bool), String> {
    let types = Command::new("wl-paste").arg("--list-types").output().map_err(|e| format!("wl-paste: {e}"))?;
    let types = String::from_utf8_lossy(&types.stdout);
    let mime = ["x-special/gnome-copied-files", URI_LIST].into_iter().find(|m| types.lines().any(|t| t == *m));
    let Some(mime) = mime else { return Ok((Vec::new(), false)) };
    let out = Command::new("wl-paste").args(["--no-newline", "--type", mime]).output().map_err(|e| format!("wl-paste: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let (paths, cut) = decode(&text);
    let marked = cut_marker().and_then(|m| std::fs::read_to_string(m).ok()).is_some_and(|m| decode(&m).0 == paths);
    Ok((paths, cut || marked))
}

pub fn forget_cut() {
    if let Some(marker) = cut_marker() {
        let _ = std::fs::remove_file(marker);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_lists() {
        let paths = vec![PathBuf::from("/a b/c"), PathBuf::from("/d")];
        let text = encode(&paths);
        assert_eq!(text, "file:///a%20b/c\r\nfile:///d\r\n");
        assert_eq!(decode(&text), (paths.clone(), false));
        assert_eq!(decode("cut\nfile:///d\n"), (vec![PathBuf::from("/d")], true));
        assert_eq!(decode("# comment\nhttps://x\nfile:///d"), (vec![PathBuf::from("/d")], false));
    }
}
