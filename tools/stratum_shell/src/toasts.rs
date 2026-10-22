//! Notification toasts.
//!
//! A stack of transient cards in the corner, shown when notifications arrive
//! and removed on dismissal or expiry. Sticky notifications (critical urgency,
//! or an explicit "never expire") stay until dismissed. As in the QML toast, a
//! card does not expire while the pointer is over it or its reply field has
//! focus.
//!
//! Cards are drawn by `ui::notification_card`: clicking the card runs the
//! notification's default action (or dismisses it when it has none), buttons
//! run the other actions, and notifications that accept one get an inline
//! reply field. Typing needs keyboard focus, so the toast surface takes
//! exclusive focus only while a reply field is focused.
//!
//! Toasts live on their own overlay-layer surface per output. Every output's
//! stack has the same width and anchor, so one set of card layouts serves all.

use std::collections::HashMap;

use crate::notifications::Notification;
use crate::render::display_list::{DisplayList, Point, Rect};
use crate::ui::animation::Tween;
use crate::ui::notification_card::{self, CardHit, CardLayout, CardPaint, ReplyMode};
use crate::ui::text_field::{FieldKey, TextField};
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

/// What a click or key on the stack asks the caller to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToastAction {
    None,
    /// The stack changed; repaint.
    Redraw,
    /// The user closed the notification.
    Dismiss(u32),
    /// Run an action (`default` for a click on the card).
    Invoke(u32, String),
    /// Send an inline reply.
    Reply(u32, String),
}

/// The focused reply field and the card it belongs to.
struct ReplyFocus {
    id: u32,
    /// Output whose surface takes keyboard focus while typing.
    output: String,
    field: TextField,
}

/// The toast stack.
pub struct Toasts {
    entries: Vec<Toast>,
    /// Ids already dismissed, so a re-publish does not resurrect them.
    dismissed: HashMap<u32, ()>,
    /// Sizes, timings, and limits, refreshed from the config each frame.
    metrics: crate::ui::style::ToastMetrics,
    /// Do-not-disturb: keep the stack consistent, but do not add cards.
    muted: bool,
    /// Card layouts from the last paint, for hit-testing and hover.
    layouts: Vec<(u32, CardLayout)>,
    /// The card under the pointer, which does not expire.
    hovered: Option<u32>,
    reply: Option<ReplyFocus>,
}

impl Toasts {
    /// Adopt the configured sizes and timing.
    pub fn set_metrics(&mut self, metrics: &crate::ui::style::ToastMetrics) {
        self.metrics = metrics.clone();
    }

    /// Enable or disable do-not-disturb. Turning it on does not remove cards
    /// already on screen; turning it off lets the next sync adopt anything that
    /// arrived in the meantime, since `sync` sees the whole live list.
    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            dismissed: HashMap::new(),
            metrics: crate::ui::style::ToastMetrics::default(),
            muted: false,
            layouts: Vec::new(),
            hovered: None,
            reply: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The output whose toast surface should hold keyboard focus: the one
    /// whose reply field is being typed into.
    pub fn keyboard_output(&self) -> Option<&str> {
        self.reply.as_ref().map(|r| r.output.as_str())
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
            if let Some(existing) = self
                .entries
                .iter_mut()
                .find(|t| t.notification.id == notification.id)
            {
                // Update the content in place (a replaces_id refresh).
                existing.notification = notification.clone();
                continue;
            }
            if self.dismissed.contains_key(&notification.id) {
                continue;
            }
            if self.muted {
                // Do-not-disturb: the notification still reaches the page; it
                // just does not interrupt. It is not marked dismissed, so
                // turning DND off brings it back only if it is still live.
                continue;
            }

            let seconds = if Toast::sticky(notification) {
                None
            } else if notification.expire_timeout > 0 {
                Some(notification.expire_timeout as f32 / 1000.0)
            } else {
                Some(self.metrics.default_seconds)
            };

            let mut reveal = Tween::new(0.0);
            reveal.to(
                1.0,
                self.metrics.reveal_seconds,
                ui::animation::Easing::OutBack,
            );
            self.entries.push(Toast {
                notification: notification.clone(),
                remaining: seconds,
                reveal,
            });

            // Bound the stack; the page still shows everything.
            while self.entries.len() > self.metrics.max_cards.max(1) {
                let removed = self.entries.remove(0);
                self.dismissed.insert(removed.notification.id, ());
            }
        }
        self.drop_stale_reply();
    }

    /// Advance timers. Returns ids that expired this tick.
    pub fn tick(&mut self, dt: f32) -> Vec<u32> {
        let mut expired = Vec::new();
        let paused = [self.hovered, self.reply.as_ref().map(|r| r.id)];
        for toast in &mut self.entries {
            toast.reveal.tick(dt);
            if paused.contains(&Some(toast.notification.id)) {
                continue;
            }
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
        self.drop_stale_reply();
    }

    fn drop_stale_reply(&mut self) {
        if let Some(reply) = &self.reply {
            if !self.entries.iter().any(|t| t.notification.id == reply.id) {
                self.reply = None;
            }
        }
    }

    /// Track the pointer over the stack (`None` when it is elsewhere).
    /// Returns true when the hovered card changed.
    pub fn set_pointer(&mut self, point: Option<Point>) -> bool {
        let hovered = point.and_then(|p| {
            self.layouts
                .iter()
                .find(|(_, layout)| layout.card.contains(p))
                .map(|(id, _)| *id)
        });
        let changed = hovered != self.hovered;
        self.hovered = hovered;
        changed
    }

    /// Resolve a click on the toast surface of `output`.
    pub fn on_click(&mut self, point: Point, output: &str) -> ToastAction {
        let Some((id, hit)) = self
            .layouts
            .iter()
            .find_map(|(id, layout)| layout.hit(point).map(|hit| (*id, hit)))
        else {
            return ToastAction::None;
        };
        let Some(notification) = self
            .entries
            .iter()
            .find(|t| t.notification.id == id)
            .map(|t| t.notification.clone())
        else {
            return ToastAction::None;
        };
        // Any click outside the focused field gives up keyboard focus.
        if hit != CardHit::ReplyField && hit != CardHit::Send {
            self.reply = None;
        }
        match hit {
            CardHit::Dismiss => {
                self.dismiss(id);
                ToastAction::Dismiss(id)
            }
            CardHit::Action(key) => ToastAction::Invoke(id, key),
            CardHit::Body if notification.has_default_action() => {
                ToastAction::Invoke(id, "default".to_string())
            }
            // Without a default action a click dismisses, as it always has.
            CardHit::Body => {
                self.dismiss(id);
                ToastAction::Dismiss(id)
            }
            CardHit::ReplyField => {
                if self.reply.as_ref().map(|r| r.id) != Some(id) {
                    self.reply = Some(ReplyFocus {
                        id,
                        output: output.to_string(),
                        field: TextField::new(),
                    });
                }
                ToastAction::Redraw
            }
            CardHit::Send => self.send(id),
        }
    }

    fn send(&mut self, id: u32) -> ToastAction {
        let Some(reply) = self.reply.as_mut().filter(|r| r.id == id) else {
            return ToastAction::None;
        };
        let Some(text) = reply.field.take() else {
            return ToastAction::None;
        };
        let text = text.expose().trim().to_string();
        self.reply = None;
        if text.is_empty() {
            return ToastAction::Redraw;
        }
        ToastAction::Reply(id, text)
    }

    /// Whether keys should come here: a reply field has focus.
    pub fn wants_keys(&self) -> bool {
        self.reply.is_some()
    }

    pub fn key(
        &mut self,
        keysym: smithay_client_toolkit::seat::keyboard::Keysym,
        text: Option<&str>,
        ctrl: bool,
    ) -> ToastAction {
        let Some(reply) = self.reply.as_mut() else {
            return ToastAction::None;
        };
        match reply.field.key(keysym, text, ctrl) {
            FieldKey::Ignored => ToastAction::None,
            FieldKey::Changed => ToastAction::Redraw,
            FieldKey::Submit(text) => {
                let id = reply.id;
                let text = text.expose().trim().to_string();
                self.reply = None;
                if text.is_empty() {
                    ToastAction::Redraw
                } else {
                    ToastAction::Reply(id, text)
                }
            }
            FieldKey::Escape => {
                self.reply = None;
                ToastAction::Redraw
            }
        }
    }

    /// Paint the stack anchored to the top-right of `surface`. The card
    /// layouts are kept for hit-testing; the union of the cards is returned
    /// for the surface's input region.
    pub fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        surface: Rect,
    ) -> Option<Rect> {
        let m = self.metrics.clone();
        let width = m.card_width.min(surface.width - m.margin as f32 * 2.0);
        let mut y = surface.y + m.margin as f32;
        let mut layouts = Vec::with_capacity(self.entries.len());
        let mut bounds: Option<Rect> = None;

        for toast in &self.entries {
            let reveal = toast.reveal.value().clamp(0.0, 1.2);
            // Slide in from the right.
            let offset = (1.0 - reveal).max(0.0) * m.slide_distance;
            let x = surface.x + surface.width - width - m.margin as f32 + offset;
            let id = toast.notification.id;
            let layout = notification_card::layout(ctx, &toast.notification, Point::new(x, y), width, ReplyMode::Field);
            let reply = self.reply.as_ref().filter(|r| r.id == id).map(|r| &r.field);
            notification_card::paint(
                ctx,
                list,
                &toast.notification,
                &layout,
                CardPaint {
                    alpha: reveal.min(1.0),
                    hovered: self.hovered == Some(id),
                    reply,
                },
            );
            y += layout.card.height + m.gap;
            bounds = Some(match bounds {
                None => layout.card,
                Some(acc) => crate::render::display_list::union(acc, layout.card),
            });
            layouts.push((id, layout));
        }
        self.layouts = layouts;
        bounds
    }
}

impl Default for Toasts {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::display_list::Color;
    use smithay_client_toolkit::seat::keyboard::Keysym;

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
            ..Default::default()
        }
    }

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect {
            x,
            y,
            width: w,
            height: h,
            radius: 0.0,
            color: Color::TRANSPARENT,
        }
    }

    /// A stack holding one card with a fixed layout, so clicks can be tested
    /// without fonts: card 300x140, an action, and a reply row.
    fn stack_with(n: Notification) -> Toasts {
        let mut toasts = Toasts::new();
        toasts.sync(std::slice::from_ref(&n));
        let layout = crate::ui::notification_card::test_layout(
            rect(0.0, 0.0, 300.0, 140.0),
            rect(270.0, 5.0, 22.0, 22.0),
            vec![(rect(14.0, 70.0, 74.0, 26.0), "mark".into(), "Mark".into())],
            Some(rect(14.0, 104.0, 200.0, 30.0)),
            Some(rect(220.0, 104.0, 52.0, 30.0)),
        );
        toasts.layouts = vec![(n.id, layout)];
        toasts
    }

    fn type_text(toasts: &mut Toasts, text: &str) {
        for ch in text.chars() {
            toasts.key(Keysym::a, Some(&ch.to_string()), false);
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
    fn hovering_pauses_expiry() {
        let mut toasts = stack_with(notification(1, 1, 1000));
        assert!(toasts.set_pointer(Some(Point::new(150.0, 40.0))));
        assert!(toasts.tick(5.0).is_empty(), "a hovered card does not expire");
        toasts.set_pointer(None);
        assert_eq!(toasts.tick(1.5), vec![1]);
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
    fn do_not_disturb_suppresses_new_cards() {
        let mut toasts = Toasts::new();
        toasts.set_muted(true);
        toasts.sync(&[notification(1, 1, 5000)]);
        assert!(toasts.is_empty());

        // Turning it off adopts what arrived while it was on.
        toasts.set_muted(false);
        toasts.sync(&[notification(1, 1, 5000)]);
        assert_eq!(toasts.entries.len(), 1);
    }

    #[test]
    fn clicks_run_actions_or_dismiss() {
        let mut with_default = notification(1, 1, 5000);
        with_default.actions = vec!["default".into(), "Open".into(), "mark".into(), "Mark".into()];
        let mut toasts = stack_with(with_default);
        assert_eq!(toasts.on_click(Point::new(20.0, 80.0), "a"), ToastAction::Invoke(1, "mark".into()));
        assert_eq!(toasts.on_click(Point::new(150.0, 40.0), "a"), ToastAction::Invoke(1, "default".into()));
        assert_eq!(toasts.on_click(Point::new(280.0, 10.0), "a"), ToastAction::Dismiss(1));
        assert!(toasts.is_empty());

        // No default action: the body click dismisses.
        let mut toasts = stack_with(notification(2, 1, 5000));
        assert_eq!(toasts.on_click(Point::new(150.0, 40.0), "a"), ToastAction::Dismiss(2));
    }

    #[test]
    fn reply_takes_focus_types_and_sends() {
        let mut n = notification(1, 1, 1000);
        n.reply_placeholder = Some("Reply...".into());
        let mut toasts = stack_with(n);
        assert!(!toasts.wants_keys());
        assert_eq!(toasts.on_click(Point::new(20.0, 110.0), "HDMI-A-1"), ToastAction::Redraw);
        assert!(toasts.wants_keys());
        assert_eq!(toasts.keyboard_output(), Some("HDMI-A-1"));
        assert!(toasts.tick(5.0).is_empty(), "a card being replied to does not expire");

        type_text(&mut toasts, " on my way ");
        assert_eq!(toasts.key(Keysym::Return, None, false), ToastAction::Reply(1, "on my way".into()));
        assert!(!toasts.wants_keys(), "sending gives the keyboard back");
    }

    #[test]
    fn send_button_and_escape() {
        let mut n = notification(1, 1, 5000);
        n.reply_placeholder = Some("Reply...".into());
        let mut toasts = stack_with(n);
        // Send with nothing typed (and no focus) does nothing.
        assert_eq!(toasts.on_click(Point::new(230.0, 110.0), "a"), ToastAction::None);
        toasts.on_click(Point::new(20.0, 110.0), "a");
        type_text(&mut toasts, "ok");
        assert_eq!(toasts.on_click(Point::new(230.0, 110.0), "a"), ToastAction::Reply(1, "ok".into()));

        toasts.on_click(Point::new(20.0, 110.0), "a");
        assert_eq!(toasts.key(Keysym::Escape, None, false), ToastAction::Redraw);
        assert!(!toasts.wants_keys());
    }

    #[test]
    fn focus_is_dropped_with_its_card() {
        let mut n = notification(1, 1, 5000);
        n.reply_placeholder = Some("Reply...".into());
        let mut toasts = stack_with(n);
        toasts.on_click(Point::new(20.0, 110.0), "a");
        toasts.sync(&[]);
        assert!(!toasts.wants_keys(), "a closed notification releases the keyboard");
    }
}
