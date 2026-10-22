//! A tray item's context menu.
//!
//! Tray menus are fetched over `com.canonical.dbusmenu` and shown in an
//! overlay-layer surface anchored at the icon that was right-clicked. It owns
//! keyboard focus while open (Escape closes it) and closes as soon as an entry
//! is clicked, matching `sidebar/Tray.qml`.
//!
//! The menu is separate from the bar surface because the rail is only a few
//! dozen pixels wide; drawing the panel there would be clipped.

use crate::render::display_list::{Color, DisplayList, Rect};
use crate::tray::TrayMenu;
use crate::ui::{self, PaintCtx};

/// Where the menu should appear, in the coordinates of its own surface.
///
/// The menu's layer surface is anchored to the top-left of the output and moved
/// with margins, so these are always distances from that corner.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Placement {
    /// Offset from the left edge of the output.
    pub x: f32,
    /// Offset from the top edge of the output.
    pub y: f32,
}

impl Placement {
    /// Place the menu beside the rail on the side it occupies.
    ///
    /// A left rail opens its menu to the right of the rail; a right rail opens
    /// to the left, so the panel is not pushed off screen. `surface_width` is
    /// the output's logical width and `panel_width` the menu's.
    pub fn beside_rail(
        position: crate::config::SidebarPosition,
        rail_thickness: f32,
        surface_width: f32,
        panel_width: f32,
        item_y: f32,
    ) -> Self {
        let x = match position {
            crate::config::SidebarPosition::Left => rail_thickness,
            crate::config::SidebarPosition::Right => (surface_width - rail_thickness - panel_width).max(0.0),
            _ => rail_thickness,
        };
        Self { x, y: item_y }
    }
}

/// The open menu and where it is anchored.
pub struct TrayMenuOverlay {
    /// Output the menu belongs to. Only that output's surface is mapped.
    target: Option<String>,
    placement: Placement,
    metrics: crate::ui::style::TrayMenuMetrics,
}

impl TrayMenuOverlay {
    pub fn new() -> Self {
        Self {
            target: None,
            placement: Placement::default(),
            metrics: crate::ui::style::TrayMenuMetrics::default(),
        }
    }

    pub fn set_metrics(&mut self, metrics: &crate::ui::style::TrayMenuMetrics) {
        self.metrics = metrics.clone();
    }

    /// Anchor the menu on `output` at `placement`. Called when the item is
    /// right-clicked, before the menu contents are fetched.
    pub fn open(&mut self, output: &str, placement: Placement) {
        self.target = Some(output.to_string());
        self.placement = placement;
    }

    pub fn close(&mut self) {
        self.target = None;
    }

    pub fn target(&self) -> Option<&str> {
        self.target.as_deref()
    }

    pub fn placement(&self) -> Placement {
        self.placement
    }

    /// Surface size in logical pixels, given the menu contents.
    ///
    /// Returns `None` when nothing should be shown.
    pub fn size(&self, menu: Option<&TrayMenu>) -> Option<(f32, f32)> {
        let menu = menu?;
        let visible = menu
            .entries
            .iter()
            .filter(|entry| entry.visible)
            .count()
            .min(self.metrics.max_rows);
        if visible == 0 {
            return None;
        }
        let height = self.metrics.padding * 2.0 + visible as f32 * self.metrics.row_height;
        Some((self.metrics.width, height))
    }

    /// Map a click inside the menu surface to an entry id.
    pub fn on_click(&self, point: crate::render::display_list::Point, menu: &TrayMenu) -> Option<i32> {
        let visible: Vec<_> = menu
            .entries
            .iter()
            .filter(|entry| entry.visible)
            .take(self.metrics.max_rows)
            .collect();

        let mut y = self.metrics.padding;
        for entry in visible {
            let row = Rect {
                x: self.metrics.padding,
                y,
                width: self.metrics.width - self.metrics.padding * 2.0,
                height: self.metrics.row_height,
                radius: 0.0,
                color: Color::TRANSPARENT,
            };
            if row.contains(point) {
                return entry.selectable().then_some(entry.id);
            }
            y += self.metrics.row_height;
        }
        None
    }

    /// Paint the panel. Returns the rows that can be clicked, for input regions.
    pub fn paint(
        &self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        surface: Rect,
        menu: &TrayMenu,
        pointer: Option<(f32, f32)>,
    ) -> Vec<Rect> {
        let m = &self.metrics;
        ui::paint_frame(
            list,
            surface,
            ctx.theme.bg_main,
            ctx.theme.border_active,
            true,
        );

        let mut hits = Vec::new();
        let mut y = surface.y + m.padding;
        for entry in menu
            .entries
            .iter()
            .filter(|entry| entry.visible)
            .take(m.max_rows)
        {
            let row = Rect {
                x: surface.x + m.padding,
                y,
                width: surface.width - m.padding * 2.0,
                height: m.row_height,
                radius: 0.0,
                color: Color::TRANSPARENT,
            };

            if entry.separator {
                list.push_rect(Rect {
                    x: row.x,
                    y: row.y + row.height * 0.5,
                    width: row.width,
                    height: 1.0,
                    radius: 0.0,
                    color: ctx.theme.border_inactive,
                });
                y += m.row_height;
                continue;
            }

            let hovered = pointer
                .map(|(px, py)| row.contains(crate::render::display_list::Point::new(px, py)))
                .unwrap_or(false)
                && entry.selectable();
            if hovered {
                list.push_rect(Rect {
                    color: ctx.theme.bg_hover,
                    ..row
                });
            }

            let color = if entry.enabled && !entry.has_children {
                ctx.theme.text_main
            } else {
                ctx.theme.text_muted
            };
            ui::paint_text_vcentered(
                ctx,
                list,
                &entry.label,
                row.x + 8.0 + entry.depth as f32 * m.indent,
                row,
                m.text,
                ctx.metrics.typography.weight_regular,
                color,
            );

            // A submenu marker, since nested menus are shown inline rather than
            // as a flyout.
            if entry.has_children {
                ui::paint_text_vcentered(
                    ctx,
                    list,
                    "\u{f054}",
                    row.x + row.width - 18.0,
                    row,
                    ctx.metrics.typography.caption,
                    ctx.metrics.typography.weight_regular,
                    ctx.theme.text_muted,
                );
            }

            hits.push(row);
            y += m.row_height;
        }
        hits
    }
}

impl Default for TrayMenuOverlay {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tray::{MenuEntry, TrayMenu};
    use crate::render::display_list::Point;

    fn entry(id: i32, label: &str) -> MenuEntry {
        MenuEntry {
            id,
            path: format!("0/{id}"),
            label: label.to_string(),
            enabled: true,
            visible: true,
            separator: false,
            depth: 0,
            has_children: false,
        }
    }

    fn menu(entries: Vec<MenuEntry>) -> TrayMenu {
        TrayMenu {
            service: "org.example".into(),
            path: "/Menu".into(),
            entries,
        }
    }

    #[test]
    fn size_follows_the_visible_rows() {
        let overlay = TrayMenuOverlay::new();
        let m = &overlay.metrics;
        let menu = menu(vec![entry(1, "One"), entry(2, "Two")]);
        let (width, height) = overlay.size(Some(&menu)).unwrap();
        assert_eq!(width, m.width);
        assert_eq!(height, m.padding * 2.0 + 2.0 * m.row_height);
    }

    #[test]
    fn hidden_entries_do_not_take_space_or_clicks() {
        let overlay = TrayMenuOverlay::new();
        let mut hidden = entry(2, "Hidden");
        hidden.visible = false;
        let menu = menu(vec![entry(1, "One"), hidden, entry(3, "Three")]);
        let (_, height) = overlay.size(Some(&menu)).unwrap();
        assert_eq!(
            height,
            overlay.metrics.padding * 2.0 + 2.0 * overlay.metrics.row_height
        );

        // The second visible row is id 3, not the hidden id 2.
        let y = overlay.metrics.padding + overlay.metrics.row_height * 1.5;
        let id = overlay
            .on_click(Point::new(10.0, y), &menu)
            .expect("row two is clickable");
        assert_eq!(id, 3);
    }

    #[test]
    fn separators_and_disabled_rows_are_not_clickable() {
        let overlay = TrayMenuOverlay::new();
        let mut separator = entry(2, "");
        separator.separator = true;
        let mut disabled = entry(3, "Disabled");
        disabled.enabled = false;
        let menu = menu(vec![entry(1, "One"), separator, disabled]);

        let row = overlay.metrics.row_height;
        let pad = overlay.metrics.padding;
        assert_eq!(overlay.on_click(Point::new(10.0, pad + row * 1.5), &menu), None);
        assert_eq!(overlay.on_click(Point::new(10.0, pad + row * 2.5), &menu), None);
        assert_eq!(
            overlay.on_click(Point::new(10.0, pad + row * 0.5), &menu),
            Some(1)
        );
    }

    #[test]
    fn empty_menu_has_no_size() {
        let overlay = TrayMenuOverlay::new();
        assert_eq!(overlay.size(None), None);
        assert_eq!(overlay.size(Some(&menu(Vec::new()))), None);
    }

    #[test]
    fn placement_follows_the_rail_side() {
        use crate::config::SidebarPosition;

        // A left rail opens to its right.
        let left = Placement::beside_rail(SidebarPosition::Left, 40.0, 1920.0, 200.0, 120.0);
        assert_eq!(left.x, 40.0);
        assert_eq!(left.y, 120.0);

        // A right rail opens to its left, so the panel stays on screen.
        let right = Placement::beside_rail(SidebarPosition::Right, 40.0, 1920.0, 200.0, 120.0);
        assert_eq!(right.x, 1920.0 - 40.0 - 200.0);
        assert!(right.x + 200.0 <= 1920.0 - 40.0, "panel clears the rail");
    }

    #[test]
    fn opening_pins_the_output_and_closing_forgets_it() {
        let mut overlay = TrayMenuOverlay::new();
        assert_eq!(overlay.target(), None);
        overlay.open("eDP-2", Placement { x: 40.0, y: 100.0 });
        assert_eq!(overlay.target(), Some("eDP-2"));
        assert_eq!(overlay.placement(), Placement { x: 40.0, y: 100.0 });
        overlay.close();
        assert_eq!(overlay.target(), None);
    }
}
