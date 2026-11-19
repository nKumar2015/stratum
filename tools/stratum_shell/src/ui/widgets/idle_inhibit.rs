//! Stay awake: a cup that holds the idle stages (dim, lock, screens off,
//! suspend) off while it is full, like a phone's caffeine tile.

use std::collections::HashMap;

use crate::render::display_list::{DisplayList, Rect};
use crate::ui::{self, Action, ClickCtx, InputState, PaintCtx, Widget};

#[derive(Default)]
pub struct IdleInhibit {
    /// Where it was last painted on each output, for hit-testing. The widget
    /// tree is shared by every output, whose rails differ.
    rects: HashMap<String, Rect>,
}

impl Widget for IdleInhibit {
    fn height(&self, _width: f32, metrics: &crate::ui::style::Metrics) -> Option<f32> {
        Some(metrics.typography.icon)
    }

    fn width(&self, height: f32, metrics: &crate::ui::style::Metrics) -> Option<f32> {
        Some(height.max(metrics.typography.icon))
    }

    fn paint(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, rect: Rect, _input: &InputState) {
        self.rects.insert(ctx.output_name.to_string(), rect);
        // nf-md-coffee and nf-md-coffee_outline.
        let (glyph, color) = if ctx.state.idle_inhibit.get() {
            ("\u{f0176}", ctx.theme.primary)
        } else {
            ("\u{f06ca}", ctx.theme.text_muted)
        };
        ui::paint_icon_centered(ctx, list, glyph, rect, ctx.metrics.typography.icon, color);
    }

    fn on_click(&mut self, ctx: &ClickCtx<'_>) -> Option<Action> {
        self.rects
            .get(ctx.output_name)
            .filter(|rect| rect.contains(ctx.point))
            .map(|_| Action::ToggleIdleInhibit)
    }
}
