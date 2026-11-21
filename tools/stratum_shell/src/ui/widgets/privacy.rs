//! The privacy indicator: a dot per thing in use (microphone, camera, a
//! shared screen), like a phone's, and nothing at all otherwise. Resting
//! on it or clicking it opens the menu naming the apps
//! (`HoverKind::Privacy`).

use std::collections::HashMap;

use crate::privacy::Indicators;
use crate::render::display_list::{DisplayList, Point, Rect};
use crate::ui::hover_menu::{HoverKind, OpenMenu};
use crate::ui::{self, Action, ClickCtx, InputState, PaintCtx, Widget};

/// nf-md-microphone, nf-md-webcam, nf-md-monitor_share.
pub const MICROPHONE: &str = "\u{f036c}";
pub const CAMERA: &str = "\u{f05a0}";
pub const SCREEN: &str = "\u{f1483}";

/// The glyphs to show and their colours, in order.
pub fn glyphs(ctx: &PaintCtx<'_>, shown: &Indicators) -> Vec<(&'static str, crate::render::display_list::Color)> {
    let mut out = Vec::new();
    if !shown.microphone.is_empty() {
        out.push((MICROPHONE, ctx.theme.warning));
    }
    if !shown.camera.is_empty() {
        out.push((CAMERA, ctx.theme.success));
    }
    if shown.screen {
        out.push((SCREEN, ctx.theme.error));
    }
    out
}

#[derive(Default)]
pub struct Privacy {
    /// Rect from the last paint on each output; the tree is shared.
    rects: HashMap<String, Rect>,
    /// How many glyphs show, read in `sync`.
    count: usize,
}

impl Privacy {
    fn length(&self, metrics: &crate::ui::style::Metrics) -> f32 {
        let m = &metrics.privacy;
        if self.count == 0 {
            0.0
        } else {
            self.count as f32 * m.icon + (self.count - 1) as f32 * m.gap + m.padding * 2.0
        }
    }

    fn hit(&self, output: &str, point: Point) -> Option<Rect> {
        self.rects.get(output).copied().filter(|r| r.contains(point))
    }
}

impl Widget for Privacy {
    fn sync(&mut self, ctx: &PaintCtx<'_>) {
        self.count = glyphs(ctx, &ctx.state.privacy()).len();
    }

    fn height(&self, _width: f32, metrics: &crate::ui::style::Metrics) -> Option<f32> {
        Some(self.length(metrics))
    }

    fn width(&self, _height: f32, metrics: &crate::ui::style::Metrics) -> Option<f32> {
        Some(self.length(metrics))
    }

    fn paint(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, rect: Rect, _input: &InputState) {
        let shown = ctx.state.privacy();
        let glyphs = glyphs(ctx, &shown);
        if glyphs.is_empty() {
            self.rects.remove(ctx.output_name);
            return;
        }
        self.rects.insert(ctx.output_name.to_string(), rect);
        let m = ctx.metrics.privacy.clone();
        // A pill behind the glyphs, like a phone's.
        let vertical = ctx.is_vertical();
        let thickness = m.icon + m.padding * 2.0;
        let pill = if vertical {
            Rect { x: rect.x + (rect.width - thickness) * 0.5, width: thickness, radius: thickness * 0.5, color: ctx.theme.bg_widget, ..rect }
        } else {
            Rect { y: rect.y + (rect.height - thickness) * 0.5, height: thickness, radius: thickness * 0.5, color: ctx.theme.bg_widget, ..rect }
        };
        list.push_rect(pill);
        let mut at = if vertical { pill.y + m.padding } else { pill.x + m.padding };
        for (glyph, color) in glyphs {
            let cell = if vertical {
                Rect { y: at, height: m.icon, ..pill }
            } else {
                Rect { x: at, width: m.icon, ..pill }
            };
            ui::paint_icon_centered(ctx, list, glyph, cell, m.icon, color);
            at += m.icon + m.gap;
        }
    }

    fn on_click(&mut self, ctx: &ClickCtx<'_>) -> Option<Action> {
        let rect = self.hit(ctx.output_name, ctx.point)?;
        Some(Action::ShowHoverMenu(OpenMenu {
            kind: HoverKind::Privacy,
            output: ctx.output_name.to_string(),
            anchor_x: rect.x + rect.width * 0.5,
            anchor_y: rect.y + rect.height * 0.5,
        }))
    }

    fn hover_target(&self, output: &str, point: Point) -> Option<(HoverKind, Rect)> {
        self.hit(output, point).map(|rect| (HoverKind::Privacy, rect))
    }
}
