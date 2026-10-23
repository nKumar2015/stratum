//! The Today tab's own panels: the glance strip and the weather.
//!
//! - Glance: the time and date, uptime, then on the right CPU and RAM, the
//!   battery (with time left), each connected Bluetooth device that reports
//!   its charge, the Focus tab's timer while set, and do-not-disturb, which
//!   a click toggles.
//! - Weather: the place, current conditions, the next hours and the next
//!   days, from [`super::weather`]. A forecast older than `refresh_min`, or
//!   a failed fetch, says so at the bottom.

use chrono::{NaiveDate, NaiveDateTime};
use serde_json::Value;
use stratum_sys::managers::sysmon::Sample;
use stratum_sys::managers::weather::{describe, Units};

use super::{Button, Dashboard};
use crate::render::display_list::{Color, DisplayList, Rect};
use crate::ui::{self, PaintCtx};

/// "up 3h 12m": days and hours past a day, hours and minutes past an hour.
pub fn uptime(seconds: u64) -> String {
    let (days, hours, minutes) = (seconds / 86_400, seconds % 86_400 / 3600, seconds % 3600 / 60);
    if days > 0 {
        format!("up {days}d {hours}h")
    } else if hours > 0 {
        format!("up {hours}h {minutes}m")
    } else {
        format!("up {minutes}m")
    }
}

/// How long ago a forecast was fetched.
pub fn age(seconds: u64) -> String {
    match seconds / 60 {
        0 => "just now".into(),
        minutes @ 1..=59 => format!("{minutes} min ago"),
        minutes if minutes < 48 * 60 => format!("{} h ago", minutes / 60),
        minutes => format!("{} days ago", minutes / (24 * 60)),
    }
}

/// A Nerd Font glyph for a WMO weather code.
pub fn weather_glyph(code: u8, is_day: bool) -> &'static str {
    match code {
        0 | 1 if is_day => "\u{f0599}",
        0 | 1 => "\u{f0594}",
        2 if is_day => "\u{f0595}",
        2 => "\u{f0f31}",
        3 => "\u{f0590}",
        45 | 48 => "\u{f0591}",
        51..=57 | 80..=82 => "\u{f0597}",
        61..=67 => "\u{f0596}",
        71..=77 | 85 | 86 => "\u{f0598}",
        95..=99 => "\u{f067e}",
        _ => "\u{f0590}",
    }
}

/// "3 PM" for an hourly forecast's `2026-10-23T15:00`.
pub fn hour_label(time: &str) -> String {
    NaiveDateTime::parse_from_str(time, "%Y-%m-%dT%H:%M").map_or_else(|_| time.to_string(), |t| t.format("%-I %p").to_string())
}

/// "Today", then weekday abbreviations, for a daily forecast's dates.
pub fn day_label(date: &str, index: usize) -> String {
    if index == 0 {
        return "Today".into();
    }
    NaiveDate::parse_from_str(date, "%Y-%m-%d").map_or_else(|_| date.to_string(), |d| d.format("%a").to_string())
}

/// Connected Bluetooth devices that report their charge:
/// `(glyph, name, percent)`.
pub fn device_batteries(bluetooth: &Value) -> Vec<(&'static str, String, u64)> {
    bluetooth["devices"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|d| d["connected"] == "yes")
        .filter_map(|d| {
            let percent = d["battery"].as_u64()?;
            let glyph = crate::ui::pages::bluetooth::device_glyph(d["icon"].as_str().unwrap_or_default());
            Some((glyph, d["name"].as_str().unwrap_or_default().to_string(), percent))
        })
        .collect()
}

fn degrees(value: f32) -> String {
    format!("{}\u{b0}", value.round())
}

pub(super) fn measure(ctx: &mut PaintCtx<'_>, text: &str, size: f32, weight: u16) -> f32 {
    let style = crate::text::TextStyle {
        family: std::sync::Arc::from(ctx.theme.font.as_str()),
        size,
        weight,
        color: ctx.theme.text_main,
    };
    ctx.text.measure(text, &style).0
}

impl Dashboard {
    pub(super) fn paint_glance(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, panel: Rect, sample: Option<&Sample>) {
        let m = self.metrics.clone();
        let (regular, medium, bold) = (
            ctx.metrics.typography.weight_regular,
            ctx.metrics.typography.weight_medium,
            ctx.metrics.typography.weight_bold,
        );
        let row = Rect { x: panel.x + m.inner, width: panel.width - m.inner * 2.0, ..panel };

        // Left: the time, then the date and uptime.
        let now = chrono::Local::now();
        let time = now.format("%-I:%M").to_string();
        let mut x = row.x;
        ui::paint_text_vcentered(ctx, list, &time, x, row, m.clock, bold, ctx.theme.text_main);
        x += measure(ctx, &time, m.clock, bold) + m.inner * 0.4;
        let meridiem = now.format("%p").to_string();
        ui::paint_text_vcentered(ctx, list, &meridiem, x, row, m.text, medium, ctx.theme.text_muted);
        x += measure(ctx, &meridiem, m.text, medium) + m.inner * 1.5;
        let date = now.format("%A, %-d %B").to_string();
        ui::paint_text_vcentered(ctx, list, &date, x, row, m.text, medium, ctx.theme.text_main);
        x += measure(ctx, &date, m.text, medium);

        // Right, laid out from the edge inwards: do-not-disturb, devices,
        // the battery, the next event, then CPU and RAM. Short of room, the
        // uptime goes first, then the leftmost chips.
        let dnd = ctx.state.dnd.get();
        let mut chips: Vec<(&str, String, Color)> = Vec::new();
        if let Some(s) = sample {
            let cpu = s.cpu_percent.map_or_else(|| "\u{2013}".into(), |p| format!("{}%", p.round()));
            chips.push(("", format!("CPU {cpu} \u{b7} RAM {}%", s.memory.percent().round()), ctx.theme.primary));
        }
        if let Some(next) = self.next_event_chip(ctx) {
            chips.push(("\u{f00ed}", next, ctx.theme.primary));
        }
        let battery = ctx.state.battery.get();
        if battery["battery"]["present"].as_bool() == Some(true) {
            let percent = battery["battery"]["pct"].as_u64().unwrap_or(0);
            let left = battery["battery"]["projected_text"].as_str().filter(|t| *t != "Unknown");
            let text = left.map_or_else(|| format!("{percent}%"), |left| format!("{percent}% \u{b7} {left}"));
            chips.push((crate::ui::widgets::battery::snapshot_glyph(&battery), text, ctx.theme.primary));
        }
        for (glyph, name, percent) in device_batteries(&ctx.state.bluetooth.get()) {
            chips.push((glyph, format!("{name} {percent}%"), ctx.theme.primary));
        }
        // The Focus tab's timer, last so it sits by the bell and goes last.
        if let Some((glyph, text)) = super::focus::glance_chip(&self.focus.timer, std::time::Instant::now()) {
            let color = if self.focus.timer.running() { ctx.theme.warning } else { ctx.theme.text_muted };
            chips.push((glyph, text, color));
        }
        let mut right = row.x + row.width;
        let (bell, bell_color) = if dnd { ("\u{f009b}", ctx.theme.warning) } else { ("\u{f009a}", ctx.theme.text_muted) };
        let bell_w = measure(ctx, bell, m.title, regular);
        right -= bell_w;
        ui::paint_text_vcentered(ctx, list, bell, right, row, m.title, regular, bell_color);
        let hit = Rect { x: right - m.inner * 0.5, width: bell_w + m.inner, ..row };
        self.buttons.push((hit, Button::Dnd));
        for (glyph, text, color) in chips.into_iter().rev() {
            let text_w = measure(ctx, &text, m.text, medium);
            // A chip without a glyph has no space for one either.
            let glyph_w = if glyph.is_empty() { -m.inner * 0.4 } else { measure(ctx, glyph, m.text, regular) };
            let start = right - m.inner * 1.5 - text_w - glyph_w - m.inner * 0.4;
            // Chips that would run into the date are left out.
            if start < x + m.inner {
                break;
            }
            ui::paint_text_vcentered(ctx, list, glyph, start, row, m.text, regular, color);
            ui::paint_text_vcentered(ctx, list, &text, start + glyph_w + m.inner * 0.4, row, m.text, medium, ctx.theme.text_main);
            right = start;
        }
        // Uptime last, in what the chips leave: it matters least.
        if let Some(up) = sample.and_then(|s| s.uptime) {
            let up = format!("  \u{b7}  {}", uptime(up));
            if x + measure(ctx, &up, m.caption, regular) + m.inner <= right {
                ui::paint_text_vcentered(ctx, list, &up, x, row, m.caption, regular, ctx.theme.text_muted);
            }
        }
    }

    pub(super) fn paint_weather(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, panel: Rect) {
        let m = self.metrics.clone();
        let weather = ctx.state.weather.get();
        let (regular, medium, bold) = (
            ctx.metrics.typography.weight_regular,
            ctx.metrics.typography.weight_medium,
            ctx.metrics.typography.weight_bold,
        );
        let x = panel.x + m.inner;
        let w = panel.width - m.inner * 2.0;
        let Some(report) = weather.report.as_ref() else {
            let mut y = Self::title(ctx, list, panel, "Weather", &m);
            let (text, color) = match &weather.error {
                Some(error) if !weather.loading => (error.as_str(), ctx.theme.error),
                _ => ("Loading\u{2026}", ctx.theme.text_muted),
            };
            for line in ui::wrap_text(ctx, text, m.caption, regular, w, 4) {
                let row = Rect { x, y, width: w, height: m.line, radius: 0.0, color: Color::TRANSPARENT };
                ui::paint_text_vcentered(ctx, list, &line, x, row, m.caption, regular, color);
                y += m.line;
            }
            return;
        };
        let title = if report.place.is_empty() { "Weather" } else { report.place.as_str() };
        let mut y = Self::title(ctx, list, panel, title, &m);
        let current = &report.forecast.current;

        // Now: glyph and temperature, then the conditions beside them.
        let block = Rect { x, y, width: w, height: m.weather_icon, radius: 0.0, color: Color::TRANSPARENT };
        let glyph = weather_glyph(current.code, current.is_day);
        ui::paint_text_vcentered(ctx, list, glyph, x, block, m.weather_icon, regular, ctx.theme.primary);
        let mut cx = x + measure(ctx, glyph, m.weather_icon, regular) + m.inner;
        let temperature = degrees(current.temperature);
        ui::paint_text_vcentered(ctx, list, &temperature, cx, block, m.weather_temp, bold, ctx.theme.text_main);
        cx += measure(ctx, &temperature, m.weather_temp, bold) + m.inner;
        let half = Rect { x: cx, height: block.height * 0.5, width: x + w - cx, ..block };
        let condition = ui::elide(ctx, describe(current.code), m.text, medium, half.width);
        ui::paint_text_vcentered(ctx, list, &condition, cx, half, m.text, medium, ctx.theme.text_main);
        let feels = format!("Feels {}", degrees(current.feels_like));
        ui::paint_text_vcentered(ctx, list, &feels, cx, Rect { y: half.y + half.height, ..half }, m.caption, regular, ctx.theme.text_muted);
        y += block.height + m.inner * 0.5;

        let wind = match report.units {
            Units::Imperial => "mph",
            Units::Metric => "km/h",
        };
        let mut details = Vec::new();
        if let Some(today) = report.forecast.days.first() {
            details.push(format!("H {}  L {}", degrees(today.high), degrees(today.low)));
        }
        details.push(format!("{} {wind}", current.wind.round()));
        details.push(format!("\u{f058e} {}%", current.humidity.round()));
        let line = Rect { x, y, width: w, height: m.line, radius: 0.0, color: Color::TRANSPARENT };
        let details = ui::elide(ctx, &details.join(" \u{b7} "), m.caption, regular, w);
        ui::paint_text_vcentered(ctx, list, &details, x, line, m.caption, regular, ctx.theme.text_muted);
        y += m.line + m.inner * 0.5;

        // The next hours, as columns.
        let hours = &report.forecast.hours;
        if !hours.is_empty() {
            let column = w / hours.len() as f32;
            for (i, hour) in hours.iter().enumerate() {
                let cell = |row: usize, height: f32| Rect {
                    x: x + column * i as f32,
                    y: y + m.line * row as f32,
                    width: column,
                    height,
                    radius: 0.0,
                    color: Color::TRANSPARENT,
                };
                ui::paint_text_centered(ctx, list, &hour_label(&hour.time), cell(0, m.line), m.caption, ctx.theme.text_muted);
                ui::paint_text_centered(ctx, list, weather_glyph(hour.code, hour.is_day), cell(1, m.line), m.text * 1.2, ctx.theme.primary);
                ui::paint_text_centered(ctx, list, &degrees(hour.temperature), cell(2, m.line), m.text, ctx.theme.text_main);
            }
            y += m.line * 3.0 + m.inner;
        }

        // The next days, as rows: name, glyph, chance of rain, high / low.
        let bottom = panel.y + panel.height - m.inner - m.line;
        for (i, day) in report.forecast.days.iter().enumerate() {
            if y + m.line > bottom {
                break;
            }
            let row = Rect { x, y, width: w, height: m.line, radius: 0.0, color: Color::TRANSPARENT };
            ui::paint_text_vcentered(ctx, list, &day_label(&day.date, i), x, row, m.text, medium, ctx.theme.text_main);
            let glyph_x = x + w * 0.3;
            ui::paint_text_vcentered(ctx, list, weather_glyph(day.code, true), glyph_x, row, m.text * 1.2, regular, ctx.theme.primary);
            if day.precipitation > 0 {
                let rain = format!("{}%", day.precipitation);
                ui::paint_text_vcentered(ctx, list, &rain, glyph_x + m.text * 1.8, row, m.caption, regular, ctx.theme.text_muted);
            }
            let range = format!("{} / {}", degrees(day.high), degrees(day.low));
            let rw = measure(ctx, &range, m.text, regular);
            ui::paint_text_vcentered(ctx, list, &range, x + w - rw, row, m.text, regular, ctx.theme.text_main);
            y += m.line;
        }

        // Say when this is not fresh.
        let footer = Rect { x, y: panel.y + panel.height - m.inner - m.line, width: w, height: m.line, radius: 0.0, color: Color::TRANSPARENT };
        let stale = report.age() >= self.config.weather.refresh_min * 60;
        let note = match (&weather.error, weather.loading) {
            (_, true) => Some(("Updating\u{2026}".to_string(), ctx.theme.text_muted)),
            (Some(_), false) => Some((format!("Offline \u{b7} updated {}", age(report.age())), ctx.theme.warning)),
            (None, false) if stale => Some((format!("Updated {}", age(report.age())), ctx.theme.text_muted)),
            _ => None,
        };
        if let Some((note, color)) = note {
            let note = ui::elide(ctx, &note, m.caption, regular, w);
            ui::paint_text_vcentered(ctx, list, &note, x, footer, m.caption, regular, color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn uptime_reads_in_the_two_largest_units() {
        assert_eq!(uptime(59), "up 0m");
        assert_eq!(uptime(3 * 3600 + 12 * 60), "up 3h 12m");
        assert_eq!(uptime(2 * 86_400 + 5 * 3600 + 7), "up 2d 5h");
    }

    #[test]
    fn ages_round_down() {
        assert_eq!(age(30), "just now");
        assert_eq!(age(40 * 60), "40 min ago");
        assert_eq!(age(5 * 3600), "5 h ago");
        assert_eq!(age(3 * 86_400), "3 days ago");
    }

    #[test]
    fn forecast_times_read_as_hours_and_weekdays() {
        assert_eq!(hour_label("2026-10-23T15:00"), "3 PM");
        assert_eq!(hour_label("2026-10-23T00:00"), "12 AM");
        assert_eq!(day_label("2026-10-23", 0), "Today");
        assert_eq!(day_label("2026-10-24", 1), "Sat");
        assert_eq!(hour_label("garbage"), "garbage");
    }

    #[test]
    fn clear_skies_differ_by_day_and_night() {
        assert_ne!(weather_glyph(0, true), weather_glyph(0, false));
        assert_eq!(weather_glyph(3, true), weather_glyph(3, false));
    }

    #[test]
    fn only_connected_devices_with_a_charge_are_listed() {
        let bluetooth = json!({"devices": [
            {"name": "Buds", "connected": "yes", "icon": "audio-headset", "battery": 80},
            {"name": "Mouse", "connected": "yes", "icon": "input-mouse", "battery": null},
            {"name": "Old", "connected": "no", "icon": "audio-headset", "battery": 50},
        ]});
        let devices = device_batteries(&bluetooth);
        assert_eq!(devices.len(), 1);
        assert_eq!((devices[0].1.as_str(), devices[0].2), ("Buds", 80));
    }
}
