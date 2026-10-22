//! One notification drawn as a card, shared by the toasts and the
//! notifications page so both show the same content and actions.
//!
//! Matches the QML `NotificationToast`: app name and a close button, the
//! summary, a wrapped body, one button per action (the `default` action is a
//! click on the card itself), and, when the notification accepts one, an
//! inline reply field with a Send button. The page shows the last reply sent
//! instead of the field, as the QML centre did.
//!
//! `layout` decides every rect; `paint` draws from it and returns the same
//! rects for hit-testing, so what is drawn and what is clickable cannot drift.

use crate::notifications::Notification;
use crate::render::display_list::{Color, DisplayList, Point, Rect};
use crate::ui::style::NotificationCardMetrics;
use crate::ui::text_field::{self, FieldStyle, TextField};
use crate::ui::{self, PaintCtx};

/// What the card offers in place of a reply field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplyMode {
    /// A live reply field (toasts).
    Field,
    /// The last reply sent, read-only (the page).
    Preview,
}

/// Where everything on a card is.
#[derive(Clone, Debug)]
pub struct CardLayout {
    pub card: Rect,
    pub dismiss: Rect,
    pub actions: Vec<(Rect, String, String)>,
    pub reply: Option<Rect>,
    pub send: Option<Rect>,
    summary_y: f32,
    body: Vec<String>,
    body_y: f32,
    preview: Option<(f32, String)>,
}

/// A click on a card, resolved to what it means.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CardHit {
    Dismiss,
    Action(String),
    /// The reply field itself: focus it.
    ReplyField,
    Send,
    /// The card's body: the default action, if any.
    Body,
}

impl CardLayout {
    pub fn hit(&self, point: Point) -> Option<CardHit> {
        if !self.card.contains(point) {
            return None;
        }
        if self.dismiss.contains(point) {
            return Some(CardHit::Dismiss);
        }
        if let Some((_, key, _)) = self.actions.iter().find(|(rect, _, _)| rect.contains(point)) {
            return Some(CardHit::Action(key.clone()));
        }
        if self.send.is_some_and(|rect| rect.contains(point)) {
            return Some(CardHit::Send);
        }
        if self.reply.is_some_and(|rect| rect.contains(point)) {
            return Some(CardHit::ReplyField);
        }
        Some(CardHit::Body)
    }
}

fn line(m: &NotificationCardMetrics, size: f32) -> f32 {
    size * m.line_height
}

/// Lay out `n` in a card of `width` whose top-left is `origin`.
pub fn layout(
    ctx: &mut PaintCtx<'_>,
    n: &Notification,
    origin: Point,
    width: f32,
    mode: ReplyMode,
) -> CardLayout {
    let m = ctx.metrics.notification_card.clone();
    let typo = ctx.metrics.typography.clone();
    let x = origin.x + m.padding;
    let inner = (width - m.padding * 2.0).max(0.0);
    let mut y = origin.y + m.padding;
    let rect = |x: f32, y: f32, w: f32, h: f32, radius: f32| Rect {
        x,
        y,
        width: w,
        height: h,
        radius,
        color: Color::TRANSPARENT,
    };

    let dismiss = rect(
        origin.x + width - m.padding * 0.5 - m.dismiss,
        origin.y + m.padding * 0.5,
        m.dismiss,
        m.dismiss,
        m.dismiss * 0.5,
    );
    // App name row, then summary.
    y += line(&m, m.app);
    let summary_y = y;
    y += line(&m, m.summary);

    let body = if n.body.trim().is_empty() {
        Vec::new()
    } else {
        ui::wrap_text(ctx, &plain(&n.body), m.body, typo.weight_regular, inner, m.body_lines)
    };
    let body_y = y;
    y += line(&m, m.body) * body.len() as f32;

    // Buttons in one row, each at least `button_min_width`; those that do not
    // fit are left out rather than drawn off the card.
    let mut actions = Vec::new();
    let buttons = n.buttons();
    if !buttons.is_empty() {
        y += m.row_gap;
        let style = crate::text::TextStyle {
            family: std::sync::Arc::from(ctx.theme.font.as_str()),
            size: m.button_text,
            weight: typo.weight_medium,
            color: ctx.theme.text_main,
        };
        let mut bx = x;
        for (key, label) in buttons {
            let w = (ctx.text.measure(&label, &style).0 + m.button_padding * 2.0).max(m.button_min_width);
            if bx + w > x + inner + 0.5 {
                break;
            }
            actions.push((rect(bx, y, w, m.button_height, m.button_radius), key, label));
            bx += w + m.button_gap;
        }
        y += m.button_height;
    }

    let mut reply = None;
    let mut send = None;
    let mut preview = None;
    match mode {
        ReplyMode::Field if n.reply_placeholder.is_some() => {
            y += m.row_gap;
            let field_w = (inner - m.send_width - m.button_gap).max(0.0);
            reply = Some(rect(x, y, field_w, m.reply_height, m.reply_radius));
            send = Some(rect(x + field_w + m.button_gap, y, m.send_width, m.reply_height, m.reply_radius));
            y += m.reply_height;
        }
        ReplyMode::Preview => {
            if let Some(text) = &n.last_reply {
                y += m.row_gap;
                preview = Some((y, format!("You: {text}")));
                y += line(&m, m.body);
            }
        }
        ReplyMode::Field => {}
    }
    y += m.padding;

    CardLayout {
        card: Rect {
            radius: ctx.metrics.toasts.radius,
            ..rect(origin.x, origin.y, width, y - origin.y, 0.0)
        },
        dismiss,
        actions,
        reply,
        send,
        summary_y,
        body,
        body_y,
        preview,
    }
}

/// How the card is drawn this frame.
pub struct CardPaint<'a> {
    /// Fade, for toasts sliding in.
    pub alpha: f32,
    pub hovered: bool,
    /// The reply field's contents, when this card's field has focus.
    pub reply: Option<&'a TextField>,
}

/// Draw a laid-out card.
pub fn paint(
    ctx: &mut PaintCtx<'_>,
    list: &mut DisplayList,
    n: &Notification,
    layout: &CardLayout,
    state: CardPaint<'_>,
) {
    let m = ctx.metrics.notification_card.clone();
    let typo = ctx.metrics.typography.clone();
    let theme = ctx.theme;
    let a = state.alpha;
    let card = layout.card;
    let x = card.x + m.padding;
    let inner = card.width - m.padding * 2.0;

    ui::paint_frame(
        list,
        card,
        theme.surface_high.with_alpha(0.98 * a),
        if state.hovered { theme.border_active } else { theme.outline }.with_alpha(a),
        ctx.appearance.framed_border,
    );
    // Urgency stripe.
    list.push_rect(Rect {
        width: ctx.metrics.toasts.stripe_width,
        radius: 0.0,
        color: match n.urgency {
            2 => theme.error,
            0 => theme.text_muted,
            _ => theme.primary,
        }
        .with_alpha(a),
        ..card
    });

    let text = theme.text_main.with_alpha(a);
    let muted = theme.text_muted.with_alpha(a);
    let row = |y: f32, h: f32, w: f32| Rect {
        x,
        y,
        width: w,
        height: h,
        radius: 0.0,
        color: Color::TRANSPARENT,
    };

    if !n.app_name.is_empty() {
        ui::paint_text_vcentered(ctx, list, &n.app_name, x, row(card.y + m.padding, line(&m, m.app), inner), m.app, typo.weight_bold, muted);
    }
    let summary = ui::wrap_text(ctx, &plain(&n.summary), m.summary, typo.weight_bold, inner - m.dismiss, 1);
    if let Some(summary) = summary.first() {
        ui::paint_text_vcentered(ctx, list, summary, x, row(layout.summary_y, line(&m, m.summary), inner), m.summary, typo.weight_bold, text);
    }
    for (i, body) in layout.body.iter().enumerate() {
        let y = layout.body_y + line(&m, m.body) * i as f32;
        ui::paint_text_vcentered(ctx, list, body, x, row(y, line(&m, m.body), inner), m.body, typo.weight_regular, muted);
    }

    ui::paint_text_centered(ctx, list, "\u{f0156}", layout.dismiss, m.dismiss_icon, muted);

    for (rect, _, label) in &layout.actions {
        // Outlined: a plain widget fill is nearly the card's own colour.
        text_field::paint_frame(
            list,
            *rect,
            m.reply_border,
            theme.border_inactive.with_alpha(a),
            theme.bg_widget.with_alpha(a),
        );
        ui::paint_text_centered_weighted(ctx, list, label, *rect, m.button_text, typo.weight_medium, text);
    }

    if let (Some(field), Some(send)) = (layout.reply, layout.send) {
        let focused = state.reply.is_some();
        text_field::paint_frame(
            list,
            field,
            m.reply_border,
            if focused { theme.border_active } else { theme.border_inactive }.with_alpha(a),
            theme.bg_widget.with_alpha(a),
        );
        let empty = TextField::new();
        let style = FieldStyle {
            text: m.reply_text,
            padding: m.reply_padding,
            caret_width: m.caret_width,
            masked: false,
            placeholder: n.reply_placeholder.as_deref(),
        };
        // An unfocused field shows its placeholder without a caret.
        state.reply.unwrap_or(&empty).paint(ctx, list, field, style, !focused);
        list.push_rect(Rect {
            color: theme.primary.with_alpha(a),
            ..send
        });
        ui::paint_text_centered_weighted(ctx, list, "Send", send, m.button_text, typo.weight_bold, theme.bg_main.with_alpha(a));
    }

    if let Some((y, preview)) = &layout.preview {
        let preview = ui::wrap_text(ctx, preview, m.body, typo.weight_regular, inner, 1);
        if let Some(preview) = preview.first() {
            ui::paint_text_vcentered(ctx, list, preview, x, row(*y, line(&m, m.body), inner), m.body, typo.weight_regular, theme.secondary.with_alpha(a));
        }
    }
}

/// Notification bodies may carry the spec's small markup subset (we advertise
/// `body-markup`). There is no rich text yet, so tags are dropped and the
/// common entities decoded rather than shown raw.
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_tag = false;
    for ch in text.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// A layout with fixed rects, for tests of the code that consumes hits.
#[cfg(test)]
pub fn test_layout(
    card: Rect,
    dismiss: Rect,
    actions: Vec<(Rect, String, String)>,
    reply: Option<Rect>,
    send: Option<Rect>,
) -> CardLayout {
    CardLayout {
        card,
        dismiss,
        actions,
        reply,
        send,
        summary_y: 0.0,
        body: Vec::new(),
        body_y: 0.0,
        preview: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markup_is_flattened() {
        assert_eq!(plain("<b>Ada</b> sent &lt;3 &amp; more"), "Ada sent <3 & more");
        assert_eq!(plain("no tags"), "no tags");
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

    #[test]
    fn hits_resolve_most_specific_first() {
        let layout = CardLayout {
            card: rect(0.0, 0.0, 300.0, 140.0),
            dismiss: rect(270.0, 5.0, 22.0, 22.0),
            actions: vec![(rect(14.0, 70.0, 74.0, 26.0), "mark".into(), "Mark".into())],
            reply: Some(rect(14.0, 104.0, 200.0, 30.0)),
            send: Some(rect(220.0, 104.0, 52.0, 30.0)),
            summary_y: 0.0,
            body: Vec::new(),
            body_y: 0.0,
            preview: None,
        };
        let at = |x, y| layout.hit(Point::new(x, y));
        assert_eq!(at(280.0, 10.0), Some(CardHit::Dismiss));
        assert_eq!(at(20.0, 80.0), Some(CardHit::Action("mark".into())));
        assert_eq!(at(20.0, 110.0), Some(CardHit::ReplyField));
        assert_eq!(at(230.0, 110.0), Some(CardHit::Send));
        assert_eq!(at(150.0, 40.0), Some(CardHit::Body));
        assert_eq!(at(400.0, 40.0), None);
    }
}
