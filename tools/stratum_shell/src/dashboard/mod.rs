//! The dashboard overlay: tabs of panels (performance, calendar, now
//! playing), grown from `modules/DashboardMenu.qml`.
//!
//! - Opened by `stratum-cli dashboard open|toggle [tab]` (and `close`), on
//!   the focused output, as a card at the top centre over a dimmed screen.
//!   It always opens on the first tab unless one is named. A click outside
//!   the card or Escape closes it.
//! - Tabs come from `[[dashboard.tabs]]`: each is a row of panels sharing
//!   its width by weight. The card is the same size on every tab. Keys 1–9
//!   pick a tab, Ctrl+Tab and Ctrl+Shift+Tab step through them. Where their
//!   panel shows: ← → turn the month (with Shift, the year), T returns to
//!   today, Space plays/pauses (the timer where it shows), D toggles
//!   do-not-disturb, and Tab moves between the Focus tab's fields. The tab
//!   bar's far end lists the shown tab's keys.
//! - Performance: CPU, each GPU (by name), RAM, and storage, each with a
//!   bar coloured by load (≥ 90 % error, ≥ 75 % warning, else success).
//! - Calendar: the month grid (Sunday first, neighbouring months' days
//!   dimmed, today marked), with «« « Today » »» for year and month, and
//!   ← → while a tab with a calendar is shown.
//! - Now playing: art, title, artist, album, position and length, and
//!   previous / play-pause / next.
//! - Focus: to-dos, a note and a timer ([`focus`]). A focused field takes
//!   the keys, so 1–9 type there, and Escape leaves the field before it
//!   closes the dashboard.
//!
//! Resource data comes from the sampler in [`feed`]; now playing is
//! `SystemState::music`, which the sampler refreshes while the dashboard is
//! open. The calendar grid is computed here: the QML shell cached ±3 months
//! to hide the daemon's round trip, and computing a month locally is
//! instant. Closing resets the calendar to the current month, as QML did.

mod agenda;
pub mod calendar;
pub mod feed;
mod focus;
mod note;
mod system;
pub mod timer;
mod today;
pub mod weather;

use chrono::Datelike;
use serde_json::Value;
use smithay_client_toolkit::seat::keyboard::Modifiers;

use stratum_sys::managers::gpu::Load;
use stratum_sys::managers::sysmon::{Sample, Usage};

use chrono::NaiveDate;
use stratum_sys::managers::calendar::Occurrence;

use crate::config::{DashboardConfig, PanelKind, WeekStart};
use crate::render::display_list::{Color, DisplayList, Point, Rect};
use crate::ui::pages::devices::Marquee;
use crate::ui::{self, PaintCtx};

/// What the shell has to do after input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DashboardAction {
    None,
    Redraw,
    /// `previous`, `play-pause`, or `next`, for the player.
    Media { player: String, action: String },
    /// Flip do-not-disturb.
    ToggleDnd,
    /// Seek the player to this second.
    Seek { player: String, seconds: i64 },
    /// Set the player's `shuffle` or `loop`.
    MediaSet { player: String, property: &'static str, value: String },
    /// Show this player from now on, and read it at once.
    ChoosePlayer(String),
}

/// The 6×7 grid of dates for `year`/`month`, weeks starting on
/// `week_start`, padded with the neighbouring months' days.
pub fn month_grid(year: i32, month: u32, week_start: WeekStart) -> [NaiveDate; 42] {
    let first = NaiveDate::from_ymd_opt(year, month, 1).unwrap_or_default();
    let lead = match week_start {
        WeekStart::Sunday => first.weekday().num_days_from_sunday(),
        WeekStart::Monday => first.weekday().num_days_from_monday(),
    };
    let start = first - chrono::Days::new(lead.into());
    std::array::from_fn(|i| start + chrono::Days::new(i as u64))
}

/// Weekday labels from `week_start`.
fn weekday_labels(week_start: WeekStart) -> [&'static str; 7] {
    let mut labels = WEEKDAYS;
    if week_start == WeekStart::Monday {
        labels.rotate_left(1);
    }
    labels
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

/// `body` cut into stacked rows, `gap` apart, sharing its height by
/// `weights`.
fn split_down(body: Rect, weights: &[f32], gap: f32) -> Vec<Rect> {
    // The same cut, across the transposed rect.
    let across = Rect { x: body.y, y: body.x, width: body.height, height: body.width, ..body };
    split(across, weights, gap)
        .into_iter()
        .map(|r| Rect { x: body.x, y: r.x, width: body.width, height: r.width, ..body })
        .collect()
}

fn today() -> NaiveDate {
    chrono::Local::now().date_naive()
}

/// The loop mode after `current`: off, the playlist, the track, off again.
fn next_loop(current: &str) -> &'static str {
    match current {
        "Playlist" => "Track",
        "Track" => "None",
        _ => "Playlist",
    }
}

/// The player after `current` in now playing's list, wrapping; `None`
/// with fewer than two.
fn next_player(music: &Value, current: &str) -> Option<String> {
    let ids: Vec<&str> = music["players"].as_array()?.iter().filter_map(|p| p["id"].as_str()).collect();
    if ids.len() < 2 {
        return None;
    }
    let at = ids.iter().position(|id| *id == current).map_or(0, |i| i + 1);
    Some(ids[at % ids.len()].to_string())
}

/// The shown player's name, numbered when another player has the same one
/// (two mpv windows are "Mpv 1" and "Mpv 2").
fn player_label(music: &Value) -> String {
    let current = music["player"].as_str().unwrap_or_default();
    let name = music["player_title"].as_str().unwrap_or_default().to_string();
    let players = music["players"].as_array().map(Vec::as_slice).unwrap_or_default();
    let namesakes: Vec<&str> =
        players.iter().filter(|p| p["name"].as_str() == Some(name.as_str())).filter_map(|p| p["id"].as_str()).collect();
    match namesakes.iter().position(|id| *id == current) {
        Some(i) if namesakes.len() > 1 => format!("{name} {}", i + 1),
        _ => name,
    }
}

/// A calendar's colour: feeds take the theme's accents in turn.
fn calendar_color(ctx: &PaintCtx<'_>, calendar: usize) -> Color {
    let accents = [ctx.theme.primary, ctx.theme.secondary, ctx.theme.tertiary, ctx.theme.success, ctx.theme.warning];
    accents[calendar % accents.len()]
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
    /// `[metrics.dashboard]` as configured, and scaled for the surface
    /// being painted (`screen_fraction`), which is what painting reads.
    base: crate::ui::style::DashboardMetrics,
    metrics: crate::ui::style::DashboardMetrics,
    config: DashboardConfig,
    /// The day the agenda starts from: today until a day is clicked.
    selected: NaiveDate,
    /// The player picked with the player chip; `None` follows whichever
    /// plays. Kept across openings: it falls back by itself when that player
    /// goes away.
    player: Option<String>,
    /// Expanded events by date range, for the calendar generation they came
    /// from. Expanding a long-running series walks every instance, so it is
    /// done once per range rather than on every paint.
    occurrences: (u64, std::collections::HashMap<(NaiveDate, NaiveDate), Vec<Occurrence>>),
    /// The Focus tab's to-dos, note and timer, which outlive closing.
    focus: focus::Focus,
    /// Playing after a play/pause press, until the player reports it: read
    /// straight after the command, it usually still says the old status.
    playing: crate::ui::pages::Optimistic<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Button {
    Month(i32),
    Today,
    /// A day in the month grid.
    Day(NaiveDate),
    Previous,
    PlayPause,
    Next,
    /// The glance strip's do-not-disturb bell.
    Dnd,
    /// The progress bar: where it is clicked is where to seek.
    Seek,
    Shuffle,
    Loop,
    /// The player chip, which steps to the next player.
    NextPlayer,
    /// The Focus tab: a to-do's checkbox and text, its delete button, the
    /// to-do input, clearing the done ones, and the note.
    TodoToggle(usize),
    TodoDelete(usize),
    TodoInput,
    ClearDone,
    NoteArea,
    /// The timer: a countdown of these minutes, the other modes, and the
    /// controls of a set timer.
    TimerPreset(u64),
    Stopwatch,
    Pomodoro,
    TimerToggle,
    TimerReset,
    TimerAdd,
    TimerStop,
}

impl Dashboard {
    pub fn new(metrics: &crate::ui::style::DashboardMetrics, config: &DashboardConfig) -> Self {
        let today = today();
        Self {
            open: false,
            target: None,
            year: today.year(),
            month: today.month(),
            selected: today,
            player: None,
            occurrences: (0, std::collections::HashMap::new()),
            focus: focus::Focus::default(),
            playing: crate::ui::pages::Optimistic::new(),
            card: None,
            buttons: Vec::new(),
            error: None,
            title: Marquee::ticker(),
            tab: 0,
            tab_hits: Vec::new(),
            base: metrics.clone(),
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
        self.base = metrics.clone();
        self.config = config.clone();
        if self.tab >= self.config.tabs.len() {
            self.tab = 0;
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The player picked on the dashboard, for the sampler.
    pub fn player(&self) -> Option<&str> {
        self.player.as_deref()
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

    /// Close, and return to the first tab, the current month and today for
    /// next time.
    pub fn close(&mut self) {
        self.open = false;
        self.target = None;
        self.card = None;
        self.buttons.clear();
        self.tab_hits.clear();
        self.tab = 0;
        self.title = Marquee::ticker();
        self.focus.field = None;
        let today = today();
        (self.year, self.month, self.selected) = (today.year(), today.month(), today);
        crate::memory::release_freed();
    }

    /// The events overlapping `from`..`to`, expanded once per calendar
    /// generation.
    fn occurrences(&mut self, ctx: &PaintCtx<'_>, from: NaiveDate, to: NaiveDate) -> Vec<Occurrence> {
        let state = ctx.state.calendar.get();
        if self.occurrences.0 != state.generation {
            self.occurrences = (state.generation, std::collections::HashMap::new());
        }
        self.occurrences
            .1
            .entry((from, to))
            .or_insert_with(|| stratum_sys::managers::calendar::occurrences(&state.events, from, to))
            .clone()
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
        self.focus.field = None;
        DashboardAction::Redraw
    }

    fn shows(&self, kind: PanelKind) -> bool {
        self.config.tabs.get(self.tab).is_some_and(|t| t.rows.iter().flat_map(|r| &r.panels).any(|p| p.kind == kind))
    }

    /// A key; `text` is what it types, for a focused Focus-tab field.
    pub fn key(
        &mut self,
        keysym: smithay_client_toolkit::seat::keyboard::Keysym,
        text: Option<&str>,
        modifiers: Modifiers,
        music: &Value,
    ) -> DashboardAction {
        use smithay_client_toolkit::seat::keyboard::Keysym;
        if !self.open {
            return DashboardAction::None;
        }
        let count = self.config.tabs.len();
        // A focused field types everything but Tab, which moves between
        // fields (Ctrl+Tab between tabs, leaving the field).
        // Shift+Tab is ISO_Left_Tab in most keymaps, but not in every one
        // (wtype's sends Tab with Shift held).
        let tab_key = matches!(keysym, Keysym::Tab | Keysym::ISO_Left_Tab);
        let back = modifiers.shift || keysym == Keysym::ISO_Left_Tab;
        if !tab_key {
            if let Some(action) = self.focus_key(keysym, text, modifiers.ctrl) {
                return action;
            }
        }
        let plain = !modifiers.ctrl && !modifiers.alt && !modifiers.logo;
        match keysym {
            Keysym::Escape => {
                self.close();
                DashboardAction::Redraw
            }
            _ if tab_key && modifiers.ctrl => self.select_tab((self.tab + if back { count - 1 } else { 1 }) % count),
            _ if tab_key => self.next_field(back),
            // A year with Shift, as «« »» do.
            Keysym::Left if self.shows(PanelKind::Calendar) => {
                self.navigate(Button::Month(if modifiers.shift { -12 } else { -1 }))
            }
            Keysym::Right if self.shows(PanelKind::Calendar) => {
                self.navigate(Button::Month(if modifiers.shift { 12 } else { 1 }))
            }
            Keysym::t if plain && [PanelKind::Calendar, PanelKind::Agenda, PanelKind::Day].iter().any(|&k| self.shows(k)) => {
                self.navigate(Button::Today)
            }
            Keysym::d if plain && self.shows(PanelKind::Glance) => DashboardAction::ToggleDnd,
            // The timer where it shows, else the player.
            Keysym::space if plain && self.shows(PanelKind::Timer) => self.press(Button::TimerToggle, 0.0, music),
            Keysym::space if plain && self.shows(PanelKind::Music) => self.press(Button::PlayPause, 0.0, music),
            other => match other.raw() {
                raw @ 0x31..=0x39 => self.select_tab((raw - 0x31) as usize),
                _ => DashboardAction::None,
            },
        }
    }

    fn navigate(&mut self, button: Button) -> DashboardAction {
        match button {
            Button::Month(delta) => (self.year, self.month) = shift(self.year, self.month, delta),
            Button::Today => self.pick(today()),
            // A neighbouring month's day turns the page to it.
            Button::Day(date) => self.pick(date),
            _ => return DashboardAction::None,
        }
        DashboardAction::Redraw
    }

    fn pick(&mut self, date: NaiveDate) {
        (self.year, self.month, self.selected) = (date.year(), date.month(), date);
    }

    /// A click on the dashboard's surface; `music` is `SystemState::music`.
    pub fn on_click(&mut self, point: Point, music: &Value) -> DashboardAction {
        if !self.open {
            return DashboardAction::None;
        }
        if let Some(tab) = self.tab_hits.iter().find(|(r, _)| r.contains(point)).map(|(_, t)| *t) {
            return self.select_tab(tab);
        }
        // Later buttons are drawn over earlier ones (a to-do's delete over
        // its row), so the last hit wins.
        let hit = self.buttons.iter().rev().find(|(r, _)| r.contains(point)).copied();
        // Anything but a field takes the focus away from the field.
        if !matches!(hit, Some((_, Button::TodoInput | Button::NoteArea))) {
            self.focus.field = None;
        }
        if let Some((rect, button)) = hit {
            // Where along the button: a seek goes there.
            let fraction = crate::ui::controls::fraction_in(rect, point.x);
            return self.press(button, fraction, music);
        }
        if self.card.is_some_and(|card| !card.contains(point)) {
            self.close();
            return DashboardAction::Redraw;
        }
        DashboardAction::None
    }

    /// A button, clicked `fraction` of the way across or pressed from the
    /// keyboard.
    fn press(&mut self, button: Button, fraction: f32, music: &Value) -> DashboardAction {
        let player = music["player"].as_str().unwrap_or("None").to_string();
        let media = matches!(
            button,
            Button::Previous | Button::PlayPause | Button::Next | Button::Seek | Button::Shuffle | Button::Loop
        );
        if media && matches!(player.as_str(), "" | "None" | "N/A") {
            self.error = Some("No active player".into());
            return DashboardAction::Redraw;
        }
        self.error = None;
        match button {
            Button::Previous | Button::PlayPause | Button::Next => {
                let action = match button {
                    Button::Previous => "previous",
                    Button::Next => "next",
                    _ => {
                        self.playing.set(music["status"].as_str() != Some("Playing"));
                        "play-pause"
                    }
                };
                DashboardAction::Media { player, action: action.to_string() }
            }
            Button::Seek => {
                let length = music["length_sec"].as_f64().unwrap_or(0.0);
                DashboardAction::Seek { player, seconds: (f64::from(fraction) * length).round() as i64 }
            }
            Button::Shuffle => DashboardAction::MediaSet { player, property: "shuffle", value: "Toggle".into() },
            Button::Loop => {
                let next = next_loop(music["loop"].as_str().unwrap_or_default());
                DashboardAction::MediaSet { player, property: "loop", value: next.into() }
            }
            Button::NextPlayer => match next_player(music, &player) {
                Some(next) => {
                    self.player = Some(next.clone());
                    DashboardAction::ChoosePlayer(next)
                }
                None => DashboardAction::None,
            },
            Button::Dnd => DashboardAction::ToggleDnd,
            other => self.focus_click(other).unwrap_or_else(|| self.navigate(other)),
        }
    }

    pub fn paint(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, surface: Rect, snapshot: &feed::Snapshot, music: &Value) {
        if !self.open {
            return;
        }
        let scale = self.base.scale_for(surface.width, surface.height, self.config.screen_fraction);
        self.metrics = self.base.scaled(scale);
        let m = self.metrics.clone();
        ui::paint_scrim(list, surface, m.scrim_alpha);

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
        ui::paint_frame(list, card, m.border, ctx.theme.border_active, ctx.theme.bg_main);
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
        let body = Rect { x: card.x + m.padding, y, width: card.width - m.padding * 2.0, height: m.panel_height, radius: m.radius, color: Color::TRANSPARENT };
        let row_weights: Vec<f32> = tab.rows.iter().map(|r| r.weight).collect();
        for (spec, row) in tab.rows.iter().zip(split_down(body, &row_weights, m.gap)) {
            let weights: Vec<f32> = spec.panels.iter().map(|p| p.weight).collect();
            for (panel_spec, panel) in spec.panels.iter().zip(split(row, &weights, m.gap)) {
                ui::paint_frame(list, panel, m.border, ctx.theme.border_inactive, ctx.theme.bg_widget);
                match panel_spec.kind {
                    PanelKind::Glance => self.paint_glance(ctx, list, panel, snapshot.sample.as_ref()),
                    PanelKind::Weather => self.paint_weather(ctx, list, panel),
                    PanelKind::Agenda => self.paint_agenda(ctx, list, panel, false),
                    PanelKind::Day => self.paint_agenda(ctx, list, panel, true),
                    PanelKind::Performance => self.paint_performance(ctx, list, panel, snapshot.sample.as_ref()),
                    PanelKind::Graphs => self.paint_graphs(ctx, list, panel, snapshot),
                    PanelKind::Calendar => self.paint_calendar(ctx, list, panel),
                    PanelKind::Music => self.paint_music(ctx, list, panel, music),
                    PanelKind::Todo => self.paint_todo(ctx, list, panel),
                    PanelKind::Note => self.paint_note(ctx, list, panel),
                    PanelKind::Timer => self.paint_timer(ctx, list, panel),
                }
            }
        }
        self.painted_timer(std::time::Instant::now());
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
                ui::paint_frame(list, pill, m.border, ctx.theme.border_active, ctx.theme.bg_widget);
            }
            let color = if shown { ctx.theme.text_main } else { ctx.theme.text_muted };
            ui::paint_text_vcentered(ctx, list, &label, pill.x + m.inner, pill, m.text, weight, color);
            self.tab_hits.push((pill, i));
            x += width + m.tab_gap;
        }
        // The shown tab's keys, at the far end while there is room.
        let hint = self.key_hint();
        let regular = ctx.metrics.typography.weight_regular;
        let hint_w = today::measure(ctx, &hint, m.caption, regular);
        if x + m.inner + hint_w <= bar.x + bar.width {
            ui::paint_text_vcentered(ctx, list, &hint, bar.x + bar.width - hint_w, bar, m.caption, regular, ctx.theme.text_muted);
        }
    }

    /// The keys that do something on the shown tab, shortest first to last.
    fn key_hint(&self) -> String {
        let mut keys = Vec::new();
        if self.shows(PanelKind::Calendar) {
            keys.push("\u{2190}\u{2192} month \u{b7} \u{21e7} year");
        }
        if [PanelKind::Calendar, PanelKind::Agenda, PanelKind::Day].iter().any(|&k| self.shows(k)) {
            keys.push("T today");
        }
        if self.shows(PanelKind::Todo) || self.shows(PanelKind::Note) {
            keys.push("Tab write");
        }
        if self.shows(PanelKind::Timer) {
            keys.push("Space timer");
        } else if self.shows(PanelKind::Music) {
            keys.push("Space play");
        }
        if self.shows(PanelKind::Glance) {
            keys.push("D quiet");
        }
        keys.push("Esc close");
        keys.join(" \u{b7} ")
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
        let calendar = self.config.calendar.clone();
        // An extra, leading column for week numbers.
        let columns = if calendar.week_numbers { 8 } else { 7 };
        let cell_w = w / columns as f32;
        let first_col = columns - 7;
        for (i, label) in weekday_labels(calendar.week_start).iter().enumerate() {
            let cell = Rect { x: x + cell_w * (i + first_col) as f32, y, width: cell_w, height: m.line, radius: 0.0, color: Color::TRANSPARENT };
            ui::paint_text_centered(ctx, list, label, cell, m.caption, ctx.theme.text_muted);
        }
        y += m.line;
        // The rows share what the navigation row leaves.
        let buttons_y = panel.y + panel.height - m.inner - m.button;
        let cell_h = ((buttons_y - m.inner - y) / 6.0).max(m.cell);
        let grid = month_grid(self.year, self.month, calendar.week_start);
        let today = today();
        let events = self.occurrences(ctx, grid[0], grid[41] + chrono::Days::new(1));
        for (i, date) in grid.iter().enumerate() {
            let (row, col) = (i / 7, i % 7 + first_col);
            let row_y = y + cell_h * row as f32;
            if first_col == 1 && i % 7 == 0 {
                // ISO weeks start on Monday: number the row by its Monday.
                let monday = grid[row * 7] + chrono::Days::new(u64::from(calendar.week_start == WeekStart::Sunday));
                let label = Rect { x, y: row_y, width: cell_w, height: cell_h, radius: 0.0, color: Color::TRANSPARENT };
                ui::paint_text_centered(ctx, list, &monday.iso_week().week().to_string(), label, m.caption, ctx.theme.text_muted);
            }
            let cell = Rect {
                x: x + cell_w * col as f32 + m.cell_gap * 0.5,
                y: row_y + m.cell_gap * 0.5,
                width: cell_w - m.cell_gap,
                height: cell_h - m.cell_gap,
                radius: m.radius * 0.6,
                color: Color::TRANSPARENT,
            };
            let in_month = date.month() == self.month;
            let selected = *date == self.selected;
            ui::paint_frame(
                list,
                cell,
                m.border,
                if selected { ctx.theme.border_active } else { ctx.theme.border_inactive },
                if selected { ctx.theme.bg_hover } else if in_month { ctx.theme.bg_widget } else { ctx.theme.bg_main },
            );
            let color = if *date == today {
                ctx.theme.primary
            } else if in_month {
                ctx.theme.text_main
            } else {
                ctx.theme.text_muted
            };
            let number = Rect { height: cell.height - m.dot * 2.0, ..cell };
            ui::paint_text_centered(ctx, list, &date.day().to_string(), number, m.text, color);
            // A dot per calendar with something on this day, up to three.
            let start = date.and_time(chrono::NaiveTime::MIN);
            let end = start + chrono::Duration::days(1);
            let mut calendars: Vec<usize> = events
                .iter()
                .filter(|o| o.start < end && (o.end > start || o.start == o.end && o.start >= start))
                .map(|o| o.calendar)
                .collect();
            calendars.dedup();
            calendars.truncate(3);
            let dots_w = calendars.len() as f32 * m.dot * 2.0 - m.dot;
            for (k, calendar) in calendars.into_iter().enumerate() {
                list.push_rect(Rect {
                    x: cell.x + (cell.width - dots_w) * 0.5 + k as f32 * m.dot * 2.0,
                    y: cell.y + cell.height - m.dot * 2.0,
                    width: m.dot,
                    height: m.dot,
                    radius: m.dot * 0.5,
                    color: calendar_color(ctx, calendar),
                });
            }
            self.buttons.push((cell, Button::Day(*date)));
        }
        let buttons = [("\u{f0141}", Button::Month(-1), false), ("Today", Button::Today, false), ("\u{f0142}", Button::Month(1), false)];
        self.button_row(ctx, list, Rect { x, y: buttons_y, width: w, height: m.button, radius: 0.0, color: Color::TRANSPARENT }, &buttons);
    }

    fn paint_music(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, panel: Rect, music: &Value) {
        let m = self.metrics.clone();
        let mut y = Self::title(ctx, list, panel, "Now Playing", &m);
        let x = panel.x + m.inner;
        let w = panel.width - m.inner * 2.0;
        let art = Rect { x: x + (w - m.art) * 0.5, y, width: m.art, height: m.art, radius: m.radius, color: Color::TRANSPARENT };
        ui::paint_frame(list, art, m.border, ctx.theme.border_inactive, ctx.theme.bg_main);
        // `art_path` is the art as a local file, remote covers included.
        let image = music["art_path"]
            .as_str()
            .filter(|path| !path.is_empty())
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
        // Progress, which a click seeks along. Its hit area is a whole line
        // tall: the bar alone is too thin to aim at.
        let length = music["length_sec"].as_f64().unwrap_or(0.0);
        let position = music["position_sec"].as_f64().unwrap_or(0.0);
        let track = Rect { x, y: y + m.bar * 0.5, width: w, height: m.bar, radius: m.bar * 0.5, color: ctx.theme.bg_hover };
        list.push_rect(track);
        if length > 0.0 {
            list.push_rect(Rect { width: w * (position / length).clamp(0.0, 1.0) as f32, color: ctx.theme.primary, ..track });
            let hit = Rect { y: track.y + track.height * 0.5 - m.line * 0.5, height: m.line, ..track };
            self.buttons.push((hit, Button::Seek));
        }
        y += m.bar * 2.0 + m.inner;
        self.playing.tick(ctx.dt);
        let playing = self.playing.view(music["status"].as_str() == Some("Playing"));
        // Shuffle and loop only for players that have them; lit while on.
        let shuffle = music["shuffle"].as_str().unwrap_or_default();
        let repeat = music["loop"].as_str().unwrap_or_default();
        let mut buttons = Vec::new();
        if !shuffle.is_empty() {
            buttons.push(("\u{f049d}", Button::Shuffle, shuffle == "On"));
        }
        buttons.extend([
            ("\u{f04a}", Button::Previous, false),
            (if playing { "\u{f04c}" } else { "\u{f04b}" }, Button::PlayPause, false),
            ("\u{f04e}", Button::Next, false),
        ]);
        if !repeat.is_empty() {
            let glyph = if repeat == "Track" { "\u{f0458}" } else { "\u{f0456}" };
            buttons.push((glyph, Button::Loop, repeat != "None"));
        }
        self.button_row(ctx, list, Rect { x, y, width: w, height: m.button, radius: 0.0, color: Color::TRANSPARENT }, &buttons);

        // With more than one player, the title row says which is shown, and
        // a click steps to the next.
        let players = music["players"].as_array().map_or(0, Vec::len);
        if players > 1 {
            let label = format!("{} \u{f0140}", player_label(music));
            let weight = ctx.metrics.typography.weight_medium;
            let style = crate::text::TextStyle { family: std::sync::Arc::from(ctx.theme.font.as_str()), size: m.caption, weight, color: ctx.theme.text_muted };
            let lw = ctx.text.measure(&label, &style).0;
            let chip = Rect { x: x + w - lw - m.inner, y: panel.y + m.inner, width: lw + m.inner, height: m.line * 1.2, radius: m.chip_radius, color: Color::TRANSPARENT };
            ui::paint_frame(list, chip, m.border, ctx.theme.border_inactive, ctx.theme.bg_main);
            ui::paint_text_centered(ctx, list, &label, chip, m.caption, ctx.theme.text_muted);
            self.buttons.push((chip, Button::NextPlayer));
        }
    }

    /// Buttons side by side; a lit one carries the accent.
    fn button_row(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, row: Rect, buttons: &[(&str, Button, bool)]) {
        let m = self.metrics.clone();
        let n = buttons.len() as f32;
        let bw = (row.width - m.cell_gap * (n - 1.0)) / n;
        for (i, (label, button, lit)) in buttons.iter().enumerate() {
            let rect = Rect { x: row.x + i as f32 * (bw + m.cell_gap), width: bw, radius: m.radius * 0.6, ..row };
            let border = if *lit { ctx.theme.border_active } else { ctx.theme.border_inactive };
            ui::paint_frame(list, rect, m.border, border, ctx.theme.bg_main);
            let color = if *lit { ctx.theme.primary } else { ctx.theme.text_main };
            ui::paint_text_centered(ctx, list, label, rect, m.text, color);
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
        let date = |m, d| NaiveDate::from_ymd_opt(2026, m, d).unwrap();
        let grid = month_grid(2026, 10, WeekStart::Sunday);
        // Sunday-first: Su Mo Tu We are September's 27..30.
        assert_eq!(grid[..5], [date(9, 27), date(9, 28), date(9, 29), date(9, 30), date(10, 1)]);
        // 4 + 31 = 35 cells of this month and before; then November.
        assert_eq!((grid[35], grid[41]), (date(11, 1), date(11, 7)));
        // Monday-first, one fewer of September's days leads.
        let monday = month_grid(2026, 10, WeekStart::Monday);
        assert_eq!((monday[0], monday[3]), (date(9, 28), date(10, 1)));
        assert_eq!(weekday_labels(WeekStart::Monday)[0], "Mo");
    }

    #[test]
    fn february_in_a_leap_year() {
        let grid = month_grid(2028, 2, WeekStart::Sunday);
        assert_eq!(grid.iter().filter(|d| d.month() == 2).count(), 29);
    }

    #[test]
    fn clicking_a_day_picks_it_and_turns_to_its_month() {
        let mut d = dashboard();
        d.open_on("a", None);
        let elsewhere = NaiveDate::from_ymd_opt(2027, 3, 14).unwrap();
        d.navigate(Button::Day(elsewhere));
        assert_eq!((d.selected, d.year, d.month), (elsewhere, 2027, 3));
        d.navigate(Button::Today);
        assert_eq!(d.selected, today());
        d.navigate(Button::Day(elsewhere));
        d.close();
        assert_eq!(d.selected, today(), "closing returns to today");
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
        let calendar = d.config.tab_index("calendar").expect("a Calendar tab ships");
        d.toggle("HDMI-A-1", Some(calendar));
        assert_eq!(d.target(), Some("HDMI-A-1"));
        let now = (d.year, d.month);
        assert_eq!(d.key(Keysym::Right, None, Modifiers::default(), &Value::Null), DashboardAction::Redraw);
        assert_eq!((d.year, d.month), shift(now.0, now.1, 1));
        assert_eq!(d.navigate(Button::Month(-3)), DashboardAction::Redraw);
        d.navigate(Button::Today);
        assert_eq!((d.year, d.month), now);
        d.key(Keysym::Left, None, Modifiers::default(), &Value::Null);
        // Closing returns to this month for next time.
        assert_eq!(d.key(Keysym::Escape, None, Modifiers::default(), &Value::Null), DashboardAction::Redraw);
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
        assert_eq!(d.key(Keysym::_2, Some("2"), Modifiers::default(), &Value::Null), DashboardAction::Redraw);
        assert_eq!(d.tab, 1);
        assert_eq!(d.key(Keysym::_9, Some("9"), Modifiers::default(), &Value::Null), DashboardAction::None, "no ninth tab");
        let ctrl = Modifiers { ctrl: true, ..Modifiers::default() };
        let ctrl_shift = Modifiers { shift: true, ..ctrl };
        d.key(Keysym::Tab, None, ctrl, &Value::Null);
        assert_eq!(d.tab, 2 % count);
        d.key(Keysym::_1, Some("1"), Modifiers::default(), &Value::Null);
        d.key(Keysym::ISO_Left_Tab, None, ctrl_shift, &Value::Null);
        assert_eq!(d.tab, count - 1, "back from the first wraps to the last");
        d.key(Keysym::Tab, None, ctrl_shift, &Value::Null);
        assert_eq!(d.tab, count - 2, "Shift+Tab may arrive as Tab with Shift held");
        assert_eq!(d.key(Keysym::Tab, None, Modifiers::default(), &Value::Null), DashboardAction::None, "Tab alone is not a tab switch");
    }

    #[test]
    fn arrows_move_the_month_only_where_a_calendar_shows() {
        let mut d = dashboard();
        let system = d.config.tab_index("system").unwrap();
        d.open_on("a", Some(system));
        let now = (d.year, d.month);
        assert_eq!(d.key(Keysym::Right, None, Modifiers::default(), &Value::Null), DashboardAction::None);
        assert_eq!((d.year, d.month), now);
    }

    #[test]
    fn a_focused_field_types_digits_and_escape_leaves_it_first() {
        let mut d = dashboard();
        let focus = d.config.tab_index("focus").expect("a Focus tab ships");
        d.open_on("a", Some(focus));
        let rect = Rect { x: 0.0, y: 0.0, width: 10.0, height: 10.0, radius: 0.0, color: Color::TRANSPARENT };
        d.buttons = vec![(rect, Button::NoteArea)];
        d.on_click(Point::new(5.0, 5.0), &Value::Null);
        assert_eq!(d.key(Keysym::_1, Some("1"), Modifiers::default(), &Value::Null), DashboardAction::Redraw);
        assert_eq!((d.tab, d.focus.note.text()), (focus, "1"), "typed, not a tab switch");
        d.key(Keysym::Escape, None, Modifiers::default(), &Value::Null);
        assert!(d.is_open(), "the first Escape leaves the field");
        d.key(Keysym::Escape, None, Modifiers::default(), &Value::Null);
        assert!(!d.is_open());
    }

    #[test]
    fn ctrl_tab_leaves_a_focused_field() {
        let mut d = dashboard();
        d.open_on("a", None);
        let rect = Rect { x: 0.0, y: 0.0, width: 10.0, height: 10.0, radius: 0.0, color: Color::TRANSPARENT };
        d.buttons = vec![(rect, Button::TodoInput)];
        d.on_click(Point::new(5.0, 5.0), &Value::Null);
        d.key(Keysym::Tab, None, Modifiers { ctrl: true, ..Modifiers::default() }, &Value::Null);
        assert_eq!((d.tab, d.focus.field), (1, None));
    }

    #[test]
    fn tab_moves_between_the_focus_fields() {
        let mut d = dashboard();
        let focus = d.config.tab_index("focus").unwrap();
        d.open_on("a", Some(focus));
        let none = Modifiers::default();
        d.key(Keysym::Tab, None, none, &Value::Null);
        assert_eq!(d.focus.field, Some(focus::Field::Todo));
        d.key(Keysym::Tab, None, none, &Value::Null);
        assert_eq!(d.focus.field, Some(focus::Field::Note), "Tab leaves a field");
        d.key(Keysym::Tab, None, Modifiers { shift: true, ..none }, &Value::Null);
        assert_eq!(d.focus.field, Some(focus::Field::Todo), "Shift+Tab goes back");
        let system = d.config.tab_index("system").unwrap();
        d.select_tab(system);
        assert_eq!(d.key(Keysym::Tab, None, none, &Value::Null), DashboardAction::None, "no fields there");
    }

    #[test]
    fn letter_keys_act_where_their_panel_shows() {
        let mut d = dashboard();
        let none = Modifiers::default();
        d.open_on("a", None);
        assert_eq!(d.key(Keysym::d, Some("d"), none, &Value::Null), DashboardAction::ToggleDnd, "the glance strip");
        let spotify = json!({"player": "spotify"});
        assert_eq!(
            d.key(Keysym::space, Some(" "), none, &spotify),
            DashboardAction::Media { player: "spotify".into(), action: "play-pause".into() }
        );
        let calendar = d.config.tab_index("calendar").unwrap();
        d.select_tab(calendar);
        assert_eq!(d.key(Keysym::d, Some("d"), none, &Value::Null), DashboardAction::None, "no glance strip");
        let now = (d.year, d.month);
        d.key(Keysym::Right, None, Modifiers { shift: true, ..none }, &Value::Null);
        assert_eq!((d.year, d.month), (now.0 + 1, now.1), "Shift moves a year");
        d.key(Keysym::t, Some("t"), none, &Value::Null);
        assert_eq!((d.year, d.month, d.selected), (now.0, now.1, today()));
        let focus = d.config.tab_index("focus").unwrap();
        d.select_tab(focus);
        d.focus.timer.start_stopwatch(std::time::Instant::now());
        d.key(Keysym::space, Some(" "), none, &Value::Null);
        assert!(!d.focus.timer.running(), "Space pauses the timer on its tab");
    }

    #[test]
    fn the_card_scales_to_its_share_of_the_screen() {
        let mut m = crate::config::Config::test_defaults().metrics.dashboard;
        (m.width, m.margin, m.padding, m.tab_height, m.gap, m.panel_height) = (800.0, 10.0, 10.0, 30.0, 10.0, 400.0);
        // Card height 460, plus margins 480.
        assert_eq!(m.scale_for(2000.0, 2000.0, 0.6), 1.5, "1200 of 2000 wide");
        assert_eq!(m.scale_for(4000.0, 960.0, 0.6), 2.0, "3 across, held to 2 by the height");
        assert_eq!(m.scale_for(1000.0, 800.0, 0.6), 1.0, "never smaller than configured");
        let scaled = m.scaled(2.0);
        assert_eq!((scaled.width, scaled.text, scaled.scrim_alpha), (1600.0, m.text * 2.0, m.scrim_alpha));
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
            network: None,
            temperatures: Default::default(),
            uptime: None,
            gpus: vec![stratum_sys::managers::gpu::GpuReading {
                name: "Radeon".into(),
                load: Load::Asleep,
                temperature: None,
                vram: None,
            }],
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
        assert!(d.playing.view(false), "shown playing before the player says so");
        assert!(d.playing.view(true));
        assert_eq!(d.playing.pending(), None, "dropped once the player agrees");
    }

    #[test]
    fn a_click_on_the_progress_bar_seeks_there() {
        let mut d = dashboard();
        d.open_on("a", None);
        let bar = Rect { x: 100.0, y: 0.0, width: 200.0, height: 10.0, radius: 0.0, color: Color::TRANSPARENT };
        d.buttons = vec![(bar, Button::Seek)];
        let music = json!({"player": "mpv", "length_sec": 240});
        assert_eq!(d.on_click(Point::new(150.0, 5.0), &music), DashboardAction::Seek { player: "mpv".into(), seconds: 60 });
    }

    #[test]
    fn loop_cycles_and_shuffle_toggles() {
        assert_eq!((next_loop("None"), next_loop("Playlist"), next_loop("Track")), ("Playlist", "Track", "None"));
        let mut d = dashboard();
        d.open_on("a", None);
        let rect = Rect { x: 0.0, y: 0.0, width: 10.0, height: 10.0, radius: 0.0, color: Color::TRANSPARENT };
        d.buttons = vec![(rect, Button::Shuffle)];
        let music = json!({"player": "mpv", "shuffle": "Off"});
        assert_eq!(
            d.on_click(Point::new(5.0, 5.0), &music),
            DashboardAction::MediaSet { player: "mpv".into(), property: "shuffle", value: "Toggle".into() }
        );
        assert_eq!(d.on_click(Point::new(5.0, 5.0), &json!({"player": "None"})), DashboardAction::Redraw, "no player");
    }

    #[test]
    fn the_player_chip_steps_through_players_and_is_remembered() {
        let music = json!({"player": "b", "players": [{"id": "a"}, {"id": "b"}, {"id": "c"}]});
        assert_eq!(next_player(&music, "b").as_deref(), Some("c"));
        assert_eq!(next_player(&music, "c").as_deref(), Some("a"), "wrapping");
        assert_eq!(next_player(&json!({"players": [{"id": "a"}]}), "a"), None);
        let twins = json!({"player": "mpv.2", "player_title": "Mpv", "players": [
            {"id": "mpv.1", "name": "Mpv"}, {"id": "mpv.2", "name": "Mpv"}, {"id": "spotify", "name": "Spotify"}]});
        assert_eq!(player_label(&twins), "Mpv 2");
        assert_eq!(player_label(&json!({"player": "spotify", "player_title": "Spotify", "players": twins["players"]})), "Spotify");
        let mut d = dashboard();
        d.open_on("a", None);
        let rect = Rect { x: 0.0, y: 0.0, width: 10.0, height: 10.0, radius: 0.0, color: Color::TRANSPARENT };
        d.buttons = vec![(rect, Button::NextPlayer)];
        assert_eq!(d.on_click(Point::new(5.0, 5.0), &music), DashboardAction::ChoosePlayer("c".into()));
        d.close();
        assert_eq!(d.player(), Some("c"), "kept across openings");
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
