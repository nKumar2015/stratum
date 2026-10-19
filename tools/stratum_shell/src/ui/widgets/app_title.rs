//! The app title.
//!
//! Horizontal bars show one elided line; vertical rails rotate the run so it
//! reads upward, matching the QML `rotation: 270` label.

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
            fallback: "Desktop".to_string(),
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
    fn height(&self, _width: f32) -> Option<f32> {
        None
    }

    /// In a horizontal bar the title absorbs slack and reports no width.
    fn width(&self, _height: f32) -> Option<f32> {
        None
    }

    fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        rect: Rect,
        _input: &InputState,
    ) {
        let live_title = ctx
            .state
            .hypr
            .get()
            .title
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| self.fallback.clone());

        if !ctx.is_vertical() {
            let style = TextStyle {
                family: Arc::from(ctx.theme.font.as_str()),
                size: 13.0,
                weight: 500,
                color: ctx.theme.text_main,
            };
            let insets = 6.0;
            let available = rect.width - insets * 2.0;
            if available <= 0.0 {
                return;
            }
            let text = elide(&live_title, &style, ctx, available);
            if text.is_empty() {
                return;
            }
            ui::paint_text_vcentered(
                ctx,
                list,
                &text,
                rect.x + insets,
                rect,
                style.size,
                style.weight,
                style.color,
            );
            return;
        }

        // Vertical label. The run advances upward, so its cross-axis extent is
        // the line height and must fit the rail.
        let mut size = 14.0_f32;
        let max_thickness = (rect.width - 12.0).max(6.0);
        while size > 7.0 && size * 1.25 > max_thickness {
            size -= 0.5;
        }

        let style = TextStyle {
            family: Arc::from(ctx.theme.font.as_str()),
            size,
            weight: 600,
            color: ctx.theme.text_main,
        };

        let available = rect.height - 16.0;
        let title = elide(&live_title, &style, ctx, available);
        if title.is_empty() {
            return;
        }

        let mut run = ctx.text.shape(&title, &style, 0.0, 0.0);
        let line_height = size * 1.2;
        let length = ctx.text.measure(&title, &style).0;

        // The run is centred in the widget. Rotated -90 degrees it spans
        // `[baseline - length, baseline]`, so centring puts the baseline at
        // centre + length/2. Across the rail, bias by half the line height.
        let baseline_y = rect.y + rect.height * 0.5 + length * 0.5;
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
