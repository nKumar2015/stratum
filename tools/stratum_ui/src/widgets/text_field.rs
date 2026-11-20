//! A single-line text field: the lock screen's and polkit dialog's password
//! fields (masked) and the notification reply field (plain).
//!
//! Holds the typed text in a [`Secret`], which costs nothing for plain text and
//! means a password is wiped however the field is used. Interprets editing
//! keys and draws the text (or one dot per character) followed by a caret.
//! What Return and Escape mean is up to the owner, so they are reported rather
//! than acted on.

use std::sync::Arc;

use crate::secret::Secret;
use crate::render::display_list::{DisplayList, Rect};
use crate::text::TextStyle;
use crate::widgets::draw as ui;
use crate::widgets::Ui;

/// What a keypress did to the field.
#[derive(Debug)]
pub enum FieldKey {
    /// Not a key the field handles.
    Ignored,
    /// The text changed.
    Changed,
    /// Return: the typed text, moved out of the field.
    Submit(Secret),
    /// Escape. The field is left as it was; the owner decides.
    Escape,
}

/// Compared by variant only: passwords are never compared outside PAM.
impl PartialEq for FieldKey {
    fn eq(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

/// How the field draws, taken from the owner's metrics.
#[derive(Clone, Copy, Debug)]
pub struct FieldStyle<'a> {
    /// Text (or dot) size.
    pub text: f32,
    /// Horizontal padding inside the field.
    pub padding: f32,
    pub caret_width: f32,
    /// Draw one dot per character instead of the text.
    pub masked: bool,
    /// Shown, muted, while the field is empty.
    pub placeholder: Option<&'a str>,
}

#[derive(Default)]
pub struct TextField {
    input: Secret,
}

impl TextField {
    pub fn new() -> Self {
        Self::default()
    }

    /// A field already holding `text`, for editing a shown value.
    pub fn with_text(text: &str) -> Self {
        Self::with_text_capacity(text, crate::secret::DEFAULT_CAPACITY)
    }

    /// An empty field that holds up to `bytes` bytes rather than the
    /// default 256, for text longer than a name (a typed path, a search).
    pub fn with_capacity(bytes: usize) -> Self {
        Self { input: Secret::with_capacity(bytes) }
    }

    /// [`TextField::with_text`] with room for `bytes` bytes.
    pub fn with_text_capacity(text: &str, bytes: usize) -> Self {
        let mut field = Self::with_capacity(bytes);
        text.chars().for_each(|ch| field.input.push(ch));
        field
    }

    pub fn clear(&mut self) {
        self.input.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.input.is_empty()
    }

    pub fn chars(&self) -> usize {
        self.input.chars()
    }

    /// The current text, for fields whose contents are not secret (search).
    pub fn text(&self) -> &str {
        self.input.expose()
    }

    /// Move the text out for a submit from somewhere other than Return (a
    /// button). `None` when empty, for the same reason Return ignores it.
    pub fn take(&mut self) -> Option<Secret> {
        (!self.input.is_empty()).then(|| self.input.take())
    }

    /// Apply a keypress. `text` is what the key types under the current layout
    /// and modifiers. An empty field never submits: for a password an empty
    /// attempt is a guaranteed failure that costs PAM's failure delay, and an
    /// empty reply is not a reply.
    pub fn key(
        &mut self,
        keysym: smithay_client_toolkit::seat::keyboard::Keysym,
        text: Option<&str>,
        ctrl: bool,
    ) -> FieldKey {
        use smithay_client_toolkit::seat::keyboard::Keysym;
        match keysym {
            Keysym::Return | Keysym::KP_Enter => match self.take() {
                Some(text) => FieldKey::Submit(text),
                None => FieldKey::Ignored,
            },
            Keysym::Escape => FieldKey::Escape,
            Keysym::BackSpace => {
                if ctrl {
                    self.input.clear();
                } else {
                    self.input.pop();
                }
                FieldKey::Changed
            }
            _ if ctrl && matches!(keysym, Keysym::u | Keysym::U) => {
                self.input.clear();
                FieldKey::Changed
            }
            _ => {
                let Some(text) = text else {
                    return FieldKey::Ignored;
                };
                let mut typed = false;
                for ch in text.chars().filter(|ch| !ch.is_control()) {
                    self.input.push(ch);
                    typed = true;
                }
                if typed {
                    FieldKey::Changed
                } else {
                    FieldKey::Ignored
                }
            }
        }
    }

    /// Draw the contents, then a caret unless `busy`. When the text is wider
    /// than the field its end is shown, since that is where typing happens.
    /// The frame around the field is the owner's.
    pub fn paint(
        &self,
        ctx: &mut impl Ui,
        list: &mut DisplayList,
        field: Rect,
        style: FieldStyle<'_>,
        busy: bool,
    ) {
        let weight = ctx.metrics().typography.weight_regular;
        let text_style = TextStyle {
            family: Arc::from(ctx.theme().font.as_str()),
            size: style.text,
            weight,
            color: ctx.theme().text_main,
        };
        let usable = field.width - style.padding * 2.0 - style.caret_width * 2.0;
        let x = field.x + style.padding;

        let shown = if style.masked {
            let dot = "\u{25cf}";
            let dot_w = ctx.text().measure(dot, &text_style).0.max(1.0);
            let fits = (usable / dot_w).floor().max(0.0) as usize;
            dot.repeat(self.input.chars().min(fits))
        } else {
            tail_that_fits(self.input.expose(), usable, |s| ctx.text().measure(s, &text_style).0)
        };
        let width = if shown.is_empty() {
            0.0
        } else {
            ctx.text().measure(&shown, &text_style).0
        };

        if !shown.is_empty() {
            ui::paint_text_vcentered(ctx, list, &shown, x, field, style.text, weight, ctx.theme().text_main);
        } else if let Some(placeholder) = style.placeholder {
            ui::paint_text_vcentered(ctx, list, placeholder, x + style.caret_width * 2.0, field, style.text, weight, ctx.theme().text_muted);
        }
        if !busy {
            let caret_h = style.text * 1.1;
            list.push_rect(Rect {
                x: x + width + style.caret_width,
                y: field.y + (field.height - caret_h) * 0.5,
                width: style.caret_width,
                height: caret_h,
                radius: 0.0,
                color: ctx.theme().text_main,
            });
        }
    }
}

/// The longest end of `text` that fits `max_width`, with a leading ellipsis
/// when anything was cut.
fn tail_that_fits(text: &str, max_width: f32, mut measure: impl FnMut(&str) -> f32) -> String {
    if text.is_empty() || measure(text) <= max_width {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let candidate = |start: usize| -> String { std::iter::once('\u{2026}').chain(chars[start..].iter().copied()).collect() };
    // The first cut that fits, found by bisection: a shorter tail is never
    // wider, and a path field holds thousands of characters, which one
    // measurement per character would shape (and push out of the shaping
    // cache) on every paint.
    let (mut lo, mut hi) = (1, chars.len());
    while lo < hi {
        let mid = (lo + hi) / 2;
        if measure(&candidate(mid)) <= max_width {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    if lo < chars.len() { candidate(lo) } else { String::new() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay_client_toolkit::seat::keyboard::Keysym;

    fn typed(text: &str) -> TextField {
        let mut field = TextField::new();
        for ch in text.chars() {
            field.key(Keysym::a, Some(&ch.to_string()), false);
        }
        field
    }

    #[test]
    fn the_tail_that_fits_is_found_in_few_measurements() {
        let text: String = (0..4000).map(|i| char::from(b'a' + (i % 26) as u8)).collect();
        let mut calls = 0;
        let shown = tail_that_fits(&text, 100.0, |s| {
            calls += 1;
            s.chars().count() as f32
        });
        assert_eq!(shown.chars().count(), 100);
        assert!(shown.starts_with('\u{2026}') && text.ends_with(&shown[3..]));
        assert!(calls < 20, "{calls} measurements");
        assert_eq!(tail_that_fits("abc", 10.0, |s| s.chars().count() as f32), "abc");
        assert_eq!(tail_that_fits("abc", 0.5, |s| s.chars().count() as f32), "");
    }

    #[test]
    fn a_long_field_keeps_a_long_path() {
        let path: String = "/a".repeat(500);
        let mut field = TextField::with_text_capacity(&path, 4096);
        assert_eq!(field.text(), path);
        field.key(Keysym::a, Some("b"), false);
        assert_eq!(field.chars(), 1001);
        let FieldKey::Submit(secret) = field.key(Keysym::Return, None, false) else {
            panic!("expected a submit");
        };
        assert_eq!(secret.chars(), 1001);
        assert_eq!(TextField::with_text(&path).chars(), 256, "a default field still stops at 256 bytes");
    }

    #[test]
    fn return_moves_the_text_out() {
        let mut field = typed("pw");
        let FieldKey::Submit(secret) = field.key(Keysym::Return, None, false) else {
            panic!("expected a submit");
        };
        assert_eq!(secret.chars(), 2);
        assert!(field.is_empty());
    }

    #[test]
    fn empty_return_is_ignored() {
        let mut field = TextField::new();
        assert_eq!(field.key(Keysym::Return, None, false), FieldKey::Ignored);
        assert!(field.take().is_none());
    }

    #[test]
    fn editing_keys() {
        let mut field = typed("abc");
        field.key(Keysym::BackSpace, None, false);
        assert_eq!(field.chars(), 2);
        field.key(Keysym::BackSpace, None, true);
        assert!(field.is_empty());
        let mut field = typed("abc");
        field.key(Keysym::u, Some("\u{15}"), true);
        assert!(field.is_empty());
        assert_eq!(field.key(Keysym::Tab, Some("\t"), false), FieldKey::Ignored);
    }

    #[test]
    fn escape_is_reported_and_leaves_the_text() {
        let mut field = typed("abc");
        assert_eq!(field.key(Keysym::Escape, None, false), FieldKey::Escape);
        assert_eq!(field.chars(), 3);
    }

    #[test]
    fn spaces_are_part_of_the_text() {
        let field = typed(" a ");
        assert_eq!(field.chars(), 3);
    }

    #[test]
    fn long_text_shows_its_end() {
        let mono = |s: &str| s.chars().count() as f32 * 10.0;
        assert_eq!(tail_that_fits("short", 100.0, mono), "short");
        let tail = tail_that_fits("a long reply that overflows", 100.0, mono);
        assert!(tail.starts_with('\u{2026}') && tail.ends_with("overflows"), "{tail}");
        assert!(mono(&tail) <= 100.0);
    }
}
