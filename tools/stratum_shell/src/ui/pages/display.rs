//! Display page: theme selection, then the font and output.
//!
//! Themes are a grid of cards, each drawn in its own theme's colours, so the
//! grid is a row of small previews. A larger preview card below it draws a
//! panel in the theme under the pointer (or the selected one), so a palette
//! can be judged before it is applied.

use crate::actions::SystemAction;
use crate::config::Theme;
use crate::render::display_list::{Color, DisplayList, Rect};
use crate::ui::pages::layout;
use crate::ui::pages::Optimistic;
use crate::ui::{self, InputState, PaintCtx};

pub struct DisplayPage {
    theme: Optimistic<String>,
}

/// The colours a row's strip shows, in order.
fn swatches(theme: &Theme) -> [Color; 5] {
    [theme.bg_main, theme.bg_widget, theme.primary, theme.secondary, theme.tertiary]
}

impl DisplayPage {
    pub fn new() -> Self {
        Self {
            theme: Optimistic::new(),
        }
    }

    pub fn tick(&mut self, dt: f32) {
        self.theme.tick(dt);
    }

    /// Returns the bottom of the page.
    pub fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        area: Rect,
        input: &InputState,
        clicked: bool,
        pending: &mut Vec<SystemAction>,
    ) -> f32 {
        let m = ctx.metrics.page.clone();
        let current = ctx.theme.name.clone();
        let selected_theme = self.theme.view(current);

        let area = layout::column(area, m.max_width);
        let summary = format!("Theme {selected_theme} \u{b7} {}", ctx.output_name);
        let (_, body) = layout::header(ctx, list, area, "Display", &summary);

        // Themes come from the shell configuration, so the grid grows as the
        // user adds palettes without any code change.
        let mut y = layout::section(ctx, list, body, body.y, "Theme");
        let themes: Vec<(String, Theme)> = ctx
            .themes
            .iter()
            .map(|(name, config)| (name.clone(), config.resolved()))
            .collect();
        let cols = layout::columns(body.width, m.theme_min_width, m.grid_gap, 4);
        let cells = layout::cells(body.x, y, body.width, themes.len(), cols, m.theme_height, m.grid_gap);
        let mut previewed: Option<usize> = None;
        for (index, ((name, theme), cell)) in themes.iter().zip(&cells).enumerate() {
            let selected = *name == selected_theme;
            let hot = layout::hovered(input, *cell);
            paint_theme_card(ctx, list, *cell, name, theme, selected, hot);
            if hot && clicked && !selected {
                self.theme.set(name.clone());
                pending.push(SystemAction::SetTheme(name.clone()));
            }
            if hot || (selected && previewed.is_none()) {
                previewed = Some(index);
            }
        }
        y += layout::grid_height(themes.len(), cols, m.theme_height, m.grid_gap);

        if let Some((name, theme)) = previewed.and_then(|i| themes.get(i)) {
            y += m.section_spacing;
            y = layout::section(ctx, list, body, y, "Preview");
            let card = Rect {
                radius: m.card_radius,
                ..layout::rect(body.x, y, body.width, m.preview_height)
            };
            paint_preview(ctx, list, card, name, theme);
            y += card.height;
        }

        // The text and output facts.
        y += m.section_spacing;
        y = layout::section(ctx, list, body, y, "Text and output");
        let facts = [
            ("Font", ctx.theme.font.clone()),
            ("Font size", ctx.appearance.font_size.to_string()),
            ("Output", ctx.output_name.to_string()),
        ];
        let cols = layout::columns(body.width, m.card_min_width, m.grid_gap, facts.len());
        let cols = layout::even_columns(cols, facts.len());
        let cells = layout::cells(body.x, y, body.width, facts.len(), cols, m.info_height, m.grid_gap);
        for ((label, value), cell) in facts.iter().zip(&cells) {
            layout::info_card(ctx, list, *cell, label, value);
        }
        y + layout::grid_height(facts.len(), cols, m.info_height, m.grid_gap)
    }
}

/// A theme card, drawn in `theme` itself: its background, its name in its
/// text colour, a widget strip with a level in its primary, and a row of its
/// swatches. The outline is the current theme's, so the selection reads the
/// same on every card.
fn paint_theme_card(
    ctx: &mut PaintCtx<'_>,
    list: &mut DisplayList,
    cell: Rect,
    name: &str,
    theme: &Theme,
    selected: bool,
    hot: bool,
) {
    let m = ctx.metrics.page.clone();
    let typo = ctx.metrics.typography.clone();
    let card = Rect { radius: m.card_radius, ..cell };
    list.push_rect(Rect { color: theme.bg_main, ..card });
    let (border, width) = if selected {
        (ctx.theme.primary, m.outline_active)
    } else if hot {
        (ctx.theme.text_muted, m.outline)
    } else {
        (ctx.theme.outline, m.outline)
    };
    ui::stroke_rounded(list, card, border, width);

    let pad = m.card_padding;
    let inner_w = cell.width - pad * 2.0;
    let line = typo.body * 1.4;
    let check_w = if selected { typo.body + pad * 0.5 } else { 0.0 };
    let label = ui::elide(ctx, name, typo.body, typo.weight_bold, inner_w - check_w);
    let label_row = layout::rect(cell.x, cell.y + pad, cell.width, line);
    ui::paint_text_vcentered(ctx, list, &label, cell.x + pad, label_row, typo.body, typo.weight_bold, theme.text_main);
    if selected {
        ui::paint_icon_centered(
            ctx,
            list,
            "\u{f00c}",
            layout::rect(cell.x + cell.width - pad - typo.body, label_row.y, typo.body, line),
            typo.caption,
            ctx.theme.primary,
        );
    }

    // A widget strip holding a level bar, between the name and the swatches.
    let strip_y = label_row.y + line + pad * 0.5;
    let swatch_y = cell.y + cell.height - pad - m.swatch;
    let strip = Rect {
        radius: m.button_radius * 0.5,
        color: theme.bg_widget,
        ..layout::rect(cell.x + pad, strip_y, inner_w, (swatch_y - pad * 0.5 - strip_y).max(0.0))
    };
    list.push_rect(strip);
    let track = Rect {
        radius: m.slider_track * 0.5,
        color: theme.bg_hover,
        ..layout::rect(
            strip.x + pad * 0.5,
            strip.y + (strip.height - m.slider_track) * 0.5,
            strip.width - pad,
            m.slider_track,
        )
    };
    list.push_rect(track);
    list.push_rect(Rect { width: track.width * 0.6, color: theme.primary, ..track });

    let mut x = cell.x + pad;
    for color in swatches(theme) {
        let swatch = Rect {
            radius: m.swatch * 0.5,
            color,
            ..layout::rect(x, swatch_y, m.swatch, m.swatch)
        };
        // Outlined, or a background swatch vanishes into its own card.
        ui::paint_frame(list, swatch, color, theme.border_inactive, true);
        x += m.swatch + m.swatch_gap;
    }
}

/// A miniature panel drawn entirely in `theme`: its background and border, a
/// widget card with text and a level bar, the three accents as buttons, and
/// the status colours.
fn paint_preview(ctx: &mut PaintCtx<'_>, list: &mut DisplayList, card: Rect, name: &str, theme: &Theme) {
    let page = ctx.metrics.page.clone();
    let type_ = ctx.metrics.typography.clone();
    crate::ui::paint_frame(list, card, theme.bg_main, theme.border_active, true);
    let inner = Rect {
        x: card.x + page.card_padding,
        y: card.y + page.card_padding,
        width: card.width - page.card_padding * 2.0,
        height: card.height - page.card_padding * 2.0,
        radius: 0.0,
        color: Color::TRANSPARENT,
    };

    // Widget: title, caption, and a level bar.
    let widget = Rect {
        height: page.stat_height + page.slider_track * 3.0,
        radius: page.button_radius,
        color: theme.bg_widget,
        ..inner
    };
    list.push_rect(widget);
    let text_row = Rect { x: widget.x + page.card_padding, height: page.stat_height, ..widget };
    crate::ui::paint_text_vcentered(ctx, list, name, text_row.x, text_row, type_.body, type_.weight_bold, theme.text_main);
    let caption = "Secondary text";
    let caption_w = ctx.measure(caption, type_.caption);
    crate::ui::paint_text_vcentered(
        ctx,
        list,
        caption,
        widget.x + widget.width - page.card_padding - caption_w,
        text_row,
        type_.caption,
        type_.weight_regular,
        theme.text_muted,
    );
    let track = Rect {
        x: text_row.x,
        y: widget.y + page.stat_height,
        width: widget.width - page.card_padding * 2.0,
        height: page.slider_track,
        radius: page.slider_track * 0.5,
        color: theme.bg_hover,
    };
    list.push_rect(track);
    list.push_rect(Rect { width: track.width * 0.6, color: theme.primary, ..track });

    // Accents as filled buttons, then the status colours as dots.
    let buttons_y = widget.y + widget.height + page.section_gap * 0.5;
    let dot = page.swatch;
    let status = [theme.success, theme.warning, theme.error];
    let dots_width = status.len() as f32 * (dot + page.swatch_gap);
    let button_w = ((inner.width - dots_width - page.swatch_gap * 3.0) / 3.0).max(0.0);
    let mut x = inner.x;
    for (label, fill) in [("Primary", theme.primary), ("Secondary", theme.secondary), ("Tertiary", theme.tertiary)] {
        let button = Rect {
            x,
            y: buttons_y,
            width: button_w,
            height: page.button_height,
            radius: page.button_radius,
            color: fill,
        };
        list.push_rect(button);
        crate::ui::paint_text_centered(ctx, list, label, button, type_.caption, theme.bg_main);
        x += button_w + page.swatch_gap;
    }
    x = inner.x + inner.width - dots_width + page.swatch_gap;
    for color in status {
        list.push_rect(Rect {
            x,
            y: buttons_y + (page.button_height - dot) * 0.5,
            width: dot,
            height: dot,
            radius: dot * 0.5,
            color,
        });
        x += dot + page.swatch_gap;
    }
}

impl Default for DisplayPage {
    fn default() -> Self {
        Self::new()
    }
}
