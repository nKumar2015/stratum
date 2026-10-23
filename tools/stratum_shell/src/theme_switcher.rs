//! The theme switcher overlay.
//!
//! Matches `modules/ThemeSwitcher.qml`: a search field filtering the theme
//! names, Up/Down moving the selection (wrapping), Return applying it, Escape
//! or a click outside the card closing, hover selecting a row, and a click
//! applying it. Applying closes the switcher. Opened by
//! `stratum-cli theme open|toggle`.
//!
//! The QML list grew without bound; here at most `max_rows` rows are shown
//! and the list scrolls to keep the selection visible.
//!
//! A fullscreen overlay with exclusive keyboard focus on the output that was
//! focused when it opened, like the power menu.

use crate::render::display_list::{Color, DisplayList, Point, Rect};
use crate::ui::text_field::{self, FieldKey, FieldStyle, TextField};
use crate::ui::{self, PaintCtx};

/// Something the caller has to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SwitcherAction {
    None,
    Redraw,
    /// Switch to this theme. The switcher has already closed.
    Apply(String),
}

pub struct ThemeSwitcher {
    open: bool,
    target: Option<String>,
    search: TextField,
    /// Index into the filtered list.
    selection: Option<usize>,
    /// First filtered row shown.
    scroll: usize,
    /// Row rects from the last paint, with their filtered index.
    rows: Vec<(Rect, usize)>,
    card: Option<Rect>,
    metrics: crate::ui::style::ThemeSwitcherMetrics,
}

impl ThemeSwitcher {
    pub fn new(metrics: &crate::ui::style::ThemeSwitcherMetrics) -> Self {
        Self {
            open: false,
            target: None,
            search: TextField::new(),
            selection: None,
            scroll: 0,
            rows: Vec::new(),
            card: None,
            metrics: metrics.clone(),
        }
    }

    pub fn set_metrics(&mut self, metrics: &crate::ui::style::ThemeSwitcherMetrics) {
        self.metrics = metrics.clone();
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn target(&self) -> Option<&str> {
        self.open.then_some(self.target.as_deref()).flatten()
    }

    /// Open on `output` with an empty search and the current theme selected.
    pub fn open_on(&mut self, output: &str, themes: &[String], current: &str) {
        self.open = true;
        self.target = Some(output.to_string());
        self.search.clear();
        self.selection = themes.iter().position(|t| t == current).or((!themes.is_empty()).then_some(0));
        self.scroll = 0;
        self.rows.clear();
        self.card = None;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.target = None;
        self.search.clear();
        self.rows.clear();
        self.card = None;
    }

    pub fn toggle(&mut self, output: &str, themes: &[String], current: &str) {
        if self.open {
            self.close();
        } else {
            self.open_on(output, themes, current);
        }
    }

    /// Themes matching the search, in order.
    pub fn filtered<'a>(&self, themes: &'a [String]) -> Vec<&'a String> {
        let query = self.search.text().trim().to_lowercase();
        themes
            .iter()
            .filter(|name| query.is_empty() || name.to_lowercase().contains(&query))
            .collect()
    }

    fn clamp(&mut self, count: usize) {
        self.selection = match self.selection {
            _ if count == 0 => None,
            Some(i) if i < count => Some(i),
            _ => Some(0),
        };
    }

    fn apply(&mut self, themes: &[String], index: usize) -> SwitcherAction {
        let Some(name) = self.filtered(themes).get(index).map(|s| s.to_string()) else {
            return SwitcherAction::None;
        };
        self.close();
        SwitcherAction::Apply(name)
    }

    pub fn key(
        &mut self,
        keysym: smithay_client_toolkit::seat::keyboard::Keysym,
        text: Option<&str>,
        ctrl: bool,
        themes: &[String],
    ) -> SwitcherAction {
        use smithay_client_toolkit::seat::keyboard::Keysym;
        if !self.open {
            return SwitcherAction::None;
        }
        let count = self.filtered(themes).len();
        match keysym {
            Keysym::Up | Keysym::Down => {
                if count == 0 {
                    return SwitcherAction::None;
                }
                let delta = if keysym == Keysym::Down { 1 } else { count - 1 };
                self.selection = Some(match self.selection {
                    Some(i) if i < count => (i + delta) % count,
                    _ => 0,
                });
                return SwitcherAction::Redraw;
            }
            Keysym::Return | Keysym::KP_Enter => {
                return match self.selection {
                    Some(i) => self.apply(themes, i),
                    None => SwitcherAction::None,
                };
            }
            Keysym::Escape => {
                self.close();
                return SwitcherAction::Redraw;
            }
            _ => {}
        }
        match self.search.key(keysym, text, ctrl) {
            FieldKey::Changed => {
                // Keep the selection in range of the new results, as the QML
                // switcher did.
                self.clamp(self.filtered(themes).len());
                self.scroll = 0;
                SwitcherAction::Redraw
            }
            _ => SwitcherAction::None,
        }
    }

    /// Hovering a row selects it. Returns true when the selection changed.
    pub fn on_hover(&mut self, point: Point) -> bool {
        let hovered = self.rows.iter().find(|(r, _)| r.contains(point)).map(|(_, i)| *i);
        if hovered.is_some() && hovered != self.selection {
            self.selection = hovered;
            return true;
        }
        false
    }

    /// A click on a row applies it; a click outside the card closes.
    pub fn on_click(&mut self, point: Point, themes: &[String]) -> SwitcherAction {
        if !self.open {
            return SwitcherAction::None;
        }
        if let Some(index) = self.rows.iter().find(|(r, _)| r.contains(point)).map(|(_, i)| *i) {
            return self.apply(themes, index);
        }
        if self.card.is_some_and(|card| !card.contains(point)) {
            self.close();
            return SwitcherAction::Redraw;
        }
        SwitcherAction::None
    }

    /// Keep the selection within the shown window of rows.
    fn scroll_to_selection(&mut self, count: usize) {
        let rows = self.metrics.max_rows.max(1);
        if let Some(i) = self.selection {
            if i < self.scroll {
                self.scroll = i;
            } else if i >= self.scroll + rows {
                self.scroll = i + 1 - rows;
            }
        }
        self.scroll = self.scroll.min(count.saturating_sub(rows));
    }

    pub fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        surface: Rect,
        themes: &[String],
        current: &str,
    ) {
        if !self.open {
            return;
        }
        let m = self.metrics.clone();
        let theme = ctx.theme;
        let typo = ctx.metrics.typography.clone();
        let filtered: Vec<String> = self.filtered(themes).into_iter().cloned().collect();
        self.clamp(filtered.len());
        self.scroll_to_selection(filtered.len());
        let shown = filtered.len().min(m.max_rows.max(1));

        list.push_rect(Rect {
            color: Color::rgba(0.0, 0.0, 0.0, m.scrim_alpha),
            ..surface
        });

        let line = |size: f32| size * 1.4;
        let list_h = if filtered.is_empty() {
            m.empty_height
        } else {
            shown as f32 * m.row_height + (shown.saturating_sub(1)) as f32 * m.row_gap
        };
        let height = m.padding * 2.0
            + line(m.title)
            + m.spacing
            + line(m.subtitle)
            + m.spacing
            + m.search_height
            + m.spacing
            + list_h;
        let card = Rect {
            x: surface.x + (surface.width - m.width) * 0.5,
            y: surface.y + (surface.height - height) * 0.5,
            width: m.width,
            height,
            radius: m.radius,
            color: Color::TRANSPARENT,
        };
        text_field::paint_frame(list, card, m.border, theme.border_active, theme.bg_main);

        let x = card.x + m.padding;
        let inner = m.width - m.padding * 2.0;
        let row = |y: f32, h: f32| Rect {
            x,
            y,
            width: inner,
            height: h,
            radius: 0.0,
            color: Color::TRANSPARENT,
        };
        let mut y = card.y + m.padding;
        ui::paint_text_vcentered(ctx, list, "Theme Switcher", x, row(y, line(m.title)), m.title, typo.weight_bold, theme.text_main);
        y += line(m.title) + m.spacing;
        ui::paint_text_vcentered(ctx, list, &format!("Current: {current}"), x, row(y, line(m.subtitle)), m.subtitle, typo.weight_regular, theme.text_muted);
        y += line(m.subtitle) + m.spacing;

        let search = Rect {
            radius: m.search_radius,
            ..row(y, m.search_height)
        };
        text_field::paint_frame(list, search, m.border, theme.border_inactive, theme.bg_widget);
        self.search.paint(
            ctx,
            list,
            search,
            FieldStyle {
                text: m.search_text,
                padding: m.search_padding,
                caret_width: m.caret_width,
                masked: false,
                placeholder: Some("Search themes"),
            },
            false,
        );
        y += m.search_height + m.spacing;

        self.rows.clear();
        if filtered.is_empty() {
            let empty = Rect {
                radius: m.row_radius,
                ..row(y, m.empty_height)
            };
            text_field::paint_frame(list, empty, m.border, theme.border_inactive, theme.bg_widget);
            ui::paint_text_centered(ctx, list, "No matching themes", empty, m.empty_text, theme.text_muted);
        }
        for (offset, name) in filtered.iter().enumerate().skip(self.scroll).take(shown) {
            let rect = Rect {
                radius: m.row_radius,
                ..row(y, m.row_height)
            };
            let selected = self.selection == Some(offset);
            let is_current = name == current;
            if selected || is_current {
                text_field::paint_frame(
                    list,
                    rect,
                    m.border,
                    if selected { theme.border_active } else { theme.border_inactive },
                    theme.bg_hover,
                );
            }
            ui::paint_text_vcentered(
                ctx,
                list,
                name,
                rect.x + m.row_padding,
                rect,
                m.row_text,
                if is_current { typo.weight_bold } else { typo.weight_regular },
                if selected || is_current { theme.text_main } else { theme.text_muted },
            );
            self.rows.push((rect, offset));
            y += m.row_height + m.row_gap;
        }
        self.card = Some(card);
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use smithay_client_toolkit::seat::keyboard::Keysym;

    fn themes() -> Vec<String> {
        ["carbon", "catppuccin", "gruvbox", "nord", "rose-pine"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    fn typed(switcher: &mut ThemeSwitcher, text: &str) {
        for ch in text.chars() {
            switcher.key(Keysym::a, Some(&ch.to_string()), false, &themes());
        }
    }

    #[test]
    fn opens_on_the_current_theme() {
        let mut s = ThemeSwitcher::new(&crate::config::Config::test_defaults().metrics.theme_switcher);
        s.open_on("HDMI-A-1", &themes(), "nord");
        assert_eq!(s.target(), Some("HDMI-A-1"));
        assert_eq!(s.selection, Some(3));
    }

    #[test]
    fn search_filters_case_insensitively_and_clamps() {
        let mut s = ThemeSwitcher::new(&crate::config::Config::test_defaults().metrics.theme_switcher);
        s.open_on("a", &themes(), "rose-pine");
        assert_eq!(s.selection, Some(4));
        typed(&mut s, "CA");
        let names: Vec<_> = s.filtered(&themes()).into_iter().cloned().collect();
        assert_eq!(names, vec!["carbon", "catppuccin"]);
        assert_eq!(s.selection, Some(0), "selection is clamped into the results");
        typed(&mut s, "zz");
        assert!(s.filtered(&themes()).is_empty());
        assert_eq!(s.selection, None);
        assert_eq!(s.key(Keysym::Return, None, false, &themes()), SwitcherAction::None);
    }

    #[test]
    fn arrows_wrap_and_return_applies() {
        let mut s = ThemeSwitcher::new(&crate::config::Config::test_defaults().metrics.theme_switcher);
        s.open_on("a", &themes(), "carbon");
        s.key(Keysym::Up, None, false, &themes());
        assert_eq!(s.selection, Some(4), "up from the first wraps to the last");
        s.key(Keysym::Down, None, false, &themes());
        assert_eq!(s.selection, Some(0));
        s.key(Keysym::Down, None, false, &themes());
        assert_eq!(
            s.key(Keysym::Return, None, false, &themes()),
            SwitcherAction::Apply("catppuccin".into())
        );
        assert!(!s.is_open(), "applying closes the switcher");
    }

    #[test]
    fn escape_closes() {
        let mut s = ThemeSwitcher::new(&crate::config::Config::test_defaults().metrics.theme_switcher);
        s.toggle("a", &themes(), "carbon");
        assert!(s.is_open());
        s.key(Keysym::Escape, None, false, &themes());
        assert!(!s.is_open());
        assert_eq!(s.target(), None);
    }

    #[test]
    fn backspace_edits_the_search() {
        let mut s = ThemeSwitcher::new(&crate::config::Config::test_defaults().metrics.theme_switcher);
        s.open_on("a", &themes(), "carbon");
        typed(&mut s, "nordx");
        assert!(s.filtered(&themes()).is_empty());
        s.key(Keysym::BackSpace, None, false, &themes());
        assert_eq!(s.search.text(), "nord");
        assert_eq!(s.filtered(&themes()).len(), 1);
    }

    #[test]
    fn scrolling_keeps_the_selection_visible() {
        let mut s = ThemeSwitcher::new(&crate::config::Config::test_defaults().metrics.theme_switcher);
        s.metrics.max_rows = 2;
        s.open_on("a", &themes(), "carbon");
        s.selection = Some(4);
        s.scroll_to_selection(5);
        assert_eq!(s.scroll, 3, "rows 3 and 4 are shown");
        s.selection = Some(0);
        s.scroll_to_selection(5);
        assert_eq!(s.scroll, 0);
    }

    #[test]
    fn clicks_apply_rows_and_close_outside() {
        let mut s = ThemeSwitcher::new(&crate::config::Config::test_defaults().metrics.theme_switcher);
        s.open_on("a", &themes(), "carbon");
        let rect = |x: f32, y: f32, w: f32, h: f32| Rect {
            x,
            y,
            width: w,
            height: h,
            radius: 0.0,
            color: Color::TRANSPARENT,
        };
        s.card = Some(rect(100.0, 100.0, 360.0, 400.0));
        s.rows = vec![(rect(116.0, 300.0, 328.0, 42.0), 2)];
        assert!(s.on_hover(Point::new(200.0, 320.0)));
        assert_eq!(s.selection, Some(2));
        // Inside the card but not on a row: nothing.
        assert_eq!(s.on_click(Point::new(200.0, 150.0), &themes()), SwitcherAction::None);
        assert_eq!(
            s.on_click(Point::new(200.0, 320.0), &themes()),
            SwitcherAction::Apply("gruvbox".into())
        );
        s.open_on("a", &themes(), "carbon");
        s.card = Some(rect(100.0, 100.0, 360.0, 400.0));
        assert_eq!(s.on_click(Point::new(10.0, 10.0), &themes()), SwitcherAction::Redraw);
        assert!(!s.is_open());
    }
}
