//! Battery page: charge, how long it lasts, and the power profile.
//!
//! A charge card leads, with the percentage and a meter coloured by level.
//! The facts the battery hover menu shows (`BatteryHoverMenu.qml`: screen-on
//! time, the projection, the rate) follow as cards, then the power profiles
//! as tiles. Profiles switch on one click: they are easily changed back.

use crate::actions::SystemAction;
use crate::config::Theme;
use crate::render::display_list::{Color, DisplayList, Rect};
use crate::ui::pages::layout::{self, Card};
use crate::ui::pages::overview::{battery_detail, battery_glyph};
use crate::ui::pages::Optimistic;
use crate::ui::{self, InputState, PaintCtx};

/// The profiles offered, with the hover menu's keys: `(key, name, glyph,
/// description)`. Only these are written, because writing a value the
/// platform does not advertise fails silently.
const PROFILES: [(&str, &str, &str, &str); 3] = [
    ("low-power", "Power saver", "\u{f032a}", "Longest battery life"),
    ("balanced", "Balanced", "\u{f05d1}", "Everyday use"),
    ("balanced-performance", "Performance", "\u{f04c5}", "Fastest, uses more power"),
];

pub struct BatteryPage {
    profile: Optimistic<String>,
}

impl BatteryPage {
    pub fn new() -> Self {
        Self {
            profile: Optimistic::new(),
        }
    }

    pub fn tick(&mut self, dt: f32) {
        self.profile.tick(dt);
    }

    /// Returns the bottom of the page.
    pub fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        area: Rect,
        input: &InputState,
        clicked: bool,
        pending: &mut Vec<SystemAction>,
    ) -> f32 {
        let m = ctx.metrics.page.clone();
        let snapshot = ctx.state.battery.get();
        let battery = &snapshot["battery"];
        let present = battery["present"].as_bool().unwrap_or(false);
        let pct = battery["pct"].as_u64().map(|p| p.min(100));
        let state = battery["state"].as_str().unwrap_or("");
        let charging = matches!(state, "charging" | "fully-charged");
        let detail = battery_detail(&snapshot);

        let area = layout::column(area, m.max_width);
        let summary = match (present, pct) {
            (true, Some(p)) => format!("{p}% \u{b7} {detail}"),
            _ => detail.clone(),
        };
        let (_, body) = layout::header(ctx, list, area, "Battery", &summary);

        // The charge card.
        let hero = layout::rect(body.x, body.y, body.width, m.hero_height);
        let color = level_color(ctx.theme, pct, charging);
        // Its title already says "Plugged in", so a held charge explains why.
        let caption = if state == "pending-charge" { "Not charging: held at its charge limit" } else { &detail };
        paint_hero(ctx, list, hero, pct, state_label(state), caption, color, charging);
        let mut y = hero.y + hero.height + m.grid_gap;

        // Facts, three across when they fit and stacked otherwise.
        let (projected_label, projected) = match state {
            "charging" => ("Time to full", text_or_dash(battery["projected_text"].as_str())),
            "discharging" => ("Time remaining", text_or_dash(battery["projected_text"].as_str())),
            "fully-charged" => ("Status", "Fully charged".to_string()),
            "pending-charge" => ("Status", "Holding charge".to_string()),
            _ => ("Projected", text_or_dash(battery["projected_text"].as_str())),
        };
        // The manager reports a held charge as its rate too; the hero card
        // already says it, so the rate card says only that nothing flows.
        let rate = match snapshot["charging_info"].as_str().filter(|r| !r.is_empty()) {
            _ if state == "pending-charge" => "None".to_string(),
            Some(rate) => rate.to_string(),
            None if state == "discharging" => "On battery".to_string(),
            None => "\u{2014}".to_string(),
        };
        let facts = [
            (projected_label, projected),
            ("Screen on", text_or_dash(battery["screen_on_time"].as_str())),
            ("Charge rate", rate),
        ];
        let cols = layout::columns(body.width, m.card_min_width, m.grid_gap, facts.len());
        let cols = layout::even_columns(cols, facts.len());
        let cells = layout::cells(body.x, y, body.width, facts.len(), cols, m.info_height, m.grid_gap);
        for ((label, value), cell) in facts.iter().zip(&cells) {
            layout::info_card(ctx, list, *cell, label, value);
        }
        y += layout::grid_height(facts.len(), cols, m.info_height, m.grid_gap);

        // Power profiles.
        y += m.section_spacing;
        y = layout::section(ctx, list, body, y, "Power profile");
        let real = snapshot["profile"].as_str().unwrap_or("").to_string();
        let active = self.profile.view(real);
        let cols = layout::columns(body.width, m.tile_min_width, m.grid_gap, PROFILES.len());
        let cols = layout::even_columns(cols, PROFILES.len());
        let cells = layout::cells(body.x, y, body.width, PROFILES.len(), cols, m.tile_height, m.grid_gap);
        for ((key, name, glyph, description), cell) in PROFILES.iter().zip(&cells) {
            let selected = active == *key;
            let hot = layout::tile(ctx, list, *cell, glyph, name, description, selected, input);
            if hot && clicked && !selected {
                self.profile.set(key.to_string());
                pending.push(SystemAction::SetPowerProfile(key.to_string()));
            }
        }
        y + layout::grid_height(PROFILES.len(), cols, m.tile_height, m.grid_gap)
    }
}

impl Default for BatteryPage {
    fn default() -> Self {
        Self::new()
    }
}

/// The meter's colour: green while charging or comfortable, then warning and
/// error as the charge runs down.
pub(crate) fn level_color(theme: &Theme, pct: Option<u64>, charging: bool) -> Color {
    match pct {
        _ if charging => theme.success,
        Some(p) if p <= 15 => theme.error,
        Some(p) if p <= 30 => theme.warning,
        _ => theme.success,
    }
}

/// The hover menu's names for each state.
fn state_label(state: &str) -> &'static str {
    match state {
        "charging" => "Charging",
        "discharging" => "On battery",
        "fully-charged" | "pending-charge" => "Plugged in",
        _ => "Battery",
    }
}

fn text_or_dash(text: Option<&str>) -> String {
    text.filter(|t| !t.is_empty() && *t != "Unknown")
        .unwrap_or("\u{2014}")
        .to_string()
}

#[allow(clippy::too_many_arguments)]
fn paint_hero(
    ctx: &mut PaintCtx<'_>,
    list: &mut DisplayList,
    hero: Rect,
    pct: Option<u64>,
    state: &str,
    detail: &str,
    color: Color,
    charging: bool,
) {
    let m = ctx.metrics.page.clone();
    let typo = ctx.metrics.typography.clone();
    let theme = ctx.theme.clone();
    layout::card(ctx, list, hero, Card::Plain);
    let pad = m.card_padding;
    let inner_w = hero.width - pad * 2.0;

    // The badge and state on the top line.
    let badge_bg = layout::mix(theme.surface_high, color, 0.2);
    layout::badge(ctx, list, hero.x + pad, hero.y + pad, battery_glyph(pct.unwrap_or(0), charging), color, badge_bg);
    let text_x = hero.x + pad + m.badge + pad * 0.75;
    layout::two_lines(
        ctx,
        list,
        layout::rect(text_x, hero.y + pad, hero.x + hero.width - pad - text_x, m.badge),
        state,
        detail,
        theme.text_muted,
    );

    // The percentage, large, at the right of the top line.
    let value = pct.map(|p| format!("{p}%")).unwrap_or_else(|| "--".into());
    let value_w = ctx.measure(&value, m.hero_value);
    let value_h = m.hero_value * 1.2;
    let meter_y = hero.y + hero.height - pad - m.hero_meter;
    ui::paint_text_vcentered(
        ctx,
        list,
        &value,
        hero.x + hero.width - pad - value_w,
        layout::rect(hero.x, meter_y - pad * 0.75 - value_h, hero.width, value_h),
        m.hero_value,
        typo.weight_bold,
        theme.text_main,
    );

    let bar = layout::rect(hero.x + pad, meter_y, inner_w, m.hero_meter);
    layout::meter(ctx, list, bar, pct.unwrap_or(0) as f32 / 100.0, color);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_meter_warns_as_charge_runs_down() {
        let theme = Theme::default();
        assert_eq!(level_color(&theme, Some(80), false), theme.success);
        assert_eq!(level_color(&theme, Some(25), false), theme.warning);
        assert_eq!(level_color(&theme, Some(10), false), theme.error);
        assert_eq!(level_color(&theme, Some(10), true), theme.success, "charging is never alarming");
    }

    #[test]
    fn unknown_durations_show_a_dash() {
        assert_eq!(text_or_dash(Some("Unknown")), "\u{2014}");
        assert_eq!(text_or_dash(Some("")), "\u{2014}");
        assert_eq!(text_or_dash(None), "\u{2014}");
        assert_eq!(text_or_dash(Some("2h 05m")), "2h 05m");
    }
}
