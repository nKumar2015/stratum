//! Notifications page: the live notification list.

use crate::actions::SystemAction;
use crate::render::display_list::{Color, DisplayList, Point, Rect};
use crate::ui::controls;
use crate::ui::notification_card::{self, CardHit, CardPaint, ReplyMode};
use crate::ui::pages::heading;
use crate::ui::{self, InputState, PaintCtx};

pub struct NotificationsPage;

impl NotificationsPage {
    pub fn new() -> Self {
        Self
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
        let area = heading(ctx, list, area, "Notifications");
        let notifications = ctx.state.notifications.get();
        if notifications.is_empty() {
            ui::paint_text_vcentered(
                ctx,
                list,
                "No notifications",
                area.x,
                Rect {
                    height: 24.0,
                    ..area
                },
                ctx.metrics.typography.body,
ctx.metrics.typography.weight_regular,
                ctx.theme.text_muted,
            );
            return;
        }

        let clear = Rect {
            x: area.x,
            y: area.y,
            width: ctx.metrics.page.button_width - 20.0,
            height: ctx.metrics.page.button_height,
            radius: ctx.metrics.page.button_radius,
            color: Color::TRANSPARENT,
        };
        if controls::button(ctx, list, clear, "Clear all", input, clicked, false) {
            pending.push(SystemAction::DismissAll);
        }

        // Newest first, as many as fit. Cards are the toasts' cards, showing
        // the last reply instead of a reply field, as the QML centre did.
        // Cards are stacked with the toast stack's gap; `page.row_spacing` is
        // a list row's pitch, not a gap.
        let gap = ctx.metrics.toasts.gap;
        let mut y = clear.y + clear.height + gap * 2.0;
        let pointer = input.pointer.map(|(x, y)| Point::new(x, y));
        for notification in notifications.iter().rev() {
            let layout = notification_card::layout(
                ctx,
                notification,
                Point::new(area.x, y),
                area.width,
                ReplyMode::Preview,
            );
            if layout.card.y + layout.card.height > area.y + area.height {
                break;
            }
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
                let hit = pointer.and_then(|p| layout.hit(p));
                match hit {
                    Some(CardHit::Dismiss) => pending.push(SystemAction::Dismiss(notification.id)),
                    Some(CardHit::Action(key)) => {
                        pending.push(SystemAction::InvokeNotification(notification.id, key))
                    }
                    Some(CardHit::Body) if notification.has_default_action() => pending.push(
                        SystemAction::InvokeNotification(notification.id, "default".into()),
                    ),
                    _ => {}
                }
            }
            y += layout.card.height + gap;
        }
    }
}

impl Default for NotificationsPage {
    fn default() -> Self {
        Self::new()
    }
}
