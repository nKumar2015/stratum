//! The overview page: quick toggles, a status card per subsystem, and what is
//! playing.
//!
//! Toggles act at once (Wi-Fi, Bluetooth, Do Not Disturb, mute) and hold the
//! value the user asked for through [`Optimistic`] until the system agrees.
//! A status card opens its page when clicked.

use serde_json::Value;

use crate::actions::SystemAction;
use crate::render::display_list::{Color, DisplayList, Rect};
use crate::ui::pages::layout::{self, Card};
use crate::ui::pages::{devices, Optimistic, Page};
use crate::ui::{self, InputState, PaintCtx};

pub struct OverviewPage {
    wifi: Optimistic<bool>,
    bluetooth: Optimistic<bool>,
    muted: Optimistic<bool>,
}

/// One quick toggle as painted.
struct Toggle {
    label: &'static str,
    glyph: &'static str,
    on: bool,
    state: String,
}

/// One status card as painted.
struct Status {
    page: Page,
    label: &'static str,
    glyph: &'static str,
    value: String,
    detail: String,
    /// Fill of the meter along the bottom, and its colour; `None` for none.
    meter: Option<(f32, Color)>,
    accent: Color,
}

impl OverviewPage {
    pub fn new() -> Self {
        Self {
            wifi: Optimistic::new(),
            bluetooth: Optimistic::new(),
            muted: Optimistic::new(),
        }
    }

    pub fn tick(&mut self, dt: f32) {
        self.wifi.tick(dt);
        self.bluetooth.tick(dt);
        self.muted.tick(dt);
    }

    /// Returns the bottom of the page, and a page to open when a status card
    /// was clicked.
    pub fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        area: Rect,
        input: &InputState,
        clicked: bool,
        pending: &mut Vec<SystemAction>,
    ) -> (f32, Option<Page>) {
        let m = ctx.metrics.page.clone();
        let area = layout::column(area, m.max_width);
        let today = chrono::Local::now().format("%A, %-d %B").to_string();
        let (_, body) = layout::header(ctx, list, area, "Overview", &today);

        let mut y = layout::section(ctx, list, body, body.y, "Quick settings");
        y = self.paint_toggles(ctx, list, body, y, input, clicked, pending);

        y += m.section_spacing;
        y = layout::section(ctx, list, body, y, "Status");
        let (bottom, navigate) = self.paint_status(ctx, list, body, y, input, clicked);
        y = bottom;

        y += m.section_spacing;
        y = layout::section(ctx, list, body, y, "Now playing");
        y = paint_music(ctx, list, body, y, input, clicked, pending);
        (y, navigate)
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_toggles(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        body: Rect,
        y: f32,
        input: &InputState,
        clicked: bool,
        pending: &mut Vec<SystemAction>,
    ) -> f32 {
        let m = ctx.metrics.page.clone();
        let wifi_snapshot = ctx.state.wifi.get();
        let net = ctx.state.net.get();
        let bt = ctx.state.bluetooth.get();
        let audio = ctx.state.audio.get();

        let wifi = self.wifi.view(wifi_snapshot["state"].as_str() != Some("disabled"));
        let bluetooth = self.bluetooth.view(bt["state"].as_str().is_some_and(|s| s != "off"));
        let dnd = ctx.state.dnd.get();
        let muted = self.muted.view(audio["mute"].as_str() == Some("yes"));

        let connected = connected_devices(&bt);
        let toggles = [
            Toggle {
                label: "Wi-Fi",
                glyph: "\u{f05a9}",
                on: wifi,
                state: if wifi {
                    wifi_connection(&net).unwrap_or_else(|| "On".into())
                } else {
                    "Off".into()
                },
            },
            Toggle {
                label: "Bluetooth",
                glyph: "\u{f00af}",
                on: bluetooth,
                state: match (bluetooth, connected.len()) {
                    (false, _) => "Off".into(),
                    (true, 0) => "On".into(),
                    (true, 1) => connected[0].clone(),
                    (true, n) => format!("{n} connected"),
                },
            },
            Toggle {
                label: "Do Not Disturb",
                glyph: if dnd { "\u{f009b}" } else { "\u{f009a}" },
                on: dnd,
                state: if dnd { "On".into() } else { "Off".into() },
            },
            Toggle {
                label: "Mute",
                glyph: if muted { "\u{f075f}" } else { "\u{f057e}" },
                on: muted,
                state: if muted { "Muted".into() } else { volume_text(&audio) },
            },
        ];

        let cols = layout::columns(body.width, m.tile_min_width, m.grid_gap, toggles.len());
        let cols = even_columns(cols, toggles.len());
        let cells = layout::cells(body.x, y, body.width, toggles.len(), cols, m.tile_height, m.grid_gap);
        for (index, (toggle, cell)) in toggles.iter().zip(&cells).enumerate() {
            if paint_toggle(ctx, list, *cell, toggle, input) && clicked {
                let next = !toggle.on;
                match index {
                    0 => {
                        self.wifi.set(next);
                        pending.push(SystemAction::WifiToggle(next));
                    }
                    1 => {
                        self.bluetooth.set(next);
                        pending.push(SystemAction::BluetoothPower(next));
                    }
                    2 => {
                        ctx.state.dnd.set(next);
                        ctx.state.notify();
                    }
                    _ => {
                        self.muted.set(next);
                        pending.push(SystemAction::SetMuted(next));
                    }
                }
            }
        }
        y + layout::grid_height(toggles.len(), cols, m.tile_height, m.grid_gap)
    }

    fn paint_status(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        body: Rect,
        y: f32,
        input: &InputState,
        clicked: bool,
    ) -> (f32, Option<Page>) {
        let m = ctx.metrics.page.clone();
        let theme = ctx.theme.clone();
        let audio = ctx.state.audio.get();
        let battery = ctx.state.battery.get();
        let net = ctx.state.net.get();
        let bt = ctx.state.bluetooth.get();

        let muted = self.muted.view(audio["mute"].as_str() == Some("yes"));
        let volume = volume_percent(&audio);
        let pct = battery["battery"]["pct"].as_u64().map(|p| p.min(100));
        let battery_state = battery["battery"]["state"].as_str().unwrap_or("");
        let charging = matches!(battery_state, "charging" | "fully-charged");
        let battery_color = match pct {
            _ if charging => theme.success,
            Some(p) if p <= 15 => theme.error,
            Some(p) if p <= 30 => theme.warning,
            _ => theme.success,
        };
        let connection = wifi_connection(&net);
        let signal = net["signal_pct"]
            .as_str()
            .and_then(|s| s.parse::<f32>().ok())
            .or_else(|| net["signal_pct"].as_f64().map(|v| v as f32));
        let connected = connected_devices(&bt);
        let paired = bt["devices"]
            .as_array()
            .map(|d| d.iter().filter(|d| d["paired"].as_str() == Some("yes")).count())
            .unwrap_or(0);
        let bt_on = bt["state"].as_str().is_some_and(|s| s != "off");

        let cards = [
            Status {
                page: Page::Audio,
                label: "Volume",
                glyph: if muted { "\u{f075f}" } else { "\u{f057e}" },
                value: volume.map(|v| format!("{v}%")).unwrap_or_else(|| "--".into()),
                detail: if muted { "Muted".into() } else { default_device(&audio) },
                meter: volume.map(|v| {
                    (v as f32 / 100.0, if muted { theme.text_muted } else { theme.primary })
                }),
                accent: theme.primary,
            },
            Status {
                page: Page::Battery,
                label: "Battery",
                glyph: battery_glyph(pct.unwrap_or(0), charging),
                value: pct.map(|p| format!("{p}%")).unwrap_or_else(|| "--".into()),
                detail: battery_detail(&battery),
                meter: pct.map(|p| (p as f32 / 100.0, battery_color)),
                accent: battery_color,
            },
            Status {
                page: Page::Wifi,
                label: "Network",
                glyph: "\u{f0928}",
                value: connection.clone().unwrap_or_else(|| "Offline".into()),
                detail: match (&connection, signal) {
                    (Some(_), Some(s)) => format!("Signal {s:.0}%"),
                    (Some(_), None) => "Connected".into(),
                    (None, _) => "Not connected".into(),
                },
                meter: connection.as_ref().and(signal).map(|s| (s / 100.0, theme.tertiary)),
                accent: theme.tertiary,
            },
            Status {
                page: Page::Bluetooth,
                label: "Bluetooth",
                glyph: "\u{f00af}",
                value: match (bt_on, connected.first()) {
                    (false, _) => "Off".into(),
                    (true, Some(name)) => name.clone(),
                    (true, None) => "On".into(),
                },
                detail: match (bt_on, connected.len()) {
                    (false, _) => "Adapter is off".into(),
                    (true, 0) => format!("{paired} paired"),
                    (true, n) => format!("{n} connected, {paired} paired"),
                },
                meter: None,
                accent: theme.secondary,
            },
        ];

        let cols = layout::columns(body.width, m.card_min_width, m.grid_gap, cards.len());
        let cols = even_columns(cols, cards.len());
        let cells = layout::cells(body.x, y, body.width, cards.len(), cols, m.stat_card_height, m.grid_gap);
        let mut navigate = None;
        for (card, cell) in cards.iter().zip(&cells) {
            if paint_status_card(ctx, list, *cell, card, input) && clicked {
                navigate = Some(card.page);
            }
        }
        let bottom = y + layout::grid_height(cards.len(), cols, m.stat_card_height, m.grid_gap);
        (bottom, navigate)
    }
}

impl Default for OverviewPage {
    fn default() -> Self {
        Self::new()
    }
}

/// Fewer columns when the count does not divide evenly, so a row of four
/// never leaves a lone card under three.
fn even_columns(cols: usize, count: usize) -> usize {
    (1..=cols).rev().find(|c| count.is_multiple_of(*c)).unwrap_or(1)
}

/// Draw a quick toggle. Returns whether the pointer is over it.
fn paint_toggle(ctx: &mut PaintCtx<'_>, list: &mut DisplayList, cell: Rect, toggle: &Toggle, input: &InputState) -> bool {
    let m = ctx.metrics.page.clone();
    let typo = ctx.metrics.typography.clone();
    let theme = ctx.theme.clone();
    let hot = layout::hovered(input, cell);
    layout::card(ctx, list, cell, layout::clickable(input, cell, toggle.on));

    let pad = m.card_padding;
    let (fg, bg) = if toggle.on {
        (theme.on_primary, theme.primary)
    } else {
        (theme.text_muted, theme.surface_highest)
    };
    layout::badge(ctx, list, cell.x + pad, cell.y + (cell.height - m.badge) * 0.5, toggle.glyph, fg, bg);

    let text_x = cell.x + pad + m.badge + pad * 0.75;
    let text_w = cell.x + cell.width - pad - text_x;
    let line = typo.body * 1.4;
    let caption = typo.caption * 1.4;
    let top = cell.y + (cell.height - line - caption) * 0.5;
    let label = ui::elide(ctx, toggle.label, typo.body, typo.weight_bold, text_w);
    ui::paint_text_vcentered(
        ctx,
        list,
        &label,
        text_x,
        layout::rect(text_x, top, text_w, line),
        typo.body,
        typo.weight_bold,
        theme.text_main,
    );
    let state = ui::elide(ctx, &toggle.state, typo.caption, typo.weight_regular, text_w);
    ui::paint_text_vcentered(
        ctx,
        list,
        &state,
        text_x,
        layout::rect(text_x, top + line, text_w, caption),
        typo.caption,
        typo.weight_regular,
        if toggle.on { theme.primary } else { theme.text_muted },
    );
    hot
}

/// Draw a status card. Returns whether the pointer is over it.
fn paint_status_card(ctx: &mut PaintCtx<'_>, list: &mut DisplayList, cell: Rect, card: &Status, input: &InputState) -> bool {
    let m = ctx.metrics.page.clone();
    let typo = ctx.metrics.typography.clone();
    let theme = ctx.theme.clone();
    let hot = layout::hovered(input, cell);
    layout::card(ctx, list, cell, if hot { Card::Hover } else { Card::Plain });

    let pad = m.card_padding;
    let inner_w = cell.width - pad * 2.0;
    let badge_bg = layout::mix(theme.surface_high, card.accent, 0.2);
    layout::badge(ctx, list, cell.x + pad, cell.y + pad, card.glyph, card.accent, badge_bg);
    ui::paint_text_vcentered(
        ctx,
        list,
        card.label,
        cell.x + pad + m.badge + pad * 0.75,
        layout::rect(cell.x, cell.y + pad, cell.width, m.badge),
        typo.label,
        typo.weight_bold,
        theme.text_muted,
    );
    // A chevron says the card opens its page.
    ui::paint_icon_centered(
        ctx,
        list,
        "\u{f054}",
        layout::rect(cell.x + cell.width - pad - m.badge, cell.y + pad, m.badge, m.badge),
        typo.caption,
        if hot { theme.text_main } else { theme.text_muted },
    );

    let meter_space = if card.meter.is_some() { m.meter + pad * 0.75 } else { 0.0 };
    let detail_h = typo.caption * 1.4;
    let value_h = m.value * 1.3;
    let detail_y = cell.y + cell.height - pad - meter_space - detail_h;
    let value_y = detail_y - value_h;
    let value = ui::elide(ctx, &card.value, m.value, typo.weight_bold, inner_w);
    ui::paint_text_vcentered(
        ctx,
        list,
        &value,
        cell.x + pad,
        layout::rect(cell.x, value_y, cell.width, value_h),
        m.value,
        typo.weight_bold,
        theme.text_main,
    );
    let detail = ui::elide(ctx, &card.detail, typo.caption, typo.weight_regular, inner_w);
    ui::paint_text_vcentered(
        ctx,
        list,
        &detail,
        cell.x + pad,
        layout::rect(cell.x, detail_y, cell.width, detail_h),
        typo.caption,
        typo.weight_regular,
        theme.text_muted,
    );
    if let Some((fraction, color)) = card.meter {
        let bar = layout::rect(cell.x + pad, cell.y + cell.height - pad - m.meter, inner_w, m.meter);
        layout::meter(ctx, list, bar, fraction, color);
    }
    hot
}

/// The now-playing card. Returns its bottom.
fn paint_music(
    ctx: &mut PaintCtx<'_>,
    list: &mut DisplayList,
    body: Rect,
    y: f32,
    input: &InputState,
    clicked: bool,
    pending: &mut Vec<SystemAction>,
) -> f32 {
    let m = ctx.metrics.page.clone();
    let typo = ctx.metrics.typography.clone();
    let audio_m = ctx.metrics.audio_page.clone();
    let theme = ctx.theme.clone();
    let music = ctx.state.music.get();
    let status = music["status"].as_str().unwrap_or("");
    let player = music["player"].as_str().unwrap_or("None").to_string();
    let active = player != "None" && matches!(status, "Playing" | "Paused");

    let cell = layout::rect(body.x, y, body.width, m.tile_height);
    layout::card(ctx, list, cell, Card::Plain);
    let pad = m.card_padding;
    let (fg, bg) = if status == "Playing" {
        (theme.on_primary, theme.primary)
    } else {
        (theme.text_muted, theme.surface_highest)
    };
    layout::badge(ctx, list, cell.x + pad, cell.y + (cell.height - m.badge) * 0.5, "\u{f075a}", fg, bg);

    let buttons = [
        ("\u{f04a}", "previous"),
        (if status == "Playing" { "\u{f04c}" } else { "\u{f04b}" }, "play-pause"),
        ("\u{f04e}", "next"),
    ];
    let size = audio_m.media_button;
    let controls_w = size * 3.0 + audio_m.gap * 2.0;
    let mut bx = cell.x + cell.width - pad - controls_w;
    let by = cell.y + (cell.height - size) * 0.5;
    for (glyph, action) in buttons {
        let button = layout::rect(bx, by, size, size);
        if devices::icon_button(ctx, list, button, glyph, theme.text_main, active, input, clicked) {
            pending.push(SystemAction::Media { player: player.clone(), action: action.into() });
        }
        bx += size + audio_m.gap;
    }

    let text_x = cell.x + pad + m.badge + pad * 0.75;
    let text_w = (cell.x + cell.width - pad - controls_w - pad - text_x).max(0.0);
    let (title, artist) = if active {
        (
            music["title"].as_str().filter(|t| !t.is_empty()).unwrap_or("Unknown title").to_string(),
            music["artist"].as_str().filter(|a| !a.is_empty()).unwrap_or(&player).to_string(),
        )
    } else {
        ("Nothing playing".to_string(), "Media controls appear here".to_string())
    };
    let line = typo.body * 1.4;
    let caption = typo.caption * 1.4;
    let top = cell.y + (cell.height - line - caption) * 0.5;
    let title = ui::elide(ctx, &title, typo.body, typo.weight_bold, text_w);
    ui::paint_text_vcentered(
        ctx,
        list,
        &title,
        text_x,
        layout::rect(text_x, top, text_w, line),
        typo.body,
        typo.weight_bold,
        theme.text_main,
    );
    let artist = ui::elide(ctx, &artist, typo.caption, typo.weight_regular, text_w);
    ui::paint_text_vcentered(
        ctx,
        list,
        &artist,
        text_x,
        layout::rect(text_x, top + line, text_w, caption),
        typo.caption,
        typo.weight_regular,
        theme.text_muted,
    );
    cell.y + cell.height
}

fn volume_percent(audio: &Value) -> Option<u64> {
    audio["volume"]
        .as_str()
        .and_then(|v| v.trim_end_matches('%').parse::<f64>().ok())
        .or_else(|| audio["volume"].as_f64())
        .map(|v| v.round().clamp(0.0, 150.0) as u64)
}

fn volume_text(audio: &Value) -> String {
    volume_percent(audio)
        .map(|v| format!("{v}%"))
        .unwrap_or_else(|| "On".into())
}

/// The default output's description, as the Audio page names it.
fn default_device(audio: &Value) -> String {
    let sink = audio["default_sink"].as_str().unwrap_or_default();
    audio["sinks"]
        .as_array()
        .and_then(|sinks| sinks.iter().find(|d| d["name"].as_str() == Some(sink)))
        .and_then(|d| d["description"].as_str())
        .filter(|d| !d.is_empty())
        .unwrap_or("Output")
        .to_string()
}

/// The active Wi-Fi or wired connection's name.
fn wifi_connection(net: &Value) -> Option<String> {
    net["connections"]
        .as_array()
        .and_then(|list| list.first())
        .and_then(|conn| conn["connection"].as_str())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
}

/// Names of the connected Bluetooth devices.
fn connected_devices(bt: &Value) -> Vec<String> {
    bt["devices"]
        .as_array()
        .map(|devices| {
            devices
                .iter()
                .filter(|d| d["connected"].as_str() == Some("yes"))
                .map(|d| d["name"].as_str().unwrap_or("Device").to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn battery_detail(snapshot: &Value) -> String {
    let battery = &snapshot["battery"];
    if !battery["present"].as_bool().unwrap_or(false) {
        return "No battery".into();
    }
    let state = battery["state"].as_str().unwrap_or("");
    let projected = battery["projected_text"].as_str().filter(|t| !t.is_empty() && *t != "Unknown");
    match (state, projected) {
        ("fully-charged", _) => "Fully charged".into(),
        ("charging", Some(t)) => format!("Charging, full in {t}"),
        ("charging", None) => "Charging".into(),
        ("discharging", Some(t)) => format!("{t} remaining"),
        (state, _) if !state.is_empty() => capitalise(state),
        _ => "Unknown".into(),
    }
}

fn battery_glyph(pct: u64, charging: bool) -> &'static str {
    if charging {
        return "\u{f0084}";
    }
    // Material battery glyphs in steps of ten.
    const LEVELS: [&str; 11] = [
        "\u{f008e}", "\u{f007a}", "\u{f007b}", "\u{f007c}", "\u{f007d}", "\u{f007e}",
        "\u{f007f}", "\u{f0080}", "\u{f0081}", "\u{f0082}", "\u{f0079}",
    ];
    LEVELS[(pct.min(100) / 10) as usize]
}

fn capitalise(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn columns_divide_the_cards_evenly() {
        assert_eq!(even_columns(4, 4), 4);
        assert_eq!(even_columns(3, 4), 2, "never three and a straggler");
        assert_eq!(even_columns(2, 4), 2);
        assert_eq!(even_columns(1, 4), 1);
    }

    #[test]
    fn volume_reads_either_shape() {
        assert_eq!(volume_percent(&json!({ "volume": "4%" })), Some(4));
        assert_eq!(volume_percent(&json!({ "volume": 37.4 })), Some(37));
        assert_eq!(volume_percent(&json!({})), None);
    }

    #[test]
    fn battery_detail_reads_like_a_sentence() {
        let snapshot = |state: &str, projected: &str| {
            json!({ "battery": { "present": true, "state": state, "projected_text": projected } })
        };
        assert_eq!(battery_detail(&snapshot("discharging", "3h 12m")), "3h 12m remaining");
        assert_eq!(battery_detail(&snapshot("charging", "40m")), "Charging, full in 40m");
        assert_eq!(battery_detail(&snapshot("fully-charged", "")), "Fully charged");
        assert_eq!(battery_detail(&snapshot("pending-charge", "")), "Pending-charge");
        assert_eq!(battery_detail(&json!({ "battery": { "present": false } })), "No battery");
    }
}
