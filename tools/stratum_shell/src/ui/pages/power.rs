//! Power page: session and system actions.

use crate::actions::SystemAction;
use crate::render::display_list::{Color, DisplayList, Rect};
use crate::ui::controls;
use crate::ui::pages::heading;
use crate::ui::{InputState, PaintCtx};

/// Entries are `(label, argv, accent)`. The accent marks the destructive ones so
/// they read differently from Lock.
const ACTIONS: [(&str, &[&str], bool); 4] = [
    ("Lock", &["loginctl", "lock-session"], false),
    ("Suspend", &["systemctl", "suspend"], false),
    ("Reboot", &["systemctl", "reboot"], false),
    ("Shutdown", &["systemctl", "poweroff"], true),
];

pub struct PowerPage;

impl PowerPage {
    pub fn new() -> Self {
        Self
    }

    pub fn tick(&mut self, _dt: f32) {}

    pub fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        area: Rect,
        input: &InputState,
        clicked: bool,
        pending: &mut Vec<SystemAction>,
    ) {
        let area = heading(ctx, list, area, "Power");
        for (index, (label, argv, accent)) in ACTIONS.iter().enumerate() {
            let rect = Rect {
                x: area.x,
                y: area.y + index as f32 * 48.0,
                width: area.width.min(280.0),
                height: 38.0,
                radius: 8.0,
                color: Color::TRANSPARENT,
            };
            if controls::button(ctx, list, rect, label, input, clicked, *accent) {
                pending.push(SystemAction::Spawn(
                    argv.iter().map(|s| s.to_string()).collect(),
                ));
            }
        }
    }
}

impl Default for PowerPage {
    fn default() -> Self {
        Self::new()
    }
}
