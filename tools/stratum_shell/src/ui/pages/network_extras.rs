//! The Network page's sections beyond Wi-Fi: traffic (download and upload
//! over the last minute), wired devices with their addresses, VPN profiles
//! with a switch each, and the Wi-Fi hotspot. Their data is
//! `SystemState::connections` and `net_rates`, both sampled only while the
//! page is shown (`net_rates.rs`).

use serde_json::Value;

use crate::actions::SystemAction;
use crate::net_rates::{format_rate, CAPACITY};
use crate::render::display_list::{DisplayList, Rect};
use crate::ui::graph::{self, GraphStyle};
use crate::ui::pages::devices::{self, Status};
use crate::ui::pages::layout::{self, rect, Card};
use crate::ui::pages::Optimistic;
use crate::ui::{self, controls, InputState, PaintCtx};

const ETHERNET: &str = "\u{f0200}";
const VPN: &str = "\u{f0582}";
const HOTSPOT: &str = "\u{f0003}";
/// The graph's floor: an idle link still draws a flat line, not noise.
const MIN_SCALE: f32 = 64.0 * 1024.0;

#[derive(Clone, Debug, PartialEq)]
enum Job {
    Vpn { name: String, on: bool },
    Hotspot(bool),
    Password,
}

#[derive(Default)]
pub struct NetworkExtras {
    job: Option<(u64, Job)>,
    vpn: std::collections::HashMap<String, Optimistic<bool>>,
    hotspot: Optimistic<bool>,
    /// The hotspot's password is shown.
    reveal: bool,
    status: Status,
}

/// A wired device's speed: `1 Gb/s`, `100 Mb/s`.
pub fn speed_label(mbps: u64) -> Option<String> {
    match mbps {
        0 => None,
        m if m >= 1000 && m % 1000 == 0 => Some(format!("{} Gb/s", m / 1000)),
        m if m >= 1000 => Some(format!("{:.1} Gb/s", m as f64 / 1000.0)),
        m => Some(format!("{m} Mb/s")),
    }
}

/// A wired device's address line: IP, gateway, DNS.
pub fn wired_detail(device: &Value) -> String {
    let get = |k: &str| device[k].as_str().filter(|s| !s.is_empty());
    let mut parts = Vec::new();
    if let Some(ip) = get("ip") {
        parts.push(format!("IP {ip}"));
    }
    if let Some(gateway) = get("gateway") {
        parts.push(format!("Gateway {gateway}"));
    }
    let dns: Vec<&str> = device["dns"].as_array().map(|d| d.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    if !dns.is_empty() {
        parts.push(format!("DNS {}", dns.join(", ")));
    }
    if let Some(mac) = get("mac") {
        parts.push(mac.to_string());
    }
    parts.join(" \u{b7} ")
}

impl NetworkExtras {
    pub fn tick(&mut self, dt: f32) {
        self.hotspot.tick(dt);
        for v in self.vpn.values_mut() {
            v.tick(dt);
        }
        self.status.tick(dt);
    }

    /// Leaving the page hides the password again.
    pub fn on_hidden(&mut self) {
        self.reveal = false;
    }

    fn start(&mut self, job: Job, action: SystemAction, pending: &mut Vec<SystemAction>) {
        let message = match &job {
            Job::Vpn { name, on: true } => format!("Connecting {name}..."),
            Job::Vpn { name, on: false } => format!("Disconnecting {name}..."),
            Job::Hotspot(true) => "Starting the hotspot...".to_string(),
            Job::Hotspot(false) => "Stopping the hotspot...".to_string(),
            Job::Password => "Reading the password...".to_string(),
        };
        self.status.progress(message);
        let ticket = crate::actions::next_ticket();
        self.job = Some((ticket, job));
        pending.push(SystemAction::Tracked { ticket, action: Box::new(action) });
    }

    fn collect(&mut self, ctx: &PaintCtx<'_>) {
        let result = ctx.state.action_result.get();
        let Some((ticket, job)) = self.job.clone() else { return };
        if ticket != result.ticket {
            return;
        }
        self.job = None;
        let status_ms = ctx.metrics.device_page.status_ms;
        match (result.error, job) {
            (Some(error), Job::Password) => {
                self.reveal = false;
                self.status.result(error, true, status_ms);
            }
            (Some(error), _) => self.status.result(error, true, status_ms),
            (None, Job::Vpn { name, on }) => {
                let what = if on { "Connected" } else { "Disconnected" };
                self.status.result(format!("{what} {name}."), false, status_ms);
            }
            (None, Job::Hotspot(on)) => {
                let what = if on { "Hotspot on." } else { "Hotspot off." };
                self.status.result(what.into(), false, status_ms);
            }
            (None, Job::Password) => self.status.clear(),
        }
    }

    /// Paint the sections at `y` in `body`; returns the bottom.
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
    ) -> f32 {
        // Keep the sampler (and the snapshot it reads) going while shown.
        if !ctx.state.net_rates_watch.touch() {
            pending.push(SystemAction::WatchNetRates);
        }
        self.collect(ctx);
        let snapshot = ctx.state.connections.get();
        let page = ctx.metrics.page.clone();
        let busy = self.job.is_some();

        let mut y = self.paint_traffic(ctx, list, body, y);

        let wired: Vec<Value> = snapshot["wired"].as_array().cloned().unwrap_or_default();
        if !wired.is_empty() {
            y += page.section_spacing;
            y = layout::section(ctx, list, body, y, "Wired");
            for device in &wired {
                y = paint_wired(ctx, list, body, y, device) + page.grid_gap;
            }
            y -= page.grid_gap;
        }

        y += page.section_spacing;
        y = layout::section(ctx, list, body, y, "VPN");
        let vpns: Vec<Value> = snapshot["vpns"].as_array().cloned().unwrap_or_default();
        if vpns.is_empty() {
            let caption = if snapshot.is_null() { "Reading NetworkManager…" } else { "Import one with nmcli connection import, or a VPN plugin" };
            y = devices::empty_card(ctx, list, body, y, "No VPN connections", caption);
        } else {
            for vpn in &vpns {
                y = self.paint_vpn(ctx, list, body, y, vpn, busy, input, clicked, pending) + page.grid_gap;
            }
            y -= page.grid_gap;
        }

        y += page.section_spacing;
        y = layout::section(ctx, list, body, y, "Hotspot");
        y = self.paint_hotspot(ctx, list, body, y, &snapshot["hotspot"], busy, input, clicked, pending);

        let footer = ctx.metrics.device_page.footer_height;
        self.status.paint(ctx, list, rect(body.x, y + page.label_gap, body.width, footer));
        y + page.label_gap + footer
    }

    fn paint_traffic(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, body: Rect, y: f32) -> f32 {
        let page = ctx.metrics.page.clone();
        let dm = ctx.metrics.device_page.clone();
        let typo = ctx.metrics.typography.clone();
        let theme = ctx.theme.clone();
        let y = layout::section(ctx, list, body, y, "Traffic");
        let pad = page.card_padding;
        let line_h = typo.line(typo.body);
        let card = rect(body.x, y, body.width, pad * 2.0 + line_h + page.label_gap + dm.traffic_height);
        layout::card(ctx, list, card, Card::Plain);
        let rates = ctx.state.net_rates.get();
        let now = |v: &std::collections::VecDeque<f32>| v.back().copied().unwrap_or(0.0);
        let row = rect(card.x + pad, card.y + pad, card.width - pad * 2.0, line_h);
        let down = format!("\u{2193} {}", format_rate(now(&rates.down)));
        let up = format!("\u{2191} {}", format_rate(now(&rates.up)));
        ui::paint_text_vcentered(ctx, list, &down, row.x, row, typo.body, typo.weight_bold, theme.primary);
        let down_w = ctx.measure(&down, typo.body);
        ui::paint_text_vcentered(ctx, list, &up, row.x + down_w + pad * 1.5, row, typo.body, typo.weight_bold, theme.tertiary);
        let note = "Last minute";
        let note_w = ctx.measure(note, typo.caption);
        ui::paint_text_vcentered(ctx, list, note, row.x + row.width - note_w, row, typo.caption, typo.weight_regular, theme.text_muted);

        let area = rect(row.x, row.y + line_h + page.label_gap, row.width, dm.traffic_height);
        let peak = rates.down.iter().chain(&rates.up).copied().fold(0.0, f32::max);
        let style = GraphStyle { capacity: CAPACITY, max: (peak * 1.2).max(MIN_SCALE), line: dm.traffic_line, fill: dm.traffic_fill };
        graph::paint(list, area, &rates.up, theme.tertiary, &style);
        graph::paint(list, area, &rates.down, theme.primary, &style);
        card.y + card.height
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_vpn(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        body: Rect,
        y: f32,
        vpn: &Value,
        busy: bool,
        input: &InputState,
        clicked: bool,
        pending: &mut Vec<SystemAction>,
    ) -> f32 {
        let m = ctx.metrics.device_page.clone();
        let page = ctx.metrics.page.clone();
        let uuid = vpn["uuid"].as_str().unwrap_or("").to_string();
        let name = vpn["name"].as_str().unwrap_or("VPN").to_string();
        let real = vpn["active"].as_bool().unwrap_or(false);
        let on = self.vpn.entry(uuid.clone()).or_default().view(real);
        let connecting = vpn["connecting"].as_bool().unwrap_or(false)
            || matches!(&self.job, Some((_, Job::Vpn { name: n, .. })) if *n == name);
        let row = rect(body.x, y, body.width, m.row_height);
        let hot = !busy && layout::hovered(input, row);
        layout::card(ctx, list, row, if real { Card::Active } else if hot { Card::Hover } else { Card::Plain });
        let kind = vpn["kind"].as_str().unwrap_or("VPN");
        let state = match (connecting, real) {
            (true, _) => "Working…",
            (false, true) => "Connected",
            (false, false) => "Off",
        };
        let switch_x = row.x + row.width - page.card_padding - page.toggle_width;
        let content = rect(row.x, row.y, switch_x - row.x, row.height);
        layout::row_content(ctx, list, content, VPN, &name, &format!("{kind} \u{b7} {state}"), None, real);
        let switch_y = row.y + (row.height - page.toggle_height) * 0.5;
        controls::switch(ctx, list, switch_x, switch_y, on, input, false);
        if hot && clicked {
            if let Some(v) = self.vpn.get_mut(&uuid) {
                v.set(!on);
            }
            self.start(Job::Vpn { name, on: !on }, SystemAction::VpnSet { uuid, on: !on }, pending);
        }
        row.y + row.height
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_hotspot(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        body: Rect,
        y: f32,
        hotspot: &Value,
        busy: bool,
        input: &InputState,
        clicked: bool,
        pending: &mut Vec<SystemAction>,
    ) -> f32 {
        let m = ctx.metrics.device_page.clone();
        let page = ctx.metrics.page.clone();
        let supported = hotspot["supported"].as_bool().unwrap_or(false);
        let real = hotspot["active"].as_bool().unwrap_or(false);
        let on = self.hotspot.view(real);
        let ssid = hotspot["ssid"].as_str().unwrap_or("");
        let caption = match (supported, on) {
            (false, _) if hotspot.is_null() => "Reading NetworkManager…".to_string(),
            (false, _) => "This Wi-Fi card cannot be a hotspot".to_string(),
            (true, true) if !ssid.is_empty() => format!("On \u{b7} {ssid}"),
            (true, true) => "On".to_string(),
            (true, false) => "Shares this connection over Wi-Fi; turning it on disconnects Wi-Fi".to_string(),
        };
        let height = if real { m.row_height + m.expanded } else { m.row_height };
        let card = rect(body.x, y, body.width, height);
        let top = rect(card.x, card.y, card.width, m.row_height);
        let enabled = supported && !busy;
        let hot = enabled && layout::hovered(input, top);
        layout::card(ctx, list, card, if real { Card::Active } else if hot { Card::Hover } else { Card::Plain });
        let switch_x = card.x + card.width - page.card_padding - page.toggle_width;
        let content = rect(top.x, top.y, switch_x - top.x, top.height);
        layout::row_content(ctx, list, content, HOTSPOT, "Hotspot", &caption, None, on);
        controls::switch(ctx, list, switch_x, top.y + (top.height - page.toggle_height) * 0.5, on, input, false);
        if hot && clicked {
            self.hotspot.set(!on);
            self.reveal = false;
            pending.push(SystemAction::HotspotPassword(None));
            self.start(Job::Hotspot(!on), SystemAction::Hotspot(!on), pending);
        }
        let password = ctx.state.hotspot_password.get();
        // A password read to be shown is not kept once hidden.
        if password.is_some() && !self.reveal && !busy {
            pending.push(SystemAction::HotspotPassword(None));
        }
        if real {
            let line = devices::expansion(ctx, card, m.row_height);
            let shown = match (&password, self.reveal) {
                (Some(p), true) => format!("Password {p}"),
                _ => "Password \u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}".to_string(),
            };
            let label = if self.reveal { "Hide" } else { "Show" };
            if devices::row_actions(ctx, list, line, &shown, &[label], !busy, input, clicked).is_some() {
                self.reveal = !self.reveal;
                match (self.reveal, hotspot["uuid"].as_str()) {
                    (true, Some(uuid)) => self.start(Job::Password, SystemAction::HotspotPassword(Some(uuid.to_string())), pending),
                    _ => pending.push(SystemAction::HotspotPassword(None)),
                }
            }
        }
        card.y + card.height
    }
}

fn paint_wired(ctx: &mut PaintCtx<'_>, list: &mut DisplayList, body: Rect, y: f32, device: &Value) -> f32 {
    let m = ctx.metrics.device_page.clone();
    let typo = ctx.metrics.typography.clone();
    let connected = device["connected"].as_bool().unwrap_or(false);
    let height = if connected { m.row_height + m.expanded } else { m.row_height };
    let row = rect(body.x, y, body.width, height);
    layout::card(ctx, list, row, if connected { Card::Active } else { Card::Plain });
    let interface = device["device"].as_str().unwrap_or("Ethernet");
    let title = device["connection"].as_str().filter(|c| !c.is_empty()).unwrap_or(interface);
    let state = device["state"].as_str().unwrap_or("");
    let mut caption = vec![state.to_string()];
    if let Some(speed) = speed_label(device["speed"].as_u64().unwrap_or(0)).filter(|_| connected) {
        caption.push(speed);
    }
    caption.push(interface.to_string());
    let top = rect(row.x, row.y, row.width, m.row_height);
    layout::row_content(ctx, list, top, ETHERNET, title, &caption.join(" \u{b7} "), None, connected);
    if connected {
        let line = devices::expansion(ctx, row, m.row_height);
        let detail = ui::elide(ctx, &wired_detail(device), typo.caption, typo.weight_regular, line.width);
        ui::paint_text_vcentered(ctx, list, &detail, line.x, line, typo.caption, typo.weight_regular, ctx.theme.text_muted);
    }
    row.y + row.height
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speeds_and_details_read_naturally() {
        assert_eq!(speed_label(1000).as_deref(), Some("1 Gb/s"));
        assert_eq!(speed_label(2500).as_deref(), Some("2.5 Gb/s"));
        assert_eq!(speed_label(100).as_deref(), Some("100 Mb/s"));
        assert_eq!(speed_label(0), None);
        let device = serde_json::json!({ "ip": "10.0.0.2/24", "gateway": "10.0.0.1", "dns": ["1.1.1.1", "9.9.9.9"], "mac": "AA:BB" });
        assert_eq!(wired_detail(&device), "IP 10.0.0.2/24 \u{b7} Gateway 10.0.0.1 \u{b7} DNS 1.1.1.1, 9.9.9.9 \u{b7} AA:BB");
    }
}
