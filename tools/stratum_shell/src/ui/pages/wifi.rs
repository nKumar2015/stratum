//! Network page: matching `modules/WifiMenu.qml`.
//!
//! - The radio switch ("Working…" while it changes).
//! - A card for the active connection: SSID, state, signal bars, security,
//!   IP and gateway, with disconnect and forget buttons.
//! - Refresh (a rescan), the network count, and "Loading" while it runs.
//! - The networks, strongest first: SSID, signal, security and state. A click
//!   selects one, and a second click deselects it.
//! - The selected network's panel: its SSID (scrolling when it does not fit),
//!   state, signal, security, a password field for a secured network that is
//!   not saved (or whose saved password just failed), and Connect/Disconnect.
//!   Return in the field connects.
//! - A status line: "Connecting to …" while an action runs, then its result,
//!   with QML's wording.
//!
//! One action at a time, as in QML. The active connection and the saved
//! networks come from `SystemState::wifi_details`, which actions refresh.

use serde_json::Value;

use crate::actions::SystemAction;
use crate::render::display_list::{DisplayList, Point, Rect};
use crate::ui::controls;
use crate::ui::pages::devices::{self, rect, Marquee, Status};
use crate::ui::pages::{heading, Optimistic};
use crate::ui::text_field::{FieldKey, FieldStyle, TextField};
use crate::ui::{self, InputState, PaintCtx};

/// QML's thresholds for `▮▮▮▯`.
const SIGNAL_STRONG: i64 = 75;
const SIGNAL_GOOD: i64 = 50;
const SIGNAL_FAIR: i64 = 25;

/// Four bars, filled by signal strength.
pub fn signal_bars(signal: i64) -> String {
    let level = match signal {
        s if s >= SIGNAL_STRONG => 4,
        s if s >= SIGNAL_GOOD => 3,
        s if s >= SIGNAL_FAIR => 2,
        s if s > 0 => 1,
        _ => 0,
    };
    (0..4).map(|i| if i < level { '▮' } else { '▯' }).collect()
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

pub struct WifiPage {
    enabled: Optimistic<bool>,
    selected: Option<String>,
    job: Option<(u64, Job)>,
    refresh: Option<(u64, f32)>,
    refresh_wanted: bool,
    status: Status,
    marquee: Marquee,
    password: TextField,
    /// The password field has the keyboard.
    typing: bool,
    /// The saved password failed: ask for one even though the network is
    /// known.
    retry_password: bool,
    /// Return was pressed in the field, with this password; the next paint
    /// connects.
    submit: Option<crate::pam::Secret>,
    scroll: usize,
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
            marquee: Marquee::default(),
            password: TextField::new(),
            typing: false,
            retry_password: false,
            submit: None,
            scroll: 0,
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
        self.refresh.is_some() || self.marquee.moving()
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
                self.retry_password = false;
                self.password.clear();
                self.typing = false;
                self.status.result(format!("Connected to {ssid}."), false, status_ms);
            }
            (None, Job::Forget(ssid)) => self.status.result(format!("Forgot network {ssid}."), false, status_ms),
            (None, _) => self.status.result("Action completed.".into(), false, status_ms),
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
        let area = heading(ctx, list, area, "Network");
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
        let busy = self.job.is_some();

        // Radio switch.
        let mut y = area.y;
        let working = matches!(self.job, Some((_, Job::Radio(_))));
        let label = if working { "Working…" } else { "Wi-Fi" };
        if let Some(next) =
            controls::toggle(ctx, list, rect(area.x, y, area.width, page.button_height), label, enabled, input, clicked && !busy)
        {
            self.enabled.set(next);
            self.start(Job::Radio(next), SystemAction::WifiToggle(next), pending);
        }
        y += page.button_height + m.spacing;

        // The active connection.
        let card_h = m.card_padding * 2.0 + m.strip_line * 3.0;
        let card_rect = rect(area.x, y, area.width, card_h);
        devices::card(ctx, list, card_rect, theme.bg_widget, theme.border_inactive);
        let buttons_w = m.icon_button * 2.0 + m.gap;
        let inner = rect(
            card_rect.x + m.card_padding,
            card_rect.y + m.card_padding,
            card_rect.width - m.card_padding * 3.0 - buttons_w,
            card_h - m.card_padding * 2.0,
        );
        let headline = if active_ssid.is_empty() { "Not connected".to_string() } else { format!("Connected to {active_ssid}") };
        let line = |i: f32| rect(inner.x, inner.y + i * m.strip_line, inner.width, m.strip_line);
        list.push_clip(line(0.0));
        ui::paint_text_vcentered(ctx, list, &headline, inner.x, line(0.0), m.headline, ctx.metrics.typography.weight_bold, theme.text_main);
        list.pop_clip();
        let na = "N/A".to_string();
        let state_text = active["state"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| if enabled { "idle".into() } else { "wifi disabled".into() });
        let (signal_text, signal_color) = match &active_net {
            Some(n) => (signal_bars(n.signal), if n.signal >= SIGNAL_GOOD { theme.success } else { theme.warning }),
            None => (na.clone(), theme.warning),
        };
        let security = active_net.as_ref().map(|n| n.security.clone()).filter(|s| !s.is_empty()).unwrap_or(na.clone());
        devices::strip(
            ctx,
            list,
            line(1.0),
            &[("State", &state_text, theme.tertiary), ("Signal", &signal_text, signal_color), ("Security", &security, theme.tertiary)],
            m.strip,
            theme.secondary,
        );
        let value = |k: &str| active[k].as_str().filter(|s| !s.is_empty()).unwrap_or("N/A").to_string();
        let (ip, gateway) = (value("ip"), value("gateway"));
        devices::strip(
            ctx,
            list,
            line(2.0),
            &[("IP", &ip, theme.tertiary), ("Gateway", &gateway, theme.tertiary)],
            m.strip,
            theme.secondary,
        );
        let bx = card_rect.x + card_rect.width - m.card_padding - buttons_w;
        let by = card_rect.y + (card_h - m.icon_button) * 0.5;
        if devices::icon_button(ctx, list, rect(bx, by, m.icon_button, m.icon_button), "󰖪", theme.text_main, !active_device.is_empty() && !busy, input, clicked) {
            self.start(Job::Disconnect(active_ssid.clone()), SystemAction::WifiDisconnect(active_device.clone()), pending);
        }
        let forget = rect(bx + m.icon_button + m.gap, by, m.icon_button, m.icon_button);
        if devices::icon_button(ctx, list, forget, "󰆴", theme.text_main, !active_ssid.is_empty() && !busy, input, clicked) {
            self.start(Job::Forget(active_ssid.clone()), SystemAction::WifiForget(active_ssid.clone()), pending);
        }
        y += card_h + m.spacing;

        // Toolbar.
        let bar = rect(area.x, y, area.width, m.icon_button);
        if devices::icon_button(ctx, list, rect(bar.x, bar.y, m.icon_button, m.icon_button), "󰑐", theme.text_main, self.refresh.is_none(), input, clicked) {
            self.refresh(pending);
        }
        let mut right = bar.x + bar.width;
        if let Some((_, phase)) = self.refresh {
            right -= devices::loading(ctx, list, bar, phase) + m.gap * 2.0;
        }
        let count = format!("{} networks", all.len());
        let count_w = ctx.measure(&count, m.strip);
        ui::paint_text_vcentered(ctx, list, &count, right - count_w, bar, m.strip, 400, theme.text_muted);
        y += m.icon_button + m.spacing;

        // The list, and the selected network's panel.
        let footer_top = area.y + area.height - m.footer_height;
        let body = rect(area.x, y, area.width, footer_top - m.spacing - y);
        let selected = self.selected.as_ref().and_then(|s| all.iter().find(|n| &n.ssid == s)).cloned();
        if self.selected.is_some() && selected.is_none() {
            self.deselect();
        }
        let list_w = if selected.is_some() { body.width - m.panel_width - m.spacing } else { body.width };
        let list_rect = rect(body.x, body.y, list_w, body.height);
        if all.is_empty() {
            let note = if enabled { "No networks found" } else { "Wi-Fi is off" };
            let line = rect(list_rect.x, list_rect.y, list_rect.width, m.row_height);
            ui::paint_text_vcentered(ctx, list, note, line.x, line, m.row_title, 400, theme.text_muted);
        }
        let pitch = m.row_height + m.row_gap;
        let visible = ((list_rect.height + m.row_gap) / pitch).floor().max(1.0) as usize;
        self.scroll = self.scroll.min(all.len().saturating_sub(visible));
        list.push_clip(list_rect);
        let mut clicked_row = None;
        for (i, net) in all.iter().skip(self.scroll).take(visible).enumerate() {
            let row = rect(list_rect.x, list_rect.y + i as f32 * pitch, list_rect.width, m.row_height);
            let is_selected = self.selected.as_deref() == Some(net.ssid.as_str());
            let fill = if is_selected { theme.bg_hover } else { theme.bg_widget };
            let border = if net.in_use { theme.border_active } else { theme.border_inactive };
            devices::card(ctx, list, row, fill, border);
            let text_w = row.width - m.row_padding * 2.0;
            let title = rect(row.x + m.row_padding, row.y + m.row_padding, text_w, (m.row_height - m.row_padding * 2.0) * 0.55);
            list.push_clip(title);
            ui::paint_text_vcentered(ctx, list, &net.ssid, title.x, title, m.row_title, ctx.metrics.typography.weight_bold, theme.text_main);
            list.pop_clip();
            let state_line = rect(title.x, title.y + title.height, text_w, row.y + row.height - m.row_padding - title.y - title.height);
            let bars = signal_bars(net.signal);
            let security = if is_secure(&net.security) { net.security.as_str() } else { "Open" };
            devices::strip(
                ctx,
                list,
                state_line,
                &[
                    ("Signal", &bars, if net.signal >= SIGNAL_GOOD { theme.success } else { theme.warning }),
                    ("Security", security, theme.tertiary),
                    (
                        "State",
                        if net.in_use { "Connected" } else { "Disconnected" },
                        if net.in_use { theme.success } else { theme.error },
                    ),
                ],
                m.row_strip,
                theme.secondary,
            );
            if clicked && input.pointer.is_some_and(|(x, y)| row.contains(Point::new(x, y)) && list_rect.contains(Point::new(x, y))) {
                clicked_row = Some((net.ssid.clone(), is_selected));
            }
        }
        list.pop_clip();
        if let Some((ssid, was_selected)) = clicked_row {
            self.deselect();
            if !was_selected {
                self.selected = Some(ssid);
            }
        }

        if let Some(net) = selected.filter(|n| self.selected.as_deref() == Some(n.ssid.as_str())) {
            let panel = rect(body.x + list_w + m.spacing, body.y, m.panel_width, body.height);
            let known = known.iter().any(|k| k == &net.ssid);
            self.paint_panel(ctx, list, panel, &net, known, &active_device, busy, input, clicked, pending);
        } else {
            self.marquee = Marquee::default();
        }

        self.status.paint(ctx, list, rect(area.x, footer_top, area.width, m.footer_height));
    }

    fn deselect(&mut self) {
        self.selected = None;
        self.password.clear();
        self.typing = false;
        self.retry_password = false;
        self.submit = None;
    }

    /// QML `SelectedNetworkPanel`.
    #[allow(clippy::too_many_arguments)]
    fn paint_panel(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        panel: Rect,
        net: &Network,
        known: bool,
        active_device: &str,
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

        let title = rect(x, y, w - m.icon_button - m.gap, m.icon_button);
        ui::paint_text_vcentered(ctx, list, "Selected Network", x, title, m.strip, ctx.metrics.typography.weight_bold, theme.text_muted);
        if devices::icon_button(ctx, list, rect(x + w - m.icon_button, y, m.icon_button, m.icon_button), "󰅖", theme.error, true, input, clicked) {
            self.deselect();
            return;
        }
        y += m.icon_button + m.gap;

        self.marquee.paint(ctx, list, rect(x, y, w, m.strip_line + m.gap), &net.ssid, m.marquee, theme.text_main);
        y += m.strip_line + m.gap * 2.0;

        let (state, state_color) = if net.in_use { ("Connected", theme.success) } else { ("Disconnected", theme.error) };
        devices::strip(ctx, list, rect(x, y, w, m.strip_line), &[("", state, state_color)], m.strip, theme.tertiary);
        y += m.strip_line;
        let bars = signal_bars(net.signal);
        let bars_color = if net.signal >= SIGNAL_GOOD { theme.success } else { theme.warning };
        devices::strip(ctx, list, rect(x, y, w, m.strip_line), &[("Signal", &bars, bars_color)], m.strip, theme.secondary);
        y += m.strip_line;
        let security = if net.security.is_empty() { "Open" } else { net.security.as_str() };
        devices::strip(ctx, list, rect(x, y, w, m.strip_line), &[("Security", security, theme.tertiary)], m.strip, theme.secondary);
        y += m.strip_line + m.gap;

        // A password, for a secured network that is not saved or whose saved
        // password just failed.
        let secure = is_secure(&net.security);
        let ask = secure && !net.in_use && (!known || self.retry_password);
        if ask {
            let field = rect(x, y, w, m.panel_button);
            let border = if self.typing { theme.border_active } else { theme.border_inactive };
            crate::ui::text_field::paint_frame(list, Rect { radius: m.radius, ..field }, m.border, border, theme.bg_dark);
            let style = FieldStyle {
                text: m.field_text,
                padding: m.field_padding,
                caret_width: m.caret_width,
                masked: true,
                placeholder: Some("Enter password"),
            };
            self.password.paint(ctx, list, field, style, !self.typing);
            if clicked {
                self.typing = input.pointer.is_some_and(|(px, py)| field.contains(Point::new(px, py)));
            }
            y += m.panel_button + m.gap;
        }

        let label = if net.in_use { "Disconnect" } else { "Connect" };
        let enabled = !busy && (!net.in_use || !active_device.is_empty());
        let pressed = devices::text_button(ctx, list, rect(x, y, w, m.panel_button), label, enabled, input, clicked);
        let submitted = self.submit.take().filter(|_| !busy && !net.in_use && ask);
        if !pressed && submitted.is_none() {
            return;
        }
        if net.in_use {
            self.start(Job::Disconnect(net.ssid.clone()), SystemAction::WifiDisconnect(active_device.to_string()), pending);
            return;
        }
        let password = if ask {
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
        let job = Job::Connect { ssid: net.ssid.clone(), known_secure: known && secure };
        self.start(job, SystemAction::WifiConnect { ssid: net.ssid.clone(), password }, pending);
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
    fn signal_bars_follow_the_qml_thresholds() {
        assert_eq!(signal_bars(80), "▮▮▮▮");
        assert_eq!(signal_bars(50), "▮▮▮▯");
        assert_eq!(signal_bars(30), "▮▮▯▯");
        assert_eq!(signal_bars(5), "▮▯▯▯");
        assert_eq!(signal_bars(0), "▯▯▯▯");
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
