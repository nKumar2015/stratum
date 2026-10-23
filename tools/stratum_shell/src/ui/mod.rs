//! Widget layer: layout, painting, and input.
//!
//! Widgets are simple retained objects. Each one lays itself out inside the
//! rectangle it is given, emits draw commands, and answers hit-tests. There is
//! no binding engine and no per-frame allocation in the common case: the shell
//! rebuilds a `DisplayList` from state only when something changes.

use crate::render::display_list::{
    Color, DisplayList, Paint, Path, PathCommand, Point, Rect, Transform,
};
pub mod animation;
pub mod control_center;
pub mod controls;
pub mod graph;
pub mod hover_menu;
pub mod notification_card;
pub mod pages;
pub mod sidebar;
pub mod text_field;
pub mod style;
pub mod widgets;

use crate::text::{TextEngine, TextStyle};

/// Shared state every widget can read while painting.
pub struct PaintCtx<'a> {
    pub theme: &'a crate::config::Theme,
    pub appearance: &'a crate::config::AppearanceConfig,
    /// Sizes for every widget. Kept beside the theme so styling a widget means
    /// consulting one palette and one metrics table, not constants in its code.
    pub metrics: &'a crate::ui::style::Metrics,
    pub text: &'a mut TextEngine,
    /// Decoded icons, for widgets that display themed artwork.
    pub icons: &'a mut crate::icons::IconTheme,
    /// Live system state, read by widgets that display real data.
    pub state: &'a crate::sys_state::SystemState,
    /// Theme definitions, for the theme picker.
    pub themes: &'a std::collections::BTreeMap<String, crate::config::ThemeConfig>,
    /// Which output's bar is being painted (for per-monitor workspace focus).
    pub output_name: &'a str,
    /// Orientation of the bar being painted.
    pub axis: crate::config::Axis,
    /// Seconds since the previous paint, for widgets that animate themselves.
    pub dt: f32,
}

impl PaintCtx<'_> {
    pub fn is_vertical(&self) -> bool {
        self.axis == crate::config::Axis::Vertical
    }
}

impl PaintCtx<'_> {
    pub fn text_text(
        &mut self,
        list: &mut DisplayList,
        value: &str,
        x: f32,
        y: f32,
        style: TextStyle,
    ) {
        if value.is_empty() {
            return;
        }
        let run = self.text.shape(value, &style, x, y);
        list.push_text(run);
    }

    /// Measure a string in the theme font at `size`.
    pub fn measure(&mut self, value: &str, size: f32) -> f32 {
        let style = TextStyle {
            family: std::sync::Arc::from(self.theme.font.as_str()),
            size,
            weight: 400,
            color: Color::WHITE,
        };
        self.text.measure(value, &style).0
    }
}

/// What a widget needs to know each frame.
#[derive(Clone, Debug, Default)]
pub struct InputState {
    /// Pointer position in the surface, logical pixels.
    pub pointer: Option<(f32, f32)>,
    /// Whether the primary button is currently held.
    pub pressed: bool,
}

/// Something a widget asks the shell to do when clicked.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Open the control centre on a specific page.
    OpenPanel(crate::ui::control_center::Page),
    /// Activate a tray item by id.
    ActivateTray(String),
    /// Open a tray item's context menu. `service` and `menu_path` come from the
    /// item the widget drew; `y` is the icon's top edge in bar coordinates, so
    /// the menu can open beside it.
    OpenTrayMenu {
        service: String,
        path: String,
        y: f32,
    },
    /// Activate a tray item with the secondary (middle) button.
    SecondaryActivateTray(String),
    /// Focus a workspace. The shell applies it to the output whose surface was
    /// clicked and updates its local state immediately, so the bar reflects the
    /// change before the compositor's event arrives.
    FocusWorkspace(i32),
    /// Open the dashboard on the named output (the clock's tap, as in QML).
    OpenDashboard(String),
}

/// Which mouse button produced a click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// Everything a widget needs to interpret a click.
///
/// The output name matters because the shell owns one widget tree but paints it
/// on every monitor: a click on the left monitor's workspaces must move that
/// monitor, not whichever output happens to be focused.
pub struct ClickCtx<'a> {
    pub point: Point,
    pub output_name: &'a str,
    /// Primary unless the shell reports otherwise.
    pub button: MouseButton,
}

/// A bar widget.
pub trait Widget {
    /// Read what the size depends on, before `height`/`width` are asked:
    /// those see only metrics, and a widget sized by its data (the tray, by
    /// its items) would otherwise lay out with last frame's.
    fn sync(&mut self, _ctx: &PaintCtx<'_>) {}

    /// Intrinsic height in a vertical bar at the given width. `None` means
    /// "fill available".
    fn height(&self, width: f32, metrics: &crate::ui::style::Metrics) -> Option<f32> {
        let _ = (width, metrics);
        None
    }

    /// Intrinsic width in a horizontal bar at the given height. `None` means
    /// "fill available".
    fn width(&self, height: f32, metrics: &crate::ui::style::Metrics) -> Option<f32> {
        let _ = (height, metrics);
        None
    }

    /// Draw into `rect` (surface coordinates, logical pixels).
    fn paint(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, rect: Rect, input: &InputState);

    /// Handle a click at `point`, optionally returning an action for the shell.
    fn on_click(&mut self, _ctx: &ClickCtx<'_>) -> Option<Action> {
        None
    }

    /// Handle wheel input at `point` on `output`. `delta` is in wheel units:
    /// 120 per notch, positive scrolling down.
    fn on_scroll(&mut self, _output: &str, _point: Point, _delta: f32) -> Option<Action> {
        None
    }

    /// The hover menu this widget opens when the pointer rests on it at
    /// `point` on `output`, with the widget's rect (which the menu anchors
    /// beside).
    fn hover_target(&self, _output: &str, _point: Point) -> Option<(hover_menu::HoverKind, Rect)> {
        None
    }

    /// Advance animations by `dt` seconds. Returns true while still moving, so
    /// the shell knows to request another frame.
    fn tick(&mut self, _dt: f32) -> bool {
        false
    }

    /// Whether an animation is currently in flight, without advancing it.
    ///
    /// The event loop needs this to choose a frame cadence before it knows how
    /// much time has passed.
    fn animating(&self) -> bool {
        false
    }
}

/// Advance every widget's animation. Returns true while anything is moving.
pub fn tick_all(widgets: &mut [Box<dyn Widget>], dt: f32) -> bool {
    let mut animating = false;
    for widget in widgets {
        animating |= widget.tick(dt);
    }
    animating
}

// ---------------------------------------------------------------------------
// Reusable drawing helpers
// ---------------------------------------------------------------------------

/// Draw the inverted corner scoop that joins a vertical rail to a screen edge.
///
/// The shape occupies the `size` square at `(x, y)` and fills the region
/// between the rail's edge and the screen edge, curving back to the rail so
/// the join looks continuous; the path is QML's `InvertedCorner`. `flip`
/// mirrors it vertically for the bottom corner, `mirror` horizontally for a
/// rail on the right, whose edge is then the square's right side.
pub fn paint_inverted_corner(
    list: &mut DisplayList,
    x: f32,
    y: f32,
    size: f32,
    flip: bool,
    mirror: bool,
    color: Color,
) {
    // In the corner's own frame: `0` along x is the rail's edge, `0` or
    // `size` along y the screen edge.
    let (edge, outward) = if mirror { (x + size, -1.0) } else { (x, 1.0) };
    let at = |along: f32, down: f32| Point::new(edge + outward * along, y + down);
    let (rail_end, screen) = if flip { (0.0, size) } else { (size, 0.0) };

    list.push_path(Path::new(
        vec![
            PathCommand::MoveTo(at(0.0, rail_end)),
            PathCommand::LineTo(at(0.0, screen)),
            PathCommand::LineTo(at(size, screen)),
            // Back to the start, pulled toward the corner to make the scoop.
            PathCommand::QuadTo(at(0.0, screen), at(0.0, rail_end)),
            PathCommand::Close,
        ],
        Paint::Solid(color),
    ));
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
    ctx: &mut PaintCtx<'_>,
    list: &mut DisplayList,
    value: &str,
    rect: Rect,
    size: f32,
    color: Color,
) {
    let weight = ctx.metrics.typography.weight_regular;
    paint_text_centered_weighted(ctx, list, value, rect, size, weight, color);
}

/// A standalone icon glyph centred in `rect` by its ink, in
/// `[appearance] icon_font`. (A proportional Nerd Font's icons overflow their
/// advance, so centring by advance put them to the right of their pill.)
pub fn paint_icon_centered(
    ctx: &mut PaintCtx<'_>,
    list: &mut DisplayList,
    glyph: &str,
    rect: Rect,
    size: f32,
    color: Color,
) {
    let style = TextStyle {
        family: std::sync::Arc::from(ctx.appearance.icon_font.as_str()),
        size,
        weight: 400,
        color,
    };
    let (cx, cy) = (rect.x + rect.width * 0.5, rect.y + rect.height * 0.5);
    let (x, baseline) = match ctx.text.ink_bounds(glyph, &style) {
        Some((left, top, right, bottom)) => (cx - (left + right) * 0.5, cy - (top + bottom) * 0.5),
        None => {
            let width = ctx.text.measure(glyph, &style).0;
            let (ascent, line_height) = ctx.text.metrics(&style);
            (cx - width * 0.5, rect.y + (rect.height - line_height) * 0.5 + ascent)
        }
    };
    ctx.text_text(list, glyph, x, baseline, style);
}

/// [`paint_text_centered`], turned clockwise by `radians` about the centre of
/// `rect` (QML's `rotation` on a centred item).
pub fn paint_text_centered_rotated(
    ctx: &mut PaintCtx<'_>,
    list: &mut DisplayList,
    value: &str,
    rect: Rect,
    size: f32,
    color: Color,
    radians: f32,
) {
    let style = TextStyle {
        family: std::sync::Arc::from(ctx.theme.font.as_str()),
        size,
        weight: 400,
        color,
    };
    let width = ctx.text.measure(value, &style).0;
    let (ascent, line_height) = ctx.text.metrics(&style);
    let x = rect.x + (rect.width - width) * 0.5;
    let baseline = rect.y + (rect.height - line_height) * 0.5 + ascent;
    let mut run = ctx.text.shape(value, &style, x, baseline);
    run.transform = Transform::rotate_about(radians, rect.x + rect.width * 0.5, rect.y + rect.height * 0.5);
    list.push_text(run);
}

/// Draw a single line vertically centred in `rect`, starting at `x`.
///
/// Uses the font's real ascent and line height, so text sits on the optical
/// centre regardless of size. Horizontal placement is the caller's choice.
#[allow(clippy::too_many_arguments)]
pub fn paint_text_vcentered(
    ctx: &mut PaintCtx<'_>,
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
        family: std::sync::Arc::from(ctx.theme.font.as_str()),
        size,
        weight,
        color,
    };
    let (ascent, line_height) = ctx.text.metrics(&style);
    let box_top = rect.y + (rect.height - line_height) * 0.5;
    let baseline = box_top + ascent;
    ctx.text_text(list, value, x, baseline, style);
}

/// Like [`paint_text_centered`] but with an explicit weight.
pub fn paint_text_centered_weighted(
    ctx: &mut PaintCtx<'_>,
    list: &mut DisplayList,
    value: &str,
    rect: Rect,
    size: f32,
    weight: u16,
    color: Color,
) {
    let style = TextStyle {
        family: std::sync::Arc::from(ctx.theme.font.as_str()),
        size,
        weight,
        color,
    };
    let width = ctx.text.measure(value, &style).0;
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
pub fn wrap_text(ctx: &mut PaintCtx<'_>, text: &str, size: f32, weight: u16, max_width: f32, max_lines: usize) -> Vec<String> {
    let style = TextStyle {
        family: std::sync::Arc::from(ctx.theme.font.as_str()),
        size,
        weight,
        color: ctx.theme.text_main,
    };
    wrap_with(text, max_width, max_lines, |s| ctx.text.measure(s, &style).0)
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
pub fn elide(ctx: &mut PaintCtx<'_>, text: &str, size: f32, weight: u16, max_width: f32) -> String {
    let style = TextStyle {
        family: std::sync::Arc::from(ctx.theme.font.as_str()),
        size,
        weight,
        color: ctx.theme.text_main,
    };
    let mut measure = |s: &str| ctx.text.measure(s, &style).0;
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
    fn inverted_corner_produces_a_closed_path() {
        let mut list = DisplayList::default();
        paint_inverted_corner(&mut list, 0.0, 0.0, 20.0, false, false, Color::WHITE);
        assert_eq!(list.commands.len(), 1);
        match &list.commands[0] {
            crate::render::display_list::DrawCommand::Path(path) => {
                assert_eq!(path.commands.last(), Some(&PathCommand::Close));
            }
            _ => panic!("expected a path"),
        }
    }

    fn corner_commands(flip: bool, mirror: bool) -> Vec<PathCommand> {
        let mut list = DisplayList::default();
        paint_inverted_corner(&mut list, 40.0, 0.0, 20.0, flip, mirror, Color::WHITE);
        match &list.commands[0] {
            crate::render::display_list::DrawCommand::Path(path) => path.commands.clone(),
            _ => panic!("expected a path"),
        }
    }

    /// The curve used to end at the corner point itself, which made it lie
    /// along the screen edge and enclose nothing: the corners never drew.
    #[test]
    fn inverted_corner_curves_back_to_its_start() {
        for (flip, mirror) in [(false, false), (true, false), (false, true), (true, true)] {
            let commands = corner_commands(flip, mirror);
            let PathCommand::MoveTo(start) = commands[0] else {
                panic!("expected MoveTo");
            };
            let PathCommand::QuadTo(control, end) = commands[3] else {
                panic!("expected QuadTo");
            };
            assert_eq!(end, start, "flip {flip} mirror {mirror}");
            assert_ne!(control, end);
        }
    }

    #[test]
    fn mirrored_corner_hugs_the_right_edge_of_its_square() {
        let PathCommand::MoveTo(start) = corner_commands(false, true)[0] else {
            panic!("expected MoveTo");
        };
        assert_eq!(start, Point::new(60.0, 20.0));
        let PathCommand::MoveTo(start) = corner_commands(true, false)[0] else {
            panic!("expected MoveTo");
        };
        assert_eq!(start, Point::new(40.0, 0.0));
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
