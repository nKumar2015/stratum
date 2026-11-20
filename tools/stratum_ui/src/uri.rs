//! `file://` URIs: paths percent-encoded as RFC 8089 and the XDG specs
//! (trash info, `text/uri-list`, the FileChooser portal) write them.

use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

/// Bytes left as they are in a path's URI.
fn plain(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"-._~/!$&'()*+,;=:@".contains(&b)
}

/// Percent-encode `bytes`, leaving `/` and the unreserved characters.
pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        if plain(b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Undo percent-encoding; `None` for a bad escape.
pub fn decode(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Some(out)
}

/// `file:///a%20b` for `/a b`.
pub fn from_path(path: &Path) -> String {
    format!("file://{}", encode(path.as_os_str().as_bytes()))
}

/// The local path a `file://` URI names (with an empty or `localhost` host).
pub fn to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    Some(PathBuf::from(std::ffi::OsString::from_vec(decode(rest)?)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_round_trip() {
        let path = Path::new("/home/me/A file #1 (ü).txt");
        let uri = from_path(path);
        assert_eq!(uri, "file:///home/me/A%20file%20%231%20(%C3%BC).txt");
        assert_eq!(to_path(&uri).as_deref(), Some(path));
        assert_eq!(to_path("file://localhost/tmp/x").as_deref(), Some(Path::new("/tmp/x")));
        assert_eq!(to_path("https://example.com/x"), None);
        assert_eq!(to_path("file:///bad%zz"), None);
    }
}
