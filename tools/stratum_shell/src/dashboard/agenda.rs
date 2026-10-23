//! The agenda: events from the selected day (today unless a day was clicked
//! in the month grid) for `agenda_days`, grouped by day. The first day is
//! always shown, empty or not; later days only when they hold something.
//! Today's next event says how soon it starts, and so does the glance
//! strip's chip for it.

use chrono::{Days, NaiveDate, NaiveDateTime};
use stratum_sys::managers::calendar::Occurrence;

use super::{calendar_color, today, Dashboard};
use crate::render::display_list::{Color, DisplayList, Rect};
use crate::ui::{self, PaintCtx};

/// How far ahead the glance strip looks for the next event.
const NEXT_WITHIN_HOURS: i64 = 12;

/// "3:00 PM".
pub fn time_label(time: NaiveDateTime) -> String {
    time.format("%-I:%M %p").to_string()
}

/// "Today", "Tomorrow", then "Sat 24 Oct".
pub fn day_heading(date: NaiveDate, today: NaiveDate) -> String {
    match (date - today).num_days() {
        0 => "Today".into(),
        1 => "Tomorrow".into(),
        -1 => "Yesterday".into(),
        _ => date.format("%a %-d %b").to_string(),
    }
}

/// "now", "in 28m", "in 2h 5m".
pub fn starts_in(now: NaiveDateTime, start: NaiveDateTime) -> String {
    let minutes = (start - now).num_minutes();
    match minutes {
        ..=0 => "now".into(),
        1..=59 => format!("in {minutes}m"),
        _ if minutes % 60 == 0 => format!("in {}h", minutes / 60),
        _ => format!("in {}h {}m", minutes / 60, minutes % 60),
    }
}

/// The timed event on now or starting next, within the next
/// [`NEXT_WITHIN_HOURS`].
pub fn next_event(occurrences: &[Occurrence], now: NaiveDateTime) -> Option<&Occurrence> {
    let horizon = now + chrono::Duration::hours(NEXT_WITHIN_HOURS);
    occurrences
        .iter()
        .filter(|o| !o.all_day && o.end > now && o.start <= horizon)
        .min_by_key(|o| o.start)
}

/// What `date` shows: all-day events covering it, and timed events that
/// start on it.
pub fn on_day(occurrences: &[Occurrence], date: NaiveDate) -> Vec<&Occurrence> {
    let start = date.and_time(chrono::NaiveTime::MIN);
    let end = start + chrono::Duration::days(1);
    occurrences
        .iter()
        .filter(|o| if o.all_day { o.start < end && o.end > start } else { o.start.date() == date })
        .collect()
}

fn measure(ctx: &mut PaintCtx<'_>, text: &str, size: f32, weight: u16) -> f32 {
    let style = crate::text::TextStyle {
        family: std::sync::Arc::from(ctx.theme.font.as_str()),
        size,
        weight,
        color: ctx.theme.text_main,
    };
    ctx.text.measure(text, &style).0
}

impl Dashboard {
    /// The glance strip's chip for the next event, if one is near.
    pub(super) fn next_event_chip(&mut self, ctx: &PaintCtx<'_>) -> Option<String> {
        let now = chrono::Local::now().naive_local();
        let today = today();
        let events = self.occurrences(ctx, today, today + Days::new(2));
        next_event(&events, now).map(|o| format!("{} \u{b7} {}", o.summary, starts_in(now, o.start)))
    }

    pub(super) fn paint_agenda(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, panel: Rect) {
        let m = self.metrics.clone();
        let (regular, medium) = (ctx.metrics.typography.weight_regular, ctx.metrics.typography.weight_medium);
        let mut y = Self::title(ctx, list, panel, "Agenda", &m);
        let x = panel.x + m.inner;
        let w = panel.width - m.inner * 2.0;
        let state = ctx.state.calendar.get();
        let line = |y: f32| Rect { x, y, width: w, height: m.line, radius: 0.0, color: Color::TRANSPARENT };

        if !state.configured {
            let text = "No calendars yet: point [dashboard.calendar] urls_file at a file of ICS addresses.";
            for row in ui::wrap_text(ctx, text, m.caption, regular, w, 4) {
                ui::paint_text_vcentered(ctx, list, &row, x, line(y), m.caption, regular, ctx.theme.text_muted);
                y += m.line;
            }
            return;
        }

        let footer_y = panel.y + panel.height - m.inner - m.line;
        let note = match (&state.error, state.loading) {
            (_, true) => Some(("Updating\u{2026}".to_string(), ctx.theme.text_muted)),
            (Some(err), false) => Some((err.clone(), ctx.theme.warning)),
            _ => None,
        };
        // The last line is kept for a note, or for how many did not fit.
        let bottom = footer_y;

        let today = today();
        let now = chrono::Local::now().naive_local();
        let from = self.selected;
        let days = u64::from(self.config.calendar.agenda_days);
        let events = self.occurrences(ctx, from, from + Days::new(days));
        let next = next_event(&events, now).cloned();
        let time_w = measure(ctx, "12:00 PM", m.caption, regular) + m.inner;
        let mut hidden = 0usize;
        for offset in 0..days {
            let date = from + Days::new(offset);
            let day = on_day(&events, date);
            if day.is_empty() && offset > 0 {
                continue;
            }
            if y + m.line * 2.0 > bottom {
                hidden += day.len();
                continue;
            }
            let heading = day_heading(date, today);
            let color = if date == today { ctx.theme.primary } else { ctx.theme.text_main };
            ui::paint_text_vcentered(ctx, list, &heading, x, line(y), m.text, medium, color);
            y += m.line;
            if day.is_empty() {
                ui::paint_text_vcentered(ctx, list, "Nothing scheduled", x, line(y), m.caption, regular, ctx.theme.text_muted);
                y += m.line;
            }
            for event in day {
                if y + m.line > bottom {
                    hidden += 1;
                    continue;
                }
                let row = line(y);
                let stripe = Rect {
                    x,
                    y: y + m.line * 0.15,
                    width: m.stripe,
                    height: m.line * 0.7,
                    radius: m.stripe * 0.5,
                    color: calendar_color(ctx, event.calendar),
                };
                list.push_rect(stripe);
                let time = if event.all_day { "All day".to_string() } else { time_label(event.start) };
                let tx = x + m.stripe + m.inner * 0.6;
                ui::paint_text_vcentered(ctx, list, &time, tx, row, m.caption, regular, ctx.theme.text_muted);
                // Today's next event says when it starts, on the right.
                let soon = (next.as_ref() == Some(event)).then(|| starts_in(now, event.start));
                let soon_w = soon.as_deref().map_or(0.0, |s| measure(ctx, s, m.caption, medium) + m.inner);
                if let Some(soon) = &soon {
                    let sx = x + w - soon_w + m.inner;
                    ui::paint_text_vcentered(ctx, list, soon, sx, row, m.caption, medium, ctx.theme.primary);
                }
                let sx = tx + time_w;
                let mut summary = event.summary.clone();
                if !event.location.is_empty() {
                    summary = format!("{summary} \u{b7} {}", event.location);
                }
                let summary = ui::elide(ctx, &summary, m.text, regular, x + w - soon_w - sx);
                ui::paint_text_vcentered(ctx, list, &summary, sx, row, m.text, regular, ctx.theme.text_main);
                y += m.line;
            }
            y += m.inner * 0.5;
        }
        if hidden > 0 && note.is_none() {
            let more = format!("+{hidden} more");
            ui::paint_text_vcentered(ctx, list, &more, x, line(footer_y), m.caption, regular, ctx.theme.text_muted);
        }
        if let Some((note, color)) = note {
            let note = ui::elide(ctx, &note, m.caption, regular, w);
            ui::paint_text_vcentered(ctx, list, &note, x, line(footer_y), m.caption, regular, color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(d: u32, h: u32, min: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, d).unwrap().and_hms_opt(h, min, 0).unwrap()
    }

    fn event(summary: &str, start: NaiveDateTime, end: NaiveDateTime, all_day: bool) -> Occurrence {
        Occurrence { summary: summary.into(), location: String::new(), start, end, all_day, calendar: 0 }
    }

    #[test]
    fn headings_and_countdowns_read_naturally() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 23).unwrap();
        assert_eq!(day_heading(today, today), "Today");
        assert_eq!(day_heading(today + Days::new(1), today), "Tomorrow");
        assert_eq!(day_heading(today + Days::new(2), today), "Sun 25 Oct");
        assert_eq!(starts_in(at(23, 14, 32), at(23, 15, 0)), "in 28m");
        assert_eq!(starts_in(at(23, 13, 0), at(23, 15, 0)), "in 2h");
        assert_eq!(starts_in(at(23, 12, 55), at(23, 15, 0)), "in 2h 5m");
        assert_eq!(starts_in(at(23, 15, 5), at(23, 15, 0)), "now");
        assert_eq!(time_label(at(23, 15, 0)), "3:00 PM");
    }

    #[test]
    fn the_next_event_is_ongoing_or_soonest_and_never_all_day() {
        let events = [
            event("Holiday", at(23, 0, 0), at(24, 0, 0), true),
            event("Done", at(23, 9, 0), at(23, 10, 0), false),
            event("Meeting", at(23, 14, 0), at(23, 15, 0), false),
            event("Later", at(23, 18, 0), at(23, 19, 0), false),
        ];
        assert_eq!(next_event(&events, at(23, 12, 0)).map(|o| o.summary.as_str()), Some("Meeting"));
        assert_eq!(next_event(&events, at(23, 14, 30)).map(|o| o.summary.as_str()), Some("Meeting"), "still on");
        assert_eq!(next_event(&events, at(23, 2, 0)).map(|o| o.summary.as_str()), Some("Done"));
        // Nothing within twelve hours.
        assert_eq!(next_event(&events[..3], at(23, 15, 30)), None);
    }

    #[test]
    fn a_day_shows_all_day_events_covering_it_and_timed_ones_starting_on_it() {
        let events = [
            event("Trip", at(22, 0, 0), at(25, 0, 0), true),
            event("Overnight", at(22, 22, 0), at(23, 2, 0), false),
            event("Lunch", at(23, 12, 0), at(23, 13, 0), false),
        ];
        let day = on_day(&events, NaiveDate::from_ymd_opt(2026, 10, 23).unwrap());
        assert_eq!(day.iter().map(|o| o.summary.as_str()).collect::<Vec<_>>(), ["Trip", "Lunch"]);
    }
}
