//! The on-screen display for volume and brightness.
//!
//! Shows a transient overlay when either value changes, then fades out. Volume
//! is read from the pushed audio state; brightness is polled, because there is
//! no push source for the backlight.
//!
//! The overlay is a layer surface on the overlay layer with no exclusive zone,
//! anchored bottom-centre, so it floats above windows and takes no space.

use crate::render::display_list::{Color, DisplayList, Rect};
use crate::ui::animation::Tween;
use crate::ui::{self, PaintCtx};

/// Which value the OSD is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Volume,
    Brightness,
}

impl Channel {
    fn title(self) -> &'static str {
        match self {
            Channel::Volume => "Volume",
            Channel::Brightness => "Brightness",
        }
    }

    fn max(self) -> f32 {
        match self {
            // The audio pipeline allows up to 150%, matching the QML shell.
            Channel::Volume => 150.0,
            Channel::Brightness => 100.0,
        }
    }

    fn glyph(self, value: i64, muted: bool) -> &'static str {
        match self {
            Channel::Brightness => {
                if value >= 80 {
                    "\u{f00e0}"
                } else if value >= 40 {
                    "\u{f00df}"
                } else {
                    "\u{f00de}"
                }
            }
            Channel::Volume => {
                if muted || value == 0 {
                    "\u{f075e}"
                } else if value < 34 {
                    "\u{f057e}"
                } else if value < 67 {
                    "\u{f057f}"
                } else {
                    "\u{f0580}"
                }
            }
        }
    }
}

/// OSD visibility and content.
pub struct Osd {
    channel: Channel,
    value: i64,
    muted: bool,
    /// 0 fully hidden, 1 fully shown.
    opacity: Tween,
    /// Seconds remaining before fading out.
    hold_remaining: f32,
    /// Sizes and timings, refreshed from the config on each tick.
    metrics: crate::ui::style::OsdMetrics,
    /// Last observed values, used to detect changes.
    last_volume: Option<i64>,
    last_muted: Option<bool>,
    last_brightness: Option<i64>,
    visible: bool,
}

impl Osd {
    pub fn new() -> Self {
        Self {
            channel: Channel::Volume,
            value: 0,
            muted: false,
            opacity: Tween::new(0.0),
            metrics: crate::ui::style::OsdMetrics::default(),
            hold_remaining: 0.0,
            last_volume: None,
            last_muted: None,
            last_brightness: None,
            visible: false,
        }
    }

    /// True when the overlay surface should be mapped.
    pub fn visible(&self) -> bool {
        self.visible || self.opacity.value() > 0.005
    }

    /// Adopt the configured sizes and timings.
    pub fn set_metrics(&mut self, metrics: &crate::ui::style::OsdMetrics) {
        self.metrics = metrics.clone();
    }

    /// Show the OSD for a channel, restarting the hold timer.
    fn show(&mut self, channel: Channel, value: i64, muted: bool) {
        self.channel = channel;
        self.value = value.clamp(0, channel.max() as i64);
        self.muted = muted;
        self.hold_remaining = self.metrics.hold_seconds;
        self.opacity
            .to(1.0, self.metrics.fade_in_seconds, ui::animation::Easing::EaseOut);
        self.visible = true;
    }

    /// Compare pushed audio state against the last observation and show the
    /// OSD when the volume or mute state changed.
    ///
    /// The first observation primes the baseline and shows nothing, which is
    /// what prevents the OSD appearing on every shell start.
    pub fn observe_audio(&mut self, state: &crate::sys_state::SystemState) {
        let audio = state.audio.get();
        let volume = audio["volume"]
            .as_str()
            .and_then(|v| v.trim_end_matches('%').parse::<i64>().ok());
        let muted = audio["mute"].as_str().map(|m| m == "yes");

        if let (Some(volume), Some(muted)) = (volume, muted) {
            match (self.last_volume, self.last_muted) {
                (Some(prev), Some(prev_muted)) => {
                    if volume != prev || muted != prev_muted {
                        self.show(Channel::Volume, volume, muted);
                    }
                }
                _ => {}
            }
            self.last_volume = Some(volume);
            self.last_muted = Some(muted);
        }
    }

    /// Compare a polled brightness reading and show the OSD on change.
    pub fn observe_brightness(&mut self, percent: i64) {
        if let Some(previous) = self.last_brightness {
            if percent != previous {
                self.show(Channel::Brightness, percent, false);
            }
        }
        self.last_brightness = Some(percent);
    }

    /// Advance timers. Returns true while the overlay needs redrawing.
    pub fn tick(&mut self, dt: f32) -> bool {
        let mut animating = self.opacity.tick(dt);

        if self.visible {
            self.hold_remaining -= dt;
            if self.hold_remaining <= 0.0 {
                self.visible = false;
                self.opacity
                    .to(0.0, self.metrics.fade_out_seconds, ui::animation::Easing::EaseIn);
                animating = true;
            }
        }
        animating
    }

    /// Paint the OSD centred in the surface.
    pub fn paint(&self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, surface: Rect) {
        let alpha = self.opacity.value().clamp(0.0, 1.0);
        if alpha <= 0.005 {
            return;
        }

        // Panel geometry, centred horizontally.
        let m = &self.metrics;
        let width = (m.width as f32).min(surface.width - 16.0);
        let height = (m.height as f32).min(surface.height);
        let panel = Rect {
            x: surface.x + (surface.width - width) * 0.5,
            y: surface.y,
            width,
            height,
            radius: m.radius,
            color: Color::rgba(
                ctx.theme.bg_main.r,
                ctx.theme.bg_main.g,
                ctx.theme.bg_main.b,
                0.95 * alpha,
            ),
        };
        ui::paint_frame(
            list,
            panel,
            panel.color,
            ctx.theme.border_active.with_alpha(alpha),
            ctx.appearance.framed_border,
        );

        let accent = match self.channel {
            Channel::Volume => ctx.theme.secondary,
            Channel::Brightness => ctx.theme.tertiary,
        }
        .with_alpha(alpha);
        let text = ctx.theme.text_main.with_alpha(alpha);

        let margin = m.padding;
        let row = Rect {
            x: panel.x + margin,
            y: panel.y + margin,
            width: panel.width - margin * 2.0,
            height: 30.0,
            radius: 0.0,
            color: Color::TRANSPARENT,
        };

        ui::paint_text_vcentered(
            ctx,
            list,
            self.channel.glyph(self.value, self.muted),
            row.x,
            row,
            m.icon,
            ctx.metrics.typography.weight_regular,
            text,
        );
        ui::paint_text_vcentered(
            ctx,
            list,
            self.channel.title(),
            row.x + 32.0,
            row,
            m.title,
            ctx.metrics.typography.weight_bold,
            text,
        );

        let value_text = if self.channel == Channel::Volume && self.muted {
            "Muted".to_string()
        } else {
            format!("{}%", self.value)
        };
        let value_width = ctx.text.measure(
            &value_text,
            &crate::text::TextStyle {
                family: std::sync::Arc::from(ctx.theme.font.as_str()),
                size: m.value,
                weight: ctx.metrics.typography.weight_bold,
                color: accent,
            },
        ).0;
        ui::paint_text_vcentered(
            ctx,
            list,
            &value_text,
            row.x + row.width - value_width,
            row,
            m.value,
            ctx.metrics.typography.weight_bold,
            accent,
        );

        // Progress track.
        let track = Rect {
            x: panel.x + margin,
            y: panel.y + margin + 40.0,
            width: panel.width - margin * 2.0,
            height: m.track_height,
            radius: m.track_height * 0.5,
            color: ctx.theme.bg_widget.with_alpha(alpha),
        };
        list.push_rect(track);
        let ratio = (self.value as f32 / self.channel.max()).clamp(0.0, 1.0);
        list.push_rect(Rect {
            width: track.width * ratio,
            color: accent,
            ..track
        });
    }
}

impl Default for Osd {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_hidden_and_ignores_first_observation() {
        let mut osd = Osd::new();
        assert!(!osd.visible());
        assert_eq!(osd.last_volume, None);
    }

    #[test]
    fn volume_change_shows_then_fades() {
        let mut osd = Osd::new();
        osd.last_volume = Some(50);
        osd.last_muted = Some(false);
        osd.last_brightness = None;

        osd.show(Channel::Volume, 70, false);
        assert!(osd.visible());

        // Fade in fully.
        for _ in 0..30 {
            osd.tick(0.016);
        }
        assert!((osd.opacity.value() - 1.0).abs() < 0.01);

        // Hold expires and it fades out.
        for _ in 0..300 {
            osd.tick(0.016);
        }
        assert!(!osd.visible(), "opacity {}", osd.opacity.value());
    }

    #[test]
    fn brightness_first_reading_does_not_show() {
        let mut osd = Osd::new();
        osd.observe_brightness(60);
        assert!(!osd.visible());
        osd.observe_brightness(70);
        assert!(osd.visible());
    }

    #[test]
    fn volume_clamps_to_channel_maximum() {
        let mut osd = Osd::new();
        osd.show(Channel::Volume, 500, false);
        assert_eq!(osd.value, 150);
        osd.show(Channel::Brightness, 500, false);
        assert_eq!(osd.value, 100);
    }
}
