//! The Audio page's per-app mixer: a card per app playing (and, under
//! Recording, per app using a microphone) with its icon, name and what it
//! plays, the device it uses (a dropdown when there is a choice), mute, and
//! a 0-150 % slider, as the volume cards above it. The open dropdown is
//! painted last, over the page, by [`Mixer::paint_menu`].

use std::collections::HashMap;

use serde_json::Value;
use stratum_sys::managers::audio::Stream;

use crate::actions::SystemAction;
use crate::render::display_list::{DisplayList, Rect};
use crate::ui::pages::layout::{self, rect, Card};
use crate::ui::pages::Optimistic;
use crate::ui::{self, controls, InputState, PaintCtx};

const MUTED: &str = "\u{f075f}";
const VOLUME: &str = "\u{f057e}";
const MIC: &str = "\u{f036c}";
const MIC_OFF: &str = "\u{f036d}";

/// One app's stream, from the audio snapshot.
#[derive(Clone, Debug, PartialEq)]
pub struct AppStream {
    pub stream: Stream,
    pub app: String,
    pub title: String,
    pub icon: String,
    pub binary: String,
    pub volume: f32,
    pub muted: bool,
    pub device: String,
    pub paused: bool,
    pub writable: bool,
}

/// The snapshot's `playback` or `recording` list.
pub fn streams(audio: &Value, recording: bool) -> Vec<AppStream> {
    let key = if recording { "recording" } else { "playback" };
    let text = |v: &Value| v.as_str().unwrap_or("").to_string();
    audio[key]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|s| {
                    let id = u32::try_from(s["id"].as_u64()?).ok()?;
                    Some(AppStream {
                        stream: if recording { Stream::Recording(id) } else { Stream::Playback(id) },
                        app: text(&s["app"]),
                        title: text(&s["title"]),
                        icon: text(&s["icon"]),
                        binary: text(&s["binary"]),
                        volume: s["volume"].as_f64().unwrap_or(0.0) as f32,
                        muted: s["mute"].as_bool().unwrap_or(false),
                        device: text(&s["device"]),
                        paused: s["corked"].as_bool().unwrap_or(false),
                        writable: s["volume_writable"].as_bool().unwrap_or(true),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The line under an app's name: what it plays, and whether it is paused.
pub fn caption(stream: &AppStream) -> String {
    match (stream.title.is_empty(), stream.paused) {
        (false, true) => format!("{} \u{b7} Paused", stream.title),
        (false, false) => stream.title.clone(),
        (true, true) => "Paused".to_string(),
        (true, false) => if stream.stream_is_recording() { "Recording" } else { "Playing" }.to_string(),
    }
}

impl AppStream {
    fn stream_is_recording(&self) -> bool {
        matches!(self.stream, Stream::Recording(_))
    }
}

/// The open device list's contents for this frame.
struct Menu {
    stream: Stream,
    anchor: Rect,
    names: Vec<String>,
    labels: Vec<String>,
    selected: Option<usize>,
}

#[derive(Default)]
pub struct Mixer {
    /// The slider held, and where.
    dragging: Option<(Stream, Option<f32>)>,
    muted: HashMap<Stream, Optimistic<bool>>,
    open: Option<Stream>,
    menu: Option<Menu>,
    list_scroll: Option<f32>,
    list_rect: Option<Rect>,
}

impl Mixer {
    pub fn tick(&mut self, dt: f32) {
        for muted in self.muted.values_mut() {
            muted.tick(dt);
        }
    }

    /// Whether the pointer is over the open device list, which covers what
    /// is under it.
    pub fn covers(&self, input: &InputState) -> bool {
        self.list_rect.is_some_and(|r| layout::hovered(input, r))
    }

    /// The wheel over the open list scrolls it. Returns whether it did.
    pub fn on_scroll(&mut self, point: (f32, f32), delta: f32) -> bool {
        let over = self.list_rect.is_some_and(|r| r.contains(crate::render::display_list::Point::new(point.0, point.1)));
        if over {
            layout::scroll_list(&mut self.list_scroll, delta);
        }
        over
    }

    pub fn on_hidden(&mut self) {
        self.open = None;
        self.list_rect = None;
        self.dragging = None;
    }

    /// The Applications section (and Recording, when an app records) at `y`.
    /// `devices` are `(name, label)` for outputs and inputs. Returns the
    /// bottom.
    #[allow(clippy::too_many_arguments)]
    pub fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        body: Rect,
        y: f32,
        outputs: &[(String, String)],
        inputs: &[(String, String)],
        input: &InputState,
        clicked: bool,
        pending: &mut Vec<SystemAction>,
    ) -> f32 {
        let audio = ctx.state.audio.get();
        let playback = streams(&audio, false);
        let recording = streams(&audio, true);
        let page = ctx.metrics.page.clone();
        let m = ctx.metrics.audio_page.clone();
        self.menu = None;
        // Forget what belongs to streams that ended.
        let live: Vec<Stream> = playback.iter().chain(&recording).map(|s| s.stream).collect();
        self.muted.retain(|s, _| live.contains(s));
        if self.open.is_some_and(|s| !live.contains(&s)) {
            self.open = None;
        }

        let mut y = layout::section(ctx, list, body, y, "Applications");
        if playback.is_empty() {
            let cell = rect(body.x, y, body.width, page.info_height);
            layout::info_card(ctx, list, cell, "Nothing is playing", "Apps appear here while they play sound");
            y += page.info_height;
        }
        for stream in &playback {
            let card = rect(body.x, y, body.width, m.volume_height);
            self.paint_stream(ctx, list, card, stream, outputs, input, clicked, pending);
            y += m.volume_height + page.grid_gap;
        }
        if !playback.is_empty() {
            y -= page.grid_gap;
        }
        if !recording.is_empty() {
            y += page.section_spacing;
            y = layout::section(ctx, list, body, y, "Recording");
            for stream in &recording {
                let card = rect(body.x, y, body.width, m.volume_height);
                self.paint_stream(ctx, list, card, stream, inputs, input, clicked, pending);
                y += m.volume_height + page.grid_gap;
            }
            y -= page.grid_gap;
        }
        y
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_stream(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        card: Rect,
        stream: &AppStream,
        devices: &[(String, String)],
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
        let recording = stream.stream_is_recording();
        let muted = self.muted.entry(stream.stream).or_default().view(stream.muted);

        // The app's icon, or a badge when it has none.
        let badge = rect(card.x + pad, card.y + pad, page.badge, page.badge);
        let size = (page.badge * 2.0) as u32;
        let icon = [&stream.icon, &stream.binary, &stream.app.to_lowercase()]
            .into_iter()
            .filter(|n| !n.is_empty())
            .find_map(|name| ctx.icons.load_icon(name, size, 1).or_else(|| ctx.icons.load_app_icon(name, size, 1)));
        match icon {
            Some(image) => list.push_image(crate::render::display_list::Image {
                width: image.width,
                height: image.height,
                pixels: image.pixels,
                x: badge.x,
                y: badge.y,
                width_logical: badge.width,
                height_logical: badge.height,
            }),
            None => {
                let glyph = if recording { MIC } else { VOLUME };
                layout::badge(ctx, list, badge.x, badge.y, glyph, theme.primary, layout::mix(theme.surface_high, theme.primary, 0.2));
            }
        }

        // Mute at the right, and the device left of it when there is a choice.
        let mute_x = card.x + card.width - pad - page.badge;
        let (on, off) = if recording { (MIC, MIC_OFF) } else { (VOLUME, MUTED) };
        if layout::icon_button(ctx, list, mute_x, card.y + pad, if muted { off } else { on }, muted, theme.error, input, clicked) {
            if let Some(state) = self.muted.get_mut(&stream.stream) {
                state.set(!muted);
            }
            pending.push(SystemAction::SetStreamMuted(stream.stream, !muted));
        }
        let mut text_end = mute_x - pad;
        if devices.len() > 1 {
            let labels: Vec<String> = devices.iter().map(|(_, label)| label.clone()).collect();
            let selected = devices.iter().position(|(name, _)| *name == stream.device);
            let face_h = page.badge;
            let face_w = layout::dropdown_width(ctx, &labels, face_h).min(card.width * 0.35);
            let face = rect(mute_x - pad * 0.5 - face_w, card.y + pad, face_w, face_h);
            let label = selected.map_or_else(|| stream.device.clone(), |i| labels[i].clone());
            let open = self.open == Some(stream.stream);
            if layout::dropdown(ctx, list, face, &label, open, input, clicked) {
                self.open = if open { None } else { Some(stream.stream) };
                self.list_scroll = None;
            }
            if self.open == Some(stream.stream) {
                let names = devices.iter().map(|(name, _)| name.clone()).collect();
                self.menu = Some(Menu { stream: stream.stream, anchor: face, names, labels, selected });
            }
            text_end = face.x - pad;
        }
        let text_x = badge.x + badge.width + pad * 0.75;
        let caption = caption(stream);
        let caption = if muted { format!("{caption} \u{b7} Muted") } else { caption };
        layout::two_lines(ctx, list, rect(text_x, card.y + pad, (text_end - text_x).max(0.0), page.badge), &stream.app, &caption, theme.text_muted);

        // The slider along the bottom, as the volume cards'.
        let slider_y = card.y + card.height - pad - m.slider_height;
        let readout_x = card.x + card.width - pad - m.readout_width;
        let slider = rect(card.x + pad, slider_y, readout_x - m.gap - card.x - pad, m.slider_height);
        let held = self.dragging.filter(|(s, _)| *s == stream.stream);
        if stream.writable && input.pressed && clicked && layout::hovered(input, slider) {
            self.dragging = Some((stream.stream, None));
        } else if !input.pressed && held.is_some() {
            self.dragging = None;
        }
        let held = self.dragging.filter(|(s, _)| *s == stream.stream);
        let fraction = held.and_then(|(_, v)| v).unwrap_or((stream.volume / 100.0).clamp(0.0, 1.5) / 1.5);
        if let Some(next) = controls::slider(ctx, list, slider, fraction, held.is_some(), input) {
            let sent = (fraction * 150.0).round() as i64;
            self.dragging = Some((stream.stream, Some(next)));
            let percent = (next * 150.0).round() as i64;
            if percent != sent {
                pending.push(SystemAction::SetStreamVolume(stream.stream, percent));
            }
        }
        let readout = format!("{}%", (fraction * 150.0).round() as i64);
        let readout_w = ctx.measure(&readout, typo.body);
        let color = if muted { theme.text_muted } else { theme.text_main };
        ui::paint_text_vcentered(ctx, list, &readout, card.x + card.width - pad - readout_w, slider, typo.body, typo.weight_bold, color);
    }

    /// The open device list, over everything painted before it. A click
    /// anywhere but the list or its face closes it.
    pub fn paint_menu(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        input: &InputState,
        clicked: bool,
        pending: &mut Vec<SystemAction>,
    ) {
        self.list_rect = None;
        let Some(menu) = self.menu.take() else {
            self.open = None;
            return;
        };
        list.push_layer();
        let (card, hit) =
            layout::dropdown_list(ctx, list, menu.anchor, &menu.labels, menu.selected, &mut self.list_scroll, input, clicked);
        self.list_rect = Some(card);
        if let Some(i) = hit {
            if Some(i) != menu.selected {
                pending.push(SystemAction::MoveStream(menu.stream, menu.names[i].clone()));
            }
            self.open = None;
        } else if clicked && !layout::hovered(input, card) && !layout::hovered(input, menu.anchor) {
            self.open = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streams_are_read_from_the_snapshot() {
        let audio = serde_json::json!({
            "playback": [{ "id": 7, "app": "Firefox", "title": "Video", "icon": "firefox", "binary": "firefox",
                           "volume": 80, "mute": false, "device": "alsa_output.a", "corked": true, "volume_writable": true }],
            "recording": [{ "id": 9, "app": "OBS", "title": "", "volume": 100, "mute": true, "device": "mic" }],
        });
        let playing = streams(&audio, false);
        assert_eq!(playing.len(), 1);
        assert_eq!(playing[0].stream, Stream::Playback(7));
        assert_eq!(caption(&playing[0]), "Video \u{b7} Paused");
        let recording = streams(&audio, true);
        assert_eq!(recording[0].stream, Stream::Recording(9));
        assert!(recording[0].muted);
        assert_eq!(caption(&recording[0]), "Recording");
        assert!(streams(&Value::Null, false).is_empty());
    }
}
