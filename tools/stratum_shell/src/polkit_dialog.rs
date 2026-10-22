//! The polkit authentication dialog.
//!
//! When a program asks polkit for a privileged action, polkitd calls the
//! shell's authentication agent (`stratum_sys::managers::polkit`), which
//! publishes the request into `SystemState::polkit`. This module turns that
//! into a dialog matching `modules/PolkitDialog.qml`: the action, the message,
//! a password field, Cancel and Authenticate. The password is checked by
//! polkit's own setuid helper, off the UI thread; wrong passwords keep the
//! dialog open with the attempts left, and the agent gives up after three.
//!
//! Like the power menu it is a fullscreen overlay with exclusive keyboard
//! focus, mapped only on the output that was focused when the request came in.

use serde_json::Value;

use crate::pam::Secret;
use crate::render::display_list::{Color, DisplayList, Point, Rect};
use crate::ui::password_field::{FieldKey, FieldStyle, PasswordField};
use crate::ui::{self, PaintCtx};

/// One request from polkitd.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub cookie: String,
    pub action: String,
    pub message: String,
    pub user: String,
}

/// Something the caller has to do.
#[derive(Debug)]
pub enum DialogAction {
    None,
    /// The dialog changed and needs repainting.
    Redraw,
    /// Check this password for the request.
    Submit {
        cookie: String,
        user: String,
        password: Secret,
    },
    /// The user closed the dialog; tell the agent so the requester stops
    /// waiting.
    Dismiss { cookie: String },
}

/// The answer to a submitted password.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Accepted,
    Retry { remaining: u32 },
    /// No attempts left; the agent has already failed the request.
    Exhausted,
}

/// Placement of the card's interactive parts from the last paint.
#[derive(Clone, Copy, Debug)]
struct Hits {
    card: Rect,
    cancel: Rect,
    confirm: Rect,
}

pub struct PolkitDialog {
    request: Option<Request>,
    /// The last request that was closed. The published state keeps showing it
    /// as active until the agent reports the end, which must not reopen it.
    closed: Option<String>,
    field: PasswordField,
    verifying: bool,
    error: Option<String>,
    /// Output the dialog opened on.
    target: Option<String>,
    hits: Option<Hits>,
    metrics: crate::ui::style::PolkitMetrics,
}

impl PolkitDialog {
    pub fn new() -> Self {
        Self {
            request: None,
            closed: None,
            field: PasswordField::new(),
            verifying: false,
            error: None,
            target: None,
            hits: None,
            metrics: crate::ui::style::PolkitMetrics::default(),
        }
    }

    pub fn set_metrics(&mut self, metrics: &crate::ui::style::PolkitMetrics) {
        self.metrics = metrics.clone();
    }

    pub fn is_open(&self) -> bool {
        self.request.is_some()
    }

    /// The output whose surface should be mapped and focused.
    pub fn target(&self) -> Option<&str> {
        self.request.as_ref().and(self.target.as_deref())
    }

    /// Follow the agent's published state. `focused` is where a new request
    /// opens. Returns true when the dialog changed.
    pub fn sync(&mut self, payload: &Value, focused: &str) -> bool {
        let active = payload.get("active").and_then(Value::as_bool).unwrap_or(false);
        let cookie = payload
            .get("cookie")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let text = |key: &str| {
            payload
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };

        if active {
            if cookie.is_empty() || self.closed.as_deref() == Some(cookie) {
                return false;
            }
            if self.request.as_ref().map(|r| r.cookie.as_str()) == Some(cookie) {
                return false;
            }
            self.request = Some(Request {
                cookie: cookie.to_string(),
                action: text("action"),
                message: text("message"),
                user: text("user"),
            });
            self.target = Some(focused.to_string());
            self.field.clear();
            self.verifying = false;
            self.error = None;
            self.hits = None;
            return true;
        }

        // The agent finished a request: closed by polkitd, answered, or
        // exhausted. An inactive payload without a cookie closes whatever is
        // open.
        match &self.request {
            Some(request) if cookie.is_empty() || request.cookie == cookie => {
                self.close();
                true
            }
            _ => false,
        }
    }

    fn close(&mut self) {
        if let Some(request) = self.request.take() {
            self.closed = Some(request.cookie);
        }
        self.field.clear();
        self.verifying = false;
        self.error = None;
        self.target = None;
        self.hits = None;
    }

    fn submit(&mut self, password: Secret) -> DialogAction {
        let Some(request) = &self.request else {
            return DialogAction::None;
        };
        self.verifying = true;
        self.error = None;
        DialogAction::Submit {
            cookie: request.cookie.clone(),
            user: request.user.clone(),
            password,
        }
    }

    fn dismiss(&mut self) -> DialogAction {
        let Some(request) = &self.request else {
            return DialogAction::None;
        };
        let cookie = request.cookie.clone();
        self.close();
        DialogAction::Dismiss { cookie }
    }

    pub fn key(
        &mut self,
        keysym: smithay_client_toolkit::seat::keyboard::Keysym,
        text: Option<&str>,
        ctrl: bool,
    ) -> DialogAction {
        if self.request.is_none() {
            return DialogAction::None;
        }
        // Escape works while verifying too, the way QML's Shortcut did; the
        // helper's answer for a dismissed request is then ignored.
        if self.verifying
            && keysym != smithay_client_toolkit::seat::keyboard::Keysym::Escape
        {
            return DialogAction::None;
        }
        match self.field.key(keysym, text, ctrl) {
            FieldKey::Ignored => DialogAction::None,
            FieldKey::Changed => {
                self.error = None;
                DialogAction::Redraw
            }
            FieldKey::Submit(password) => self.submit(password),
            FieldKey::Escape => self.dismiss(),
        }
    }

    pub fn on_click(&mut self, point: Point) -> DialogAction {
        let Some(hits) = self.hits else {
            return DialogAction::None;
        };
        if self.request.is_none() || self.verifying || !hits.card.contains(point) {
            return DialogAction::None;
        }
        if hits.cancel.contains(point) {
            return self.dismiss();
        }
        if hits.confirm.contains(point) {
            return match self.field.take() {
                Some(password) => self.submit(password),
                None => DialogAction::None,
            };
        }
        DialogAction::None
    }

    /// Apply the answer for `cookie`. Answers for a request that is no longer
    /// shown (dismissed meanwhile) are dropped.
    pub fn verdict(&mut self, cookie: &str, verdict: Verdict) -> bool {
        if self.request.as_ref().map(|r| r.cookie.as_str()) != Some(cookie) {
            return false;
        }
        match verdict {
            Verdict::Accepted | Verdict::Exhausted => self.close(),
            Verdict::Retry { remaining } => {
                self.verifying = false;
                self.error = Some(format!(
                    "Incorrect password. {remaining} attempt{} remaining.",
                    if remaining == 1 { "" } else { "s" }
                ));
            }
        }
        true
    }

    pub fn paint(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, surface: Rect) {
        let Some(request) = self.request.clone() else {
            return;
        };
        let m = self.metrics.clone();
        let theme = ctx.theme;
        let typo = ctx.metrics.typography.clone();

        list.push_rect(Rect {
            color: Color::rgba(0.0, 0.0, 0.0, m.scrim_alpha),
            ..surface
        });

        // Wrap first: the card's height follows its text.
        let inner_w = m.width - m.padding * 2.0;
        let title_w = inner_w - m.icon - m.icon_gap;
        let action = if request.action.is_empty() {
            "System action requires elevation.".to_string()
        } else {
            request.action.clone()
        };
        let action_lines = ui::wrap_text(ctx, &action, m.subtitle, typo.weight_regular, title_w, 2);
        let message = if request.message.is_empty() {
            "An application is attempting to perform an action that requires privileges. \
             Please enter your password to continue."
                .to_string()
        } else {
            request.message.clone()
        };
        let message_lines =
            ui::wrap_text(ctx, &message, m.message, typo.weight_regular, inner_w, m.message_lines);

        let line = |size: f32| size * 1.4;
        let header_h = m
            .icon
            .max(line(m.title) + line(m.subtitle) * action_lines.len() as f32);
        let message_h = line(m.message) * message_lines.len() as f32;
        // Caption, field, and a row under the field for the error. The row is
        // always reserved so the card does not jump when an error appears.
        // (QML put the error beside the caption; "Incorrect password. 2
        // attempts remaining." does not fit there and overlapped it.)
        let password_h = line(m.caption) + 8.0 + m.field_height + 8.0 + line(m.caption);
        let height = m.padding * 2.0
            + header_h
            + m.spacing
            + message_h
            + m.spacing
            + password_h
            + m.spacing
            + m.button_height;

        let card = Rect {
            x: surface.x + (surface.width - m.width) * 0.5,
            y: surface.y + (surface.height - height) * 0.5,
            width: m.width,
            height,
            radius: m.radius,
            color: Color::TRANSPARENT,
        };
        let border = if self.error.is_some() {
            theme.error
        } else {
            theme.border_inactive
        };
        list.push_rect(Rect {
            color: border,
            ..card
        });
        list.push_rect(Rect {
            color: theme.bg_main,
            ..inset(card, m.border)
        });

        let x = card.x + m.padding;
        let mut y = card.y + m.padding;
        let row = |x: f32, y: f32, width: f32, height: f32| Rect {
            x,
            y,
            width,
            height,
            radius: 0.0,
            color: Color::TRANSPARENT,
        };

        // Header: shield glyph, title, action id.
        ui::paint_text_centered(
            ctx,
            list,
            "\u{f15fc}",
            row(x, y, m.icon, m.icon),
            m.icon,
            theme.primary,
        );
        let tx = x + m.icon + m.icon_gap;
        let mut ty = y + (header_h - line(m.title) - line(m.subtitle) * action_lines.len() as f32) * 0.5;
        ui::paint_text_vcentered(
            ctx,
            list,
            "Authentication Required",
            tx,
            row(tx, ty, title_w, line(m.title)),
            m.title,
            typo.weight_bold,
            theme.text_main,
        );
        ty += line(m.title);
        for text in &action_lines {
            ui::paint_text_vcentered(
                ctx,
                list,
                text,
                tx,
                row(tx, ty, title_w, line(m.subtitle)),
                m.subtitle,
                typo.weight_regular,
                theme.text_muted,
            );
            ty += line(m.subtitle);
        }
        y += header_h + m.spacing;

        for text in &message_lines {
            ui::paint_text_vcentered(
                ctx,
                list,
                text,
                x,
                row(x, y, inner_w, line(m.message)),
                m.message,
                typo.weight_regular,
                theme.text_main,
            );
            y += line(m.message);
        }
        y += m.spacing;

        ui::paint_text_vcentered(
            ctx,
            list,
            &format!("Password for {}", request.user),
            x,
            row(x, y, inner_w, line(m.caption)),
            m.caption,
            typo.weight_regular,
            theme.text_muted,
        );
        y += line(m.caption) + 8.0;

        let field = Rect {
            radius: m.field_radius,
            ..row(x, y, inner_w, m.field_height)
        };
        list.push_rect(Rect {
            color: if self.verifying {
                theme.border_inactive
            } else {
                theme.primary
            },
            ..field
        });
        list.push_rect(Rect {
            color: theme.bg_widget,
            ..inset(field, m.field_border)
        });
        self.field.paint(
            ctx,
            list,
            field,
            FieldStyle {
                text: m.field_text,
                padding: m.field_padding,
                caret_width: m.caret_width,
            },
            self.verifying,
        );
        y += m.field_height + 8.0;
        if let Some(error) = &self.error {
            ui::paint_text_vcentered(
                ctx,
                list,
                error,
                x,
                row(x, y, inner_w, line(m.caption)),
                m.caption,
                typo.weight_bold,
                theme.error,
            );
        }
        y += line(m.caption) + m.spacing;

        // Buttons, right-aligned: Cancel (outlined), Authenticate (filled).
        let confirm = Rect {
            radius: m.button_radius,
            ..row(x + inner_w - m.confirm_width, y, m.confirm_width, m.button_height)
        };
        let cancel = Rect {
            radius: m.button_radius,
            ..row(
                confirm.x - m.button_gap - m.cancel_width,
                y,
                m.cancel_width,
                m.button_height,
            )
        };
        list.push_rect(Rect {
            color: theme.border_inactive,
            ..cancel
        });
        list.push_rect(Rect {
            color: theme.bg_main,
            ..inset(cancel, m.border)
        });
        ui::paint_text_centered(
            ctx,
            list,
            "Cancel",
            cancel,
            typo.body,
            if self.verifying {
                theme.text_muted
            } else {
                theme.text_main
            },
        );
        if self.verifying {
            list.push_rect(Rect {
                color: theme.border_inactive,
                ..confirm
            });
            list.push_rect(Rect {
                color: theme.bg_widget,
                ..inset(confirm, m.border)
            });
        } else {
            list.push_rect(Rect {
                color: theme.primary,
                ..confirm
            });
        }
        ui::paint_text_centered_weighted(
            ctx,
            list,
            if self.verifying {
                "Verifying..."
            } else {
                "Authenticate"
            },
            confirm,
            typo.body,
            typo.weight_bold,
            if self.verifying {
                theme.text_muted
            } else {
                theme.bg_main
            },
        );

        self.hits = Some(Hits {
            card,
            cancel,
            confirm,
        });
    }

    /// The card's rect from the last paint, for tests and hit-testing.
    #[cfg(test)]
    fn card(&self) -> Option<Rect> {
        self.hits.map(|h| h.card)
    }
}

impl Default for PolkitDialog {
    fn default() -> Self {
        Self::new()
    }
}

fn inset(rect: Rect, by: f32) -> Rect {
    Rect {
        x: rect.x + by,
        y: rect.y + by,
        width: (rect.width - by * 2.0).max(0.0),
        height: (rect.height - by * 2.0).max(0.0),
        radius: (rect.radius - by).max(0.0),
        color: rect.color,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use smithay_client_toolkit::seat::keyboard::Keysym;

    fn request(cookie: &str) -> Value {
        json!({
            "active": true,
            "cookie": cookie,
            "action": "org.freedesktop.policykit.exec",
            "message": "Authentication is needed to run `true' as the super user",
            "user": "nakul",
        })
    }

    fn typed(dialog: &mut PolkitDialog, text: &str) {
        for ch in text.chars() {
            dialog.key(Keysym::a, Some(&ch.to_string()), false);
        }
    }

    #[test]
    fn a_request_opens_on_the_focused_output() {
        let mut dialog = PolkitDialog::new();
        assert!(dialog.sync(&request("c1"), "HDMI-A-1"));
        assert_eq!(dialog.target(), Some("HDMI-A-1"));
        // Republishing the same request changes nothing, and a focus change
        // does not move an open dialog.
        assert!(!dialog.sync(&request("c1"), "eDP-2"));
        assert_eq!(dialog.target(), Some("HDMI-A-1"));
    }

    #[test]
    fn the_agent_finishing_closes_it() {
        let mut dialog = PolkitDialog::new();
        dialog.sync(&request("c1"), "a");
        // Another cookie finishing leaves this one alone.
        assert!(!dialog.sync(&json!({"active": false, "cookie": "c2"}), "a"));
        assert!(dialog.is_open());
        assert!(dialog.sync(&json!({"active": false, "cookie": "c1"}), "a"));
        assert!(!dialog.is_open());
    }

    #[test]
    fn enter_submits_and_escape_dismisses() {
        let mut dialog = PolkitDialog::new();
        dialog.sync(&request("c1"), "a");
        typed(&mut dialog, "pw");
        let DialogAction::Submit { cookie, user, password } =
            dialog.key(Keysym::Return, None, false)
        else {
            panic!("expected a submit");
        };
        assert_eq!((cookie.as_str(), user.as_str(), password.chars()), ("c1", "nakul", 2));
        // Typing waits for the answer, but Escape still closes.
        assert!(matches!(dialog.key(Keysym::a, Some("a"), false), DialogAction::None));
        let DialogAction::Dismiss { cookie } = dialog.key(Keysym::Escape, None, false) else {
            panic!("expected a dismiss");
        };
        assert_eq!(cookie, "c1");
        assert!(!dialog.is_open());
    }

    #[test]
    fn a_dismissed_request_is_not_reopened_by_stale_state() {
        let mut dialog = PolkitDialog::new();
        dialog.sync(&request("c1"), "a");
        dialog.key(Keysym::Escape, None, false);
        // The cell still says active until the agent reports the end.
        assert!(!dialog.sync(&request("c1"), "a"));
        assert!(!dialog.is_open());
        // A new request still opens.
        assert!(dialog.sync(&request("c2"), "a"));
    }

    #[test]
    fn wrong_passwords_retry_then_close() {
        let mut dialog = PolkitDialog::new();
        dialog.sync(&request("c1"), "a");
        typed(&mut dialog, "x");
        dialog.key(Keysym::Return, None, false);
        assert!(dialog.verdict("c1", Verdict::Retry { remaining: 2 }));
        assert_eq!(dialog.error.as_deref(), Some("Incorrect password. 2 attempts remaining."));
        assert!(dialog.is_open());
        typed(&mut dialog, "y");
        assert_eq!(dialog.error, None, "typing clears the error");
        dialog.key(Keysym::Return, None, false);
        dialog.verdict("c1", Verdict::Retry { remaining: 1 });
        assert_eq!(dialog.error.as_deref(), Some("Incorrect password. 1 attempt remaining."));
        dialog.verdict("c1", Verdict::Exhausted);
        assert!(!dialog.is_open());
    }

    #[test]
    fn answers_for_another_request_are_dropped() {
        let mut dialog = PolkitDialog::new();
        dialog.sync(&request("c2"), "a");
        assert!(!dialog.verdict("c1", Verdict::Accepted));
        assert!(dialog.is_open());
    }

    #[test]
    fn empty_payloads_do_nothing() {
        let mut dialog = PolkitDialog::new();
        assert!(!dialog.sync(&Value::Null, "a"));
        assert!(!dialog.sync(&json!({"active": true}), "a"), "no cookie, no dialog");
        assert!(!dialog.is_open());
        assert_eq!(dialog.card(), None);
    }
}
