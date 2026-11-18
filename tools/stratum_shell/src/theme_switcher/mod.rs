//! The theme switcher: a band of slanted slices across the output it opens
//! on, one per theme, over the desktop rather than in a card.
//!
//! The focused slice opens wide and the band slides to keep it centred.
//! A slice shows the theme's linked wallpaper (decoded large by the
//! wallpaper worker only while the switcher is open) over art made of the
//! theme's palette. Moving the focus previews the theme on the whole shell
//! at once, and its wallpaper once the focus has rested
//! (`[theme_switcher] wallpaper_delay_ms`), so browsing quickly does not
//! make hyprpaper load every image passed. Return keeps the focused theme;
//! Escape (after clearing the search) or a click outside the band goes
//! back to the theme and wallpaper from before.
//!
//! Left/Right (and Up/Down, Tab), Home/End and the wheel move the focus.
//! Typing filters the themes, as the QML switcher's search field did. A
//! click on a slice focuses it; a click on the focused slice keeps it.
//! Hovering only highlights: the band slides under the pointer, so a
//! focus that followed it would chase the mouse.
//!
//! A fullscreen overlay with exclusive keyboard focus on the output that was
//! focused when it opened, like the power menu. Opened by
//! `stratum-cli theme open|toggle`.

mod layout;

use std::collections::BTreeMap;

use crate::config::Theme;
use crate::render::display_list::{Color, DisplayList, Paint, Path, PathCommand, Point, Rect, TexturedQuad};
use crate::ui::animation::{Easing, Tween};
use crate::ui::style::ThemeSwitcherMetrics;
use crate::ui::text_field::{FieldKey, FieldStyle, TextField};
use crate::ui::{self, PaintCtx};
use layout::{Quad, Slice};

/// Something the caller has to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SwitcherAction {
    None,
    Redraw,
    /// Show this theme live, without keeping it.
    Preview(String),
    /// Show this theme's wallpaper, or for a theme without one, the
    /// wallpaper from before.
    PreviewWallpaper(String),
    /// Keep this theme. The switcher is closing.
    Apply(String),
    /// Go back to the theme and wallpaper from before. The switcher is
    /// closing.
    Cancel,
}

/// How a slice is painted where it has no wallpaper: columns of its
/// palette at the slice's slant, as fractions of its width.
const PALETTE_ART: [f32; 5] = [0.55, 0.15, 0.10, 0.10, 0.10];

pub struct ThemeSwitcher {
    /// The output it is shown on, `None` once it has faded out.
    target: Option<String>,
    /// Seconds since it began to fade out; `None` while open.
    closing: Option<f32>,
    /// Seconds since it opened.
    opened: f32,
    search: TextField,
    /// The focused theme's index among the filtered themes, and its name.
    focus: usize,
    focused: Option<String>,
    /// The focus the band is drawn at, sliding toward `focus`.
    slide: Tween,
    /// The theme shown when it opened.
    original: String,
    /// Seconds the focus has rested since it moved.
    rest: f32,
    /// The focused theme's wallpaper is still to be previewed.
    wallpaper_due: bool,
    /// Wheel movement not yet acted on, in notches.
    wheel: f32,
    /// The slice under the pointer.
    hovered: Option<usize>,
    /// Slices from the last paint, for hit testing, and the band.
    slices: Vec<Slice>,
    band: Option<Rect>,
    /// Seconds each wallpaper has been shown, by preview key, to fade in.
    fades: BTreeMap<u64, f32>,
    metrics: ThemeSwitcherMetrics,
    wallpaper_delay: f32,
}

fn seconds(ms: u32) -> f32 {
    ms as f32 / 1000.0
}

impl ThemeSwitcher {
    pub fn new(metrics: &ThemeSwitcherMetrics, wallpaper_delay_ms: u32) -> Self {
        Self {
            target: None,
            closing: None,
            opened: 0.0,
            search: TextField::new(),
            focus: 0,
            focused: None,
            slide: Tween::new(0.0),
            original: String::new(),
            rest: 0.0,
            wallpaper_due: false,
            wheel: 0.0,
            hovered: None,
            slices: Vec::new(),
            band: None,
            fades: BTreeMap::new(),
            metrics: metrics.clone(),
            wallpaper_delay: seconds(wallpaper_delay_ms),
        }
    }

    pub fn set_config(&mut self, metrics: &ThemeSwitcherMetrics, wallpaper_delay_ms: u32) {
        self.metrics = metrics.clone();
        self.wallpaper_delay = seconds(wallpaper_delay_ms);
    }

    /// Open and taking input; false while fading out.
    pub fn is_open(&self) -> bool {
        self.target.is_some() && self.closing.is_none()
    }

    /// The output it is shown on, fading out included.
    pub fn target(&self) -> Option<&str> {
        self.target.as_deref()
    }

    /// Open on `output` with an empty search and `current` focused.
    pub fn open_on(&mut self, output: &str, themes: &[String], current: &str) {
        self.target = Some(output.to_string());
        self.closing = None;
        self.opened = 0.0;
        self.search.clear();
        self.original = current.to_string();
        self.focus = themes.iter().position(|t| t == current).unwrap_or(0);
        self.focused = themes.get(self.focus).cloned();
        self.slide.snap(self.focus as f32);
        self.rest = 0.0;
        self.wallpaper_due = false;
        self.wheel = 0.0;
        self.hovered = None;
        self.slices.clear();
        self.band = None;
        self.fades.clear();
    }

    /// Close without keeping anything.
    pub fn cancel(&mut self) -> SwitcherAction {
        if !self.is_open() {
            return SwitcherAction::None;
        }
        self.begin_close();
        SwitcherAction::Cancel
    }

    fn begin_close(&mut self) {
        self.closing = Some(0.0);
        self.wallpaper_due = false;
        self.hovered = None;
    }

    /// Themes matching the search, in order.
    pub fn filtered<'a>(&self, themes: &'a [String]) -> Vec<&'a String> {
        let query = self.search.text().trim().to_lowercase();
        themes
            .iter()
            .filter(|name| query.is_empty() || name.to_lowercase().contains(&query))
            .collect()
    }

    /// Focus `index` of the filtered themes, sliding there or jumping.
    fn focus_on(&mut self, index: usize, themes: &[String], slide: bool) -> SwitcherAction {
        let names = self.filtered(themes);
        let Some(name) = names.get(index).map(|s| s.to_string()) else {
            return SwitcherAction::Redraw;
        };
        self.focus = index;
        if slide {
            self.slide.to(index as f32, seconds(self.metrics.slide_ms), Easing::EaseOut);
        } else {
            self.slide.snap(index as f32);
        }
        if self.focused.as_deref() == Some(name.as_str()) {
            return SwitcherAction::Redraw;
        }
        self.focused = Some(name.clone());
        self.rest = 0.0;
        self.wallpaper_due = true;
        SwitcherAction::Preview(name)
    }

    /// Move the focus `delta` themes, wrapping.
    fn step(&mut self, delta: isize, themes: &[String]) -> SwitcherAction {
        let count = self.filtered(themes).len() as isize;
        if count == 0 {
            return SwitcherAction::None;
        }
        let index = (self.focus as isize + delta).rem_euclid(count) as usize;
        self.focus_on(index, themes, true)
    }

    /// After the search changed: keep the focused theme if it still
    /// matches, else focus the first match.
    fn refilter(&mut self, themes: &[String]) -> SwitcherAction {
        let names = self.filtered(themes);
        let index = self.focused.as_ref().and_then(|f| names.iter().position(|n| *n == f)).unwrap_or(0);
        self.hovered = None;
        self.focus_on(index, themes, false)
    }

    fn keep_focused(&mut self, themes: &[String]) -> SwitcherAction {
        let Some(name) = self.filtered(themes).get(self.focus).map(|s| s.to_string()) else {
            return SwitcherAction::None;
        };
        self.begin_close();
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
        if !self.is_open() {
            return SwitcherAction::None;
        }
        match keysym {
            Keysym::Left | Keysym::Up | Keysym::ISO_Left_Tab => return self.step(-1, themes),
            Keysym::Right | Keysym::Down | Keysym::Tab => return self.step(1, themes),
            Keysym::Home => return self.focus_on(0, themes, true),
            Keysym::End => {
                let last = self.filtered(themes).len().saturating_sub(1);
                return self.focus_on(last, themes, true);
            }
            Keysym::Return | Keysym::KP_Enter => return self.keep_focused(themes),
            Keysym::Escape => {
                if self.search.text().is_empty() {
                    return self.cancel();
                }
                self.search.clear();
                return self.refilter(themes);
            }
            _ => {}
        }
        match self.search.key(keysym, text, ctrl) {
            FieldKey::Changed => self.refilter(themes),
            _ => SwitcherAction::None,
        }
    }

    /// The wheel moves the focus a theme per notch.
    pub fn on_wheel(&mut self, notches: f32, themes: &[String]) -> SwitcherAction {
        if !self.is_open() {
            return SwitcherAction::None;
        }
        self.wheel += notches;
        let whole = self.wheel.trunc();
        if whole == 0.0 {
            return SwitcherAction::None;
        }
        self.wheel -= whole;
        self.step(whole as isize, themes)
    }

    fn slice_at(&self, point: Point) -> Option<usize> {
        self.slices.iter().find(|s| layout::contains(&s.quad, point)).map(|s| s.index)
    }

    /// Highlight the slice under the pointer.
    pub fn on_hover(&mut self, point: Point) -> SwitcherAction {
        if !self.is_open() {
            return SwitcherAction::None;
        }
        let hovered = self.slice_at(point);
        if hovered == self.hovered {
            return SwitcherAction::None;
        }
        self.hovered = hovered;
        SwitcherAction::Redraw
    }

    /// A click on a slice focuses it, or keeps it when already focused; a
    /// click outside the band goes back.
    pub fn on_click(&mut self, point: Point, themes: &[String]) -> SwitcherAction {
        if !self.is_open() {
            return SwitcherAction::None;
        }
        match self.slice_at(point) {
            Some(index) if index == self.focus => self.keep_focused(themes),
            Some(index) => self.focus_on(index, themes, true),
            None if self.band.is_some_and(|band| band.contains(point)) => SwitcherAction::None,
            None => self.cancel(),
        }
    }

    /// Advance the animations and the wallpaper's delay.
    pub fn tick(&mut self, dt: f32) -> SwitcherAction {
        if self.target.is_none() {
            return SwitcherAction::None;
        }
        if let Some(closing) = &mut self.closing {
            *closing += dt;
            if *closing >= seconds(self.metrics.close_ms) {
                self.target = None;
                self.closing = None;
                self.slices.clear();
                self.band = None;
                self.fades.clear();
            }
            return SwitcherAction::Redraw;
        }
        self.opened += dt;
        self.slide.tick(dt);
        for shown in self.fades.values_mut() {
            *shown += dt;
        }
        if self.wallpaper_due {
            self.rest += dt;
            if self.rest >= self.wallpaper_delay {
                self.wallpaper_due = false;
                if let Some(name) = &self.focused {
                    return SwitcherAction::PreviewWallpaper(name.clone());
                }
            }
        }
        SwitcherAction::None
    }

    /// Whether frames must keep coming: something moves, or a wallpaper
    /// preview is waiting for its delay.
    pub fn animating(&self) -> bool {
        if self.target.is_none() {
            return false;
        }
        let m = &self.metrics;
        let opening = seconds(m.open_ms) + seconds(m.stagger_ms) * self.slices.len() as f32;
        self.closing.is_some()
            || self.slide.is_animating()
            || self.opened < opening
            || self.wallpaper_due
            || self.fades.values().any(|shown| *shown < seconds(m.open_ms))
    }

    pub fn paint(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, surface: Rect, themes: &[String]) {
        if self.target.is_none() {
            return;
        }
        let m = self.metrics.clone();
        let typo = ctx.metrics.typography.clone();
        let theme = ctx.theme.clone();
        let names: Vec<String> = self.filtered(themes).into_iter().cloned().collect();
        let fade = self.closing.map_or(1.0, |t| 1.0 - t / seconds(m.close_ms).max(0.001)).clamp(0.0, 1.0);
        let open_s = seconds(m.open_ms).max(0.001);
        let appear = Easing::EaseOut.apply(self.opened / open_s) * fade;
        list.push_rect(Rect { color: theme.bg_dark.with_alpha(m.scrim * appear), ..surface });

        let (band, slices) = layout::slices(names.len(), self.slide.value(), surface, &m);
        let links = ctx.state.theme_wallpapers.get();
        let previews = ctx.state.wallpaper_previews.get();
        for slice in &slices {
            let name = &names[slice.index];
            let Some(slice_theme) = ctx.themes.get(name).and_then(|c| c.to_theme().ok()) else {
                continue;
            };
            // Slices rise in outward from the focus.
            let rank = (slice.index as f32 - self.focus as f32).abs();
            let t = (self.opened - rank * seconds(m.stagger_ms)) / open_s;
            let eased = Easing::EaseOut.apply(t);
            let alpha = eased * fade;
            if alpha <= 0.0 {
                continue;
            }
            let rise = m.rise * (1.0 - eased);
            let quad = slice.quad.map(|p| Point::new(p.x, p.y + rise));

            paint_columns(list, &quad, (0.0, 1.0), &palette_art(&slice_theme), alpha);
            if let Some(preview) = links.get(name).and_then(|path| previews.get(path)) {
                let shown = *self.fades.entry(preview.key).or_insert(0.0);
                list.push_texture(TexturedQuad {
                    key: preview.key,
                    width: preview.width,
                    height: preview.height,
                    pixels: preview.pixels.clone(),
                    corners: quad,
                    uv: layout::cover_uv(&quad, (preview.width, preview.height)),
                    alpha: alpha * (shown / open_s).min(1.0),
                });
                let footer = (1.0 - m.footer_height / band.height.max(1.0)).clamp(0.0, 1.0);
                let colours = ui::theme_swatches(&slice_theme).map(|c| (c, 1.0 / 5.0));
                paint_columns(list, &quad, (footer, 1.0), &colours, alpha);
            }
            let (color, width) = if slice.index == self.focus {
                (theme.primary, m.outline_focus)
            } else if self.hovered == Some(slice.index) {
                (theme.text_main, m.outline)
            } else {
                (theme.outline, m.outline)
            };
            stroke_quad(list, &quad, color.with_alpha(color.a * alpha), width);
        }
        self.slices = slices;
        self.band = Some(band);

        let row = |y: f32, h: f32| Rect { x: surface.x, y, width: surface.width, height: h, radius: 0.0, color: Color::TRANSPARENT };
        let text = |c: Color| c.with_alpha(c.a * appear);

        // The search, above the band, only while there is one.
        let query = self.search.text().to_string();
        if !query.is_empty() {
            let width = ctx.measure(&query, m.search_text) + (m.search_padding + m.caret_width) * 2.0;
            let width = width.max(m.search_height * 4.0);
            let pill = Rect {
                x: surface.x + (surface.width - width) * 0.5,
                y: band.y - m.label_gap * 2.0 - m.search_height,
                width,
                height: m.search_height,
                radius: m.search_radius,
                color: Color::TRANSPARENT,
            };
            ui::paint_frame(list, pill, m.outline, text(theme.primary), text(theme.bg_main));
            let style = FieldStyle {
                text: m.search_text,
                padding: m.search_padding,
                caret_width: m.caret_width,
                masked: false,
                placeholder: None,
            };
            self.search.paint(ctx, list, pill, style, false);
        }

        // The focused theme, under the band.
        let mut y = band.y + band.height + m.label_gap * 2.0;
        match names.get(self.focus) {
            Some(name) => {
                let line = typo.line(m.name_text);
                ui::paint_text_centered_weighted(ctx, list, name, row(y, line), m.name_text, typo.weight_bold, text(theme.text_main));
                y += line + m.label_gap;
                if let Some(focused) = ctx.themes.get(name).and_then(|c| c.to_theme().ok()) {
                    let colours = ui::theme_swatches(&focused);
                    let gap = m.swatch * 0.5;
                    let total = colours.len() as f32 * (m.swatch + gap) - gap;
                    let mut x = surface.x + (surface.width - total) * 0.5;
                    for colour in colours {
                        let dot = Rect { x, y, width: m.swatch, height: m.swatch, radius: m.swatch * 0.5, color: text(colour) };
                        ui::paint_frame(list, dot, m.outline, text(theme.outline), dot.color);
                        x += m.swatch + gap;
                    }
                    y += m.swatch + m.label_gap;
                }
                let wallpaper = links
                    .get(name)
                    .and_then(|p| p.file_stem())
                    .map_or_else(|| "No wallpaper".to_string(), |s| s.to_string_lossy().into_owned());
                let caption = if *name == self.original { format!("{wallpaper} \u{b7} current") } else { wallpaper };
                ui::paint_text_centered(ctx, list, &caption, row(y, typo.line(m.caption_text)), m.caption_text, text(theme.text_muted));
            }
            None => {
                ui::paint_text_centered(ctx, list, "No matching themes", band, m.name_text, text(theme.text_muted));
            }
        }

        let hint = "\u{2190} \u{2192} browse \u{b7} type to search \u{b7} Enter keep \u{b7} Esc revert";
        let line = typo.line(m.hint_text);
        let bottom = surface.y + surface.height - m.label_gap * 2.0 - line;
        ui::paint_text_centered(ctx, list, hint, row(bottom, line), m.hint_text, text(theme.text_muted));
    }
}

/// A theme's palette as columns of a slice without a wallpaper.
fn palette_art(theme: &Theme) -> [(Color, f32); 5] {
    let colours = [theme.bg_main, theme.bg_widget, theme.primary, theme.secondary, theme.tertiary];
    std::array::from_fn(|i| (colours[i], PALETTE_ART[i]))
}

/// Fill `quad` between `rows` (fractions of its height) with columns of
/// colour, each `(colour, fraction of the width)`, at its slant.
fn paint_columns(list: &mut DisplayList, quad: &Quad, rows: (f32, f32), columns: &[(Color, f32)], alpha: f32) {
    let mut u = 0.0;
    for (colour, share) in columns {
        let part = layout::sub_quad(quad, (u, (u + share).min(1.0)), rows);
        fill_quad(list, &part, colour.with_alpha(colour.a * alpha));
        u += share;
    }
}

fn quad_path(quad: &Quad) -> Vec<PathCommand> {
    vec![
        PathCommand::MoveTo(quad[0]),
        PathCommand::LineTo(quad[1]),
        PathCommand::LineTo(quad[2]),
        PathCommand::LineTo(quad[3]),
        PathCommand::Close,
    ]
}

fn fill_quad(list: &mut DisplayList, quad: &Quad, colour: Color) {
    list.push_path(Path::new(quad_path(quad), Paint::Solid(colour)));
}

/// The outline, butt-capped and so antialiased, which also smooths the
/// hard edges of the fills and the wallpaper beneath it.
fn stroke_quad(list: &mut DisplayList, quad: &Quad, colour: Color, width: f32) {
    list.push_path(Path::new(quad_path(quad), Paint::Solid(colour)).stroked(width));
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay_client_toolkit::seat::keyboard::Keysym;

    fn themes() -> Vec<String> {
        ["carbon", "catppuccin", "gruvbox", "nord", "rose-pine"].iter().map(|s| s.to_string()).collect()
    }

    fn open() -> ThemeSwitcher {
        let config = crate::config::Config::test_defaults();
        let mut s = ThemeSwitcher::new(&config.metrics.theme_switcher, 250);
        s.open_on("DP-1", &themes(), "gruvbox");
        s
    }

    fn key(s: &mut ThemeSwitcher, keysym: Keysym) -> SwitcherAction {
        s.key(keysym, None, false, &themes())
    }

    fn typed(s: &mut ThemeSwitcher, text: &str) -> SwitcherAction {
        let mut last = SwitcherAction::None;
        for ch in text.chars() {
            last = s.key(Keysym::a, Some(&ch.to_string()), false, &themes());
        }
        last
    }

    #[test]
    fn opens_on_the_current_theme_without_previewing() {
        let mut s = open();
        assert_eq!(s.focused.as_deref(), Some("gruvbox"));
        assert_eq!(s.tick(1.0), SwitcherAction::None);
    }

    #[test]
    fn arrows_preview_and_wrap() {
        let mut s = open();
        assert_eq!(key(&mut s, Keysym::Right), SwitcherAction::Preview("nord".into()));
        assert_eq!(key(&mut s, Keysym::Right), SwitcherAction::Preview("rose-pine".into()));
        assert_eq!(key(&mut s, Keysym::Right), SwitcherAction::Preview("carbon".into()));
        assert_eq!(key(&mut s, Keysym::Left), SwitcherAction::Preview("rose-pine".into()));
        assert_eq!(key(&mut s, Keysym::Home), SwitcherAction::Preview("carbon".into()));
        assert_eq!(key(&mut s, Keysym::Home), SwitcherAction::Redraw);
    }

    #[test]
    fn the_wallpaper_follows_once_the_focus_rests() {
        let mut s = open();
        key(&mut s, Keysym::Right);
        assert_eq!(s.tick(0.2), SwitcherAction::None);
        // Moving again restarts the wait.
        key(&mut s, Keysym::Right);
        assert_eq!(s.tick(0.2), SwitcherAction::None);
        assert!(s.animating());
        assert_eq!(s.tick(0.1), SwitcherAction::PreviewWallpaper("rose-pine".into()));
        // Once.
        assert_eq!(s.tick(1.0), SwitcherAction::None);
    }

    #[test]
    fn return_keeps_and_escape_reverts() {
        let mut s = open();
        key(&mut s, Keysym::Right);
        assert_eq!(key(&mut s, Keysym::Return), SwitcherAction::Apply("nord".into()));
        assert!(!s.is_open());
        assert!(s.target().is_some(), "still fading out");
        s.tick(1.0);
        assert!(s.target().is_none());

        let mut s = open();
        assert_eq!(key(&mut s, Keysym::Escape), SwitcherAction::Cancel);
        assert_eq!(key(&mut s, Keysym::Right), SwitcherAction::None);
    }

    #[test]
    fn searching_keeps_the_focus_or_takes_the_first_match() {
        let mut s = open();
        // "r" still matches gruvbox.
        assert_eq!(typed(&mut s, "r"), SwitcherAction::Redraw);
        assert_eq!(s.focused.as_deref(), Some("gruvbox"));
        assert_eq!(typed(&mut s, "o"), SwitcherAction::Preview("rose-pine".into()));
        // Escape clears the search first, keeping the focus, then reverts.
        assert_eq!(key(&mut s, Keysym::Escape), SwitcherAction::Redraw);
        assert_eq!(s.focused.as_deref(), Some("rose-pine"));
        assert_eq!(s.focus, 4);
        assert_eq!(key(&mut s, Keysym::Escape), SwitcherAction::Cancel);
    }

    #[test]
    fn nothing_matching_keeps_nothing() {
        let mut s = open();
        typed(&mut s, "zzz");
        assert_eq!(key(&mut s, Keysym::Return), SwitcherAction::None);
        assert_eq!(key(&mut s, Keysym::Right), SwitcherAction::None);
    }

    #[test]
    fn the_wheel_moves_a_theme_per_notch() {
        let mut s = open();
        assert_eq!(s.on_wheel(0.5, &themes()), SwitcherAction::None);
        assert_eq!(s.on_wheel(0.5, &themes()), SwitcherAction::Preview("nord".into()));
        assert_eq!(s.on_wheel(-1.0, &themes()), SwitcherAction::Preview("gruvbox".into()));
    }

    #[test]
    fn clicks_focus_then_keep_and_outside_reverts() {
        let mut s = open();
        s.slices = vec![
            Slice { index: 2, quad: [Point::new(10.0, 0.0), Point::new(50.0, 0.0), Point::new(40.0, 100.0), Point::new(0.0, 100.0)], openness: 1.0 },
            Slice { index: 3, quad: [Point::new(60.0, 0.0), Point::new(80.0, 0.0), Point::new(70.0, 100.0), Point::new(50.0, 100.0)], openness: 0.0 },
        ];
        s.band = Some(Rect { x: 0.0, y: 0.0, width: 200.0, height: 100.0, radius: 0.0, color: Color::TRANSPARENT });
        assert_eq!(s.on_click(Point::new(65.0, 50.0), &themes()), SwitcherAction::Preview("nord".into()));
        // Between slices, inside the band: nothing.
        assert_eq!(s.on_click(Point::new(150.0, 50.0), &themes()), SwitcherAction::None);
        assert_eq!(s.on_click(Point::new(65.0, 50.0), &themes()), SwitcherAction::Apply("nord".into()));

        let mut s = open();
        s.band = Some(Rect { x: 0.0, y: 0.0, width: 200.0, height: 100.0, radius: 0.0, color: Color::TRANSPARENT });
        assert_eq!(s.on_click(Point::new(50.0, 500.0), &themes()), SwitcherAction::Cancel);
    }

    #[test]
    fn cancelling_twice_reverts_once() {
        let mut s = open();
        assert_eq!(s.cancel(), SwitcherAction::Cancel);
        assert_eq!(s.cancel(), SwitcherAction::None);
    }
}
