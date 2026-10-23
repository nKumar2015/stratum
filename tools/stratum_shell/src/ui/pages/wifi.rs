//! Network page: the behaviour of `modules/WifiMenu.qml`.
//!
//! - Refresh (a rescan) in the header, with "Loading" while it runs.
//! - The radio card and its switch ("Working…" while it changes). Its
//!   caption is the status line: "Connecting to …" while an action runs,
//!   then its result, with QML's wording.
//! - The active connection: SSID, signal, security, IP and gateway, with
//!   Disconnect and Forget.
//! - The other networks, strongest first, with signal, security and whether
//!   they are saved. A click expands a row: a password field for a secured
//!   network that is not saved (or whose saved password just failed), and
//!   Connect (Forget too, for a saved one). Return in the field connects. A
//!   second click folds it. (QML showed these in a side panel.)
//!
//! One action at a time, as in QML. The active connection and the saved
//! networks come from `SystemState::wifi_details`, which actions refresh.

use serde_json::Value;

use crate::actions::SystemAction;
use crate::render::display_list::{DisplayList, Rect};
use crate::ui::pages::bluetooth::{COLLAPSE, EXPAND};
use crate::ui::pages::devices::{self, Status};
use crate::ui::pages::layout::{self, rect, Card};
use crate::ui::pages::Optimistic;
use crate::ui::text_field::{FieldKey, FieldStyle, TextField};
use crate::ui::{InputState, PaintCtx};

const WIFI: &str = "\u{f05a9}";
const WIFI_OFF: &str = "\u{f05aa}";
const REFRESH: &str = "\u{f0450}";

/// QML's signal thresholds, for four bars.
const SIGNAL_STRONG: i64 = 75;
const SIGNAL_GOOD: i64 = 50;
const SIGNAL_FAIR: i64 = 25;

/// Bars lit, out of four.
pub fn signal_level(signal: i64) -> usize {
    match signal {
        s if s >= SIGNAL_STRONG => 4,
        s if s >= SIGNAL_GOOD => 3,
        s if s >= SIGNAL_FAIR => 2,
        s if s > 0 => 1,
        _ => 0,
    }
}

/// A Wi-Fi glyph with `signal`'s bars, padlocked when `secure`.
fn signal_glyph(signal: i64, secure: bool) -> &'static str {
    const OPEN: [&str; 5] = ["\u{f092f}", "\u{f091f}", "\u{f0922}", "\u{f0925}", "\u{f0928}"];
    const LOCKED: [&str; 5] = ["\u{f092c}", "\u{f0921}", "\u{f0924}", "\u{f0927}", "\u{f092a}"];
    let level = signal_level(signal);
    if secure { LOCKED[level] } else { OPEN[level] }
}

/// Whether a network asks for a password, by its `nmcli` security field.
pub fn is_secure(security: &str) -> bool {
    let value = security.trim().to_ascii_lowercase();
    !value.is_empty() && value != "--" && value != "none"
}

#[derive(Clone, Debug, PartialEq)]
struct Network {
    ssid: String,
    signal: i64,
    security: String,
    in_use: bool,
}

impl Network {
    fn security_label(&self) -> &str {
        if is_secure(&self.security) { &self.security } else { "Open" }
    }
}

/// One row per SSID, as QML listed them: `nmcli` reports every access point,
/// so a campus network appears dozens of times. The connected entry wins,
/// then the strongest; a missing security is filled from a duplicate.
/// Connected first, then strongest, then by name.
fn networks(snapshot: &Value) -> Vec<Network> {
    let mut unique: Vec<Network> = Vec::new();
    for n in snapshot["networks"].as_array().into_iter().flatten() {
        let Some(ssid) = n["ssid"].as_str().filter(|s| !s.is_empty()) else {
            continue;
        };
        let candidate = Network {
            ssid: ssid.to_string(),
            signal: n["signal"].as_i64().unwrap_or(0),
            security: n["security"].as_str().unwrap_or("").to_string(),
            in_use: n["in_use"].as_bool().unwrap_or(false),
        };
        match unique.iter_mut().find(|u| u.ssid == candidate.ssid) {
            None => unique.push(candidate),
            Some(existing) => {
                if !existing.in_use && (candidate.in_use || candidate.signal > existing.signal) {
                    *existing = candidate;
                } else if existing.security.is_empty() && !candidate.security.is_empty() {
                    existing.security = candidate.security;
                }
            }
        }
    }
    unique.sort_by(|a, b| b.in_use.cmp(&a.in_use).then(b.signal.cmp(&a.signal)).then_with(|| a.ssid.cmp(&b.ssid)));
    unique
}

#[derive(Clone, Debug, PartialEq)]
enum Job {
    Radio(bool),
    /// `known_secure`: a saved, secured network, whose failure means its
    /// saved password is wrong (QML then asks for one).
    Connect { ssid: String, known_secure: bool },
    Disconnect(String),
    Forget(String),
}

/// What a click in the page asked for.
enum Request {
    /// With the password Return submitted, if that is what asked.
    Connect(Network, Option<crate::pam::Secret>),
    Disconnect,
    Forget(String),
}

pub struct WifiPage {
    enabled: Optimistic<bool>,
    /// The expanded row, by SSID.
    selected: Option<String>,
    job: Option<(u64, Job)>,
    refresh: Option<(u64, f32)>,
    refresh_wanted: bool,
    status: Status,
    password: TextField,
    /// The password field has the keyboard.
    typing: bool,
    /// The saved password failed: ask for one even though the network is
    /// known.
    retry_password: bool,
    /// Return was pressed in the field, with this password; the next paint
    /// connects.
    submit: Option<crate::pam::Secret>,
}

impl WifiPage {
    pub fn new() -> Self {
        Self {
            enabled: Optimistic::new(),
            selected: None,
            job: None,
            refresh: None,
            refresh_wanted: true,
            status: Status::default(),
            password: TextField::new(),
            typing: false,
            retry_password: false,
            submit: None,
        }
    }

    pub fn tick(&mut self, dt: f32) {
        self.enabled.tick(dt);
        self.status.tick(dt);
        if let Some((_, elapsed)) = &mut self.refresh {
            *elapsed += dt;
        }
    }

    pub fn animating(&self) -> bool {
        self.refresh.is_some()
    }

    /// A rescan when the page is shown, as the QML menu did on open. Sent
    /// by the next paint.
    pub fn request_refresh(&mut self, _pending: &mut Vec<SystemAction>) {
        self.refresh_wanted = true;
    }

    pub fn wants_keys(&self) -> bool {
        self.typing && self.selected.is_some()
    }

    /// Typing into the password field. Return connects; Escape leaves it.
    pub fn key(
        &mut self,
        keysym: smithay_client_toolkit::seat::keyboard::Keysym,
        text: Option<&str>,
        ctrl: bool,
    ) {
        match self.password.key(keysym, text, ctrl) {
            FieldKey::Submit(secret) => self.submit = Some(secret),
            FieldKey::Escape => self.typing = false,
            _ => {}
        }
    }

    fn refresh(&mut self, pending: &mut Vec<SystemAction>) {
        let ticket = crate::actions::next_ticket();
        self.refresh = Some((ticket, 0.0));
        pending.push(SystemAction::Tracked { ticket, action: Box::new(SystemAction::WifiRefresh { rescan: true }) });
    }

    fn start(&mut self, job: Job, action: SystemAction, pending: &mut Vec<SystemAction>) {
        let message = match &job {
            Job::Radio(on) => if *on { "Turning Wi-Fi on..." } else { "Turning Wi-Fi off..." }.to_string(),
            Job::Connect { ssid, .. } => format!("Connecting to {ssid}..."),
            Job::Disconnect(ssid) => format!("Disconnecting {ssid}..."),
            Job::Forget(ssid) => format!("Forgetting {ssid}..."),
        };
        self.status.progress(message);
        let ticket = crate::actions::next_ticket();
        self.job = Some((ticket, job));
        pending.push(SystemAction::Tracked { ticket, action: Box::new(action) });
    }

    fn collect(&mut self, ctx: &PaintCtx<'_>) {
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
            (Some(_), Job::Connect { known_secure: true, .. }) if !self.retry_password => {
                self.retry_password = true;
                self.typing = true;
                self.status.result("Saved credentials failed. Enter password and retry.".into(), true, status_ms);
            }
            (Some(error), _) => self.status.result(error, true, status_ms),
            (None, Job::Connect { ssid, .. }) => {
                self.deselect();
                self.status.result(format!("Connected to {ssid}."), false, status_ms);
            }
            (None, Job::Forget(ssid)) => self.status.result(format!("Forgot network {ssid}."), false, status_ms),
            (None, _) => self.status.result("Action completed.".into(), false, status_ms),
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
        self.collect(ctx);

        let m = ctx.metrics.device_page.clone();
        let page = ctx.metrics.page.clone();
        let theme = ctx.theme.clone();
        let wifi = ctx.state.wifi.get();
        let real_enabled = wifi["state"].as_str() != Some("disabled");
        let enabled = self.enabled.view(real_enabled);
        let details = ctx.state.wifi_details.get();
        let all = if enabled { networks(&ctx.state.wifi_networks.get()) } else { Vec::new() };
        let known: Vec<String> = details["known"]
            .as_array()
            .map(|k| k.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        let active = &details["active"];
        let active_device = active["device"].as_str().unwrap_or("").to_string();
        let active_ssid = active["ssid"].as_str().unwrap_or("").to_string();
        let active_net = all.iter().find(|n| n.in_use).cloned();
        let others: Vec<&Network> = all.iter().filter(|n| !n.in_use).collect();
        let busy = self.job.is_some();

        let area = layout::column(area, page.max_width);
        let summary = match (enabled, active_ssid.is_empty()) {
            (false, _) => "Wi-Fi is off".to_string(),
            (true, true) => "Not connected".to_string(),
            (true, false) => format!("Connected to {active_ssid}"),
        };
        let loading = self.refresh.map(|(_, phase)| phase);
        let buttons = [(REFRESH, self.refresh.is_none())];
        let head = devices::header(ctx, list, area, "Network", &summary, &buttons, loading, input, clicked);
        if head.button.is_some() {
            self.refresh(pending);
        }
        let body = head.body;

        // The radio.
        let (caption, error) = match self.status.message() {
            Some((text, error)) => (text.to_string(), error),
            None if matches!(self.job, Some((_, Job::Radio(_)))) => ("Working…".to_string(), false),
            None if !enabled => ("Off".to_string(), false),
            None if active_ssid.is_empty() => ("On".to_string(), false),
            None => (format!("On \u{b7} {active_ssid}"), false),
        };
        let caption_color = if error { theme.error } else { theme.text_muted };
        let card = rect(body.x, body.y, body.width, page.tile_height);
        let glyph = if enabled { WIFI } else { WIFI_OFF };
        if let Some(next) = devices::power_card(ctx, list, card, glyph, "Wi-Fi", &caption, caption_color, enabled, !busy, input, clicked) {
            self.enabled.set(next);
            self.start(Job::Radio(next), SystemAction::WifiToggle(next), pending);
        }
        let mut y = card.y + card.height + page.section_spacing;
        let mut request = None;

        // The active connection.
        if !active_ssid.is_empty() {
            y = layout::section(ctx, list, body, y, "Connection");
            let row = rect(body.x, y, body.width, m.row_height + m.expanded);
            layout::card(ctx, list, row, Card::Active);
            let (signal, security) = match &active_net {
                Some(n) => (n.signal, n.security_label().to_string()),
                None => (0, "Unknown".to_string()),
            };
            let caption = format!("Connected \u{b7} Signal {signal}% \u{b7} {security}");
            let glyph = signal_glyph(signal, is_secure(&security));
            let top = rect(row.x, row.y, row.width, m.row_height);
            layout::row_content(ctx, list, top, glyph, &active_ssid, &caption, None, true);
            let value = |k: &str| active[k].as_str().filter(|s| !s.is_empty()).unwrap_or("N/A").to_string();
            let detail = format!("IP {} \u{b7} Gateway {}", value("ip"), value("gateway"));
            let line = devices::expansion(ctx, row, m.row_height);
            let can_disconnect = !busy && !active_device.is_empty();
            match devices::row_actions(ctx, list, line, &detail, &["Forget", "Disconnect"], can_disconnect, input, clicked) {
                Some(0) => request = Some(Request::Forget(active_ssid.clone())),
                Some(_) => request = Some(Request::Disconnect),
                None => {}
            }
            y = row.y + row.height + page.section_spacing;
        }

        // Every other network.
        if self.selected.as_ref().is_some_and(|s| !others.iter().any(|n| &n.ssid == s)) {
            self.deselect();
        }
        let label = if others.is_empty() { "Networks".to_string() } else { format!("Networks \u{b7} {}", others.len()) };
        y = layout::section(ctx, list, body, y, &label);
        let bottom = if others.is_empty() {
            match enabled {
                true => devices::empty_card(ctx, list, body, y, "No networks found", "Refresh to scan again"),
                false => devices::empty_card(ctx, list, body, y, "Wi-Fi is off", "Turn it on to see networks"),
            }
        } else {
            let mut toggle_row = None;
            for net in &others {
                let expanded = self.selected.as_deref() == Some(net.ssid.as_str());
                let height = if expanded { m.row_height + m.expanded } else { m.row_height };
                let row = rect(body.x, y, body.width, height);
                let top = rect(row.x, row.y, row.width, m.row_height);
                let hot = layout::hovered(input, top);
                layout::card(ctx, list, row, if hot { Card::Hover } else { Card::Plain });
                let saved = known.iter().any(|k| k == &net.ssid);
                let caption = match saved {
                    true => format!("Saved \u{b7} {} \u{b7} {}%", net.security_label(), net.signal),
                    false => format!("{} \u{b7} {}%", net.security_label(), net.signal),
                };
                let glyph = signal_glyph(net.signal, is_secure(&net.security));
                let trailing = if expanded { COLLAPSE } else { EXPAND };
                layout::row_content(ctx, list, top, glyph, &net.ssid, &caption, Some(trailing), false);
                if clicked && hot {
                    toggle_row = Some(net.ssid.clone());
                }
                if expanded {
                    let line = devices::expansion(ctx, row, m.row_height);
                    if let Some(r) = self.paint_expansion(ctx, list, line, net, saved, busy, input, clicked) {
                        request = Some(r);
                    }
                }
                y += height + page.grid_gap;
            }
            if let Some(ssid) = toggle_row {
                let was_selected = self.selected.as_ref() == Some(&ssid);
                self.deselect();
                if !was_selected {
                    self.selected = Some(ssid);
                }
            }
            y - page.grid_gap
        };

        match request {
            Some(Request::Disconnect) => {
                self.start(Job::Disconnect(active_ssid.clone()), SystemAction::WifiDisconnect(active_device), pending);
            }
            Some(Request::Forget(ssid)) => self.start(Job::Forget(ssid.clone()), SystemAction::WifiForget(ssid), pending),
            Some(Request::Connect(net, submitted)) => self.connect(net, submitted, &known, pending),
            None => {}
        }
        bottom
    }

    /// An expanded network's line: the password field when one is needed,
    /// and its buttons.
    #[allow(clippy::too_many_arguments)]
    fn paint_expansion(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        line: Rect,
        net: &Network,
        saved: bool,
        busy: bool,
        input: &InputState,
        clicked: bool,
    ) -> Option<Request> {
        let m = ctx.metrics.device_page.clone();
        let page = ctx.metrics.page.clone();
        let theme = ctx.theme.clone();
        let ask = self.asks_password(net, saved);
        // Return is acted on in the frame it arrives, or not at all.
        let submitted = self.submit.take().filter(|_| !busy && ask);
        if !ask {
            let labels: &[&str] = if saved { &["Forget", "Connect"] } else { &["Connect"] };
            let detail = format!("Signal {}% \u{b7} {}", net.signal, net.security_label());
            return match devices::row_actions(ctx, list, line, &detail, labels, !busy, input, clicked) {
                Some(0) if saved => Some(Request::Forget(net.ssid.clone())),
                Some(_) => Some(Request::Connect(net.clone(), None)),
                None => None,
            };
        }

        let y = line.y + (line.height - page.badge) * 0.5;
        let (left, hit) = layout::text_buttons(ctx, list, line.x + line.width, y, &["Connect"], !busy, input, clicked);
        let field = rect(line.x, y, left - page.grid_gap - line.x, page.badge);
        let border = if self.typing { theme.primary } else { theme.outline };
        crate::ui::text_field::paint_frame(list, Rect { radius: m.radius, ..field }, m.border, border, theme.surface);
        let style = FieldStyle {
            text: m.field_text,
            padding: m.field_padding,
            caret_width: m.caret_width,
            masked: true,
            placeholder: Some("Enter password"),
        };
        self.password.paint(ctx, list, field, style, !self.typing);
        if clicked {
            self.typing = layout::hovered(input, field);
        }
        match (hit, submitted) {
            (_, Some(secret)) => Some(Request::Connect(net.clone(), Some(secret))),
            (Some(_), None) => Some(Request::Connect(net.clone(), None)),
            (None, None) => None,
        }
    }

    /// A secured network that is not saved, or whose saved password failed.
    fn asks_password(&self, net: &Network, saved: bool) -> bool {
        is_secure(&net.security) && (!saved || self.retry_password)
    }

    /// Connect to `net`, with the password Return `submitted`, else the one
    /// in the field.
    fn connect(&mut self, net: Network, submitted: Option<crate::pam::Secret>, known: &[String], pending: &mut Vec<SystemAction>) {
        let saved = known.iter().any(|k| k == &net.ssid);
        let password = if self.asks_password(&net, saved) {
            // Return moved the text out of the field; the button reads it.
            let typed = match &submitted {
                Some(secret) => secret.expose().trim().to_string(),
                None => self.password.text().trim().to_string(),
            };
            if typed.is_empty() {
                self.status.progress("Enter password to connect.".into());
                self.typing = true;
                return;
            }
            Some(typed)
        } else {
            None
        };
        let job = Job::Connect { ssid: net.ssid.clone(), known_secure: saved && is_secure(&net.security) };
        self.start(job, SystemAction::WifiConnect { ssid: net.ssid, password }, pending);
    }

    fn deselect(&mut self) {
        self.selected = None;
        self.password.clear();
        self.typing = false;
        self.retry_password = false;
        self.submit = None;
    }
}

impl Default for WifiPage {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_levels_follow_the_qml_thresholds() {
        assert_eq!(signal_level(80), 4);
        assert_eq!(signal_level(50), 3);
        assert_eq!(signal_level(30), 2);
        assert_eq!(signal_level(5), 1);
        assert_eq!(signal_level(0), 0);
        assert_ne!(signal_glyph(80, true), signal_glyph(80, false), "a secured network shows a lock");
    }

    #[test]
    fn networks_are_one_per_ssid_connected_first() {
        let snapshot = serde_json::json!({ "networks": [
            { "ssid": "Cafe", "signal": 90, "security": "", "in_use": false },
            { "ssid": "Campus", "signal": 70, "security": "WPA2", "in_use": false },
            { "ssid": "Campus", "signal": 40, "security": "WPA2", "in_use": true },
            { "ssid": "Cafe", "signal": 60, "security": "WPA2", "in_use": false },
            { "ssid": "", "signal": 99, "security": "", "in_use": false },
        ]});
        let list = networks(&snapshot);
        let summary: Vec<(&str, i64, bool, &str)> =
            list.iter().map(|n| (n.ssid.as_str(), n.signal, n.in_use, n.security.as_str())).collect();
        assert_eq!(summary, [("Campus", 40, true, "WPA2"), ("Cafe", 90, false, "WPA2")]);
    }

    #[test]
    fn security_marks_secured_networks() {
        assert!(is_secure("WPA2"));
        assert!(!is_secure("--"));
        assert!(!is_secure(""));
        assert!(!is_secure(" none "));
    }
}
