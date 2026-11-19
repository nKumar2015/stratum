//! The Display page's night light section: what it is doing now, a hand
//! override, and every `[night_light]` setting. Changes are kept over the
//! config (`settings_override`) through `SystemAction::SetSetting`.

use chrono::TimeZone;

use crate::actions::SystemAction;
use crate::config::NightSchedule;
use crate::night_light::{self as night, Status};
use crate::render::display_list::DisplayList;
use crate::render::display_list::Rect;
use crate::ui::pages::layout;
use crate::ui::pages::settings_card::{with_current, SettingsCard};
use crate::ui::{InputState, PaintCtx};

const TEMPERATURES: [u32; 9] = [2500, 3000, 3500, 4000, 4500, 5000, 5500, 6000, 6500];
const FADES: [u32; 9] = [0, 5, 10, 15, 20, 30, 45, 60, 90];

#[derive(Default)]
pub struct NightLightSection {
    temperature_drag: bool,
    start_drag: bool,
    end_drag: bool,
    fade_drag: bool,
}

/// A Unix time as the local clock shows it.
fn clock(t: i64) -> String {
    chrono::Local.timestamp_opt(t, 0).single().map(|t| t.format("%H:%M").to_string()).unwrap_or_default()
}

/// The section's caption: what it is doing, and until when.
pub fn summary(status: &Status, enabled: bool) -> String {
    if let Some(problem) = &status.problem {
        return problem.clone();
    }
    let until = status.until.map(clock);
    match (status.on, status.manual, until) {
        (true, Some(_), Some(t)) => format!("On by hand at {} K, until {t}", status.kelvin),
        (true, Some(_), None) => format!("On by hand at {} K", status.kelvin),
        (true, None, Some(t)) => format!("On at {} K, until {t}", status.kelvin),
        (true, None, None) => format!("On at {} K", status.kelvin),
        (false, Some(_), Some(t)) => format!("Off by hand until {t}"),
        (false, _, Some(t)) if enabled => format!("Off, on from {t}"),
        (false, Some(_), _) => "Off by hand".to_string(),
        _ if enabled => "Off".to_string(),
        _ => "Off: not scheduled".to_string(),
    }
}

/// Half-hours from `first` to `last` (minutes after midnight).
fn half_hours(first: u32, last: u32) -> Vec<u32> {
    (first..=last).step_by(30).collect()
}

impl NightLightSection {
    /// Paint the section at `y`; returns its bottom.
    #[allow(clippy::too_many_arguments)]
    pub fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        body: Rect,
        y: f32,
        input: &InputState,
        clicked: bool,
        pending: &mut Vec<SystemAction>,
    ) -> f32 {
        let config = ctx.night_light.clone();
        let status = ctx.state.night_light.get();
        let theme = ctx.theme.clone();
        let fixed = config.schedule == NightSchedule::Fixed;
        let y = layout::section(ctx, list, body, y, "Night light");

        let mut rows = vec!["Scheduled", "Right now", "Temperature", "Schedule"];
        rows.extend(if fixed { ["Starts at", "Ends at"] } else { ["Today", "Location"] });
        rows.push("Fade");
        let caption = summary(&status, config.enabled);
        let mut card = SettingsCard::begin(ctx, list, body, y, ("Warmer screens at night", &caption), &rows);

        let set = |pending: &mut Vec<SystemAction>, path: &'static str, value: toml::Value| {
            pending.push(SystemAction::SetSetting { path, value });
        };

        let note = if config.enabled { None } else { Some("Only by hand") };
        if let Some(on) = card.switch(ctx, list, "Scheduled", note, config.enabled, input, clicked) {
            set(pending, "night_light.enabled", toml::Value::Boolean(on));
        }

        let now = ["Auto", "On", "Off"].map(String::from);
        let selected = match status.manual {
            None => 0,
            Some(true) => 1,
            Some(false) => 2,
        };
        if let Some(i) = card.chips(ctx, list, "Right now", &now, selected, input, clicked) {
            pending.push(SystemAction::NightLight(["auto", "on", "off"][i]));
        }

        let (values, index) = with_current(&TEMPERATURES, config.temperature);
        let labels: Vec<String> =
            values.iter().map(|k| if *k >= night::DAY_KELVIN { "6500 K (none)".into() } else { format!("{k} K") }).collect();
        if let Some(i) = card.steps(ctx, list, "Temperature", &labels, index, &mut self.temperature_drag, input, clicked) {
            set(pending, "night_light.temperature", toml::Value::Integer(i64::from(values[i])));
        }

        let schedules = ["Sunset to sunrise", "Set times"].map(String::from);
        if let Some(i) = card.chips(ctx, list, "Schedule", &schedules, usize::from(fixed), input, clicked) {
            set(pending, "night_light.schedule", toml::Value::String(["sun", "fixed"][i].into()));
        }

        if fixed {
            for (label, key, current, choices, drag) in [
                ("Starts at", "night_light.start", &config.start, half_hours(17 * 60, 23 * 60 + 30), &mut self.start_drag),
                ("Ends at", "night_light.end", &config.end, half_hours(4 * 60, 10 * 60), &mut self.end_drag),
            ] {
                let current = night::parse_time(current).unwrap_or(choices[0]);
                let (values, index) = with_current(&choices, current);
                let labels: Vec<String> = values.iter().map(|m| night::format_time(*m)).collect();
                if let Some(i) = card.steps(ctx, list, label, &labels, index, drag, input, clicked) {
                    set(pending, key, toml::Value::String(labels[i].clone()));
                }
            }
        } else {
            let today = chrono::Local::now().date_naive();
            let sun = status.location.and_then(|(l, _)| night::sun_times(today, l.latitude, l.longitude));
            let (text, color) = match (status.location, sun) {
                (Some(_), Some((rise, set))) => (format!("Sunrise {} \u{b7} sunset {}", clock(rise), clock(set)), theme.text_main),
                (Some(_), None) => ("No sunset here today: polar day or night".to_string(), theme.text_muted),
                (None, _) => (format!("No location: {} to {} instead", config.start, config.end), theme.warning),
            };
            card.note(ctx, list, "Today", &text, color);
            let text = match status.location {
                Some((l, from_config)) => {
                    let ns = if l.latitude >= 0.0 { "N" } else { "S" };
                    let ew = if l.longitude >= 0.0 { "E" } else { "W" };
                    let source = if from_config { "from the config" } else { "from the weather" };
                    format!("{:.2}\u{b0} {ns}, {:.2}\u{b0} {ew} \u{b7} {source}", l.latitude.abs(), l.longitude.abs())
                }
                None => "Set one in [night_light], or open the dashboard's weather".to_string(),
            };
            card.note(ctx, list, "Location", &text, theme.text_muted);
        }

        let (values, index) = with_current(&FADES, config.fade_min);
        let labels: Vec<String> = values.iter().map(|m| if *m == 0 { "Instant".into() } else { format!("{m} min") }).collect();
        if let Some(i) = card.steps(ctx, list, "Fade", &labels, index, &mut self.fade_drag, input, clicked) {
            set(pending, "night_light.fade_min", toml::Value::Integer(i64::from(values[i])));
        }
        card.bottom()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_summary_says_what_and_until_when() {
        let on = Status { on: true, kelvin: 4000, ..Status::default() };
        assert_eq!(summary(&on, true), "On at 4000 K");
        let hand = Status { manual: Some(false), ..Status::default() };
        assert_eq!(summary(&hand, true), "Off by hand");
        assert_eq!(summary(&Status::default(), false), "Off: not scheduled");
        let broken = Status { problem: Some("Another program".into()), on: true, ..Status::default() };
        assert_eq!(summary(&broken, true), "Another program");
        assert_eq!(half_hours(60, 120), [60, 90, 120]);
    }
}
