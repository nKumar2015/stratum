//! Audio page: matching the Playback tab of `modules/AudioMenu.qml`, with the
//! volume and mute controls the QML put in the hover menu.
//!
//! - Volume (0–150 %) and mute.
//! - Now playing: art, title (scrolling when it does not fit), artist, album,
//!   a seek slider with position and length, and previous / play-pause /
//!   next. "Nothing is playing right now" when no player is playing or
//!   paused.
//! - Output and input devices. A choice shows as selected at once and holds
//!   for `pending_ms` unless the system agrees sooner (QML's optimistic
//!   guard), with "Switching output..." until it is done.
//!
//! The Equalizer tab is the Equalizer page.

use serde_json::Value;

use crate::actions::SystemAction;
use crate::render::display_list::{DisplayList, Point, Rect};
use crate::ui::controls;
use crate::ui::pages::devices::{self, rect, Marquee, Status};
use crate::ui::pages::{heading, Optimistic};
use crate::ui::{self, InputState, PaintCtx};

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

pub struct AudioPage {
    muted: Optimistic<bool>,
    dragging: bool,
    drag_value: Option<f32>,
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
            muted: Optimistic::new(),
            dragging: false,
            drag_value: None,
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
        self.muted.tick(dt);
        self.output.tick(dt);
        self.input.tick(dt);
        self.sought.tick(dt);
        self.status.tick(dt);
    }

    /// A scrolling title needs frames.
    pub fn animating(&self) -> bool {
        self.marquee.moving()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        area: Rect,
        input: &InputState,
        clicked: bool,
        pending: &mut Vec<SystemAction>,
    ) {
        let area = heading(ctx, list, area, "Audio");
        let m = ctx.metrics.audio_page.clone();
        let page = ctx.metrics.page.clone();
        let theme = ctx.theme.clone();
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
        let mut y = area.y;

        // Volume.
        let real_volume = audio["volume"]
            .as_str()
            .and_then(|v| v.trim_end_matches('%').parse::<f32>().ok())
            .or_else(|| audio["volume"].as_f64().map(|v| v as f32))
            .unwrap_or(0.0);
        let readout_x = area.x + area.width - m.readout_width;
        let slider_rect = rect(area.x, y, readout_x - m.gap - area.x, m.slider_height);
        let inside = input.pointer.is_some_and(|(x, y)| slider_rect.contains(Point::new(x, y)));
        if input.pressed && inside && clicked {
            self.dragging = true;
        } else if !input.pressed {
            self.dragging = false;
            self.drag_value = None;
        }
        let fraction = self.drag_value.unwrap_or(((real_volume / 100.0).clamp(0.0, 1.5)) / 1.5);
        if let Some(next) = controls::slider(ctx, list, slider_rect, fraction, self.dragging, input) {
            self.drag_value = Some(next);
            let percent = (next * 150.0).round() as i64;
            if percent != real_volume.round() as i64 {
                pending.push(SystemAction::SetVolume(percent));
            }
        }
        let muted = self.muted.view(audio["mute"].as_str() == Some("yes"));
        let readout = if muted { "Muted".to_string() } else { format!("{}%", (fraction * 150.0).round() as i64) };
        ui::paint_text_vcentered(ctx, list, &readout, readout_x, slider_rect, m.text, 400, theme.text_muted);
        y += m.slider_height + m.gap;
        if let Some(next) = controls::toggle(ctx, list, rect(area.x, y, area.width, page.button_height), "Mute", muted, input, clicked) {
            self.muted.set(next);
            pending.push(SystemAction::SetMuted(next));
        }
        y += page.button_height + m.spacing;

        // Now playing.
        let card_h = m.card_padding * 2.0 + m.art;
        let card = rect(area.x, y, area.width, card_h);
        self.paint_music(ctx, list, card, &ctx.state.music.get(), input, clicked, pending);
        y += card_h + m.spacing;

        // Outputs and inputs, side by side.
        let footer_h = ctx.metrics.device_page.footer_height;
        let footer_top = area.y + area.height - footer_h;
        let col_w = (area.width - m.gap) * 0.5;
        let real_sink = audio["default_sink"].as_str().unwrap_or("").to_string();
        let real_source = audio["default_source"].as_str().unwrap_or("").to_string();
        let sink = self.output.view(real_sink);
        let source = self.input.view(real_source);
        let columns = [
            ("Audio Output", "sinks", sink, area.x, true),
            ("Audio Input", "sources", source, area.x + col_w + m.gap, false),
        ];
        for (title, key, current, x, is_output) in columns {
            let label_rect = rect(x, y, col_w, m.label_height);
            ui::paint_text_vcentered(ctx, list, title, x, label_rect, m.label, ctx.metrics.typography.weight_bold, theme.text_muted);
            let top = y + m.label_height + m.gap;
            let column = rect(x, top, col_w, footer_top - m.gap - top);
            list.push_clip(column);
            let mut row_y = top;
            let mut chosen = None;
            let devices = devices_of(&audio, key);
            if devices.is_empty() {
                let none = rect(x, row_y, col_w, m.row_height);
                ui::paint_text_vcentered(ctx, list, "No devices", x, none, m.row_title, 400, theme.text_muted);
            }
            for device in &devices {
                let row = rect(x, row_y, col_w, m.row_height);
                let selected = device.name == current;
                let fill = if selected { theme.bg_hover } else { theme.bg_widget };
                let border = if selected { theme.border_active } else { theme.border_inactive };
                devices::card(ctx, list, row, fill, border);
                let inner = rect(row.x + m.row_padding, row.y, row.width - m.row_padding * 2.0, row.height * 0.6);
                list.push_clip(inner);
                ui::paint_text_vcentered(ctx, list, &device.label, inner.x, inner, m.row_title, ctx.metrics.typography.weight_bold, theme.text_main);
                list.pop_clip();
                let sub = rect(inner.x, row.y + row.height * 0.55, inner.width, row.height * 0.35);
                list.push_clip(sub);
                ui::paint_text_vcentered(ctx, list, &device.name, inner.x, sub, m.row_caption, 400, theme.text_muted);
                list.pop_clip();
                let hit = input.pointer.is_some_and(|(px, py)| row.contains(Point::new(px, py)) && column.contains(Point::new(px, py)));
                if hit && clicked && !selected && self.switching.is_none() {
                    chosen = Some(device.name.clone());
                }
                row_y += m.row_height + m.row_gap;
            }
            list.pop_clip();
            if let Some(name) = chosen {
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
        }

        self.status.paint(ctx, list, rect(area.x, footer_top, area.width, footer_h));
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
        let theme = ctx.theme.clone();
        devices::card(ctx, list, card, theme.bg_widget, theme.border_inactive);
        let status = music["status"].as_str().unwrap_or("");
        let playable = status == "Playing" || status == "Paused";
        let player = music["player"].as_str().unwrap_or("None").to_string();

        // Art, or a note glyph.
        let art = rect(card.x + m.card_padding, card.y + m.card_padding, m.art, m.art);
        crate::ui::text_field::paint_frame(list, Rect { radius: ctx.metrics.device_page.radius, ..art }, ctx.metrics.device_page.border, theme.border_inactive, theme.bg_main);
        let url = music["art_url"].as_str().unwrap_or_default();
        let image = url
            .strip_prefix("file://")
            .or_else(|| url.starts_with('/').then_some(url))
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
            None => ui::paint_text_centered(ctx, list, "\u{f0386}", art, m.art * 0.4, theme.text_muted),
        }

        let x = art.x + art.width + m.card_padding;
        let w = card.x + card.width - m.card_padding - x;
        let mut y = art.y;
        let title = if playable {
            music["title"].as_str().filter(|t| !t.is_empty()).unwrap_or("Unknown Title")
        } else {
            "Nothing is playing right now"
        };
        self.marquee.paint(ctx, list, rect(x, y, w, m.line), title, m.title, theme.text_main);
        y += m.line;
        let line = |ctx: &mut PaintCtx<'_>, list: &mut DisplayList, y: f32, text: &str| {
            let row = rect(x, y, w, m.line);
            list.push_clip(row);
            ui::paint_text_vcentered(ctx, list, text, x, row, m.text, 400, theme.text_muted);
            list.pop_clip();
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
        let buttons_y = card.y + card.height - m.card_padding - m.media_button;
        let seek_y = buttons_y - m.gap - m.line;
        let times_w = ctx.measure("000:00", m.caption);
        let seek = rect(x + times_w + m.gap, seek_y, w - (times_w + m.gap) * 2.0, m.line);
        let seekable = playable && length > 0.0;
        let over = input.pointer.is_some_and(|(px, py)| seek.contains(Point::new(px, py)));
        if seekable && clicked && over {
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
        ui::paint_text_vcentered(ctx, list, &left, x, rect(x, seek_y, times_w, m.line), m.caption, 400, theme.text_muted);
        let right = format_time(length);
        let right_w = ctx.measure(&right, m.caption);
        ui::paint_text_vcentered(ctx, list, &right, x + w - right_w, rect(x, seek_y, w, m.line), m.caption, 400, theme.text_muted);

        // Controls, centred.
        let playing = status == "Playing";
        let buttons = [
            ("\u{f04a}", "previous"),
            (if playing { "\u{f04c}" } else { "\u{f04b}" }, "play-pause"),
            ("\u{f04e}", "next"),
        ];
        let total = m.media_button * 3.0 + m.gap * 2.0;
        let mut bx = x + (w - total) * 0.5;
        let enabled = player != "None";
        for (glyph, action) in buttons {
            let button = rect(bx, buttons_y, m.media_button, m.media_button);
            if devices::icon_button(ctx, list, button, glyph, theme.text_main, enabled, input, clicked) {
                pending.push(SystemAction::Media { player: player.clone(), action: action.into() });
            }
            bx += m.media_button + m.gap;
        }
    }
}

impl Default for AudioPage {
    fn default() -> Self {
        Self::new()
    }
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
}
