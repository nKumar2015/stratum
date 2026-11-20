//! The overview: every workspace of the output it opens on, in a grid,
//! each a small copy of the output with pictures of its windows
//! (`window_capture`). Click a window to go to it, a workspace to show
//! it; drag a window onto another workspace to move it there. Arrows
//! choose a workspace, Return shows it, a digit shows that one, Escape
//! closes.
//!
//! It keeps no Hyprland state of its own: the shell reads the desktop when
//! it opens and whenever the compositor reports a change, and hands it in.

pub mod layout;

use std::collections::HashMap;

use smithay_client_toolkit::seat::keyboard::Keysym;

use crate::hyprland::{Desktop, Window};
use crate::render::display_list::{Color, DisplayList, Image, Point, Rect, TexturedQuad};
use crate::ui::animation::Easing;
use crate::ui::style::OverviewMetrics;
use crate::ui::{self, PaintCtx};
use crate::wallpapers::Preview;
use crate::window_capture::Thumb;

/// What the shell does after an event.
#[derive(Debug, PartialEq)]
pub enum OverviewAction {
    None,
    Redraw,
    Close,
    /// Go to this window (and its workspace), then close.
    FocusWindow(u64),
    /// Show this workspace, then close.
    FocusWorkspace(i32),
    /// Move the window to the workspace and stay open.
    MoveWindow { address: u64, workspace: i32 },
}

/// A press waiting to become a click or a drag.
#[derive(Clone, Copy, Debug)]
struct Press {
    at: Point,
    window: Option<u64>,
    card: Option<usize>,
}

/// A workspace as last painted, for finding what the pointer is over.
#[derive(Clone, Debug)]
struct Card {
    id: i32,
    rect: Rect,
    /// Its windows' rects, bottom first.
    windows: Vec<(u64, Rect)>,
}

pub struct Overview {
    metrics: OverviewMetrics,
    target: Option<String>,
    desktop: Desktop,
    /// Seconds since it opened, for the fade.
    opened: f32,
    /// The workspace the keys select, by its place in the grid.
    selected: usize,
    columns: usize,
    pointer: Option<Point>,
    press: Option<Press>,
    dragging: bool,
    cards: Vec<Card>,
}

impl Overview {
    pub fn new(metrics: &OverviewMetrics) -> Self {
        Self {
            metrics: metrics.clone(),
            target: None,
            desktop: Desktop::default(),
            opened: 0.0,
            selected: 0,
            columns: 1,
            pointer: None,
            press: None,
            dragging: false,
            cards: Vec::new(),
        }
    }

    pub fn set_metrics(&mut self, metrics: &OverviewMetrics) {
        self.metrics = metrics.clone();
    }

    pub fn is_open(&self) -> bool {
        self.target.is_some()
    }

    /// The output it is open on.
    pub fn target(&self) -> Option<&str> {
        self.target.as_deref()
    }

    /// Open on `output`, selecting the workspace it shows.
    pub fn open_on(&mut self, output: &str, desktop: Desktop) {
        self.target = Some(output.to_string());
        self.opened = 0.0;
        self.press = None;
        self.dragging = false;
        self.cards.clear();
        self.desktop = desktop;
        let active = self.desktop.monitor(output).map(|m| m.active_workspace);
        self.selected = self.workspace_ids().iter().position(|id| Some(*id) == active).unwrap_or(0);
    }

    pub fn close(&mut self) {
        self.target = None;
        self.desktop = Desktop::default();
        self.press = None;
        self.dragging = false;
        self.pointer = None;
        self.cards.clear();
    }

    /// A newer reading of the desktop while open.
    pub fn set_desktop(&mut self, desktop: Desktop) {
        if desktop == self.desktop {
            return;
        }
        self.desktop = desktop;
        let count = self.workspace_ids().len();
        self.selected = self.selected.min(count.saturating_sub(1));
    }

    /// The windows to take pictures of: the shown workspace's first.
    pub fn windows(&self) -> Vec<u64> {
        let Some(output) = &self.target else { return Vec::new() };
        let active = self.desktop.monitor(output).map(|m| m.active_workspace);
        let ids = self.workspace_ids();
        let mut windows: Vec<&Window> = self.desktop.windows.iter().filter(|w| ids.contains(&w.workspace)).collect();
        windows.sort_by_key(|w| (Some(w.workspace) != active, w.workspace));
        windows.into_iter().map(|w| w.address).collect()
    }

    fn workspace_ids(&self) -> Vec<i32> {
        match &self.target {
            Some(output) => self.desktop.workspaces_on(output).iter().map(|w| w.id).collect(),
            None => Vec::new(),
        }
    }

    pub fn tick(&mut self, dt: f32) {
        if self.is_open() {
            self.opened += dt;
        }
    }

    pub fn animating(&self) -> bool {
        self.is_open() && self.opened < self.metrics.open_ms as f32 / 1000.0
    }

    pub fn key(&mut self, keysym: Keysym, text: Option<&str>) -> OverviewAction {
        let count = self.workspace_ids().len();
        let moved = |overview: &mut Self, dx: i32, dy: i32| {
            overview.selected = layout::step(overview.selected, count, overview.columns, dx, dy);
            OverviewAction::Redraw
        };
        match keysym {
            Keysym::Escape => OverviewAction::Close,
            Keysym::Left => moved(self, -1, 0),
            Keysym::Right | Keysym::Tab => moved(self, 1, 0),
            Keysym::ISO_Left_Tab => moved(self, -1, 0),
            Keysym::Up => moved(self, 0, -1),
            Keysym::Down => moved(self, 0, 1),
            Keysym::Return | Keysym::KP_Enter | Keysym::space => self.show(self.selected),
            _ => {
                // 1 to 9 and 0 for the tenth, by place in the grid.
                let digit = text.and_then(|t| t.chars().next()).and_then(|c| c.to_digit(10));
                match digit {
                    Some(0) => self.show(9),
                    Some(d) => self.show(d as usize - 1),
                    None => OverviewAction::None,
                }
            }
        }
    }

    fn show(&self, index: usize) -> OverviewAction {
        match self.workspace_ids().get(index) {
            Some(id) => OverviewAction::FocusWorkspace(*id),
            None => OverviewAction::None,
        }
    }

    /// The topmost window and the card under `point`.
    fn hit(&self, point: Point) -> (Option<u64>, Option<usize>) {
        let Some(index) = self.cards.iter().position(|c| c.rect.contains(point)) else {
            return (None, None);
        };
        let window = self.cards[index].windows.iter().rev().find(|(_, r)| r.contains(point)).map(|(a, _)| *a);
        (window, Some(index))
    }

    pub fn pointer_motion(&mut self, point: Point) -> OverviewAction {
        self.pointer = Some(point);
        if let Some(press) = self.press.filter(|p| p.window.is_some()) {
            let (dx, dy) = (point.x - press.at.x, point.y - press.at.y);
            if !self.dragging && (dx * dx + dy * dy).sqrt() >= self.metrics.drag_threshold {
                self.dragging = true;
            }
        }
        OverviewAction::Redraw
    }

    pub fn pointer_leave(&mut self) -> OverviewAction {
        self.pointer = None;
        OverviewAction::Redraw
    }

    pub fn pointer_press(&mut self, point: Point) -> OverviewAction {
        self.pointer = Some(point);
        let (window, card) = self.hit(point);
        self.press = Some(Press { at: point, window, card });
        self.dragging = false;
        OverviewAction::Redraw
    }

    pub fn pointer_release(&mut self, point: Point) -> OverviewAction {
        self.pointer = Some(point);
        let Some(press) = self.press.take() else {
            return OverviewAction::None;
        };
        let dragging = std::mem::take(&mut self.dragging);
        let (_, card) = self.hit(point);
        if dragging {
            let (Some(address), Some(card)) = (press.window, card) else {
                return OverviewAction::Redraw;
            };
            let from = self.desktop.windows.iter().find(|w| w.address == address).map(|w| w.workspace);
            let to = self.cards[card].id;
            return if from == Some(to) {
                OverviewAction::Redraw
            } else {
                OverviewAction::MoveWindow { address, workspace: to }
            };
        }
        // A click: where it was pressed, if it was let go over the same card.
        match (press.window, press.card) {
            (Some(address), Some(pressed)) if card == Some(pressed) => OverviewAction::FocusWindow(address),
            (None, Some(pressed)) if card == Some(pressed) => OverviewAction::FocusWorkspace(self.cards[pressed].id),
            // Pressed and let go outside every workspace.
            (None, None) if card.is_none() => OverviewAction::Close,
            _ => OverviewAction::Redraw,
        }
    }

    pub fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        surface: Rect,
        thumbs: &HashMap<u64, Thumb>,
        wallpaper: Option<&Preview>,
    ) {
        let Some(output) = self.target.clone() else { return };
        let m = self.metrics.clone();
        let theme = ctx.theme.clone();
        let typo = ctx.metrics.typography.clone();
        let appear = Easing::EaseOut.apply(self.opened / (m.open_ms as f32 / 1000.0).max(0.001));
        let fade = |c: Color| c.with_alpha(c.a * appear);
        list.push_rect(Rect { color: fade(theme.bg_dark.with_alpha(m.scrim)), ..surface });

        let hint_line = typo.line(m.hint_text);
        let row = |y: f32, h: f32| Rect { x: surface.x, y, width: surface.width, height: h, radius: 0.0, color: Color::TRANSPARENT };
        let hint = "Click a window to go to it \u{b7} drag it to another workspace \u{b7} \u{2190}\u{2191}\u{2192}\u{2193} Enter \u{b7} Esc";
        let hint_y = surface.y + surface.height - m.margin * 0.5 - hint_line * 0.5;
        ui::paint_text_centered(ctx, list, hint, row(hint_y, hint_line), m.hint_text, fade(theme.text_muted));

        let Some(monitor) = self.desktop.monitor(&output).cloned() else {
            ui::paint_text_centered(ctx, list, "No workspaces to show", surface, m.label_text, fade(theme.text_muted));
            return;
        };
        let workspaces: Vec<(i32, String)> =
            self.desktop.workspaces_on(&output).iter().map(|w| (w.id, w.name.clone())).collect();
        let label = typo.line(m.label_text) + m.label_gap;
        let area = Rect {
            x: surface.x + m.margin,
            y: surface.y + m.margin,
            width: surface.width - m.margin * 2.0,
            height: surface.height - m.margin * 2.0 - hint_line,
            ..surface
        };
        let aspect = monitor.width.max(1) as f32 / monitor.height.max(1) as f32;
        let (rects, columns) = layout::grid(workspaces.len(), area, aspect, m.gap, label);
        self.columns = columns;
        let geometry = (monitor.x, monitor.y, monitor.width, monitor.height);

        // Cards and their windows, bottom first: tiled, floating, then
        // fullscreen, each least recently focused first.
        self.cards = workspaces
            .iter()
            .zip(&rects)
            .map(|((id, _), rect)| {
                let mut windows: Vec<(usize, &Window)> =
                    self.desktop.windows.iter().enumerate().filter(|(_, w)| w.workspace == *id).collect();
                windows.sort_by_key(|(recency, w)| (w.fullscreen, w.floating, std::cmp::Reverse(*recency)));
                let windows = windows
                    .into_iter()
                    .map(|(_, w)| (w.address, layout::place((w.x, w.y, w.width, w.height), geometry, *rect)))
                    .collect();
                Card { id: *id, rect: *rect, windows }
            })
            .collect();

        let dragged = self.press.and_then(|p| p.window).filter(|_| self.dragging);
        let (hovered, hovered_card) = match self.pointer {
            Some(point) => self.hit(point),
            None => (None, None),
        };
        let source = dragged.and_then(|a| self.desktop.windows.iter().find(|w| w.address == a)).map(|w| w.workspace);

        for (index, (card, (_, name))) in self.cards.iter().zip(&workspaces).enumerate() {
            let rect = card.rect;
            let active = card.id == monitor.active_workspace;
            let label_rect = Rect { y: rect.y - label, height: label - m.label_gap, ..rect };
            let weight = if active { typo.weight_bold } else { typo.weight_regular };
            let colour = if active { theme.text_main } else { theme.text_muted };
            ui::paint_text_centered_weighted(ctx, list, name, label_rect, m.label_text, weight, fade(colour));

            list.push_rect(Rect { color: fade(theme.surface), ..rect });
            if let Some(wallpaper) = wallpaper {
                list.push_texture(TexturedQuad {
                    key: wallpaper.key,
                    width: wallpaper.width,
                    height: wallpaper.height,
                    pixels: wallpaper.pixels.clone(),
                    corners: corners(rect),
                    uv: layout::cover_uv(rect, (wallpaper.width, wallpaper.height)),
                    alpha: appear,
                });
            }
            list.push_clip(rect);
            for (address, window_rect) in &card.windows {
                let alpha = if Some(*address) == dragged { 0.3 } else { 1.0 } * appear;
                self.paint_window(ctx, list, *address, *window_rect, alpha, thumbs);
                let (colour, width) = if Some(*address) == hovered && dragged.is_none() {
                    (theme.primary, m.outline_hover)
                } else {
                    (theme.outline, m.outline)
                };
                ui::stroke_rounded(list, *window_rect, colour.with_alpha(colour.a * alpha), width);
            }
            list.pop_clip();

            // Where a dragged window would go.
            let target = dragged.is_some() && hovered_card == Some(index) && source != Some(card.id);
            if target {
                list.push_rect(Rect { color: fade(theme.primary.with_alpha(0.18)), ..rect });
            }
            let (colour, width) = if target || active {
                (theme.primary, m.outline_active)
            } else if index == self.selected {
                (theme.text_main, m.outline_active)
            } else {
                (theme.outline, m.outline)
            };
            ui::stroke_rounded(list, rect, fade(colour), width);
            if index == self.selected && active {
                // The selection on the shown workspace: a second ring inside.
                let inner = inset(rect, m.outline_active * 2.0);
                ui::stroke_rounded(list, inner, fade(theme.text_main), m.outline);
            }
        }

        // The title of the window under the pointer, over everything else.
        if let (Some(address), None) = (hovered, dragged) {
            let window_rect = self.cards.iter().flat_map(|c| &c.windows).find(|(a, _)| *a == address).map(|(_, r)| *r);
            let title = self.desktop.windows.iter().find(|w| w.address == address).map(title_of);
            if let (Some(window_rect), Some(title)) = (window_rect, title) {
                self.paint_title(ctx, list, &title, window_rect, appear);
            }
        }

        // The dragged window follows the pointer, over every card.
        if let (Some(address), Some(press), Some(pointer)) = (dragged, self.press, self.pointer) {
            let from = self.cards.iter().flat_map(|c| &c.windows).find(|(a, _)| *a == address).map(|(_, r)| *r);
            if let Some(from) = from {
                list.push_layer();
                let ghost = Rect { x: from.x + pointer.x - press.at.x, y: from.y + pointer.y - press.at.y, ..from };
                self.paint_window(ctx, list, address, ghost, m.drag_alpha, thumbs);
                ui::stroke_rounded(list, ghost, theme.primary, m.outline_hover);
            }
        }
    }

    /// A window's picture, or a plain fill until it has one, with its app's
    /// icon.
    fn paint_window(
        &self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        address: u64,
        rect: Rect,
        alpha: f32,
        thumbs: &HashMap<u64, Thumb>,
    ) {
        let m = &self.metrics;
        match thumbs.get(&address) {
            Some(thumb) => list.push_texture(TexturedQuad {
                key: thumb.key,
                width: thumb.width,
                height: thumb.height,
                pixels: thumb.pixels.clone(),
                corners: corners(rect),
                uv: [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)],
                alpha,
            }),
            None => list.push_rect(Rect { color: ctx.theme.surface_high.with_alpha(alpha), ..rect }),
        }
        let Some(class) = self.desktop.windows.iter().find(|w| w.address == address).map(|w| w.class.clone()) else {
            return;
        };
        let size = m.icon.min(rect.width * 0.4).min(rect.height * 0.4);
        if size < 8.0 || alpha < 0.5 {
            return;
        }
        if let Some(icon) = ctx.icons.load_app_icon(&class, m.icon as u32, 2) {
            list.push_image(Image {
                width: icon.width,
                height: icon.height,
                pixels: icon.pixels.clone(),
                x: rect.x + (rect.width - size) * 0.5,
                y: rect.y + rect.height - size - (rect.height * 0.08).min(m.title_padding),
                width_logical: size,
                height_logical: size,
            });
        }
    }

    /// A pill with `title` across the middle of `window`.
    fn paint_title(&self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, title: &str, window: Rect, alpha: f32) {
        let m = &self.metrics;
        let typo = ctx.metrics.typography.clone();
        let line = typo.line(m.title_text);
        let max = (window.width - m.title_padding * 2.0).max(m.title_padding * 4.0);
        let text = ui::elide(ctx, title, m.title_text, typo.weight_medium, max - m.title_padding * 2.0);
        let width = ctx.measure(&text, m.title_text) + m.title_padding * 2.0;
        let pill = Rect {
            x: window.x + (window.width - width) * 0.5,
            y: window.y + (window.height - line - m.title_padding) * 0.5,
            width,
            height: line + m.title_padding,
            radius: m.title_radius,
            color: ctx.theme.bg_main.with_alpha(0.92 * alpha),
        };
        list.push_rect(pill);
        let colour = ctx.theme.text_main.with_alpha(alpha);
        ui::paint_text_centered_weighted(ctx, list, &text, pill, m.title_text, typo.weight_medium, colour);
    }
}

fn title_of(window: &Window) -> String {
    if window.title.is_empty() {
        window.class.clone()
    } else {
        window.title.clone()
    }
}

fn corners(rect: Rect) -> [Point; 4] {
    [
        Point::new(rect.x, rect.y),
        Point::new(rect.x + rect.width, rect.y),
        Point::new(rect.x + rect.width, rect.y + rect.height),
        Point::new(rect.x, rect.y + rect.height),
    ]
}

fn inset(rect: Rect, by: f32) -> Rect {
    Rect { x: rect.x + by, y: rect.y + by, width: (rect.width - by * 2.0).max(0.0), height: (rect.height - by * 2.0).max(0.0), ..rect }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hyprland::{Monitor, Workspace};

    fn desktop() -> Desktop {
        let window = |address, workspace, x| Window {
            address,
            class: "kitty".into(),
            title: format!("w{address}"),
            workspace,
            x,
            y: 0,
            width: 800,
            height: 1000,
            floating: false,
            fullscreen: false,
        };
        Desktop {
            monitors: vec![
                Monitor { name: "A".into(), x: 0, y: 0, width: 1600, height: 1000, active_workspace: 2 },
                Monitor { name: "B".into(), x: 1600, y: 0, width: 1600, height: 1000, active_workspace: 11 },
            ],
            workspaces: vec![
                Workspace { id: 1, name: "1".into(), monitor: "A".into() },
                Workspace { id: 2, name: "2".into(), monitor: "A".into() },
                Workspace { id: 3, name: "3".into(), monitor: "A".into() },
                Workspace { id: 11, name: "11".into(), monitor: "B".into() },
            ],
            windows: vec![window(0xa, 1, 0), window(0xb, 2, 800), window(0xc, 11, 1600), window(0xd, 2, 0)],
        }
    }

    fn metrics() -> OverviewMetrics {
        crate::config::Config::test_defaults().metrics.overview
    }

    /// An overview open on A with its cards laid out as a paint would.
    fn open() -> Overview {
        let mut overview = Overview::new(&metrics());
        overview.open_on("A", desktop());
        let rect = |x: f32| Rect { x, y: 0.0, width: 160.0, height: 100.0, radius: 0.0, color: Color::TRANSPARENT };
        overview.cards = vec![
            Card { id: 1, rect: rect(0.0), windows: vec![(0xa, Rect { width: 80.0, ..rect(0.0) })] },
            Card {
                id: 2,
                rect: rect(200.0),
                windows: vec![(0xd, Rect { width: 80.0, ..rect(200.0) }), (0xb, Rect { width: 80.0, ..rect(280.0) })],
            },
            Card { id: 3, rect: rect(400.0), windows: vec![] },
        ];
        overview.columns = 3;
        overview
    }

    #[test]
    fn opens_on_the_shown_workspace_and_watches_its_windows_first() {
        let overview = open();
        assert_eq!(overview.selected, 1);
        assert_eq!(overview.windows(), [0xb, 0xd, 0xa], "workspace 2 first; B's window left out");
    }

    #[test]
    fn a_click_goes_to_the_window_or_the_workspace() {
        let mut o = open();
        o.pointer_press(Point::new(300.0, 50.0));
        assert_eq!(o.pointer_release(Point::new(301.0, 50.0)), OverviewAction::FocusWindow(0xb));
        o.pointer_press(Point::new(450.0, 50.0));
        assert_eq!(o.pointer_release(Point::new(450.0, 50.0)), OverviewAction::FocusWorkspace(3));
        o.pointer_press(Point::new(700.0, 50.0));
        assert_eq!(o.pointer_release(Point::new(700.0, 50.0)), OverviewAction::Close, "outside every workspace");
    }

    #[test]
    fn dragging_a_window_to_another_workspace_moves_it() {
        let mut o = open();
        o.pointer_press(Point::new(20.0, 50.0));
        o.pointer_motion(Point::new(220.0, 50.0));
        assert!(o.dragging);
        assert_eq!(o.pointer_release(Point::new(450.0, 50.0)), OverviewAction::MoveWindow { address: 0xa, workspace: 3 });
        // Dropped back where it came from: nothing moves.
        o.pointer_press(Point::new(20.0, 50.0));
        o.pointer_motion(Point::new(60.0, 50.0));
        assert_eq!(o.pointer_release(Point::new(60.0, 50.0)), OverviewAction::Redraw);
    }

    #[test]
    fn a_small_wobble_is_still_a_click() {
        let mut o = open();
        o.pointer_press(Point::new(20.0, 50.0));
        o.pointer_motion(Point::new(22.0, 51.0));
        assert!(!o.dragging);
        assert_eq!(o.pointer_release(Point::new(22.0, 51.0)), OverviewAction::FocusWindow(0xa));
    }

    #[test]
    fn keys_choose_and_show_a_workspace() {
        let mut o = open();
        assert_eq!(o.key(Keysym::Right, None), OverviewAction::Redraw);
        assert_eq!(o.key(Keysym::Return, None), OverviewAction::FocusWorkspace(3));
        assert_eq!(o.key(Keysym::_1, Some("1")), OverviewAction::FocusWorkspace(1));
        assert_eq!(o.key(Keysym::_9, Some("9")), OverviewAction::None, "there is no ninth");
        assert_eq!(o.key(Keysym::Escape, None), OverviewAction::Close);
    }

    #[test]
    fn the_topmost_window_takes_the_pointer() {
        let o = open();
        // 0xb is above 0xd in card 2, and they do not overlap at 290.
        assert_eq!(o.hit(Point::new(290.0, 50.0)), (Some(0xb), Some(1)));
        assert_eq!(o.hit(Point::new(210.0, 50.0)), (Some(0xd), Some(1)));
    }
}
