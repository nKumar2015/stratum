//! The power menu overlay.
//!
//! A fullscreen overlay with one tile per session action, matching the QML
//! `PowerMenu.qml`. It takes exclusive keyboard focus while open, so Left/Right
//! move the selection, Return runs it, and Escape dismisses. External keybinds
//! still reach it through the IPC socket (`stratum-cli powermenu toggle`); the
//! shortcut itself stays in the compositor.
//!
//! Like the OSD, it lives on overlay-layer surfaces created per output at
//! startup. Only the output the menu opened on is painted and mapped, and only
//! that one is given keyboard interactivity — the others stay inert.

use crate::render::display_list::{Color, DisplayList, Rect};
use crate::ui::{self, PaintCtx};

/// One session action.
pub struct PowerOption {
    pub name: &'static str,
    /// Nerd Font glyph (Font Awesome codepoints, which Nerd Fonts carry).
    pub glyph: &'static str,
    pub argv: &'static [&'static str],
    /// Destructive entries are drawn with the error accent.
    pub danger: bool,
}

/// The entries and their order, copied from `modules/PowerMenu.qml`.
pub const OPTIONS: [PowerOption; 6] = [
    PowerOption {
        name: "Shutdown",
        glyph: "\u{f011}",
        argv: &["systemctl", "poweroff"],
        danger: true,
    },
    PowerOption {
        name: "Reboot",
        glyph: "\u{f021}",
        argv: &["systemctl", "reboot"],
        danger: false,
    },
    PowerOption {
        name: "Suspend",
        glyph: "\u{f186}",
        argv: &["systemctl", "suspend"],
        danger: false,
    },
    PowerOption {
        name: "Logout",
        glyph: "\u{f08b}",
        argv: &["hyprctl", "dispatch", "hl.dsp.exit()"],
        danger: false,
    },
    PowerOption {
        name: "Reboot into Windows",
        glyph: "\u{f17a}",
        argv: &["systemctl", "reboot", "--boot-loader-entry=windows.conf"],
        danger: false,
    },
    PowerOption {
        name: "Reboot into BIOS",
        glyph: "\u{f085}",
        argv: &["systemctl", "reboot", "--firmware-setup"],
        danger: false,
    },
];

/// What a keyboard event should do while the menu owns focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyAction {
    Previous,
    Next,
    Activate,
    Close,
}

/// The menu's visibility and selection.
pub struct PowerMenu {
    selection: usize,
    /// Output name the menu is open on, `None` while closed. Chosen when it
    /// opens and kept for its lifetime, so a focus change mid-interaction
    /// cannot move it.
    target: Option<String>,
    metrics: crate::ui::style::PowerMenuMetrics,
}

impl PowerMenu {
    pub fn new(metrics: &crate::ui::style::PowerMenuMetrics) -> Self {
        Self {
            selection: 0,
            target: None,
            metrics: metrics.clone(),
        }
    }

    pub fn set_metrics(&mut self, metrics: &crate::ui::style::PowerMenuMetrics) {
        self.metrics = metrics.clone();
    }

    pub fn is_open(&self) -> bool {
        self.target.is_some()
    }

    /// Show on `output`, resetting the selection the way the QML menu does.
    pub fn open_on(&mut self, output: &str) {
        self.selection = 0;
        self.target = Some(output.to_string());
    }

    pub fn close(&mut self) {
        self.target = None;
    }

    /// Flip the menu for the given output. Opening pins the output; closing
    /// ignores it.
    pub fn toggle(&mut self, output: &str) {
        if self.is_open() {
            self.close();
        } else {
            self.open_on(output);
        }
    }

    /// The output whose surface should be mapped and focused.
    pub fn target(&self) -> Option<&str> {
        self.target.as_deref()
    }

    pub fn selection(&self) -> usize {
        self.selection
    }

    /// Map a keypress to an action. Returns `None` for keys the menu ignores,
    /// so the caller can leave them for the focused window.
    pub fn key_action(&self, keysym: smithay_client_toolkit::seat::keyboard::Keysym) -> Option<KeyAction> {
        use smithay_client_toolkit::seat::keyboard::Keysym;
        match keysym {
            Keysym::Escape => Some(KeyAction::Close),
            Keysym::Left => Some(KeyAction::Previous),
            Keysym::Right => Some(KeyAction::Next),
            Keysym::Return | Keysym::KP_Enter => Some(KeyAction::Activate),
            _ => None,
        }
    }

    /// Apply a navigation action. Returns the argv to run when activating.
    pub fn apply(&mut self, action: KeyAction) -> Option<Vec<String>> {
        match action {
            KeyAction::Previous => {
                self.selection = if self.selection == 0 {
                    OPTIONS.len() - 1
                } else {
                    self.selection - 1
                };
                None
            }
            KeyAction::Next => {
                self.selection = (self.selection + 1) % OPTIONS.len();
                None
            }
            KeyAction::Activate => {
                let argv = OPTIONS
                    .get(self.selection)
                    .map(|option| option.argv.iter().map(|s| s.to_string()).collect());
                self.close();
                argv
            }
            KeyAction::Close => {
                self.close();
                None
            }
        }
    }

    /// Tile geometry, shared by painting and hit-testing so the two cannot
    /// drift apart.
    fn tile_rect(&self, surface: Rect, index: usize) -> Rect {
        let m = &self.metrics;
        let count = OPTIONS.len() as f32;
        let row_width = count * m.tile_width + (count - 1.0) * m.tile_gap;
        let row_height = m.tile_height;
        let origin_x = surface.x + (surface.width - row_width) * 0.5;
        let origin_y = surface.y + (surface.height - row_height) * 0.5;
        Rect {
            x: origin_x + index as f32 * (m.tile_width + m.tile_gap),
            y: origin_y,
            width: m.tile_width,
            height: m.tile_height,
            radius: m.tile_radius,
            color: Color::TRANSPARENT,
        }
    }

    /// Route a click. Returns the argv to run when a tile was hit.
    pub fn on_click(&mut self, point: crate::render::display_list::Point, surface: Rect) -> Option<Vec<String>> {
        if !self.is_open() {
            return None;
        }
        for index in 0..OPTIONS.len() {
            if self.tile_rect(surface, index).contains(point) {
                self.selection = index;
                return self.apply(KeyAction::Activate);
            }
        }
        None
    }

    /// Paint the scrim and the row of tiles.
    pub fn paint(&self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, surface: Rect) {
        if !self.is_open() {
            return;
        }
        let m = &self.metrics;

        // Full-screen scrim, matching the QML `#a0000000`.
        ui::paint_scrim(list, surface, m.scrim_alpha);

        for (index, option) in OPTIONS.iter().enumerate() {
            let tile = self.tile_rect(surface, index);
            let active = index == self.selection;
            ui::paint_frame(
                list,
                tile,
                ctx.appearance.border_width,
                if active { ctx.theme.border_active } else { ctx.theme.border_inactive },
                if active { ctx.theme.bg_hover } else { ctx.theme.bg_widget },
            );

            // Matching the QML menu: the active tile's icon is tinted with the
            // secondary colour, everything else stays dark. Shutdown is not
            // permanently red — only the selection marks it.
            let icon_color = if active {
                if option.danger {
                    ctx.theme.error
                } else {
                    ctx.theme.secondary
                }
            } else {
                ctx.theme.bg_dark
            };
            ui::paint_text_centered(
                ctx,
                list,
                option.glyph,
                Rect {
                    x: tile.x,
                    y: tile.y + m.panel_padding,
                    width: tile.width,
                    height: m.tile_height - m.panel_padding * 2.0,
                    radius: 0.0,
                    color: Color::TRANSPARENT,
                },
                m.icon,
                icon_color,
            );
            ui::paint_text_centered(
                ctx,
                list,
                option.name,
                Rect {
                    x: tile.x,
                    y: tile.y + tile.height - m.panel_padding - m.label,
                    width: tile.width,
                    height: ctx.metrics.typography.line(m.label),
                    radius: 0.0,
                    color: Color::TRANSPARENT,
                },
                m.label,
                if active {
                    ctx.theme.text_main
                } else {
                    ctx.theme.text_muted
                },
            );
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn surface() -> Rect {
        Rect {
            x: 0.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
            radius: 0.0,
            color: Color::TRANSPARENT,
        }
    }

    #[test]
    fn toggle_opens_on_the_given_output_and_closes() {
        let mut menu = PowerMenu::new(&crate::config::Config::test_defaults().metrics.power_menu);
        assert_eq!(menu.target(), None);
        menu.toggle("eDP-2");
        assert!(menu.is_open());
        assert_eq!(menu.target(), Some("eDP-2"));
        menu.toggle("HDMI-A-1");
        assert!(!menu.is_open());
        assert_eq!(menu.target(), None);
    }

    #[test]
    fn opening_always_resets_the_selection() {
        let mut menu = PowerMenu::new(&crate::config::Config::test_defaults().metrics.power_menu);
        menu.open_on("a");
        menu.apply(KeyAction::Next);
        assert_eq!(menu.selection(), 1);
        menu.close();
        menu.open_on("a");
        assert_eq!(menu.selection(), 0, "a fresh open starts at the first tile");
    }

    #[test]
    fn navigation_wraps_at_both_ends() {
        let mut menu = PowerMenu::new(&crate::config::Config::test_defaults().metrics.power_menu);
        menu.open_on("a");
        menu.apply(KeyAction::Previous);
        assert_eq!(menu.selection(), OPTIONS.len() - 1);
        menu.apply(KeyAction::Next);
        assert_eq!(menu.selection(), 0);
    }

    #[test]
    fn activation_returns_the_command_and_closes() {
        let mut menu = PowerMenu::new(&crate::config::Config::test_defaults().metrics.power_menu);
        menu.open_on("a");
        menu.apply(KeyAction::Next);
        let argv = menu.apply(KeyAction::Activate).expect("reboot argv");
        assert_eq!(argv, vec!["systemctl", "reboot"]);
        assert!(!menu.is_open());
    }

    #[test]
    fn escape_closes_without_running_anything() {
        let mut menu = PowerMenu::new(&crate::config::Config::test_defaults().metrics.power_menu);
        menu.open_on("a");
        assert_eq!(menu.apply(KeyAction::Close), None);
        assert!(!menu.is_open());
    }

    #[test]
    fn only_known_keys_are_handled() {
        use smithay_client_toolkit::seat::keyboard::Keysym;
        let menu = PowerMenu::new(&crate::config::Config::test_defaults().metrics.power_menu);
        assert_eq!(menu.key_action(Keysym::Escape), Some(KeyAction::Close));
        assert_eq!(menu.key_action(Keysym::Left), Some(KeyAction::Previous));
        assert_eq!(menu.key_action(Keysym::Right), Some(KeyAction::Next));
        assert_eq!(menu.key_action(Keysym::Return), Some(KeyAction::Activate));
        assert_eq!(menu.key_action(Keysym::space), None);
    }

    #[test]
    fn tiles_are_centred_and_do_not_overlap() {
        let menu = PowerMenu::new(&crate::config::Config::test_defaults().metrics.power_menu);
        let surface = surface();
        let first = menu.tile_rect(surface, 0);
        let second = menu.tile_rect(surface, 1);
        assert!(first.x + first.width <= second.x, "tiles do not overlap");
        let last = menu.tile_rect(surface, OPTIONS.len() - 1);
        let left = first.x - surface.x;
        let right = surface.x + surface.width - (last.x + last.width);
        assert!((left - right).abs() < 0.01, "row is centred");
    }

    #[test]
    fn clicking_a_tile_selects_and_activates_it() {
        let mut menu = PowerMenu::new(&crate::config::Config::test_defaults().metrics.power_menu);
        menu.open_on("a");
        let surface = surface();
        let tile = menu.tile_rect(surface, 2);
        let point = crate::render::display_list::Point::new(
            tile.x + tile.width * 0.5,
            tile.y + tile.height * 0.5,
        );
        let argv = menu.on_click(point, surface).expect("suspend argv");
        assert_eq!(argv, vec!["systemctl", "suspend"]);
        assert!(!menu.is_open());
    }

    #[test]
    fn clicking_outside_every_tile_does_nothing() {
        let mut menu = PowerMenu::new(&crate::config::Config::test_defaults().metrics.power_menu);
        menu.open_on("a");
        let point = crate::render::display_list::Point::new(2.0, 2.0);
        assert_eq!(menu.on_click(point, surface()), None);
        assert!(menu.is_open());
    }
}
