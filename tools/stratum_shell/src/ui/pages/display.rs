//! Display page: font size, output, and theme selection.
//!
//! Each theme row carries a strip of its colours, and a preview card below
//! the list draws a small panel in the theme under the pointer (or the
//! selected one), so a palette can be judged before it is applied.

use crate::actions::SystemAction;
use crate::config::Theme;
use crate::render::display_list::{Color, DisplayList, Point, Rect};
use crate::ui::controls;
use crate::ui::pages::{heading, section_label, stat_row, Optimistic};
use crate::ui::{InputState, PaintCtx};

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

    pub fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        area: Rect,
        input: &InputState,
        clicked: bool,
        pending: &mut Vec<SystemAction>,
    ) {
        let area = heading(ctx, list, area, "Display");
        let current = ctx.theme.name.clone();
        let selected_theme = self.theme.view(current);

        stat_row(
            ctx,
            list,
            area,
            0,
            "Font size",
            &ctx.appearance.font_size.to_string(),
        );
        stat_row(ctx, list, area, 1, "Output", ctx.output_name);

        let mut y = area.y + 84.0;
        section_label(ctx, list, area, y, "Theme");
        y += 24.0;

        // Themes come from the shell configuration, so the list grows as the
        // user adds palettes without any code change.
        let page = ctx.metrics.page.clone();
        let pointer = input.pointer.map(|(x, y)| Point::new(x, y));
        let width = area.width.min(320.0);
        let mut previewed: Option<(String, Theme)> = None;
        let themes: Vec<(String, Theme)> = ctx
            .themes
            .iter()
            .map(|(name, config)| (name.clone(), config.to_theme()))
            .collect();
        for (name, theme) in themes {
            let row = Rect {
                x: area.x,
                y,
                width,
                height: page.row_height,
                radius: page.button_radius,
                color: Color::TRANSPARENT,
            };
            let selected = name == selected_theme;
            if controls::selectable_row(ctx, list, row, &name, "", selected, input, clicked)
                && !selected
            {
                self.theme.set(name.clone());
                pending.push(SystemAction::SetTheme(name.clone()));
            }
            self.paint_strip(ctx, list, row, &theme);
            let hovered = pointer.is_some_and(|p| row.contains(p));
            if hovered || (selected && previewed.is_none()) {
                previewed = Some((name, theme));
            }
            y += page.stat_spacing;
        }

        if let Some((name, theme)) = previewed {
            y += page.section_gap - page.stat_spacing + page.row_height;
            section_label(ctx, list, area, y, "Preview");
            y += page.section_label_height + page.section_gap * 0.25;
            let card = Rect {
                x: area.x,
                y,
                width,
                height: page.preview_height,
                radius: page.card_radius,
                color: Color::TRANSPARENT,
            };
            paint_preview(ctx, list, card, &name, &theme);
        }
    }

    /// The theme's colours at the right end of its row.
    fn paint_strip(&self, ctx: &PaintCtx<'_>, list: &mut DisplayList, row: Rect, theme: &Theme) {
        let page = &ctx.metrics.page;
        let colors = swatches(theme);
        let step = page.swatch + page.swatch_gap;
        let mut x = row.x + row.width - page.card_padding - step * colors.len() as f32 + page.swatch_gap;
        let y = row.y + (row.height - page.swatch) * 0.5;
        for color in colors {
            let swatch = Rect { x, y, width: page.swatch, height: page.swatch, radius: page.swatch * 0.3, color };
            // Outlined, or a background swatch vanishes into a dark row.
            crate::ui::paint_frame(list, swatch, color, ctx.theme.border_inactive, true);
            x += step;
        }
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
