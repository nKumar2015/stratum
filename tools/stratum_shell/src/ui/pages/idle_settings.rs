//! The Battery page's idle and sleep section: every `[idle]` setting but
//! the dim's colour, and the stay-awake switch. Changes are kept over the
//! config (`settings_override`) through `SystemAction::SetSetting`.

use crate::actions::SystemAction;
use crate::config::IdleConfig;
use crate::render::display_list::{DisplayList, Rect};
use crate::ui::pages::layout;
use crate::ui::pages::settings_card::{duration_label, with_current, SettingsCard};
use crate::ui::{InputState, PaintCtx};

const DIM: [u32; 9] = [0, 30, 60, 120, 180, 240, 300, 600, 900];
const LOCK: [u32; 9] = [0, 60, 120, 180, 300, 600, 900, 1800, 3600];
const SCREENS_OFF: [u32; 9] = [0, 60, 120, 300, 330, 600, 900, 1800, 3600];
const SUSPEND: [u32; 9] = [0, 300, 600, 900, 1200, 1800, 2700, 3600, 7200];
/// Darkness in percent.
const DARKNESS: [u32; 8] = [20, 30, 40, 50, 60, 70, 80, 90];
const FADE_MS: [u32; 7] = [0, 500, 1000, 2000, 3000, 5000, 10_000];

#[derive(Default)]
pub struct IdleSection {
    drags: [bool; 6],
}

/// The stages in the order they happen, as the card's caption tells them.
pub fn sequence(config: &IdleConfig) -> String {
    if !config.enabled {
        return "Off: nothing happens when idle".to_string();
    }
    let mut stages: Vec<(u32, &str)> = [
        (config.dim_s, "dims"),
        (config.lock_s, "locks"),
        (config.dpms_s, "screens off"),
        (config.suspend_s, if config.suspend_on_ac { "suspends" } else { "suspends on battery" }),
    ]
    .into_iter()
    .filter(|(s, _)| *s > 0)
    .collect();
    if stages.is_empty() {
        return "Every stage is off".to_string();
    }
    stages.sort_by_key(|(s, _)| *s);
    let parts: Vec<String> = stages.iter().map(|(s, what)| format!("{what} at {}", duration_label(*s))).collect();
    let mut text = parts.join(", ");
    if let Some(first) = text.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    if config.lock_s == 0 {
        text.push_str(" \u{b7} never locks by itself");
    }
    text
}

fn seconds(pending: &mut Vec<SystemAction>, path: &'static str, value: u32) {
    pending.push(SystemAction::SetSetting { path, value: toml::Value::Integer(i64::from(value)) });
}

fn flag(pending: &mut Vec<SystemAction>, path: &'static str, value: bool) {
    pending.push(SystemAction::SetSetting { path, value: toml::Value::Boolean(value) });
}

impl IdleSection {
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
        let config = ctx.idle.clone();
        let gap = ctx.metrics.page.grid_gap;
        let y = layout::section(ctx, list, body, y, "Idle and sleep");

        // The stages.
        let rows = ["Stay awake", "Idle actions", "Dim after", "Lock after", "Screens off", "Suspend after"];
        let caption = sequence(&config);
        let mut card = SettingsCard::begin(ctx, list, body, y, ("When idle", &caption), &rows);
        let awake = ctx.state.idle_inhibit.get();
        let note = awake.then_some("Holding every stage off");
        if let Some(on) = card.switch(ctx, list, "Stay awake", note, awake, input, clicked) {
            pending.push(SystemAction::IdleInhibit(on));
        }
        if let Some(on) = card.switch(ctx, list, "Idle actions", None, config.enabled, input, clicked) {
            flag(pending, "idle.enabled", on);
        }
        let [dim_drag, lock_drag, off_drag, suspend_drag, dark_drag, fade_drag] = &mut self.drags;
        for (label, path, choices, current, drag) in [
            ("Dim after", "idle.dim_s", &DIM[..], config.dim_s, dim_drag),
            ("Lock after", "idle.lock_s", &LOCK[..], config.lock_s, lock_drag),
            ("Screens off", "idle.dpms_s", &SCREENS_OFF[..], config.dpms_s, off_drag),
            ("Suspend after", "idle.suspend_s", &SUSPEND[..], config.suspend_s, suspend_drag),
        ] {
            let (values, index) = with_current(choices, current);
            let labels: Vec<String> = values.iter().map(|s| duration_label(*s)).collect();
            if let Some(i) = card.steps(ctx, list, label, &labels, index, drag, input, clicked) {
                seconds(pending, path, values[i]);
            }
        }
        let y = card.bottom() + gap;

        // The dim.
        let mut card = SettingsCard::begin(
            ctx,
            list,
            body,
            y,
            ("Dimming", "Fades the screens down; any input brings them back"),
            &["Darkness", "Fade"],
        );
        let darkness = (config.dim_opacity * 100.0).round() as u32;
        let (values, index) = with_current(&DARKNESS, darkness);
        let labels: Vec<String> = values.iter().map(|p| format!("{p}%")).collect();
        if let Some(i) = card.steps(ctx, list, "Darkness", &labels, index, dark_drag, input, clicked) {
            let value = f64::from(values[i]) / 100.0;
            pending.push(SystemAction::SetSetting { path: "idle.dim_opacity", value: toml::Value::Float(value) });
        }
        let (values, index) = with_current(&FADE_MS, config.dim_fade_ms);
        let labels: Vec<String> = values
            .iter()
            .map(|ms| if *ms == 0 { "Instant".into() } else { format!("{:.1} s", f64::from(*ms) / 1000.0) })
            .collect();
        if let Some(i) = card.steps(ctx, list, "Fade", &labels, index, fade_drag, input, clicked) {
            seconds(pending, "idle.dim_fade_ms", values[i]);
        }
        let y = card.bottom() + gap;

        // Sleep.
        let switches = [
            ("Suspend on AC power", "idle.suspend_on_ac", config.suspend_on_ac, None),
            ("Suspend while playing", "idle.suspend_while_playing", config.suspend_while_playing, Some("Otherwise media holds suspend off")),
            ("Lock before sleep", "idle.lock_before_sleep", config.lock_before_sleep, Some("Lid, power menu and systemctl suspend")),
            ("Lock on lock-session", "idle.lock_on_loginctl", config.lock_on_loginctl, Some("loginctl lock-session")),
        ];
        let labels: Vec<&str> = switches.iter().map(|s| s.0).collect();
        let mut card = SettingsCard::begin(
            ctx,
            list,
            body,
            y,
            ("Sleep and locking", "The last two take effect when the shell next starts"),
            &labels,
        );
        for (label, path, on, note) in switches {
            if let Some(value) = card.switch(ctx, list, label, note, on, input, clicked) {
                flag(pending, path, value);
            }
        }
        card.bottom()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_caption_lists_the_stages_in_order() {
        let config = crate::config::Config::test_defaults().idle;
        assert_eq!(
            sequence(&config),
            "Dims at 4 min, locks at 5 min, screens off at 5:30, suspends on battery at 15 min"
        );
        let no_lock = IdleConfig { lock_s: 0, dim_s: 0, ..config.clone() };
        assert_eq!(sequence(&no_lock), "Screens off at 5:30, suspends on battery at 15 min \u{b7} never locks by itself");
        assert_eq!(sequence(&IdleConfig { enabled: false, ..config }), "Off: nothing happens when idle");
    }
}
