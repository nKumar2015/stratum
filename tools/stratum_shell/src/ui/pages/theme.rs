//! Theme page: every theme as a card, a preview, and the custom theme editor.
//!
//! Themes are a grid of cards, each drawn in its own theme's colours, so the
//! grid is a row of small previews. A larger preview card below it draws a
//! panel in the theme under the pointer (or the selected one), so a palette
//! can be judged before it is applied. Custom themes (`custom_themes.rs`)
//! sit among the configured ones, marked with a badge, and can be edited or
//! deleted; New theme starts a copy of the selected one. While the editor is
//! open the whole shell shows the draft (see `theme_editor.rs`), and the
//! cards only preview.

use crate::actions::SystemAction;
use crate::config::Theme;
use crate::render::display_list::{Color, DisplayList, Rect};
use crate::ui::pages::layout;
use crate::ui::pages::theme_editor::{suggest_name, Editor, EditorEvent};
use crate::ui::pages::Optimistic;
use crate::ui::{self, InputState, PaintCtx};

/// The custom theme waiting for a second click to be deleted.
#[derive(Debug, Default)]
struct ArmedDelete {
    name: Option<String>,
    remaining: f32,
}

pub struct ThemePage {
    theme: Optimistic<String>,
    editor: Option<Editor>,
    armed: ArmedDelete,
}

/// The colours a card's strip shows, in order.
fn swatches(theme: &Theme) -> [Color; 5] {
    [theme.bg_main, theme.bg_widget, theme.primary, theme.secondary, theme.tertiary]
}

impl ThemePage {
    pub fn new() -> Self {
        Self {
            theme: Optimistic::new(),
            editor: None,
            armed: ArmedDelete::default(),
        }
    }

    pub fn tick(&mut self, dt: f32) {
        self.theme.tick(dt);
        if self.armed.name.is_some() {
            self.armed.remaining -= dt;
            if self.armed.remaining <= 0.0 {
                self.armed.name = None;
            }
        }
    }

    /// Frames are needed while a delete is armed, so it disarms on time.
    pub fn animating(&self) -> bool {
        self.armed.name.is_some()
    }

    /// Leaving the page ends an edit; the shell drops its live preview.
    pub fn on_hidden(&mut self) {
        self.editor = None;
        self.armed.name = None;
    }

    pub fn wants_keys(&self) -> bool {
        self.editor.as_ref().is_some_and(Editor::wants_keys)
    }

    pub fn key(&mut self, keysym: smithay_client_toolkit::seat::keyboard::Keysym, text: Option<&str>, ctrl: bool) {
        if let Some(editor) = &mut self.editor {
            editor.key(keysym, text, ctrl);
        }
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
        let selected_theme = self.theme.view(current.clone());
        let custom_count = ctx.custom_themes.len();

        let area = layout::column(area, m.max_width);
        let summary = match &self.editor {
            Some(editor) => format!("Editing {} \u{b7} changes show live", editor.preview_name()),
            None if custom_count == 0 => format!("Theme {selected_theme}"),
            None => format!("Theme {selected_theme} \u{b7} {custom_count} custom"),
        };
        let (head, body) = layout::header(ctx, list, area, "Theme", &summary);
        if self.editor.is_none() {
            self.paint_actions(ctx, list, head, &selected_theme, input, clicked, pending);
        }

        // The draft is in the theme list under its preview name; a new one
        // has no card until it is saved. An edited saved one keeps its card,
        // which shows the draft.
        let draft = self
            .editor
            .as_ref()
            .filter(|e| e.original() != Some(e.preview_name()))
            .map(|e| e.preview_name().to_string());
        let mut y = layout::section(ctx, list, body, body.y, "Themes");
        let themes: Vec<(String, Theme)> = ctx
            .themes
            .iter()
            .filter(|(name, _)| Some(*name) != draft.as_ref())
            .map(|(name, config)| (name.clone(), config.resolved()))
            .collect();
        let cols = layout::columns(body.width, m.theme_min_width, m.grid_gap, 4);
        let cells = layout::cells(body.x, y, body.width, themes.len(), cols, m.theme_height, m.grid_gap);
        let mut previewed: Option<(String, Theme)> = None;
        let mut selected_preview: Option<(String, Theme)> = None;
        for ((name, theme), cell) in themes.iter().zip(&cells) {
            let selected = *name == selected_theme;
            let hot = layout::hovered(input, *cell);
            let custom = ctx.custom_themes.contains(name);
            paint_theme_card(ctx, list, *cell, name, theme, selected, hot, custom);
            // While editing, applying another theme would end the edit.
            if hot && clicked && !selected && self.editor.is_none() {
                self.theme.set(name.clone());
                self.armed.name = None;
                pending.push(SystemAction::SetTheme(name.clone()));
            }
            if hot {
                previewed = Some((name.clone(), theme.clone()));
            }
            if selected {
                selected_preview = Some((name.clone(), theme.clone()));
            }
        }
        y += layout::grid_height(themes.len(), cols, m.theme_height, m.grid_gap);

        // A new draft has no card, but it is what the shell shows.
        let shown = previewed.or(selected_preview).or_else(|| Some((current.clone(), ctx.theme.clone())));
        if let Some((name, theme)) = shown {
            y += m.section_spacing;
            y = layout::section(ctx, list, body, y, "Preview");
            let card = Rect {
                radius: m.card_radius,
                ..layout::rect(body.x, y, body.width, m.preview_height)
            };
            paint_preview(ctx, list, card, &name, &theme);
            y += card.height;
        }

        if let Some(editor) = &mut self.editor {
            y += m.section_spacing;
            let label = if editor.original().is_some() { "Edit theme" } else { "New theme" };
            y = layout::section(ctx, list, body, y, label);
            let (bottom, event) = editor.paint(ctx, list, body, y, input, clicked, pending);
            y = bottom;
            match event {
                EditorEvent::None => {}
                EditorEvent::Saved | EditorEvent::Discarded => self.editor = None,
            }
        }
        y
    }

    /// New theme, and Edit and Delete for a custom theme, at the header's
    /// right.
    #[allow(clippy::too_many_arguments)]
    fn paint_actions(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        head: Rect,
        selected: &str,
        input: &InputState,
        clicked: bool,
        pending: &mut Vec<SystemAction>,
    ) {
        let m = ctx.metrics.page.clone();
        let custom = ctx.custom_themes.contains(selected);
        let armed = self.armed.name.as_deref() == Some(selected);
        let delete = if armed { "Click again to delete" } else { "Delete" };
        let labels: &[&str] = if custom { &[delete, "Edit", "New theme"] } else { &["New theme"] };
        let y = head.y + (head.height - m.badge) * 0.5;
        let (_, hit) = layout::text_buttons(ctx, list, head.x + head.width, y, labels, true, input, clicked);
        let Some(hit) = hit else { return };
        let Some(config) = ctx.themes.get(selected).cloned() else { return };
        match labels[hit] {
            "New theme" => {
                let name = suggest_name(selected, |n| ctx.themes.contains_key(n));
                self.editor = Some(Editor::new(&name, config, None));
                self.armed.name = None;
            }
            "Edit" => {
                self.editor = Some(Editor::new(selected, config, Some(selected.to_string())));
                self.armed.name = None;
            }
            _ if armed => {
                self.armed.name = None;
                pending.push(SystemAction::DeleteTheme(selected.to_string()));
            }
            _ => {
                self.armed.name = Some(selected.to_string());
                self.armed.remaining = m.confirm_ms as f32 / 1000.0;
            }
        }
    }
}

/// A theme card, drawn in `theme` itself: its background, its name in its
/// text colour, a widget strip with a level in its primary, and a row of its
/// swatches. The outline is the current theme's, so the selection reads the
/// same on every card. A custom theme carries a pencil by its swatches.
#[allow(clippy::too_many_arguments)]
fn paint_theme_card(
    ctx: &mut PaintCtx<'_>,
    list: &mut DisplayList,
    cell: Rect,
    name: &str,
    theme: &Theme,
    selected: bool,
    hot: bool,
    custom: bool,
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
    let line = typo.line(typo.body);
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
        ui::paint_frame(list, swatch, ctx.appearance.border_width, theme.border_inactive, color);
        x += m.swatch + m.swatch_gap;
    }
    if custom {
        let mark = layout::rect(cell.x + cell.width - pad - typo.caption, swatch_y, typo.caption, m.swatch);
        ui::paint_icon_centered(ctx, list, "\u{f03eb}", mark, typo.caption, theme.text_muted);
    }
}

/// A miniature panel drawn entirely in `theme`: its background and border, a
/// widget card with text and a level bar, the three accents as buttons, and
/// the status colours.
fn paint_preview(ctx: &mut PaintCtx<'_>, list: &mut DisplayList, card: Rect, name: &str, theme: &Theme) {
    let page = ctx.metrics.page.clone();
    let type_ = ctx.metrics.typography.clone();
    crate::ui::paint_frame(list, card, ctx.appearance.border_width, theme.border_active, theme.bg_main);
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

impl Default for ThemePage {
    fn default() -> Self {
        Self::new()
    }
}
