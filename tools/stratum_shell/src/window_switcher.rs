//! The window switcher: a row of every window, most recently used first,
//! with pictures (`window_capture`). `switcher next` opens it on the
//! previous window, or moves on once open; `prev` goes back. It goes to the
//! window chosen when the modifier held to open it is let go (or on Return
//! or a click), and closes without going anywhere on Escape.
//!
//! Its surface is only as big as the row, centred on the output.

use std::collections::HashMap;

use smithay_client_toolkit::seat::keyboard::Keysym;

use crate::hyprland::Window;
use crate::render::display_list::{Color, DisplayList, Image, Point, Rect, TexturedQuad};
use crate::ui::animation::Easing;
use crate::ui::style::SwitcherMetrics;
use crate::ui::{self, PaintCtx};
use crate::window_capture::Thumb;

#[derive(Debug, PartialEq)]
pub enum SwitcherAction {
    None,
    Redraw,
    Close,
    /// Go to this window, then close.
    Focus(u64),
}

pub struct WindowSwitcher {
    metrics: SwitcherMetrics,
    target: Option<String>,
    windows: Vec<Window>,
    selected: usize,
    /// The first window in view, when there are more than fit.
    first: usize,
    /// How many fit, set when the surface is sized.
    visible: usize,
    /// Seconds since it opened, for the fade.
    opened: f32,
    /// Its surface has the keyboard: from then on, the modifiers being let
    /// go chooses.
    focused: bool,
    /// The modifier keys down when it got the keyboard, until each is let
    /// go.
    held: Vec<Keysym>,
    /// Each window's cell as last painted.
    cells: Vec<(usize, Rect)>,
}

impl WindowSwitcher {
    pub fn new(metrics: &SwitcherMetrics) -> Self {
        Self {
            metrics: metrics.clone(),
            target: None,
            windows: Vec::new(),
            selected: 0,
            first: 0,
            visible: 1,
            opened: 0.0,
            focused: false,
            held: Vec::new(),
            cells: Vec::new(),
        }
    }

    pub fn set_metrics(&mut self, metrics: &SwitcherMetrics) {
        self.metrics = metrics.clone();
    }

    pub fn is_open(&self) -> bool {
        self.target.is_some()
    }

    pub fn target(&self) -> Option<&str> {
        self.target.as_deref()
    }

    /// Open on `output` with `windows`, most recent first, on the previous
    /// window (`forward`) or the oldest. Nothing opens without a window.
    pub fn open_on(&mut self, output: &str, windows: Vec<Window>, forward: bool, output_width: f32) {
        if windows.is_empty() {
            return;
        }
        let count = windows.len();
        self.target = Some(output.to_string());
        self.windows = windows;
        self.selected = if forward { 1.min(count - 1) } else { count - 1 };
        self.first = 0;
        self.visible = Self::fit(&self.metrics, count, output_width);
        self.opened = 0.0;
        self.focused = false;
        self.held.clear();
        self.cells.clear();
        self.keep_in_view();
    }

    pub fn close(&mut self) {
        self.target = None;
        self.windows.clear();
        self.cells.clear();
        self.focused = false;
        self.held.clear();
    }

    /// The windows to take pictures of, from the selected one on.
    pub fn addresses(&self) -> Vec<u64> {
        let n = self.windows.len();
        (0..n).map(|i| self.windows[(self.selected + i) % n].address).collect()
    }

    /// How many cells fit across `output_width`.
    fn fit(m: &SwitcherMetrics, count: usize, output_width: f32) -> usize {
        cells_in(m, output_width - m.margin * 2.0).clamp(1, count.max(1))
    }

    /// The surface's size for `count` windows on an output `output_width`
    /// wide.
    pub fn size(m: &SwitcherMetrics, count: usize, output_width: f32) -> (u32, u32) {
        let shown = Self::fit(m, count, output_width) as f32;
        let width = m.padding * 2.0 + shown * m.cell_width + (shown - 1.0) * m.gap;
        let height = m.padding * 2.0 + m.cell_padding * 2.0 + m.thumb_height + m.title_height;
        (width.ceil() as u32, height.ceil() as u32)
    }

    pub fn tick(&mut self, dt: f32) {
        if self.is_open() {
            self.opened += dt;
        }
    }

    pub fn animating(&self) -> bool {
        self.is_open() && self.opened < self.metrics.open_ms as f32 / 1000.0
    }

    /// Move the choice by `delta`, around the ends.
    pub fn step(&mut self, delta: i32) -> SwitcherAction {
        let count = self.windows.len();
        if count == 0 {
            return SwitcherAction::None;
        }
        self.selected = (self.selected as i64 + delta as i64).rem_euclid(count as i64) as usize;
        self.keep_in_view();
        SwitcherAction::Redraw
    }

    fn keep_in_view(&mut self) {
        let visible = self.visible.max(1);
        if self.selected < self.first {
            self.first = self.selected;
        } else if self.selected >= self.first + visible {
            self.first = self.selected + 1 - visible;
        }
    }

    fn choose(&self) -> SwitcherAction {
        match self.windows.get(self.selected) {
            Some(window) => SwitcherAction::Focus(window.address),
            None => SwitcherAction::Close,
        }
    }

    /// Its surface got the keyboard with `keys` down. The modifier keys
    /// among them hold it open; with none (a quick tap, let go before the
    /// surface even had the keyboard) it chooses at once, which goes
    /// straight to the previous window.
    pub fn entered(&mut self, keys: &[Keysym]) -> SwitcherAction {
        self.focused = true;
        self.held = keys.iter().copied().filter(|k| holds_open(*k)).collect();
        if self.held.is_empty() {
            self.choose()
        } else {
            SwitcherAction::None
        }
    }

    /// A key was let go: the last of the held modifiers chooses.
    pub fn released(&mut self, keysym: Keysym) -> SwitcherAction {
        let before = self.held.len();
        self.held.retain(|k| *k != keysym);
        if self.is_open() && before > 0 && self.held.is_empty() {
            self.choose()
        } else {
            SwitcherAction::None
        }
    }

    /// The modifier state changed. Seen as well as the keys, for a key
    /// whose release never arrives: once no modifier that holds it open is
    /// down and no held key is still known to be, it chooses.
    pub fn modifiers(&mut self, held: bool) -> SwitcherAction {
        if self.is_open() && self.focused && !held && self.held.is_empty() {
            self.choose()
        } else {
            SwitcherAction::None
        }
    }

    pub fn key(&mut self, keysym: Keysym, shift: bool) -> SwitcherAction {
        match keysym {
            Keysym::Escape => SwitcherAction::Close,
            Keysym::Return | Keysym::KP_Enter | Keysym::space => self.choose(),
            Keysym::Tab if shift => self.step(-1),
            Keysym::Tab | Keysym::Right | Keysym::Down => self.step(1),
            Keysym::ISO_Left_Tab | Keysym::Left | Keysym::Up => self.step(-1),
            _ => SwitcherAction::None,
        }
    }

    fn cell_at(&self, point: Point) -> Option<usize> {
        self.cells.iter().find(|(_, r)| r.contains(point)).map(|(i, _)| *i)
    }

    pub fn on_hover(&mut self, point: Point) -> SwitcherAction {
        match self.cell_at(point) {
            Some(index) if index != self.selected => {
                self.selected = index;
                SwitcherAction::Redraw
            }
            _ => SwitcherAction::None,
        }
    }

    pub fn on_click(&mut self, point: Point) -> SwitcherAction {
        match self.cell_at(point) {
            Some(index) => {
                self.selected = index;
                self.choose()
            }
            None => SwitcherAction::None,
        }
    }

    pub fn paint(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, surface: Rect, thumbs: &HashMap<u64, Thumb>) {
        if !self.is_open() {
            return;
        }
        let m = self.metrics.clone();
        let theme = ctx.theme.clone();
        let typo = ctx.metrics.typography.clone();
        let appear = Easing::EaseOut.apply(self.opened / (m.open_ms as f32 / 1000.0).max(0.001));
        let fade = |c: Color| c.with_alpha(c.a * appear);
        let frame = Rect { radius: m.radius, ..surface };
        ui::paint_frame(list, frame, m.outline, fade(theme.outline), fade(theme.bg_main));

        // As many as fit, from the first in view.
        self.visible = cells_in(&m, surface.width).max(1);
        self.keep_in_view();
        self.cells.clear();
        let shown = self.first..(self.first + self.visible).min(self.windows.len());
        for (slot, index) in shown.enumerate() {
            let window = self.windows[index].clone();
            let cell = Rect {
                x: surface.x + m.padding + slot as f32 * (m.cell_width + m.gap),
                y: surface.y + m.padding,
                width: m.cell_width,
                height: surface.height - m.padding * 2.0,
                radius: m.cell_radius,
                color: Color::TRANSPARENT,
            };
            self.cells.push((index, cell));
            if index == self.selected {
                ui::paint_frame(list, cell, m.outline_selected, fade(theme.primary), fade(theme.bg_hover));
            }
            let area = Rect {
                x: cell.x + m.cell_padding,
                y: cell.y + m.cell_padding,
                width: cell.width - m.cell_padding * 2.0,
                height: m.thumb_height,
                ..cell
            };
            match thumbs.get(&window.address) {
                Some(thumb) => {
                    let picture = fit(area, thumb.width, thumb.height);
                    list.push_texture(TexturedQuad {
                        key: thumb.key,
                        width: thumb.width,
                        height: thumb.height,
                        pixels: thumb.pixels.clone(),
                        corners: [
                            Point::new(picture.x, picture.y),
                            Point::new(picture.x + picture.width, picture.y),
                            Point::new(picture.x + picture.width, picture.y + picture.height),
                            Point::new(picture.x, picture.y + picture.height),
                        ],
                        uv: [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)],
                        alpha: appear,
                    });
                    ui::stroke_rounded(list, Rect { radius: 0.0, ..picture }, fade(theme.outline), m.outline);
                }
                None => {
                    let shape = fit(area, window.width.max(1) as u32, window.height.max(1) as u32);
                    list.push_rect(Rect { radius: m.cell_radius * 0.5, color: fade(theme.surface_high), ..shape });
                    paint_icon(ctx, list, &window.class, m.placeholder_icon, shape, appear);
                }
            }

            // The app's icon and the title under the picture.
            let title_row = Rect { y: area.y + area.height, height: m.title_height, ..area };
            let icon_box = Rect { width: m.icon, ..title_row };
            paint_icon(ctx, list, &window.class, m.icon, icon_box, appear);
            let text_x = title_row.x + m.icon + m.cell_padding * 0.75;
            let room = title_row.x + title_row.width - text_x;
            let weight = if index == self.selected { typo.weight_medium } else { typo.weight_regular };
            let title = if window.title.is_empty() { window.class.clone() } else { window.title.clone() };
            let title = ui::elide(ctx, &title, m.title_text, weight, room);
            let colour = if index == self.selected { theme.text_main } else { theme.text_muted };
            ui::paint_text_vcentered(ctx, list, &title, text_x, title_row, m.title_text, weight, fade(colour));
        }
    }
}

/// How many cells fit in a row `width` wide, padding included. Half a
/// pixel is allowed for: a row sized for three cells measured 2.9999.
fn cells_in(m: &SwitcherMetrics, width: f32) -> usize {
    let room = width - m.padding * 2.0 + m.gap + 0.5;
    (room / (m.cell_width + m.gap)).floor().max(0.0) as usize
}

/// The modifier keys that hold the switcher open while down.
fn holds_open(keysym: Keysym) -> bool {
    matches!(
        keysym,
        Keysym::Super_L
            | Keysym::Super_R
            | Keysym::Alt_L
            | Keysym::Alt_R
            | Keysym::Meta_L
            | Keysym::Meta_R
            | Keysym::Hyper_L
            | Keysym::Hyper_R
            | Keysym::Control_L
            | Keysym::Control_R
    )
}

/// The largest rect of `width`:`height` inside `area`, centred.
fn fit(area: Rect, width: u32, height: u32) -> Rect {
    let scale = (area.width / width.max(1) as f32).min(area.height / height.max(1) as f32);
    let (w, h) = (width as f32 * scale, height as f32 * scale);
    Rect { x: area.x + (area.width - w) * 0.5, y: area.y + (area.height - h) * 0.5, width: w, height: h, ..area }
}

/// `class`'s icon, `size` across, centred in `area`.
fn paint_icon(ctx: &mut PaintCtx<'_>, list: &mut DisplayList, class: &str, size: f32, area: Rect, alpha: f32) {
    if alpha < 0.5 {
        return;
    }
    if let Some(icon) = ctx.icons.load_app_icon(class, size as u32, 2) {
        list.push_image(Image {
            width: icon.width,
            height: icon.height,
            pixels: icon.pixels.clone(),
            x: area.x + (area.width - size) * 0.5,
            y: area.y + (area.height - size) * 0.5,
            width_logical: size,
            height_logical: size,
        });
    } else {
        // nf-md-application.
        ui::paint_icon_centered(ctx, list, "\u{f08c6}", area, size * 0.8, ctx.theme.text_muted);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn windows(n: u64) -> Vec<Window> {
        (0..n).map(|address| Window { address, class: "kitty".into(), width: 800, height: 600, ..Default::default() }).collect()
    }

    fn metrics() -> SwitcherMetrics {
        crate::config::Config::test_defaults().metrics.switcher
    }

    #[test]
    fn opens_on_the_previous_window_or_the_oldest() {
        let mut s = WindowSwitcher::new(&metrics());
        s.open_on("A", windows(4), true, 2000.0);
        assert_eq!(s.selected, 1);
        s.open_on("A", windows(4), false, 2000.0);
        assert_eq!(s.selected, 3);
        s.open_on("A", windows(1), true, 2000.0);
        assert_eq!(s.selected, 0, "one window chooses itself");
        let mut empty = WindowSwitcher::new(&metrics());
        empty.open_on("A", Vec::new(), true, 2000.0);
        assert!(!empty.is_open());
    }

    #[test]
    fn letting_go_of_the_held_modifier_chooses() {
        let mut s = WindowSwitcher::new(&metrics());
        s.open_on("A", windows(3), true, 2000.0);
        assert_eq!(s.modifiers(false), SwitcherAction::None, "not before the keyboard arrives");
        assert_eq!(s.entered(&[Keysym::Super_L, Keysym::Tab]), SwitcherAction::None);
        // A modifier state that does not show the key (a virtual keyboard
        // with its own keymap) does not choose while the key is down.
        assert_eq!(s.modifiers(false), SwitcherAction::None);
        s.step(1);
        assert_eq!(s.released(Keysym::Tab), SwitcherAction::None);
        assert_eq!(s.released(Keysym::Super_L), SwitcherAction::Focus(2));
    }

    #[test]
    fn a_tap_let_go_before_the_keyboard_arrived_chooses_at_once() {
        let mut s = WindowSwitcher::new(&metrics());
        s.open_on("A", windows(3), true, 2000.0);
        assert_eq!(s.entered(&[]), SwitcherAction::Focus(1));
    }

    #[test]
    fn the_modifier_state_chooses_when_no_key_was_seen_down() {
        let mut s = WindowSwitcher::new(&metrics());
        s.open_on("A", windows(3), true, 2000.0);
        s.entered(&[Keysym::Alt_L]);
        s.held.clear(); // its release was lost
        assert_eq!(s.modifiers(true), SwitcherAction::None);
        assert_eq!(s.modifiers(false), SwitcherAction::Focus(1));
    }

    #[test]
    fn steps_wrap_and_keys_move() {
        let mut s = WindowSwitcher::new(&metrics());
        s.open_on("A", windows(3), true, 2000.0);
        s.step(1);
        s.step(1);
        assert_eq!(s.selected, 0);
        s.key(Keysym::ISO_Left_Tab, true);
        assert_eq!(s.selected, 2);
        assert_eq!(s.key(Keysym::Return, false), SwitcherAction::Focus(2));
        assert_eq!(s.key(Keysym::Escape, false), SwitcherAction::Close);
    }

    #[test]
    fn a_long_row_scrolls_to_keep_the_choice_in_view() {
        let m = metrics();
        // Room for three cells.
        let width = m.margin * 2.0 + m.padding * 2.0 + m.cell_width * 3.0 + m.gap * 2.0 + 1.0;
        let mut s = WindowSwitcher::new(&m);
        s.open_on("A", windows(8), false, width);
        assert_eq!(s.visible, 3);
        assert_eq!((s.selected, s.first), (7, 5));
        s.step(1);
        assert_eq!((s.selected, s.first), (0, 0));
        let (w, _) = WindowSwitcher::size(&m, 8, width);
        assert_eq!(w as f32, (m.padding * 2.0 + m.cell_width * 3.0 + m.gap * 2.0).ceil());
        assert_eq!(cells_in(&m, w as f32), 3, "a row sized for three holds three");
    }

    #[test]
    fn pictures_are_taken_from_the_choice_on() {
        let mut s = WindowSwitcher::new(&metrics());
        s.open_on("A", windows(4), true, 2000.0);
        assert_eq!(s.addresses(), [1, 2, 3, 0]);
    }
}
