//! Bluetooth page: adapter power, discovery, and devices.
//!
//! Every interaction records an optimistic value and asks the worker for a
//! refresh, so the page changes on the same frame the user clicks. `Scan` shows
//! a live indicator because discovery takes seconds and silence looks broken.

use crate::actions::SystemAction;
use crate::render::display_list::{Color, DisplayList, Rect};
use crate::ui::controls;
use crate::ui::pages::{heading, Optimistic};
use crate::ui::{self, InputState, PaintCtx};

pub struct BluetoothPage {
    powered: Optimistic<bool>,
    /// Seconds the scan indicator has been visible.
    scanning_for: Option<f32>,
    /// Set when the page is first shown, so the device list is fresh.
    refresh_requested: bool,
}

impl BluetoothPage {
    pub fn new() -> Self {
        Self {
            powered: Optimistic::new(),
            scanning_for: None,
            refresh_requested: false,
        }
    }

    pub fn tick(&mut self, dt: f32) {
        self.powered.tick(dt);
        if let Some(elapsed) = &mut self.scanning_for {
            *elapsed += dt;
            // A scan should not leave a spinner forever if discovery fails.
            if *elapsed > 20.0 {
                self.scanning_for = None;
            }
        }
    }

    /// The scan indicator animates, so the window needs a frame clock.
    pub fn animating(&self) -> bool {
        self.scanning_for.is_some()
    }

    /// Ask for a device refresh when the page becomes visible.
    pub fn request_refresh(&mut self, pending: &mut Vec<SystemAction>) {
        if self.refresh_requested {
            return;
        }
        self.refresh_requested = true;
        pending.push(SystemAction::BluetoothRefresh);
    }

    pub fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        area: Rect,
        input: &InputState,
        clicked: bool,
        pending: &mut Vec<SystemAction>,
    ) {
        let area = heading(ctx, list, area, "Bluetooth");
        if !self.refresh_requested {
            // The page was opened without going through `on_page_shown`.
            self.request_refresh(pending);
        }

        let bluetooth = ctx.state.bluetooth.get();
        let state = bluetooth["state"].as_str().unwrap_or("off");
        let real_powered = state != "off";
        let powered = self.powered.view(real_powered);
        let scanning = bluetooth["scanning"].as_str() == Some("yes")
            || self.scanning_for.is_some();

        let toggle_rect = Rect {
            x: area.x,
            y: area.y + 2.0,
            width: area.width,
            height: ctx.metrics.page.button_height,
            radius: 0.0,
            color: Color::TRANSPARENT,
        };
        if let Some(next) = controls::toggle(
            ctx,
            list,
            toggle_rect,
            "Adapter",
            powered,
            input,
            clicked,
        ) {
            self.powered.set(next);
            pending.push(SystemAction::BluetoothPower(next));
        }

        let scan_rect = Rect {
            x: area.x,
            y: area.y + 44.0,
            width: ctx.metrics.page.button_width,
            height: 32.0,
            radius: ctx.metrics.page.button_radius,
            color: Color::TRANSPARENT,
        };
        if controls::button(
            ctx,
            list,
            scan_rect,
            if scanning { "Scanning…" } else { "Scan" },
            input,
            clicked,
            !scanning,
        ) && !scanning
            && powered
        {
            self.scanning_for = Some(0.0);
            pending.push(SystemAction::BluetoothScan);
        }

        let mut y = area.y + 88.0;
        let devices = bluetooth["devices"].as_array().cloned().unwrap_or_default();
        if devices.is_empty() {
            ui::paint_text_vcentered(
                ctx,
                list,
                if powered {
                    "No devices found"
                } else {
                    "Adapter is off"
                },
                area.x,
                Rect { y, height: 24.0, ..area },
                ctx.metrics.typography.body,
ctx.metrics.typography.weight_regular,
                ctx.theme.text_muted,
            );
            return;
        }

        for device in devices.iter().take(8) {
            let address = device["address"]
                .as_str()
                .or_else(|| device["mac"].as_str())
                .unwrap_or("")
                .to_string();
            let name = device["name"]
                .as_str()
                .or_else(|| device["alias"].as_str())
                .unwrap_or(&address);
            // The bluer snapshot uses strings ("yes"/"no"); the CLI variant uses
            // booleans. Accept both so the page works with either source.
            let connected = device["connected"].as_bool().unwrap_or(false)
                || device["connected"].as_str() == Some("yes");
            let subtitle = if connected {
                "Connected"
            } else {
                "Click to connect"
            };
            let row = Rect {
                x: area.x,
                y,
                width: area.width,
                height: ctx.metrics.page.row_height,
                radius: ctx.metrics.page.button_radius,
                color: Color::TRANSPARENT,
            };
            if controls::selectable_row(
                ctx, list, row, name, subtitle, connected, input, clicked,
            ) && !address.is_empty()
            {
                pending.push(SystemAction::BluetoothConnect {
                    address,
                    connect: !connected,
                });
            }
            y += ctx.metrics.page.row_spacing + 4.0;
        }

        // Discovery indicator: a subtle bar under the scan button.
        if let Some(elapsed) = self.scanning_for {
            let width = area.width * 0.4;
            let progress = ((elapsed % 1.2) / 1.2).clamp(0.0, 1.0);
            list.push_rect(Rect {
                x: area.x,
                y: area.y + 80.0,
                width,
                height: 2.0,
                radius: 1.0,
                color: ctx.theme.bg_widget,
            });
            list.push_rect(Rect {
                x: area.x + width * progress,
                y: area.y + 80.0,
                width: 24.0,
                height: 2.0,
                radius: 1.0,
                color: ctx.theme.primary,
            });
        }
    }
}

impl Default for BluetoothPage {
    fn default() -> Self {
        Self::new()
    }
}
