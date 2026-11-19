//! The clipboard history, kept by the shell itself: it watches the
//! selection through `ext-data-control-v1` (`wayland/state/clipboard.rs`),
//! reads each new copy on a thread, and keeps it in a [`store::Store`] the
//! launcher's clipboard mode shows. Restoring an entry hands it to
//! `wl-copy`.

pub mod store;

/// Marks a copy a password manager made; such copies are never recorded.
pub const SECRET_HINT: &str = "x-kde-passwordManagerHint";

/// The text type a restored text entry is offered as.
pub const TEXT: &str = "text/plain;charset=utf-8";

/// Which of a copy's types to keep, or `None` to keep nothing: a password
/// manager's copy, or one with no text or image (a file manager's cut
/// alone, application-private data).
pub fn choose(mimes: &[String]) -> Option<&str> {
    if mimes.iter().any(|m| m == SECRET_HINT) {
        return None;
    }
    let find = |wanted: &str| mimes.iter().find(|m| m.eq_ignore_ascii_case(wanted)).map(String::as_str);
    // An image wins over text: copying an image in a browser also offers
    // its HTML.
    ["image/png", "image/jpeg"]
        .into_iter()
        .find_map(find)
        .or_else(|| mimes.iter().find(|m| m.starts_with("image/")).map(String::as_str))
        .or_else(|| {
            ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain", "STRING", "TEXT"].into_iter().find_map(find)
        })
}

/// Whether `mime` is kept as text.
pub fn is_text(mime: &str) -> bool {
    !mime.starts_with("image/")
}

/// One line of at most `max` characters, for the list.
pub fn preview(data: &[u8], max: usize) -> String {
    let text = String::from_utf8_lossy(&data[..data.len().min(max * 4)]);
    let mut out = String::new();
    for word in text.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
        if out.chars().count() >= max {
            break;
        }
    }
    out.chars().take(max).collect()
}

/// FNV-1a: stable across builds, unlike `DefaultHasher`, so the index's
/// hashes still match after an update.
pub fn hash(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mimes(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn images_beat_text_and_secrets_are_skipped() {
        assert_eq!(choose(&mimes(&["text/html", "image/png"])), Some("image/png"));
        assert_eq!(choose(&mimes(&["UTF8_STRING", "text/plain;charset=utf-8"])), Some("text/plain;charset=utf-8"));
        assert_eq!(choose(&mimes(&["TEXT", "STRING"])), Some("STRING"));
        assert_eq!(choose(&mimes(&["text/plain", SECRET_HINT])), None);
        assert_eq!(choose(&mimes(&["application/x-kde-cutselection"])), None);
        assert_eq!(choose(&mimes(&["image/webp"])), Some("image/webp"));
    }

    #[test]
    fn previews_are_one_short_line() {
        assert_eq!(preview(b"  line one\n\tline two  ", 50), "line one line two");
        assert_eq!(preview(b"abcdefghij", 4), "abcd");
        assert_eq!(preview(&[0xff, b'a'], 10), "\u{fffd}a");
    }

    #[test]
    fn hashes_are_stable() {
        assert_eq!(hash(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(hash(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_ne!(hash(b"ab"), hash(b"ba"));
    }
}
