//! Status icons that open a control-centre page.
//!
//! Each icon varies its glyph and colour with the live subsystem state, so a
//! muted sink or a radio that is off reads differently at a glance. The glyph
//! table comes from the QML sidebar widgets.

use std::collections::HashMap;

use crate::render::display_list::{Color, DisplayList, Point, Rect};
use crate::ui::control_center::Page;
use crate::ui::hover_menu::HoverKind;
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

    fn hover(self) -> HoverKind {
        match self {
            IconKind::Audio => HoverKind::Audio,
            IconKind::Bluetooth => HoverKind::Bluetooth,
            IconKind::Wifi => HoverKind::Wifi,
        }
    }

    /// Glyph and colour for the current system state.
    ///
    /// The QML sidebar varied the glyph by signal strength and mute state; the
    /// important part is that a disabled or muted subsystem reads differently
    /// from an active one at a glance.
    pub(crate) fn appearance(self, ctx: &PaintCtx<'_>) -> (&'static str, Color) {
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
                // QML's glyphs (`sidebar/Wifi.qml`): the wifi-strength set,
                // with the outlined "off" glyph when not connected.
                let glyph = match state {
                    "ethernet" => "\u{f0200}",
                    "wifi" => match signal {
                        80.. => "\u{f0928}",
                        60..=79 => "\u{f0925}",
                        40..=59 => "\u{f0922}",
                        20..=39 => "\u{f091f}",
                        _ => "\u{f092f}",
                    },
                    _ => "\u{f092e}",
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
                // QML's glyphs (`sidebar/Audio.qml`): volume-off, headphones,
                // then low/medium/high. Its "boosted" glyph was unreachable.
                let glyph = if muted || volume == 0 {
                    "\u{f0581}"
                } else if headphones {
                    "\u{f02cb}"
                } else if volume < 34 {
                    "\u{f057f}"
                } else if volume < 67 {
                    "\u{f0580}"
                } else {
                    "\u{f057e}"
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

/// A status icon that opens its page in the control centre, and its hover
/// menu when the pointer rests on it.
pub struct Icon {
    pub kind: IconKind,
    /// Rect from the last paint on each output, for hit-testing. Keyed by
    /// output because the widget tree is shared and the rails differ in
    /// height: one rect would only match the monitor painted last.
    rects: HashMap<String, Rect>,
}

impl Icon {
    pub fn new(kind: IconKind) -> Self {
        Self {
            kind,
            rects: HashMap::new(),
        }
    }

    fn hit(&self, output: &str, point: Point) -> Option<Rect> {
        self.rects.get(output).copied().filter(|r| r.contains(point))
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
        self.rects.insert(ctx.output_name.to_string(), rect);
        let (glyph, color) = self.kind.appearance(ctx);
        ui::paint_text_centered(ctx, list, glyph, rect, ctx.metrics.typography.icon, color);
    }

    fn on_click(&mut self, ctx: &ClickCtx<'_>) -> Option<Action> {
        self.hit(ctx.output_name, ctx.point)?;
        Some(Action::OpenPanel(self.kind.page()))
    }

    fn hover_target(&self, output: &str, point: Point) -> Option<(HoverKind, Rect)> {
        self.hit(output, point).map(|rect| (self.kind.hover(), rect))
    }
}
