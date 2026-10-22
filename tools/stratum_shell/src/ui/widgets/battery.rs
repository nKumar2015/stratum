//! Battery indicator.

use std::collections::HashMap;

use crate::render::display_list::{DisplayList, Point, Rect};
use crate::ui::control_center::Page;
use crate::ui::hover_menu::HoverKind;
use crate::ui::{self, Action, ClickCtx, InputState, PaintCtx, Widget};

pub struct Battery {
    /// Rect from the last paint on each output; see `Icon` for why per output.
    rects: HashMap<String, Rect>,
}

impl Battery {
    pub fn new() -> Self {
        Self {
            rects: HashMap::new(),
        }
    }

    fn hit(&self, output: &str, point: Point) -> Option<Rect> {
        self.rects.get(output).copied().filter(|r| r.contains(point))
    }
}

impl Default for Battery {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for Battery {
    fn height(&self, _width: f32, metrics: &crate::ui::style::Metrics) -> Option<f32> {
        Some(metrics.typography.battery)
    }

    fn width(&self, height: f32, metrics: &crate::ui::style::Metrics) -> Option<f32> {
        Some(height.max(metrics.typography.battery))
    }

    fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        rect: Rect,
        _input: &InputState,
    ) {
        self.rects.insert(ctx.output_name.to_string(), rect);
        let battery = ctx.state.battery.get();
        let percent = battery["battery"]["pct"]
            .as_u64()
            .or_else(|| battery["battery"]["percentage"].as_f64().map(|p| p as u64))
            .unwrap_or(0)
            .min(100) as u8;
        let state_text = battery["battery"]["state"].as_str().unwrap_or("");
        let charging = matches!(state_text, "charging" | "pending-charge" | "fully-charged");

        // Codepoints from the QML battery widget's level buckets.
        let glyph = if charging {
            "\u{f0084}"
        } else {
            match percent {
                90..=100 => "\u{f0079}",
                80..=89 => "\u{f007a}",
                70..=79 => "\u{f007b}",
                60..=69 => "\u{f007c}",
                50..=59 => "\u{f007d}",
                40..=49 => "\u{f007e}",
                30..=39 => "\u{f007f}",
                20..=29 => "\u{f0080}",
                10..=19 => "\u{f0081}",
                _ => "\u{f0082}",
            }
        };
        let color = if percent <= 20 {
            ctx.theme.error
        } else {
            ctx.theme.success
        };
        ui::paint_text_centered(ctx, list, glyph, rect, ctx.metrics.typography.battery, color);
    }

    fn on_click(&mut self, ctx: &ClickCtx<'_>) -> Option<Action> {
        self.hit(ctx.output_name, ctx.point)
            .map(|_| Action::OpenPanel(Page::Battery))
    }

    fn hover_target(&self, output: &str, point: Point) -> Option<(HoverKind, Rect)> {
        self.hit(output, point).map(|rect| (HoverKind::Battery, rect))
    }
}
