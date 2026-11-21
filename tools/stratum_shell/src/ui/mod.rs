//! Widget layer: layout, painting, and input.
//!
//! Widgets are simple retained objects. Each one lays itself out inside the
//! rectangle it is given, emits draw commands, and answers hit-tests. There is
//! no binding engine and no per-frame allocation in the common case: the shell
//! rebuilds a `DisplayList` from state only when something changes.

use crate::render::display_list::{
    Color, DisplayList, Paint, Path, PathCommand, Point, Rect,
};
pub use stratum_ui::widgets::draw::*;
pub use stratum_ui::widgets::{controls, text_field};
pub use stratum_ui::{animation, color, graph, style};
pub mod control_center;
pub mod hover_menu;
pub mod notification_card;
pub mod pages;
pub mod sidebar;
pub mod widgets;

use crate::text::TextEngine;

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
    /// The Display page's settings.
    pub displays: &'a crate::config::DisplaysConfig,
    /// The settings the Battery and Display pages edit.
    pub idle: &'a crate::config::IdleConfig,
    pub night_light: &'a crate::config::NightLightConfig,
    /// Which of `themes` are custom themes the user made (and may edit).
    pub custom_themes: &'a std::collections::BTreeSet<String>,
    /// Which output's bar is being painted (for per-monitor workspace focus).
    pub output_name: &'a str,
    /// Orientation of the bar being painted.
    pub axis: crate::config::Axis,
    /// Seconds since the previous paint, for widgets that animate themselves.
    pub dt: f32,
}

impl stratum_ui::widgets::Ui for PaintCtx<'_> {
    fn theme(&self) -> &crate::config::Theme {
        self.theme
    }
    fn appearance(&self) -> &crate::config::AppearanceConfig {
        self.appearance
    }
    fn metrics(&self) -> &crate::ui::style::Metrics {
        self.metrics
    }
    fn text(&mut self) -> &mut TextEngine {
        self.text
    }
    fn icons(&mut self) -> &mut crate::icons::IconTheme {
        self.icons
    }
}

impl PaintCtx<'_> {
    pub fn is_vertical(&self) -> bool {
        self.axis == crate::config::Axis::Vertical
    }
}

impl PaintCtx<'_> {
    /// Measure a string in the theme font at `size` (the [`Ui`] method, here
    /// without importing the trait).
    ///
    /// [`Ui`]: stratum_ui::widgets::Ui
    pub fn measure(&mut self, value: &str, size: f32) -> f32 {
        stratum_ui::widgets::Ui::measure(self, value, size)
    }
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
    /// Stay awake, or stop (the idle_inhibit icon).
    ToggleIdleInhibit,
    /// Night light on or off by hand (the night_light icon).
    ToggleNightLight,
    /// Open a hover menu at once (an icon with no page behind it).
    ShowHoverMenu(crate::ui::hover_menu::OpenMenu),
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

/// The colours that stand for a theme, in order: its two main backgrounds
/// and its three accents (a Theme page card's strip, the switcher's dots).
pub fn theme_swatches(theme: &crate::config::Theme) -> [Color; 5] {
    [theme.bg_main, theme.bg_widget, theme.primary, theme.secondary, theme.tertiary]
}

#[cfg(test)]
mod tests {
    use super::*;

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

}
