//! The drive icon: there only while a removable drive is attached, so it
//! takes no room otherwise. Resting on it, or clicking it, opens the drive
//! menu (`HoverKind::Drives`); there is no settings page behind it.

use std::collections::HashMap;

use crate::render::display_list::{DisplayList, Point, Rect};
use crate::ui::hover_menu::{HoverKind, OpenMenu};
use crate::ui::{self, Action, ClickCtx, InputState, PaintCtx, Widget};

/// The glyph for a drive: a disc for optical drives, else a USB stick.
pub fn glyph(drive: &stratum_drives::Drive) -> &'static str {
    if drive.optical {
        "\u{f05ee}"
    } else if drive.bus == "sdio" || drive.name.to_lowercase().contains("card") {
        "\u{f0479}"
    } else {
        "\u{f129e}"
    }
}

#[derive(Default)]
pub struct Drives {
    /// Rect from the last paint on each output; the tree is shared.
    rects: HashMap<String, Rect>,
    /// Whether a removable drive is attached, read in `sync`.
    shown: bool,
}

impl Drives {
    fn hit(&self, output: &str, point: Point) -> Option<Rect> {
        self.rects.get(output).copied().filter(|r| r.contains(point))
    }
}

impl Widget for Drives {
    fn sync(&mut self, ctx: &PaintCtx<'_>) {
        self.shown = ctx.state.drives.get().iter().any(|d| d.removable);
    }

    fn height(&self, _width: f32, metrics: &crate::ui::style::Metrics) -> Option<f32> {
        Some(if self.shown { metrics.typography.icon } else { 0.0 })
    }

    fn width(&self, height: f32, metrics: &crate::ui::style::Metrics) -> Option<f32> {
        Some(if self.shown { height.max(metrics.typography.icon) } else { 0.0 })
    }

    fn paint(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, rect: Rect, _input: &InputState) {
        if !self.shown {
            self.rects.remove(ctx.output_name);
            return;
        }
        self.rects.insert(ctx.output_name.to_string(), rect);
        let drives = ctx.state.drives.get();
        let removable: Vec<_> = drives.iter().filter(|d| d.removable).collect();
        let glyph = removable.first().map_or("\u{f129e}", |d| glyph(d));
        let color = if removable.iter().any(|d| d.mounted()) { ctx.theme.primary } else { ctx.theme.text_muted };
        ui::paint_icon_centered(ctx, list, glyph, rect, ctx.metrics.typography.icon, color);
    }

    fn on_click(&mut self, ctx: &ClickCtx<'_>) -> Option<Action> {
        let rect = self.hit(ctx.output_name, ctx.point)?;
        Some(Action::ShowHoverMenu(OpenMenu {
            kind: HoverKind::Drives,
            output: ctx.output_name.to_string(),
            anchor_x: rect.x + rect.width * 0.5,
            anchor_y: rect.y + rect.height * 0.5,
        }))
    }

    fn hover_target(&self, output: &str, point: Point) -> Option<(HoverKind, Rect)> {
        self.hit(output, point).map(|rect| (HoverKind::Drives, rect))
    }
}
