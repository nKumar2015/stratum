//! Notifications page: the live notification list.

use crate::actions::SystemAction;
use crate::render::display_list::{Color, DisplayList, Rect};
use crate::ui::controls;
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
                13.0,
                400,
                ctx.theme.text_muted,
            );
            return;
        }

        let clear = Rect {
            x: area.x,
            y: area.y,
            width: 120.0,
            height: 30.0,
            radius: 8.0,
            color: Color::TRANSPARENT,
        };
        if controls::button(ctx, list, clear, "Clear all", input, clicked, false) {
            pending.push(SystemAction::DismissAll);
        }

        let mut y = area.y + 42.0;
        for notification in notifications.iter().rev().take(8) {
            let card = Rect {
                x: area.x,
                y,
                width: area.width,
                height: 66.0,
                radius: 10.0,
                color: Color::TRANSPARENT,
            };
            ui::paint_frame(
                list,
                card,
                ctx.theme.surface_high,
                ctx.theme.outline,
                ctx.appearance.framed_border,
            );
            list.push_rect(Rect {
                x: card.x,
                y: card.y,
                width: 3.0,
                height: card.height,
                radius: 0.0,
                color: match notification.urgency {
                    2 => ctx.theme.error,
                    0 => ctx.theme.text_muted,
                    _ => ctx.theme.primary,
                },
            });

            ui::paint_text_vcentered(
                ctx,
                list,
                &notification.summary,
                card.x + 14.0,
                Rect {
                    height: 26.0,
                    ..card
                },
                13.0,
                600,
                ctx.theme.text_main,
            );
            let body = if notification.app_name.is_empty() {
                notification.body.clone()
            } else {
                format!("{} · {}", notification.app_name, notification.body)
            };
            ui::paint_text_vcentered(
                ctx,
                list,
                &body,
                card.x + 14.0,
                Rect {
                    y: card.y + 24.0,
                    height: 20.0,
                    width: card.width - 90.0,
                    ..card
                },
                11.0,
                400,
                ctx.theme.text_muted,
            );

            let dismiss = Rect {
                x: card.x + card.width - 34.0,
                y: card.y + (card.height - 22.0) * 0.5,
                width: 22.0,
                height: 22.0,
                radius: 11.0,
                color: Color::TRANSPARENT,
            };
            if controls::button(ctx, list, dismiss, "x", input, clicked, false) {
                pending.push(SystemAction::Dismiss(notification.id));
            }
            y += 72.0;
        }
    }
}

impl Default for NotificationsPage {
    fn default() -> Self {
        Self::new()
    }
}
