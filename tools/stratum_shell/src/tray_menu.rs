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

/// Where the menu should appear: level with the icon that opened it. The
/// compositor puts it beside the rail (`wayland::surfaces::beside_rail`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Placement {
    /// Offset from the top edge of the output.
    pub y: f32,
}

/// The open menu and where it is anchored.
pub struct TrayMenuOverlay {
    /// Output the menu belongs to. Only that output's surface is mapped.
    target: Option<String>,
    placement: Placement,
    metrics: crate::ui::style::TrayMenuMetrics,
}

impl TrayMenuOverlay {
    pub fn new(metrics: &crate::ui::style::TrayMenuMetrics) -> Self {
        Self {
            target: None,
            placement: Placement::default(),
            metrics: metrics.clone(),
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
    /// Returns `None` when nothing should be shown. `measure` gives a label's
    /// width at `metrics.text`: the panel fits the widest, between `width`
    /// and `max_width`.
    pub fn size(&self, menu: Option<&TrayMenu>, mut measure: impl FnMut(&str) -> f32) -> Option<(f32, f32)> {
        let m = &self.metrics;
        let menu = menu?;
        let visible: Vec<_> = menu
            .entries
            .iter()
            .filter(|entry| entry.visible)
            .take(m.max_rows)
            .collect();
        if visible.is_empty() {
            return None;
        }
        let widest = visible
            .iter()
            .filter(|entry| !entry.separator)
            .map(|entry| {
                let chevron = if entry.has_children { m.chevron_width } else { 0.0 };
                entry.depth as f32 * m.indent + measure(&entry.label) + chevron
            })
            .fold(0.0, f32::max);
        let width = (widest + (m.padding + m.label_inset) * 2.0).clamp(m.width, m.max_width.max(m.width));
        let height = m.padding * 2.0 + visible.len() as f32 * m.row_height;
        Some((width.ceil(), height))
    }

    /// Map a click at `point` on a menu surface `width` wide to an entry id.
    pub fn on_click(&self, point: crate::render::display_list::Point, menu: &TrayMenu, width: f32) -> Option<i32> {
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
                width: width - self.metrics.padding * 2.0,
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
            let label_x = row.x + m.label_inset + entry.depth as f32 * m.indent;
            let chevron = if entry.has_children { m.chevron_width } else { 0.0 };
            let room = row.x + row.width - m.label_inset - chevron - label_x;
            let weight = ctx.metrics.typography.weight_regular;
            let label = ui::elide(ctx, &entry.label, m.text, weight, room);
            ui::paint_text_vcentered(ctx, list, &label, label_x, row, m.text, weight, color);

            // A submenu marker, since nested menus are shown inline rather than
            // as a flyout.
            if entry.has_children {
                ui::paint_text_vcentered(
                    ctx,
                    list,
                    "\u{f054}",
                    row.x + row.width - m.chevron_width,
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
        let overlay = TrayMenuOverlay::new(&crate::config::Config::test_defaults().metrics.tray_menu);
        let m = &overlay.metrics;
        let menu = menu(vec![entry(1, "One"), entry(2, "Two")]);
        let (width, height) = overlay.size(Some(&menu), |_| 20.0).unwrap();
        assert_eq!(width, m.width, "short labels keep the narrowest width");
        assert_eq!(height, m.padding * 2.0 + 2.0 * m.row_height);
    }

    #[test]
    fn width_fits_the_widest_label_up_to_the_maximum() {
        let overlay = TrayMenuOverlay::new(&crate::config::Config::test_defaults().metrics.tray_menu);
        let m = &overlay.metrics;
        let mut nested = entry(2, "Nested");
        nested.depth = 1;
        let short = menu(vec![entry(1, "One"), nested]);
        // 10 px per character: "Nested" is 60 wide, indented one level.
        let measure = |label: &str| label.len() as f32 * 10.0;
        let (width, _) = overlay.size(Some(&short), measure).unwrap();
        assert_eq!(width, m.width, "60 px of label still fits the narrowest panel");

        let long = menu(vec![entry(1, &"x".repeat(25))]);
        let (width, _) = overlay.size(Some(&long), measure).unwrap();
        assert_eq!(width, 250.0 + (m.padding + m.label_inset) * 2.0);

        let huge = menu(vec![entry(1, &"x".repeat(200))]);
        assert_eq!(overlay.size(Some(&huge), measure).unwrap().0, m.max_width);
    }

    #[test]
    fn hidden_entries_do_not_take_space_or_clicks() {
        let overlay = TrayMenuOverlay::new(&crate::config::Config::test_defaults().metrics.tray_menu);
        let mut hidden = entry(2, "Hidden");
        hidden.visible = false;
        let menu = menu(vec![entry(1, "One"), hidden, entry(3, "Three")]);
        let (_, height) = overlay.size(Some(&menu), |_| 0.0).unwrap();
        assert_eq!(
            height,
            overlay.metrics.padding * 2.0 + 2.0 * overlay.metrics.row_height
        );

        // The second visible row is id 3, not the hidden id 2.
        let y = overlay.metrics.padding + overlay.metrics.row_height * 1.5;
        let id = overlay
            .on_click(Point::new(10.0, y), &menu, overlay.metrics.width)
            .expect("row two is clickable");
        assert_eq!(id, 3);
    }

    #[test]
    fn separators_and_disabled_rows_are_not_clickable() {
        let overlay = TrayMenuOverlay::new(&crate::config::Config::test_defaults().metrics.tray_menu);
        let mut separator = entry(2, "");
        separator.separator = true;
        let mut disabled = entry(3, "Disabled");
        disabled.enabled = false;
        let menu = menu(vec![entry(1, "One"), separator, disabled]);

        let row = overlay.metrics.row_height;
        let pad = overlay.metrics.padding;
        let width = overlay.metrics.width;
        assert_eq!(overlay.on_click(Point::new(10.0, pad + row * 1.5), &menu, width), None);
        assert_eq!(overlay.on_click(Point::new(10.0, pad + row * 2.5), &menu, width), None);
        assert_eq!(
            overlay.on_click(Point::new(10.0, pad + row * 0.5), &menu, width),
            Some(1)
        );
    }

    #[test]
    fn empty_menu_has_no_size() {
        let overlay = TrayMenuOverlay::new(&crate::config::Config::test_defaults().metrics.tray_menu);
        assert_eq!(overlay.size(None, |_| 0.0), None);
        assert_eq!(overlay.size(Some(&menu(Vec::new())), |_| 0.0), None);
    }

    #[test]
    fn opening_pins_the_output_and_closing_forgets_it() {
        let mut overlay = TrayMenuOverlay::new(&crate::config::Config::test_defaults().metrics.tray_menu);
        assert_eq!(overlay.target(), None);
        overlay.open("eDP-2", Placement { y: 100.0 });
        assert_eq!(overlay.target(), Some("eDP-2"));
        assert_eq!(overlay.placement(), Placement { y: 100.0 });
        overlay.close();
        assert_eq!(overlay.target(), None);
    }
}
