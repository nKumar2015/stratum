//! Audio page: the Playback tab of `modules/AudioMenu.qml`, with the volume
//! and mute controls the QML put in the hover menu.
//!
//! - A volume card each for the output and the input: the device, a 0–150 %
//!   slider with its readout, and mute as a round button. Side by side when
//!   both fit, stacked otherwise.
//! - Now playing: art, title (scrolling when it does not fit), artist, album,
//!   a seek slider with position and length, and previous / play-pause /
//!   next. "Nothing is playing right now" when no player is playing or
//!   paused.
//! - Output and input devices as full-width rows, so long names have room.
//!   A choice shows as selected at once and holds for `pending_ms` unless
//!   the system agrees sooner (QML's optimistic guard), with "Switching
//!   output..." until it is done.
//!
//! The Equalizer tab is the Equalizer page.

use serde_json::Value;

use crate::actions::SystemAction;
use crate::render::display_list::{DisplayList, Rect};
use crate::ui::controls;
use crate::ui::pages::devices::{Marquee, Status};
use crate::ui::pages::layout::{self, rect, Card};
use crate::ui::pages::Optimistic;
use crate::ui::style::{AudioPageMetrics, PageMetrics};
use crate::ui::{self, InputState, PaintCtx};

const SPEAKER: &str = "\u{f04c3}";
const HEADPHONES: &str = "\u{f02cb}";
const MUTED: &str = "\u{f075f}";
const VOLUME: &str = "\u{f057e}";
const MIC: &str = "\u{f036c}";
const MIC_OFF: &str = "\u{f036d}";
const SELECTED: &str = "\u{f012c}";

/// `mm:ss`, as the QML `formatTime` (minutes keep growing past 99).
pub fn format_time(seconds: f64) -> String {
    if seconds.is_nan() || seconds < 0.0 {
        return "00:00".into();
    }
    let total = seconds.floor() as i64;
    format!("{:02}:{:02}", total / 60, total % 60)
}

/// One sink or source.
struct Device {
    name: String,
    label: String,
}

fn devices_of(audio: &Value, key: &str) -> Vec<Device> {
    audio[key]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|d| {
                    // Sink monitors are not microphones.
                    let name = d["name"].as_str().filter(|n| !n.is_empty() && !n.ends_with(".monitor"))?.to_string();
                    let label = d["description"].as_str().filter(|n| !n.is_empty()).unwrap_or(&name).to_string();
                    Some(Device { name, label })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A percentage the manager reports as `"45%"` (or a number).
fn percent(value: &Value) -> f32 {
    value
        .as_str()
        .and_then(|v| v.trim_end_matches('%').parse::<f32>().ok())
        .or_else(|| value.as_f64().map(|v| v as f32))
        .unwrap_or(0.0)
}

/// The glyph for an output: headphones for a headset or a Bluetooth device,
/// a speaker otherwise.
fn output_glyph(name: &str, label: &str) -> &'static str {
    let label = label.to_lowercase();
    if name.starts_with("bluez_") || label.contains("headphone") || label.contains("headset") {
        HEADPHONES
    } else {
        SPEAKER
    }
}

/// The volume and mute of one side, output or input.
struct Volume {
    muted: Optimistic<bool>,
    dragging: bool,
    drag_value: Option<f32>,
}

impl Volume {
    fn new() -> Self {
        Self {
            muted: Optimistic::new(),
            dragging: false,
            drag_value: None,
        }
    }
}

/// What a volume card shows, and the actions it sends.
struct VolumeCard<'a> {
    device: &'a str,
    caption: &'a str,
    glyph: &'a str,
    /// The mute button, before and after it is pressed.
    mute_glyph: &'a str,
    muted_glyph: &'a str,
    volume: f32,
    muted: bool,
    set_volume: fn(i64) -> SystemAction,
    set_muted: fn(bool) -> SystemAction,
}

pub struct AudioPage {
    output_volume: Volume,
    input_volume: Volume,
    output: Optimistic<String>,
    input: Optimistic<String>,
    /// The seek slider is held, at this fraction.
    seeking: Option<f32>,
    /// A seek just sent: the position to show until the player reports it.
    sought: Optimistic<i64>,
    /// The device switch running, and its ticket.
    switching: Option<u64>,
    marquee: Marquee,
    status: Status,
}

impl AudioPage {
    pub fn new() -> Self {
        Self {
            output_volume: Volume::new(),
            input_volume: Volume::new(),
            output: Optimistic::new(),
            input: Optimistic::new(),
            seeking: None,
            sought: Optimistic::new(),
            switching: None,
            marquee: Marquee::default(),
            status: Status::default(),
        }
    }

    pub fn tick(&mut self, dt: f32) {
        self.output_volume.muted.tick(dt);
        self.input_volume.muted.tick(dt);
        self.output.tick(dt);
        self.input.tick(dt);
        self.sought.tick(dt);
        self.status.tick(dt);
    }

    /// A scrolling title needs frames.
    pub fn animating(&self) -> bool {
        self.marquee.moving()
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
        let m = ctx.metrics.audio_page.clone();
        let page = ctx.metrics.page.clone();
        let status_ms = ctx.metrics.device_page.status_ms;

        // A finished device switch.
        let result = ctx.state.action_result.get();
        if self.switching == Some(result.ticket) {
            self.switching = None;
            match result.error {
                Some(error) => self.status.result(error, true, status_ms),
                None => self.status.result("Audio route updated.".into(), false, status_ms),
            }
        }

        let audio = ctx.state.audio.get();
        let sinks = devices_of(&audio, "sinks");
        let sources = devices_of(&audio, "sources");
        let sink = self.output.view(audio["default_sink"].as_str().unwrap_or("").to_string());
        let source = self.input.view(audio["default_source"].as_str().unwrap_or("").to_string());
        let label_of = |devices: &[Device], name: &str, none: &str| {
            devices
                .iter()
                .find(|d| d.name == name)
                .map(|d| d.label.clone())
                .unwrap_or_else(|| none.to_string())
        };
        let sink_label = label_of(&sinks, &sink, "No output");
        let source_label = label_of(&sources, &source, "No input");

        let out_muted = self.output_volume.muted.view(audio["mute"].as_str() == Some("yes"));
        let in_muted = self.input_volume.muted.view(audio["input_mute"].as_str() == Some("yes"));
        let out_volume = percent(&audio["volume"]);
        let in_volume = percent(&audio["input_volume"]);

        let area = layout::column(area, page.max_width);
        let summary = if out_muted {
            format!("{sink_label} \u{b7} Muted")
        } else {
            format!("{sink_label} \u{b7} {}%", out_volume.round() as i64)
        };
        let (_, body) = layout::header(ctx, list, area, "Audio", &summary);

        // Volume, output and input.
        let mut y = layout::section(ctx, list, body, body.y, "Volume");
        let cols = layout::columns(body.width, m.volume_min_width, page.grid_gap, 2);
        let cells = layout::cells(body.x, y, body.width, 2, cols, m.volume_height, page.grid_gap);
        let cards = [
            VolumeCard {
                device: &sink_label,
                caption: "Output",
                glyph: output_glyph(&sink, &sink_label),
                mute_glyph: VOLUME,
                muted_glyph: MUTED,
                volume: out_volume,
                muted: out_muted,
                set_volume: SystemAction::SetVolume,
                set_muted: SystemAction::SetMuted,
            },
            VolumeCard {
                device: &source_label,
                caption: "Input",
                glyph: MIC,
                mute_glyph: MIC,
                muted_glyph: MIC_OFF,
                volume: in_volume,
                muted: in_muted,
                set_volume: SystemAction::SetInputVolume,
                set_muted: SystemAction::SetInputMuted,
            },
        ];
        for ((card, cell), state) in cards.iter().zip(&cells).zip([&mut self.output_volume, &mut self.input_volume]) {
            paint_volume(ctx, list, *cell, card, state, input, clicked, pending);
        }
        y += layout::grid_height(2, cols, m.volume_height, page.grid_gap);

        // Now playing.
        y += page.section_spacing;
        y = layout::section(ctx, list, body, y, "Now playing");
        let card = rect(body.x, y, body.width, page.card_padding * 2.0 + music_height(&m, &page));
        self.paint_music(ctx, list, card, &ctx.state.music.get(), input, clicked, pending);
        y += card.height;

        // Devices.
        let lists = [
            ("Output devices", &sinks, &sink, true),
            ("Input devices", &sources, &source, false),
        ];
        let mut chosen = None;
        for (title, devices, current, is_output) in lists {
            y += page.section_spacing;
            y = layout::section(ctx, list, body, y, title);
            if devices.is_empty() {
                layout::info_card(ctx, list, rect(body.x, y, body.width, page.info_height), "None found", "No devices");
                y += page.info_height;
                continue;
            }
            for device in devices.iter() {
                let row = rect(body.x, y, body.width, m.device_height);
                let selected = device.name == *current;
                let glyph = if is_output { output_glyph(&device.name, &device.label) } else { MIC };
                let check = selected.then_some(SELECTED);
                let hot = layout::list_row(ctx, list, row, glyph, &device.label, &device.name, check, selected, input);
                if hot && clicked && !selected && self.switching.is_none() {
                    chosen = Some((device.name.clone(), is_output));
                }
                y += m.device_height + page.grid_gap;
            }
            y -= page.grid_gap;
        }
        if let Some((name, is_output)) = chosen {
            let seconds = m.pending_ms as f32 / 1000.0;
            let (action, message) = if is_output {
                self.output.set_for(name.clone(), seconds);
                (SystemAction::SetOutput(name), "Switching output...")
            } else {
                self.input.set_for(name.clone(), seconds);
                (SystemAction::SetInput(name), "Switching input...")
            };
            self.status.progress(message.into());
            let ticket = crate::actions::next_ticket();
            self.switching = Some(ticket);
            pending.push(SystemAction::Tracked { ticket, action: Box::new(action) });
        }

        // How the last switch went. The line is always reserved, so the page
        // does not jump when a message appears.
        let footer_h = ctx.metrics.device_page.footer_height;
        self.status.paint(ctx, list, rect(body.x, y + page.label_gap, body.width, footer_h));
        y + page.label_gap + footer_h
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_music(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        card: Rect,
        music: &Value,
        input: &InputState,
        clicked: bool,
        pending: &mut Vec<SystemAction>,
    ) {
        let m = ctx.metrics.audio_page.clone();
        let page = ctx.metrics.page.clone();
        let typo = ctx.metrics.typography.clone();
        let theme = ctx.theme.clone();
        layout::card(ctx, list, card, Card::Plain);
        let pad = page.card_padding;
        let status = music["status"].as_str().unwrap_or("");
        let playable = status == "Playing" || status == "Paused";
        let player = music["player"].as_str().unwrap_or("None").to_string();

        // Art, or a note glyph.
        let art = Rect {
            radius: page.card_radius * 0.75,
            color: theme.surface_highest,
            ..rect(card.x + pad, card.y + (card.height - m.art) * 0.5, m.art, m.art)
        };
        // `art_path` is the art as a local file, remote covers included.
        let image = music["art_path"]
            .as_str()
            .filter(|path| !path.is_empty())
            .and_then(|path| ctx.icons.load_icon(path, (m.art * 2.0) as u32, 1));
        match image.filter(|_| playable) {
            Some(img) => list.push_image(crate::render::display_list::Image {
                width: img.width,
                height: img.height,
                pixels: img.pixels,
                x: art.x,
                y: art.y,
                width_logical: art.width,
                height_logical: art.height,
            }),
            None => {
                list.push_rect(art);
                ui::paint_icon_centered(ctx, list, "\u{f0386}", art, page.badge_icon, theme.text_muted);
            }
        }

        let x = art.x + art.width + pad;
        let w = card.x + card.width - pad - x;
        let mut y = card.y + pad;
        let title = if playable {
            music["title"].as_str().filter(|t| !t.is_empty()).unwrap_or("Unknown Title")
        } else {
            "Nothing is playing right now"
        };
        self.marquee.paint(ctx, list, rect(x, y, w, m.line), title, m.title, theme.text_main);
        y += m.line;
        let line = |ctx: &mut PaintCtx<'_>, list: &mut DisplayList, y: f32, text: &str| {
            let text = ui::elide(ctx, text, m.text, typo.weight_regular, w);
            ui::paint_text_vcentered(ctx, list, &text, x, rect(x, y, w, m.line), m.text, typo.weight_regular, theme.text_muted);
        };
        let field = |k: &str| music[k].as_str().filter(|s| !s.is_empty() && *s != "N/A").map(str::to_string);
        if playable {
            line(ctx, list, y, &field("artist").unwrap_or_else(|| "Unknown Artist".into()));
            if let Some(album) = field("album") {
                line(ctx, list, y + m.line, &album);
            }
        }

        // Seek, at the bottom of the text column, above the buttons.
        let length = music["length_sec"].as_f64().unwrap_or(0.0);
        let real_position = music["position_sec"].as_f64().unwrap_or(0.0).round() as i64;
        let position = self.sought.view(real_position) as f64;
        let buttons_y = card.y + card.height - pad - page.badge;
        let seek_y = buttons_y - m.gap - m.line;
        let times_w = ctx.measure("000:00", m.caption);
        let seek = rect(x + times_w + m.gap, seek_y, w - (times_w + m.gap) * 2.0, m.line);
        let seekable = playable && length > 0.0;
        if seekable && clicked && layout::hovered(input, seek) {
            self.seeking = Some(0.0);
        }
        let shown = match self.seeking {
            Some(_) => {
                let t = input.pointer.map(|(px, _)| controls::fraction_in(seek, px)).unwrap_or(0.0);
                self.seeking = Some(t);
                t
            }
            None if length > 0.0 => (position / length).clamp(0.0, 1.0) as f32,
            None => 0.0,
        };
        if let Some(t) = self.seeking.filter(|_| !input.pressed) {
            self.seeking = None;
            let seconds = (t as f64 * length).round() as i64;
            self.sought.set(seconds);
            pending.push(SystemAction::MediaSeek { player: player.clone(), seconds });
        }
        controls::slider(ctx, list, seek, shown, false, &InputState { pointer: None, pressed: false });
        let at = if self.seeking.is_some() { shown as f64 * length } else { position };
        let left = format_time(at);
        ui::paint_text_vcentered(ctx, list, &left, x, rect(x, seek_y, times_w, m.line), m.caption, typo.weight_regular, theme.text_muted);
        let right = format_time(length);
        let right_w = ctx.measure(&right, m.caption);
        ui::paint_text_vcentered(ctx, list, &right, x + w - right_w, rect(x, seek_y, w, m.line), m.caption, typo.weight_regular, theme.text_muted);

        // Controls, centred.
        let playing = status == "Playing";
        let buttons = [
            ("\u{f04a}", "previous"),
            (if playing { "\u{f04c}" } else { "\u{f04b}" }, "play-pause"),
            ("\u{f04e}", "next"),
        ];
        let total = page.badge * 3.0 + m.gap * 2.0;
        let mut bx = x + (w - total) * 0.5;
        let enabled = player != "None";
        for (glyph, action) in buttons {
            // Play-pause carries the accent while something plays.
            let on = playing && action == "play-pause";
            if layout::icon_button(ctx, list, bx, buttons_y, glyph, on, theme.primary, input, clicked && enabled) {
                pending.push(SystemAction::Media { player: player.clone(), action: action.into() });
            }
            bx += page.badge + m.gap;
        }
    }
}

impl Default for AudioPage {
    fn default() -> Self {
        Self::new()
    }
}

/// The now-playing card's content height: the taller of the art and the text
/// column, which stacks title, artist and album, then the seek row and the
/// buttons.
fn music_height(m: &AudioPageMetrics, page: &PageMetrics) -> f32 {
    m.art.max(m.line * 4.0 + m.gap * 2.0 + page.badge)
}

/// A volume card: the device and mute on the top line, the slider and its
/// readout below.
#[allow(clippy::too_many_arguments)]
fn paint_volume(
    ctx: &mut PaintCtx<'_>,
    list: &mut DisplayList,
    cell: Rect,
    card: &VolumeCard<'_>,
    state: &mut Volume,
    input: &InputState,
    clicked: bool,
    pending: &mut Vec<SystemAction>,
) {
    let m = ctx.metrics.audio_page.clone();
    let page = ctx.metrics.page.clone();
    let typo = ctx.metrics.typography.clone();
    let theme = ctx.theme.clone();
    layout::card(ctx, list, cell, Card::Plain);
    let pad = page.card_padding;

    let (fg, bg) = if card.muted {
        (theme.text_muted, theme.surface_highest)
    } else {
        (theme.primary, layout::mix(theme.surface_high, theme.primary, 0.2))
    };
    let glyph = if card.muted { card.muted_glyph } else { card.glyph };
    layout::badge(ctx, list, cell.x + pad, cell.y + pad, glyph, fg, bg);

    let mute_x = cell.x + cell.width - pad - page.badge;
    let text_x = cell.x + pad + page.badge + pad * 0.75;
    let caption = if card.muted { format!("{} \u{b7} Muted", card.caption) } else { card.caption.to_string() };
    layout::two_lines(
        ctx,
        list,
        rect(text_x, cell.y + pad, mute_x - pad - text_x, page.badge),
        card.device,
        &caption,
        theme.text_muted,
    );
    let mute_glyph = if card.muted { card.muted_glyph } else { card.mute_glyph };
    if layout::icon_button(ctx, list, mute_x, cell.y + pad, mute_glyph, card.muted, theme.error, input, clicked) {
        state.muted.set(!card.muted);
        pending.push((card.set_muted)(!card.muted));
    }

    // The slider, along the bottom.
    let slider_y = cell.y + cell.height - pad - m.slider_height;
    let readout_x = cell.x + cell.width - pad - m.readout_width;
    let slider = rect(cell.x + pad, slider_y, readout_x - m.gap - cell.x - pad, m.slider_height);
    if input.pressed && clicked && layout::hovered(input, slider) {
        state.dragging = true;
    } else if !input.pressed {
        state.dragging = false;
        state.drag_value = None;
    }
    let fraction = state.drag_value.unwrap_or((card.volume / 100.0).clamp(0.0, 1.5) / 1.5);
    if let Some(next) = controls::slider(ctx, list, slider, fraction, state.dragging, input) {
        // Compare with what this drag last sent, not the published volume,
        // which lags behind and would resend the same value every frame.
        let sent = (fraction * 150.0).round() as i64;
        state.drag_value = Some(next);
        let percent = (next * 150.0).round() as i64;
        if percent != sent {
            pending.push((card.set_volume)(percent));
        }
    }
    let readout = format!("{}%", (fraction * 150.0).round() as i64);
    let readout_w = ctx.measure(&readout, typo.body);
    ui::paint_text_vcentered(
        ctx,
        list,
        &readout,
        cell.x + cell.width - pad - readout_w,
        slider,
        typo.body,
        typo.weight_bold,
        if card.muted { theme.text_muted } else { theme.text_main },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_format_like_qml() {
        assert_eq!(format_time(0.0), "00:00");
        assert_eq!(format_time(65.9), "01:05");
        assert_eq!(format_time(3600.0), "60:00");
        assert_eq!(format_time(-1.0), "00:00");
        assert_eq!(format_time(f64::NAN), "00:00");
    }

    #[test]
    fn percentages_parse_from_either_form() {
        assert_eq!(percent(&serde_json::json!("45%")), 45.0);
        assert_eq!(percent(&serde_json::json!(30)), 30.0);
        assert_eq!(percent(&Value::Null), 0.0);
    }

    #[test]
    fn headsets_get_headphones() {
        assert_eq!(output_glyph("bluez_output.AA_BB.1", "WH-1000XM4"), HEADPHONES);
        assert_eq!(output_glyph("alsa_output.usb", "USB Headset Analog Stereo"), HEADPHONES);
        assert_eq!(output_glyph("alsa_output.pci", "Built-in Audio Analog Stereo"), SPEAKER);
    }
}
