//! Pieces shared by the Bluetooth and Network pages.
//!
//! Both follow the QML menus' layout: a status card for the active device, a
//! toolbar, the list with a detail panel beside it, and a status line. The
//! QML `MarqueeLabel`, `TripleStatusStrip`, `StatusMessageFooter` and
//! `CompactIconButton` live here as immediate-mode helpers.

use std::sync::Arc;

use crate::render::display_list::{Color, DisplayList, Point, Rect};
use crate::text::TextStyle;
use crate::ui::{self, InputState, PaintCtx};

pub fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
    Rect { x, y, width: width.max(0.0), height: height.max(0.0), radius: 0.0, color: Color::TRANSPARENT }
}

fn hovered(input: &InputState, rect: Rect) -> bool {
    input.pointer.is_some_and(|(x, y)| rect.contains(Point::new(x, y)))
}

/// A raised block with the theme's border (the QML cards and list rows).
pub fn card(ctx: &PaintCtx<'_>, list: &mut DisplayList, rect: Rect, fill: Color, border: Color) {
    let m = &ctx.metrics.device_page;
    crate::ui::text_field::paint_frame(list, Rect { radius: m.radius, ..rect }, m.border, border, fill);
}

/// A square button with a glyph. Returns true when clicked while enabled.
#[allow(clippy::too_many_arguments)]
pub fn icon_button(
    ctx: &mut PaintCtx<'_>,
    list: &mut DisplayList,
    rect: Rect,
    glyph: &str,
    color: Color,
    enabled: bool,
    input: &InputState,
    clicked: bool,
) -> bool {
    let m = &ctx.metrics.device_page;
    let hot = enabled && hovered(input, rect);
    let fill = if !enabled {
        ctx.theme.bg_dark
    } else if hot {
        ctx.theme.bg_hover
    } else {
        ctx.theme.bg_widget
    };
    list.push_rect(Rect { radius: m.radius, color: fill, ..rect });
    let color = if enabled { color } else { ctx.theme.text_muted };
    ui::paint_text_centered(ctx, list, glyph, rect, m.icon, color);
    hot && clicked
}

/// A labelled button that can be disabled. Returns true when clicked while
/// enabled.
#[allow(clippy::too_many_arguments)]
pub fn text_button(
    ctx: &mut PaintCtx<'_>,
    list: &mut DisplayList,
    rect: Rect,
    label: &str,
    enabled: bool,
    input: &InputState,
    clicked: bool,
) -> bool {
    let m = &ctx.metrics.device_page;
    let hot = enabled && hovered(input, rect);
    let fill = if !enabled {
        ctx.theme.bg_dark
    } else if hot {
        ctx.theme.bg_hover
    } else {
        ctx.theme.bg_widget
    };
    card(ctx, list, rect, fill, ctx.theme.border_inactive);
    let color = if enabled { ctx.theme.text_main } else { ctx.theme.text_muted };
    ui::paint_text_centered_weighted(ctx, list, label, rect, m.strip, ctx.metrics.typography.weight_bold, color);
    hot && clicked
}

/// `Label value · Label value · …` on one line (QML `TripleStatusStrip`),
/// labels in `label_color`, each value in its own colour. Clipped to `rect`.
pub fn strip(
    ctx: &mut PaintCtx<'_>,
    list: &mut DisplayList,
    rect: Rect,
    items: &[(&str, &str, Color)],
    size: f32,
    label_color: Color,
) {
    let space = ctx.measure(" ", size);
    let dot = " · ";
    let mut x = rect.x;
    list.push_clip(rect);
    for (i, (label, value, color)) in items.iter().enumerate() {
        if i > 0 {
            ui::paint_text_vcentered(ctx, list, dot, x, rect, size, 400, ctx.theme.text_muted);
            x += ctx.measure(dot, size);
        }
        if !label.is_empty() {
            ui::paint_text_vcentered(ctx, list, label, x, rect, size, 400, label_color);
            x += ctx.measure(label, size) + space;
        }
        ui::paint_text_vcentered(ctx, list, value, x, rect, size, 400, *color);
        x += ctx.measure(value, size);
    }
    list.pop_clip();
}

/// The scrolling name in a detail panel (QML `MarqueeLabel`), and the
/// dashboard's track title (`Marquee::ticker`): still when it
/// fits; otherwise it scrolls left at `speed` px/s with a separator and
/// `gap` before it repeats, seamlessly.
#[derive(Default)]
pub struct Marquee {
    text: String,
    offset: f32,
    moving: bool,
    /// The dashboard's title ticker: centred while it fits and repeating
    /// with only the gap, no separator (`DashboardMusicPanel.qml`).
    ticker: bool,
}

/// The separator QML appended to an overflowing label.
const MARQUEE_SEPARATOR: &str = "    | ";

/// Advance a marquee by `dt`, wrapping at `span`.
pub fn advance(offset: f32, dt: f32, speed: f32, span: f32) -> f32 {
    if span <= 0.0 {
        return 0.0;
    }
    (offset + dt.max(0.0) * speed).rem_euclid(span)
}

impl Marquee {
    pub fn ticker() -> Self {
        Self { ticker: true, ..Self::default() }
    }

    /// Whether it needs frames.
    pub fn moving(&self) -> bool {
        self.moving
    }

    #[allow(clippy::too_many_arguments)]
    pub fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        rect: Rect,
        text: &str,
        size: f32,
        color: Color,
    ) {
        let m = ctx.metrics.device_page.clone();
        if self.text != text {
            self.text = text.to_string();
            self.offset = 0.0;
        }
        let style = TextStyle {
            family: Arc::from(ctx.theme.font.as_str()),
            size,
            weight: ctx.metrics.typography.weight_bold,
            color,
        };
        let width = ctx.text.measure(text, &style).0;
        let weight = ctx.metrics.typography.weight_bold;
        if width <= rect.width {
            self.moving = false;
            self.offset = 0.0;
            let x = if self.ticker { rect.x + ((rect.width - width) * 0.5).round() } else { rect.x };
            ui::paint_text_vcentered(ctx, list, text, x, rect, size, weight, color);
            return;
        }
        self.moving = true;
        let separator = if self.ticker { "" } else { MARQUEE_SEPARATOR };
        let looped = format!("{text}{separator}");
        let span = ctx.text.measure(&looped, &style).0 + m.marquee_gap;
        self.offset = advance(self.offset, ctx.dt, m.marquee_speed, span);
        list.push_clip(rect);
        for start in [rect.x - self.offset, rect.x - self.offset + span] {
            ui::paint_text_vcentered(ctx, list, &looped, start, rect, size, weight, color);
        }
        list.pop_clip();
    }
}

/// The status line under a page (QML `StatusMessageFooter`). Progress
/// messages stay until replaced; results clear after `status_ms`.
#[derive(Default)]
pub struct Status {
    text: String,
    error: bool,
    /// Seconds left for a result; `None` for a message that stays.
    remaining: Option<f32>,
}

impl Status {
    /// A message that stays until replaced ("Connecting to …").
    pub fn progress(&mut self, text: String) {
        self.text = text;
        self.error = false;
        self.remaining = None;
    }

    /// A result that clears on its own.
    pub fn result(&mut self, text: String, error: bool, status_ms: u64) {
        self.text = text;
        self.error = error;
        self.remaining = Some(status_ms as f32 / 1000.0);
    }

    pub fn tick(&mut self, dt: f32) {
        if let Some(remaining) = &mut self.remaining {
            *remaining -= dt;
            if *remaining <= 0.0 {
                self.text.clear();
                self.remaining = None;
            }
        }
    }

    pub fn paint(&self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, rect: Rect) {
        if self.text.is_empty() {
            return;
        }
        let m = &ctx.metrics.device_page;
        let color = if self.error { ctx.theme.error } else { ctx.theme.text_muted };
        list.push_clip(rect);
        ui::paint_text_vcentered(ctx, list, &self.text, rect.x, rect, m.footer, 400, color);
        list.pop_clip();
    }
}

/// A pulsing dot and "Loading", while a refresh is in flight. `phase` is
/// seconds since it started.
pub fn loading(ctx: &mut PaintCtx<'_>, list: &mut DisplayList, rect: Rect, phase: f32) -> f32 {
    let m = ctx.metrics.device_page.clone();
    let period = (m.loading_ms as f32 / 1000.0).max(0.1);
    // 0.3 → 1 → 0.3 once a period, as the QML animation did.
    let t = (phase % period) / period;
    let alpha = 0.3 + 0.7 * (1.0 - (2.0 * t - 1.0).abs());
    let label = "Loading";
    let width = m.loading_dot + m.gap + ctx.measure(label, m.strip);
    let x = rect.x + rect.width - width;
    list.push_rect(Rect {
        x,
        y: rect.y + (rect.height - m.loading_dot) * 0.5,
        width: m.loading_dot,
        height: m.loading_dot,
        radius: m.loading_dot * 0.5,
        color: ctx.theme.primary.with_alpha(alpha),
    });
    ui::paint_text_vcentered(ctx, list, label, x + m.loading_dot + m.gap, rect, m.strip, 400, ctx.theme.text_muted);
    width
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_marquee_wraps_seamlessly() {
        assert_eq!(advance(0.0, 1.0, 42.0, 100.0), 42.0);
        assert_eq!(advance(90.0, 0.5, 42.0, 100.0), 11.0);
        assert_eq!(advance(10.0, 1.0, 42.0, 0.0), 0.0, "nothing to scroll");
    }

    #[test]
    fn results_clear_but_progress_stays() {
        let mut status = Status::default();
        status.progress("Connecting…".into());
        status.tick(60.0);
        assert_eq!(status.text, "Connecting…");
        status.result("Connected.".into(), false, 3500);
        status.tick(3.0);
        assert_eq!(status.text, "Connected.");
        status.tick(0.6);
        assert!(status.text.is_empty());
    }
}
