//! The clipboard viewer over cliphist. The shell only reads: `wl-paste
//! --watch cliphist store` keeps the history, `cliphist list` lists it,
//! `cliphist decode` fetches an entry and `cliphist delete` removes one.
//! Nothing is kept once the launcher closes.

use std::process::{Command, Stdio};

/// One history entry, as `cliphist list` prints it.
#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    /// The whole line, which `cliphist delete` takes.
    pub line: String,
    pub id: String,
    /// The text, or cliphist's description of binary data.
    pub preview: String,
    /// For an image: its format and size.
    pub image: Option<ImageInfo>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImageInfo {
    pub format: String,
    pub width: u32,
    pub height: u32,
}

/// The lines of `cliphist list`, newest first: `id<TAB>preview`.
pub fn parse_list(text: &str) -> Vec<Clip> {
    text.lines()
        .filter_map(|line| {
            let (id, preview) = line.split_once('\t')?;
            if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            Some(Clip { line: line.to_string(), id: id.to_string(), preview: preview.to_string(), image: image_info(preview) })
        })
        .collect()
}

/// `[[ binary data 12 KiB png 800x600 ]]` describes an image.
fn image_info(preview: &str) -> Option<ImageInfo> {
    let inner = preview.strip_prefix("[[ binary data ")?.strip_suffix(" ]]")?;
    let mut words = inner.split_whitespace().rev();
    let (width, height) = words.next()?.split_once('x')?;
    let format = words.next()?.to_string();
    Some(ImageInfo { format, width: width.parse().ok()?, height: height.parse().ok()? })
}

/// Run `cliphist list`. Blocking: call it off the event loop.
pub fn list() -> Result<Vec<Clip>, String> {
    let output = Command::new("cliphist")
        .arg("list")
        .stdin(Stdio::null())
        .output()
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => "cliphist is not installed".to_string(),
            _ => format!("cliphist failed: {err}"),
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // A history that was never written has no database yet.
        if stderr.contains("no such file") {
            return Ok(Vec::new());
        }
        return Err(format!("cliphist list failed: {}", stderr.trim()));
    }
    Ok(parse_list(&String::from_utf8_lossy(&output.stdout)))
}

/// An image entry decoded and fitted into `size`² pixels, premultiplied,
/// for its row. Blocking: call it off the event loop.
pub fn thumbnail(id: &str, size: u32) -> Option<crate::icons::IconImage> {
    if !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let output = Command::new("cliphist").args(["decode", id]).stdin(Stdio::null()).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let image = image::load_from_memory(&output.stdout).ok()?.thumbnail(size, size).to_rgba8();
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

/// The command that puts entry `id` back on the clipboard.
pub fn restore_command(id: &str) -> Option<Vec<String>> {
    id.bytes().all(|b| b.is_ascii_digit()).then(|| {
        ["sh", "-c", r#"cliphist decode "$1" | wl-copy"#, "sh", id].map(String::from).to_vec()
    })
}

/// The command that removes `line` (as listed) from the history.
pub fn delete_command(line: &str) -> Vec<String> {
    ["sh", "-c", r#"printf '%s\n' "$1" | cliphist delete"#, "sh", line].map(String::from).to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_text_and_image_entries() {
        let clips = parse_list("12\thello world\n11\t[[ binary data 31 KiB png 800x600 ]]\nbad line\n\n");
        assert_eq!(clips.len(), 2);
        assert_eq!(clips[0].id, "12");
        assert_eq!(clips[0].image, None);
        assert_eq!(clips[1].image, Some(ImageInfo { format: "png".into(), width: 800, height: 600 }));
        assert_eq!(clips[1].line, "11\t[[ binary data 31 KiB png 800x600 ]]");
    }

    #[test]
    fn commands_pass_values_as_arguments_not_code() {
        assert_eq!(restore_command("12; rm -rf ~"), None);
        let restore = restore_command("12").unwrap();
        assert_eq!(restore.last().map(String::as_str), Some("12"));
        let delete = delete_command("7\t$(touch x)");
        assert_eq!(delete.last().map(String::as_str), Some("7\t$(touch x)"), "passed as $1, never interpolated");
    }
}
