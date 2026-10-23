//! Pieces shared by the Bluetooth and Network pages.
//!
//! Both are built the same way on [`layout`]: a header with refresh (and
//! scan) at its right, a power card whose caption doubles as the status
//! line (QML `StatusMessageFooter`), then full-width rows that expand in
//! place to show their actions. The QML menus put those actions in a side
//! panel, which left no room for the list in a tiled window.
//!
//! The QML `MarqueeLabel` lives here too, for the Audio page and the
//! dashboard.

use std::sync::Arc;

use crate::render::display_list::{Color, DisplayList, Rect};
use crate::text::TextStyle;
use crate::ui::controls;
use crate::ui::pages::layout::{self, rect, Card};
use crate::ui::{self, InputState, PaintCtx};

/// What the header's buttons asked for.
pub struct HeaderClick {
    pub body: Rect,
    /// The index of the glyph button clicked.
    pub button: Option<usize>,
}

/// The page header with round glyph buttons at its right, `(glyph,
/// enabled)`. While `loading` holds a phase, a pulsing "Loading" sits
/// before the buttons.
#[allow(clippy::too_many_arguments)]
pub fn header(
    ctx: &mut PaintCtx<'_>,
    list: &mut DisplayList,
    area: Rect,
    title: &str,
    summary: &str,
    buttons: &[(&str, bool)],
    loading: Option<f32>,
    input: &InputState,
    clicked: bool,
) -> HeaderClick {
    let m = ctx.metrics.page.clone();
    let gap = m.grid_gap * 0.5;
    let buttons_w = buttons.len() as f32 * (m.badge + gap);
    let loading_w = if loading.is_some() { loading_width(ctx) + m.grid_gap } else { 0.0 };
    let (head, body) = layout::header(
        ctx,
        list,
        Rect {
            width: area.width - buttons_w - loading_w,
            ..area
        },
        title,
        summary,
    );
    let y = head.y + (head.height - m.badge) * 0.5;
    let mut x = area.x + area.width - buttons_w + gap;
    if let Some(phase) = loading {
        paint_loading(ctx, list, rect(x - loading_w, y, loading_w - m.grid_gap, m.badge), phase);
    }
    let mut button = None;
    for (i, (glyph, enabled)) in buttons.iter().enumerate() {
        if layout::icon_button(ctx, list, x, y, glyph, false, ctx.theme.primary, input, clicked && *enabled) {
            button = Some(i);
        }
        x += m.badge + gap;
    }
    HeaderClick {
        body: Rect { width: area.width, ..body },
        button,
    }
}

/// The radio's card: a badge, a title over a caption, and a switch. A click
/// anywhere on it flips the switch while `enabled`. Returns the new state.
#[allow(clippy::too_many_arguments)]
pub fn power_card(
    ctx: &mut PaintCtx<'_>,
    list: &mut DisplayList,
    cell: Rect,
    glyph: &str,
    title: &str,
    caption: &str,
    caption_color: Color,
    on: bool,
    enabled: bool,
    input: &InputState,
    clicked: bool,
) -> Option<bool> {
    let m = ctx.metrics.page.clone();
    let theme = ctx.theme.clone();
    let hot = enabled && layout::hovered(input, cell);
    layout::card(ctx, list, cell, if hot { Card::Hover } else { Card::Plain });
    let pad = m.card_padding;
    let (fg, bg) = if on {
        (theme.on_primary, theme.primary)
    } else {
        (theme.text_muted, theme.surface_highest)
    };
    layout::badge(ctx, list, cell.x + pad, cell.y + (cell.height - m.badge) * 0.5, glyph, fg, bg);
    let switch_x = cell.x + cell.width - pad - m.toggle_width;
    let switch_y = cell.y + (cell.height - m.toggle_height) * 0.5;
    controls::switch(ctx, list, switch_x, switch_y, on, input, false);
    let text_x = cell.x + pad + m.badge + pad * 0.75;
    layout::two_lines(ctx, list, rect(text_x, cell.y, switch_x - pad - text_x, cell.height), title, caption, caption_color);
    (hot && clicked).then_some(!on)
}

/// The line under an expanded row's title: a caption at the left and pills
/// at the right, inside `area`. Returns the pill clicked.
#[allow(clippy::too_many_arguments)]
pub fn row_actions(
    ctx: &mut PaintCtx<'_>,
    list: &mut DisplayList,
    area: Rect,
    caption: &str,
    labels: &[&str],
    enabled: bool,
    input: &InputState,
    clicked: bool,
) -> Option<usize> {
    let m = ctx.metrics.page.clone();
    let typo = ctx.metrics.typography.clone();
    let y = area.y + (area.height - m.badge) * 0.5;
    let (left, hit) = layout::text_buttons(ctx, list, area.x + area.width, y, labels, enabled, input, clicked);
    let caption = ui::elide(ctx, caption, typo.caption, typo.weight_regular, left - m.grid_gap - area.x);
    ui::paint_text_vcentered(ctx, list, &caption, area.x, area, typo.caption, typo.weight_regular, ctx.theme.text_muted);
    hit
}

/// Where a row's expansion goes: below its top `height`, inset like its text.
pub fn expansion(ctx: &PaintCtx<'_>, row: Rect, height: f32) -> Rect {
    let m = &ctx.metrics.page;
    let x = row.x + m.card_padding + m.badge + m.card_padding * 0.75;
    rect(x, row.y + height, row.x + row.width - m.card_padding - x, row.height - height - m.card_padding)
}

/// A card for an empty list.
pub fn empty_card(ctx: &mut PaintCtx<'_>, list: &mut DisplayList, body: Rect, y: f32, title: &str, caption: &str) -> f32 {
    let height = ctx.metrics.page.info_height;
    layout::info_card(ctx, list, rect(body.x, y, body.width, height), title, caption);
    y + height
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
        let Some((text, error)) = self.message() else {
            return;
        };
        let typo = ctx.metrics.typography.clone();
        let color = if error { ctx.theme.error } else { ctx.theme.text_muted };
        let text = ui::elide(ctx, text, typo.caption, typo.weight_regular, rect.width);
        ui::paint_text_vcentered(ctx, list, &text, rect.x, rect, typo.caption, typo.weight_regular, color);
    }

    /// The message showing, and whether it is an error.
    pub fn message(&self) -> Option<(&str, bool)> {
        (!self.text.is_empty()).then_some((self.text.as_str(), self.error))
    }
}

const LOADING: &str = "Loading";

fn loading_width(ctx: &mut PaintCtx<'_>) -> f32 {
    let m = ctx.metrics.device_page.clone();
    m.loading_dot + m.gap + ctx.measure(LOADING, ctx.metrics.typography.caption)
}

/// A pulsing dot and "Loading" at the right of `rect`, while a refresh is
/// in flight. `phase` is seconds since it started.
fn paint_loading(ctx: &mut PaintCtx<'_>, list: &mut DisplayList, rect: Rect, phase: f32) {
    let m = ctx.metrics.device_page.clone();
    let typo = ctx.metrics.typography.clone();
    let period = (m.loading_ms as f32 / 1000.0).max(0.1);
    // 0.3 → 1 → 0.3 once a period, as the QML animation did.
    let t = (phase % period) / period;
    let alpha = 0.3 + 0.7 * (1.0 - (2.0 * t - 1.0).abs());
    let x = rect.x + rect.width - loading_width(ctx);
    list.push_rect(Rect {
        x,
        y: rect.y + (rect.height - m.loading_dot) * 0.5,
        width: m.loading_dot,
        height: m.loading_dot,
        radius: m.loading_dot * 0.5,
        color: ctx.theme.primary.with_alpha(alpha),
    });
    let label_x = x + m.loading_dot + m.gap;
    ui::paint_text_vcentered(ctx, list, LOADING, label_x, rect, typo.caption, typo.weight_regular, ctx.theme.text_muted);
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
