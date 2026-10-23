//! Hover menus: the compact panels that open beside the audio, Bluetooth,
//! Wi-Fi, and battery icons when the pointer rests on them.
//!
//! Matching the QML hover menus (`modules/*HoverMenu.qml`):
//!
//! - A menu opens after the pointer has rested on its icon for `open_ms`
//!   (350 ms in QML), on the monitor the icon is on, beside the icon.
//! - It closes once neither the icon nor the menu has been hovered for
//!   `close_ms` (the QML 420 ms icon grace plus the menu's 800 ms timer came to
//!   about this), so the pointer can cross the gap between them.
//! - Clicking the icon closes it and opens the full page, as before.
//! - Each menu has a close button and "Open full settings →".
//!
//! Content: audio volume with output and input selection; Bluetooth quick
//! connect for up to six paired devices; the active network connections; and
//! battery status with the power-mode choice. Everything reads the same live
//! state as the control-centre pages and sends the same actions.

use std::time::{Duration, Instant};

use crate::actions::SystemAction;
use crate::render::display_list::{Color, DisplayList, Point, Rect};
use crate::ui::control_center::Page;
use crate::ui::pages::Optimistic;
use crate::ui::{self, controls, InputState, PaintCtx};

/// Which hover menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoverKind {
    Audio,
    Bluetooth,
    Wifi,
    Battery,
}

impl HoverKind {
    pub fn page(self) -> Page {
        match self {
            HoverKind::Audio => Page::Audio,
            HoverKind::Bluetooth => Page::Bluetooth,
            HoverKind::Wifi => Page::Wifi,
            HoverKind::Battery => Page::Battery,
        }
    }
}

/// An open menu: what, on which output, beside which icon.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenMenu {
    pub kind: HoverKind,
    pub output: String,
    /// The icon's centre, in the output's coordinates.
    pub anchor_x: f32,
    pub anchor_y: f32,
}

/// The pointer-intent timing, separate from drawing so it can be tested.
#[derive(Default)]
pub struct HoverIntent {
    open: Option<OpenMenu>,
    /// The icon under the pointer, and since when.
    resting: Option<(OpenMenu, Instant)>,
    /// When the pointer last left both the icon and the menu.
    left_at: Option<Instant>,
}

impl HoverIntent {
    pub fn open(&self) -> Option<&OpenMenu> {
        self.open.as_ref()
    }

    pub fn close(&mut self) {
        self.open = None;
        self.resting = None;
        self.left_at = None;
    }

    /// Whether a timer is running, so the loop keeps waking to fire it.
    pub fn pending(&self) -> bool {
        self.resting.is_some() || (self.open.is_some() && self.left_at.is_some())
    }

    /// Feed the pointer: the icon it is over (if any) and whether it is over
    /// the open menu. Returns true when the open menu changed.
    pub fn update(
        &mut self,
        now: Instant,
        icon: Option<OpenMenu>,
        over_menu: bool,
        open_after: Duration,
        close_after: Duration,
    ) -> bool {
        let before = self.open.clone();

        match icon {
            Some(target) => {
                self.left_at = None;
                let same_as_open = self.open.as_ref().is_some_and(|o| o.kind == target.kind && o.output == target.output);
                if same_as_open {
                    self.resting = None;
                } else {
                    // Restart the rest timer only when moving to a new icon.
                    let since = match &self.resting {
                        Some((resting, since)) if resting.kind == target.kind && resting.output == target.output => *since,
                        _ => now,
                    };
                    if now.duration_since(since) >= open_after {
                        self.open = Some(target);
                        self.resting = None;
                    } else {
                        self.resting = Some((target, since));
                    }
                }
            }
            None => {
                // Leaving an icon before the delay cancels the open.
                self.resting = None;
                if over_menu {
                    self.left_at = None;
                } else if self.open.is_some() {
                    let left = *self.left_at.get_or_insert(now);
                    if now.duration_since(left) >= close_after {
                        self.open = None;
                        self.left_at = None;
                    }
                }
            }
        }
        self.open != before
    }
}

/// Something a menu asks the shell to do.
#[derive(Clone, Debug)]
pub enum MenuEvent {
    System(SystemAction),
    OpenPage(Page),
    Close,
}

/// State the menus keep between frames.
#[derive(Default)]
pub struct HoverMenus {
    pub intent: HoverIntent,
    dragging: bool,
    drag_value: Option<f32>,
    profile: Optimistic<String>,
    output: Optimistic<String>,
    input: Optimistic<String>,
}

impl HoverMenus {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn tick(&mut self, dt: f32) {
        self.profile.tick(dt);
        self.output.tick(dt);
        self.input.tick(dt);
    }

    /// Paint `kind` into a card at `origin` of the configured width. Returns
    /// the card's height and the events this frame's input produced. Called
    /// with no pointer to measure.
    pub fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        kind: HoverKind,
        origin: Point,
        input: &InputState,
        clicked: bool,
    ) -> (f32, Vec<MenuEvent>) {
        let m = ctx.metrics.hover_menu.clone();
        let mut frame = Frame {
            x: origin.x + m.padding,
            y: origin.y + m.padding,
            width: m.width - m.padding * 2.0,
            pointer: input.pointer.map(|(x, y)| Point::new(x, y)),
            clicked,
            events: Vec::new(),
        };
        // The card's height is only known once its content is laid out, so
        // the frame is drawn afterwards and spliced in beneath the content.
        let frame_at = list.commands.len();

        let (glyph, title) = match kind {
            HoverKind::Audio => ("\u{f057e}", "Audio"),
            HoverKind::Bluetooth => ("\u{f00af}", "Quick Connect"),
            HoverKind::Wifi => ("\u{f0928}", "Network"),
            HoverKind::Battery => ("\u{f0084}", "Battery"),
        };
        frame.header(ctx, list, &format!("{glyph}  {title}"));

        match kind {
            HoverKind::Audio => self.audio(ctx, list, &mut frame, input),
            HoverKind::Bluetooth => bluetooth(ctx, list, &mut frame),
            HoverKind::Wifi => network(ctx, list, &mut frame),
            HoverKind::Battery => self.battery(ctx, list, &mut frame),
        }

        frame.gap(m.section_gap);
        if frame.link(ctx, list, "Open full settings \u{2192}") {
            frame.events.push(MenuEvent::OpenPage(kind.page()));
        }
        let height = frame.y + m.padding - origin.y;
        let card = Rect {
            x: origin.x,
            y: origin.y,
            width: m.width,
            height,
            radius: m.radius,
            color: Color::TRANSPARENT,
        };
        let mut background = DisplayList::default();
        ui::paint_frame(&mut background, card, m.border, ctx.theme.border_inactive, ctx.theme.bg_main);
        list.commands.splice(frame_at..frame_at, background.commands);
        (height, frame.events)
    }

    fn audio(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, f: &mut Frame, input: &InputState) {
        let m = ctx.metrics.hover_menu.clone();
        let audio = ctx.state.audio.get();
        let volume = audio["volume"]
            .as_str()
            .and_then(|v| v.trim_end_matches('%').parse::<f32>().ok())
            .unwrap_or(0.0);

        // Volume: glyph, slider over 0–150% (the shell's ceiling), percentage.
        let row = f.take(m.row_height);
        let (glyph, color) = crate::ui::widgets::icon::IconKind::Audio.appearance(ctx);
        let glyph_rect = Rect { width: m.row_height, ..row };
        ui::paint_text_centered(ctx, list, glyph, glyph_rect, m.icon, color);
        let label_w = m.value_width;
        let slider = Rect {
            x: row.x + m.row_height + m.gap,
            width: row.width - m.row_height - m.gap * 2.0 - label_w,
            ..row
        };
        let inside = f.pointer.is_some_and(|p| slider.contains(p));
        if input.pressed && inside && !self.dragging {
            self.dragging = true;
        } else if !input.pressed {
            self.dragging = false;
            self.drag_value = None;
        }
        let fraction = self.drag_value.unwrap_or((volume / 100.0).clamp(0.0, 1.5) / 1.5);
        if let Some(next) = controls::slider(ctx, list, slider, fraction, self.dragging, input) {
            // Compare with what this drag last sent, not the published volume,
            // which lags behind and would resend the same value every frame.
            let sent = (fraction * 150.0).round() as i64;
            self.drag_value = Some(next);
            let percent = (next * 150.0).round() as i64;
            if percent != sent {
                f.events.push(MenuEvent::System(SystemAction::SetVolume(percent)));
            }
        }
        let shown = (self.drag_value.unwrap_or(fraction) * 150.0).round() as i64;
        ui::paint_text_vcentered(ctx, list, &format!("{shown}%"), row.x + row.width - label_w, row, m.text, ctx.metrics.typography.weight_medium, ctx.theme.text_muted);

        let default_sink = audio["default_sink"].as_str().unwrap_or_default().to_string();
        let sink = self.output.view(default_sink);
        f.section(ctx, list, "\u{f057e}  Output");
        let sinks = devices(&audio["sinks"]);
        if sinks.is_empty() {
            f.note(ctx, list, "No output devices");
        }
        for (name, label) in sinks {
            if f.radio_row(ctx, list, &label, name == sink) && name != sink {
                self.output.set(name.clone());
                f.events.push(MenuEvent::System(SystemAction::SetOutput(name)));
            }
        }

        let default_source = audio["default_source"].as_str().unwrap_or_default().to_string();
        let source = self.input.view(default_source);
        f.section(ctx, list, "\u{f036c}  Input");
        // Sink monitors are not microphones.
        let sources: Vec<_> = devices(&audio["sources"])
            .into_iter()
            .filter(|(name, _)| !name.ends_with(".monitor"))
            .collect();
        if sources.is_empty() {
            f.note(ctx, list, "No input devices");
        }
        for (name, label) in sources {
            if f.radio_row(ctx, list, &label, name == source) && name != source {
                self.input.set(name.clone());
                f.events.push(MenuEvent::System(SystemAction::SetInput(name)));
            }
        }
    }

    fn battery(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, f: &mut Frame) {
        let m = ctx.metrics.hover_menu.clone();
        let snapshot = ctx.state.battery.get();
        let battery = &snapshot["battery"];
        let state = battery["state"].as_str().unwrap_or("");
        let pct = battery["pct"].as_u64().unwrap_or(0);
        // Labels from BatteryHoverMenu.qml.
        let state_text = match state {
            "charging" => "Charging",
            "discharging" => "On Battery",
            "fully-charged" | "pending-charge" => "Plugged In",
            _ => "Battery",
        };
        let row = f.take(line(&m, m.title));
        ui::paint_text_vcentered(ctx, list, &format!("{state_text} \u{b7} {pct}%"), row.x, row, m.title, ctx.metrics.typography.weight_bold, ctx.theme.text_main);
        f.gap(m.gap);

        let projected_label = match state {
            "charging" => "To Full",
            "discharging" => "Remaining",
            "fully-charged" | "pending-charge" => "Status",
            _ => "Projected",
        };
        let projected = match state {
            "fully-charged" => "Fully Charged".to_string(),
            "pending-charge" => "Not Charging".to_string(),
            _ => battery["projected_text"].as_str().unwrap_or("Unknown").to_string(),
        };
        let screen_on = battery["screen_on_time"].as_str().unwrap_or("\u{2014}").to_string();
        let rate = snapshot["charging_info"].as_str().unwrap_or("\u{2014}").to_string();
        f.stat(ctx, list, "Screen On", &screen_on);
        f.stat(ctx, list, projected_label, &projected);
        f.stat(ctx, list, "\u{f140b}  Rate", &rate);

        f.section(ctx, list, "Power Mode");
        let real = snapshot["profile"].as_str().unwrap_or("balanced").to_string();
        let active = self.profile.view(real);
        let options = [("low-power", "Saver"), ("balanced", "Normal"), ("balanced-performance", "Performance")];
        let row = f.take(m.button_height);
        let w = (row.width - m.gap * (options.len() - 1) as f32) / options.len() as f32;
        for (i, (key, label)) in options.iter().enumerate() {
            let rect = Rect {
                x: row.x + i as f32 * (w + m.gap),
                width: w,
                radius: m.button_radius,
                ..row
            };
            let selected = active == *key;
            ui::paint_frame(
                list,
                rect,
                m.border,
                if selected { ctx.theme.border_active } else { ctx.theme.border_inactive },
                if selected { ctx.theme.bg_hover } else { ctx.theme.bg_widget },
            );
            ui::paint_text_centered_weighted(ctx, list, label, rect, m.text, ctx.metrics.typography.weight_medium, if selected { ctx.theme.text_main } else { ctx.theme.text_muted });
            if f.clicked_in(rect) && !selected {
                self.profile.set(key.to_string());
                f.events.push(MenuEvent::System(SystemAction::SetPowerProfile(key.to_string())));
            }
        }
    }
}

fn bluetooth(ctx: &mut PaintCtx<'_>, list: &mut DisplayList, f: &mut Frame) {
    let m = ctx.metrics.hover_menu.clone();
    let status = ctx.state.bluetooth.get();
    if status["state"].as_str() == Some("off") {
        f.note(ctx, list, "Bluetooth is off");
        return;
    }
    let paired: Vec<_> = status["devices"]
        .as_array()
        .map(|list| list.iter().filter(|d| d["paired"].as_str() == Some("yes")).cloned().collect())
        .unwrap_or_default();
    if paired.is_empty() {
        f.note(ctx, list, "No paired devices");
    }
    for device in paired.iter().take(m.max_devices) {
        let name = device["name"].as_str().unwrap_or("Unknown");
        let address = device["mac"].as_str().unwrap_or_default().to_string();
        let connected = device["connected"].as_str() == Some("yes");
        let row = f.take(m.row_height);
        let hovered = f.hover(row);
        if hovered {
            list.push_rect(Rect { radius: m.button_radius, color: ctx.theme.bg_hover, ..row });
        }
        ui::paint_text_vcentered(ctx, list, if connected { "\u{f00b1}" } else { "\u{f00af}" }, row.x + m.gap, row, m.icon, ctx.metrics.typography.weight_regular, if connected { ctx.theme.primary } else { ctx.theme.text_muted });
        let name_x = row.x + m.gap * 2.0 + m.icon;
        ui::paint_text_vcentered(ctx, list, name, name_x, row, m.text, ctx.metrics.typography.weight_medium, ctx.theme.text_main);
        let hint = if connected { "Disconnect" } else { "Connect" };
        let style = crate::text::TextStyle {
            family: std::sync::Arc::from(ctx.theme.font.as_str()),
            size: m.caption,
            weight: ctx.metrics.typography.weight_regular,
            color: ctx.theme.text_muted,
        };
        let hint_w = ctx.text.measure(hint, &style).0;
        ui::paint_text_vcentered(ctx, list, hint, row.x + row.width - hint_w - m.gap, row, m.caption, ctx.metrics.typography.weight_regular, if hovered { ctx.theme.secondary } else { ctx.theme.text_muted });
        if f.clicked_in(row) && !address.is_empty() {
            f.events.push(MenuEvent::System(SystemAction::BluetoothConnect { address, connect: !connected }));
        }
        f.gap(m.row_gap);
    }
    if paired.len() > m.max_devices {
        f.note(ctx, list, &format!("+{} more in full settings", paired.len() - m.max_devices));
    }
}

fn network(ctx: &mut PaintCtx<'_>, list: &mut DisplayList, f: &mut Frame) {
    let m = ctx.metrics.hover_menu.clone();
    let net = ctx.state.net.get();
    let connections: Vec<_> = net["connections"].as_array().cloned().unwrap_or_default();
    if connections.is_empty() {
        f.note(ctx, list, "Not connected");
    }
    for (i, c) in connections.iter().enumerate() {
        if i > 0 {
            let rule = f.take(m.border);
            list.push_rect(Rect { color: ctx.theme.border_inactive, ..rule });
            f.gap(m.gap);
        }
        let ethernet = c["type"].as_str() == Some("ethernet");
        let name = c["connection"].as_str().filter(|s| !s.is_empty()).unwrap_or(if ethernet { "Ethernet" } else { "Unknown" });
        let signal = c["signal"].as_str().and_then(|s| s.parse::<i64>().ok()).or_else(|| c["signal"].as_i64());
        let row = f.take(m.row_height);
        ui::paint_text_vcentered(ctx, list, if ethernet { "\u{f0200}" } else { "\u{f0928}" }, row.x, row, m.icon, ctx.metrics.typography.weight_regular, ctx.theme.primary);
        ui::paint_text_vcentered(ctx, list, name, row.x + m.icon + m.gap, row, m.text, ctx.metrics.typography.weight_medium, ctx.theme.text_main);
        if let (false, Some(pct)) = (ethernet, signal) {
            let bars = signal_bars(pct);
            let style = crate::text::TextStyle {
                family: std::sync::Arc::from(ctx.theme.font.as_str()),
                size: m.caption,
                weight: ctx.metrics.typography.weight_regular,
                color: ctx.theme.text_muted,
            };
            let w = ctx.text.measure(&bars, &style).0;
            ui::paint_text_vcentered(ctx, list, &bars, row.x + row.width - w, row, m.caption, ctx.metrics.typography.weight_regular, ctx.theme.primary);
        }
        for (label, key) in [("IP", "ip_address"), ("Gateway", "gateway"), ("Device", "device")] {
            if let Some(value) = c[key].as_str().filter(|v| !v.is_empty()) {
                f.stat(ctx, list, label, value);
            }
        }
        if let (false, Some(pct)) = (ethernet, signal) {
            f.stat(ctx, list, "Signal", &format!("{pct}%"));
        }
    }
}

/// Four bars, as the QML menu drew them.
fn signal_bars(pct: i64) -> String {
    let level = match pct {
        75.. => 4,
        50..=74 => 3,
        25..=49 => 2,
        1..=24 => 1,
        _ => 0,
    };
    (0..4).map(|i| if i < level { '\u{25ae}' } else { '\u{25af}' }).collect()
}

/// (name, label) pairs from an audio device list.
fn devices(value: &serde_json::Value) -> Vec<(String, String)> {
    value
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|d| {
                    let name = d["name"].as_str()?.to_string();
                    let label = d["description"].as_str().filter(|s| !s.is_empty()).unwrap_or(&name).to_string();
                    Some((name, label))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn line(m: &crate::ui::style::HoverMenuMetrics, size: f32) -> f32 {
    size * m.line_height
}

/// Layout cursor for one paint.
struct Frame {
    x: f32,
    y: f32,
    width: f32,
    pointer: Option<Point>,
    clicked: bool,
    events: Vec<MenuEvent>,
}

impl Frame {
    fn take(&mut self, height: f32) -> Rect {
        let rect = Rect {
            x: self.x,
            y: self.y,
            width: self.width,
            height,
            radius: 0.0,
            color: Color::TRANSPARENT,
        };
        self.y += height;
        rect
    }

    fn gap(&mut self, height: f32) {
        self.y += height;
    }

    fn hover(&self, rect: Rect) -> bool {
        self.pointer.is_some_and(|p| rect.contains(p))
    }

    fn clicked_in(&self, rect: Rect) -> bool {
        self.clicked && self.hover(rect)
    }

    fn header(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, title: &str) {
        let m = ctx.metrics.hover_menu.clone();
        let row = self.take(line(&m, m.title));
        ui::paint_text_vcentered(ctx, list, title, row.x, row, m.title, ctx.metrics.typography.weight_bold, ctx.theme.text_main);
        let close = Rect {
            x: row.x + row.width - m.close,
            width: m.close,
            ..row
        };
        let hovered = self.hover(close);
        ui::paint_text_centered(ctx, list, "\u{f0156}", close, m.icon, if hovered { ctx.theme.text_main } else { ctx.theme.text_muted });
        if self.clicked_in(close) {
            self.events.push(MenuEvent::Close);
        }
        self.gap(m.section_gap);
    }

    fn section(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, label: &str) {
        let m = ctx.metrics.hover_menu.clone();
        self.gap(m.section_gap);
        let row = self.take(line(&m, m.caption));
        ui::paint_text_vcentered(ctx, list, label, row.x, row, m.caption, ctx.metrics.typography.weight_bold, ctx.theme.text_muted);
        self.gap(m.gap);
    }

    fn note(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, text: &str) {
        let m = ctx.metrics.hover_menu.clone();
        let row = self.take(line(&m, m.text));
        ui::paint_text_vcentered(ctx, list, text, row.x, row, m.text, ctx.metrics.typography.weight_regular, ctx.theme.text_muted);
    }

    fn stat(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, label: &str, value: &str) {
        let m = ctx.metrics.hover_menu.clone();
        let row = self.take(line(&m, m.text));
        ui::paint_text_vcentered(ctx, list, label, row.x, row, m.caption, ctx.metrics.typography.weight_regular, ctx.theme.text_muted);
        let value_x = row.x + row.width * m.stat_split;
        let value = ui::wrap_text(ctx, value, m.text, ctx.metrics.typography.weight_regular, row.x + row.width - value_x, 1);
        if let Some(value) = value.first() {
            ui::paint_text_vcentered(ctx, list, value, value_x, row, m.text, ctx.metrics.typography.weight_regular, ctx.theme.text_main);
        }
    }

    /// A device row with a radio mark. Returns true when clicked.
    fn radio_row(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, label: &str, selected: bool) -> bool {
        let m = ctx.metrics.hover_menu.clone();
        let row = self.take(m.row_height);
        if self.hover(row) || selected {
            list.push_rect(Rect { radius: m.button_radius, color: ctx.theme.bg_hover, ..row });
        }
        let mark = Rect { width: m.row_height, ..row };
        ui::paint_text_centered(ctx, list, if selected { "\u{25c9}" } else { "\u{25cb}" }, mark, m.text, if selected { ctx.theme.primary } else { ctx.theme.text_muted });
        let label = ui::wrap_text(ctx, label, m.text, ctx.metrics.typography.weight_regular, row.width - m.row_height - m.gap, 1);
        if let Some(label) = label.first() {
            ui::paint_text_vcentered(ctx, list, label, row.x + m.row_height, row, m.text, ctx.metrics.typography.weight_regular, if selected { ctx.theme.text_main } else { ctx.theme.text_muted });
        }
        self.gap(m.row_gap);
        self.clicked_in(row)
    }

    /// A text link. Returns true when clicked.
    fn link(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, text: &str) -> bool {
        let m = ctx.metrics.hover_menu.clone();
        let row = self.take(line(&m, m.text));
        let hovered = self.hover(row);
        ui::paint_text_vcentered(ctx, list, text, row.x, row, m.text, ctx.metrics.typography.weight_medium, if hovered { ctx.theme.text_main } else { ctx.theme.text_muted });
        self.clicked_in(row)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPEN: Duration = Duration::from_millis(350);
    const CLOSE: Duration = Duration::from_millis(800);

    fn menu(kind: HoverKind) -> OpenMenu {
        OpenMenu {
            kind,
            output: "HDMI-A-1".into(),
            anchor_x: 20.0,
            anchor_y: 900.0,
        }
    }

    fn ms(base: Instant, n: u64) -> Instant {
        base + Duration::from_millis(n)
    }

    #[test]
    fn opens_only_after_resting_on_the_icon() {
        let mut intent = HoverIntent::default();
        let t = Instant::now();
        assert!(!intent.update(t, Some(menu(HoverKind::Audio)), false, OPEN, CLOSE));
        assert!(intent.pending());
        assert!(!intent.update(ms(t, 200), Some(menu(HoverKind::Audio)), false, OPEN, CLOSE));
        assert!(intent.update(ms(t, 360), Some(menu(HoverKind::Audio)), false, OPEN, CLOSE));
        assert_eq!(intent.open().map(|o| o.kind), Some(HoverKind::Audio));
    }

    #[test]
    fn passing_over_an_icon_does_not_open_it() {
        let mut intent = HoverIntent::default();
        let t = Instant::now();
        intent.update(t, Some(menu(HoverKind::Wifi)), false, OPEN, CLOSE);
        intent.update(ms(t, 100), None, false, OPEN, CLOSE);
        assert!(!intent.update(ms(t, 500), None, false, OPEN, CLOSE));
        assert!(intent.open().is_none());
        assert!(!intent.pending());
    }

    #[test]
    fn stays_open_crossing_the_gap_and_while_over_the_menu() {
        let mut intent = HoverIntent::default();
        let t = Instant::now();
        intent.update(t, Some(menu(HoverKind::Battery)), false, OPEN, CLOSE);
        intent.update(ms(t, 400), Some(menu(HoverKind::Battery)), false, OPEN, CLOSE);
        // Off the icon, in the gap: still open for a while.
        intent.update(ms(t, 500), None, false, OPEN, CLOSE);
        intent.update(ms(t, 900), None, false, OPEN, CLOSE);
        assert!(intent.open().is_some(), "the grace period covers the gap");
        // Onto the menu: stays open however long.
        intent.update(ms(t, 1000), None, true, OPEN, CLOSE);
        intent.update(ms(t, 9000), None, true, OPEN, CLOSE);
        assert!(intent.open().is_some());
        // Away from both: closes after the grace period.
        intent.update(ms(t, 9100), None, false, OPEN, CLOSE);
        assert!(intent.open().is_some());
        assert!(intent.update(ms(t, 9950), None, false, OPEN, CLOSE));
        assert!(intent.open().is_none());
    }

    #[test]
    fn resting_on_another_icon_switches_menus() {
        let mut intent = HoverIntent::default();
        let t = Instant::now();
        intent.update(t, Some(menu(HoverKind::Audio)), false, OPEN, CLOSE);
        intent.update(ms(t, 400), Some(menu(HoverKind::Audio)), false, OPEN, CLOSE);
        intent.update(ms(t, 500), Some(menu(HoverKind::Wifi)), false, OPEN, CLOSE);
        assert_eq!(intent.open().map(|o| o.kind), Some(HoverKind::Audio), "not before the delay");
        intent.update(ms(t, 900), Some(menu(HoverKind::Wifi)), false, OPEN, CLOSE);
        assert_eq!(intent.open().map(|o| o.kind), Some(HoverKind::Wifi));
    }

    #[test]
    fn signal_bars_match_the_qml_thresholds() {
        assert_eq!(signal_bars(80), "\u{25ae}\u{25ae}\u{25ae}\u{25ae}");
        assert_eq!(signal_bars(67), "\u{25ae}\u{25ae}\u{25ae}\u{25af}");
        assert_eq!(signal_bars(10), "\u{25ae}\u{25af}\u{25af}\u{25af}");
        assert_eq!(signal_bars(0), "\u{25af}\u{25af}\u{25af}\u{25af}");
    }

    #[test]
    fn device_labels_fall_back_to_names() {
        let value = serde_json::json!([
            {"name": "alsa_output.pci", "description": "Speakers"},
            {"name": "bluez_output.80", "description": ""},
        ]);
        assert_eq!(
            devices(&value),
            vec![
                ("alsa_output.pci".to_string(), "Speakers".to_string()),
                ("bluez_output.80".to_string(), "bluez_output.80".to_string()),
            ]
        );
    }
}
