//! The Focus tab: to-dos, a note, and a timer.
//!
//! - To-dos and the note are kept in `$XDG_STATE_HOME/stratum/focus.json`,
//!   saved on every change. `stratum-cli todo add <text>` adds one.
//! - The to-do input and the note take the keyboard while focused (a click
//!   focuses them): 1–9 type rather than switch tabs, Escape gives the
//!   focus back first, and Ctrl+Tab still switches.
//! - The timer ([`super::timer`]) keeps running while the dashboard is
//!   closed; the glance strip shows it, and its end is a notification.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use smithay_client_toolkit::seat::keyboard::Keysym;

use super::note::{line_of, Note, NoteKey};
use super::timer::{self, Ended, Mode, Timer};
use super::{Button, Dashboard, DashboardAction};
use crate::config::TimerConfig;
use crate::render::display_list::{Color, DisplayList, Rect};
use crate::ui::text_field::{FieldKey, FieldStyle, TextField};
use crate::ui::{self, PaintCtx};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Todo {
    pub text: String,
    pub done: bool,
}

/// What `focus.json` holds.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Saved {
    #[serde(default)]
    pub todos: Vec<Todo>,
    #[serde(default)]
    pub note: String,
}

fn default_path() -> Option<PathBuf> {
    crate::paths::state_file("focus.json")
}

/// The field that has the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Todo,
    Note,
}

#[derive(Default)]
pub struct Focus {
    pub todos: Vec<Todo>,
    pub note: Note,
    pub input: TextField,
    pub field: Option<Field>,
    pub timer: Timer,
    /// Where to save; `None` (the tests) saves nothing.
    path: Option<PathBuf>,
    /// The timer's face at the last paint, to repaint only when it changes.
    pub painted_face: Option<String>,
}

impl Focus {
    /// Read the saved to-dos and note, and save there from now on.
    pub fn load(&mut self) {
        let Some(path) = default_path() else { return };
        if let Ok(text) = std::fs::read_to_string(&path) {
            match serde_json::from_str::<Saved>(&text) {
                Ok(saved) => {
                    self.todos = saved.todos;
                    self.note = Note::new(&saved.note);
                }
                Err(err) => tracing::warn!("ignoring unreadable {}: {err}", path.display()),
            }
        }
        self.path = Some(path);
    }

    /// Write atomically, so a crash mid-write cannot lose the list.
    fn save(&self) {
        let Some(path) = &self.path else { return };
        let saved = Saved { todos: self.todos.clone(), note: self.note.text().to_string() };
        let result = serde_json::to_string(&saved)
            .map_err(std::io::Error::from)
            .and_then(|text| crate::paths::write_atomic(path, format!("{text}\n")));
        if let Err(err) = result {
            tracing::warn!("failed to save {}: {err}", path.display());
        }
    }

    pub fn add_todo(&mut self, text: &str) -> bool {
        let text = text.trim();
        if text.is_empty() {
            return false;
        }
        self.todos.push(Todo { text: text.to_string(), done: false });
        self.save();
        true
    }

    fn toggle_todo(&mut self, index: usize) {
        if let Some(todo) = self.todos.get_mut(index) {
            todo.done = !todo.done;
            self.save();
        }
    }

    fn delete_todo(&mut self, index: usize) {
        if index < self.todos.len() {
            self.todos.remove(index);
            self.save();
        }
    }

    fn clear_done(&mut self) {
        self.todos.retain(|t| !t.done);
        self.save();
    }

    /// A key for the focused field. `None` when no field has focus.
    pub fn key(&mut self, keysym: Keysym, text: Option<&str>, ctrl: bool) -> Option<DashboardAction> {
        let field = self.field?;
        let redraw = match field {
            Field::Todo => match self.input.key(keysym, text, ctrl) {
                FieldKey::Ignored => false,
                FieldKey::Changed => true,
                FieldKey::Submit(text) => self.add_todo(text.expose()),
                FieldKey::Escape => {
                    self.field = None;
                    true
                }
            },
            Field::Note => match self.note.key(keysym, text, ctrl) {
                NoteKey::Ignored => false,
                NoteKey::Moved => true,
                NoteKey::Changed => {
                    self.save();
                    true
                }
                NoteKey::Escape => {
                    self.field = None;
                    true
                }
            },
        };
        Some(if redraw { DashboardAction::Redraw } else { DashboardAction::None })
    }

    /// `stratum-cli timer <function> [arg]`: `start <length>|pomodoro|
    /// stopwatch` (no argument resumes), `pause`, `toggle`, `stop`.
    pub fn timer_command(&mut self, function: &str, arg: Option<&str>, config: &TimerConfig, now: Instant) -> Result<(), String> {
        match (function, arg) {
            ("start", None) if self.timer.active() && !self.timer.finished(now) => self.timer.resume(now),
            ("start", None) => return Err("no timer to resume: give a length (25m, 90s, 1h30m), pomodoro or stopwatch".into()),
            ("start", Some("pomodoro")) => self.timer.start_pomodoro(config.pomodoro(), now),
            ("start", Some("stopwatch")) => self.timer.start_stopwatch(now),
            ("start", Some(length)) => match timer::parse_duration(length) {
                Some(length) => self.timer.start_countdown(length, now),
                None => return Err(format!("not a length: {length} (try 25m, 90s, 1h30m or 25)")),
            },
            ("pause", _) => self.timer.pause(now),
            ("toggle", _) => self.timer.toggle(now),
            ("stop", _) => self.timer.stop(),
            _ => return Err(format!("unknown timer function: {function}")),
        }
        Ok(())
    }

    /// Act on a click on one of the Focus tab's buttons; `false` for a
    /// button that is not one.
    fn click(&mut self, button: Button, config: &TimerConfig, now: Instant) -> bool {
        // A click anywhere but a field takes the focus away from it.
        self.field = match button {
            Button::TodoInput => Some(Field::Todo),
            Button::NoteArea => Some(Field::Note),
            _ => None,
        };
        match button {
            Button::TodoInput => {}
            Button::NoteArea => self.note.caret_to_end(),
            Button::TodoToggle(i) => self.toggle_todo(i),
            Button::TodoDelete(i) => self.delete_todo(i),
            Button::ClearDone => self.clear_done(),
            Button::TimerPreset(minutes) => self.timer.start_countdown(Duration::from_secs(minutes * 60), now),
            Button::Stopwatch => self.timer.start_stopwatch(now),
            Button::Pomodoro => self.timer.start_pomodoro(config.pomodoro(), now),
            Button::TimerToggle => self.timer.toggle(now),
            Button::TimerReset => self.timer.reset(),
            Button::TimerAdd => self.timer.add(Duration::from_secs(60), now),
            Button::TimerStop => self.timer.stop(),
            _ => return false,
        }
        true
    }
}

/// "Pomodoro · Work 2/4", "Timer", "Stopwatch".
fn mode_label(mode: Option<Mode>) -> String {
    match mode {
        None | Some(Mode::Countdown) => "Timer".into(),
        Some(Mode::Stopwatch) => "Stopwatch".into(),
        Some(Mode::Pomodoro { phase, round, settings }) => {
            format!("Pomodoro \u{b7} {} {round}/{}", phase.label(), settings.rounds)
        }
    }
}

/// The glance strip's chip: the face, and the phase for Pomodoro.
pub fn glance_chip(timer: &Timer, now: Instant) -> Option<(&'static str, String)> {
    if !timer.active() {
        return None;
    }
    let face = timer.face(now);
    let text = match timer.mode() {
        Some(Mode::Pomodoro { phase, .. }) => format!("{} {face}", phase.label()),
        _ => face,
    };
    let glyph = if timer.running() { "\u{f051b}" } else { "\u{f03e4}" };
    Some((glyph, text))
}

impl Dashboard {
    /// Load the saved to-dos and note (not done in `new`, so tests never
    /// read or write the real file).
    pub fn load_focus(&mut self) {
        self.focus.load();
    }

    pub fn add_todo(&mut self, text: &str) -> bool {
        self.focus.add_todo(text)
    }

    pub fn timer_command(&mut self, function: &str, arg: Option<&str>) -> Result<(), String> {
        self.focus.timer_command(function, arg, &self.config.timer, Instant::now())
    }

    /// A countdown or Pomodoro phase that ended since the last call.
    pub fn tick_timer(&mut self, now: Instant) -> Option<Ended> {
        self.focus.timer.tick(now)
    }

    /// Whether the timer is on screen: an open dashboard on a tab with the
    /// timer or the glance strip.
    fn timer_shown(&self) -> bool {
        self.open && (self.shows(crate::config::PanelKind::Timer) || self.shows(crate::config::PanelKind::Glance))
    }

    /// How long the loop may sleep for the timer's sake.
    pub fn timer_wake(&self, now: Instant) -> Option<Duration> {
        self.focus.timer.next_wake(now, self.timer_shown())
    }

    /// Whether the shown timer reads differently from its last paint.
    pub fn timer_changed(&self, now: Instant) -> bool {
        let face = self.focus.timer.active().then(|| self.focus.timer.face(now));
        self.timer_shown() && face != self.focus.painted_face
    }

    /// A key for a focused field, or `None` to let the dashboard have it.
    pub(super) fn focus_key(&mut self, keysym: Keysym, text: Option<&str>, ctrl: bool) -> Option<DashboardAction> {
        self.focus.key(keysym, text, ctrl)
    }

    /// Tab: the shown fields in turn (the to-do input, then the note),
    /// wrapping; Shift+Tab backwards.
    pub(super) fn next_field(&mut self, back: bool) -> DashboardAction {
        let mut fields = Vec::new();
        if self.shows(crate::config::PanelKind::Todo) {
            fields.push(Field::Todo);
        }
        if self.shows(crate::config::PanelKind::Note) {
            fields.push(Field::Note);
        }
        if fields.is_empty() {
            return DashboardAction::None;
        }
        let n = fields.len();
        let next = match self.focus.field.and_then(|f| fields.iter().position(|&g| g == f)) {
            Some(i) if back => (i + n - 1) % n,
            Some(i) => (i + 1) % n,
            None if back => n - 1,
            None => 0,
        };
        self.focus.field = Some(fields[next]);
        DashboardAction::Redraw
    }

    /// Clicks on the Focus tab's buttons; `None` for other buttons.
    pub(super) fn focus_click(&mut self, button: Button) -> Option<DashboardAction> {
        let config = self.config.timer.clone();
        self.focus.click(button, &config, Instant::now()).then_some(DashboardAction::Redraw)
    }

    pub(super) fn paint_todo(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, panel: Rect) {
        let m = self.metrics.clone();
        let (regular, medium) = (ctx.metrics.typography.weight_regular, ctx.metrics.typography.weight_medium);
        let mut y = Self::title(ctx, list, panel, "To-do", &m);
        let x = panel.x + m.inner;
        let w = panel.width - m.inner * 2.0;

        // The title row's right end: what is left, and a way to clear done.
        let left = self.focus.todos.iter().filter(|t| !t.done).count();
        let done = self.focus.todos.len() - left;
        let title_row = Rect { x, y: panel.y + m.inner, width: w, height: m.line * 1.2, radius: 0.0, color: Color::TRANSPARENT };
        let mut right = x + w;
        if done > 0 {
            let label = "\u{f00e2} Clear done";
            let lw = super::today::measure(ctx, label, m.caption, regular) + m.inner;
            right -= lw;
            let chip = Rect { x: right, width: lw, radius: m.radius * 0.6, ..title_row };
            ui::paint_frame(list, chip, m.border, ctx.theme.border_inactive, ctx.theme.bg_main);
            ui::paint_text_centered(ctx, list, label, chip, m.caption, ctx.theme.text_muted);
            self.buttons.push((chip, Button::ClearDone));
            right -= m.inner * 0.5;
        }
        if !self.focus.todos.is_empty() {
            let count = format!("{left} left");
            let cw = super::today::measure(ctx, &count, m.caption, regular);
            ui::paint_text_vcentered(ctx, list, &count, right - cw, title_row, m.caption, regular, ctx.theme.text_muted);
        }

        // The input.
        let focused = self.focus.field == Some(Field::Todo);
        let field = Rect { x, y, width: w, height: m.button, radius: m.radius * 0.6, color: Color::TRANSPARENT };
        let border = if focused { ctx.theme.border_active } else { ctx.theme.border_inactive };
        ui::paint_frame(list, field, m.border, border, ctx.theme.bg_main);
        let style = FieldStyle { text: m.text, padding: m.inner * 0.6, caret_width: m.caret, masked: false, placeholder: Some("Add a to-do") };
        self.focus.input.paint(ctx, list, field, style, !focused);
        self.buttons.push((field, Button::TodoInput));
        y += m.button + m.inner * 0.5;

        // Open ones first, then done, each in the order added.
        let mut order: Vec<usize> = (0..self.focus.todos.len()).collect();
        order.sort_by_key(|&i| self.focus.todos[i].done);
        let row_h = m.line * 1.3;
        let bottom = panel.y + panel.height - m.inner;
        let fits = ((bottom - y) / row_h).floor().max(0.0) as usize;
        let shown = if order.len() > fits { fits.saturating_sub(1) } else { order.len() };
        for &i in &order[..shown] {
            let todo = self.focus.todos[i].clone();
            let row = Rect { x, y, width: w, height: row_h, radius: 0.0, color: Color::TRANSPARENT };
            let check = Rect { x, y: y + (row_h - m.check) * 0.5, width: m.check, height: m.check, radius: m.check * 0.25, color: Color::TRANSPARENT };
            if todo.done {
                list.push_rect(Rect { color: ctx.theme.primary, ..check });
                ui::paint_text_centered(ctx, list, "\u{f012c}", check, m.check * 0.9, ctx.theme.bg_main);
            } else {
                ui::paint_frame(list, check, m.border, ctx.theme.border_active, ctx.theme.bg_main);
            }
            let delete_w = m.check + m.inner;
            let text_x = x + m.check + m.inner * 0.6;
            let text = ui::elide(ctx, &todo.text, m.text, medium, x + w - delete_w - text_x);
            let color = if todo.done { ctx.theme.text_muted } else { ctx.theme.text_main };
            ui::paint_text_vcentered(ctx, list, &text, text_x, row, m.text, medium, color);
            let delete = Rect { x: x + w - delete_w, width: delete_w, ..row };
            ui::paint_text_centered(ctx, list, "\u{f0156}", delete, m.text, ctx.theme.text_muted);
            self.buttons.push((delete, Button::TodoDelete(i)));
            self.buttons.push((Rect { width: w - delete_w, ..row }, Button::TodoToggle(i)));
            y += row_h;
        }
        let hidden = order.len() - shown;
        let note = match (hidden, order.is_empty()) {
            (0, true) => Some("Nothing to do".to_string()),
            (0, false) => None,
            (n, _) => Some(format!("+{n} more")),
        };
        if let Some(note) = note {
            let row = Rect { x, y, width: w, height: row_h, radius: 0.0, color: Color::TRANSPARENT };
            ui::paint_text_vcentered(ctx, list, &note, x, row, m.caption, regular, ctx.theme.text_muted);
        }
    }

    pub(super) fn paint_note(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, panel: Rect) {
        let m = self.metrics.clone();
        let regular = ctx.metrics.typography.weight_regular;
        let y = Self::title(ctx, list, panel, "Note", &m);
        let area = Rect {
            x: panel.x + m.inner,
            y,
            width: panel.width - m.inner * 2.0,
            height: panel.y + panel.height - m.inner - y,
            radius: m.radius * 0.6,
            color: Color::TRANSPARENT,
        };
        let focused = self.focus.field == Some(Field::Note);
        let border = if focused { ctx.theme.border_active } else { ctx.theme.border_inactive };
        ui::paint_frame(list, area, m.border, border, ctx.theme.bg_main);
        self.buttons.push((area, Button::NoteArea));

        let pad = m.inner * 0.6;
        let text_x = area.x + pad;
        let width = area.width - pad * 2.0 - m.caret * 2.0;
        let visible = ((area.height - pad * 2.0) / m.line).floor().max(1.0) as usize;
        let style = crate::text::TextStyle {
            family: std::sync::Arc::from(ctx.theme.font.as_str()),
            size: m.text,
            weight: regular,
            color: ctx.theme.text_main,
        };
        let lines = self.focus.note.relayout(width, visible, |s| ctx.text.measure(s, &style).0).to_vec();
        let text = self.focus.note.text().to_string();
        if text.is_empty() && !focused {
            let row = Rect { x: text_x, y: area.y + pad, width, height: m.line, radius: 0.0, color: Color::TRANSPARENT };
            ui::paint_text_vcentered(ctx, list, "Click to write", text_x, row, m.text, regular, ctx.theme.text_muted);
            return;
        }
        let cursor = self.focus.note.cursor();
        let caret_line = line_of(&lines, cursor);
        let scroll = self.focus.note.scroll;
        for (n, &(start, end)) in lines.iter().enumerate().skip(scroll).take(visible) {
            let row = Rect { x: text_x, y: area.y + pad + m.line * (n - scroll) as f32, width, height: m.line, radius: 0.0, color: Color::TRANSPARENT };
            let line = &text[start..end];
            if !line.trim().is_empty() {
                ui::paint_text_vcentered(ctx, list, line, text_x, row, m.text, regular, ctx.theme.text_main);
            }
            if focused && n == caret_line {
                let before = &text[start..cursor.min(end).max(start)];
                let cx = if before.is_empty() { 0.0 } else { ctx.text.measure(before, &style).0 };
                let caret_h = m.text * 1.1;
                list.push_rect(Rect {
                    x: text_x + cx,
                    y: row.y + (row.height - caret_h) * 0.5,
                    width: m.caret,
                    height: caret_h,
                    radius: 0.0,
                    color: ctx.theme.text_main,
                });
            }
        }
    }

    pub(super) fn paint_timer(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, panel: Rect) {
        let m = self.metrics.clone();
        let now = Instant::now();
        let (regular, bold) = (ctx.metrics.typography.weight_regular, ctx.metrics.typography.weight_bold);
        let timer = self.focus.timer.clone();
        let title = mode_label(timer.mode());
        let top = Self::title(ctx, list, panel, &title, &m);
        let x = panel.x + m.inner;
        let w = panel.width - m.inner * 2.0;
        let row = |y: f32| Rect { x, y, width: w, height: m.button, radius: 0.0, color: Color::TRANSPARENT };

        // From the bottom up: controls while set, the modes, the presets.
        let mut bottom = panel.y + panel.height - m.inner - m.button;
        if timer.active() {
            let play = if timer.running() { "\u{f03e4}" } else { "\u{f040a}" };
            let controls = [
                (play, Button::TimerToggle, false),
                ("\u{f0709}", Button::TimerReset, false),
                ("+1m", Button::TimerAdd, false),
                ("\u{f04db}", Button::TimerStop, false),
            ];
            self.button_row(ctx, list, row(bottom), &controls);
            bottom -= m.button + m.cell_gap;
        }
        let mode = timer.mode();
        let modes = [
            ("\u{f051b} Stopwatch", Button::Stopwatch, mode == Some(Mode::Stopwatch)),
            ("\u{f0176} Pomodoro", Button::Pomodoro, matches!(mode, Some(Mode::Pomodoro { .. }))),
        ];
        self.button_row(ctx, list, row(bottom), &modes);
        bottom -= m.button + m.cell_gap;
        let labels: Vec<String> = self.config.timer.presets_min.iter().map(|&min| preset_label(min)).collect();
        let presets: Vec<(&str, Button, bool)> = labels
            .iter()
            .zip(&self.config.timer.presets_min)
            .map(|(label, &min)| {
                let lit = mode == Some(Mode::Countdown) && timer.total() == Some(Duration::from_secs(min * 60));
                (label.as_str(), Button::TimerPreset(min), lit)
            })
            .collect();
        self.button_row(ctx, list, row(bottom), &presets);

        // The face, centred in what is left, with its state and a bar.
        let face = timer.face(now);
        let finished = timer.finished(now);
        let status = if finished {
            "Done".to_string()
        } else if !timer.active() {
            "Pick a length".to_string()
        } else if !timer.running() {
            "Paused".to_string()
        } else if let Some(left) = timer.remaining(now) {
            let end = chrono::Local::now() + chrono::Duration::from_std(left).unwrap_or_default();
            format!("Ends {}", end.format("%-I:%M %p"))
        } else {
            "Running".to_string()
        };
        let block = m.timer_face * 1.2 + m.line + m.bar * 2.0;
        let fy = top + ((bottom - m.inner - top) - block).max(0.0) * 0.5;
        let face_row = Rect { x, y: fy, width: w, height: m.timer_face * 1.2, radius: 0.0, color: Color::TRANSPARENT };
        let color = match (finished, timer.running()) {
            (true, _) => ctx.theme.warning,
            (false, true) => ctx.theme.text_main,
            (false, false) => ctx.theme.text_muted,
        };
        let face_w = super::today::measure(ctx, &face, m.timer_face, bold);
        ui::paint_text_vcentered(ctx, list, &face, x + (w - face_w) * 0.5, face_row, m.timer_face, bold, color);
        let status_row = Rect { y: face_row.y + face_row.height, height: m.line, ..face_row };
        let sw = super::today::measure(ctx, &status, m.caption, regular);
        ui::paint_text_vcentered(ctx, list, &status, x + (w - sw) * 0.5, status_row, m.caption, regular, ctx.theme.text_muted);
        if let Some(progress) = timer.progress(now) {
            let track = Rect { x, y: status_row.y + status_row.height + m.bar * 0.5, width: w, height: m.bar, radius: m.bar * 0.5, color: ctx.theme.bg_hover };
            list.push_rect(track);
            list.push_rect(Rect { width: w * progress, color: ctx.theme.primary, ..track });
        }
    }

    /// Remember what the timer showed, wherever it was painted.
    pub(super) fn painted_timer(&mut self, now: Instant) {
        self.focus.painted_face = self.focus.timer.active().then(|| self.focus.timer.face(now));
    }
}

/// "25m", "1h", "1h30m".
fn preset_label(minutes: u64) -> String {
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h{m}m"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> TimerConfig {
        crate::config::Config::test_defaults().dashboard.timer
    }

    #[test]
    fn saved_state_round_trips_and_tolerates_missing_keys() {
        let saved = Saved { todos: vec![Todo { text: "a".into(), done: true }], note: "n\nm".into() };
        let text = serde_json::to_string(&saved).unwrap();
        assert_eq!(serde_json::from_str::<Saved>(&text).unwrap(), saved);
        assert_eq!(serde_json::from_str::<Saved>(r#"{"note":"x"}"#).unwrap().todos, []);
    }

    #[test]
    fn todos_add_toggle_and_clear() {
        let mut focus = Focus::default();
        assert!(!focus.add_todo("   "), "blank is not a to-do");
        focus.add_todo(" milk ");
        focus.add_todo("eggs");
        assert_eq!(focus.todos[0].text, "milk");
        focus.click(Button::TodoToggle(0), &config(), Instant::now());
        assert!(focus.todos[0].done);
        focus.click(Button::ClearDone, &config(), Instant::now());
        assert_eq!(focus.todos, [Todo { text: "eggs".into(), done: false }]);
        focus.click(Button::TodoDelete(0), &config(), Instant::now());
        assert!(focus.todos.is_empty());
    }

    #[test]
    fn the_focused_field_takes_keys_until_escape() {
        let mut focus = Focus::default();
        assert_eq!(focus.key(Keysym::_1, Some("1"), false), None, "unfocused, keys are the dashboard's");
        focus.click(Button::TodoInput, &config(), Instant::now());
        for ch in "call 2".chars() {
            focus.key(Keysym::a, Some(&ch.to_string()), false);
        }
        assert_eq!(focus.key(Keysym::Return, None, false), Some(DashboardAction::Redraw));
        assert_eq!(focus.todos[0].text, "call 2");
        assert_eq!(focus.field, Some(Field::Todo), "still focused for the next one");
        focus.key(Keysym::Escape, None, false);
        assert_eq!(focus.field, None);
        focus.click(Button::NoteArea, &config(), Instant::now());
        focus.key(Keysym::a, Some("x"), false);
        assert_eq!(focus.note.text(), "x");
        focus.click(Button::TimerStop, &config(), Instant::now());
        assert_eq!(focus.field, None, "a click elsewhere unfocuses");
    }

    #[test]
    fn timer_commands() {
        let mut focus = Focus::default();
        let t0 = Instant::now();
        assert!(focus.timer_command("start", None, &config(), t0).is_err(), "nothing to resume");
        assert!(focus.timer_command("start", Some("5x"), &config(), t0).is_err());
        focus.timer_command("start", Some("90s"), &config(), t0).unwrap();
        assert_eq!(focus.timer.face(t0), "1:30");
        focus.timer_command("pause", None, &config(), t0).unwrap();
        assert!(!focus.timer.running());
        focus.timer_command("start", None, &config(), t0).unwrap();
        assert!(focus.timer.running(), "start alone resumes");
        focus.timer_command("start", Some("pomodoro"), &config(), t0).unwrap();
        assert_eq!(mode_label(focus.timer.mode()), "Pomodoro \u{b7} Work 1/4");
        focus.timer_command("stop", None, &config(), t0).unwrap();
        assert!(glance_chip(&focus.timer, t0).is_none());
    }

    #[test]
    fn presets_read_short() {
        assert_eq!((preset_label(25), preset_label(60), preset_label(90)), ("25m".into(), "1h".into(), "1h30m".into()));
    }
}
