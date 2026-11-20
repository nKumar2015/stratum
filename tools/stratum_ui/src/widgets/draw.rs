//! Drawing helpers every widget uses: text placed by its real metrics,
//! icons centred by their ink, frames, wrapping and eliding, and the
//! pointer state a paint reads.

use crate::render::display_list::{Color, DisplayList, Rect, Transform};
use crate::text::TextStyle;
use crate::widgets::Ui;

/// What a widget needs to know each frame.
#[derive(Clone, Debug, Default)]
pub struct InputState {
    /// Pointer position in the surface, logical pixels.
    pub pointer: Option<(f32, f32)>,
    /// Whether the primary button is currently held.
    pub pressed: bool,
}

/// Dim everything under an overlay: black at `alpha` over `surface`.
pub fn paint_scrim(list: &mut DisplayList, surface: Rect, alpha: f32) {
    list.push_rect(Rect { color: Color::rgba(0.0, 0.0, 0.0, alpha), ..surface });
}

/// Fill `rect` and outline it `border_width` thick inside its edge (no
/// outline at zero).
pub fn paint_frame(list: &mut DisplayList, rect: Rect, border_width: f32, border: Color, fill: Color) {
    list.push_rect(Rect { color: fill, ..rect });
    if border_width > 0.0 {
        stroke_rounded(list, rect, border, border_width);
    }
}

/// Stroke the outline of a rounded rectangle, `width` thick inside its edge,
/// leaving its inside untouched (a translucent fill shows through, unlike two
/// stacked rects). It is drawn from the rect's own distance field, so it
/// follows a fill of the same rect exactly, with true arcs at the corners.
pub fn stroke_rounded(list: &mut DisplayList, rect: Rect, color: Color, width: f32) {
    list.push_outline(Rect { color, ..rect }, width);
}

/// Center a line of regular-weight text inside `rect`.
pub fn paint_text_centered(
    ctx: &mut impl Ui,
    list: &mut DisplayList,
    value: &str,
    rect: Rect,
    size: f32,
    color: Color,
) {
    let weight = ctx.metrics().typography.weight_regular;
    paint_text_centered_weighted(ctx, list, value, rect, size, weight, color);
}

/// A standalone icon glyph centred in `rect` by its ink, in
/// `[appearance] icon_font`. (A proportional Nerd Font's icons overflow their
/// advance, so centring by advance put them to the right of their pill.)
pub fn paint_icon_centered(
    ctx: &mut impl Ui,
    list: &mut DisplayList,
    glyph: &str,
    rect: Rect,
    size: f32,
    color: Color,
) {
    let style = TextStyle {
        family: std::sync::Arc::from(ctx.appearance().icon_font.as_str()),
        size,
        weight: 400,
        color,
    };
    let (cx, cy) = (rect.x + rect.width * 0.5, rect.y + rect.height * 0.5);
    let (x, baseline) = match ctx.text().ink_bounds(glyph, &style) {
        Some((left, top, right, bottom)) => (cx - (left + right) * 0.5, cy - (top + bottom) * 0.5),
        None => {
            let width = ctx.text().measure(glyph, &style).0;
            let (ascent, line_height) = ctx.text().metrics(&style);
            (cx - width * 0.5, rect.y + (rect.height - line_height) * 0.5 + ascent)
        }
    };
    ctx.text_text(list, glyph, x, baseline, style);
}

/// [`paint_text_centered`], turned clockwise by `radians` about the centre of
/// `rect` (QML's `rotation` on a centred item).
pub fn paint_text_centered_rotated(
    ctx: &mut impl Ui,
    list: &mut DisplayList,
    value: &str,
    rect: Rect,
    size: f32,
    color: Color,
    radians: f32,
) {
    let style = TextStyle {
        family: std::sync::Arc::from(ctx.theme().font.as_str()),
        size,
        weight: 400,
        color,
    };
    let width = ctx.text().measure(value, &style).0;
    let (ascent, line_height) = ctx.text().metrics(&style);
    let x = rect.x + (rect.width - width) * 0.5;
    let baseline = rect.y + (rect.height - line_height) * 0.5 + ascent;
    let mut run = ctx.text().shape(value, &style, x, baseline);
    run.transform = Transform::rotate_about(radians, rect.x + rect.width * 0.5, rect.y + rect.height * 0.5);
    list.push_text(run);
}

/// Draw a single line vertically centred in `rect`, starting at `x`.
///
/// Uses the font's real ascent and line height, so text sits on the optical
/// centre regardless of size. Horizontal placement is the caller's choice.
#[allow(clippy::too_many_arguments)]
pub fn paint_text_vcentered(
    ctx: &mut impl Ui,
    list: &mut DisplayList,
    value: &str,
    x: f32,
    rect: Rect,
    size: f32,
    weight: u16,
    color: Color,
) {
    if value.is_empty() || rect.height <= 0.0 {
        return;
    }
    let style = TextStyle {
        family: std::sync::Arc::from(ctx.theme().font.as_str()),
        size,
        weight,
        color,
    };
    let (ascent, line_height) = ctx.text().metrics(&style);
    let box_top = rect.y + (rect.height - line_height) * 0.5;
    let baseline = box_top + ascent;
    ctx.text_text(list, value, x, baseline, style);
}

/// Like [`paint_text_centered`] but with an explicit weight.
pub fn paint_text_centered_weighted(
    ctx: &mut impl Ui,
    list: &mut DisplayList,
    value: &str,
    rect: Rect,
    size: f32,
    weight: u16,
    color: Color,
) {
    let style = TextStyle {
        family: std::sync::Arc::from(ctx.theme().font.as_str()),
        size,
        weight,
        color,
    };
    let width = ctx.text().measure(value, &style).0;
    let x = rect.x + (rect.width - width) * 0.5;
    paint_text_vcentered(ctx, list, value, x, rect, size, weight, color);
}

/// Rotate a text run so it reads upward along a vertical rail.
///
/// The rotation maps the run's advance onto `-y` and its cross-axis extent onto
/// `+x`. `baseline_x` is therefore where the glyph baselines land; to centre a
/// run of `size` in a rail, pass roughly `rail_centre + size * 0.5`.
pub fn vertical_text_transform(baseline_x: f32, bottom_y: f32) -> Transform {
    Transform::rotate(-std::f32::consts::FRAC_PI_2)
        .then(Transform::translate(baseline_x, bottom_y))
}

/// Break `text` into lines no wider than `max_width`, at spaces, the way
/// QML's `wrapMode: Text.WordWrap` does. A single word wider than the line is
/// kept whole. At most `max_lines` lines are returned; when text is cut off the
/// last line ends in an ellipsis.
pub fn wrap_text(ctx: &mut impl Ui, text: &str, size: f32, weight: u16, max_width: f32, max_lines: usize) -> Vec<String> {
    let style = TextStyle {
        family: std::sync::Arc::from(ctx.theme().font.as_str()),
        size,
        weight,
        color: ctx.theme().text_main,
    };
    wrap_with(text, max_width, max_lines, |s| ctx.text().measure(s, &style).0)
}

fn wrap_with(text: &str, max_width: f32, max_lines: usize, mut measure: impl FnMut(&str) -> f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut words = text.split_whitespace().peekable();
    while let Some(word) = words.next() {
        let candidate = if current.is_empty() {
            word.to_string()
        } else {
            format!("{current} {word}")
        };
        if current.is_empty() || measure(&candidate) <= max_width {
            current = candidate;
            continue;
        }
        lines.push(std::mem::replace(&mut current, word.to_string()));
        if lines.len() == max_lines {
            // Out of lines: what is left goes into the last one, elided.
            let mut last = lines.pop().unwrap_or_default();
            last.push(' ');
            last.push_str(&current);
            for rest in words.by_ref() {
                last.push(' ');
                last.push_str(rest);
            }
            lines.push(elide_with(&last, max_width, &mut measure));
            return lines;
        }
    }
    if !current.is_empty() && max_lines > 0 {
        lines.push(current);
    }
    lines
}

/// `text` cut to fit `max_width` on one line, ending in an ellipsis when cut.
/// Unlike [`wrap_text`], a single long word is cut too.
pub fn elide(ctx: &mut impl Ui, text: &str, size: f32, weight: u16, max_width: f32) -> String {
    let style = TextStyle {
        family: std::sync::Arc::from(ctx.theme().font.as_str()),
        size,
        weight,
        color: ctx.theme().text_main,
    };
    let mut measure = |s: &str| ctx.text().measure(s, &style).0;
    if measure(text) <= max_width {
        return text.to_string();
    }
    elide_with(text, max_width, &mut measure)
}

fn elide_with(text: &str, max_width: f32, measure: &mut impl FnMut(&str) -> f32) -> String {
    let mut chars: Vec<char> = text.chars().collect();
    loop {
        let candidate: String = chars.iter().collect::<String>() + "\u{2026}";
        if chars.is_empty() || measure(&candidate) <= max_width {
            return candidate;
        }
        chars.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ten pixels per character, so widths are easy to reason about.
    fn mono(s: &str) -> f32 {
        s.chars().count() as f32 * 10.0
    }

    #[test]
    fn wrap_breaks_at_spaces() {
        let lines = wrap_with("the quick brown fox", 100.0, 5, mono);
        assert_eq!(lines, vec!["the quick", "brown fox"]);
    }

    #[test]
    fn wrap_keeps_a_long_word_whole() {
        let lines = wrap_with("a /run/current-system/sw/bin/true b", 100.0, 5, mono);
        assert_eq!(lines, vec!["a", "/run/current-system/sw/bin/true", "b"]);
    }

    #[test]
    fn wrap_elides_what_does_not_fit() {
        let lines = wrap_with("one two three four five six", 90.0, 2, mono);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "one two");
        assert!(lines[1].ends_with('\u{2026}'));
        assert!(mono(&lines[1]) <= 90.0);
    }

    #[test]
    fn wrap_of_nothing_is_nothing() {
        assert!(wrap_with("   ", 100.0, 3, mono).is_empty());
    }

    #[test]
    fn vertical_transform_advances_upward() {
        let t = vertical_text_transform(20.0, 900.0);
        let (start_x, start_y) = t.apply(0.0, 0.0);
        let (end_x, end_y) = t.apply(10.0, 0.0);

        // The baseline sits at the requested x ...
        assert!((start_x - 20.0).abs() < 1e-3, "start x was {start_x}");
        assert!((start_y - 900.0).abs() < 1e-3, "start y was {start_y}");
        // ... both points share that x, and advancing the run moves upward.
        assert!((end_x - 20.0).abs() < 1e-3, "end x was {end_x}");
        assert!((start_y - end_y - 10.0).abs() < 1e-3, "expected 10px of advance");
    }
}
