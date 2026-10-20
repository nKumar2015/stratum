//! Battery page: charge, state, and power profile.

use crate::actions::SystemAction;
use crate::render::display_list::{Color, DisplayList, Rect};
use crate::ui::controls;
use crate::ui::pages::{heading, section_label, stat_row, Optimistic};
use crate::ui::{InputState, PaintCtx};

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

    pub fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        area: Rect,
        input: &InputState,
        clicked: bool,
        pending: &mut Vec<SystemAction>,
    ) {
        let area = heading(ctx, list, area, "Battery");
        let battery = ctx.state.battery.get();
        let percent = battery["battery"]["pct"]
            .as_u64()
            .map(|p| format!("{p}%"))
            .unwrap_or_else(|| "--".into());
        let state = battery["battery"]["state"].as_str().unwrap_or("unknown");
        let charging = battery["charging_info"].as_str().unwrap_or("");
        let real_profile = battery["profile"].as_str().unwrap_or("").to_string();
        let profile = self.profile.view(real_profile);

        stat_row(ctx, list, area, 0, "Charge", &percent);
        stat_row(ctx, list, area, 1, "State", state);
        stat_row(ctx, list, area, 2, "Power supply", charging);

        let mut y = area.y + 132.0;
        section_label(ctx, list, area, y, "Power profile");
        y += 26.0;

        // Only profiles the platform actually advertises are offered; writing an
        // unsupported value to the ACPI node fails silently otherwise.
        for option in ["low-power", "balanced", "balanced-performance"] {
            let row = Rect {
                x: area.x,
                y,
                width: area.width.min(320.0),
                height: ctx.metrics.page.row_height,
                radius: ctx.metrics.page.button_radius,
                color: Color::TRANSPARENT,
            };
            let selected = profile == option;
            if controls::selectable_row(
                ctx, list, row, option, "", selected, input, clicked,
            ) && !selected
            {
                self.profile.set(option.to_string());
                pending.push(SystemAction::SetPowerProfile(option.to_string()));
            }
            y += ctx.metrics.page.stat_spacing;
        }
    }
}

impl Default for BatteryPage {
    fn default() -> Self {
        Self::new()
    }
}
