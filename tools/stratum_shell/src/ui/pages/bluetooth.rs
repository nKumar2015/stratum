//! Bluetooth page: the behaviour of `modules/BluetoothMenu.qml`.
//!
//! - Refresh and scan in the header, with "Loading" while a refresh runs.
//! - The adapter card and its switch ("Working…" while it changes). Turning
//!   it on starts a scan, as QML did. Its caption is the status line:
//!   "Connecting to …" while an action runs, then its result.
//! - Every device the adapter knows, discovered ones included, connected
//!   first, with its kind, state and battery. A click expands a row to its
//!   MAC, trust, and Pair/Remove, Trust/Untrust, Connect/Disconnect; a
//!   second click folds it. (QML showed these in a side panel.)
//!
//! One action runs at a time, and the buttons are disabled meanwhile, as in
//! QML. Actions are `SystemAction::Tracked`, so the page learns its own
//! outcome from `SystemState::action_result`.

use serde_json::Value;

use crate::actions::{BluetoothOp, SystemAction};
use crate::render::display_list::{DisplayList, Rect};
use crate::ui::pages::devices::{self, Status};
use crate::ui::pages::layout::{self, rect, Card};
use crate::ui::pages::Optimistic;
use crate::ui::{InputState, PaintCtx};

const BLUETOOTH: &str = "\u{f00af}";
const BLUETOOTH_OFF: &str = "\u{f00b2}";
const REFRESH: &str = "\u{f0450}";
const SCAN: &str = "\u{f0349}";
pub(crate) const EXPAND: &str = "\u{f0140}";
pub(crate) const COLLAPSE: &str = "\u{f0143}";

/// A glyph for BlueZ's guess at what a device is (its freedesktop icon name).
pub(crate) fn device_glyph(icon: &str) -> &'static str {
    match icon {
        "audio-headset" | "audio-headphones" => "\u{f02cb}",
        "audio-card" | "audio-speakers" => "\u{f04c3}",
        "input-mouse" => "\u{f037d}",
        "input-keyboard" => "\u{f030c}",
        "input-gaming" => "\u{f0297}",
        "phone" => "\u{f011c}",
        "computer" => "\u{f0322}",
        _ => BLUETOOTH,
    }
}

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
    icon: String,
    battery: Option<u64>,
}

impl Device {
    /// The row's caption: state, then charge when the device reports it.
    fn caption(&self) -> String {
        let state = match (self.connected, self.paired) {
            (true, _) => "Connected",
            (false, true) => "Paired",
            (false, false) => "Not paired",
        };
        match self.battery {
            Some(pct) => format!("{state} \u{b7} {pct}% battery"),
            None => state.to_string(),
        }
    }

    /// The actions an expanded row offers, in QML's order.
    fn actions(&self) -> [(&'static str, BluetoothOp); 3] {
        [
            if self.paired { ("Remove", BluetoothOp::Forget) } else { ("Pair", BluetoothOp::Pair) },
            if self.trusted { ("Untrust", BluetoothOp::Untrust) } else { ("Trust", BluetoothOp::Trust) },
            if self.connected { ("Disconnect", BluetoothOp::Disconnect) } else { ("Connect", BluetoothOp::Connect) },
        ]
    }
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
                        icon: d["icon"].as_str().unwrap_or("").to_string(),
                        battery: d["battery"].as_u64(),
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
    /// The expanded row, by MAC.
    selected: Option<String>,
    /// The running action and its ticket.
    job: Option<(u64, Job)>,
    /// The running refresh's ticket, and how long it has run (for the dot).
    refresh: Option<(u64, f32)>,
    /// A refresh to send on the next paint (the page was just shown).
    refresh_wanted: bool,
    status: Status,
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
        }
    }

    pub fn tick(&mut self, dt: f32) {
        self.powered.tick(dt);
        self.status.tick(dt);
        if let Some((_, elapsed)) = &mut self.refresh {
            *elapsed += dt;
        }
    }

    /// The loading dot needs frames.
    pub fn animating(&self) -> bool {
        self.refresh.is_some()
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

    fn start(&mut self, job: Job, action: SystemAction, pending: &mut Vec<SystemAction>) {
        let ticket = crate::actions::next_ticket();
        let message = match &job {
            Job::Power(on) => if *on { "Turning Bluetooth on..." } else { "Turning Bluetooth off..." }.to_string(),
            Job::Scan => "Scanning for devices...".to_string(),
            Job::Device { op, name } => messages(*op, name).0,
        };
        self.status.progress(message);
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
                self.start(Job::Scan, SystemAction::BluetoothScan, pending);
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

    /// Returns the bottom of the page.
    pub fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        area: Rect,
        input: &InputState,
        clicked: bool,
        pending: &mut Vec<SystemAction>,
    ) -> f32 {
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
        let scanning = snapshot["scanning"].as_str() == Some("yes");
        let busy = self.job.is_some();
        let all = devices(&snapshot, "all_devices");
        let all = if all.is_empty() { devices(&snapshot, "devices") } else { all };
        let connected: Vec<&Device> = all.iter().filter(|d| d.connected).collect();
        let paired = all.iter().filter(|d| d.paired).count();

        let area = layout::column(area, page.max_width);
        let summary = match (powered, connected.as_slice()) {
            (false, _) => "Off".to_string(),
            (true, []) => format!("On \u{b7} {paired} paired"),
            (true, [one]) => format!("Connected to {}", one.name),
            (true, many) => format!("{} connected \u{b7} {paired} paired", many.len()),
        };
        let buttons = [(REFRESH, self.refresh.is_none()), (SCAN, powered && !busy)];
        let loading = self.refresh.map(|(_, phase)| phase);
        let head = devices::header(ctx, list, area, "Bluetooth", &summary, &buttons, loading, input, clicked);
        match head.button {
            Some(0) => self.refresh(pending),
            Some(_) => self.start(Job::Scan, SystemAction::BluetoothScan, pending),
            None => {}
        }
        let body = head.body;

        // The adapter.
        let (caption, error) = match self.status.message() {
            Some((text, error)) => (text.to_string(), error),
            None if matches!(self.job, Some((_, Job::Power(_)))) => ("Working…".to_string(), false),
            None if powered && scanning => ("On \u{b7} Discovering devices".to_string(), false),
            None => (if powered { "On" } else { "Off" }.to_string(), false),
        };
        let caption_color = if error { theme.error } else { theme.text_muted };
        let card = rect(body.x, body.y, body.width, page.tile_height);
        let glyph = if powered { BLUETOOTH } else { BLUETOOTH_OFF };
        let toggled = devices::power_card(ctx, list, card, glyph, "Bluetooth", &caption, caption_color, powered, !busy, input, clicked);
        if let Some(next) = toggled {
            self.powered.set(next);
            self.start(Job::Power(next), SystemAction::BluetoothPower(next), pending);
        }
        let mut y = card.y + card.height + page.section_spacing;

        // Devices.
        if self.selected.as_ref().is_some_and(|mac| !all.iter().any(|d| &d.mac == mac)) {
            self.selected = None;
        }
        let label = if all.is_empty() { "Devices".to_string() } else { format!("Devices \u{b7} {}", all.len()) };
        y = layout::section(ctx, list, body, y, &label);
        if all.is_empty() {
            return match powered {
                true => devices::empty_card(ctx, list, body, y, "No devices found", "Scan to look for nearby devices"),
                false => devices::empty_card(ctx, list, body, y, "Bluetooth is off", "Turn it on to see your devices"),
            };
        }
        let mut chosen = None;
        let mut toggle_row = None;
        for device in &all {
            let expanded = self.selected.as_deref() == Some(device.mac.as_str());
            let height = if expanded { m.row_height + m.expanded } else { m.row_height };
            let row = rect(body.x, y, body.width, height);
            let top = rect(row.x, row.y, row.width, m.row_height);
            let style = if device.connected {
                Card::Active
            } else if layout::hovered(input, top) {
                Card::Hover
            } else {
                Card::Plain
            };
            layout::card(ctx, list, row, style);
            let trailing = if expanded { COLLAPSE } else { EXPAND };
            let glyph = device_glyph(&device.icon);
            layout::row_content(ctx, list, top, glyph, &device.name, &device.caption(), Some(trailing), device.connected);
            if clicked && layout::hovered(input, top) {
                toggle_row = Some(device.mac.clone());
            }
            if expanded {
                let trust = if device.trusted { "Trusted" } else { "Not trusted" };
                let detail = format!("{} \u{b7} {trust}", device.mac);
                let actions = device.actions();
                let labels = actions.map(|(label, _)| label);
                let line = devices::expansion(ctx, row, m.row_height);
                if let Some(i) = devices::row_actions(ctx, list, line, &detail, &labels, !busy, input, clicked) {
                    chosen = Some((actions[i].1, device.clone()));
                }
            }
            y += height + page.grid_gap;
        }
        if let Some(mac) = toggle_row {
            self.selected = if self.selected.as_ref() == Some(&mac) { None } else { Some(mac) };
        }
        if let Some((op, device)) = chosen {
            let job = Job::Device { op, name: device.name.clone() };
            self.start(job, SystemAction::BluetoothDevice { address: device.mac, op }, pending);
        }
        y - page.grid_gap
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
            "all_devices": [{ "mac": "BB", "name": "Phone", "connected": false, "paired": false, "trusted": true, "icon": "phone", "battery": 60 }],
        });
        let paired = devices(&snapshot, "devices");
        assert_eq!((paired[0].name.as_str(), paired[0].connected, paired[0].trusted), ("AA", true, false));
        assert_eq!(paired[0].battery, None);
        let all = devices(&snapshot, "all_devices");
        assert_eq!((all[0].name.as_str(), all[0].connected, all[0].trusted), ("Phone", false, true));
        assert_eq!((all[0].icon.as_str(), all[0].battery), ("phone", Some(60)));
    }

    #[test]
    fn captions_give_state_then_charge() {
        let snapshot = serde_json::json!({ "devices": [
            { "mac": "AA", "name": "Buds", "connected": "yes", "paired": "yes", "trusted": "yes", "battery": 80 },
            { "mac": "BB", "name": "Mouse", "connected": "no", "paired": "no", "trusted": "no" },
        ]});
        let list = devices(&snapshot, "devices");
        assert_eq!(list[0].caption(), "Connected \u{b7} 80% battery");
        assert_eq!(list[1].caption(), "Not paired");
        assert_eq!(list[1].actions().map(|(label, _)| label), ["Pair", "Trust", "Connect"]);
    }

    #[test]
    fn messages_match_the_qml_menu() {
        assert_eq!(messages(BluetoothOp::Connect, "Buds").0, "Connecting to Buds...");
        assert_eq!(messages(BluetoothOp::Untrust, "Buds"), ("Removing trust for Buds...".into(), "Untrusted Buds.".into()));
    }
}
