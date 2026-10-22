//! Control-centre pages.
//!
//! Each page is a self-contained module that owns its own interaction state and
//! paints into whatever rect the window gives it. The control centre only does
//! window chrome (navigation rail, background) and page selection; adding a page
//! means adding a module here and an arm in [`Pages::paint`].
//!
//! Pages use an [`Optimistic`] overlay for values the user just changed, so a
//! click or drag is reflected on the very next frame. The real state catches up
//! a few milliseconds later and clears the overlay.

use crate::actions::SystemAction;
use crate::render::display_list::{DisplayList, Rect};
use crate::ui::{InputState, PaintCtx};

pub mod audio;
pub mod battery;
pub mod bluetooth;
pub mod devices;
pub mod display;
pub mod equalizer;
pub mod notifications;
pub mod overview;
pub mod power;
pub mod wifi;

/// A control-centre page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Overview,
    Audio,
    Equalizer,
    Bluetooth,
    Wifi,
    Battery,
    Notifications,
    Display,
    Power,
}

impl Page {
    /// Pages shown in the navigation rail, in order.
    pub const ALL: [Page; 9] = [
        Page::Overview,
        Page::Audio,
        Page::Equalizer,
        Page::Bluetooth,
        Page::Wifi,
        Page::Battery,
        Page::Notifications,
        Page::Display,
        Page::Power,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Page::Overview => "Overview",
            Page::Audio => "Audio",
            Page::Equalizer => "Equalizer",
            Page::Bluetooth => "Bluetooth",
            Page::Wifi => "Network",
            Page::Battery => "Battery",
            Page::Notifications => "Notifications",
            Page::Display => "Display",
            Page::Power => "Power",
        }
    }

    /// Nerd Font glyph for the navigation rail.
    pub fn glyph(self) -> &'static str {
        match self {
            Page::Overview => "\u{f009}",
            Page::Audio => "\u{f0581}",
            Page::Equalizer => "\u{f00ec}",
            Page::Bluetooth => "\u{f00b1}",
            Page::Wifi => "\u{f05a9}",
            Page::Battery => "\u{f007a}",
            Page::Notifications => "\u{f009a}",
            Page::Display => "\u{f108}",
            Page::Power => "\u{f011}",
        }
    }
}

/// A user-set value held until the real state catches up.
///
/// This is what makes a toggle feel instantaneous: the click writes the value
/// the user asked for, the page paints it, and when the system snapshot finally
/// reports the same value the overlay is dropped. If the change is rejected the
/// overlay expires on its own, so the UI never lies for long.
pub struct Optimistic<T> {
    value: Option<T>,
    /// Seconds remaining before the overlay gives up.
    remaining: f32,
}

impl<T: Clone + PartialEq> Optimistic<T> {
    pub fn new() -> Self {
        Self {
            value: None,
            remaining: 0.0,
        }
    }

    /// Record an intent for the next few frames.
    pub fn set(&mut self, value: T) {
        self.value = Some(value);
        self.remaining = 2.0;
    }

    /// Resolve the value to paint, dropping the overlay once reality agrees.
    pub fn view(&mut self, real: T) -> T {
        match &self.value {
            Some(value) if *value == real => {
                self.value = None;
                real
            }
            Some(value) => value.clone(),
            None => real,
        }
    }

    pub fn tick(&mut self, dt: f32) {
        if self.value.is_none() {
            return;
        }
        self.remaining -= dt;
        if self.remaining <= 0.0 {
            self.value = None;
        }
    }

    /// The pending intent, if any.
    #[cfg(test)]
    pub fn pending(&self) -> Option<&T> {
        self.value.as_ref()
    }
}

impl<T: Clone + PartialEq> Default for Optimistic<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// State shared by every page for the current frame.
pub struct Pages {
    pub pending: Vec<SystemAction>,
    /// A press edge happened on this frame.
    pub clicked: bool,
    /// The pointer was down last frame, for detecting press edges.
    pressed_last: bool,
    /// Time since the previous frame, for optimistic values.
    dt: f32,
    pub overview: overview::OverviewPage,
    pub audio: audio::AudioPage,
    pub equalizer: equalizer::EqualizerPage,
    pub bluetooth: bluetooth::BluetoothPage,
    pub wifi: wifi::WifiPage,
    pub battery: battery::BatteryPage,
    pub notifications: notifications::NotificationsPage,
    pub display: display::DisplayPage,
    pub power: power::PowerPage,
}

impl Pages {
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
            clicked: false,
            pressed_last: false,
            dt: 0.0,
            overview: overview::OverviewPage::new(),
            audio: audio::AudioPage::new(),
            equalizer: equalizer::EqualizerPage::new(),
            bluetooth: bluetooth::BluetoothPage::new(),
            wifi: wifi::WifiPage::new(),
            battery: battery::BatteryPage::new(),
            notifications: notifications::NotificationsPage::new(),
            display: display::DisplayPage::new(),
            power: power::PowerPage::new(),
        }
    }

    /// Start a frame: derives the click edge.
    ///
    /// Pending actions are not cleared here: `take_actions` drains them after
    /// each paint, and clearing dropped everything queued between paints (a
    /// key press submitting a text field, a page being shown).
    pub fn begin_frame(&mut self, input: &InputState, dt: f32) {
        self.dt = dt;
        self.clicked = input.pressed && !self.pressed_last;
        self.pressed_last = input.pressed;

        self.audio.tick(dt);
        self.bluetooth.tick(dt);
        self.wifi.tick(dt);
        self.battery.tick(dt);
        self.display.tick(dt);
        self.power.tick(dt);
    }

    /// Called when the visible page changes. Pages refresh lazily, so this is
    /// where a newly opened list asks for its data.
    pub fn on_page_shown(&mut self, page: Page) {
        match page {
            Page::Wifi => self.wifi.request_refresh(&mut self.pending),
            Page::Bluetooth => self.bluetooth.request_refresh(&mut self.pending),
            _ => {}
        }
    }

    pub fn take_actions(&mut self) -> Vec<SystemAction> {
        std::mem::take(&mut self.pending)
    }

    /// Whether `page` has a text field that wants the keyboard.
    pub fn wants_keys(&self, page: Page) -> bool {
        match page {
            Page::Equalizer => self.equalizer.wants_keys(),
            Page::Wifi => self.wifi.wants_keys(),
            _ => false,
        }
    }

    /// Deliver a key to `page`'s text field.
    pub fn key(
        &mut self,
        page: Page,
        keysym: smithay_client_toolkit::seat::keyboard::Keysym,
        text: Option<&str>,
        ctrl: bool,
    ) {
        match page {
            Page::Equalizer => self.equalizer.key(keysym, text, ctrl, &mut self.pending),
            Page::Wifi => self.wifi.key(keysym, text, ctrl),
            _ => {}
        }
    }

    /// Whether the visible page needs continuous frames (a scan spinner, say).
    pub fn animating(&self, page: Page) -> bool {
        match page {
            Page::Bluetooth => self.bluetooth.animating(),
            Page::Wifi => self.wifi.animating(),
            _ => false,
        }
    }

    pub fn paint(
        &mut self,
        page: Page,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        area: Rect,
        input: &InputState,
    ) {
        let clicked = self.clicked;
        let pending = &mut self.pending;
        match page {
            Page::Overview => self.overview.paint(ctx, list, area),
            Page::Audio => self.audio.paint(ctx, list, area, input, clicked, pending),
            Page::Equalizer => self.equalizer.paint(ctx, list, area, input, clicked, pending),
            Page::Bluetooth => {
                self.bluetooth
                    .paint(ctx, list, area, input, clicked, pending)
            }
            Page::Wifi => self.wifi.paint(ctx, list, area, input, clicked, pending),
            Page::Battery => self.battery.paint(ctx, list, area, input, clicked, pending),
            Page::Notifications => {
                self.notifications
                    .paint(ctx, list, area, input, clicked, pending)
            }
            Page::Display => self.display.paint(ctx, list, area, input, clicked, pending),
            Page::Power => self.power.paint(ctx, list, area, input, clicked, pending),
        }
    }
}

impl Default for Pages {
    fn default() -> Self {
        Self::new()
    }
}

/// Draw a page heading and return the content rect below it.
pub(crate) fn heading(
    ctx: &mut PaintCtx<'_>,
    list: &mut DisplayList,
    area: Rect,
    title: &str,
) -> Rect {
    crate::ui::paint_text_vcentered(
        ctx,
        list,
        title,
        area.x,
        Rect {
            height: ctx.metrics.page.button_height,
            ..area
        },
        ctx.metrics.typography.heading,
ctx.metrics.typography.weight_bold,
        ctx.theme.text_main,
    );
    Rect {
        y: area.y + 40.0,
        height: (area.height - 40.0).max(0.0),
        ..area
    }
}

/// One label/value row, used by several pages.
pub(crate) fn stat_row(
    ctx: &mut PaintCtx<'_>,
    list: &mut DisplayList,
    area: Rect,
    index: usize,
    label: &str,
    value: &str,
) {
    let rect = Rect {
        x: area.x,
        y: area.y + index as f32 * 38.0,
        width: area.width,
        height: 32.0,
        radius: 0.0,
        color: crate::render::display_list::Color::TRANSPARENT,
    };
    crate::ui::paint_text_vcentered(
        ctx,
        list,
        label,
        rect.x,
        rect,
        ctx.metrics.typography.body,
ctx.metrics.typography.weight_regular,
        ctx.theme.text_muted,
    );
    crate::ui::paint_text_vcentered(
        ctx,
        list,
        value,
        rect.x + rect.width * 0.45,
        rect,
        ctx.metrics.typography.body,
ctx.metrics.typography.weight_medium,
        ctx.theme.text_main,
    );
}

/// A small section label.
pub(crate) fn section_label(
    ctx: &mut PaintCtx<'_>,
    list: &mut DisplayList,
    area: Rect,
    y: f32,
    label: &str,
) {
    crate::ui::paint_text_vcentered(
        ctx,
        list,
        label,
        area.x,
        Rect {
            y,
            height: 20.0,
            ..area
        },
        ctx.metrics.typography.label,
ctx.metrics.typography.weight_bold,
        ctx.theme.text_muted,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optimistic_yields_until_reality_matches() {
        let mut value: Optimistic<bool> = Optimistic::new();
        // No intent: the real value wins.
        assert!(!value.view(false));

        value.set(true);
        assert!(value.view(false), "intent should win immediately");

        // Reality catches up and the overlay clears.
        assert!(value.view(true));
        assert_eq!(value.pending(), None);
    }

    #[test]
    fn optimistic_expires_when_reality_never_agrees() {
        let mut value: Optimistic<bool> = Optimistic::new();
        value.set(true);
        value.tick(1.5);
        assert!(value.view(false));
        value.tick(1.0);
        assert!(!value.view(false), "overlay should have expired");
    }
}
