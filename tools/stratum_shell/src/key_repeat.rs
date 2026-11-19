//! Which held keys repeat.
//!
//! Only text fields get repeats: a held arrow racing across the power menu
//! or a held Return submitting a password twice is worse than no repeat at
//! all. The timing (delay and rate) is the compositor's, applied by SCTK.

use smithay_client_toolkit::seat::keyboard::Keysym;

/// What a key would go to right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// Nothing that types: a menu, an overlay, nothing focused.
    None,
    /// A text field, where arrows and Home/End move the cursor.
    Field,
    /// A search box whose arrows move something else (the theme switcher).
    Search,
}

/// Whether a repeat of `keysym`, which types `text`, reaches `target`.
pub fn repeats(target: Target, keysym: Keysym, text: Option<&str>) -> bool {
    // Never: these submit, close or move focus.
    if matches!(
        keysym,
        Keysym::Return | Keysym::KP_Enter | Keysym::Escape | Keysym::Tab | Keysym::ISO_Left_Tab
    ) {
        return false;
    }
    let edits = matches!(keysym, Keysym::BackSpace | Keysym::Delete)
        || text.is_some_and(|t| t.chars().any(|c| !c.is_control()));
    match target {
        Target::None => false,
        Target::Field => true,
        Target::Search => edits,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_repeat_typing_and_cursor_keys_but_never_submit() {
        assert!(repeats(Target::Field, Keysym::a, Some("a")));
        assert!(repeats(Target::Field, Keysym::BackSpace, None));
        assert!(repeats(Target::Field, Keysym::Left, None));
        assert!(!repeats(Target::Field, Keysym::Return, Some("\r")));
        assert!(!repeats(Target::Field, Keysym::KP_Enter, None));
        assert!(!repeats(Target::Field, Keysym::Escape, None));
        assert!(!repeats(Target::Field, Keysym::Tab, Some("\t")));
    }

    #[test]
    fn a_search_repeats_typing_only() {
        assert!(repeats(Target::Search, Keysym::x, Some("x")));
        assert!(repeats(Target::Search, Keysym::BackSpace, None));
        assert!(!repeats(Target::Search, Keysym::Right, None), "arrows move the band");
    }

    #[test]
    fn nothing_repeats_without_a_field() {
        assert!(!repeats(Target::None, Keysym::Down, None));
        assert!(!repeats(Target::None, Keysym::a, Some("a")));
    }
}
