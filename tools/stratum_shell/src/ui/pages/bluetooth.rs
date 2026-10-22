//! Bluetooth page: matching `modules/BluetoothMenu.qml`.
//!
//! - The adapter switch (showing "Working…" while it changes). Turning it on
//!   starts a scan, as QML did.
//! - A card for the connected device: name, MAC, trusted, paired, and a
//!   disconnect button.
//! - Refresh and scan, the device count, and "Loading" while a refresh runs.
//! - Every device the adapter knows, discovered ones included, connected
//!   first: name, MAC, state and trust. A click selects a device, and a
//!   second click deselects it.
//! - The selected device's panel: its name (scrolling when it does not fit),
//!   MAC, state, and Pair/Remove, Connect/Disconnect, Trust/Untrust.
//! - A status line: "Connecting to …" while an action runs, then its result.
//!
//! One action runs at a time, and the buttons are disabled meanwhile, as in
//! QML. Actions are `SystemAction::Tracked`, so the page learns its own
//! outcome from `SystemState::action_result`.

use serde_json::Value;

use crate::actions::{BluetoothOp, SystemAction};
use crate::render::display_list::{DisplayList, Point, Rect};
use crate::ui::controls;
use crate::ui::pages::devices::{self, rect, Marquee, Status};
use crate::ui::pages::{heading, Optimistic};
use crate::ui::{self, InputState, PaintCtx};

/// What is running.
#[derive(Clone, Debug, PartialEq)]
enum Job {
    Power(bool),
    Scan,
    Device { op: BluetoothOp, name: String },
}

/// One device from the snapshot.
#[derive(Clone, Debug, PartialEq)]
struct Device {
    mac: String,
    name: String,
    connected: bool,
    paired: bool,
    trusted: bool,
}

fn devices(snapshot: &Value, key: &str) -> Vec<Device> {
    let yes = |d: &Value, k: &str| d[k].as_str() == Some("yes") || d[k].as_bool() == Some(true);
    snapshot[key]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|d| {
                    let mac = d["mac"].as_str()?.to_string();
                    let name = d["name"].as_str().filter(|n| !n.is_empty()).unwrap_or(&mac).to_string();
                    Some(Device {
                        name,
                        connected: yes(d, "connected"),
                        paired: yes(d, "paired"),
                        trusted: yes(d, "trusted"),
                        mac,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The status line while `op` runs on `name`, and once it has succeeded.
fn messages(op: BluetoothOp, name: &str) -> (String, String) {
    match op {
        BluetoothOp::Pair => (format!("Pairing with {name}..."), format!("Paired with {name}.")),
        BluetoothOp::Forget => (format!("Removing {name}..."), format!("Removed {name}.")),
        BluetoothOp::Connect => (format!("Connecting to {name}..."), format!("Connected to {name}.")),
        BluetoothOp::Disconnect => (format!("Disconnecting {name}..."), format!("Disconnected {name}.")),
        BluetoothOp::Trust => (format!("Trusting {name}..."), format!("Trusted {name}.")),
        BluetoothOp::Untrust => (format!("Removing trust for {name}..."), format!("Untrusted {name}.")),
    }
}

pub struct BluetoothPage {
    powered: Optimistic<bool>,
    selected: Option<String>,
    /// The running action and its ticket.
    job: Option<(u64, Job)>,
    /// The running refresh's ticket, and how long it has run (for the dot).
    refresh: Option<(u64, f32)>,
    /// A refresh to send on the next paint (the page was just shown).
    refresh_wanted: bool,
    status: Status,
    marquee: Marquee,
    scroll: usize,
}

impl BluetoothPage {
    pub fn new() -> Self {
        Self {
            powered: Optimistic::new(),
            selected: None,
            job: None,
            refresh: None,
            // Opened for the first time: fetch at once.
            refresh_wanted: true,
            status: Status::default(),
            marquee: Marquee::default(),
            scroll: 0,
        }
    }

    pub fn tick(&mut self, dt: f32) {
        self.powered.tick(dt);
        self.status.tick(dt);
        if let Some((_, elapsed)) = &mut self.refresh {
            *elapsed += dt;
        }
    }

    /// The loading dot and a scrolling name need frames.
    pub fn animating(&self) -> bool {
        self.refresh.is_some() || self.marquee.moving()
    }

    /// Ask for a device refresh when the page becomes visible. It is sent by
    /// the next paint: an action queued here can be cleared by the frame
    /// start, which left "Loading" up forever.
    pub fn request_refresh(&mut self, _pending: &mut Vec<SystemAction>) {
        self.refresh_wanted = true;
    }

    fn refresh(&mut self, pending: &mut Vec<SystemAction>) {
        let ticket = crate::actions::next_ticket();
        self.refresh = Some((ticket, 0.0));
        pending.push(SystemAction::Tracked { ticket, action: Box::new(SystemAction::BluetoothRefresh) });
    }

    fn start(&mut self, job: Job, action: SystemAction, status_ms: u64, pending: &mut Vec<SystemAction>) {
        let ticket = crate::actions::next_ticket();
        let message = match &job {
            Job::Power(on) => if *on { "Turning Bluetooth on..." } else { "Turning Bluetooth off..." }.to_string(),
            Job::Scan => "Scanning for devices...".to_string(),
            Job::Device { op, name } => messages(*op, name).0,
        };
        self.status.progress(message);
        let _ = status_ms;
        self.job = Some((ticket, job));
        pending.push(SystemAction::Tracked { ticket, action: Box::new(action) });
    }

    /// Pick up the outcome of our running action or refresh.
    fn collect(&mut self, ctx: &PaintCtx<'_>, pending: &mut Vec<SystemAction>) {
        let result = ctx.state.action_result.get();
        let status_ms = ctx.metrics.device_page.status_ms;
        if self.refresh.as_ref().is_some_and(|(t, _)| *t == result.ticket) {
            self.refresh = None;
        }
        let Some((ticket, job)) = self.job.clone() else {
            return;
        };
        if ticket != result.ticket {
            return;
        }
        self.job = None;
        match (result.error, job) {
            (Some(error), _) => self.status.result(error, true, status_ms),
            (None, Job::Power(true)) => {
                self.status.result("Bluetooth turned on. Starting scan...".into(), false, status_ms);
                self.start(Job::Scan, SystemAction::BluetoothScan, status_ms, pending);
            }
            (None, Job::Power(false)) => self.status.result("Bluetooth turned off.".into(), false, status_ms),
            (None, Job::Scan) => self.status.result("Scan completed.".into(), false, status_ms),
            (None, Job::Device { op, name }) => {
                if op == BluetoothOp::Forget {
                    self.selected = None;
                }
                self.status.result(messages(op, &name).1, false, status_ms);
            }
        }
    }

    /// Wheel scrolling over the list. Returns true when consumed.
    pub fn on_scroll(&mut self, _point: Point, delta: f32) -> bool {
        let step: i64 = if delta > 0.0 { -1 } else { 1 };
        let next = (self.scroll as i64 + step).max(0) as usize;
        if next == self.scroll {
            return false;
        }
        self.scroll = next;
        true
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
        let area = heading(ctx, list, area, "Bluetooth");
        if self.refresh_wanted {
            self.refresh_wanted = false;
            self.refresh(pending);
        }
        self.collect(ctx, pending);

        let m = ctx.metrics.device_page.clone();
        let page = ctx.metrics.page.clone();
        let theme = ctx.theme.clone();
        let snapshot = ctx.state.bluetooth.get();
        let real_powered = snapshot["state"].as_str().unwrap_or("off") != "off";
        let powered = self.powered.view(real_powered);
        let busy = self.job.is_some();
        let all = devices(&snapshot, "all_devices");
        let all = if all.is_empty() { devices(&snapshot, "devices") } else { all };
        let active = all.iter().find(|d| d.connected).cloned();

        // Adapter switch.
        let mut y = area.y;
        let working = matches!(self.job, Some((_, Job::Power(_))));
        let toggle_rect = rect(area.x, y, area.width, page.button_height);
        let label = if working { "Working…" } else { "Adapter" };
        if let Some(next) = controls::toggle(ctx, list, toggle_rect, label, powered, input, clicked && !busy) {
            self.powered.set(next);
            self.start(Job::Power(next), SystemAction::BluetoothPower(next), m.status_ms, pending);
        }
        y += page.button_height + m.spacing;

        // The connected device.
        let card_h = m.card_padding * 2.0 + m.icon_button.max(m.strip_line * 2.0);
        let card_rect = rect(area.x, y, area.width, card_h);
        devices::card(ctx, list, card_rect, theme.bg_widget, theme.border_inactive);
        let inner = rect(
            card_rect.x + m.card_padding,
            card_rect.y + m.card_padding,
            card_rect.width - m.card_padding * 3.0 - m.icon_button,
            card_h - m.card_padding * 2.0,
        );
        let headline = match &active {
            Some(d) => format!("Connected to {}", d.name),
            None => "No active Bluetooth device".to_string(),
        };
        let line = rect(inner.x, inner.y, inner.width, m.strip_line);
        list.push_clip(line);
        ui::paint_text_vcentered(ctx, list, &headline, inner.x, line, m.headline, ctx.metrics.typography.weight_bold, theme.text_main);
        list.pop_clip();
        let na = "N/A";
        let yes_no = |v: bool| if v { "yes" } else { "no" };
        let (mac, trusted, paired) = match &active {
            Some(d) => (d.mac.as_str(), yes_no(d.trusted), yes_no(d.paired)),
            None => (na, na, na),
        };
        let good = |v: &str| if v == "yes" { theme.success } else { theme.error };
        devices::strip(
            ctx,
            list,
            rect(inner.x, inner.y + m.strip_line, inner.width, m.strip_line),
            &[
                ("MAC", mac, if active.is_some() { theme.secondary } else { theme.tertiary }),
                ("Trusted", trusted, good(trusted)),
                ("Paired", paired, good(paired)),
            ],
            m.strip,
            theme.tertiary,
        );
        let disconnect = rect(
            card_rect.x + card_rect.width - m.card_padding - m.icon_button,
            card_rect.y + (card_h - m.icon_button) * 0.5,
            m.icon_button,
            m.icon_button,
        );
        if devices::icon_button(ctx, list, disconnect, "󰂲", theme.text_main, active.is_some() && !busy, input, clicked) {
            if let Some(d) = &active {
                let job = Job::Device { op: BluetoothOp::Disconnect, name: d.name.clone() };
                let action = SystemAction::BluetoothDevice { address: d.mac.clone(), op: BluetoothOp::Disconnect };
                self.start(job, action, m.status_ms, pending);
            }
        }
        y += card_h + m.spacing;

        // Toolbar.
        let bar = rect(area.x, y, area.width, m.icon_button);
        let refresh_rect = rect(bar.x, bar.y, m.icon_button, m.icon_button);
        if devices::icon_button(ctx, list, refresh_rect, "󰑐", theme.text_main, self.refresh.is_none(), input, clicked) {
            self.refresh(pending);
        }
        let scan_rect = rect(bar.x + m.icon_button + m.gap, bar.y, m.icon_button, m.icon_button);
        if devices::icon_button(ctx, list, scan_rect, "\u{f0349}", theme.text_main, powered && !busy, input, clicked) {
            self.start(Job::Scan, SystemAction::BluetoothScan, m.status_ms, pending);
        }
        let mut right = bar.x + bar.width;
        if let Some((_, phase)) = self.refresh {
            right -= devices::loading(ctx, list, bar, phase) + m.gap * 2.0;
        }
        let count = format!("{} devices", all.len());
        let count_w = ctx.measure(&count, m.strip);
        ui::paint_text_vcentered(ctx, list, &count, right - count_w, bar, m.strip, 400, theme.text_muted);
        y += m.icon_button + m.spacing;

        // The list, and the selected device's panel beside it.
        let footer_top = area.y + area.height - m.footer_height;
        let body = rect(area.x, y, area.width, footer_top - m.spacing - y);
        let selected = self.selected.as_ref().and_then(|mac| all.iter().find(|d| &d.mac == mac)).cloned();
        if self.selected.is_some() && selected.is_none() {
            self.selected = None;
        }
        let list_w = if selected.is_some() { body.width - m.panel_width - m.spacing } else { body.width };
        let list_rect = rect(body.x, body.y, list_w, body.height);
        if all.is_empty() {
            let note = if powered { "No devices found" } else { "Adapter is off" };
            let line = rect(list_rect.x, list_rect.y, list_rect.width, m.row_height);
            ui::paint_text_vcentered(ctx, list, note, line.x, line, m.row_title, 400, theme.text_muted);
        }
        let pitch = m.row_height + m.row_gap;
        let visible = ((list_rect.height + m.row_gap) / pitch).floor().max(1.0) as usize;
        self.scroll = self.scroll.min(all.len().saturating_sub(visible));
        list.push_clip(list_rect);
        for (i, device) in all.iter().skip(self.scroll).take(visible).enumerate() {
            let row = rect(list_rect.x, list_rect.y + i as f32 * pitch, list_rect.width, m.row_height);
            let is_selected = self.selected.as_deref() == Some(device.mac.as_str());
            let fill = if is_selected { theme.bg_hover } else { theme.bg_widget };
            let border = if device.connected { theme.border_active } else { theme.border_inactive };
            devices::card(ctx, list, row, fill, border);
            let text_w = row.width - m.row_padding * 2.0;
            let title = rect(row.x + m.row_padding, row.y + m.row_padding, text_w, (m.row_height - m.row_padding * 2.0) * 0.55);
            list.push_clip(title);
            ui::paint_text_vcentered(ctx, list, &device.name, title.x, title, m.row_title, ctx.metrics.typography.weight_bold, theme.text_main);
            list.pop_clip();
            let state_line = rect(title.x, title.y + title.height, text_w, row.y + row.height - m.row_padding - title.y - title.height);
            devices::strip(
                ctx,
                list,
                state_line,
                &[
                    ("MAC", &device.mac, if is_selected { theme.secondary } else { theme.tertiary }),
                    (
                        "State",
                        if device.connected { "Connected" } else { "Disconnected" },
                        if device.connected { theme.success } else { theme.error },
                    ),
                    (
                        "Trust",
                        if device.trusted { "Trusted" } else { "Untrusted" },
                        if device.trusted { theme.success } else { theme.error },
                    ),
                ],
                m.row_strip,
                theme.tertiary,
            );
            let hit = input.pointer.is_some_and(|(x, y)| row.contains(Point::new(x, y)) && list_rect.contains(Point::new(x, y)));
            if hit && clicked {
                self.selected = if is_selected { None } else { Some(device.mac.clone()) };
            }
        }
        list.pop_clip();

        if let Some(device) = selected {
            let panel = rect(body.x + list_w + m.spacing, body.y, m.panel_width, body.height);
            self.paint_panel(ctx, list, panel, &device, busy, input, clicked, pending);
        } else {
            self.marquee = Marquee::default();
        }

        self.status.paint(ctx, list, rect(area.x, footer_top, area.width, m.footer_height));
    }

    /// QML `SelectedDevicePanel`.
    #[allow(clippy::too_many_arguments)]
    fn paint_panel(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        panel: Rect,
        device: &Device,
        busy: bool,
        input: &InputState,
        clicked: bool,
        pending: &mut Vec<SystemAction>,
    ) {
        let m = ctx.metrics.device_page.clone();
        let theme = ctx.theme.clone();
        devices::card(ctx, list, panel, theme.bg_widget, theme.border_inactive);
        let x = panel.x + m.panel_padding;
        let w = panel.width - m.panel_padding * 2.0;
        let mut y = panel.y + m.panel_padding;

        // Title and close.
        let title = rect(x, y, w - m.icon_button - m.gap, m.icon_button);
        ui::paint_text_vcentered(ctx, list, "Selected Device", x, title, m.strip, ctx.metrics.typography.weight_bold, theme.text_muted);
        let close = rect(x + w - m.icon_button, y, m.icon_button, m.icon_button);
        if devices::icon_button(ctx, list, close, "󰅖", theme.error, true, input, clicked) {
            self.selected = None;
        }
        y += m.icon_button + m.gap;

        self.marquee.paint(ctx, list, rect(x, y, w, m.strip_line + m.gap), &device.name, m.marquee, theme.text_main);
        y += m.strip_line + m.gap * 2.0;

        devices::strip(ctx, list, rect(x, y, w, m.strip_line), &[("MAC", &device.mac, theme.secondary)], m.strip, theme.tertiary);
        y += m.strip_line;
        let flag = |on: bool, yes: &'static str, no: &'static str| {
            (if on { yes } else { no }, if on { theme.success } else { theme.error })
        };
        let (connected, cc) = flag(device.connected, "Connected", "Disconnected");
        let (trusted, tc) = flag(device.trusted, "Trusted", "Untrusted");
        let (paired, pc) = flag(device.paired, "Paired", "Unpaired");
        devices::strip(ctx, list, rect(x, y, w, m.strip_line), &[("", connected, cc), ("", trusted, tc), ("", paired, pc)], m.strip, theme.tertiary);
        y += m.strip_line + m.gap;

        let buttons = [
            (if device.paired { "Remove" } else { "Pair" }, if device.paired { BluetoothOp::Forget } else { BluetoothOp::Pair }),
            (
                if device.connected { "Disconnect" } else { "Connect" },
                if device.connected { BluetoothOp::Disconnect } else { BluetoothOp::Connect },
            ),
            (
                if device.trusted { "Untrust" } else { "Trust" },
                if device.trusted { BluetoothOp::Untrust } else { BluetoothOp::Trust },
            ),
        ];
        for (label, op) in buttons {
            let button = rect(x, y, w, m.panel_button);
            if devices::text_button(ctx, list, button, label, !busy, input, clicked) {
                let job = Job::Device { op, name: device.name.clone() };
                let action = SystemAction::BluetoothDevice { address: device.mac.clone(), op };
                self.start(job, action, m.status_ms, pending);
            }
            y += m.panel_button + m.gap;
        }
    }
}

impl Default for BluetoothPage {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_read_both_snapshot_shapes() {
        let snapshot = serde_json::json!({
            "devices": [{ "mac": "AA", "name": "", "connected": "yes", "paired": "yes", "trusted": "no" }],
            "all_devices": [{ "mac": "BB", "name": "Phone", "connected": false, "paired": false, "trusted": true }],
        });
        let paired = devices(&snapshot, "devices");
        assert_eq!((paired[0].name.as_str(), paired[0].connected, paired[0].trusted), ("AA", true, false));
        let all = devices(&snapshot, "all_devices");
        assert_eq!((all[0].name.as_str(), all[0].connected, all[0].trusted), ("Phone", false, true));
    }

    #[test]
    fn messages_match_the_qml_menu() {
        assert_eq!(messages(BluetoothOp::Connect, "Buds").0, "Connecting to Buds...");
        assert_eq!(messages(BluetoothOp::Untrust, "Buds"), ("Removing trust for Buds...".into(), "Untrusted Buds.".into()));
    }
}
