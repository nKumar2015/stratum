//! The dashboard overlay: performance, calendar, and now playing.
//!
//! Matching `modules/DashboardMenu.qml` and its three panels:
//!
//! - Opened by `stratum-cli dashboard open|close|toggle`, on the focused
//!   output, as a card dropping in at the top centre over a dimmed screen.
//!   A click outside the card or Escape closes it.
//! - Performance: CPU, GPU (with its source), RAM, and storage, each with a
//!   bar coloured by load (≥ 90 % error, ≥ 75 % warning, else success).
//! - Calendar: the month grid (Sunday first, neighbouring months' days
//!   dimmed, today marked), with «« « Today » »» for year and month.
//! - Now playing: art, title, artist, album, position and length, and
//!   previous / play-pause / next.
//!
//! Data comes from `stratum_sys::managers::dashboard::status` through the
//! action worker, refreshed every `refresh_ms` only while the dashboard is
//! open, as the QML `dashboard.watch` / `unwatch` lifecycle did. The calendar
//! grid is computed here: the QML shell cached ±3 months to hide the daemon's
//! round trip, and computing a month locally is instant. Closing resets the
//! calendar to the current month, as QML did.

use chrono::Datelike;
use serde_json::Value;

use crate::render::display_list::{Color, DisplayList, Point, Rect};
use crate::ui::text_field;
use crate::ui::{self, PaintCtx};

/// What the shell has to do after input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DashboardAction {
    None,
    Redraw,
    /// Fetch the dashboard for this month now.
    Refresh,
    /// `previous`, `play-pause`, or `next`, for the player.
    Media { player: String, action: String },
}

/// One cell of the month grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Day {
    pub day: u32,
    pub in_month: bool,
    pub today: bool,
}

fn days_in(year: i32, month: u32) -> u32 {
    let (ny, nm) = if month == 12 { (year + 1, 1) } else { (year, month + 1) };
    chrono::NaiveDate::from_ymd_opt(ny, nm, 1)
        .and_then(|d| d.pred_opt())
        .map(|d| d.day())
        .unwrap_or(30)
}

/// The 6×7 grid for `year`/`month`, Sunday first, padded with the
/// neighbouring months' days.
pub fn month_grid(year: i32, month: u32, today: (i32, u32, u32)) -> [Day; 42] {
    let first = chrono::NaiveDate::from_ymd_opt(year, month, 1).unwrap_or_default();
    let lead = first.weekday().num_days_from_sunday();
    let count = days_in(year, month);
    let (py, pm) = shift(year, month, -1);
    let prev = days_in(py, pm);
    let mut grid = [Day { day: 0, in_month: false, today: false }; 42];
    for (i, cell) in grid.iter_mut().enumerate() {
        let i = i as u32;
        *cell = if i < lead {
            Day { day: prev - lead + i + 1, in_month: false, today: false }
        } else if i - lead < count {
            let day = i - lead + 1;
            Day { day, in_month: true, today: (year, month, day) == today }
        } else {
            Day { day: i - lead - count + 1, in_month: false, today: false }
        };
    }
    grid
}

/// Move `delta` months from `year`/`month`.
pub fn shift(year: i32, month: u32, delta: i32) -> (i32, u32) {
    let index = year * 12 + month as i32 - 1 + delta;
    (index.div_euclid(12).max(1), index.rem_euclid(12) as u32 + 1)
}

fn today() -> (i32, u32, u32) {
    let now = chrono::Local::now();
    (now.year(), now.month(), now.day())
}

/// The bar colour for a load percentage, as `metricColor` chose it.
fn load_color(ctx: &PaintCtx<'_>, percent: f64) -> Color {
    if percent >= 90.0 {
        ctx.theme.error
    } else if percent >= 75.0 {
        ctx.theme.warning
    } else {
        ctx.theme.success
    }
}

const MONTHS: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November",
    "December",
];
const WEEKDAYS: [&str; 7] = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];

pub struct Dashboard {
    open: bool,
    target: Option<String>,
    year: i32,
    month: u32,
    /// Hit rects from the last paint.
    card: Option<Rect>,
    buttons: Vec<(Rect, Button)>,
    /// "No active player", shown after a control was pressed without one.
    error: Option<String>,
    metrics: crate::ui::style::DashboardMetrics,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Button {
    Month(i32),
    Year(i32),
    Today,
    Previous,
    PlayPause,
    Next,
}

impl Dashboard {
    pub fn new() -> Self {
        let (year, month, _) = today();
        Self {
            open: false,
            target: None,
            year,
            month,
            card: None,
            buttons: Vec::new(),
            error: None,
            metrics: crate::ui::style::DashboardMetrics::default(),
        }
    }

    pub fn set_metrics(&mut self, metrics: &crate::ui::style::DashboardMetrics) {
        self.metrics = metrics.clone();
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn target(&self) -> Option<&str> {
        self.open.then_some(self.target.as_deref()).flatten()
    }

    /// The month whose data should be fetched.
    pub fn month(&self) -> (i32, u32) {
        (self.year, self.month)
    }

    pub fn open_on(&mut self, output: &str) {
        self.open = true;
        self.target = Some(output.to_string());
        self.error = None;
    }

    /// Close, and return to the current month for next time.
    pub fn close(&mut self) {
        self.open = false;
        self.target = None;
        self.card = None;
        self.buttons.clear();
        let (year, month, _) = today();
        (self.year, self.month) = (year, month);
    }

    pub fn toggle(&mut self, output: &str) {
        if self.open {
            self.close();
        } else {
            self.open_on(output);
        }
    }

    pub fn key(&mut self, keysym: smithay_client_toolkit::seat::keyboard::Keysym) -> DashboardAction {
        use smithay_client_toolkit::seat::keyboard::Keysym;
        if !self.open {
            return DashboardAction::None;
        }
        match keysym {
            Keysym::Escape => {
                self.close();
                DashboardAction::Redraw
            }
            Keysym::Left => self.navigate(Button::Month(-1)),
            Keysym::Right => self.navigate(Button::Month(1)),
            _ => DashboardAction::None,
        }
    }

    fn navigate(&mut self, button: Button) -> DashboardAction {
        (self.year, self.month) = match button {
            Button::Month(delta) => shift(self.year, self.month, delta),
            Button::Year(delta) => ((self.year + delta).max(1), self.month),
            Button::Today => {
                let (year, month, _) = today();
                (year, month)
            }
            _ => return DashboardAction::None,
        };
        DashboardAction::Refresh
    }

    pub fn on_click(&mut self, point: Point, snapshot: &Value) -> DashboardAction {
        if !self.open {
            return DashboardAction::None;
        }
        if let Some(button) = self.buttons.iter().find(|(r, _)| r.contains(point)).map(|(_, b)| *b) {
            return match button {
                Button::Previous | Button::PlayPause | Button::Next => {
                    let player = snapshot["music"]["player"].as_str().unwrap_or("None");
                    if matches!(player, "" | "None" | "N/A") {
                        self.error = Some("No active player".into());
                        return DashboardAction::Redraw;
                    }
                    self.error = None;
                    let action = match button {
                        Button::Previous => "previous",
                        Button::Next => "next",
                        _ => "play-pause",
                    };
                    DashboardAction::Media {
                        player: player.to_string(),
                        action: action.to_string(),
                    }
                }
                other => self.navigate(other),
            };
        }
        if self.card.is_some_and(|card| !card.contains(point)) {
            self.close();
            return DashboardAction::Redraw;
        }
        DashboardAction::None
    }

    pub fn paint(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, surface: Rect, snapshot: &Value) {
        if !self.open {
            return;
        }
        let m = self.metrics.clone();
        list.push_rect(Rect { color: Color::rgba(0.0, 0.0, 0.0, m.scrim_alpha), ..surface });

        let width = (m.performance_width + m.calendar_width + m.music_width + m.gap * 2.0 + m.padding * 2.0)
            .min(surface.width - m.margin * 2.0);
        let height = m.panel_height + m.padding * 2.0 + if self.error.is_some() { m.line } else { 0.0 };
        let card = Rect {
            x: surface.x + (surface.width - width) * 0.5,
            y: surface.y + m.margin,
            width,
            height,
            radius: m.radius,
            color: Color::TRANSPARENT,
        };
        text_field::paint_frame(list, card, m.border, ctx.theme.border_active, ctx.theme.bg_main);
        self.buttons.clear();

        let mut y = card.y + m.padding;
        if let Some(error) = self.error.clone() {
            let row = Rect { x: card.x + m.padding, y, width: card.width, height: m.line, radius: 0.0, color: Color::TRANSPARENT };
            ui::paint_text_vcentered(ctx, list, &error, row.x, row, m.caption, ctx.metrics.typography.weight_regular, ctx.theme.error);
            y += m.line;
        }
        let mut x = card.x + m.padding;
        let panel = |x: f32, w: f32| Rect { x, y, width: w, height: m.panel_height, radius: m.radius, color: Color::TRANSPARENT };
        let performance = panel(x, m.performance_width);
        x += m.performance_width + m.gap;
        let calendar = panel(x, m.calendar_width);
        x += m.calendar_width + m.gap;
        let music = panel(x, m.music_width);
        for p in [performance, calendar, music] {
            text_field::paint_frame(list, p, m.border, ctx.theme.border_inactive, ctx.theme.bg_widget);
        }
        self.paint_performance(ctx, list, performance, &snapshot["performance"]);
        self.paint_calendar(ctx, list, calendar);
        self.paint_music(ctx, list, music, &snapshot["music"]);
        self.card = Some(card);
    }

    fn title(ctx: &mut PaintCtx<'_>, list: &mut DisplayList, panel: Rect, text: &str, m: &crate::ui::style::DashboardMetrics) -> f32 {
        let row = Rect { x: panel.x + m.inner, y: panel.y + m.inner, width: panel.width - m.inner * 2.0, height: m.line * 1.2, radius: 0.0, color: Color::TRANSPARENT };
        ui::paint_text_vcentered(ctx, list, text, row.x, row, m.title, ctx.metrics.typography.weight_bold, ctx.theme.text_main);
        row.y + row.height + m.inner
    }

    fn paint_performance(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, panel: Rect, perf: &Value) {
        let m = self.metrics.clone();
        let mut y = Self::title(ctx, list, panel, "Performance", &m);
        let x = panel.x + m.inner;
        let w = panel.width - m.inner * 2.0;
        let num = |key: &str| perf[key].as_f64().unwrap_or(0.0);
        let gpu_text = perf["gpu_percent_text"].as_str().unwrap_or("N/A").to_string();
        let gpu_value = perf["gpu_percent_value"].as_f64().unwrap_or(0.0);
        let rows = [
            ("\u{f2db}", "CPU".to_string(), String::new(), format!("{}%", num("cpu_percent").round()), num("cpu_percent")),
            (
                "\u{f08ae}",
                "GPU".to_string(),
                perf["gpu_source"].as_str().unwrap_or_default().to_string(),
                if gpu_text == "N/A" { "N/A".to_string() } else { format!("{gpu_text}%") },
                gpu_value,
            ),
            (
                "\u{f035b}",
                "RAM".to_string(),
                format!("{:.1} / {:.1} GiB", num("ram_used_gib"), num("ram_total_gib")),
                format!("{}%", num("ram_percent").round()),
                num("ram_percent"),
            ),
            (
                "\u{f0a0}",
                "Storage".to_string(),
                format!("{:.1} / {:.1} GiB", num("storage_used_gib"), num("storage_total_gib")),
                format!("{}%", num("storage_percent").round()),
                num("storage_percent"),
            ),
        ];
        let style = |ctx: &PaintCtx<'_>, size: f32| crate::text::TextStyle {
            family: std::sync::Arc::from(ctx.theme.font.as_str()),
            size,
            weight: ctx.metrics.typography.weight_medium,
            color: ctx.theme.text_main,
        };
        for (glyph, label, detail, value, percent) in rows {
            let row = Rect { x, y, width: w, height: m.line, radius: 0.0, color: Color::TRANSPARENT };
            ui::paint_text_vcentered(ctx, list, glyph, x, row, m.text, ctx.metrics.typography.weight_regular, ctx.theme.primary);
            ui::paint_text_vcentered(ctx, list, &label, x + m.text * 1.6, row, m.text, ctx.metrics.typography.weight_medium, ctx.theme.text_main);
            let value_w = ctx.text.measure(&value, &style(ctx, m.text)).0;
            ui::paint_text_vcentered(ctx, list, &value, x + w - value_w, row, m.text, ctx.metrics.typography.weight_medium, ctx.theme.text_main);
            y += m.line;
            if !detail.is_empty() {
                let sub = Rect { y, height: m.line * 0.8, ..row };
                ui::paint_text_vcentered(ctx, list, &detail, x + m.text * 1.6, sub, m.caption, ctx.metrics.typography.weight_regular, ctx.theme.text_muted);
                y += sub.height;
            }
            let track = Rect { x, y: y + m.bar * 0.5, width: w, height: m.bar, radius: m.bar * 0.5, color: ctx.theme.bg_hover };
            list.push_rect(track);
            list.push_rect(Rect { width: w * (percent.clamp(0.0, 100.0) / 100.0) as f32, color: load_color(ctx, percent), ..track });
            y += m.bar * 2.0 + m.inner;
        }
    }

    fn paint_calendar(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, panel: Rect) {
        let m = self.metrics.clone();
        let title = format!("{} {}", MONTHS[(self.month - 1) as usize % 12], self.year);
        let mut y = Self::title(ctx, list, panel, &title, &m);
        let x = panel.x + m.inner;
        let w = panel.width - m.inner * 2.0;
        let cell_w = w / 7.0;
        for (i, label) in WEEKDAYS.iter().enumerate() {
            let cell = Rect { x: x + cell_w * i as f32, y, width: cell_w, height: m.line, radius: 0.0, color: Color::TRANSPARENT };
            ui::paint_text_centered(ctx, list, label, cell, m.caption, ctx.theme.text_muted);
        }
        y += m.line;
        let grid = month_grid(self.year, self.month, today());
        for (i, day) in grid.iter().enumerate() {
            let (row, col) = (i / 7, i % 7);
            let cell = Rect {
                x: x + cell_w * col as f32 + m.cell_gap * 0.5,
                y: y + m.cell * row as f32 + m.cell_gap * 0.5,
                width: cell_w - m.cell_gap,
                height: m.cell - m.cell_gap,
                radius: m.radius * 0.6,
                color: Color::TRANSPARENT,
            };
            text_field::paint_frame(
                list,
                cell,
                m.border,
                if day.today { ctx.theme.border_active } else { ctx.theme.border_inactive },
                if day.in_month { ctx.theme.bg_widget } else { ctx.theme.bg_main },
            );
            let color = if day.today {
                ctx.theme.primary
            } else if day.in_month {
                ctx.theme.text_main
            } else {
                ctx.theme.text_muted
            };
            ui::paint_text_centered(ctx, list, &day.day.to_string(), cell, m.text, color);
        }
        y += m.cell * 6.0 + m.inner;
        let buttons = [
            ("\u{f0141}\u{f0141}", Button::Year(-1)),
            ("\u{f0141}", Button::Month(-1)),
            ("Today", Button::Today),
            ("\u{f0142}", Button::Month(1)),
            ("\u{f0142}\u{f0142}", Button::Year(1)),
        ];
        self.button_row(ctx, list, Rect { x, y, width: w, height: m.button, radius: 0.0, color: Color::TRANSPARENT }, &buttons);
    }

    fn paint_music(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, panel: Rect, music: &Value) {
        let m = self.metrics.clone();
        let mut y = Self::title(ctx, list, panel, "Now Playing", &m);
        let x = panel.x + m.inner;
        let w = panel.width - m.inner * 2.0;
        let art = Rect { x: x + (w - m.art) * 0.5, y, width: m.art, height: m.art, radius: m.radius, color: Color::TRANSPARENT };
        text_field::paint_frame(list, art, m.border, ctx.theme.border_inactive, ctx.theme.bg_main);
        let url = music["art_url"].as_str().unwrap_or_default();
        let image = url
            .strip_prefix("file://")
            .or_else(|| url.starts_with('/').then_some(url))
            .and_then(|path| ctx.icons.load_icon(path, (m.art * 2.0) as u32, 1));
        match image {
            Some(img) => list.push_image(crate::render::display_list::Image {
                width: img.width,
                height: img.height,
                pixels: img.pixels,
                x: art.x + m.border,
                y: art.y + m.border,
                width_logical: art.width - m.border * 2.0,
                height_logical: art.height - m.border * 2.0,
            }),
            None => ui::paint_text_centered(ctx, list, "\u{f0386}", art, m.art * 0.4, ctx.theme.text_muted),
        }
        y += m.art + m.inner;
        let text = |key: &str, fallback: &str| music[key].as_str().filter(|s| !s.is_empty()).unwrap_or(fallback).to_string();
        let lines = [
            (text("title", "Nothing playing"), m.text, ctx.metrics.typography.weight_bold, ctx.theme.text_main),
            (text("artist", "N/A"), m.caption, ctx.metrics.typography.weight_regular, ctx.theme.text_muted),
            (text("album", "N/A"), m.caption, ctx.metrics.typography.weight_regular, ctx.theme.text_muted),
            (
                format!("{} / {}", text("position", "00:00"), text("length", "00:00")),
                m.caption,
                ctx.metrics.typography.weight_regular,
                ctx.theme.text_muted,
            ),
        ];
        for (line, size, weight, color) in lines {
            let shown = ui::wrap_text(ctx, &line, size, weight, w, 1);
            let row = Rect { x, y, width: w, height: m.line, radius: 0.0, color: Color::TRANSPARENT };
            if let Some(shown) = shown.first() {
                let style = crate::text::TextStyle { family: std::sync::Arc::from(ctx.theme.font.as_str()), size, weight, color };
                let tw = ctx.text.measure(shown, &style).0;
                ui::paint_text_vcentered(ctx, list, shown, x + (w - tw) * 0.5, row, size, weight, color);
            }
            y += m.line;
        }
        // Progress.
        let length = music["length_sec"].as_f64().unwrap_or(0.0);
        let position = music["position_sec"].as_f64().unwrap_or(0.0);
        let track = Rect { x, y: y + m.bar * 0.5, width: w, height: m.bar, radius: m.bar * 0.5, color: ctx.theme.bg_hover };
        list.push_rect(track);
        if length > 0.0 {
            list.push_rect(Rect { width: w * (position / length).clamp(0.0, 1.0) as f32, color: ctx.theme.primary, ..track });
        }
        y += m.bar * 2.0 + m.inner;
        let playing = music["status"].as_str() == Some("Playing");
        let buttons = [
            ("\u{f04a}", Button::Previous),
            (if playing { "\u{f04c}" } else { "\u{f04b}" }, Button::PlayPause),
            ("\u{f04e}", Button::Next),
        ];
        self.button_row(ctx, list, Rect { x, y, width: w, height: m.button, radius: 0.0, color: Color::TRANSPARENT }, &buttons);
    }

    fn button_row(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, row: Rect, buttons: &[(&str, Button)]) {
        let m = self.metrics.clone();
        let n = buttons.len() as f32;
        let bw = (row.width - m.cell_gap * (n - 1.0)) / n;
        for (i, (label, button)) in buttons.iter().enumerate() {
            let rect = Rect { x: row.x + i as f32 * (bw + m.cell_gap), width: bw, radius: m.radius * 0.6, ..row };
            text_field::paint_frame(list, rect, m.border, ctx.theme.border_inactive, ctx.theme.bg_main);
            ui::paint_text_centered(ctx, list, label, rect, m.text, ctx.theme.text_main);
            self.buttons.push((rect, *button));
        }
    }
}

impl Default for Dashboard {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use smithay_client_toolkit::seat::keyboard::Keysym;

    #[test]
    fn october_2026_starts_on_a_thursday() {
        let grid = month_grid(2026, 10, (2026, 10, 22));
        // Sunday-first: Su Mo Tu We are September's 27..30.
        assert_eq!(grid[..4].iter().map(|d| d.day).collect::<Vec<_>>(), vec![27, 28, 29, 30]);
        assert!(grid[..4].iter().all(|d| !d.in_month));
        assert_eq!(grid[4], Day { day: 1, in_month: true, today: false });
        let today: Vec<_> = grid.iter().filter(|d| d.today).collect();
        assert_eq!(today.len(), 1);
        assert_eq!(today[0].day, 22);
        // 4 + 31 = 35 cells of this month and before; then November.
        assert_eq!(grid[35], Day { day: 1, in_month: false, today: false });
        assert_eq!(grid[41].day, 7);
    }

    #[test]
    fn february_in_a_leap_year() {
        let grid = month_grid(2028, 2, (2000, 1, 1));
        assert_eq!(grid.iter().filter(|d| d.in_month).count(), 29);
        assert!(grid.iter().all(|d| !d.today));
    }

    #[test]
    fn months_shift_across_years() {
        assert_eq!(shift(2026, 12, 1), (2027, 1));
        assert_eq!(shift(2026, 1, -1), (2025, 12));
        assert_eq!(shift(2026, 10, -22), (2024, 12));
        assert_eq!(shift(1, 1, -1), (1, 12), "year 1 is the floor, as in QML");
    }

    #[test]
    fn opening_navigating_and_closing() {
        let mut d = Dashboard::new();
        d.toggle("HDMI-A-1");
        assert_eq!(d.target(), Some("HDMI-A-1"));
        let now = d.month();
        assert_eq!(d.key(Keysym::Right), DashboardAction::Refresh);
        assert_eq!(d.month(), shift(now.0, now.1, 1));
        assert_eq!(d.navigate(Button::Year(-1)), DashboardAction::Refresh);
        d.navigate(Button::Today);
        assert_eq!(d.month(), now);
        d.key(Keysym::Left);
        // Closing returns to this month for next time.
        assert_eq!(d.key(Keysym::Escape), DashboardAction::Redraw);
        assert!(!d.is_open());
        assert_eq!(d.month(), now);
    }

    #[test]
    fn media_buttons_need_a_player() {
        let mut d = Dashboard::new();
        d.open_on("a");
        let rect = Rect { x: 0.0, y: 0.0, width: 10.0, height: 10.0, radius: 0.0, color: Color::TRANSPARENT };
        d.buttons = vec![(rect, Button::PlayPause)];
        let none = json!({"music": {"player": "None"}});
        assert_eq!(d.on_click(Point::new(5.0, 5.0), &none), DashboardAction::Redraw);
        assert_eq!(d.error.as_deref(), Some("No active player"));
        let spotify = json!({"music": {"player": "spotify"}});
        assert_eq!(
            d.on_click(Point::new(5.0, 5.0), &spotify),
            DashboardAction::Media { player: "spotify".into(), action: "play-pause".into() }
        );
        assert_eq!(d.error, None);
    }

    #[test]
    fn a_click_outside_the_card_closes() {
        let mut d = Dashboard::new();
        d.open_on("a");
        d.card = Some(Rect { x: 100.0, y: 10.0, width: 400.0, height: 300.0, radius: 0.0, color: Color::TRANSPARENT });
        assert_eq!(d.on_click(Point::new(200.0, 100.0), &Value::Null), DashboardAction::None);
        assert!(d.is_open());
        assert_eq!(d.on_click(Point::new(20.0, 600.0), &Value::Null), DashboardAction::Redraw);
        assert!(!d.is_open());
    }
}
