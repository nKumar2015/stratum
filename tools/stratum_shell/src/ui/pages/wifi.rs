//! Network page: radio, active connections, and available networks.
//!
//! Opening the page refreshes the access-point list immediately rather than
//! waiting for the poller, and the radio toggle is optimistic so the switch
//! follows the click.

use crate::actions::SystemAction;
use crate::render::display_list::{Color, DisplayList, Point, Rect};
use crate::ui::controls;
use crate::ui::pages::{heading, section_label, Optimistic};
use crate::ui::{self, InputState, PaintCtx};

pub struct WifiPage {
    enabled: Optimistic<bool>,
    /// SSID of the network being joined, shown while NetworkManager works.
    connecting: Option<String>,
    refresh_requested: bool,
    /// Offset into the network list, in rows.
    scroll: usize,
}

impl WifiPage {
    pub fn new() -> Self {
        Self {
            enabled: Optimistic::new(),
            connecting: None,
            refresh_requested: false,
            scroll: 0,
        }
    }

    pub fn tick(&mut self, dt: f32) {
        self.enabled.tick(dt);
    }

    pub fn request_refresh(&mut self, pending: &mut Vec<SystemAction>) {
        if self.refresh_requested {
            return;
        }
        self.refresh_requested = true;
        // A rescan is what the QML menu did on open; `nmcli` returns quickly
        // when it already has a fresh cache.
        pending.push(SystemAction::WifiRefresh { rescan: true });
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
        let area = heading(ctx, list, area, "Network");
        if !self.refresh_requested {
            self.request_refresh(pending);
        }

        // The radio state lives in `wifi.state` ("enabled"/"disabled"); `net`
        // only lists active connections.
        let wifi = ctx.state.wifi.get();
        let real_enabled = wifi["state"].as_str() != Some("disabled");
        let enabled = self.enabled.view(real_enabled);

        let toggle_rect = Rect {
            x: area.x,
            y: area.y + 2.0,
            width: area.width,
            height: 30.0,
            radius: 0.0,
            color: Color::TRANSPARENT,
        };
        if let Some(next) = controls::toggle(
            ctx,
            list,
            toggle_rect,
            "Wi-Fi",
            enabled,
            input,
            clicked,
        ) {
            self.enabled.set(next);
            pending.push(SystemAction::WifiToggle(next));
        }

        let net = ctx.state.net.get();
        let mut y = area.y + 46.0;

        // Active connections.
        let connections = net["connections"].as_array().cloned().unwrap_or_default();
        if connections.is_empty() {
            ui::paint_text_vcentered(
                ctx,
                list,
                "Not connected",
                area.x,
                Rect { y, height: 24.0, ..area },
                13.0,
                400,
                ctx.theme.text_muted,
            );
            y += 32.0;
        } else {
            section_label(ctx, list, area, y, "Connected");
            y += 22.0;
            for conn in connections.iter().take(4) {
                let name = conn["connection"].as_str().unwrap_or("network");
                let device = conn["device"].as_str().unwrap_or("").to_string();
                let address = conn["ip_address"].as_str().unwrap_or("");
                let row = Rect {
                    x: area.x,
                    y,
                    width: area.width,
                    height: 40.0,
                    radius: 8.0,
                    color: Color::TRANSPARENT,
                };
                if controls::selectable_row(
                    ctx, list, row, name, address, true, input, clicked,
                ) && !device.is_empty()
                {
                    pending.push(SystemAction::WifiDisconnect(device));
                }
                y += 44.0;
            }
        }

        if !enabled {
            ui::paint_text_vcentered(
                ctx,
                list,
                "Wi-Fi is off",
                area.x,
                Rect {
                    y: y + 8.0,
                    height: 24.0,
                    ..area
                },
                13.0,
                400,
                ctx.theme.text_muted,
            );
            return;
        }

        let networks = ctx.state.wifi_networks.get();
        let list_items = networks["networks"].as_array().cloned().unwrap_or_default();

        section_label(ctx, list, area, y, "Available networks");
        y += 22.0;

        if list_items.is_empty() {
            ui::paint_text_vcentered(
                ctx,
                list,
                "Scanning…",
                area.x,
                Rect { y, height: 24.0, ..area },
                13.0,
                400,
                ctx.theme.text_muted,
            );
            return;
        }

        // Scroll the list with the wheel.
        let row_height = 40.0;
        let visible = (((area.y + area.height - y) / row_height).floor() as usize).max(1);
        if let Some((_, _)) = input.pointer {
            // Wheel handling happens in `on_scroll`; clamp here for safety.
            self.scroll = self.scroll.min(list_items.len().saturating_sub(visible));
        }

        let mut rows = 0usize;
        for network in list_items.iter().skip(self.scroll) {
            if rows >= visible {
                break;
            }
            let ssid = network["ssid"].as_str().unwrap_or("");
            if ssid.is_empty() {
                continue;
            }
            let in_use = network["in_use"].as_bool().unwrap_or(false);
            let signal = network["signal"].as_i64().unwrap_or(0);
            let security = network["security"].as_str().unwrap_or("");
            let label = if in_use {
                format!("{ssid}  (connected)")
            } else {
                ssid.to_string()
            };
            let subtitle = if self.connecting.as_deref() == Some(ssid) {
                "Connecting…".to_string()
            } else if security.is_empty() || security == "--" {
                format!("{signal}% · open")
            } else {
                format!("{signal}% · secured")
            };
            let row = Rect {
                x: area.x,
                y: y + rows as f32 * row_height,
                width: area.width,
                height: 36.0,
                radius: 8.0,
                color: Color::TRANSPARENT,
            };
            if controls::selectable_row(
                ctx, list, row, &label, &subtitle, in_use, input, clicked,
            ) && !in_use
            {
                pending.push(SystemAction::WifiConnect {
                    ssid: ssid.to_string(),
                    password: None,
                });
            }
            rows += 1;
        }

        // Scroll affordance.
        if list_items.len() > visible {
            let track = Rect {
                x: area.x + area.width - 4.0,
                y,
                width: 2.0,
                height: visible as f32 * row_height,
                radius: 1.0,
                color: ctx.theme.bg_widget,
            };
            list.push_rect(track);
            let thumb_height = (visible as f32 / list_items.len() as f32) * track.height;
            let max_scroll = (list_items.len() - visible) as f32;
            let offset = (self.scroll as f32 / max_scroll.max(1.0)) * (track.height - thumb_height);
            list.push_rect(Rect {
                y: track.y + offset,
                height: thumb_height,
                color: ctx.theme.text_muted,
                ..track
            });
        }
    }

    /// Wheel scrolling. Returns true when the page consumed the event.
    pub fn on_scroll(&mut self, _point: Point, delta: f32) -> bool {
        let step = if delta > 0.0 { -1 } else { 1 };
        let next = (self.scroll as i64 + step).max(0) as usize;
        if next == self.scroll {
            return false;
        }
        self.scroll = next;
        true
    }
}

impl Default for WifiPage {
    fn default() -> Self {
        Self::new()
    }
}
