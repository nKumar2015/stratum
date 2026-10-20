//! The clock.
//!
//! A horizontal bar shows one line; a vertical rail stacks hour, minute, and
//! meridiem, matching the QML clock. The widget also schedules its own repaint:
//! a clock that only redraws when other state changes would sit frozen for
//! minutes at a time.

use std::sync::Arc;

use crate::render::display_list::{Color, DisplayList, Rect};
use crate::text::TextStyle;
use crate::ui::{self, InputState, PaintCtx, Widget};

pub struct Clock {
    /// Seconds accumulated since the last minute boundary.
    elapsed: f32,
    /// Minute value from the last paint, so a change can be detected.
    last_minute: u32,
}

impl Clock {
    pub fn new() -> Self {
        Self {
            elapsed: 0.0,
            last_minute: u32::MAX,
        }
    }
}

impl Default for Clock {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for Clock {
    fn height(&self, _width: f32, metrics: &crate::ui::style::Metrics) -> Option<f32> {
        // Three stacked lines of clock text.
        Some(metrics.typography.clock * 3.6)
    }

    fn width(&self, _height: f32, _metrics: &crate::ui::style::Metrics) -> Option<f32> {
        Some(84.0)
    }

    fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        rect: Rect,
        _input: &InputState,
    ) {
        let now = chrono::Local::now();
        self.last_minute = now.format("%M").to_string().parse().unwrap_or(0);

        if !ctx.is_vertical() {
            let text = now.format("%I:%M %p").to_string();
            let style = TextStyle {
                family: Arc::from(ctx.theme.font.as_str()),
                size: ctx.metrics.typography.clock_bar,
                weight: ctx.metrics.typography.weight_medium,
                color: ctx.theme.primary,
            };
            let width = ctx.text.measure(&text, &style).0;
            let x = rect.x + (rect.width - width) * 0.5;
            ui::paint_text_vcentered(
                ctx,
                list,
                &text,
                x,
                rect,
                style.size,
                style.weight,
                style.color,
            );
            return;
        }

        let line = rect.height / 3.0;
        let lines: [(String, Color); 3] = [
            (now.format("%I").to_string(), ctx.theme.primary),
            (now.format("%M").to_string(), ctx.theme.secondary),
            (now.format("%p").to_string(), ctx.theme.tertiary),
        ];
        for (index, (value, color)) in lines.into_iter().enumerate() {
            let row = Rect {
                x: rect.x,
                y: rect.y + line * index as f32,
                width: rect.width,
                height: line,
                radius: 0.0,
                color: Color::TRANSPARENT,
            };
            ui::paint_text_centered(ctx, list, &value, row, ctx.metrics.typography.clock, color);
        }
    }

    fn tick(&mut self, dt: f32) -> bool {
        // Ask for a frame exactly when the displayed minute could have changed.
        self.elapsed += dt;
        if self.elapsed >= 1.0 {
            self.elapsed = 0.0;
            let minute: u32 = chrono::Local::now()
                .format("%M")
                .to_string()
                .parse()
                .unwrap_or(0);
            if minute != self.last_minute {
                return true;
            }
        }
        false
    }
}
