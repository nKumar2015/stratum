//! Notification toasts.
//!
//! A stack of transient cards in the corner, shown when notifications arrive
//! and dismissed on hover, click, or expiry. Sticky notifications (critical
//! urgency, or an explicit "never expire") stay until dismissed.
//!
//! Toasts live on their own overlay-layer surface per output, sharing the OSD's
//! surface pattern: no exclusive zone and no keyboard input.

use std::collections::HashMap;

use crate::notifications::Notification;
use crate::render::display_list::{Color, DisplayList, Rect};
use crate::ui::animation::Tween;
use crate::ui::{self, PaintCtx};

/// One toast on screen.
struct Toast {
    notification: Notification,
    /// Seconds until auto-dismiss. `None` means sticky.
    remaining: Option<f32>,
    /// Slide/fade progress.
    reveal: Tween,
}

impl Toast {
    fn sticky(notification: &Notification) -> bool {
        // Critical urgency and an explicit zero timeout both mean "stay until
        // acknowledged", matching the QML toast.
        notification.urgency >= 2 || notification.expire_timeout == 0
    }
}

/// The toast stack.
pub struct Toasts {
    entries: Vec<Toast>,
    /// Ids already dismissed, so a re-publish does not resurrect them.
    dismissed: HashMap<u32, ()>,
    /// Seconds a normal notification stays on screen when the server does not
    /// specify a timeout.
    default_seconds: f32,
}

impl Toasts {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            dismissed: HashMap::new(),
            default_seconds: 5.0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Reconcile the stack with the server's current list.
    ///
    /// New notifications are added (unless previously dismissed) and removed
    /// notifications are dropped, so the toasts and the notifications page never
    /// disagree.
    pub fn sync(&mut self, list: &[Notification]) {
        let live: Vec<u32> = list.iter().map(|n| n.id).collect();
        self.entries.retain(|toast| live.contains(&toast.notification.id));

        for notification in list {
            if self.entries.iter().any(|t| t.notification.id == notification.id) {
                // Update the content in place (a replaces_id refresh).
                if let Some(existing) = self
                    .entries
                    .iter_mut()
                    .find(|t| t.notification.id == notification.id)
                {
                    existing.notification = notification.clone();
                }
                continue;
            }
            if self.dismissed.contains_key(&notification.id) {
                continue;
            }

            let seconds = if Toast::sticky(notification) {
                None
            } else if notification.expire_timeout > 0 {
                Some(notification.expire_timeout as f32 / 1000.0)
            } else {
                Some(self.default_seconds)
            };

            let mut reveal = Tween::new(0.0);
            reveal.to(1.0, 0.18, ui::animation::Easing::OutBack);
            self.entries.push(Toast {
                notification: notification.clone(),
                remaining: seconds,
                reveal,
            });

            // Bound the stack; the page still shows everything.
            const MAX_TOASTS: usize = 5;
            while self.entries.len() > MAX_TOASTS {
                let removed = self.entries.remove(0);
                self.dismissed.insert(removed.notification.id, ());
            }
        }
    }

    /// Advance timers. Returns ids that expired this tick.
    pub fn tick(&mut self, dt: f32) -> Vec<u32> {
        let mut expired = Vec::new();
        for toast in &mut self.entries {
            toast.reveal.tick(dt);
            if let Some(remaining) = &mut toast.remaining {
                *remaining -= dt;
                if *remaining <= 0.0 {
                    expired.push(toast.notification.id);
                }
            }
        }
        for id in &expired {
            self.entries.retain(|t| t.notification.id != *id);
            self.dismissed.insert(*id, ());
        }
        expired
    }

    /// True while any toast is animating in.
    pub fn animating(&self) -> bool {
        self.entries.iter().any(|t| t.reveal.is_animating())
    }

    /// Mark an id dismissed so it does not reappear.
    pub fn dismiss(&mut self, id: u32) {
        self.entries.retain(|t| t.notification.id != id);
        self.dismissed.insert(id, ());
    }

    /// Paint the stack anchored to the top-right of `surface`.
    ///
    /// Returns the rects and ids for hit-testing.
    pub fn paint(
        &self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        surface: Rect,
    ) -> Vec<(Rect, u32)> {
        let width = 320.0_f32.min(surface.width - 24.0);
        let mut y = surface.y + 12.0;
        let mut hits = Vec::new();

        for toast in &self.entries {
            let reveal = toast.reveal.value().clamp(0.0, 1.2);
            let alpha = reveal.min(1.0);

            // Measure the body by wrapping roughly: two lines at most.
            let body_lines = if toast.notification.body.is_empty() { 0 } else { 2 };
            let height = 52.0 + body_lines as f32 * 16.0;

            // Slide in from the right.
            let offset = (1.0 - reveal).max(0.0) * 24.0;
            let card = Rect {
                x: surface.x + surface.width - width - 12.0 + offset,
                y,
                width,
                height,
                radius: 10.0,
                color: Color::TRANSPARENT,
            };

            let fill = Color::rgba(
                ctx.theme.surface_high.r,
                ctx.theme.surface_high.g,
                ctx.theme.surface_high.b,
                0.98 * alpha,
            );
            ui::paint_frame(
                list,
                card,
                fill,
                ctx.theme.outline.with_alpha(alpha),
                ctx.appearance.framed_border,
            );

            // Urgency stripe.
            let accent = match toast.notification.urgency {
                2 => ctx.theme.error,
                0 => ctx.theme.text_muted,
                _ => ctx.theme.primary,
            }
            .with_alpha(alpha);
            list.push_rect(Rect {
                x: card.x,
                y: card.y,
                width: 3.0,
                height: card.height,
                radius: 0.0,
                color: accent,
            });

            let text = ctx.theme.text_main.with_alpha(alpha);
            let muted = ctx.theme.text_muted.with_alpha(alpha);

            // App name, then summary.
            if !toast.notification.app_name.is_empty() {
                ui::paint_text_vcentered(
                    ctx,
                    list,
                    &toast.notification.app_name,
                    card.x + 14.0,
                    Rect {
                        height: 20.0,
                        ..card
                    },
                    10.0,
                    600,
                    muted,
                );
            }
            ui::paint_text_vcentered(
                ctx,
                list,
                &toast.notification.summary,
                card.x + 14.0,
                Rect {
                    y: card.y + 6.0,
                    height: 26.0,
                    width: card.width - 60.0,
                    ..card
                },
                13.0,
                600,
                text,
            );

            if !toast.notification.body.is_empty() {
                let body = truncate(&toast.notification.body, 90);
                ui::paint_text_vcentered(
                    ctx,
                    list,
                    &body,
                    card.x + 14.0,
                    Rect {
                        y: card.y + 30.0,
                        height: 32.0,
                        width: card.width - 28.0,
                        ..card
                    },
                    11.0,
                    400,
                    muted,
                );
            }

            // Dismiss affordance.
            ui::paint_text_vcentered(
                ctx,
                list,
                "\u{f0156}",
                card.x + card.width - 24.0,
                Rect {
                    height: 24.0,
                    ..card
                },
                12.0,
                400,
                muted,
            );

            hits.push((card, toast.notification.id));
            y += height + 8.0;
        }
        hits
    }
}

impl Default for Toasts {
    fn default() -> Self {
        Self::new()
    }
}

/// Trim a string to roughly `max` characters, adding an ellipsis.
fn truncate(value: &str, max: usize) -> String {
    let mut out = String::with_capacity(max.min(value.len()) + 1);
    for (count, ch) in value.chars().enumerate() {
        if count >= max {
            out.push('\u{2026}');
            break;
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notification(id: u32, urgency: u8, timeout: i32) -> Notification {
        Notification {
            id,
            app_name: "app".into(),
            app_icon: String::new(),
            summary: format!("summary {id}"),
            body: "body".into(),
            actions: Vec::new(),
            urgency,
            expire_timeout: timeout,
            image_path: None,
        }
    }

    #[test]
    fn new_notifications_become_toasts() {
        let mut toasts = Toasts::new();
        toasts.sync(&[notification(1, 1, 3000)]);
        assert_eq!(toasts.entries.len(), 1);
    }

    #[test]
    fn critical_notifications_are_sticky() {
        let mut toasts = Toasts::new();
        toasts.sync(&[notification(1, 2, 3000)]);
        assert!(toasts.entries[0].remaining.is_none());

        let mut toasts = Toasts::new();
        toasts.sync(&[notification(2, 1, 0)]);
        assert!(toasts.entries[0].remaining.is_none());
    }

    #[test]
    fn expiry_removes_and_does_not_resurrect() {
        let mut toasts = Toasts::new();
        toasts.sync(&[notification(1, 1, 1000)]);
        // Advance past the timeout.
        let expired = toasts.tick(1.5);
        assert_eq!(expired, vec![1]);
        assert!(toasts.is_empty());

        // The server still lists it; it must not come back.
        toasts.sync(&[notification(1, 1, 1000)]);
        assert!(toasts.is_empty());
    }

    #[test]
    fn dismiss_is_final() {
        let mut toasts = Toasts::new();
        toasts.sync(&[notification(1, 1, 5000)]);
        toasts.dismiss(1);
        assert!(toasts.is_empty());
        toasts.sync(&[notification(1, 1, 5000)]);
        assert!(toasts.is_empty());
    }

    #[test]
    fn stack_is_bounded() {
        let mut toasts = Toasts::new();
        let list: Vec<_> = (1..=8).map(|id| notification(id, 1, 5000)).collect();
        toasts.sync(&list);
        assert!(toasts.entries.len() <= 5);
    }

    #[test]
    fn truncate_adds_ellipsis() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 5), "hello\u{2026}");
    }
}
