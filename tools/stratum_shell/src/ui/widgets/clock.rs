//! The clock.
//!
//! A horizontal bar shows one line; a vertical rail stacks hour, minute, and
//! meridiem, matching the QML clock. A tap opens the dashboard, as QML's did.
//! The widget also schedules its own repaint: a clock that only redraws when
//! other state changes would sit frozen for minutes at a time.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Timelike;

use crate::render::display_list::{Color, DisplayList, Rect};
use crate::text::TextStyle;
use crate::ui::{self, Action, ClickCtx, InputState, PaintCtx, Widget};

pub struct Clock {
    /// Seconds accumulated since the last check of the minute.
    elapsed: f32,
    /// Minute value from the last paint, so a change can be detected.
    last_minute: u32,
    /// Rect from the last paint on each output; see `Icon` for why per output.
    rects: HashMap<String, Rect>,
}

impl Clock {
    pub fn new() -> Self {
        Self {
            elapsed: 0.0,
            last_minute: u32::MAX,
            rects: HashMap::new(),
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
        Some(metrics.typography.clock * metrics.bar.label_line * 3.0)
    }

    fn width(&self, _height: f32, metrics: &crate::ui::style::Metrics) -> Option<f32> {
        Some(metrics.bar.clock_width)
    }

    fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        rect: Rect,
        _input: &InputState,
    ) {
        self.rects.insert(ctx.output_name.to_string(), rect);
        let now = chrono::Local::now();
        self.last_minute = now.minute();

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
        let bold = ctx.metrics.typography.weight_bold;
        let regular = ctx.metrics.typography.weight_regular;
        let lines: [(String, Color, u16); 3] = [
            (now.format("%I").to_string(), ctx.theme.primary, bold),
            (now.format("%M").to_string(), ctx.theme.secondary, bold),
            (now.format("%p").to_string(), ctx.theme.tertiary, regular),
        ];
        for (index, (value, color, weight)) in lines.into_iter().enumerate() {
            let row = Rect {
                x: rect.x,
                y: rect.y + line * index as f32,
                width: rect.width,
                height: line,
                radius: 0.0,
                color: Color::TRANSPARENT,
            };
            ui::paint_text_centered_weighted(
                ctx,
                list,
                &value,
                row,
                ctx.metrics.typography.clock,
                weight,
                color,
            );
        }
    }

    fn on_click(&mut self, ctx: &ClickCtx<'_>) -> Option<Action> {
        self.rects
            .get(ctx.output_name)
            .filter(|r| r.contains(ctx.point))
            .map(|_| Action::OpenDashboard(ctx.output_name.to_string()))
    }

    fn tick(&mut self, dt: f32) -> bool {
        // Ask for a frame once the displayed minute has changed. The loop wakes
        // at least once a second when idle, so the clock turns over within a
        // second of the minute boundary.
        self.elapsed += dt;
        if self.elapsed >= 1.0 {
            self.elapsed = 0.0;
            return chrono::Local::now().minute() != self.last_minute;
        }
        false
    }
}
