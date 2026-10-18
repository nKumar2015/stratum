//! The control centre: a paged settings surface for the shell.
//!
//! It renders on a fullscreen transparent overlay so it can dismiss on a click
//! outside the panel, matching the QML menus. Bar widgets open it on a specific
//! page: clicking the battery icon lands on Battery, the audio icon on Audio,
//! and so on.
//!
//! Pages are driven by the same `SystemState` as the bar, so what the panel
//! shows is always the state the bar is showing.

use std::sync::Arc;

use crate::config::{Config, SidebarPosition};
use crate::render::display_list::{Color, DisplayList, Rect};
use crate::sys_state::SystemState;
use crate::ui::{self, Action, InputState, PaintCtx, Widget};

/// A control-centre page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Overview,
    Audio,
    Bluetooth,
    Wifi,
    Battery,
    Display,
    Power,
}

impl Page {
    /// Pages shown in the navigation rail, in order.
    pub const ALL: [Page; 7] = [
        Page::Overview,
        Page::Audio,
        Page::Bluetooth,
        Page::Wifi,
        Page::Battery,
        Page::Display,
        Page::Power,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Page::Overview => "Overview",
            Page::Audio => "Audio",
            Page::Bluetooth => "Bluetooth",
            Page::Wifi => "Network",
            Page::Battery => "Battery",
            Page::Display => "Display",
            Page::Power => "Power",
        }
    }

    /// Nerd Font glyph for the navigation rail.
    pub fn glyph(self) -> &'static str {
        match self {
            Page::Overview => "\u{f009}",
            Page::Audio => "\u{f0581}",
            Page::Bluetooth => "\u{f00b1}",
            Page::Wifi => "\u{f05a9}",
            Page::Battery => "\u{f007a}",
            Page::Display => "\u{f108}",
            Page::Power => "\u{f011}",
        }
    }
}

/// Visual state of the control centre.
pub struct ControlCenter {
    pub open: bool,
    pub page: Page,
    config: Arc<Config>,
    /// Panel size in logical pixels.
    width: f32,
    /// Animation progress 0..1, driven by the shell's frame ticker.
    pub progress: f32,
    /// Actions produced by the last paint, drained by the shell.
    pub pending: Vec<crate::actions::SystemAction>,
    /// Which slider is being dragged, if any.
    dragging: Option<SliderId>,
    /// Pointer was pressed during the last frame, so a click can be detected.
    pressed_last: bool,
    /// True when this paint follows a fresh press.
    clicked_this_frame: bool,
}

/// Layout constants shared by painting and hit-testing. These must agree, or
/// clicks land on the wrong row.
const NAV_WIDTH: f32 = 150.0;
const NAV_HEADER: f32 = 52.0;
const NAV_ROW_HEIGHT: f32 = 36.0;
const NAV_ROW_SPACING: f32 = 40.0;
const NAV_INSET: f32 = 8.0;
const NAV_TOP_GAP: f32 = 6.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SliderId {
    Volume,
}

impl ControlCenter {
    pub fn new(config: Arc<Config>) -> Self {
        let width = config.control_center.width as f32;
        Self {
            open: false,
            page: Page::Overview,
            config,
            width,
            progress: 0.0,
            pending: Vec::new(),
            dragging: None,
            pressed_last: false,
            clicked_this_frame: false,
        }
    }

    /// Take any actions produced by the last paint.
    pub fn take_actions(&mut self) -> Vec<crate::actions::SystemAction> {
        std::mem::take(&mut self.pending)
    }

    /// Open on `page`, or close if already open on that page.
    pub fn toggle(&mut self, page: Page) {
        if self.open && self.page == page {
            self.open = false;
        } else {
            self.open = true;
            self.page = page;
        }
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    /// Advance the open/close animation. Returns true while animating.
    pub fn tick(&mut self, dt_seconds: f32) -> bool {
        let target = if self.open { 1.0 } else { 0.0 };
        let speed = if self.config.control_center.animation_ms == 0 {
            f32::INFINITY
        } else {
            1000.0 / self.config.control_center.animation_ms as f32
        };
        let step = (speed * dt_seconds).min(1.0);
        if (self.progress - target).abs() <= step {
            self.progress = target;
            false
        } else {
            self.progress += (target - self.progress).signum() * step;
            true
        }
    }

    /// True when the surface should be mapped (animating or open).
    pub fn visible(&self) -> bool {
        self.open || self.progress > 0.001
    }

    /// Geometry of the panel inside a fullscreen surface.
    fn panel_rect(&self, surface: Rect) -> Rect {
        let width = self.width.min(surface.width - 40.0);
        let height = (surface.height - 80.0).max(200.0);
        // Slide in from the edge the bar lives on.
        let slide = (1.0 - self.progress) * (width + 24.0);
        let x = match self.config.sidebar.position {
            SidebarPosition::Right => surface.x + surface.width - width - 12.0 + slide,
            _ => surface.x + 12.0 - slide,
        };
        Rect {
            x,
            y: surface.y + 40.0,
            width,
            height,
            radius: self.config.appearance.corner_radius,
            color: Color::TRANSPARENT,
        }
    }

    /// Paint into a normal window: no scrim and no slide, just the panel
    /// filling the window. Returns the content rect so clicks can be routed.
    pub fn paint_window(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        surface: Rect,
        input: &InputState,
    ) -> Rect {
        self.pending.clear();
        // A click is a press edge: `pressed` true now, false last frame.
        let clicked = input.pressed && !self.pressed_last;
        self.pressed_last = input.pressed;
        self.clicked_this_frame = clicked;
        // Fill the whole window: `surface` carries the caller's colour, which
        // may be transparent, so paint an explicit background.
        list.push_rect(Rect {
            color: ctx.theme.bg_main,
            ..surface
        });
        let panel = Rect {
            x: 0.0,
            y: 0.0,
            width: surface.width,
            height: surface.height,
            radius: 0.0,
            color: ctx.theme.bg_main,
        };

        // Navigation rail down the left of the panel.
        let nav_width = NAV_WIDTH;
        let nav = Rect {
            x: panel.x,
            y: panel.y,
            width: nav_width,
            height: panel.height,
            radius: 0.0,
            color: Color::TRANSPARENT,
        };

        let header_height = NAV_HEADER;
        ui::paint_text_vcentered(
            ctx,
            list,
            "Settings",
            nav.x + 18.0,
            Rect { height: header_height, ..nav },
            15.0,
            700,
            ctx.theme.text_main,
        );

        let mut y = nav.y + header_height + NAV_TOP_GAP;
        for page in Page::ALL {
            let row = Rect {
                x: nav.x + NAV_INSET,
                y,
                width: nav.width - NAV_INSET * 2.0,
                height: NAV_ROW_HEIGHT,
                radius: 8.0,
                color: Color::TRANSPARENT,
            };
            let selected = page == self.page;
            if selected {
                list.push_rect(Rect { color: ctx.theme.bg_hover, ..row });
            }
            let color = if selected { ctx.theme.primary } else { ctx.theme.text_muted };
            ui::paint_text_vcentered(ctx, list, page.glyph(), row.x + 10.0, row, 15.0, 400, color);
            ui::paint_text_vcentered(ctx, list, page.title(), row.x + 36.0, row, 13.0, 400, color);
            y += NAV_ROW_SPACING;
        }

        // Subtle separator between rail and content.
        list.push_rect(Rect {
            x: panel.x + nav_width,
            y: panel.y,
            width: 1.0,
            height: panel.height,
            radius: 0.0,
            color: ctx.theme.border_inactive,
        });

        let content = Rect {
            x: panel.x + nav_width + 1.0,
            y: panel.y,
            width: panel.width - nav_width - 1.0,
            height: panel.height,
            radius: 0.0,
            color: Color::TRANSPARENT,
        };
        self.paint_page(ctx, list, content, input);
        panel
    }

    /// Paint the overlay. Returns the panel's rect so clicks can be routed.
    pub fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        surface: Rect,
        input: &InputState,
    ) -> Rect {
        // Scrim: dim the desktop and catch clicks outside the panel.
        let alpha = 0.45 * self.progress;
        list.push_rect(Rect {
            color: Color::rgba(0.0, 0.0, 0.0, alpha),
            ..surface
        });

        let panel = self.panel_rect(surface);

        ui::paint_frame(
            list,
            panel,
            ctx.theme.bg_main,
            ctx.theme.border_inactive,
            true,
        );

        // Navigation rail down the left of the panel.
        let nav_width = 132.0;
        let nav = Rect {
            x: panel.x,
            y: panel.y,
            width: nav_width,
            height: panel.height,
            radius: panel.radius,
            color: Color::TRANSPARENT,
        };

        let header_height = 44.0;
        ui::paint_text_vcentered(
            ctx,
            list,
            "Control Centre",
            nav.x + 16.0,
            Rect {
                height: header_height,
                ..nav
            },
            14.0,
            700,
            ctx.theme.text_main,
        );

        let mut y = nav.y + header_height + 8.0;
        for page in Page::ALL {
            let row = Rect {
                x: nav.x + 8.0,
                y,
                width: nav.width - 16.0,
                height: 34.0,
                radius: 8.0,
                color: Color::TRANSPARENT,
            };
            let selected = page == self.page;
            if selected {
                list.push_rect(Rect {
                    color: ctx.theme.bg_hover,
                    ..row
                });
            }
            let color = if selected {
                ctx.theme.primary
            } else {
                ctx.theme.text_muted
            };
            ui::paint_text_vcentered(
                ctx,
                list,
                page.glyph(),
                row.x + 10.0,
                row,
                15.0,
                400,
                color,
            );
            ui::paint_text_vcentered(
                ctx,
                list,
                page.title(),
                row.x + 34.0,
                row,
                13.0,
                400,
                color,
            );
            y += 36.0;
        }

        // Content area.
        let content = Rect {
            x: panel.x + nav_width + 8.0,
            y: panel.y,
            width: panel.width - nav_width - 8.0,
            height: panel.height,
            radius: 0.0,
            color: Color::TRANSPARENT,
        };
        self.paint_page(ctx, list, content, input);

        panel
    }

    fn paint_page(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        area: Rect,
        input: &InputState,
    ) {
        let padding = 20.0;
        let inner = Rect {
            x: area.x + padding,
            y: area.y + padding,
            width: area.width - padding * 2.0,
            height: area.height - padding * 2.0,
            radius: 0.0,
            color: Color::TRANSPARENT,
        };

        ui::paint_text_vcentered(
            ctx,
            list,
            self.page.title(),
            inner.x,
            Rect {
                height: 30.0,
                ..inner
            },
            18.0,
            700,
            ctx.theme.text_main,
        );

        let body = Rect {
            y: inner.y + 40.0,
            height: inner.height - 40.0,
            ..inner
        };

        match self.page {
            Page::Overview => self.paint_overview(ctx, list, body, input),
            Page::Audio => self.paint_audio(ctx, list, body, input),
            Page::Bluetooth => self.paint_bluetooth(ctx, list, body, input),
            Page::Wifi => self.paint_wifi(ctx, list, body, input),
            Page::Battery => self.paint_battery(ctx, list, body, input),
            Page::Display => self.paint_display(ctx, list, body),
            Page::Power => self.paint_power(ctx, list, body, input),
        }
    }

    fn paint_overview(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        area: Rect,
        _input: &InputState,
    ) {
        let tiles = [
            ("Audio", ctx.state.audio.get()),
            ("Bluetooth", ctx.state.bluetooth.get()),
            ("Network", ctx.state.net.get()),
            ("Battery", ctx.state.battery.get()),
        ];
        let columns = 2;
        let gap = 12.0;
        let tile_w = (area.width - gap) / columns as f32;
        let tile_h = 76.0;

        for (index, (label, snapshot)) in tiles.iter().enumerate() {
            let col = index % columns;
            let row = index / columns;
            let rect = Rect {
                x: area.x + col as f32 * (tile_w + gap),
                y: area.y + row as f32 * (tile_h + gap),
                width: tile_w,
                height: tile_h,
                radius: 10.0,
                color: Color::TRANSPARENT,
            };
            ui::paint_frame(
                list,
                rect,
                ctx.theme.bg_widget,
                ctx.theme.border_inactive,
                ctx.appearance.framed_border,
            );
            ui::paint_text_vcentered(
                ctx,
                list,
                label,
                rect.x + 14.0,
                Rect { height: 28.0, ..rect },
                13.0,
                600,
                ctx.theme.text_muted,
            );
            let summary = summarize(label, snapshot.clone());
            ui::paint_text_vcentered(
                ctx,
                list,
                &summary,
                rect.x + 14.0,
                Rect { y: rect.y + 28.0, height: 30.0, ..rect },
                15.0,
                500,
                ctx.theme.text_main,
            );
        }
    }

    fn paint_audio(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        area: Rect,
        input: &InputState,
    ) {
        let audio = ctx.state.audio.get();
        let muted = audio["mute"].as_str() == Some("yes");
        let volume_text = audio["volume"].as_str().unwrap_or("0%").to_string();
        let volume = volume_text
            .trim_end_matches('%')
            .parse::<f32>()
            .unwrap_or(0.0)
            / 100.0;

        // Volume slider. A drag reports a new value; release commits nothing
        // further, so the last drag frame is the applied value.
        let slider_rect = Rect {
            x: area.x,
            y: area.y + 4.0,
            width: area.width,
            height: 24.0,
            radius: 0.0,
            color: Color::TRANSPARENT,
        };
        let pointer_inside = input
            .pointer
            .map(|(x, y)| slider_rect.contains(crate::render::display_list::Point::new(x, y)))
            .unwrap_or(false);
        if input.pressed && pointer_inside {
            self.dragging = Some(SliderId::Volume);
        } else if !input.pressed {
            self.dragging = None;
        }
        let dragging = self.dragging == Some(SliderId::Volume);
        if let Some(value) = crate::ui::controls::slider(
            ctx, list, slider_rect, volume, dragging, input,
        ) {
            // Commit in 1% steps to avoid flooding the action worker.
            let percent = (value * 100.0).round() as i64;
            let current = (volume * 100.0).round() as i64;
            if percent != current {
                self.pending.push(crate::actions::SystemAction::SetVolume(percent));
            }
        }
        ui::paint_text_vcentered(
            ctx,
            list,
            &volume_text,
            area.x + area.width - 52.0,
            Rect { y: area.y + 30.0, height: 18.0, ..area },
            12.0,
            500,
            ctx.theme.text_muted,
        );

        // Mute switch.
        let mute_rect = Rect {
            x: area.x,
            y: area.y + 56.0,
            width: area.width,
            height: 30.0,
            radius: 0.0,
            color: Color::TRANSPARENT,
        };
        if let Some(next) = crate::ui::controls::toggle(
            ctx,
            list,
            mute_rect,
            "Muted",
            muted,
            input,
            self.clicked_this_frame,
        ) {
            self.pending
                .push(crate::actions::SystemAction::ToggleMute { muted: next == false && !muted });
        }

        // Output devices: click to switch.
        let mut y = area.y + 100.0;
        ui::paint_text_vcentered(
            ctx,
            list,
            "Outputs",
            area.x,
            Rect { y, height: 20.0, ..area },
            12.0,
            600,
            ctx.theme.text_muted,
        );
        y += 24.0;

        let current_output = audio["default_sink"]
            .as_str()
            .or_else(|| audio["default"]["sink"].as_str())
            .unwrap_or("");
        if let Some(sinks) = audio["sinks"].as_array() {
            for sink in sinks.iter().take(6) {
                let name = sink["name"].as_str().unwrap_or("");
                let label = sink["description"].as_str().unwrap_or(name);
                let row = Rect {
                    x: area.x,
                    y,
                    width: area.width,
                    height: 40.0,
                    radius: 8.0,
                    color: Color::TRANSPARENT,
                };
                let selected = name == current_output;
                if crate::ui::controls::selectable_row(
                    ctx,
                    list,
                    row,
                    label,
                    name,
                    selected,
                    input,
                    self.clicked_this_frame,
                ) && !selected
                {
                    self.pending.push(crate::actions::SystemAction::SetOutput(name.to_string()));
                }
                y += 44.0;
            }
        }
    }

    fn paint_bluetooth(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        area: Rect,
        input: &InputState,
    ) {
        let bt = ctx.state.bluetooth.get();
        let state = bt["state"].as_str().unwrap_or("off");
        let powered = state != "off";

        let toggle_rect = Rect {
            x: area.x,
            y: area.y + 2.0,
            width: area.width,
            height: 30.0,
            radius: 0.0,
            color: Color::TRANSPARENT,
        };
        if let Some(next) = crate::ui::controls::toggle(
            ctx,
            list,
            toggle_rect,
            "Adapter",
            powered,
            input,
            self.clicked_this_frame,
        ) {
            self.pending
                .push(crate::actions::SystemAction::BluetoothPower(next));
        }

        // Scan button.
        let scan_rect = Rect {
            x: area.x,
            y: area.y + 44.0,
            width: 140.0,
            height: 32.0,
            radius: 8.0,
            color: Color::TRANSPARENT,
        };
        if crate::ui::controls::button(
            ctx,
            list,
            scan_rect,
            "Scan",
            input,
            self.clicked_this_frame,
            true,
        ) {
            self.pending.push(crate::actions::SystemAction::BluetoothScan);
        }

        let mut y = area.y + 88.0;
        let devices = bt["devices"].as_array().cloned().unwrap_or_default();
        if devices.is_empty() {
            ui::paint_text_vcentered(
                ctx,
                list,
                "No devices",
                area.x,
                Rect { y, height: 24.0, ..area },
                13.0,
                400,
                ctx.theme.text_muted,
            );
            return;
        }
        for device in devices.iter().take(8) {
            let address = device["address"].as_str().unwrap_or("").to_string();
            let name = device["name"]
                .as_str()
                .or_else(|| device["alias"].as_str())
                .unwrap_or(&address);
            let connected = device["connected"].as_bool().unwrap_or(false);
            let subtitle = if connected { "connected" } else { "click to connect" };
            let row = Rect {
                x: area.x,
                y,
                width: area.width,
                height: 44.0,
                radius: 8.0,
                color: Color::TRANSPARENT,
            };
            if crate::ui::controls::selectable_row(
                ctx,
                list,
                row,
                name,
                subtitle,
                connected,
                input,
                self.clicked_this_frame,
            ) && !address.is_empty()
            {
                self.pending.push(crate::actions::SystemAction::BluetoothConnect {
                    address,
                    connect: !connected,
                });
            }
            y += 48.0;
        }
    }

    fn paint_wifi(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        area: Rect,
        input: &InputState,
    ) {
        let net = ctx.state.net.get();
        let enabled = net["enabled"].as_bool().unwrap_or(true);

        let toggle_rect = Rect {
            x: area.x,
            y: area.y + 2.0,
            width: area.width,
            height: 30.0,
            radius: 0.0,
            color: Color::TRANSPARENT,
        };
        if let Some(next) = crate::ui::controls::toggle(
            ctx,
            list,
            toggle_rect,
            "Wi-Fi",
            enabled,
            input,
            self.clicked_this_frame,
        ) {
            self.pending
                .push(crate::actions::SystemAction::WifiToggle(next));
        }

        let mut y = area.y + 48.0;
        let connections = net["connections"].as_array().cloned().unwrap_or_default();
        if connections.is_empty() {
            ui::paint_text_vcentered(
                ctx,
                list,
                "No active connections",
                area.x,
                Rect { y, height: 24.0, ..area },
                13.0,
                400,
                ctx.theme.text_muted,
            );
            return;
        }
        for conn in connections.iter().take(8) {
            let name = conn["connection"].as_str().unwrap_or("network");
            let device = conn["device"].as_str().unwrap_or("").to_string();
            let address = conn["ip_address"].as_str().unwrap_or("");
            let row = Rect {
                x: area.x,
                y,
                width: area.width,
                height: 44.0,
                radius: 8.0,
                color: Color::TRANSPARENT,
            };
            if crate::ui::controls::selectable_row(
                ctx,
                list,
                row,
                name,
                address,
                true,
                input,
                self.clicked_this_frame,
            ) && !device.is_empty()
            {
                self.pending
                    .push(crate::actions::SystemAction::WifiDisconnect(device));
            }
            y += 48.0;
        }
    }

    fn paint_battery(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        area: Rect,
        input: &InputState,
    ) {
        let battery = ctx.state.battery.get();
        let pct = battery["battery"]["pct"]
            .as_u64()
            .map(|p| format!("{p}%"))
            .unwrap_or_else(|| "--".into());
        let state = battery["battery"]["state"].as_str().unwrap_or("unknown");
        let charging = battery["charging_info"].as_str().unwrap_or("");
        let profile = battery["profile"].as_str().unwrap_or("").to_string();

        self.stat_row(ctx, list, area, 0, "Charge", &pct);
        self.stat_row(ctx, list, area, 1, "State", state);
        self.stat_row(ctx, list, area, 2, "Power supply", charging);

        // Power profile choices.
        let mut y = area.y + 132.0;
        ui::paint_text_vcentered(
            ctx,
            list,
            "Power profile",
            area.x,
            Rect { y, height: 20.0, ..area },
            12.0,
            600,
            ctx.theme.text_muted,
        );
        y += 26.0;
        for option in ["low-power", "balanced", "balanced-performance"] {
            let row = Rect {
                x: area.x,
                y,
                width: area.width.min(320.0),
                height: 34.0,
                radius: 8.0,
                color: Color::TRANSPARENT,
            };
            let selected = profile == option;
            if crate::ui::controls::selectable_row(
                ctx,
                list,
                row,
                option,
                "",
                selected,
                input,
                self.clicked_this_frame,
            ) && !selected
            {
                self.pending.push(crate::actions::SystemAction::SetPowerProfile(
                    option.to_string(),
                ));
            }
            y += 38.0;
        }
    }

    fn paint_display(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, area: Rect) {
        self.stat_row(ctx, list, area, 0, "Theme", &self.config.theme);
        self.stat_row(
            ctx,
            list,
            area,
            1,
            "Font size",
            &self.config.appearance.font_size.to_string(),
        );
        self.stat_row(ctx, list, area, 2, "Output", ctx.output_name);
    }

    fn paint_power(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        area: Rect,
        input: &InputState,
    ) {
        let actions: [(&str, &[&str]); 4] = [
            ("Lock", &["loginctl", "lock-session"]),
            ("Suspend", &["systemctl", "suspend"]),
            ("Reboot", &["systemctl", "reboot"]),
            ("Shutdown", &["systemctl", "poweroff"]),
        ];
        for (index, (label, argv)) in actions.iter().enumerate() {
            let rect = Rect {
                x: area.x,
                y: area.y + index as f32 * 48.0,
                width: area.width.min(280.0),
                height: 38.0,
                radius: 8.0,
                color: Color::TRANSPARENT,
            };
            let clicked = crate::ui::controls::button(
                ctx,
                list,
                rect,
                label,
                input,
                self.clicked_this_frame,
                *label == "Shutdown",
            );
            if clicked {
                self.pending.push(crate::actions::SystemAction::Spawn(
                    argv.iter().map(|s| s.to_string()).collect(),
                ));
            }
        }
    }

    /// One label/value row.
    fn stat_row(
        &self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        area: Rect,
        index: usize,
        label: &str,
        value: &str,
    ) {
        let rect = Rect {
            x: area.x,
            y: area.y + index as f32 * 38.0,
            width: area.width,
            height: 32.0,
            radius: 0.0,
            color: Color::TRANSPARENT,
        };
        ui::paint_text_vcentered(
            ctx,
            list,
            label,
            rect.x,
            rect,
            13.0,
            400,
            ctx.theme.text_muted,
        );
        ui::paint_text_vcentered(
            ctx,
            list,
            value,
            rect.x + rect.width * 0.45,
            rect,
            13.0,
            500,
            ctx.theme.text_main,
        );
    }

    /// Route a click in window coordinates.
    pub fn on_click(
        &mut self,
        point: crate::render::display_list::Point,
        surface: Rect,
        _input: &InputState,
    ) -> Option<Action> {
        if !self.open {
            return None;
        }

        // Navigation rail: the same geometry `paint_window` draws.
        let mut y = surface.y + NAV_HEADER + NAV_TOP_GAP;
        if point.x >= surface.x + NAV_INSET
            && point.x <= surface.x + NAV_WIDTH - NAV_INSET
        {
            for page in Page::ALL {
                if point.y >= y && point.y <= y + NAV_ROW_HEIGHT {
                    self.page = page;
                    return None;
                }
                y += NAV_ROW_SPACING;
            }
        }
        None
    }
}

/// A short human-readable summary of a subsystem snapshot.
fn summarize(label: &str, snapshot: serde_json::Value) -> String {
    match label {
        "Audio" => {
            let volume = snapshot["volume"].as_str().unwrap_or("--");
            let muted = snapshot["mute"].as_str() == Some("yes");
            if muted {
                format!("muted ({volume})")
            } else {
                volume.to_string()
            }
        }
        "Bluetooth" => snapshot["state"].as_str().unwrap_or("unknown").to_string(),
        "Network" => snapshot["connections"]
            .as_array()
            .and_then(|list| list.first())
            .and_then(|conn| conn["connection"].as_str())
            .unwrap_or("disconnected")
            .to_string(),
        "Battery" => snapshot["battery"]["pct"]
            .as_u64()
            .map(|p| format!("{p}%"))
            .unwrap_or_else(|| "--".into()),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_opens_the_requested_page() {
        let config = Arc::new(Config::default());
        let mut center = ControlCenter::new(config);
        assert!(!center.open);

        center.toggle(Page::Battery);
        assert!(center.open);
        assert_eq!(center.page, Page::Battery);

        // Toggling the same page closes it.
        center.toggle(Page::Battery);
        assert!(!center.open);
    }

    #[test]
    fn switching_page_while_open_stays_open() {
        let config = Arc::new(Config::default());
        let mut center = ControlCenter::new(config);
        center.toggle(Page::Audio);
        center.toggle(Page::Wifi);
        assert!(center.open);
        assert_eq!(center.page, Page::Wifi);
    }

    #[test]
    fn animation_reaches_its_target() {
        let config = Arc::new(Config::default());
        let mut center = ControlCenter::new(config);
        center.open = true;
        for _ in 0..100 {
            if !center.tick(0.016) {
                break;
            }
        }
        assert!((center.progress - 1.0).abs() < 1e-3);
        center.open = false;
        for _ in 0..100 {
            if !center.tick(0.016) {
                break;
            }
        }
        assert!(center.progress.abs() < 1e-3);
        assert!(!center.visible());
    }
}
