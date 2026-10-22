//! The app title.
//!
//! Horizontal bars show one elided line; vertical rails rotate the run so it
//! reads upward, matching the QML `rotation: 270` label. Each monitor shows
//! the window on its own active workspace, with the app's icon before the
//! title; with no window it shows the desktop label and no icon, as the QML
//! title did.

use std::sync::Arc;

use crate::render::display_list::{DisplayList, Rect};
use crate::text::TextStyle;
use crate::ui::{self, InputState, PaintCtx, Widget};

pub struct AppTitle {
    /// Shown when no window is focused.
    pub fallback: String,
}

impl AppTitle {
    pub fn new() -> Self {
        Self {
            fallback: "\u{f4a9}  Desktop".to_string(),
        }
    }
}

impl Default for AppTitle {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for AppTitle {
    /// Flexible: consumes leftover vertical space so later widgets sit at the
    /// bottom of the rail, as `Layout.fillHeight` did in QML.
    fn height(&self, _width: f32, _metrics: &crate::ui::style::Metrics) -> Option<f32> {
        None
    }

    /// In a horizontal bar the title absorbs slack and reports no width.
    fn width(&self, _height: f32, _metrics: &crate::ui::style::Metrics) -> Option<f32> {
        None
    }

    fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        rect: Rect,
        _input: &InputState,
    ) {
        let window = ctx.state.hypr.get().windows.get(ctx.output_name).cloned();
        let live_title = window
            .as_ref()
            .map(|w| w.title.clone())
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| self.fallback.clone());
        let bar = &ctx.metrics.bar;
        let (icon_size, icon_gap) = (bar.title_icon, bar.title_icon_gap);
        // Twice the logical size, as the tray does: sharp on scaled outputs.
        let icon = window
            .as_ref()
            .and_then(|w| ctx.icons.load_app_icon(&w.class, icon_size as u32, 2));
        let icon_space = if icon.is_some() { icon_size + icon_gap } else { 0.0 };
        let push_icon = |list: &mut DisplayList, x: f32, y: f32| {
            if let Some(icon) = &icon {
                list.push_image(crate::render::display_list::Image {
                    width: icon.width,
                    height: icon.height,
                    pixels: icon.pixels.clone(),
                    x,
                    y,
                    width_logical: icon_size,
                    height_logical: icon_size,
                });
            }
        };

        if !ctx.is_vertical() {
            let style = TextStyle {
                family: Arc::from(ctx.theme.font.as_str()),
                size: ctx.metrics.typography.title_bar,
                weight: ctx.metrics.typography.weight_medium,
                color: ctx.theme.text_main,
            };
            let insets = 6.0;
            let available = rect.width - insets * 2.0 - icon_space;
            if available <= 0.0 {
                return;
            }
            let text = elide(&live_title, &style, ctx, available);
            if text.is_empty() {
                return;
            }
            push_icon(list, rect.x + insets, rect.y + (rect.height - icon_size) * 0.5);
            ui::paint_text_vcentered(
                ctx,
                list,
                &text,
                rect.x + insets + icon_space,
                rect,
                style.size,
                style.weight,
                style.color,
            );
            return;
        }

        // Vertical label. The run advances upward, so its cross-axis extent is
        // the line height and must fit the rail.
        let mut size = ctx.metrics.typography.title;
        let max_thickness = (rect.width - 12.0).max(6.0);
        while size > 7.0 && size * 1.25 > max_thickness {
            size -= 0.5;
        }

        let style = TextStyle {
            family: Arc::from(ctx.theme.font.as_str()),
            size,
            weight: ctx.metrics.typography.weight_bold,
            color: ctx.theme.text_main,
        };

        let available = rect.height - 16.0 - icon_space;
        let title = elide(&live_title, &style, ctx, available);
        if title.is_empty() {
            return;
        }

        let mut run = ctx.text.shape(&title, &style, 0.0, 0.0);
        let line_height = size * 1.2;
        let length = ctx.text.measure(&title, &style).0;

        // Icon and text are centred together. The run reads upward, so the
        // icon, which comes first, sits below it. Rotated -90 degrees the run
        // spans `[baseline - length, baseline]`. Across the rail, bias by
        // half the line height.
        let bottom = rect.y + rect.height * 0.5 + (length + icon_space) * 0.5;
        push_icon(list, rect.x + (rect.width - icon_size) * 0.5, bottom - icon_size);
        let baseline_y = bottom - icon_space;
        run.transform = ui::vertical_text_transform(
            rect.x + (rect.width - line_height) * 0.5 + line_height,
            baseline_y,
        );
        list.push_text(run);
    }
}

/// Shorten `text` with an ellipsis until it fits `available` pixels,
/// mirroring QML's `elide: Text.ElideRight`.
fn elide(text: &str, style: &TextStyle, ctx: &mut PaintCtx<'_>, available: f32) -> String {
    if available <= 0.0 {
        return String::new();
    }
    if ctx.text.measure(text, style).0 <= available {
        return text.to_string();
    }
    let ellipsis = "\u{2026}";
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let candidate: String = chars.iter().collect::<String>() + ellipsis;
        if ctx.text.measure(&candidate, style).0 <= available {
            return candidate;
        }
    }
    String::new()
}
