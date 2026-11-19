//! The emoji picker's table, compiled in (`emoji.txt`, about 40 KB) and read
//! into a list only while the picker is open.

/// One emoji and what it is called.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Emoji {
    pub glyph: &'static str,
    pub name: &'static str,
    pub group: &'static str,
}

const TABLE: &str = include_str!("emoji.txt");

/// Every emoji, in Unicode's order (faces first).
pub fn all() -> Vec<Emoji> {
    let mut group = "";
    let mut out = Vec::with_capacity(2000);
    for line in TABLE.lines() {
        if let Some(name) = line.strip_prefix("# ") {
            // The licence header's lines come first and are overwritten by
            // the first real group before any emoji.
            group = name;
        } else if let Some((glyph, name)) = line.split_once('\t') {
            out.push(Emoji { glyph, name, group });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_parses_with_groups() {
        let all = all();
        assert!(all.len() > 1800, "{}", all.len());
        assert_eq!(all[0], Emoji { glyph: "😀", name: "grinning face", group: "Smileys & Emotion" });
        let heart = all.iter().find(|e| e.name == "red heart").unwrap();
        assert_eq!(heart.group, "Smileys & Emotion");
        assert!(all.iter().all(|e| !e.name.contains("skin tone")));
        assert!(all.iter().any(|e| e.group == "Flags"));
    }
}
