//! A masked password field, shared by the lock screen and the polkit dialog.
//!
//! Holds the typed text in a [`Secret`], interprets editing keys, and draws one
//! dot per character followed by a caret. What Return and Escape mean is up to
//! the owner, so they are reported rather than acted on.

use std::sync::Arc;

use crate::pam::Secret;
use crate::render::display_list::{DisplayList, Rect};
use crate::text::TextStyle;
use crate::ui::{self, PaintCtx};

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

/// Sizes the field draws with, taken from the owner's metrics.
#[derive(Clone, Copy, Debug)]
pub struct FieldStyle {
    /// Size of the dots.
    pub text: f32,
    /// Horizontal padding inside the field.
    pub padding: f32,
    pub caret_width: f32,
}

#[derive(Default)]
pub struct PasswordField {
    input: Secret,
}

impl PasswordField {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.input.clear();
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.input.is_empty()
    }

    #[cfg(test)]
    pub fn chars(&self) -> usize {
        self.input.chars()
    }

    /// Move the text out for a submit from somewhere other than Return (a
    /// button). `None` when empty, for the same reason Return ignores it.
    pub fn take(&mut self) -> Option<Secret> {
        (!self.input.is_empty()).then(|| self.input.take())
    }

    /// Apply a keypress. `text` is what the key types under the current layout
    /// and modifiers. An empty field never submits: an empty attempt is a
    /// guaranteed failure that costs PAM's failure delay.
    pub fn key(
        &mut self,
        keysym: smithay_client_toolkit::seat::keyboard::Keysym,
        text: Option<&str>,
        ctrl: bool,
    ) -> FieldKey {
        use smithay_client_toolkit::seat::keyboard::Keysym;
        match keysym {
            Keysym::Return | Keysym::KP_Enter => {
                if self.input.is_empty() {
                    FieldKey::Ignored
                } else {
                    FieldKey::Submit(self.input.take())
                }
            }
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

    /// Draw the dots, trimmed from the left when they no longer fit, then a
    /// caret unless `busy`. The frame around the field is the owner's.
    pub fn paint(
        &self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        field: Rect,
        style: FieldStyle,
        busy: bool,
    ) {
        let text_style = TextStyle {
            family: Arc::from(ctx.theme.font.as_str()),
            size: style.text,
            weight: ctx.metrics.typography.weight_regular,
            color: ctx.theme.text_main,
        };
        let dot = "\u{25cf}";
        let dot_w = ctx.text.measure(dot, &text_style).0.max(1.0);
        let usable = field.width - style.padding * 2.0 - style.caret_width * 2.0;
        let fits = (usable / dot_w).floor().max(0.0) as usize;
        let shown = self.input.chars().min(fits);
        let x = field.x + style.padding;
        if shown > 0 {
            ui::paint_text_vcentered(
                ctx,
                list,
                &dot.repeat(shown),
                x,
                field,
                style.text,
                ctx.metrics.typography.weight_regular,
                ctx.theme.text_main,
            );
        }
        if !busy {
            let caret_h = style.text * 1.1;
            list.push_rect(Rect {
                x: x + dot_w * shown as f32 + style.caret_width,
                y: field.y + (field.height - caret_h) * 0.5,
                width: style.caret_width,
                height: caret_h,
                radius: 0.0,
                color: ctx.theme.text_main,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay_client_toolkit::seat::keyboard::Keysym;

    fn typed(text: &str) -> PasswordField {
        let mut field = PasswordField::new();
        for ch in text.chars() {
            field.key(Keysym::a, Some(&ch.to_string()), false);
        }
        field
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
        let mut field = PasswordField::new();
        assert_eq!(field.key(Keysym::Return, None, false), FieldKey::Ignored);
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
    fn spaces_are_part_of_the_password() {
        let field = typed(" a ");
        assert_eq!(field.chars(), 3);
    }
}
