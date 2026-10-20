//! Status icons that open a control-centre page.
//!
//! Each icon varies its glyph and colour with the live subsystem state, so a
//! muted sink or a radio that is off reads differently at a glance. The glyph
//! table comes from the QML sidebar widgets.

use crate::render::display_list::{Color, DisplayList, Rect};
use crate::ui::control_center::Page;
use crate::ui::{self, Action, ClickCtx, InputState, PaintCtx, Widget};

/// Which subsystem a status icon opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconKind {
    Audio,
    Bluetooth,
    Wifi,
}

impl IconKind {
    fn page(self) -> Page {
        match self {
            IconKind::Audio => Page::Audio,
            IconKind::Bluetooth => Page::Bluetooth,
            IconKind::Wifi => Page::Wifi,
        }
    }

    /// Glyph and colour for the current system state.
    ///
    /// The QML sidebar varied the glyph by signal strength and mute state; the
    /// important part is that a disabled or muted subsystem reads differently
    /// from an active one at a glance.
    fn appearance(self, ctx: &PaintCtx<'_>) -> (&'static str, Color) {
        match self {
            IconKind::Bluetooth => {
                let bluetooth = ctx.state.bluetooth.get();
                let state = bluetooth["state"].as_str().unwrap_or("off");
                let scanning = bluetooth["scanning"].as_str() == Some("yes");
                let powered = state != "off";
                let connected = state == "connected";
                let glyph = if scanning {
                    "\u{f00b0}"
                } else if connected {
                    "\u{f00b1}"
                } else if powered {
                    "\u{f00af}"
                } else {
                    "\u{f00b2}"
                };
                let color = if powered {
                    ctx.theme.primary
                } else {
                    ctx.theme.text_muted
                };
                (glyph, color)
            }
            IconKind::Wifi => {
                // `net.state` is ethernet/wifi/none; prefer a wired glyph when
                // the primary connection is ethernet, like the QML widget.
                let net = ctx.state.net.get();
                let state = net["state"].as_str().unwrap_or("none");
                let signal = net["signal_pct"]
                    .as_str()
                    .and_then(|s| s.parse::<i64>().ok())
                    .or_else(|| net["signal_pct"].as_i64())
                    .unwrap_or(0);
                let glyph = match state {
                    "ethernet" => "\u{f0200}",
                    "wifi" => match signal {
                        80.. => "\u{f05a8}",
                        60..=79 => "\u{f05a5}",
                        40..=59 => "\u{f05a2}",
                        20..=39 => "\u{f059f}",
                        _ => "\u{f05a9}",
                    },
                    _ => "\u{f05aa}",
                };
                let color = if state == "none" {
                    ctx.theme.text_muted
                } else {
                    ctx.theme.primary
                };
                (glyph, color)
            }
            IconKind::Audio => {
                let audio = ctx.state.audio.get();
                let muted = audio["mute"].as_str() == Some("yes");
                let volume = audio["volume"]
                    .as_str()
                    .and_then(|v| v.trim_end_matches('%').parse::<i64>().ok())
                    .unwrap_or(0);
                let headphones = audio["headphones"].as_str() == Some("yes");
                let glyph = if muted || volume == 0 {
                    "\u{f075e}"
                } else if headphones {
                    "\u{f025d}"
                } else if volume < 34 {
                    "\u{f057e}"
                } else if volume < 67 {
                    "\u{f057f}"
                } else {
                    "\u{f0580}"
                };
                let color = if muted {
                    ctx.theme.error
                } else {
                    ctx.theme.primary
                };
                (glyph, color)
            }
        }
    }
}

/// A status icon that opens its page in the control centre.
pub struct Icon {
    pub kind: IconKind,
    /// Rect from the last paint, used for hit-testing.
    last_rect: Rect,
}

impl Icon {
    pub fn new(kind: IconKind) -> Self {
        Self {
            kind,
            last_rect: Rect::EMPTY,
        }
    }
}

impl Widget for Icon {
    fn height(&self, _width: f32, metrics: &crate::ui::style::Metrics) -> Option<f32> {
        Some(metrics.typography.icon)
    }

    fn width(&self, height: f32, metrics: &crate::ui::style::Metrics) -> Option<f32> {
        Some(height.max(metrics.typography.icon))
    }

    fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        rect: Rect,
        _input: &InputState,
    ) {
        self.last_rect = rect;
        let (glyph, color) = self.kind.appearance(ctx);
        ui::paint_text_centered(ctx, list, glyph, rect, ctx.metrics.typography.icon, color);
    }

    fn on_click(&mut self, ctx: &ClickCtx<'_>) -> Option<Action> {
        if !self.last_rect.contains(ctx.point) {
            return None;
        }
        Some(Action::OpenPanel(self.kind.page()))
    }
}
