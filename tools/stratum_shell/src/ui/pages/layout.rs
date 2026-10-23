//! The scaffold settings pages are built from.
//!
//! Every page is a header (title and a one-line summary) followed by sections
//! of cards. Cards sit in grids whose column count follows the window width,
//! so one page reads well both tiled (a tall, narrow column) and floating (a
//! wide, short window). The page column stops growing at
//! `[metrics.page] max_width` and is centred beyond that.
//!
//! Elevation comes from the theme's surface ramp: the page sits on `surface`,
//! cards on `surface_high`, and a hovered card on `surface_highest`.
//!
//! The geometry functions are pure, so the layout maths is tested without a
//! renderer.

use crate::render::display_list::{Color, DisplayList, Point, Rect};
use crate::ui::{self, InputState, PaintCtx};

/// A rect with no fill or radius.
pub fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
    Rect {
        x,
        y,
        width: width.max(0.0),
        height: height.max(0.0),
        radius: 0.0,
        color: Color::TRANSPARENT,
    }
}

/// The page column: `area` no wider than `max_width`, centred in it.
pub fn column(area: Rect, max_width: f32) -> Rect {
    let width = area.width.min(max_width).max(0.0);
    Rect {
        x: area.x + (area.width - width) * 0.5,
        width,
        ..area
    }
}

/// How many columns of at least `min` fit in `width`, from one up to `max`.
pub fn columns(width: f32, min: f32, gap: f32, max: usize) -> usize {
    let fit = ((width + gap) / (min + gap).max(1.0)).floor() as usize;
    fit.clamp(1, max.max(1))
}

/// Cells of a grid laid out left to right, then down.
pub fn cells(x: f32, y: f32, width: f32, count: usize, cols: usize, height: f32, gap: f32) -> Vec<Rect> {
    let cols = cols.max(1);
    let cell_w = ((width - gap * (cols - 1) as f32) / cols as f32).max(0.0);
    (0..count)
        .map(|i| {
            let (col, row) = (i % cols, i / cols);
            rect(
                x + col as f32 * (cell_w + gap),
                y + row as f32 * (height + gap),
                cell_w,
                height,
            )
        })
        .collect()
}

/// Fewer columns when `count` does not divide evenly, so a row of four never
/// leaves a lone card under three.
pub fn even_columns(cols: usize, count: usize) -> usize {
    (1..=cols).rev().find(|c| count.is_multiple_of(*c)).unwrap_or(1)
}

/// Height of a grid of `count` cells.
pub fn grid_height(count: usize, cols: usize, height: f32, gap: f32) -> f32 {
    if count == 0 {
        return 0.0;
    }
    let rows = count.div_ceil(cols.max(1));
    rows as f32 * height + (rows - 1) as f32 * gap
}

/// Whether the pointer is inside `rect`.
pub fn hovered(input: &InputState, rect: Rect) -> bool {
    input
        .pointer
        .is_some_and(|(x, y)| rect.contains(Point::new(x, y)))
}

/// Draw the page title and its summary line. Returns the header's own rect,
/// so a page can put buttons at its right, and the body below it.
pub fn header(
    ctx: &mut PaintCtx<'_>,
    list: &mut DisplayList,
    area: Rect,
    title: &str,
    summary: &str,
) -> (Rect, Rect) {
    let m = ctx.metrics.page.clone();
    let typo = ctx.metrics.typography.clone();
    let title_h = m.title * 1.3;
    let summary_h = m.summary * 1.5;
    let height = title_h + m.header_gap + summary_h;
    let head = rect(area.x, area.y, area.width, height);
    ui::paint_text_vcentered(
        ctx,
        list,
        title,
        area.x,
        rect(area.x, area.y, area.width, title_h),
        m.title,
        typo.weight_bold,
        ctx.theme.text_main,
    );
    if !summary.is_empty() {
        let line = ui::elide(ctx, summary, m.summary, typo.weight_regular, area.width);
        ui::paint_text_vcentered(
            ctx,
            list,
            &line,
            area.x,
            rect(area.x, area.y + title_h + m.header_gap, area.width, summary_h),
            m.summary,
            typo.weight_regular,
            ctx.theme.text_muted,
        );
    }
    let top = area.y + height + m.header_bottom;
    (head, rect(area.x, top, area.width, area.y + area.height - top))
}

/// A section label at `y`. Returns the y where the section's cards start.
pub fn section(ctx: &mut PaintCtx<'_>, list: &mut DisplayList, area: Rect, y: f32, label: &str) -> f32 {
    let height = ctx.metrics.page.section_label_height;
    ui::paint_text_vcentered(
        ctx,
        list,
        label,
        area.x,
        rect(area.x, y, area.width, height),
        ctx.metrics.typography.label,
        ctx.metrics.typography.weight_bold,
        ctx.theme.text_muted,
    );
    y + height + ctx.metrics.page.label_gap
}

/// How a card is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Card {
    /// A resting card.
    Plain,
    /// Under the pointer, when the card is clickable.
    Hover,
    /// Selected or switched on: tinted with the accent.
    Active,
    /// Waiting for a second click: tinted with the error colour.
    Danger,
}

/// Draw a card in `style`.
pub fn card(ctx: &PaintCtx<'_>, list: &mut DisplayList, rect: Rect, style: Card) {
    let m = &ctx.metrics.page;
    let theme = ctx.theme;
    let rect = Rect { radius: m.card_radius, ..rect };
    let (fill, border, width) = match style {
        Card::Plain => (theme.surface_high, theme.outline, m.outline),
        Card::Hover => (theme.surface_highest, theme.outline, m.outline),
        Card::Active => (
            mix(theme.surface_high, theme.primary, 0.16),
            theme.primary,
            m.outline_active,
        ),
        Card::Danger => (
            mix(theme.surface_high, theme.error, 0.16),
            theme.error,
            m.outline_active,
        ),
    };
    list.push_rect(Rect { color: fill, ..rect });
    if ctx.appearance.framed_border || style != Card::Plain {
        ui::stroke_rounded(list, rect, border, width);
    }
}

/// The card style for a clickable card.
pub fn clickable(input: &InputState, rect: Rect, active: bool) -> Card {
    if active {
        Card::Active
    } else if hovered(input, rect) {
        Card::Hover
    } else {
        Card::Plain
    }
}

/// A round badge with a glyph centred in it, its top-left at `(x, y)`.
pub fn badge(ctx: &mut PaintCtx<'_>, list: &mut DisplayList, x: f32, y: f32, glyph: &str, fg: Color, bg: Color) {
    let m = ctx.metrics.page.clone();
    let circle = Rect {
        radius: m.badge * 0.5,
        color: bg,
        ..rect(x, y, m.badge, m.badge)
    };
    list.push_rect(circle);
    ui::paint_icon_centered(ctx, list, glyph, circle, m.badge_icon, fg);
}

/// A round button the size of a badge, its top-left at `(x, y)`. `on` tints
/// it with `accent` (a mute that is engaged). Returns whether it was clicked.
#[allow(clippy::too_many_arguments)]
pub fn icon_button(
    ctx: &mut PaintCtx<'_>,
    list: &mut DisplayList,
    x: f32,
    y: f32,
    glyph: &str,
    on: bool,
    accent: Color,
    input: &InputState,
    clicked: bool,
) -> bool {
    let m = ctx.metrics.page.clone();
    let theme = ctx.theme.clone();
    let area = rect(x, y, m.badge, m.badge);
    let hot = hovered(input, area);
    let (fg, bg) = match (on, hot) {
        (true, _) => (accent, mix(theme.surface_high, accent, if hot { 0.3 } else { 0.2 })),
        (false, true) => (theme.text_main, mix(theme.surface_highest, theme.text_main, 0.08)),
        (false, false) => (theme.text_muted, theme.surface_highest),
    };
    badge(ctx, list, x, y, glyph, fg, bg);
    hot && clicked
}

/// How wide [`text_button`] draws `label`.
pub fn text_button_width(ctx: &mut PaintCtx<'_>, label: &str) -> f32 {
    let size = ctx.metrics.typography.caption;
    ctx.measure(label, size) + ctx.metrics.page.card_padding * 2.0
}

/// A pill with a label, badge-high, its top-left at `(x, y)`. Returns
/// whether it was clicked.
pub fn text_button(
    ctx: &mut PaintCtx<'_>,
    list: &mut DisplayList,
    x: f32,
    y: f32,
    label: &str,
    input: &InputState,
    clicked: bool,
) -> bool {
    let m = ctx.metrics.page.clone();
    let typo = ctx.metrics.typography.clone();
    let theme = ctx.theme.clone();
    let area = rect(x, y, text_button_width(ctx, label), m.badge);
    let hot = hovered(input, area);
    let fill = if hot { mix(theme.surface_highest, theme.text_main, 0.08) } else { theme.surface_highest };
    list.push_rect(Rect {
        radius: m.badge * 0.5,
        color: fill,
        ..area
    });
    ui::paint_text_centered_weighted(
        ctx,
        list,
        label,
        area,
        typo.caption,
        typo.weight_bold,
        if hot { theme.text_main } else { theme.text_muted },
    );
    hot && clicked
}

/// A clickable tile: a badge, a title and a caption beside it. `on` tints
/// the tile and fills the badge with the accent (a toggle that is on, the
/// selected option). Returns whether the pointer is over it.
#[allow(clippy::too_many_arguments)]
pub fn tile(
    ctx: &mut PaintCtx<'_>,
    list: &mut DisplayList,
    cell: Rect,
    glyph: &str,
    title: &str,
    caption: &str,
    on: bool,
    input: &InputState,
) -> bool {
    let m = ctx.metrics.page.clone();
    let theme = ctx.theme.clone();
    let hot = hovered(input, cell);
    card(ctx, list, cell, clickable(input, cell, on));

    let pad = m.card_padding;
    let (fg, bg) = if on {
        (theme.on_primary, theme.primary)
    } else {
        (theme.text_muted, theme.surface_highest)
    };
    badge(ctx, list, cell.x + pad, cell.y + (cell.height - m.badge) * 0.5, glyph, fg, bg);

    let text_x = cell.x + pad + m.badge + pad * 0.75;
    let text_w = cell.x + cell.width - pad - text_x;
    two_lines(
        ctx,
        list,
        rect(text_x, cell.y, text_w, cell.height),
        title,
        caption,
        if on { theme.primary } else { theme.text_muted },
    );
    hot
}

/// A bold line over a caption, centred vertically in `area` and cut to its
/// width.
pub fn two_lines(
    ctx: &mut PaintCtx<'_>,
    list: &mut DisplayList,
    area: Rect,
    title: &str,
    caption: &str,
    caption_color: Color,
) {
    let typo = ctx.metrics.typography.clone();
    let line = typo.body * 1.4;
    let caption_h = typo.caption * 1.4;
    let top = area.y + (area.height - line - caption_h) * 0.5;
    let title = ui::elide(ctx, title, typo.body, typo.weight_bold, area.width);
    ui::paint_text_vcentered(
        ctx,
        list,
        &title,
        area.x,
        rect(area.x, top, area.width, line),
        typo.body,
        typo.weight_bold,
        ctx.theme.text_main,
    );
    let caption = ui::elide(ctx, caption, typo.caption, typo.weight_regular, area.width);
    ui::paint_text_vcentered(
        ctx,
        list,
        &caption,
        area.x,
        rect(area.x, top + line, area.width, caption_h),
        typo.caption,
        typo.weight_regular,
        caption_color,
    );
}

/// A card with a small label over a value, for facts that are not controls.
pub fn info_card(ctx: &mut PaintCtx<'_>, list: &mut DisplayList, cell: Rect, label: &str, value: &str) {
    let m = ctx.metrics.page.clone();
    let typo = ctx.metrics.typography.clone();
    card(ctx, list, cell, Card::Plain);
    let pad = m.card_padding;
    let width = cell.width - pad * 2.0;
    let label_h = typo.label * 1.4;
    let value_h = typo.body * 1.5;
    let top = cell.y + (cell.height - label_h - value_h) * 0.5;
    let label = ui::elide(ctx, label, typo.label, typo.weight_regular, width);
    ui::paint_text_vcentered(
        ctx,
        list,
        &label,
        cell.x + pad,
        rect(cell.x, top, cell.width, label_h),
        typo.label,
        typo.weight_regular,
        ctx.theme.text_muted,
    );
    let value = ui::elide(ctx, value, typo.body, typo.weight_bold, width);
    ui::paint_text_vcentered(
        ctx,
        list,
        &value,
        cell.x + pad,
        rect(cell.x, top + label_h, cell.width, value_h),
        typo.body,
        typo.weight_bold,
        ctx.theme.text_main,
    );
}

/// A horizontal meter filled to `fraction`.
pub fn meter(ctx: &PaintCtx<'_>, list: &mut DisplayList, rect: Rect, fraction: f32, color: Color) {
    let track = Rect {
        radius: rect.height * 0.5,
        color: ctx.theme.surface_highest,
        ..rect
    };
    list.push_rect(track);
    let fraction = fraction.clamp(0.0, 1.0);
    if fraction > 0.0 {
        list.push_rect(Rect {
            width: (rect.width * fraction).max(rect.height),
            color,
            ..track
        });
    }
}

/// `a` moved `t` of the way towards `b`.
pub fn mix(a: Color, b: Color, t: f32) -> Color {
    let [ar, ag, ab, aa] = a.to_array();
    let [br, bg, bb, ba] = b.to_array();
    let lerp = |x: f32, y: f32| x + (y - x) * t;
    Color::rgba(lerp(ar, br), lerp(ag, bg), lerp(ab, bb), lerp(aa, ba))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_is_centred_once_capped() {
        let wide = column(rect(100.0, 0.0, 1000.0, 50.0), 760.0);
        assert_eq!((wide.x, wide.width), (220.0, 760.0));
        let narrow = column(rect(100.0, 0.0, 500.0, 50.0), 760.0);
        assert_eq!((narrow.x, narrow.width), (100.0, 500.0));
    }

    #[test]
    fn columns_follow_the_width() {
        // Tiled (about 514 px of page) holds two 200 px cards; floating
        // (about 700) holds three.
        assert_eq!(columns(514.0, 200.0, 12.0, 4), 2);
        assert_eq!(columns(700.0, 200.0, 12.0, 4), 3);
        // Never fewer than one, never more than asked for.
        assert_eq!(columns(50.0, 200.0, 12.0, 4), 1);
        assert_eq!(columns(5000.0, 200.0, 12.0, 4), 4);
        // Exactly two cards and a gap is two columns, not one.
        assert_eq!(columns(412.0, 200.0, 12.0, 4), 2);
    }

    #[test]
    fn cells_fill_rows_left_to_right() {
        let cells = cells(0.0, 10.0, 212.0, 3, 2, 50.0, 12.0);
        assert_eq!(cells.len(), 3);
        assert_eq!((cells[0].x, cells[0].y, cells[0].width), (0.0, 10.0, 100.0));
        assert_eq!((cells[1].x, cells[1].y), (112.0, 10.0));
        assert_eq!((cells[2].x, cells[2].y), (0.0, 72.0));
        assert_eq!(grid_height(3, 2, 50.0, 12.0), 112.0);
        assert_eq!(grid_height(0, 2, 50.0, 12.0), 0.0);
    }

    #[test]
    fn columns_divide_the_cards_evenly() {
        assert_eq!(even_columns(4, 4), 4);
        assert_eq!(even_columns(3, 4), 2, "never three and a straggler");
        assert_eq!(even_columns(2, 3), 1);
        assert_eq!(even_columns(3, 3), 3);
    }
}
