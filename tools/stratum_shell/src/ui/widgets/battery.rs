//! Battery indicator: a level glyph with the percentage beneath it, as the QML
//! battery widget drew them.

use std::collections::HashMap;

use crate::render::display_list::{Color, DisplayList, Point, Rect};
use crate::ui::control_center::Page;
use crate::ui::hover_menu::HoverKind;
use crate::ui::{self, Action, ClickCtx, InputState, PaintCtx, Widget};

/// How the glyph and label are coloured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tone {
    /// No battery reading yet, or no battery at all.
    Unknown,
    /// On the charger: charging, full, or plugged in but not charging.
    Charging,
    /// At or below 20% on battery.
    Low,
    Normal,
}

/// The reading the widget draws. `None` is QML's "not ready".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Reading<'a> {
    percent: u8,
    state: &'a str,
}

impl<'a> Reading<'a> {
    fn from_snapshot(snapshot: &'a serde_json::Value) -> Option<Self> {
        let battery = &snapshot["battery"];
        if !battery["present"].as_bool().unwrap_or(false) {
            return None;
        }
        let percent = battery["pct"].as_u64()?.min(100) as u8;
        Some(Self {
            percent,
            state: battery["state"].as_str().unwrap_or(""),
        })
    }
}

fn on_charger(state: &str) -> bool {
    matches!(state, "charging" | "pending-charge" | "fully-charged")
}

/// The QML widget's glyph: charging and full have their own, then one glyph per
/// ten-percent bucket.
fn glyph(reading: Option<Reading<'_>>) -> &'static str {
    let Some(reading) = reading else {
        return "\u{f008e}";
    };
    match reading.state {
        "charging" | "pending-charge" => return "\u{f0084}",
        "fully-charged" => return "\u{f0079}",
        _ => {}
    }
    match reading.percent {
        90.. => "\u{f0079}",
        80..=89 => "\u{f0082}",
        70..=79 => "\u{f0081}",
        60..=69 => "\u{f0080}",
        50..=59 => "\u{f007f}",
        40..=49 => "\u{f007e}",
        30..=39 => "\u{f007d}",
        20..=29 => "\u{f007c}",
        10..=19 => "\u{f007b}",
        _ => "\u{f008e}",
    }
}

fn tone(reading: Option<Reading<'_>>) -> Tone {
    match reading {
        None => Tone::Unknown,
        Some(r) if on_charger(r.state) => Tone::Charging,
        Some(r) if r.percent <= 20 => Tone::Low,
        Some(_) => Tone::Normal,
    }
}

fn label(reading: Option<Reading<'_>>) -> String {
    reading.map_or_else(|| "--".to_string(), |r| format!("{}%", r.percent))
}

pub struct Battery {
    /// Rect from the last paint on each output; see `Icon` for why per output.
    rects: HashMap<String, Rect>,
}

impl Battery {
    pub fn new() -> Self {
        Self {
            rects: HashMap::new(),
        }
    }

    fn hit(&self, output: &str, point: Point) -> Option<Rect> {
        self.rects.get(output).copied().filter(|r| r.contains(point))
    }
}

impl Default for Battery {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for Battery {
    fn height(&self, _width: f32, metrics: &crate::ui::style::Metrics) -> Option<f32> {
        let t = &metrics.typography;
        Some(t.battery + t.battery_label * metrics.bar.label_line)
    }

    fn width(&self, height: f32, metrics: &crate::ui::style::Metrics) -> Option<f32> {
        // Glyph and label side by side in a horizontal bar.
        let t = &metrics.typography;
        Some(height.max(t.battery) + metrics.bar.battery_label_width)
    }

    fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        rect: Rect,
        _input: &InputState,
    ) {
        self.rects.insert(ctx.output_name.to_string(), rect);
        let snapshot = ctx.state.battery.get();
        let reading = Reading::from_snapshot(&snapshot);
        let color = match tone(reading) {
            Tone::Unknown => ctx.theme.text_main,
            Tone::Charging => ctx.theme.success,
            Tone::Low => ctx.theme.error,
            Tone::Normal => ctx.theme.primary,
        };
        let glyph_size = ctx.metrics.typography.battery;
        let label_size = ctx.metrics.typography.battery_label;
        let weight = ctx.metrics.typography.weight_bold;

        let (glyph_rect, label_rect) = if ctx.is_vertical() {
            let glyph_rect = Rect {
                height: glyph_size,
                color: Color::TRANSPARENT,
                ..rect
            };
            let label_rect = Rect {
                y: rect.y + glyph_size,
                height: (rect.height - glyph_size).max(0.0),
                color: Color::TRANSPARENT,
                ..rect
            };
            (glyph_rect, label_rect)
        } else {
            let label_width = ctx.metrics.bar.battery_label_width;
            let glyph_rect = Rect {
                width: (rect.width - label_width).max(0.0),
                color: Color::TRANSPARENT,
                ..rect
            };
            let label_rect = Rect {
                x: glyph_rect.x + glyph_rect.width,
                width: label_width,
                color: Color::TRANSPARENT,
                ..rect
            };
            (glyph_rect, label_rect)
        };
        // Lying on its side, as QML's `rotation: 90`.
        ui::paint_text_centered_rotated(ctx, list, glyph(reading), glyph_rect, glyph_size, color, std::f32::consts::FRAC_PI_2);
        ui::paint_text_centered_weighted(
            ctx,
            list,
            &label(reading),
            label_rect,
            label_size,
            weight,
            color,
        );
    }

    fn on_click(&mut self, ctx: &ClickCtx<'_>) -> Option<Action> {
        self.hit(ctx.output_name, ctx.point)
            .map(|_| Action::OpenPanel(Page::Battery))
    }

    fn hover_target(&self, output: &str, point: Point) -> Option<(HoverKind, Rect)> {
        self.hit(output, point).map(|rect| (HoverKind::Battery, rect))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading(percent: u8, state: &str) -> Option<Reading<'_>> {
        Some(Reading { percent, state })
    }

    #[test]
    fn glyphs_follow_the_qml_buckets() {
        assert_eq!(glyph(reading(95, "discharging")), "\u{f0079}");
        assert_eq!(glyph(reading(85, "discharging")), "\u{f0082}");
        assert_eq!(glyph(reading(55, "discharging")), "\u{f007f}");
        assert_eq!(glyph(reading(15, "discharging")), "\u{f007b}");
        assert_eq!(glyph(reading(5, "discharging")), "\u{f008e}");
        assert_eq!(glyph(reading(40, "charging")), "\u{f0084}");
        assert_eq!(glyph(reading(80, "pending-charge")), "\u{f0084}");
        assert_eq!(glyph(reading(100, "fully-charged")), "\u{f0079}");
        assert_eq!(glyph(None), "\u{f008e}");
    }

    #[test]
    fn charging_wins_over_low() {
        assert_eq!(tone(reading(10, "charging")), Tone::Charging);
        assert_eq!(tone(reading(20, "discharging")), Tone::Low);
        assert_eq!(tone(reading(21, "discharging")), Tone::Normal);
        assert_eq!(tone(None), Tone::Unknown);
    }

    #[test]
    fn missing_battery_is_not_ready() {
        let none = serde_json::json!({ "battery": { "present": false, "pct": 0 } });
        assert_eq!(Reading::from_snapshot(&none), None);
        assert_eq!(Reading::from_snapshot(&serde_json::Value::Null), None);
        assert_eq!(label(None), "--");

        let some = serde_json::json!({ "battery": { "present": true, "pct": 64, "state": "discharging" } });
        assert_eq!(label(Reading::from_snapshot(&some)), "64%");
    }
}
