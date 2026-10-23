//! Power page: session and system actions.
//!
//! The entries are the power menu's (`power_menu::OPTIONS`) plus Lock, in
//! tiles grouped by what they end. Nothing runs on the first click: it arms
//! the tile, and only a second click on the same tile runs it. An armed tile
//! disarms after `[metrics.page] confirm_ms`, or when another tile is
//! clicked, the page is left, or the window closes.

use crate::actions::SystemAction;
use crate::power_menu::{PowerOption, OPTIONS};
use crate::render::display_list::{DisplayList, Rect};
use crate::ui::pages::layout::{self, Card};
use crate::ui::{self, InputState, PaintCtx};

/// Lock is not in the power menu, which the lock keybind makes redundant
/// there; the settings page has always offered it.
const LOCK: PowerOption = PowerOption {
    name: "Lock",
    glyph: "\u{f023}",
    argv: &["loginctl", "lock-session"],
    danger: false,
};

/// Sections and the entries in them, by name.
const SECTIONS: [(&str, &[&str]); 3] = [
    ("Session", &["Lock", "Logout", "Suspend"]),
    ("System", &["Reboot", "Shutdown"]),
    ("Firmware", &["Reboot into Windows", "Reboot into BIOS"]),
];

fn option(name: &str) -> Option<&'static PowerOption> {
    if name == LOCK.name {
        return Some(&LOCK);
    }
    OPTIONS.iter().find(|option| option.name == name)
}

/// The line under a resting tile's name.
fn describe(name: &str) -> &'static str {
    match name {
        "Lock" => "Lock the screen",
        "Logout" => "End this session",
        "Suspend" => "Sleep, keeping the session",
        "Reboot" => "Restart the computer",
        "Shutdown" => "Power off",
        "Reboot into Windows" => "Boot Windows once",
        "Reboot into BIOS" => "Open firmware setup",
        _ => "",
    }
}

/// The armed tile and how long it stays armed.
#[derive(Debug, Default)]
struct Armed {
    name: Option<&'static str>,
    remaining: f32,
}

impl Armed {
    /// A click on `name`: arms it, or returns true when it was already armed.
    fn click(&mut self, name: &'static str, seconds: f32) -> bool {
        if self.name == Some(name) {
            self.name = None;
            return true;
        }
        self.name = Some(name);
        self.remaining = seconds;
        false
    }

    fn tick(&mut self, dt: f32) {
        if self.name.is_some() {
            self.remaining -= dt;
            if self.remaining <= 0.0 {
                self.name = None;
            }
        }
    }
}

pub struct PowerPage {
    armed: Armed,
}

impl PowerPage {
    pub fn new() -> Self {
        Self {
            armed: Armed::default(),
        }
    }

    pub fn tick(&mut self, dt: f32) {
        self.armed.tick(dt);
    }

    /// Frames are needed while armed, so the tile disarms on time.
    pub fn animating(&self) -> bool {
        self.armed.name.is_some()
    }

    pub fn disarm(&mut self) {
        self.armed.name = None;
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
        let area = layout::column(area, m.max_width);
        let summary = match self.armed.name {
            Some(name) => format!("Click {name} again to confirm"),
            None => "Every action asks for a second click".to_string(),
        };
        let (_, body) = layout::header(ctx, list, area, "Power", &summary);

        let cols = layout::columns(body.width, m.action_min_width, m.grid_gap, 3);
        // `action_height`, or more when the badge and the two lines need it:
        // a fixed height let the name run into the badge.
        let typo = &ctx.metrics.typography;
        let content = m.badge + m.badge_gap + typo.line(typo.body) + typo.line(typo.caption);
        let tile_h = m.action_height.max(content + m.card_padding * 2.0);
        let mut y = body.y;
        let mut hit_tile = false;
        for (section, (label, names)) in SECTIONS.iter().enumerate() {
            if section > 0 {
                y += m.section_spacing;
            }
            y = layout::section(ctx, list, body, y, label);
            let options: Vec<_> = names.iter().filter_map(|name| option(name)).collect();
            let cells = layout::cells(body.x, y, body.width, options.len(), cols, tile_h, m.grid_gap);
            for (option, cell) in options.iter().zip(&cells) {
                let armed = self.armed.name == Some(option.name);
                if self.tile(ctx, list, *cell, option, armed, input) && clicked {
                    hit_tile = true;
                    if self.armed.click(option.name, m.confirm_ms as f32 / 1000.0) {
                        pending.push(SystemAction::Spawn(
                            option.argv.iter().map(|s| s.to_string()).collect(),
                        ));
                    }
                }
            }
            y += layout::grid_height(options.len(), cols, tile_h, m.grid_gap);
        }
        // A click anywhere else on the page backs out.
        if clicked && !hit_tile {
            self.disarm();
        }
        y
    }

    /// Draw one tile. Returns whether the pointer is over it.
    fn tile(
        &self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        cell: Rect,
        option: &PowerOption,
        armed: bool,
        input: &InputState,
    ) -> bool {
        let m = ctx.metrics.page.clone();
        let typo = ctx.metrics.typography.clone();
        let hot = layout::hovered(input, cell);
        let accent = if option.danger {
            ctx.theme.error
        } else {
            ctx.theme.primary
        };
        let style = match (armed, option.danger) {
            (true, true) => Card::Danger,
            (true, false) => Card::Active,
            _ if hot => Card::Hover,
            _ => Card::Plain,
        };
        layout::card(ctx, list, cell, style);

        let pad = m.card_padding;
        let badge_bg = layout::mix(ctx.theme.surface_high, accent, if armed { 0.35 } else { 0.18 });
        layout::badge(ctx, list, cell.x + pad, cell.y + pad, option.glyph, accent, badge_bg);

        let text_w = cell.width - pad * 2.0;
        let line = typo.line(typo.body);
        let caption = typo.line(typo.caption);
        let label_y = cell.y + cell.height - pad - line - caption;
        let label = ui::elide(ctx, option.name, typo.body, typo.weight_bold, text_w);
        ui::paint_text_vcentered(
            ctx,
            list,
            &label,
            cell.x + pad,
            layout::rect(cell.x, label_y, cell.width, line),
            typo.body,
            typo.weight_bold,
            ctx.theme.text_main,
        );
        let (note, color) = if armed {
            ("Click again to confirm", accent)
        } else {
            (describe(option.name), ctx.theme.text_muted)
        };
        let note = ui::elide(ctx, note, typo.caption, typo.weight_regular, text_w);
        ui::paint_text_vcentered(
            ctx,
            list,
            &note,
            cell.x + pad,
            layout::rect(cell.x, label_y + line, cell.width, caption),
            typo.caption,
            typo.weight_regular,
            color,
        );
        hot
    }
}

impl Default for PowerPage {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_entry_exists() {
        for (_, names) in SECTIONS {
            for name in names {
                assert!(option(name).is_some(), "{name} is not a power option");
            }
        }
        // Every power menu entry is reachable from the page too.
        for entry in &OPTIONS {
            assert!(SECTIONS.iter().any(|(_, names)| names.contains(&entry.name)), "{}", entry.name);
        }
    }

    #[test]
    fn only_a_second_click_runs() {
        let mut armed = Armed::default();
        assert!(!armed.click("Shutdown", 4.0), "the first click only arms");
        assert!(armed.click("Shutdown", 4.0), "the second runs");
        assert!(!armed.click("Shutdown", 4.0), "and the next starts over");
    }

    #[test]
    fn another_tile_rearms_instead_of_running() {
        let mut armed = Armed::default();
        armed.click("Reboot", 4.0);
        assert!(!armed.click("Shutdown", 4.0));
        assert_eq!(armed.name, Some("Shutdown"));
    }

    #[test]
    fn arming_expires() {
        let mut armed = Armed::default();
        armed.click("Suspend", 1.0);
        armed.tick(1.1);
        assert!(!armed.click("Suspend", 1.0), "an expired arm must not run");
    }
}
