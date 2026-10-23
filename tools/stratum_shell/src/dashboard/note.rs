//! The Focus tab's note: a small multi-line editor.
//!
//! The text wraps at word boundaries to the panel ([`layout`]) and keeps a
//! caret that the arrows, Home and End move by what is on screen: Up and
//! Down go to the same column of the wrapped line above or below, Home and
//! End to the ends of the wrapped line. Return breaks the line; Escape is
//! reported for the owner, which takes the focus away.

use smithay_client_toolkit::seat::keyboard::Keysym;

/// What a keypress did to the note.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteKey {
    Ignored,
    /// The text changed (save it).
    Changed,
    /// Only the caret moved.
    Moved,
    Escape,
}

/// The wrapped lines of `text`, as byte ranges without their line breaks,
/// each at most `width` wide unless a single word is wider than that (it is
/// then cut between characters).
pub fn layout(text: &str, width: f32, mut measure: impl FnMut(&str) -> f32) -> Vec<(usize, usize)> {
    let mut lines = Vec::new();
    let mut start = 0;
    for paragraph in text.split('\n') {
        let end = start + paragraph.len();
        wrap_paragraph(text, start, end, width, &mut measure, &mut lines);
        start = end + 1;
    }
    lines
}

fn wrap_paragraph(
    text: &str,
    from: usize,
    to: usize,
    width: f32,
    measure: &mut impl FnMut(&str) -> f32,
    lines: &mut Vec<(usize, usize)>,
) {
    let mut line = from;
    // Where the current line may break: after the last space seen.
    let mut word = from;
    let mut at = from;
    while at < to {
        let ch = text[at..].chars().next().unwrap_or(' ');
        let next = at + ch.len_utf8();
        // Trailing spaces stay on their line: they never push it over.
        if ch != ' ' && measure(&text[line..next]) > width && next - line > ch.len_utf8() {
            let cut = if word > line { word } else { at };
            lines.push((line, cut));
            line = cut;
            word = cut;
            continue;
        }
        if ch == ' ' {
            word = next;
        }
        at = next;
    }
    lines.push((line, to));
}

/// The visual line holding byte `cursor`: the last one starting at or
/// before it, so a caret at a wrap point sits at the start of the next.
pub fn line_of(lines: &[(usize, usize)], cursor: usize) -> usize {
    lines.iter().rposition(|(start, _)| *start <= cursor).unwrap_or(0)
}

#[derive(Clone, Debug, Default)]
pub struct Note {
    text: String,
    /// A byte offset on a character boundary.
    cursor: usize,
    /// The wrapped lines from the last paint, which the vertical keys use.
    lines: Vec<(usize, usize)>,
    /// The column Up and Down aim for, kept across short lines.
    goal: Option<usize>,
    /// The first visual line shown.
    pub scroll: usize,
}

impl Note {
    pub fn new(text: &str) -> Self {
        Self { text: text.to_string(), cursor: text.len(), ..Self::default() }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Put the caret at the end, as a click into the note does.
    pub fn caret_to_end(&mut self) {
        self.cursor = self.text.len();
        self.goal = None;
    }

    /// Lay out for `width`, keep the caret's line within `visible` lines, and
    /// return the lines.
    pub fn relayout(&mut self, width: f32, visible: usize, measure: impl FnMut(&str) -> f32) -> &[(usize, usize)] {
        self.lines = layout(&self.text, width, measure);
        let line = line_of(&self.lines, self.cursor);
        let visible = visible.max(1);
        if line < self.scroll {
            self.scroll = line;
        } else if line >= self.scroll + visible {
            self.scroll = line + 1 - visible;
        }
        self.scroll = self.scroll.min(self.lines.len().saturating_sub(1));
        &self.lines
    }

    pub fn key(&mut self, keysym: Keysym, text: Option<&str>, ctrl: bool) -> NoteKey {
        let vertical = matches!(keysym, Keysym::Up | Keysym::Down);
        if !vertical {
            self.goal = None;
        }
        match keysym {
            Keysym::Escape => NoteKey::Escape,
            Keysym::Return | Keysym::KP_Enter => self.insert("\n"),
            Keysym::BackSpace => {
                let from = if ctrl { self.word_back() } else { self.prev(self.cursor) };
                self.remove(from, self.cursor)
            }
            Keysym::Delete | Keysym::KP_Delete => {
                let to = if ctrl { self.word_forward() } else { self.next(self.cursor) };
                self.remove(self.cursor, to)
            }
            Keysym::Left => self.move_to(if ctrl { self.word_back() } else { self.prev(self.cursor) }),
            Keysym::Right => self.move_to(if ctrl { self.word_forward() } else { self.next(self.cursor) }),
            Keysym::Home if ctrl => self.move_to(0),
            Keysym::End if ctrl => self.move_to(self.text.len()),
            Keysym::Home => {
                let line = self.lines.get(line_of(&self.lines, self.cursor)).copied();
                self.move_to(line.map_or(0, |l| l.0))
            }
            Keysym::End => {
                let line = self.lines.get(line_of(&self.lines, self.cursor)).copied();
                self.move_to(line.map_or(self.text.len(), |l| self.line_end(l)))
            }
            Keysym::Up | Keysym::Down => self.vertical(keysym == Keysym::Down),
            _ if ctrl => NoteKey::Ignored,
            _ => match text.map(|t| t.chars().filter(|c| !c.is_control()).collect::<String>()) {
                Some(typed) if !typed.is_empty() => self.insert(&typed),
                _ => NoteKey::Ignored,
            },
        }
    }

    fn insert(&mut self, typed: &str) -> NoteKey {
        self.text.insert_str(self.cursor, typed);
        self.cursor += typed.len();
        NoteKey::Changed
    }

    fn remove(&mut self, from: usize, to: usize) -> NoteKey {
        if from >= to {
            return NoteKey::Ignored;
        }
        self.text.replace_range(from..to, "");
        self.cursor = from;
        NoteKey::Changed
    }

    fn move_to(&mut self, at: usize) -> NoteKey {
        if at == self.cursor {
            return NoteKey::Ignored;
        }
        self.cursor = at;
        NoteKey::Moved
    }

    fn prev(&self, at: usize) -> usize {
        self.text[..at].chars().next_back().map_or(at, |c| at - c.len_utf8())
    }

    fn next(&self, at: usize) -> usize {
        self.text[at..].chars().next().map_or(at, |c| at + c.len_utf8())
    }

    /// The start of the word before the caret (skipping whitespace first).
    fn word_back(&self) -> usize {
        let before = &self.text[..self.cursor];
        let trimmed = before.trim_end();
        trimmed.rfind(char::is_whitespace).map_or(0, |i| i + trimmed[i..].chars().next().map_or(1, char::len_utf8))
    }

    /// The end of the word after the caret (skipping whitespace first).
    fn word_forward(&self) -> usize {
        let after = &self.text[self.cursor..];
        let skipped = after.len() - after.trim_start().len();
        let rest = &after[skipped..];
        self.cursor + skipped + rest.find(char::is_whitespace).unwrap_or(rest.len())
    }

    /// Where End puts the caret on `line`: before a wrapped line's trailing
    /// space, so the caret does not jump to the next line.
    fn line_end(&self, line: (usize, usize)) -> usize {
        let wrapped = self.lines.iter().any(|l| l.0 == line.1) && line.1 > line.0;
        if wrapped && self.text[..line.1].ends_with(' ') {
            line.1 - 1
        } else {
            line.1
        }
    }

    fn vertical(&mut self, down: bool) -> NoteKey {
        if self.lines.is_empty() {
            return NoteKey::Ignored;
        }
        let index = line_of(&self.lines, self.cursor);
        let (start, _) = self.lines[index];
        let column = *self.goal.get_or_insert_with(|| self.text[start..self.cursor].chars().count());
        let target = match (down, index) {
            (false, 0) => return self.move_to(0),
            (false, i) => i - 1,
            (true, i) if i + 1 >= self.lines.len() => return self.move_to(self.text.len()),
            (true, i) => i + 1,
        };
        let line = self.lines[target];
        let end = self.line_end(line);
        let at = self.text[line.0..end].char_indices().nth(column).map_or(end, |(i, _)| line.0 + i);
        self.move_to(at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ten pixels a character.
    fn mono(s: &str) -> f32 {
        s.chars().count() as f32 * 10.0
    }

    fn shown(text: &str, width: f32) -> Vec<&str> {
        layout(text, width, mono).into_iter().map(|(a, b)| &text[a..b]).collect()
    }

    fn typed(text: &str) -> Note {
        let mut note = Note::default();
        for ch in text.chars() {
            if ch == '\n' {
                note.key(Keysym::Return, None, false);
            } else {
                note.key(Keysym::a, Some(&ch.to_string()), false);
            }
        }
        note
    }

    #[test]
    fn lines_wrap_at_spaces_and_break_long_words() {
        assert_eq!(shown("one two three", 80.0), ["one two ", "three"]);
        assert_eq!(shown("abcdefghij", 40.0), ["abcd", "efgh", "ij"]);
        assert_eq!(shown("a\n\nb", 80.0), ["a", "", "b"]);
        assert_eq!(shown("", 80.0), [""]);
        assert_eq!(shown("fits   ", 40.0), ["fits   "], "trailing spaces never wrap");
    }

    #[test]
    fn typing_return_and_backspace() {
        let mut note = typed("ab\ncd");
        assert_eq!(note.text(), "ab\ncd");
        assert_eq!(note.key(Keysym::BackSpace, None, false), NoteKey::Changed);
        note.key(Keysym::Left, None, false);
        note.key(Keysym::BackSpace, None, false);
        assert_eq!(note.text(), "abc", "removed the line break");
        assert_eq!(note.key(Keysym::Escape, None, false), NoteKey::Escape);
        assert_eq!(note.key(Keysym::a, Some("a"), true), NoteKey::Ignored, "Ctrl+key types nothing");
    }

    #[test]
    fn words_go_with_ctrl() {
        let mut note = typed("hello big world");
        note.key(Keysym::BackSpace, None, true);
        assert_eq!(note.text(), "hello big ");
        note.key(Keysym::Left, None, true);
        assert_eq!(note.cursor(), 6);
        note.key(Keysym::Delete, None, true);
        assert_eq!(note.text(), "hello  ");
    }

    #[test]
    fn multibyte_characters_are_whole() {
        let mut note = typed("é☃");
        note.key(Keysym::Left, None, false);
        note.key(Keysym::BackSpace, None, false);
        assert_eq!(note.text(), "☃");
    }

    #[test]
    fn up_and_down_keep_their_column_on_wrapped_lines() {
        let mut note = typed("abcdef\nxy\nlmnopq");
        note.relayout(1000.0, 10, mono);
        note.key(Keysym::Home, None, true);
        for _ in 0..4 {
            note.key(Keysym::Right, None, false);
        }
        note.key(Keysym::Down, None, false);
        assert_eq!(note.cursor(), 9, "end of the short line");
        note.key(Keysym::Down, None, false);
        assert_eq!(&note.text()[note.cursor()..], "pq", "back to column 4");
        note.key(Keysym::Down, None, false);
        assert_eq!(note.cursor(), note.text().len(), "past the last line: the end");
        note.key(Keysym::Home, None, true);
        note.key(Keysym::Up, None, false);
        assert_eq!(note.cursor(), 0);
    }

    #[test]
    fn home_and_end_follow_the_wrapped_line() {
        let mut note = typed("one two three");
        note.relayout(80.0, 10, mono);
        note.key(Keysym::Home, None, true);
        note.key(Keysym::End, None, false);
        assert_eq!(note.cursor(), 7, "before the space the line wrapped at");
        note.key(Keysym::Down, None, false);
        note.key(Keysym::Home, None, false);
        assert_eq!(note.cursor(), 8);
    }

    #[test]
    fn scrolling_keeps_the_caret_in_view() {
        let mut note = typed("1\n2\n3\n4\n5");
        note.relayout(100.0, 2, mono);
        assert_eq!(note.scroll, 3);
        note.key(Keysym::Home, None, true);
        note.relayout(100.0, 2, mono);
        assert_eq!(note.scroll, 0);
    }
}
