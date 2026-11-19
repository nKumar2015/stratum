//! Night light: a moon, filled while the screens are warmer than daylight.
//! A click turns it on or off by hand until the schedule changes anyway.

use std::collections::HashMap;

use crate::render::display_list::{DisplayList, Rect};
use crate::ui::{self, Action, ClickCtx, InputState, PaintCtx, Widget};

#[derive(Default)]
pub struct NightLight {
    /// Where it was last painted on each output, for hit-testing.
    rects: HashMap<String, Rect>,
}

impl Widget for NightLight {
    fn height(&self, _width: f32, metrics: &crate::ui::style::Metrics) -> Option<f32> {
        Some(metrics.typography.icon)
    }

    fn width(&self, height: f32, metrics: &crate::ui::style::Metrics) -> Option<f32> {
        Some(height.max(metrics.typography.icon))
    }

    fn paint(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, rect: Rect, _input: &InputState) {
        self.rects.insert(ctx.output_name.to_string(), rect);
        // nf-md-weather_night and nf-md-moon_waning_crescent.
        let (glyph, color) = if ctx.state.night_light.get().on {
            ("\u{f0594}", ctx.theme.primary)
        } else {
            ("\u{f0f65}", ctx.theme.text_muted)
        };
        ui::paint_icon_centered(ctx, list, glyph, rect, ctx.metrics.typography.icon, color);
    }

    fn on_click(&mut self, ctx: &ClickCtx<'_>) -> Option<Action> {
        self.rects
            .get(ctx.output_name)
            .filter(|rect| rect.contains(ctx.point))
            .map(|_| Action::ToggleNightLight)
    }
}
