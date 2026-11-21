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
    /// Removable drives; it has no settings page.
    Drives,
}

impl HoverKind {
    pub fn page(self) -> Option<Page> {
        match self {
            HoverKind::Audio => Some(Page::Audio),
            HoverKind::Bluetooth => Some(Page::Bluetooth),
            HoverKind::Wifi => Some(Page::Wifi),
            HoverKind::Battery => Some(Page::Battery),
            HoverKind::Drives => None,
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

    /// Open `menu` now, as a click on an icon without a page does.
    pub fn show(&mut self, menu: OpenMenu) {
        self.open = Some(menu);
        self.resting = None;
        self.left_at = None;
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
    /// The power dial's needle, in degrees, the angle it is swinging to,
    /// and where the pointer holds it while dragged.
    needle: crate::ui::animation::Tween,
    needle_target: Option<f32>,
    dial_drag: Option<f32>,
}

impl HoverMenus {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the open menu moves on its own (the power dial's needle) and
    /// must be painted every frame.
    pub fn animating(&self) -> bool {
        self.intent.open().is_some_and(|o| o.kind == HoverKind::Battery) && (self.needle.is_animating() || self.dial_drag.is_some())
    }

    pub fn tick(&mut self, dt: f32) {
        self.needle.tick(dt);
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
            HoverKind::Drives => ("\u{f129e}", "Drives"),
        };
        frame.header(ctx, list, &format!("{glyph}  {title}"));

        match kind {
            HoverKind::Audio => self.audio(ctx, list, &mut frame, input),
            HoverKind::Bluetooth => bluetooth(ctx, list, &mut frame),
            HoverKind::Wifi => network(ctx, list, &mut frame),
            HoverKind::Battery => self.battery(ctx, list, &mut frame, input),
            HoverKind::Drives => drives(ctx, list, &mut frame),
        }

        if let Some(page) = kind.page() {
            frame.gap(m.section_gap);
            if frame.link(ctx, list, "Open full settings \u{2192}") {
                frame.events.push(MenuEvent::OpenPage(page));
            }
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

    /// The battery: a big percentage over a level bar, three stat chips,
    /// the power mode on a tachometer, and the stay-awake and night-light
    /// tiles that used to be bar icons.
    fn battery(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, f: &mut Frame, input: &InputState) {
        let m = ctx.metrics.hover_menu.clone();
        let snapshot = ctx.state.battery.get();
        let battery = &snapshot["battery"];
        let state = battery["state"].as_str().unwrap_or("");
        let pct = battery["pct"].as_u64().unwrap_or(0).min(100);
        // Labels from BatteryHoverMenu.qml.
        let state_text = match state {
            "charging" => "Charging",
            "discharging" => "On Battery",
            "fully-charged" => "Full",
            "pending-charge" => "Not charging",
            _ => "Battery",
        };
        let level_color = if state == "charging" || state == "fully-charged" {
            ctx.theme.success
        } else if pct <= 20 {
            ctx.theme.error
        } else if pct <= 40 {
            ctx.theme.warning
        } else {
            ctx.theme.primary
        };

        // The percentage, large, with the state beside it.
        let hero = f.take(line(&m, m.hero));
        ui::paint_text_vcentered(ctx, list, &format!("{pct}%"), hero.x, hero, m.hero, ctx.metrics.typography.weight_bold, ctx.theme.text_main);
        let state_w = measure(ctx, state_text, m.text, ctx.metrics.typography.weight_medium);
        let glyph = if state == "charging" { "\u{f140b} " } else { "" };
        let glyph_w = if glyph.is_empty() { 0.0 } else { measure(ctx, glyph, m.text, ctx.metrics.typography.weight_medium) };
        let state_x = hero.x + hero.width - state_w - glyph_w;
        if !glyph.is_empty() {
            ui::paint_text_vcentered(ctx, list, glyph, state_x, hero, m.text, ctx.metrics.typography.weight_medium, level_color);
        }
        ui::paint_text_vcentered(ctx, list, state_text, state_x + glyph_w, hero, m.text, ctx.metrics.typography.weight_medium, level_color);
        f.gap(m.gap);

        // The level bar, with a terminal nub like a battery's.
        let bar = f.take(m.bar_height);
        let nub_w = m.bar_height * 0.35;
        let track = Rect { width: bar.width - nub_w - m.border * 2.0, radius: m.bar_height * 0.5, color: ctx.theme.bg_widget, ..bar };
        list.push_rect(track);
        if pct > 0 {
            let fill_w = (track.width * pct as f32 / 100.0).max(track.height);
            list.push_rect(Rect { width: fill_w, color: level_color, ..track });
        }
        let nub_h = m.bar_height * 0.5;
        list.push_rect(Rect {
            x: track.x + track.width + m.border * 2.0,
            y: bar.y + (bar.height - nub_h) * 0.5,
            width: nub_w,
            height: nub_h,
            radius: nub_w * 0.5,
            color: ctx.theme.bg_widget,
        });
        f.gap(m.section_gap);

        // Stat chips: screen on, then the time left and the rate where
        // there are any (plugged in and full, the hero line says so).
        let mut chips = vec![("Screen on", battery["screen_on_time"].as_str().unwrap_or("\u{2014}").to_string())];
        let projected = battery["projected_text"].as_str().filter(|t| !t.is_empty() && *t != "Unknown");
        match (state, projected) {
            ("charging", Some(time)) => chips.push(("To full", time.to_string())),
            ("discharging", Some(time)) => chips.push(("Remaining", time.to_string())),
            _ => {}
        }
        // upower's `45.312 W`, to one decimal.
        if let Some(watts) = snapshot["charging_info"].as_str().and_then(|r| r.strip_suffix(" W")).and_then(|w| w.trim().parse::<f32>().ok()) {
            chips.push(("Rate", format!("{watts:.1} W")));
        }
        let row = f.take(m.chip_height);
        let chip_w = (row.width - m.gap * (chips.len() - 1) as f32) / chips.len() as f32;
        for (i, (label, value)) in chips.iter().enumerate() {
            let chip = Rect { x: row.x + i as f32 * (chip_w + m.gap), width: chip_w, radius: m.button_radius, color: ctx.theme.bg_widget, ..row };
            list.push_rect(chip);
            let inner = Rect { x: chip.x + m.gap, width: chip.width - m.gap * 2.0, ..chip };
            let half = Rect { height: chip.height * 0.5, y: chip.y + m.gap * 0.5, ..inner };
            ui::paint_text_vcentered(ctx, list, label, inner.x, half, m.caption, ctx.metrics.typography.weight_regular, ctx.theme.text_muted);
            let value = ui::elide(ctx, value, m.text, ctx.metrics.typography.weight_medium, inner.width);
            let lower = Rect { y: chip.y + chip.height * 0.5 - m.gap * 0.5, ..half };
            ui::paint_text_vcentered(ctx, list, &value, inner.x, lower, m.text, ctx.metrics.typography.weight_medium, ctx.theme.text_main);
        }

        f.section(ctx, list, "\u{f04c5}  Power Mode");
        self.power_dial(ctx, list, f, input, &snapshot);

        f.section(ctx, list, "Quick Settings");
        let row = f.take(m.tile_height);
        let tile_w = (row.width - m.gap) * 0.5;
        let awake = ctx.state.idle_inhibit.get();
        let night = ctx.state.night_light.get();
        let night_text = if night.problem.is_some() {
            "Unavailable".to_string()
        } else {
            let until = night
                .manual
                .and(night.until)
                .filter(|t| *t < i64::MAX)
                .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
                .map(|t| t.with_timezone(&chrono::Local).format("%H:%M").to_string());
            match (night.on, until) {
                (true, Some(t)) => format!("On until {t}"),
                (false, Some(t)) => format!("Off until {t}"),
                (true, None) => "On".into(),
                (false, None) => "Off".into(),
            }
        };
        // nf-md-coffee / coffee_outline, weather_night / moon_waning_crescent.
        let tiles = [
            (if awake { "\u{f0176}" } else { "\u{f06ca}" }, "Stay awake", if awake { "Idle held off".to_string() } else { "Off".to_string() }, awake),
            (if night.on { "\u{f0594}" } else { "\u{f0f65}" }, "Night light", night_text, night.on),
        ];
        for (i, (glyph, title, subtitle, on)) in tiles.iter().enumerate() {
            let tile = Rect { x: row.x + i as f32 * (tile_w + m.gap), width: tile_w, radius: m.button_radius, ..row };
            let hovered = f.hover(tile);
            let fill = if *on {
                crate::ui::pages::layout::mix(ctx.theme.bg_widget, ctx.theme.primary, 0.22)
            } else if hovered {
                ctx.theme.bg_hover
            } else {
                ctx.theme.bg_widget
            };
            ui::paint_frame(list, tile, m.border, if *on { ctx.theme.primary } else { ctx.theme.border_inactive }, fill);
            let icon_box = Rect { width: m.icon * 2.0, ..tile };
            ui::paint_text_centered(ctx, list, glyph, icon_box, m.icon, if *on { ctx.theme.primary } else { ctx.theme.text_muted });
            let text_x = icon_box.x + icon_box.width;
            let text_w = tile.x + tile.width - text_x - m.gap;
            let top = Rect { y: tile.y + m.gap * 0.5, height: tile.height * 0.5 - m.gap * 0.5, ..tile };
            let bottom = Rect { y: tile.y + tile.height * 0.5, ..top };
            let title = ui::elide(ctx, title, m.text, ctx.metrics.typography.weight_medium, text_w);
            ui::paint_text_vcentered(ctx, list, &title, text_x, top, m.text, ctx.metrics.typography.weight_medium, ctx.theme.text_main);
            let subtitle = ui::elide(ctx, subtitle, m.caption, ctx.metrics.typography.weight_regular, text_w);
            ui::paint_text_vcentered(ctx, list, &subtitle, text_x, bottom, m.caption, ctx.metrics.typography.weight_regular, if *on { ctx.theme.primary } else { ctx.theme.text_muted });
            if f.clicked_in(tile) {
                f.events.push(MenuEvent::System(if i == 0 { SystemAction::IdleInhibit(!awake) } else { SystemAction::NightLight("toggle") }));
            }
        }
    }

    /// The power mode as a tachometer: economy, normal, the redline. Drag
    /// the needle (or click a zone); it settles in the zone it is let go
    /// in, and the mode follows.
    fn power_dial(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, f: &mut Frame, input: &InputState, snapshot: &serde_json::Value) {
        use crate::ui::tachometer as dial;
        let m = ctx.metrics.hover_menu.clone();
        // nf-md-leaf, scale_balance, rocket_launch.
        let modes = [
            ("low-power", "Saver", "Longest battery", "\u{f032a}", ctx.theme.success),
            ("balanced", "Normal", "Balanced", "\u{f05d1}", ctx.theme.primary),
            ("balanced-performance", "Performance", "Fastest, louder fans", "\u{f14de}", ctx.theme.error),
        ];
        let n = modes.len();
        let real = snapshot["profile"].as_str().unwrap_or("balanced").to_string();
        let active = self.profile.view(real);
        let active_index = modes.iter().position(|(key, ..)| *key == active).unwrap_or(1);

        // Room for the mode glyphs outside the arc, above and beside it,
        // and the readout in the open bottom.
        let outer = m.dial_radius + m.dial_band * 0.5 + m.gap + m.icon;
        let below = m.dial_radius * 0.5 + m.dial_band + line(&m, m.text) * 0.5;
        let area = f.take(outer + below);
        let centre = Point::new(area.x + area.width * 0.5, area.y + outer);

        // The needle: dragged, or swinging to the active mode.
        let target = dial::zone_centre(active_index, n);
        if self.needle_target.is_none() {
            self.needle.snap(target);
        } else if self.dial_drag.is_none() && self.needle_target != Some(target) {
            self.needle.to(target, m.dial_seconds, crate::ui::animation::Easing::OutBack);
        }
        self.needle_target = Some(target);

        let over = f.pointer.filter(|p| {
            let (dx, dy) = (p.x - centre.x, p.y - centre.y);
            let distance = (dx * dx + dy * dy).sqrt();
            distance <= m.dial_radius + m.dial_band && area.contains(*p)
        });
        let mut chosen = None;
        match (self.dial_drag, over, f.clicked, input.pressed) {
            // A press on the dial takes the needle.
            (None, Some(p), true, true) => self.dial_drag = Some(dial::pointer_angle(centre, p)),
            // Pressed and let go between two paints: a click on a zone.
            (None, Some(p), true, false) => chosen = Some(dial::zone_at(dial::pointer_angle(centre, p), n)),
            // Dragging: the needle follows the pointer anywhere.
            (Some(_), _, _, true) => {
                if let Some(p) = f.pointer {
                    self.dial_drag = Some(dial::pointer_angle(centre, p));
                }
            }
            // Let go: it settles in the zone it was let go in.
            (Some(angle), _, _, false) => {
                self.dial_drag = None;
                self.needle.snap(angle);
                chosen = Some(dial::zone_at(angle, n));
            }
            _ => {}
        }
        if let Some(index) = chosen {
            let key = modes[index].0;
            let centre_angle = dial::zone_centre(index, n);
            self.needle.to(centre_angle, m.dial_seconds, crate::ui::animation::Easing::OutBack);
            self.needle_target = Some(centre_angle);
            if key != active {
                self.profile.set(key.to_string());
                f.events.push(MenuEvent::System(SystemAction::SetPowerProfile(key.to_string())));
            }
        }

        let angle = self.dial_drag.unwrap_or_else(|| self.needle.value());
        let pointing = dial::zone_at(angle, n);
        let style = dial::Style {
            radius: m.dial_radius,
            band: m.dial_band,
            tick: m.dial_tick,
            needle: m.dial_needle,
            hub: m.dial_hub,
            zones: modes.iter().map(|mode| mode.4).collect(),
            lit: pointing,
            track: ctx.theme.bg_widget,
            ticks: ctx.theme.text_muted,
            hub_fill: ctx.theme.bg_main,
        };
        dial::paint(list, centre, angle, &style);

        // Each mode's glyph outside its zone.
        for (i, (_, _, _, glyph, color)) in modes.iter().enumerate() {
            let at = dial::point(centre, m.dial_radius + m.dial_band * 0.5 + m.gap + m.icon * 0.5, dial::zone_centre(i, n));
            let cell = Rect { x: at.x - m.icon * 0.5, y: at.y - m.icon * 0.5, width: m.icon, height: m.icon, radius: 0.0, color: Color::TRANSPARENT };
            ui::paint_text_centered(ctx, list, glyph, cell, m.icon, if i == pointing { *color } else { ctx.theme.text_muted });
        }

        // The readout in the open bottom: the mode the needle points at.
        let (_, name, detail, _, color) = modes[pointing];
        let name_row = Rect {
            x: area.x,
            y: centre.y + m.dial_hub,
            width: area.width,
            height: line(&m, m.title),
            radius: 0.0,
            color: Color::TRANSPARENT,
        };
        ui::paint_text_centered_weighted(ctx, list, name, name_row, m.title, ctx.metrics.typography.weight_bold, color);
        let detail_row = Rect { y: name_row.y + name_row.height, height: line(&m, m.caption), ..name_row };
        ui::paint_text_centered(ctx, list, detail, detail_row, m.caption, ctx.theme.text_muted);
        // The readout may reach past what the arc needed below.
        let bottom = detail_row.y + detail_row.height;
        if bottom > f.y {
            f.y = bottom;
        }
    }
}

/// A text's width at `size` and `weight` in the theme's font.
fn measure(ctx: &mut PaintCtx<'_>, text: &str, size: f32, weight: u16) -> f32 {
    let style = crate::text::TextStyle {
        family: std::sync::Arc::from(ctx.theme.font.as_str()),
        size,
        weight,
        color: ctx.theme.text_main,
    };
    ctx.text.measure(text, &style).0
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

/// Each removable drive with an eject button, and its filesystems: a click
/// opens one (mounting it first), and a mounted one can be unmounted.
fn drives(ctx: &mut PaintCtx<'_>, list: &mut DisplayList, f: &mut Frame) {
    use crate::drives::DriveOp;
    let m = ctx.metrics.hover_menu.clone();
    let drives: Vec<_> = ctx.state.drives.get().into_iter().filter(|d| d.removable).collect();
    let busy = ctx.state.drives_busy.get();
    if drives.is_empty() {
        f.note(ctx, list, "No drives");
        return;
    }
    let caption = |ctx: &mut PaintCtx<'_>, text: &str| {
        let style = crate::text::TextStyle {
            family: std::sync::Arc::from(ctx.theme.font.as_str()),
            size: m.caption,
            weight: ctx.metrics.typography.weight_regular,
            color: ctx.theme.text_muted,
        };
        ctx.text.measure(text, &style).0
    };
    for (i, drive) in drives.iter().enumerate() {
        if i > 0 {
            f.gap(m.gap);
            let rule = f.take(m.border);
            list.push_rect(Rect { color: ctx.theme.border_inactive, ..rule });
            f.gap(m.gap);
        }
        let row = f.take(m.row_height);
        let eject = Rect { x: row.x + row.width - m.row_height, width: m.row_height, radius: m.button_radius, ..row };
        let ejecting = busy.contains(&drive.object);
        ui::paint_text_vcentered(ctx, list, crate::ui::widgets::drives::glyph(drive), row.x, row, m.icon, ctx.metrics.typography.weight_regular, ctx.theme.primary);
        let name_x = row.x + m.icon + m.gap;
        let name = ui::wrap_text(ctx, &drive.name, m.text, ctx.metrics.typography.weight_medium, eject.x - name_x - m.gap, 1);
        if let Some(name) = name.first() {
            ui::paint_text_vcentered(ctx, list, name, name_x, row, m.text, ctx.metrics.typography.weight_medium, ctx.theme.text_main);
        }
        let hovered = f.hover(eject);
        if hovered && !ejecting {
            list.push_rect(Rect { color: ctx.theme.bg_hover, ..eject });
        }
        let glyph = if ejecting { "\u{2026}" } else { "\u{f01ea}" };
        ui::paint_text_centered(ctx, list, glyph, eject, m.icon, if hovered { ctx.theme.text_main } else { ctx.theme.text_muted });
        if f.clicked_in(eject) && !ejecting {
            f.events.push(MenuEvent::System(SystemAction::Drive(DriveOp::Eject(drive.object.clone()))));
        }
        f.gap(m.row_gap);

        for volume in &drive.volumes {
            let row = f.take(m.row_height);
            let working = ejecting || busy.contains(&volume.object);
            let mounted = volume.mounted().is_some();
            let hint = if working {
                "Working\u{2026}"
            } else if volume.locked {
                "Unlock"
            } else if mounted {
                "Unmount"
            } else {
                "Open"
            };
            let hint_w = caption(ctx, hint);
            let hint_rect = Rect { x: row.x + row.width - hint_w - m.gap * 2.0, width: hint_w + m.gap * 2.0, ..row };
            let hovered = f.hover(row);
            if hovered {
                list.push_rect(Rect { radius: m.button_radius, color: ctx.theme.bg_hover, ..row });
            }
            let mark_rect = Rect { x: name_x, width: m.icon, ..row };
            ui::paint_text_centered(ctx, list, if mounted { "\u{25cf}" } else { "\u{25cb}" }, mark_rect, m.caption, if mounted { ctx.theme.primary } else { ctx.theme.text_muted });
            let label_x = name_x + m.icon + m.gap;
            let size = stratum_drives::format_size(volume.size);
            let size_w = caption(ctx, &size);
            let label_room = hint_rect.x - label_x - size_w - m.gap * 2.0;
            let label = ui::wrap_text(ctx, &volume.name(), m.text, ctx.metrics.typography.weight_regular, label_room.max(0.0), 1);
            let label = label.first().cloned().unwrap_or_default();
            ui::paint_text_vcentered(ctx, list, &label, label_x, row, m.text, ctx.metrics.typography.weight_regular, if mounted { ctx.theme.text_main } else { ctx.theme.text_muted });
            if volume.label.trim().is_empty() {
                // Its name is its size already.
            } else {
                let style = crate::text::TextStyle {
                    family: std::sync::Arc::from(ctx.theme.font.as_str()),
                    size: m.text,
                    weight: ctx.metrics.typography.weight_regular,
                    color: ctx.theme.text_main,
                };
                let label_w = ctx.text.measure(&label, &style).0;
                ui::paint_text_vcentered(ctx, list, &size, label_x + label_w + m.gap, row, m.caption, ctx.metrics.typography.weight_regular, ctx.theme.text_muted);
            }
            let hint_hovered = f.hover(hint_rect);
            let hint_color = if hint_hovered && mounted && !working { ctx.theme.secondary } else if hovered { ctx.theme.text_main } else { ctx.theme.text_muted };
            ui::paint_text_vcentered(ctx, list, hint, hint_rect.x + m.gap, row, m.caption, ctx.metrics.typography.weight_regular, hint_color);
            if f.clicked_in(row) && !working {
                if mounted && hint_hovered {
                    f.events.push(MenuEvent::System(SystemAction::Drive(DriveOp::Unmount(volume.object.clone()))));
                } else {
                    f.events.push(MenuEvent::System(SystemAction::Drive(DriveOp::Open(volume.object.clone()))));
                    f.events.push(MenuEvent::Close);
                }
            }
            f.gap(m.row_gap);
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
