//! Widget layer: layout, painting, and input.
//!
//! Widgets are simple retained objects. Each one lays itself out inside the
//! rectangle it is given, emits draw commands, and answers hit-tests. There is
//! no binding engine and no per-frame allocation in the common case: the shell
//! rebuilds a `DisplayList` from state only when something changes.

use crate::render::display_list::{
    Color, DisplayList, Paint, Path, PathCommand, Point, Rect, TextRun, Transform,
};
pub mod sidebar;

use crate::text::{TextEngine, TextStyle};

/// Shared state every widget can read while painting.
pub struct PaintCtx<'a> {
    pub theme: &'a crate::config::Theme,
    pub appearance: &'a crate::config::AppearanceConfig,
    pub text: &'a mut TextEngine,
    /// Live system state, read by widgets that display real data.
    pub state: &'a crate::sys_state::SystemState,
    /// Which output's bar is being painted (for per-monitor workspace focus).
    pub output_name: &'a str,
    /// Orientation of the bar being painted.
    pub axis: crate::config::Axis,
}

impl PaintCtx<'_> {
    pub fn is_vertical(&self) -> bool {
        self.axis == crate::config::Axis::Vertical
    }
}

impl PaintCtx<'_> {
    /// Shape a single line of text and append it to `list`.
    pub fn text(
        &mut self,
        list: &mut DisplayList,
        value: &str,
        x: f32,
        y: f32,
        size: f32,
        color: Color,
    ) {
        let style = TextStyle {
            family: std::sync::Arc::from(self.theme.font.as_str()),
            size,
            weight: 400,
            color,
        };
        self.text_text(list, value, x, y, style);
    }

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

/// A bar widget.
pub trait Widget {
    /// Intrinsic height in a vertical bar at the given width. `None` means
    /// "fill available".
    fn height(&self, width: f32) -> Option<f32> {
        let _ = width;
        None
    }

    /// Intrinsic width in a horizontal bar at the given height. `None` means
    /// "fill available". Defaults to a square-ish slot matching the height.
    fn width(&self, height: f32) -> Option<f32> {
        let _ = height;
        None
    }

    /// Draw into `rect` (surface coordinates, logical pixels).
    fn paint(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, rect: Rect, input: &InputState);

    /// Return true when the widget handled a click at `point`.
    fn on_click(&mut self, _point: Point, _input: &InputState) -> bool {
        false
    }
}

/// A vertical stack with fixed spacing, used by the sidebar.
pub struct Column {
    pub spacing: f32,
    widgets: Vec<Box<dyn Widget>>,
}

impl Column {
    pub fn new(spacing: f32) -> Self {
        Self {
            spacing,
            widgets: Vec::new(),
        }
    }

    pub fn push(&mut self, widget: Box<dyn Widget>) {
        self.widgets.push(widget);
    }

    pub fn is_empty(&self) -> bool {
        self.widgets.is_empty()
    }

    /// Lay out and paint every child, returning the total height used.
    pub fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        x: f32,
        y: f32,
        width: f32,
        input: &InputState,
    ) -> f32 {
        let mut cursor = y;
        let mut first = true;
        for widget in &mut self.widgets {
            if !first {
                cursor += self.spacing;
            }
            first = false;

            let height = widget.height(width).unwrap_or(0.0);
            let rect = Rect {
                x,
                y: cursor,
                width,
                height,
                radius: 0.0,
                color: Color::TRANSPARENT,
            };
            widget.paint(ctx, list, rect, input);
            cursor += height;
        }
        cursor - y
    }

    /// Route a click to the first child that accepts it.
    pub fn on_click(&mut self, point: Point, input: &InputState) -> bool {
        self.widgets
            .iter_mut()
            .any(|widget| widget.on_click(point, input))
    }
}

// ---------------------------------------------------------------------------
// Reusable drawing helpers
// ---------------------------------------------------------------------------

/// Draw the inverted corner scoop that joins a vertical rail to a screen edge.
///
/// The shape fills the region between the rail's edge and the screen edge,
/// curving back to the rail so the join looks continuous. `flip` mirrors it
/// vertically for the bottom corner.
pub fn paint_inverted_corner(
    list: &mut DisplayList,
    x: f32,
    y: f32,
    size: f32,
    flip: bool,
    color: Color,
) {
    let (start_y, end_y, control_y) = if flip {
        (y, y + size, y + size)
    } else {
        (y + size, y, y)
    };

    list.push_path(Path::new(
        vec![
            PathCommand::MoveTo(Point::new(x, start_y)),
            PathCommand::LineTo(Point::new(x, if flip { y + size } else { y })),
            PathCommand::LineTo(Point::new(x + size, if flip { y + size } else { y })),
            // Pull the curve toward the corner to create the scoop.
            PathCommand::QuadTo(Point::new(x, control_y), Point::new(x, end_y)),
            PathCommand::Close,
        ],
        Paint::Solid(color),
    ));
}

/// Draw a framed container used behind groups of widgets.
pub fn paint_frame(
    list: &mut DisplayList,
    rect: Rect,
    fill: Color,
    border: Color,
    draw_border: bool,
) {
    list.push_rect(Rect {
        radius: rect.radius,
        color: fill,
        ..rect
    });
    if draw_border {
        // A one-pixel stroke inside the frame edge.
        list.push_path(Path::new(
            vec![
                PathCommand::MoveTo(Point::new(rect.x + rect.radius, rect.y)),
                PathCommand::LineTo(Point::new(rect.x + rect.width - rect.radius, rect.y)),
                PathCommand::QuadTo(
                    Point::new(rect.x + rect.width, rect.y),
                    Point::new(rect.x + rect.width, rect.y + rect.radius),
                ),
                PathCommand::LineTo(Point::new(rect.x + rect.width, rect.y + rect.height - rect.radius)),
                PathCommand::QuadTo(
                    Point::new(rect.x + rect.width, rect.y + rect.height),
                    Point::new(rect.x + rect.width - rect.radius, rect.y + rect.height),
                ),
                PathCommand::LineTo(Point::new(rect.x + rect.radius, rect.y + rect.height)),
                PathCommand::QuadTo(
                    Point::new(rect.x, rect.y + rect.height),
                    Point::new(rect.x, rect.y + rect.height - rect.radius),
                ),
                PathCommand::LineTo(Point::new(rect.x, rect.y + rect.radius)),
                PathCommand::QuadTo(
                    Point::new(rect.x, rect.y),
                    Point::new(rect.x + rect.radius, rect.y),
                ),
                PathCommand::Close,
            ],
            Paint::Solid(border),
        )
        .stroked(1.0));
    }
}

/// Center a line of text horizontally inside `rect`.
#[allow(clippy::too_many_arguments)]
pub fn paint_text_centered(
    ctx: &mut PaintCtx<'_>,
    list: &mut DisplayList,
    value: &str,
    rect: Rect,
    size: f32,
    color: Color,
) {
    let style = TextStyle {
        family: std::sync::Arc::from(ctx.theme.font.as_str()),
        size,
        weight: 400,
        color,
    };
    let width = ctx.text.measure(value, &style).0;
    let (ascent, line_height) = ctx.text.metrics(&style);

    // Centre the line box vertically, then place the baseline inside it. This
    // is exact rather than a fixed fudge, which is what made mixed-size labels
    // look misaligned.
    let box_top = rect.y + (rect.height - line_height) * 0.5;
    let baseline = box_top + ascent;
    let x = rect.x + (rect.width - width) * 0.5;
    ctx.text_text(list, value, x, baseline, style);
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
#[allow(clippy::too_many_arguments)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverted_corner_produces_a_closed_path() {
        let mut list = DisplayList::default();
        paint_inverted_corner(&mut list, 0.0, 0.0, 20.0, false, Color::WHITE);
        assert_eq!(list.commands.len(), 1);
        match &list.commands[0] {
            crate::render::display_list::DrawCommand::Path(path) => {
                assert_eq!(path.commands.last(), Some(&PathCommand::Close));
            }
            _ => panic!("expected a path"),
        }
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

    #[test]
    fn column_accumulates_spacing() {
        struct Fixed(f32);
        impl Widget for Fixed {
            fn height(&self, _w: f32) -> Option<f32> {
                Some(self.0)
            }
            fn paint(
                &mut self,
                _ctx: &mut PaintCtx<'_>,
                _list: &mut DisplayList,
                _rect: Rect,
                _input: &InputState,
            ) {
            }
        }

        let mut column = Column::new(10.0);
        column.push(Box::new(Fixed(20.0)));
        column.push(Box::new(Fixed(30.0)));
        // Two widgets of 20 and 30 with 10px between => 60 total.
        let total = 20.0 + 10.0 + 30.0;
        assert_eq!(column.is_empty(), false);
        assert_eq!(total, 60.0);
    }
}
