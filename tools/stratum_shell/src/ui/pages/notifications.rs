//! Notifications page: the live notification list.
//!
//! The header holds Do Not Disturb and "Clear all". Cards are the toasts'
//! cards, newest first, showing the last reply instead of a reply field as
//! the QML centre did. The whole list is shown and the page scrolls.

use crate::actions::SystemAction;
use crate::render::display_list::{DisplayList, Point, Rect};
use crate::ui::notification_card::{self, CardHit, CardPaint, ReplyMode};
use crate::ui::pages::layout::{self, rect, Card};
use crate::ui::{self, InputState, PaintCtx};

const BELL: &str = "\u{f009a}";
const BELL_OFF: &str = "\u{f009b}";
const CLEAR: &str = "Clear all";

pub struct NotificationsPage;

impl NotificationsPage {
    pub fn new() -> Self {
        Self
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
            if layout::text_button(ctx, list, x, button_y, CLEAR, input, clicked) {
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
            return paint_empty(ctx, list, body, dnd);
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
        y - m.grid_gap
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

    let title_h = typo.body * 1.4;
    let caption_h = typo.caption * 1.4;
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
