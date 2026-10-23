//! The dashboard overlay: tabs of panels (performance, calendar, now
//! playing), grown from `modules/DashboardMenu.qml`.
//!
//! - Opened by `stratum-cli dashboard open|toggle [tab]` (and `close`), on
//!   the focused output, as a card at the top centre over a dimmed screen.
//!   It always opens on the first tab unless one is named. A click outside
//!   the card or Escape closes it.
//! - Tabs come from `[[dashboard.tabs]]`: each is a row of panels sharing
//!   its width by weight. The card is the same size on every tab. Keys 1–9
//!   pick a tab, Ctrl+Tab and Ctrl+Shift+Tab step through them.
//! - Performance: CPU, each GPU (by name), RAM, and storage, each with a
//!   bar coloured by load (≥ 90 % error, ≥ 75 % warning, else success).
//! - Calendar: the month grid (Sunday first, neighbouring months' days
//!   dimmed, today marked), with «« « Today » »» for year and month, and
//!   ← → while a tab with a calendar is shown.
//! - Now playing: art, title, artist, album, position and length, and
//!   previous / play-pause / next.
//!
//! Resource data comes from the sampler in [`feed`]; now playing is
//! `SystemState::music`, which the sampler refreshes while the dashboard is
//! open. The calendar grid is computed here: the QML shell cached ±3 months
//! to hide the daemon's round trip, and computing a month locally is
//! instant. Closing resets the calendar to the current month, as QML did.

pub mod feed;

use chrono::Datelike;
use serde_json::Value;
use smithay_client_toolkit::seat::keyboard::Modifiers;

use stratum_sys::managers::gpu::Load;
use stratum_sys::managers::sysmon::{Sample, Usage};

use crate::config::{DashboardConfig, PanelKind};
use crate::render::display_list::{Color, DisplayList, Point, Rect};
use crate::ui::pages::devices::Marquee;
use crate::ui::text_field;
use crate::ui::{self, PaintCtx};

/// What the shell has to do after input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DashboardAction {
    None,
    Redraw,
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

/// `(glyph, label, detail, value, percent)` for each performance row. Before
/// the first sample, and for CPU until a second, values read "–".
fn performance_rows(sample: Option<&Sample>) -> Vec<(&'static str, String, String, String, f64)> {
    const PENDING: &str = "\u{2013}";
    let percent = |p: Option<f32>| p.map_or_else(|| PENDING.to_string(), |p| format!("{}%", p.round()));
    let gib = |bytes: u64| bytes as f64 / f64::from(1u32 << 30);
    let usage = |u: Option<&Usage>| {
        u.map_or_else(String::new, |u| format!("{:.1} / {:.1} GiB", gib(u.used), gib(u.total)))
    };
    let cpu = sample.and_then(|s| s.cpu_percent);
    let mut rows = vec![("\u{f2db}", "CPU".to_string(), String::new(), percent(cpu), f64::from(cpu.unwrap_or(0.0)))];
    // A row per GPU present: a hybrid laptop shows both, integrated-only
    // mode just the one.
    for gpu in sample.map(|s| s.gpus.as_slice()).unwrap_or_default() {
        let (text, value) = match gpu.load {
            Load::Percent(p) => (format!("{}%", p.clamp(0, 100)), p.clamp(0, 100) as f64),
            Load::Asleep => ("Asleep".to_string(), 0.0),
            Load::Unknown => ("N/A".to_string(), 0.0),
        };
        rows.push(("\u{f08ae}", "GPU".to_string(), gpu.name.clone(), text, value));
    }
    let memory = sample.map(|s| &s.memory);
    let root = sample.map(|s| &s.root);
    rows.extend([
        (
            "\u{f035b}",
            "RAM".to_string(),
            usage(memory),
            percent(memory.map(Usage::percent)),
            memory.map_or(0.0, |u| f64::from(u.percent())),
        ),
        (
            "\u{f0a0}",
            "Storage".to_string(),
            usage(root),
            percent(root.map(Usage::percent)),
            root.map_or(0.0, |u| f64::from(u.percent())),
        ),
    ]);
    rows
}

/// `row` cut into side-by-side rects, `gap` apart, sharing its width by
/// `weights`.
fn split(row: Rect, weights: &[f32], gap: f32) -> Vec<Rect> {
    let total: f32 = weights.iter().sum();
    let free = (row.width - gap * weights.len().saturating_sub(1) as f32).max(0.0);
    let mut x = row.x;
    weights
        .iter()
        .map(|w| {
            let rect = Rect { x, width: free * w / total, ..row };
            x += rect.width + gap;
            rect
        })
        .collect()
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
    /// The track title, scrolling when it overflows the panel.
    title: Marquee,
    /// The tab shown, an index into `config.tabs`, and each tab's hit rect
    /// from the last paint.
    tab: usize,
    tab_hits: Vec<(Rect, usize)>,
    metrics: crate::ui::style::DashboardMetrics,
    config: DashboardConfig,
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
    pub fn new(metrics: &crate::ui::style::DashboardMetrics, config: &DashboardConfig) -> Self {
        let (year, month, _) = today();
        Self {
            open: false,
            target: None,
            year,
            month,
            card: None,
            buttons: Vec::new(),
            error: None,
            title: Marquee::ticker(),
            tab: 0,
            tab_hits: Vec::new(),
            metrics: metrics.clone(),
            config: config.clone(),
        }
    }

    /// Whether it needs frames: an open dashboard with a scrolling title.
    pub fn animating(&self) -> bool {
        self.open && self.title.moving()
    }

    /// Follow a reload. A tab that no longer exists falls back to the first.
    pub fn set_config(&mut self, metrics: &crate::ui::style::DashboardMetrics, config: &DashboardConfig) {
        self.metrics = metrics.clone();
        if self.config.tabs != config.tabs {
            self.config = config.clone();
            if self.tab >= self.config.tabs.len() {
                self.tab = 0;
            }
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn target(&self) -> Option<&str> {
        self.open.then_some(self.target.as_deref()).flatten()
    }

    /// Open on `output`, at `tab` or the first (closing returned to it).
    /// Already open, it stays on its output and moves only to a named tab.
    pub fn open_on(&mut self, output: &str, tab: Option<usize>) {
        if !self.open {
            self.target = Some(output.to_string());
        }
        self.open = true;
        if let Some(tab) = tab.filter(|&t| t < self.config.tabs.len()) {
            self.tab = tab;
        }
        self.error = None;
    }

    /// Close, and return to the first tab and the current month for next
    /// time.
    pub fn close(&mut self) {
        self.open = false;
        self.target = None;
        self.card = None;
        self.buttons.clear();
        self.tab_hits.clear();
        self.tab = 0;
        self.title = Marquee::ticker();
        let (year, month, _) = today();
        (self.year, self.month) = (year, month);
    }

    /// Toggle with a tab named: closed, it opens there; open on another tab,
    /// it switches; open on that tab, it closes.
    pub fn toggle(&mut self, output: &str, tab: Option<usize>) {
        if self.open && tab.is_none_or(|t| t == self.tab) {
            self.close();
        } else {
            self.open_on(output, tab);
        }
    }

    fn select_tab(&mut self, tab: usize) -> DashboardAction {
        if tab >= self.config.tabs.len() || tab == self.tab {
            return DashboardAction::None;
        }
        self.tab = tab;
        self.buttons.clear();
        DashboardAction::Redraw
    }

    fn shows(&self, kind: PanelKind) -> bool {
        self.config.tabs.get(self.tab).is_some_and(|t| t.panels.iter().any(|p| p.kind == kind))
    }

    pub fn key(&mut self, keysym: smithay_client_toolkit::seat::keyboard::Keysym, modifiers: Modifiers) -> DashboardAction {
        use smithay_client_toolkit::seat::keyboard::Keysym;
        if !self.open {
            return DashboardAction::None;
        }
        let count = self.config.tabs.len();
        match keysym {
            Keysym::Escape => {
                self.close();
                DashboardAction::Redraw
            }
            // Shift+Tab is ISO_Left_Tab in most keymaps, but not in every
            // one (wtype's sends Tab with Shift held).
            Keysym::Tab | Keysym::ISO_Left_Tab if modifiers.ctrl => {
                let back = modifiers.shift || keysym == Keysym::ISO_Left_Tab;
                self.select_tab((self.tab + if back { count - 1 } else { 1 }) % count)
            }
            Keysym::Left if self.shows(PanelKind::Calendar) => self.navigate(Button::Month(-1)),
            Keysym::Right if self.shows(PanelKind::Calendar) => self.navigate(Button::Month(1)),
            other => match other.raw() {
                raw @ 0x31..=0x39 => self.select_tab((raw - 0x31) as usize),
                _ => DashboardAction::None,
            },
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
        DashboardAction::Redraw
    }

    /// A click on the dashboard's surface; `music` is `SystemState::music`.
    pub fn on_click(&mut self, point: Point, music: &Value) -> DashboardAction {
        if !self.open {
            return DashboardAction::None;
        }
        if let Some(tab) = self.tab_hits.iter().find(|(r, _)| r.contains(point)).map(|(_, t)| *t) {
            return self.select_tab(tab);
        }
        if let Some(button) = self.buttons.iter().find(|(r, _)| r.contains(point)).map(|(_, b)| *b) {
            return match button {
                Button::Previous | Button::PlayPause | Button::Next => {
                    let player = music["player"].as_str().unwrap_or("None");
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

    pub fn paint(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, surface: Rect, snapshot: &feed::Snapshot, music: &Value) {
        if !self.open {
            return;
        }
        let m = self.metrics.clone();
        list.push_rect(Rect { color: Color::rgba(0.0, 0.0, 0.0, m.scrim_alpha), ..surface });

        let width = m.width.min(surface.width - m.margin * 2.0);
        let height = m.padding * 2.0
            + m.tab_height
            + m.gap
            + m.panel_height
            + if self.error.is_some() { m.line } else { 0.0 };
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
        let bar = Rect { x: card.x + m.padding, y, width: card.width - m.padding * 2.0, height: m.tab_height, radius: 0.0, color: Color::TRANSPARENT };
        self.paint_tabs(ctx, list, bar);
        y += m.tab_height + m.gap;
        if let Some(error) = self.error.clone() {
            let row = Rect { x: card.x + m.padding, y, width: card.width, height: m.line, radius: 0.0, color: Color::TRANSPARENT };
            ui::paint_text_vcentered(ctx, list, &error, row.x, row, m.caption, ctx.metrics.typography.weight_regular, ctx.theme.error);
            y += m.line;
        }
        let Some(tab) = self.config.tabs.get(self.tab).cloned() else {
            self.card = Some(card);
            return;
        };
        let row = Rect { x: card.x + m.padding, y, width: card.width - m.padding * 2.0, height: m.panel_height, radius: m.radius, color: Color::TRANSPARENT };
        let weights: Vec<f32> = tab.panels.iter().map(|p| p.weight).collect();
        for (spec, panel) in tab.panels.iter().zip(split(row, &weights, m.gap)) {
            text_field::paint_frame(list, panel, m.border, ctx.theme.border_inactive, ctx.theme.bg_widget);
            match spec.kind {
                PanelKind::Performance => self.paint_performance(ctx, list, panel, snapshot.sample.as_ref()),
                PanelKind::Calendar => self.paint_calendar(ctx, list, panel),
                PanelKind::Music => self.paint_music(ctx, list, panel, music),
            }
        }
        self.card = Some(card);
    }

    /// The tab bar: a pill per tab, the shown one framed.
    fn paint_tabs(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, bar: Rect) {
        let m = self.metrics.clone();
        self.tab_hits.clear();
        let weight = ctx.metrics.typography.weight_medium;
        let style = crate::text::TextStyle {
            family: std::sync::Arc::from(ctx.theme.font.as_str()),
            size: m.text,
            weight,
            color: ctx.theme.text_main,
        };
        let mut x = bar.x;
        for (i, tab) in self.config.tabs.iter().enumerate() {
            let label = format!("{}  {}", tab.icon, tab.name);
            let width = ctx.text.measure(&label, &style).0 + m.inner * 2.0;
            let pill = Rect { x, width, radius: m.radius * 0.6, ..bar };
            let shown = i == self.tab;
            if shown {
                text_field::paint_frame(list, pill, m.border, ctx.theme.border_active, ctx.theme.bg_widget);
            }
            let color = if shown { ctx.theme.text_main } else { ctx.theme.text_muted };
            ui::paint_text_vcentered(ctx, list, &label, pill.x + m.inner, pill, m.text, weight, color);
            self.tab_hits.push((pill, i));
            x += width + m.tab_gap;
        }
    }

    fn title(ctx: &mut PaintCtx<'_>, list: &mut DisplayList, panel: Rect, text: &str, m: &crate::ui::style::DashboardMetrics) -> f32 {
        let row = Rect { x: panel.x + m.inner, y: panel.y + m.inner, width: panel.width - m.inner * 2.0, height: m.line * 1.2, radius: 0.0, color: Color::TRANSPARENT };
        ui::paint_text_vcentered(ctx, list, text, row.x, row, m.title, ctx.metrics.typography.weight_bold, ctx.theme.text_main);
        row.y + row.height + m.inner
    }

    fn paint_performance(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, panel: Rect, sample: Option<&Sample>) {
        let m = self.metrics.clone();
        let mut y = Self::title(ctx, list, panel, "Performance", &m);
        let x = panel.x + m.inner;
        let w = panel.width - m.inner * 2.0;
        let rows = performance_rows(sample);
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
                let weight = ctx.metrics.typography.weight_regular;
                // A GPU's name can outrun the panel.
                let detail = ui::elide(ctx, &detail, m.caption, weight, w - m.text * 1.6);
                ui::paint_text_vcentered(ctx, list, &detail, x + m.text * 1.6, sub, m.caption, weight, ctx.theme.text_muted);
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
        let row = Rect { x, y, width: w, height: m.line, radius: 0.0, color: Color::TRANSPARENT };
        self.title.paint(ctx, list, row, &text("title", "Nothing playing"), m.text, ctx.theme.text_main);
        y += m.line;
        let lines = [
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
        let mut d = dashboard();
        d.toggle("HDMI-A-1", None);
        assert_eq!(d.target(), Some("HDMI-A-1"));
        let now = (d.year, d.month);
        assert_eq!(d.key(Keysym::Right, Modifiers::default()), DashboardAction::Redraw);
        assert_eq!((d.year, d.month), shift(now.0, now.1, 1));
        assert_eq!(d.navigate(Button::Year(-1)), DashboardAction::Redraw);
        d.navigate(Button::Today);
        assert_eq!((d.year, d.month), now);
        d.key(Keysym::Left, Modifiers::default());
        // Closing returns to this month for next time.
        assert_eq!(d.key(Keysym::Escape, Modifiers::default()), DashboardAction::Redraw);
        assert!(!d.is_open());
        assert_eq!((d.year, d.month), now);
    }

    fn dashboard() -> Dashboard {
        let config = crate::config::Config::test_defaults();
        Dashboard::new(&config.metrics.dashboard, &config.dashboard)
    }

    #[test]
    fn it_opens_on_the_first_tab_unless_one_is_named() {
        let mut d = dashboard();
        let system = d.config.tab_index("system").expect("a System tab ships");
        d.open_on("a", Some(system));
        assert_eq!(d.tab, system);
        d.close();
        d.open_on("a", None);
        assert_eq!(d.tab, 0);
        // Open on another output, naming a tab only switches it.
        d.open_on("b", Some(system));
        assert_eq!((d.target(), d.tab), (Some("a"), system));
    }

    #[test]
    fn toggling_with_a_tab_switches_before_it_closes() {
        let mut d = dashboard();
        d.toggle("a", Some(1));
        assert_eq!(d.tab, 1);
        d.toggle("a", Some(2));
        assert!(d.is_open());
        assert_eq!(d.tab, 2);
        d.toggle("a", Some(2));
        assert!(!d.is_open());
        assert_eq!(d.tab, 0, "closing returns to the first tab");
    }

    #[test]
    fn keys_pick_and_step_through_tabs() {
        let mut d = dashboard();
        let count = d.config.tabs.len();
        d.open_on("a", None);
        assert_eq!(d.key(Keysym::_2, Modifiers::default()), DashboardAction::Redraw);
        assert_eq!(d.tab, 1);
        assert_eq!(d.key(Keysym::_9, Modifiers::default()), DashboardAction::None, "no ninth tab");
        let ctrl = Modifiers { ctrl: true, ..Modifiers::default() };
        let ctrl_shift = Modifiers { shift: true, ..ctrl };
        d.key(Keysym::Tab, ctrl);
        assert_eq!(d.tab, 2 % count);
        d.key(Keysym::_1, Modifiers::default());
        d.key(Keysym::ISO_Left_Tab, ctrl_shift);
        assert_eq!(d.tab, count - 1, "back from the first wraps to the last");
        d.key(Keysym::Tab, ctrl_shift);
        assert_eq!(d.tab, count - 2, "Shift+Tab may arrive as Tab with Shift held");
        assert_eq!(d.key(Keysym::Tab, Modifiers::default()), DashboardAction::None, "Tab alone is not a tab switch");
    }

    #[test]
    fn arrows_move_the_month_only_where_a_calendar_shows() {
        let mut d = dashboard();
        let system = d.config.tab_index("system").unwrap();
        d.open_on("a", Some(system));
        let now = (d.year, d.month);
        assert_eq!(d.key(Keysym::Right, Modifiers::default()), DashboardAction::None);
        assert_eq!((d.year, d.month), now);
    }

    #[test]
    fn panels_share_the_width_by_weight() {
        let row = Rect { x: 10.0, y: 0.0, width: 108.0, height: 5.0, radius: 0.0, color: Color::TRANSPARENT };
        let rects = split(row, &[1.0, 3.0], 8.0);
        assert_eq!(rects.iter().map(|r| (r.x, r.width)).collect::<Vec<_>>(), vec![(10.0, 25.0), (43.0, 75.0)]);
    }

    #[test]
    fn performance_rows_before_and_after_samples() {
        let rows = performance_rows(None);
        assert_eq!(rows.iter().map(|r| r.1.as_str()).collect::<Vec<_>>(), ["CPU", "RAM", "Storage"]);
        assert!(rows.iter().all(|r| r.3 == "\u{2013}"));
        let sample = Sample {
            cpu_percent: Some(41.6),
            memory: Usage { used: 3 << 30, total: 12 << 30 },
            root: Usage { used: 1 << 30, total: 2 << 30 },
            gpus: vec![stratum_sys::managers::gpu::GpuReading { name: "Radeon".into(), load: Load::Asleep }],
        };
        let rows = performance_rows(Some(&sample));
        assert_eq!(rows[0].3, "42%");
        assert_eq!((rows[1].2.as_str(), rows[1].3.as_str()), ("Radeon", "Asleep"));
        assert_eq!((rows[2].2.as_str(), rows[2].3.as_str()), ("3.0 / 12.0 GiB", "25%"));
        assert_eq!(rows[3].4, 50.0);
    }

    #[test]
    fn media_buttons_need_a_player() {
        let mut d = dashboard();
        d.open_on("a", None);
        let rect = Rect { x: 0.0, y: 0.0, width: 10.0, height: 10.0, radius: 0.0, color: Color::TRANSPARENT };
        d.buttons = vec![(rect, Button::PlayPause)];
        let none = json!({"player": "None"});
        assert_eq!(d.on_click(Point::new(5.0, 5.0), &none), DashboardAction::Redraw);
        assert_eq!(d.error.as_deref(), Some("No active player"));
        let spotify = json!({"player": "spotify"});
        assert_eq!(
            d.on_click(Point::new(5.0, 5.0), &spotify),
            DashboardAction::Media { player: "spotify".into(), action: "play-pause".into() }
        );
        assert_eq!(d.error, None);
    }

    #[test]
    fn a_click_on_a_tab_selects_it() {
        let mut d = dashboard();
        d.open_on("a", None);
        d.tab_hits = vec![(Rect { x: 0.0, y: 0.0, width: 10.0, height: 10.0, radius: 0.0, color: Color::TRANSPARENT }, 2)];
        assert_eq!(d.on_click(Point::new(5.0, 5.0), &Value::Null), DashboardAction::Redraw);
        assert_eq!(d.tab, 2);
    }

    #[test]
    fn a_click_outside_the_card_closes() {
        let mut d = dashboard();
        d.open_on("a", None);
        d.card = Some(Rect { x: 100.0, y: 10.0, width: 400.0, height: 300.0, radius: 0.0, color: Color::TRANSPARENT });
        assert_eq!(d.on_click(Point::new(200.0, 100.0), &Value::Null), DashboardAction::None);
        assert!(d.is_open());
        assert_eq!(d.on_click(Point::new(20.0, 600.0), &Value::Null), DashboardAction::Redraw);
        assert!(!d.is_open());
    }
}
