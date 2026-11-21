//! Notifications page: the live notification list, then its settings.
//!
//! The header holds Do Not Disturb and "Clear all". Cards are the toasts'
//! cards, newest first, showing the last reply instead of a reply field as
//! the QML centre did. The whole list is shown and the page scrolls.
//!
//! Under it, Do Not Disturb's schedule (`crate::dnd`) and how each app's
//! notifications are shown: a toast, silently in the list, or not at all
//! (`[notifications]`, kept over the config through `SetSetting`). The apps
//! listed are those with a setting, those in the list, and any that
//! notified since the page was made.

use std::collections::BTreeSet;

use crate::actions::SystemAction;
use crate::config::AppMode;
use crate::ui::pages::settings_card::{with_current, SettingsCard};
use crate::render::display_list::{DisplayList, Point, Rect};
use crate::ui::notification_card::{self, CardHit, CardPaint, ReplyMode};
use crate::ui::pages::layout::{self, rect, Card};
use crate::ui::{self, InputState, PaintCtx};

const BELL: &str = "\u{f009a}";
const BELL_OFF: &str = "\u{f009b}";
const CLEAR: &str = "Clear all";

pub struct NotificationsPage {
    start_drag: bool,
    end_drag: bool,
    /// Apps that notified while the shell ran, so clearing the list does
    /// not hide their settings.
    seen: BTreeSet<String>,
}

/// Every half hour of the day, in minutes.
fn half_hours() -> Vec<u32> {
    (0..48).map(|i| i * 30).collect()
}

const MODES: [(AppMode, &str); 3] = [(AppMode::Toast, "Toast"), (AppMode::Silent, "Silent"), (AppMode::Off, "Off")];

impl NotificationsPage {
    pub fn new() -> Self {
        Self { start_drag: false, end_drag: false, seen: BTreeSet::new() }
    }

    /// Do Not Disturb's schedule and the apps' modes, at `y`. Returns the
    /// bottom.
    #[allow(clippy::too_many_arguments)]
    fn paint_settings(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        body: Rect,
        y: f32,
        input: &InputState,
        clicked: bool,
        pending: &mut Vec<SystemAction>,
    ) -> f32 {
        let config = ctx.notifications.clone();
        let gap = ctx.metrics.page.grid_gap;
        let y = layout::section(ctx, list, body, y, "Settings");

        let caption = if config.dnd_schedule {
            format!("On from {} to {} every day; a toggle by hand lasts until the next change", config.dnd_start, config.dnd_end)
        } else {
            "Turns Do Not Disturb on and off at set times".to_string()
        };
        let rows: &[&str] = if config.dnd_schedule { &["Scheduled", "Starts at", "Ends at"] } else { &["Scheduled"] };
        let mut card = SettingsCard::begin(ctx, list, body, y, ("Do Not Disturb schedule", &caption), rows);
        if let Some(on) = card.switch(ctx, list, "Scheduled", None, config.dnd_schedule, input, clicked) {
            pending.push(SystemAction::SetSetting { path: "notifications.dnd_schedule", value: toml::Value::Boolean(on) });
        }
        if config.dnd_schedule {
            for (label, path, current, drag) in [
                ("Starts at", "notifications.dnd_start", &config.dnd_start, &mut self.start_drag),
                ("Ends at", "notifications.dnd_end", &config.dnd_end, &mut self.end_drag),
            ] {
                let choices = half_hours();
                let current = crate::config::parse_time(current).unwrap_or(0);
                let (values, index) = with_current(&choices, current);
                let labels: Vec<String> = values.iter().map(|m| crate::night_light::format_time(*m)).collect();
                if let Some(i) = card.steps(ctx, list, label, &labels, index, drag, input, clicked) {
                    pending.push(SystemAction::SetSetting { path, value: toml::Value::String(labels[i].clone()) });
                }
            }
        }
        let y = card.bottom() + gap;

        let apps: Vec<String> = config.apps.keys().cloned().chain(self.seen.iter().cloned()).collect::<BTreeSet<_>>().into_iter().collect();
        let labels: Vec<&str> = if apps.is_empty() { vec!["Apps"] } else { apps.iter().map(String::as_str).collect() };
        let mut card = SettingsCard::begin(
            ctx,
            list,
            body,
            y,
            ("Apps", "Silent keeps them in the list without a toast; Off drops them"),
            &labels,
        );
        if apps.is_empty() {
            card.note(ctx, list, "Apps", "Apps appear here once they notify", ctx.theme.text_muted);
        }
        let names: Vec<String> = MODES.iter().map(|(_, name)| name.to_string()).collect();
        for app in &apps {
            let mode = config.mode(app);
            let selected = MODES.iter().position(|(m, _)| *m == mode).unwrap_or(0);
            if let Some(i) = card.chips(ctx, list, app, &names, selected, input, clicked) {
                // Toast is the default, so it is left unnamed.
                let mut table = toml::Table::new();
                for (name, mode) in &config.apps {
                    if name != app {
                        table.insert(name.clone(), toml::Value::try_from(mode).unwrap_or(toml::Value::String("toast".into())));
                    }
                }
                if MODES[i].0 != AppMode::Toast {
                    table.insert(app.clone(), toml::Value::try_from(MODES[i].0).unwrap_or(toml::Value::String("toast".into())));
                }
                pending.push(SystemAction::SetSetting { path: "notifications.apps", value: toml::Value::Table(table) });
            }
        }
        card.bottom()
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
        let m = ctx.metrics.page.clone();
        let notifications = ctx.state.notifications.get();
        let dnd = ctx.state.dnd.get();
        self.seen.extend(notifications.iter().map(|n| n.app_name.clone()).filter(|a| !a.is_empty()));

        let area = layout::column(area, m.max_width);
        let count = match notifications.len() {
            0 => "Nothing new".to_string(),
            1 => "1 notification".to_string(),
            n => format!("{n} notifications"),
        };
        let summary = if dnd { format!("{count} \u{b7} Do Not Disturb is on") } else { count };

        // The header's buttons sit at its right, so its text stops short of
        // them.
        let clear_w = if notifications.is_empty() { 0.0 } else { layout::text_button_width(ctx, CLEAR) + m.grid_gap };
        let buttons_w = m.badge + clear_w;
        let (head, body) = layout::header(
            ctx,
            list,
            Rect {
                width: area.width - buttons_w - m.grid_gap,
                ..area
            },
            "Notifications",
            &summary,
        );
        let body = Rect { width: area.width, ..body };
        let button_y = head.y + (head.height - m.badge) * 0.5;
        let mut x = area.x + area.width - buttons_w;
        if !notifications.is_empty() {
            if layout::text_button(ctx, list, x, button_y, CLEAR, true, input, clicked) {
                pending.push(SystemAction::DismissAll);
            }
            x += clear_w;
        }
        let glyph = if dnd { BELL_OFF } else { BELL };
        if layout::icon_button(ctx, list, x, button_y, glyph, dnd, ctx.theme.primary, input, clicked) {
            ctx.state.dnd.set(!dnd);
            ctx.state.notify();
        }

        if notifications.is_empty() {
            let bottom = paint_empty(ctx, list, body, dnd);
            return self.paint_settings(ctx, list, body, bottom + m.grid_gap, input, clicked, pending);
        }

        let mut y = body.y;
        let pointer = input.pointer.map(|(x, y)| Point::new(x, y));
        for notification in notifications.iter().rev() {
            let layout = notification_card::layout(ctx, notification, Point::new(body.x, y), body.width, ReplyMode::Preview);
            let hovered = pointer.is_some_and(|p| layout.card.contains(p));
            notification_card::paint(
                ctx,
                list,
                notification,
                &layout,
                CardPaint {
                    alpha: 1.0,
                    hovered,
                    reply: None,
                },
            );
            if clicked {
                match pointer.and_then(|p| layout.hit(p)) {
                    Some(CardHit::Dismiss) => pending.push(SystemAction::Dismiss(notification.id)),
                    Some(CardHit::Action(key)) => pending.push(SystemAction::InvokeNotification(notification.id, key)),
                    Some(CardHit::Body) if notification.has_default_action() => {
                        pending.push(SystemAction::InvokeNotification(notification.id, "default".into()))
                    }
                    _ => {}
                }
            }
            y += layout.card.height + m.grid_gap;
        }
        self.paint_settings(ctx, list, body, y, input, clicked, pending)
    }
}

impl Default for NotificationsPage {
    fn default() -> Self {
        Self::new()
    }
}

/// A card saying there is nothing here. Returns its bottom.
fn paint_empty(ctx: &mut PaintCtx<'_>, list: &mut DisplayList, body: Rect, dnd: bool) -> f32 {
    let m = ctx.metrics.page.clone();
    let typo = ctx.metrics.typography.clone();
    let theme = ctx.theme.clone();
    let card = rect(body.x, body.y, body.width, m.empty_height);
    layout::card(ctx, list, card, Card::Plain);

    let title_h = typo.line(typo.body);
    let caption_h = typo.line(typo.caption);
    let content_h = m.badge + m.card_padding + title_h + caption_h;
    let top = card.y + (card.height - content_h) * 0.5;
    let badge_bg = layout::mix(theme.surface_high, theme.primary, 0.2);
    let glyph = if dnd { BELL_OFF } else { BELL };
    layout::badge(ctx, list, card.x + (card.width - m.badge) * 0.5, top, glyph, theme.primary, badge_bg);

    let title_y = top + m.badge + m.card_padding;
    let caption = if dnd {
        "Do Not Disturb is on: new ones arrive here without a toast"
    } else {
        "New notifications will appear here"
    };
    let inner = card.width - m.card_padding * 2.0;
    ui::paint_text_centered_weighted(
        ctx,
        list,
        "You're all caught up",
        rect(card.x, title_y, card.width, title_h),
        typo.body,
        typo.weight_bold,
        theme.text_main,
    );
    let caption = ui::elide(ctx, caption, typo.caption, typo.weight_regular, inner);
    ui::paint_text_centered_weighted(
        ctx,
        list,
        &caption,
        rect(card.x, title_y + title_h, card.width, caption_h),
        typo.caption,
        typo.weight_regular,
        theme.text_muted,
    );
    card.y + card.height
}
