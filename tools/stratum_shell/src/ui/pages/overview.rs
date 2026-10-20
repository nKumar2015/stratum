//! The overview page: one tile per subsystem.

use crate::render::display_list::{Color, DisplayList, Rect};
use crate::ui::{self, PaintCtx};

pub struct OverviewPage;

impl OverviewPage {
    pub fn new() -> Self {
        Self
    }

    pub fn paint(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, area: Rect) {
        let tiles = [
            ("Audio", ctx.state.audio.get()),
            ("Bluetooth", ctx.state.bluetooth.get()),
            ("Network", ctx.state.net.get()),
            ("Battery", ctx.state.battery.get()),
        ];
        let columns = 2;
        let gap = 12.0;
        let tile_w = (area.width - gap) / columns as f32;
        let tile_h = 76.0;

        for (index, (label, snapshot)) in tiles.iter().enumerate() {
            let col = index % columns;
            let row = index / columns;
            let rect = Rect {
                x: area.x + col as f32 * (tile_w + gap),
                y: area.y + row as f32 * (tile_h + gap),
                width: tile_w,
                height: tile_h,
                radius: ctx.metrics.page.card_radius,
                color: Color::TRANSPARENT,
            };
            ui::paint_frame(
                list,
                rect,
                ctx.theme.bg_widget,
                ctx.theme.border_inactive,
                ctx.appearance.framed_border,
            );
            ui::paint_text_vcentered(
                ctx,
                list,
                label,
                rect.x + 14.0,
                Rect { height: 28.0, ..rect },
                ctx.metrics.typography.body,
ctx.metrics.typography.weight_bold,
                ctx.theme.text_muted,
            );
            let summary = summarize(label, snapshot.clone());
            ui::paint_text_vcentered(
                ctx,
                list,
                &summary,
                rect.x + 14.0,
                Rect {
                    y: rect.y + 28.0,
                    height: ctx.metrics.page.button_height,
                    ..rect
                },
                ctx.metrics.typography.workspace,
ctx.metrics.typography.weight_medium,
                ctx.theme.text_main,
            );
        }
    }
}

impl Default for OverviewPage {
    fn default() -> Self {
        Self::new()
    }
}

/// A short human-readable summary of a subsystem snapshot.
fn summarize(label: &str, snapshot: serde_json::Value) -> String {
    match label {
        "Audio" => {
            let volume = snapshot["volume"].as_str().unwrap_or("--");
            let muted = snapshot["mute"].as_str() == Some("yes");
            if muted {
                format!("muted ({volume})")
            } else {
                volume.to_string()
            }
        }
        "Bluetooth" => snapshot["state"].as_str().unwrap_or("unknown").to_string(),
        "Network" => snapshot["connections"]
            .as_array()
            .and_then(|list| list.first())
            .and_then(|conn| conn["connection"].as_str())
            .unwrap_or("disconnected")
            .to_string(),
        "Battery" => snapshot["battery"]["pct"]
            .as_u64()
            .map(|p| format!("{p}%"))
            .unwrap_or_else(|| "--".into()),
        _ => String::new(),
    }
}
