//! The custom theme editor on the Theme page.
//!
//! A draft palette: a name, a font, and the fourteen core colours. The six
//! Material aliases are left unset, so they follow the core colours as they
//! do in the shipped themes. A colour row expands in place into a hex field
//! and hue, saturation and lightness sliders.
//!
//! Every change is shown live: the paint that sees the draft differ from
//! what it last sent queues [`SystemAction::PreviewTheme`], and the shell
//! paints everything in the draft until it is saved or discarded. The draft
//! is shown under its own name while that name is valid, else under the
//! last valid one.

use crate::actions::SystemAction;
use crate::config::{parse_color, ThemeConfig};
use crate::render::display_list::{Color, DisplayList, Rect};
use crate::ui::pages::layout;
use crate::ui::text_field::{FieldKey, FieldStyle, TextField};
use crate::ui::{self, InputState, PaintCtx};

/// The colours a theme sets, as `(label, key)`, in the editor's order.
pub const ROLES: [(&str, &str); 14] = [
    ("Background", "bg_main"),
    ("Widget", "bg_widget"),
    ("Hover", "bg_hover"),
    ("Deep background", "bg_dark"),
    ("Text", "text_main"),
    ("Muted text", "text_muted"),
    ("Primary", "primary"),
    ("Secondary", "secondary"),
    ("Tertiary", "tertiary"),
    ("Success", "success"),
    ("Warning", "warning"),
    ("Error", "error"),
    ("Active border", "border_active"),
    ("Inactive border", "border_inactive"),
];

fn role_mut(theme: &mut ThemeConfig, index: usize) -> &mut String {
    match index {
        0 => &mut theme.bg_main,
        1 => &mut theme.bg_widget,
        2 => &mut theme.bg_hover,
        3 => &mut theme.bg_dark,
        4 => &mut theme.text_main,
        5 => &mut theme.text_muted,
        6 => &mut theme.primary,
        7 => &mut theme.secondary,
        8 => &mut theme.tertiary,
        9 => &mut theme.success,
        10 => &mut theme.warning,
        11 => &mut theme.error,
        12 => &mut theme.border_active,
        _ => &mut theme.border_inactive,
    }
}

fn role(theme: &ThemeConfig, index: usize) -> &str {
    match index {
        0 => &theme.bg_main,
        1 => &theme.bg_widget,
        2 => &theme.bg_hover,
        3 => &theme.bg_dark,
        4 => &theme.text_main,
        5 => &theme.text_muted,
        6 => &theme.primary,
        7 => &theme.secondary,
        8 => &theme.tertiary,
        9 => &theme.success,
        10 => &theme.warning,
        11 => &theme.error,
        12 => &theme.border_active,
        _ => &theme.border_inactive,
    }
}

pub use crate::ui::color::{hsl_to_rgb, rgb_to_hsl, to_hex, Hsl};

/// A typed colour, with or without its `#`.
fn parse_typed(text: &str) -> Option<Color> {
    let text = text.trim();
    if text.starts_with('#') {
        parse_color(text)
    } else {
        parse_color(&format!("#{text}"))
    }
}

/// A free name for a theme based on `base`: `<base>-custom`, then
/// `<base>-custom-2` and so on, cut to fit the name limit.
pub fn suggest_name(base: &str, taken: impl Fn(&str) -> bool) -> String {
    let mut stem: String = base
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' { c } else { '-' })
        .collect();
    stem = stem.trim_matches('-').to_string();
    if stem.is_empty() {
        stem = "theme".into();
    }
    for n in 1.. {
        let suffix = if n == 1 { "-custom".to_string() } else { format!("-custom-{n}") };
        let keep = crate::custom_themes::MAX_NAME.saturating_sub(suffix.len());
        let name = format!("{}{suffix}", stem.chars().take(keep).collect::<String>());
        if !taken(&name) {
            return name;
        }
    }
    unreachable!("an unbounded search always finds a free name")
}

/// One of a colour's sliders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Hue,
    Saturation,
    Lightness,
}

impl Channel {
    const ALL: [Channel; 3] = [Channel::Hue, Channel::Saturation, Channel::Lightness];

    fn label(self) -> &'static str {
        match self {
            Channel::Hue => "Hue",
            Channel::Saturation => "Saturation",
            Channel::Lightness => "Lightness",
        }
    }

    /// The channel's position along its slider.
    fn fraction(self, hsl: Hsl) -> f32 {
        match self {
            Channel::Hue => hsl.h / 360.0,
            Channel::Saturation => hsl.s,
            Channel::Lightness => hsl.l,
        }
    }

    fn with(self, hsl: Hsl, fraction: f32) -> Hsl {
        let f = fraction.clamp(0.0, 1.0);
        match self {
            // 360 is the same hue as 0; stop just short so the knob stays
            // at the end it was dragged to.
            Channel::Hue => Hsl { h: (f * 360.0).min(359.9), ..hsl },
            Channel::Saturation => Hsl { s: f, ..hsl },
            Channel::Lightness => Hsl { l: f, ..hsl },
        }
    }

    fn readout(self, hsl: Hsl) -> String {
        match self {
            Channel::Hue => format!("{}\u{b0}", hsl.h.round() as i32),
            Channel::Saturation => format!("{}%", (hsl.s * 100.0).round() as i32),
            Channel::Lightness => format!("{}%", (hsl.l * 100.0).round() as i32),
        }
    }

    /// The colour at `t` along this channel's track, the others held at
    /// `hsl`. The hue track is drawn fully saturated, or a grey colour would
    /// show no hues at all.
    fn ramp(self, hsl: Hsl, t: f32) -> Color {
        let at = match self {
            Channel::Hue => Hsl { h: t * 360.0, s: 1.0, l: 0.5 },
            other => other.with(hsl, t),
        };
        hsl_to_rgb(at, 1.0)
    }
}

/// Which text field has the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Focus {
    None,
    Name,
    Font,
    Hex,
}

/// What the editor asks of its page.
#[derive(Debug, PartialEq, Eq)]
pub enum EditorEvent {
    None,
    /// Saved: the page closes the editor.
    Saved,
    /// Discarded: the page closes the editor.
    Discarded,
}

pub struct Editor {
    name: TextField,
    /// The name the theme is saved under, when editing a saved one.
    original: Option<String>,
    theme: ThemeConfig,
    /// The colour row that is open.
    expanded: Option<usize>,
    focus: Focus,
    hex: TextField,
    font: TextField,
    dragging: Option<Channel>,
    /// The open colour as HSL while a slider is dragged: a colour with no
    /// saturation has no hue, and recomputing it from the colour would
    /// make the hue knob jump back to zero.
    drag_hsl: Option<Hsl>,
    /// The name the draft is shown under.
    preview_name: String,
    /// What the last preview sent, so an unchanged draft is not resent.
    last_sent: Option<(String, ThemeConfig)>,
    /// The wallpaper the theme is linked to when saved; none by default.
    wallpaper: Option<std::path::PathBuf>,
    /// Which scheme Generate makes.
    pub scheme: crate::palette::Scheme,
}

impl Editor {
    /// Edit `theme` under `name`. `original` is set when it is a saved
    /// custom theme, which may keep its name.
    pub fn new(name: &str, theme: ThemeConfig, original: Option<String>, wallpaper: Option<std::path::PathBuf>) -> Self {
        Self {
            name: TextField::with_text(name),
            original,
            font: TextField::with_text(&theme.font),
            theme,
            expanded: None,
            focus: Focus::None,
            hex: TextField::new(),
            dragging: None,
            drag_hsl: None,
            preview_name: name.to_string(),
            last_sent: None,
            wallpaper,
            scheme: crate::palette::Scheme::Dark,
        }
    }

    pub fn wallpaper(&self) -> Option<&std::path::Path> {
        self.wallpaper.as_deref()
    }

    /// Choose the wallpaper to link (and show it, through the shell).
    pub fn set_wallpaper(&mut self, wallpaper: Option<std::path::PathBuf>, pending: &mut Vec<SystemAction>) {
        pending.push(SystemAction::PreviewWallpaper(wallpaper.clone()));
        self.wallpaper = wallpaper;
    }

    /// Replace the colours with ones generated from `sample`, keeping the
    /// name and font. The next paint previews them.
    pub fn generate(&mut self, sample: &[[u8; 3]]) {
        let generated = crate::palette::generate(sample, self.scheme, &self.theme.font);
        self.theme = ThemeConfig { font: self.theme.font.clone(), ..generated };
        self.expanded = None;
        self.drag_hsl = None;
        if self.focus == Focus::Hex {
            self.focus = Focus::None;
        }
    }

    /// The name the draft is shown under, so the page can tell its card.
    pub fn preview_name(&self) -> &str {
        &self.preview_name
    }

    pub fn original(&self) -> Option<&str> {
        self.original.as_deref()
    }

    pub fn wants_keys(&self) -> bool {
        self.focus != Focus::None
    }

    /// Typing into the focused field. Return and Escape leave it.
    pub fn key(&mut self, keysym: smithay_client_toolkit::seat::keyboard::Keysym, text: Option<&str>, ctrl: bool) {
        let field = match self.focus {
            Focus::None => return,
            Focus::Name => &mut self.name,
            Focus::Font => &mut self.font,
            Focus::Hex => &mut self.hex,
        };
        match field.key(keysym, text, ctrl) {
            FieldKey::Submit(typed) => {
                // Return takes the text out of the field; put it back.
                *field = TextField::with_text(typed.expose());
                self.focus = Focus::None;
            }
            FieldKey::Escape => self.focus = Focus::None,
            FieldKey::Changed => self.apply_typed(),
            FieldKey::Ignored => {}
        }
    }

    /// Carry what was typed into the draft, where it is a valid value.
    fn apply_typed(&mut self) {
        match self.focus {
            Focus::Font => {
                let font = self.font.text().trim();
                if !font.is_empty() {
                    self.theme.font = font.to_string();
                }
            }
            Focus::Hex => {
                if let (Some(index), Some(colour)) = (self.expanded, parse_typed(self.hex.text())) {
                    *role_mut(&mut self.theme, index) = to_hex(colour);
                }
            }
            Focus::Name | Focus::None => {}
        }
    }

    /// Why the typed name cannot be saved, if it cannot.
    fn name_error(&self, ctx: &PaintCtx<'_>) -> Option<&'static str> {
        let name = self.name.text();
        // The draft itself is in the theme list under its preview name,
        // which was free when it was chosen.
        if name == self.preview_name && self.original.as_deref() != Some(name) {
            return crate::custom_themes::check_syntax(name).err();
        }
        crate::custom_themes::check_name(name, ctx.themes, ctx.custom_themes, self.original.as_deref()).err()
    }

    /// Paint the editor as a card from `y`, and queue its preview. Returns
    /// its bottom and what it asks of the page.
    #[allow(clippy::too_many_arguments)]
    pub fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        body: Rect,
        y: f32,
        input: &InputState,
        clicked: bool,
        pending: &mut Vec<SystemAction>,
    ) -> (f32, EditorEvent) {
        let m = ctx.metrics.page.clone();
        let typo = ctx.metrics.typography.clone();
        let theme = ctx.theme.clone();
        let pad = m.card_padding;
        let gap = m.label_gap;
        let name_error = self.name_error(ctx);
        let caption_h = m.section_label_height;

        // A field that is not being typed in shows the draft's value.
        if self.focus != Focus::Font {
            self.font = TextField::with_text(&self.theme.font);
        }
        if self.focus != Focus::Hex {
            if let Some(index) = self.expanded {
                self.hex = TextField::with_text(role(&self.theme, index));
            }
        }
        if !input.pressed {
            self.dragging = None;
            self.drag_hsl = None;
        }

        // The card's height, so it can be drawn under its contents.
        let expanded_h = gap + m.field_height + (m.channel_height + gap * 0.5) * 3.0;
        let height = pad
            + m.field_height
            + if name_error.is_some() { caption_h } else { 0.0 }
            + gap
            + m.field_height
            + gap
            + m.editor_row_height * ROLES.len() as f32
            + if self.expanded.is_some() { expanded_h } else { 0.0 }
            + gap
            + m.badge
            + pad;
        let card = layout::rect(body.x, y, body.width, height);
        layout::card(ctx, list, card, layout::Card::Plain);
        let inner = layout::rect(card.x + pad, card.y + pad, card.width - pad * 2.0, card.height - pad * 2.0);
        let label_w = m.editor_label_width.min(inner.width * 0.4);
        let value_x = inner.x + label_w;
        let value_w = inner.x + inner.width - value_x;
        let mut focus_hit = false;
        let mut row_y = inner.y;

        // Name, with why it cannot be saved under it.
        let field = layout::rect(value_x, row_y, value_w, m.field_height);
        self.label(ctx, list, "Name", inner.x + gap * 0.5, field);
        if self.text_field(ctx, list, field, Focus::Name, Some("theme-name"), name_error.is_some(), input, clicked) {
            focus_hit = true;
        }
        row_y += m.field_height;
        if let Some(error) = name_error {
            let line = layout::rect(value_x, row_y, value_w, caption_h);
            let error = ui::elide(ctx, error, typo.caption, typo.weight_regular, value_w);
            ui::paint_text_vcentered(ctx, list, &error, value_x, line, typo.caption, typo.weight_regular, theme.error);
            row_y += caption_h;
        }
        row_y += gap;

        // Font.
        let field = layout::rect(value_x, row_y, value_w, m.field_height);
        self.label(ctx, list, "Font", inner.x + gap * 0.5, field);
        if self.text_field(ctx, list, field, Focus::Font, Some("A Nerd Font family"), false, input, clicked) {
            focus_hit = true;
        }
        row_y += m.field_height + gap;

        // The colours.
        for (index, (label, _)) in ROLES.iter().enumerate() {
            let row = layout::rect(inner.x, row_y, inner.width, m.editor_row_height);
            let open = self.expanded == Some(index);
            let hot = layout::hovered(input, row);
            if hot || open {
                list.push_rect(Rect {
                    radius: m.button_radius,
                    color: layout::mix(theme.surface_high, theme.surface_highest, if open { 0.7 } else { 0.5 }),
                    ..row
                });
            }
            let value = role(&self.theme, index).to_string();
            let colour = parse_color(&value).unwrap_or(Color::TRANSPARENT);
            self.label(ctx, list, label, inner.x + gap * 0.5, row);
            let swatch = Rect {
                radius: m.button_radius * 0.5,
                ..layout::rect(value_x, row.y + (row.height - m.swatch * 1.5) * 0.5, m.swatch * 3.0, m.swatch * 1.5)
            };
            ui::paint_frame(list, swatch, ctx.appearance.border_width.max(m.outline), theme.outline, colour);
            let hex_x = swatch.x + swatch.width + gap;
            ui::paint_text_vcentered(ctx, list, &value, hex_x, row, typo.body, typo.weight_regular, theme.text_main);
            let chevron = if open { "\u{f0140}" } else { "\u{f0142}" };
            let glyph = layout::rect(row.x + row.width - row.height, row.y, row.height, row.height);
            ui::paint_icon_centered(ctx, list, chevron, glyph, typo.body, theme.text_muted);
            if hot && clicked {
                self.expanded = if open { None } else { Some(index) };
                if self.focus == Focus::Hex {
                    self.focus = Focus::None;
                }
                focus_hit = true;
            }
            row_y += m.editor_row_height;

            if open {
                row_y = self.paint_expanded(ctx, list, index, value_x, value_w, inner.x, row_y + gap, input, clicked, &mut focus_hit);
            }
        }
        row_y += gap;

        // Save and Discard, at the right; a reminder at the left.
        let save_w = layout::text_button_width(ctx, "Save");
        let discard_w = layout::text_button_width(ctx, "Discard");
        let save_x = inner.x + inner.width - save_w;
        let discard_x = save_x - m.grid_gap * 0.5 - discard_w;
        let note_row = layout::rect(inner.x, row_y, discard_x - inner.x - gap, m.badge);
        let note = ui::elide(ctx, "Changes show live; Save keeps them", typo.caption, typo.weight_regular, note_row.width);
        ui::paint_text_vcentered(ctx, list, &note, inner.x, note_row, typo.caption, typo.weight_regular, theme.text_muted);
        let mut event = EditorEvent::None;
        if layout::text_button(ctx, list, discard_x, row_y, "Discard", true, input, clicked) {
            pending.push(SystemAction::EndThemePreview);
            event = EditorEvent::Discarded;
        }
        let can_save = name_error.is_none();
        if can_save && layout::chip(ctx, list, save_x, row_y, "Save", true, input, clicked) {
            pending.push(SystemAction::SaveTheme {
                name: self.name.text().to_string(),
                theme: Box::new(self.theme.clone()),
                replaces: self.original.clone(),
                wallpaper: self.wallpaper.clone(),
            });
            event = EditorEvent::Saved;
        } else if !can_save {
            layout::text_button(ctx, list, save_x, row_y, "Save", false, input, clicked);
        }

        if clicked && !focus_hit {
            self.focus = Focus::None;
        }
        if event == EditorEvent::None {
            self.send_preview(name_error.is_none(), pending);
        }
        (card.y + card.height, event)
    }

    /// The hex field and the three sliders under an open colour row.
    #[allow(clippy::too_many_arguments)]
    fn paint_expanded(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        index: usize,
        value_x: f32,
        value_w: f32,
        label_x: f32,
        y: f32,
        input: &InputState,
        clicked: bool,
        focus_hit: &mut bool,
    ) -> f32 {
        let m = ctx.metrics.page.clone();
        let typo = ctx.metrics.typography.clone();
        let gap = m.label_gap;
        let mut y = y;

        let field = layout::rect(value_x, y, value_w, m.field_height);
        self.label(ctx, list, "Hex", label_x + gap * 0.5, field);
        let invalid = self.focus == Focus::Hex && parse_typed(self.hex.text()).is_none();
        if self.text_field(ctx, list, field, Focus::Hex, Some("#rrggbb"), invalid, input, clicked) {
            *focus_hit = true;
        }
        y += m.field_height + gap * 0.5;

        let value = role(&self.theme, index).to_string();
        let colour = parse_color(&value).unwrap_or(Color::TRANSPARENT);
        let hsl = self.drag_hsl.unwrap_or_else(|| rgb_to_hsl(colour));
        let readout_w = ctx.measure("100%", typo.caption).max(ctx.measure("360\u{b0}", typo.caption)) + gap;
        for channel in Channel::ALL {
            let row = layout::rect(value_x, y, value_w, m.channel_height);
            self.label(ctx, list, channel.label(), label_x + gap * 0.5, row);
            let track = layout::rect(row.x, row.y, row.width - readout_w, row.height);
            if input.pressed && clicked && layout::hovered(input, track) {
                self.dragging = Some(channel);
                self.drag_hsl = Some(hsl);
                if self.focus == Focus::Hex {
                    self.focus = Focus::None;
                }
                *focus_hit = true;
            }
            let mut shown = hsl;
            if self.dragging == Some(channel) {
                if let Some((px, _)) = input.pointer {
                    shown = channel.with(hsl, crate::ui::controls::fraction_in(track, px));
                    self.drag_hsl = Some(shown);
                    *role_mut(&mut self.theme, index) = to_hex(hsl_to_rgb(shown, colour.a));
                }
            }
            channel_slider(ctx, list, track, channel, shown);
            let readout = layout::rect(track.x + track.width, row.y, readout_w, row.height);
            let text = channel.readout(shown);
            let w = ctx.measure(&text, typo.caption);
            ui::paint_text_vcentered(ctx, list, &text, readout.x + readout.width - w, readout, typo.caption, typo.weight_regular, ctx.theme.text_muted);
            y += m.channel_height + gap * 0.5;
        }
        y
    }

    fn label(&self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, text: &str, x: f32, row: Rect) {
        let typo = ctx.metrics.typography.clone();
        let width = ctx.metrics.page.editor_label_width;
        let text = ui::elide(ctx, text, typo.body, typo.weight_regular, width);
        ui::paint_text_vcentered(ctx, list, &text, x, row, typo.body, typo.weight_regular, ctx.theme.text_main);
    }

    /// A framed text field. Returns whether a click focused it.
    #[allow(clippy::too_many_arguments)]
    fn text_field(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        rect: Rect,
        which: Focus,
        placeholder: Option<&str>,
        invalid: bool,
        input: &InputState,
        clicked: bool,
    ) -> bool {
        let m = ctx.metrics.page.clone();
        let theme = ctx.theme.clone();
        let focused = self.focus == which;
        let rect = Rect { radius: m.button_radius, ..rect };
        let (border, width) = match (invalid, focused) {
            (true, _) => (theme.error, m.outline_active),
            (false, true) => (theme.primary, m.outline_active),
            (false, false) => (theme.outline, m.outline),
        };
        ui::paint_frame(list, rect, width, border, theme.surface);
        let field = match which {
            Focus::Name => &self.name,
            Focus::Font => &self.font,
            _ => &self.hex,
        };
        let style = FieldStyle {
            text: ctx.metrics.typography.body,
            padding: m.card_padding * 0.5,
            caret_width: m.outline_active,
            masked: false,
            placeholder,
        };
        field.paint(ctx, list, rect, style, !focused);
        let hit = clicked && layout::hovered(input, rect);
        if hit {
            self.focus = which;
        }
        hit
    }

    /// Queue a preview when the draft or its name changed since the last.
    fn send_preview(&mut self, name_valid: bool, pending: &mut Vec<SystemAction>) {
        if name_valid {
            self.preview_name = self.name.text().to_string();
        }
        let next = (self.preview_name.clone(), self.theme.clone());
        if self.last_sent.as_ref() != Some(&next) {
            pending.push(SystemAction::PreviewTheme(Box::new(next.clone())));
            self.last_sent = Some(next);
        }
    }
}

/// A slider whose track shows what each position along it would give.
fn channel_slider(ctx: &mut PaintCtx<'_>, list: &mut DisplayList, rect: Rect, channel: Channel, hsl: Hsl) {
    let m = ctx.metrics.page.clone();
    let track_h = m.channel_track.min(rect.height);
    let track = layout::rect(rect.x, rect.y + (rect.height - track_h) * 0.5, rect.width, track_h);
    let segments = m.channel_segments.max(2);
    let step = track.width / segments as f32;
    for i in 0..segments {
        let t = (i as f32 + 0.5) / segments as f32;
        // Each step overlaps the next by a pixel's fraction so no seam shows.
        list.push_rect(Rect {
            color: channel.ramp(hsl, t),
            ..layout::rect(track.x + step * i as f32, track.y, step + 0.5, track.height)
        });
    }
    ui::stroke_rounded(list, Rect { radius: 0.0, ..track }, ctx.theme.outline, m.outline);

    let knob = m.slider_knob;
    let x = track.x + track.width * channel.fraction(hsl).clamp(0.0, 1.0);
    let knob_rect = Rect {
        radius: knob * 0.5,
        ..layout::rect(x - knob * 0.5, rect.y + (rect.height - knob) * 0.5, knob, knob)
    };
    ui::paint_frame(list, knob_rect, m.outline_active, ctx.theme.bg_main, ctx.theme.text_main);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(text: &str) -> Color {
        parse_color(text).unwrap()
    }

    #[test]
    fn hex_round_trips() {
        for text in ["#7aa2f7", "#000000", "#ffffff", "#1a1b26"] {
            assert_eq!(to_hex(hex(text)), text);
        }
        assert_eq!(to_hex(hex("#11223380")), "#11223380", "a translucent colour keeps its alpha");
        assert_eq!(to_hex(hex("#abc")), "#aabbcc");
    }

    #[test]
    fn hsl_round_trips() {
        for text in ["#7aa2f7", "#bb9af7", "#f7768e", "#9ece6a", "#808080", "#0b0b0e", "#ff0000"] {
            let colour = hex(text);
            assert_eq!(to_hex(hsl_to_rgb(rgb_to_hsl(colour), 1.0)), text, "{text}");
        }
    }

    #[test]
    fn grey_has_no_saturation_and_hue_wraps() {
        let grey = rgb_to_hsl(hex("#808080"));
        assert_eq!(grey.s, 0.0);
        let red = Hsl { h: 0.0, s: 1.0, l: 0.5 };
        let wrapped = Hsl { h: 360.0, ..red };
        assert_eq!(to_hex(hsl_to_rgb(red, 1.0)), to_hex(hsl_to_rgb(wrapped, 1.0)));
        assert_eq!(to_hex(hsl_to_rgb(red, 1.0)), "#ff0000");
    }

    #[test]
    fn typed_colours_need_no_hash() {
        assert_eq!(parse_typed("7aa2f7").map(to_hex).as_deref(), Some("#7aa2f7"));
        assert_eq!(parse_typed(" #7aa2f7 ").map(to_hex).as_deref(), Some("#7aa2f7"));
        assert!(parse_typed("#7aa2f").is_none());
        assert!(parse_typed("blue").is_none());
    }

    #[test]
    fn suggested_names_are_free_and_valid() {
        let taken = ["carbon-custom".to_string(), "carbon-custom-2".to_string()];
        let name = suggest_name("carbon", |n| taken.iter().any(|t| t == n));
        assert_eq!(name, "carbon-custom-3");
        let long = suggest_name(&"x".repeat(60), |_| false);
        assert!(crate::custom_themes::check_syntax(&long).is_ok(), "{long}");
        assert_eq!(suggest_name("Rosé Pine", |_| false), "ros--pine-custom");
    }

    #[test]
    fn every_role_reads_the_field_it_writes() {
        let mut theme = crate::config::Config::test_defaults().themes["carbon"].clone();
        for index in 0..ROLES.len() {
            *role_mut(&mut theme, index) = format!("#{index:06x}");
        }
        for index in 0..ROLES.len() {
            assert_eq!(role(&theme, index), format!("#{index:06x}"));
        }
        // And every key is a distinct field of the theme.
        let text = toml::to_string(&theme).unwrap();
        for (index, (_, key)) in ROLES.iter().enumerate() {
            assert!(text.contains(&format!("{key} = \"#{index:06x}\"")), "{key}");
        }
    }

    #[test]
    fn a_new_editor_previews_once_until_it_changes() {
        let theme = crate::config::Config::test_defaults().themes["carbon"].clone();
        let mut editor = Editor::new("carbon-custom", theme, None, None);
        let mut pending = Vec::new();
        editor.send_preview(true, &mut pending);
        editor.send_preview(true, &mut pending);
        assert_eq!(pending.len(), 1);
        editor.theme.primary = "#ff0000".into();
        editor.send_preview(true, &mut pending);
        assert_eq!(pending.len(), 2);
    }

    #[test]
    fn an_invalid_name_keeps_the_last_preview_name() {
        let theme = crate::config::Config::test_defaults().themes["carbon"].clone();
        let mut editor = Editor::new("mine", theme, None, None);
        editor.name = TextField::with_text("Bad Name");
        let mut pending = Vec::new();
        editor.send_preview(false, &mut pending);
        assert_eq!(editor.preview_name(), "mine");
    }
}
