//! Layout metrics: every size the shell draws with.
//!
//! Colours live in [`crate::config::Theme`]; this module is the other half of
//! styling. Nothing here knows how to draw — it is pure data, deserialised from
//! the `[metrics]` table in the config file, so adjusting a row height or a font
//! size means editing one number rather than hunting through widget code.
//!
//! Widgets read these through `PaintCtx::metrics`. The defaults reproduce the
//! QML shell's proportions.

use serde::{Deserialize, Serialize};

/// Everything sizing-related.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Metrics {
    pub typography: Typography,
    pub bar: BarMetrics,
    pub workspaces: WorkspacesMetrics,
    pub tray: TrayMetrics,
    pub osd: OsdMetrics,
    pub toasts: ToastMetrics,
    pub power_menu: PowerMenuMetrics,
    pub tray_menu: TrayMenuMetrics,
    pub lock: LockMetrics,
    pub polkit: PolkitMetrics,
    pub notification_card: NotificationCardMetrics,
    pub theme_switcher: ThemeSwitcherMetrics,
    pub hover_menu: HoverMenuMetrics,
    pub equalizer: EqualizerMetrics,
    pub dashboard: DashboardMetrics,
    pub screenshot: ScreenshotMetrics,
    pub screenshot_viewer: ScreenshotViewerMetrics,
    pub device_page: DevicePageMetrics,
    pub audio_page: AudioPageMetrics,
    pub window: WindowMetrics,
    pub page: PageMetrics,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            typography: Typography::default(),
            bar: BarMetrics::default(),
            workspaces: WorkspacesMetrics::default(),
            tray: TrayMetrics::default(),
            osd: OsdMetrics::default(),
            toasts: ToastMetrics::default(),
            power_menu: PowerMenuMetrics::default(),
            tray_menu: TrayMenuMetrics::default(),
            lock: LockMetrics::default(),
            polkit: PolkitMetrics::default(),
            notification_card: NotificationCardMetrics::default(),
            theme_switcher: ThemeSwitcherMetrics::default(),
            hover_menu: HoverMenuMetrics::default(),
            equalizer: EqualizerMetrics::default(),
            dashboard: DashboardMetrics::default(),
            screenshot: ScreenshotMetrics::default(),
            screenshot_viewer: ScreenshotViewerMetrics::default(),
            device_page: DevicePageMetrics::default(),
            audio_page: AudioPageMetrics::default(),
            window: WindowMetrics::default(),
            page: PageMetrics::default(),
        }
    }
}

/// Font sizes by role. All in logical pixels.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Typography {
    /// Status icons in the bar.
    pub icon: f32,
    /// The battery glyph, which is deliberately larger than the others.
    pub battery: f32,
    /// The percentage beneath the battery glyph.
    pub battery_label: f32,
    /// Clock digits in a vertical rail.
    pub clock: f32,
    /// Clock text in a horizontal bar.
    pub clock_bar: f32,
    /// Workspace numbers.
    pub workspace: f32,
    /// The rotated window title.
    pub title: f32,
    /// The rotated title in a horizontal bar.
    pub title_bar: f32,
    /// Packaged tray glyphs and small labels.
    pub tray: f32,
    /// Page and section headings.
    pub heading: f32,
    /// Default body text.
    pub body: f32,
    /// Secondary text and captions.
    pub caption: f32,
    /// Small labels above values.
    pub label: f32,
    /// Weights, kept beside sizes so text styling is adjusted together.
    pub weight_regular: u16,
    pub weight_medium: u16,
    pub weight_bold: u16,
}

impl Default for Typography {
    fn default() -> Self {
        Self {
            icon: 20.0,
            battery: 24.0,
            battery_label: 11.0,
            clock: 18.0,
            clock_bar: 13.0,
            workspace: 15.0,
            title: 14.0,
            title_bar: 13.0,
            tray: 13.0,
            heading: 18.0,
            body: 13.0,
            caption: 11.0,
            label: 12.0,
            weight_regular: 400,
            weight_medium: 500,
            weight_bold: 600,
        }
    }
}

/// Sizing shared by the sidebar and any container inside it.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct BarMetrics {
    /// Size of the launcher button.
    pub launcher: f32,
    /// Inner margin of a framed group (quick panel).
    pub group_margin: f32,
    /// Gap between widgets inside a framed group.
    pub group_spacing: f32,
    /// Width of a framed group in a vertical rail.
    pub group_width: f32,
    /// Height of the corner scoop where a rail meets the screen edge.
    pub corner_size: f32,
    /// App icon before the window title, and the gap between them.
    pub title_icon: f32,
    pub title_icon_gap: f32,
    /// Line height of stacked bar text (clock rows, battery label), as a
    /// multiple of its font size.
    pub label_line: f32,
    /// Room for the battery percentage beside its glyph in a horizontal bar.
    pub battery_label_width: f32,
    /// Width of the clock in a horizontal bar.
    pub clock_width: f32,
}

impl Default for BarMetrics {
    fn default() -> Self {
        Self {
            launcher: 24.0,
            group_margin: 6.0,
            group_spacing: 4.0,
            group_width: 30.0,
            corner_size: 20.0,
            title_icon: 16.0,
            title_icon_gap: 8.0,
            label_line: 1.2,
            battery_label_width: 34.0,
            clock_width: 84.0,
        }
    }
}

/// The scrolling workspace strip.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct WorkspacesMetrics {
    /// Rows visible at once.
    pub visible: usize,
    /// Height of one row.
    pub row_height: f32,
    /// Gap between rows.
    pub gap: f32,
    /// Ids per page. Hyprland numbers workspaces globally, so a page is a fixed
    /// range rather than an allocation per monitor.
    pub page_size: i32,
    /// How many rows above centre the active workspace sits.
    pub bias: usize,
    /// Width of the underline drawn beneath the active workspace.
    pub marker_width: f32,
    /// Height of that underline.
    pub marker_height: f32,
    /// Scroll duration in milliseconds.
    pub scroll_ms: u32,
    /// Horizontal advance per cell in a horizontal bar.
    pub cell_pitch: f32,
}

impl Default for WorkspacesMetrics {
    fn default() -> Self {
        Self {
            visible: 5,
            row_height: 20.0,
            gap: 3.0,
            page_size: 10,
            bias: 2,
            marker_width: 14.0,
            marker_height: 2.0,
            scroll_ms: 400,
            cell_pitch: 26.0,
        }
    }
}

impl WorkspacesMetrics {
    /// Distance from one row's start to the next.
    pub fn step(&self) -> f32 {
        self.row_height + self.gap
    }

    /// Total height of the visible window.
    pub fn window_height(&self) -> f32 {
        self.visible as f32 * self.step() - self.gap
    }
}

/// The tray strip.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct TrayMetrics {
    /// Size of one icon cell.
    pub cell: f32,
    /// Gap between cells.
    pub gap: f32,
    /// Maximum icons shown in a vertical rail.
    pub vertical_limit: usize,
    /// Maximum icons shown in a horizontal bar.
    pub horizontal_limit: usize,
}

impl Default for TrayMetrics {
    fn default() -> Self {
        Self {
            cell: 24.0,
            gap: 4.0,
            vertical_limit: 6,
            horizontal_limit: 8,
        }
    }
}

/// The volume/brightness overlay.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct OsdMetrics {
    /// Surface width.
    pub width: u32,
    /// Surface height.
    pub height: u32,
    /// Distance from the bottom edge of the screen.
    pub margin_bottom: i32,
    /// Inner padding.
    pub padding: f32,
    /// Corner radius.
    pub radius: f32,
    /// Height of the value track.
    pub track_height: f32,
    /// Height of the icon, title and value row, and the gap below it.
    pub row_height: f32,
    pub gap: f32,
    /// Space between the icon and the title.
    pub icon_gap: f32,
    /// Icon size.
    pub icon: f32,
    /// Title size.
    pub title: f32,
    /// Value size.
    pub value: f32,
    /// Seconds the overlay stays fully visible before fading.
    pub hold_seconds: f32,
    /// Fade-in duration in seconds.
    pub fade_in_seconds: f32,
    /// Fade-out duration in seconds.
    pub fade_out_seconds: f32,
    /// How long the fill takes to slide to a new value.
    pub fill_seconds: f32,
}

impl Default for OsdMetrics {
    fn default() -> Self {
        Self {
            width: 320,
            height: 94,
            margin_bottom: 72,
            padding: 14.0,
            radius: 12.0,
            track_height: 9.0,
            row_height: 30.0,
            gap: 10.0,
            icon_gap: 12.0,
            icon: 22.0,
            title: 13.0,
            value: 12.0,
            hold_seconds: 1.2,
            fade_in_seconds: 0.13,
            fade_out_seconds: 0.22,
            fill_seconds: 0.12,
        }
    }
}

/// The power menu overlay.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct PowerMenuMetrics {
    /// Width of one option tile.
    pub tile_width: f32,
    /// Height of one option tile.
    pub tile_height: f32,
    /// Gap between tiles.
    pub tile_gap: f32,
    /// Padding around the row of tiles.
    pub panel_padding: f32,
    /// Corner radius of the panel.
    pub panel_radius: f32,
    /// Corner radius of a tile.
    pub tile_radius: f32,
    /// Icon size inside a tile.
    pub icon: f32,
    /// Label size inside a tile.
    pub label: f32,
    /// Dimming applied to the whole screen behind the menu.
    pub scrim_alpha: f32,
}

impl Default for PowerMenuMetrics {
    fn default() -> Self {
        Self {
            tile_width: 140.0,
            tile_height: 140.0,
            tile_gap: 10.0,
            panel_padding: 20.0,
            panel_radius: 12.0,
            tile_radius: 8.0,
            icon: 48.0,
            label: 10.0,
            scrim_alpha: 0.63,
        }
    }
}

/// The lock screen. Defaults follow `modules/LockScreen.qml`.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct LockMetrics {
    /// Clock text size.
    pub clock: f32,
    /// Avatar diameter.
    pub avatar: f32,
    /// Width of the ring around the avatar.
    pub avatar_border: f32,
    /// "Enter Password" label size.
    pub label: f32,
    /// Password field size.
    pub field_width: f32,
    pub field_height: f32,
    pub field_radius: f32,
    pub field_border: f32,
    /// Horizontal padding inside the field.
    pub field_padding: f32,
    /// Size of the password dots.
    pub field_text: f32,
    /// Width of the caret after the dots.
    pub caret_width: f32,
    /// Status / error line size.
    pub status: f32,
    /// Vertical gap between the column's rows.
    pub spacing: f32,
    /// Dimming over the wallpaper (QML `#8011111b`).
    pub scrim_alpha: f32,
    /// Width the wallpaper is downscaled to before blurring, in pixels.
    pub backdrop_width: f32,
    /// Blur radius at that width. 0 disables the blur.
    pub backdrop_blur: f32,
}

impl Default for LockMetrics {
    fn default() -> Self {
        Self {
            clock: 50.0,
            avatar: 120.0,
            avatar_border: 3.0,
            label: 20.0,
            field_width: 300.0,
            field_height: 50.0,
            field_radius: 8.0,
            field_border: 2.0,
            field_padding: 14.0,
            field_text: 18.0,
            caret_width: 2.0,
            status: 14.0,
            spacing: 24.0,
            scrim_alpha: 0.5,
            backdrop_width: 320.0,
            backdrop_blur: 3.0,
        }
    }
}

/// The polkit authentication dialog. Defaults follow `modules/PolkitDialog.qml`.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct PolkitMetrics {
    /// Card width; its height follows the content.
    pub width: f32,
    /// Padding inside the card.
    pub padding: f32,
    /// Vertical gap between the card's rows.
    pub spacing: f32,
    pub radius: f32,
    pub border: f32,
    /// Shield glyph and the gap to the title beside it.
    pub icon: f32,
    pub icon_gap: f32,
    /// "Authentication Required".
    pub title: f32,
    /// The action id under the title.
    pub subtitle: f32,
    /// The request's message.
    pub message: f32,
    /// "Password for …" and the error beside it.
    pub caption: f32,
    /// Lines of message shown before it is elided.
    pub message_lines: usize,
    pub field_height: f32,
    pub field_radius: f32,
    pub field_border: f32,
    pub field_padding: f32,
    pub field_text: f32,
    pub caret_width: f32,
    pub button_height: f32,
    pub button_radius: f32,
    pub cancel_width: f32,
    pub confirm_width: f32,
    pub button_gap: f32,
    /// Dimming behind the card (QML `#cc000000`).
    pub scrim_alpha: f32,
}

impl Default for PolkitMetrics {
    fn default() -> Self {
        Self {
            width: 450.0,
            padding: 32.0,
            spacing: 24.0,
            radius: 16.0,
            border: 1.0,
            icon: 48.0,
            icon_gap: 20.0,
            title: 20.0,
            subtitle: 13.0,
            message: 14.0,
            caption: 12.0,
            message_lines: 4,
            field_height: 40.0,
            field_radius: 8.0,
            field_border: 1.0,
            field_padding: 12.0,
            field_text: 14.0,
            caret_width: 2.0,
            button_height: 36.0,
            button_radius: 8.0,
            cancel_width: 80.0,
            confirm_width: 120.0,
            button_gap: 12.0,
            scrim_alpha: 0.8,
        }
    }
}

/// A notification card, shared by the toasts and the notifications page.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct NotificationCardMetrics {
    /// Inner padding, and the gap from the urgency stripe.
    pub padding: f32,
    /// Vertical gap between the card's rows.
    pub row_gap: f32,
    /// Text sizes.
    pub app: f32,
    pub summary: f32,
    pub body: f32,
    /// Lines of body shown before it is elided.
    pub body_lines: usize,
    /// Line height as a multiple of the text size.
    pub line_height: f32,
    /// The close button in the top-right corner.
    pub dismiss: f32,
    pub dismiss_icon: f32,
    /// The picture or app icon beside the header, its frame, and the gap to
    /// the text. `visual_glyph` is the urgency glyph shown without one.
    pub visual: f32,
    pub visual_radius: f32,
    pub visual_gap: f32,
    pub visual_glyph: f32,
    /// Action buttons.
    pub button_height: f32,
    pub button_min_width: f32,
    pub button_padding: f32,
    pub button_gap: f32,
    pub button_radius: f32,
    pub button_text: f32,
    /// The inline reply row.
    pub reply_height: f32,
    pub reply_radius: f32,
    pub reply_border: f32,
    pub reply_padding: f32,
    pub reply_text: f32,
    pub send_width: f32,
    pub caret_width: f32,
}

impl Default for NotificationCardMetrics {
    fn default() -> Self {
        Self {
            padding: 14.0,
            row_gap: 8.0,
            app: 11.0,
            summary: 13.0,
            body: 12.0,
            body_lines: 3,
            line_height: 1.35,
            dismiss: 22.0,
            dismiss_icon: 12.0,
            visual: 20.0,
            visual_radius: 5.0,
            visual_gap: 10.0,
            visual_glyph: 16.0,
            button_height: 26.0,
            button_min_width: 74.0,
            button_padding: 9.0,
            button_gap: 6.0,
            button_radius: 6.0,
            button_text: 12.0,
            reply_height: 30.0,
            reply_radius: 6.0,
            reply_border: 1.0,
            reply_padding: 8.0,
            reply_text: 12.0,
            send_width: 52.0,
            caret_width: 1.5,
        }
    }
}

/// The theme switcher overlay. Defaults follow `modules/ThemeSwitcher.qml`.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeSwitcherMetrics {
    pub width: f32,
    pub padding: f32,
    /// Gap between the card's rows.
    pub spacing: f32,
    pub radius: f32,
    pub border: f32,
    pub title: f32,
    /// "Current: …".
    pub subtitle: f32,
    pub search_height: f32,
    pub search_radius: f32,
    pub search_padding: f32,
    pub search_text: f32,
    pub caret_width: f32,
    pub row_height: f32,
    pub row_gap: f32,
    pub row_radius: f32,
    pub row_padding: f32,
    pub row_text: f32,
    /// The "No matching themes" box.
    pub empty_height: f32,
    pub empty_text: f32,
    /// Rows shown before the list scrolls.
    pub max_rows: usize,
    /// Dimming behind the card (QML `#90000000`).
    pub scrim_alpha: f32,
}

impl Default for ThemeSwitcherMetrics {
    fn default() -> Self {
        Self {
            width: 360.0,
            padding: 16.0,
            spacing: 12.0,
            radius: 12.0,
            border: 1.0,
            title: 20.0,
            subtitle: 13.0,
            search_height: 34.0,
            search_radius: 8.0,
            search_padding: 10.0,
            search_text: 13.0,
            caret_width: 1.5,
            row_height: 42.0,
            row_gap: 4.0,
            row_radius: 8.0,
            row_padding: 14.0,
            row_text: 14.0,
            empty_height: 36.0,
            empty_text: 12.0,
            max_rows: 8,
            scrim_alpha: 0.56,
        }
    }
}

/// The dashboard overlay.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct DashboardMetrics {
    /// Milliseconds between data refreshes while open (stratumd: 2000).
    pub refresh_ms: u64,
    /// Distance from the top of the screen, and the card's padding.
    pub margin: f32,
    pub padding: f32,
    /// Gap between panels, and padding inside one.
    pub gap: f32,
    pub inner: f32,
    pub radius: f32,
    pub border: f32,
    pub performance_width: f32,
    pub calendar_width: f32,
    pub music_width: f32,
    pub panel_height: f32,
    pub title: f32,
    pub text: f32,
    pub caption: f32,
    pub line: f32,
    /// Load and progress bars.
    pub bar: f32,
    /// Calendar cells and the gap between them.
    pub cell: f32,
    pub cell_gap: f32,
    pub button: f32,
    /// Album art.
    pub art: f32,
    /// Dimming behind the card (QML `#90000000`).
    pub scrim_alpha: f32,
}

impl Default for DashboardMetrics {
    fn default() -> Self {
        Self {
            refresh_ms: 2000,
            margin: 12.0,
            padding: 8.0,
            gap: 8.0,
            inner: 12.0,
            radius: 10.0,
            border: 1.0,
            performance_width: 240.0,
            calendar_width: 290.0,
            music_width: 250.0,
            panel_height: 340.0,
            title: 15.0,
            text: 13.0,
            caption: 11.0,
            line: 20.0,
            bar: 6.0,
            cell: 32.0,
            cell_gap: 4.0,
            button: 28.0,
            art: 110.0,
            scrim_alpha: 0.56,
        }
    }
}

/// The screenshot overlay. Defaults are `modules/ScreenshotToolbar.qml`'s.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ScreenshotMetrics {
    /// How far a press must move before it becomes a region drag.
    pub drag_threshold: f32,
    /// Added on every side of a picked window, so its shadow and border are
    /// in the shot.
    pub window_margin: f32,
    /// Between hiding the overlay and capturing, so the compositor has
    /// presented the screen without it.
    pub capture_delay_ms: u64,
    /// Outline width, and the corner radius of the window and region outlines.
    pub border: f32,
    pub window_radius: f32,
    pub region_radius: f32,
    /// Selection fill opacity, and the lighter one over a whole output.
    pub fill_alpha: f32,
    pub output_fill_alpha: f32,
    /// Crosshair: arm length from the centre, the dark stroke, and the light
    /// outline around it.
    pub crosshair_arm: f32,
    pub crosshair_width: f32,
    pub crosshair_outline: f32,
    /// The size readout beside the selection.
    pub label: f32,
    pub label_padding: f32,
    pub label_height: f32,
    pub label_radius: f32,
    pub label_gap: f32,
}

impl Default for ScreenshotMetrics {
    fn default() -> Self {
        Self {
            drag_threshold: 8.0,
            window_margin: 5.0,
            capture_delay_ms: 120,
            border: 2.0,
            window_radius: 6.0,
            region_radius: 3.0,
            fill_alpha: 0.23,
            output_fill_alpha: 0.1,
            crosshair_arm: 10.0,
            crosshair_width: 3.0,
            crosshair_outline: 1.0,
            label: 12.0,
            label_padding: 8.0,
            label_height: 22.0,
            label_radius: 6.0,
            label_gap: 6.0,
        }
    }
}

/// The Bluetooth and Network pages: a status card, a toolbar, the list, and
/// a detail panel beside it. Defaults are the QML menus'.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct DevicePageMetrics {
    /// Between the blocks of the page, and between items in a row.
    pub spacing: f32,
    pub gap: f32,
    /// The status card: padding, headline size, and its line of values.
    pub card_padding: f32,
    pub headline: f32,
    pub strip: f32,
    pub strip_line: f32,
    /// Square icon buttons (disconnect, refresh, scan, close).
    pub icon_button: f32,
    pub icon: f32,
    pub radius: f32,
    pub border: f32,
    /// List rows: height, gap, padding, name size, and its value line.
    pub row_height: f32,
    pub row_gap: f32,
    pub row_padding: f32,
    pub row_title: f32,
    pub row_strip: f32,
    /// The detail panel's width, padding, and its action buttons.
    pub panel_width: f32,
    pub panel_padding: f32,
    pub panel_button: f32,
    /// The panel's password field: text size, padding, caret width.
    pub field_text: f32,
    pub field_padding: f32,
    pub caret_width: f32,
    /// The scrolling name in the panel (QML `MarqueeLabel`): its size, the
    /// gap before it repeats, and its speed in px/s.
    pub marquee: f32,
    pub marquee_gap: f32,
    pub marquee_speed: f32,
    /// The status line at the bottom, and how long a result stays there.
    pub footer: f32,
    pub footer_height: f32,
    pub status_ms: u64,
    /// The loading dot's size and its pulse period.
    pub loading_dot: f32,
    pub loading_ms: u64,
}

impl Default for DevicePageMetrics {
    fn default() -> Self {
        Self {
            spacing: 10.0,
            gap: 8.0,
            card_padding: 10.0,
            headline: 14.0,
            strip: 11.0,
            strip_line: 18.0,
            icon_button: 30.0,
            icon: 14.0,
            radius: 6.0,
            border: 1.0,
            row_height: 56.0,
            row_gap: 6.0,
            row_padding: 8.0,
            row_title: 13.0,
            row_strip: 10.0,
            panel_width: 250.0,
            panel_padding: 10.0,
            panel_button: 30.0,
            field_text: 12.0,
            field_padding: 8.0,
            caret_width: 1.5,
            marquee: 14.0,
            marquee_gap: 24.0,
            marquee_speed: 42.0,
            footer: 11.0,
            footer_height: 22.0,
            status_ms: 3500,
            loading_dot: 8.0,
            loading_ms: 840,
        }
    }
}

/// The Audio page: volume, now playing, and the output and input lists.
/// Defaults are `modules/AudioMenu.qml`'s Playback tab.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AudioPageMetrics {
    pub spacing: f32,
    pub gap: f32,
    /// The volume row and its readout.
    pub slider_height: f32,
    pub readout_width: f32,
    /// The now-playing card: padding, art, and text sizes.
    pub card_padding: f32,
    pub art: f32,
    pub title: f32,
    pub text: f32,
    pub caption: f32,
    pub line: f32,
    /// Previous / play-pause / next.
    pub media_button: f32,
    pub media_icon: f32,
    /// Device rows and the section labels above them.
    pub label: f32,
    pub label_height: f32,
    pub row_height: f32,
    pub row_gap: f32,
    pub row_padding: f32,
    pub row_title: f32,
    pub row_caption: f32,
    /// How long a chosen device shows as selected before the system agrees
    /// (QML's optimistic guard).
    pub pending_ms: u64,
}

impl Default for AudioPageMetrics {
    fn default() -> Self {
        Self {
            spacing: 12.0,
            gap: 10.0,
            slider_height: 24.0,
            readout_width: 56.0,
            card_padding: 12.0,
            art: 96.0,
            title: 15.0,
            text: 12.0,
            caption: 11.0,
            line: 20.0,
            media_button: 32.0,
            media_icon: 14.0,
            label: 12.0,
            label_height: 20.0,
            row_height: 44.0,
            row_gap: 6.0,
            row_padding: 10.0,
            row_title: 13.0,
            row_caption: 10.0,
            pending_ms: 5000,
        }
    }
}

/// The screenshot viewer window. Defaults are `modules/ScreenshotViewer.qml`'s.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ScreenshotViewerMetrics {
    /// Size on open, and the smallest it may be made.
    pub width: u32,
    pub height: u32,
    pub min_width: u32,
    pub min_height: u32,
    /// Around the content, and between rows and between items in a row.
    pub padding: f32,
    pub spacing: f32,
    pub gap: f32,
    pub title: f32,
    pub caption: f32,
    /// Toolbar buttons and their glyphs.
    pub button_width: f32,
    pub button_height: f32,
    pub button_radius: f32,
    pub icon: f32,
    /// The pen and zoom row, its colour swatches, and its sliders.
    pub row_height: f32,
    pub swatch: f32,
    pub swatch_border: f32,
    pub swatch_border_selected: f32,
    pub slider_width: f32,
    /// Width kept for the slider readouts ("16", "800%").
    pub readout_width: f32,
    /// The status banner over the bottom of the image, and how long it stays.
    pub status_height: f32,
    pub status_radius: f32,
    pub status_ms: u64,
    /// Zoom range, the Ctrl+wheel step, and the zoom slider's step.
    pub zoom_min: f32,
    pub zoom_max: f32,
    pub zoom_wheel_step: f32,
    pub zoom_slider_step: f32,
    /// How long the portal's file chooser may take before Save As gives up.
    pub save_as_timeout_ms: u64,
}

impl Default for ScreenshotViewerMetrics {
    fn default() -> Self {
        Self {
            width: 1200,
            height: 820,
            min_width: 820,
            min_height: 560,
            padding: 12.0,
            spacing: 10.0,
            gap: 8.0,
            title: 14.0,
            caption: 11.0,
            button_width: 42.0,
            button_height: 32.0,
            button_radius: 8.0,
            icon: 14.0,
            row_height: 24.0,
            swatch: 20.0,
            swatch_border: 1.0,
            swatch_border_selected: 2.0,
            slider_width: 120.0,
            readout_width: 42.0,
            status_height: 28.0,
            status_radius: 6.0,
            status_ms: 2600,
            zoom_min: 1.0,
            zoom_max: 8.0,
            zoom_wheel_step: 0.15,
            zoom_slider_step: 0.1,
            save_as_timeout_ms: 35_000,
        }
    }
}

/// The Equalizer page of the settings window.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct EqualizerMetrics {
    /// Text and caption sizes, and the height of a line of either.
    pub text: f32,
    pub caption: f32,
    pub line: f32,
    pub gap: f32,
    pub radius: f32,
    pub border: f32,
    /// The response graph.
    pub graph_height: f32,
    pub graph_padding: f32,
    /// Points on the curve, and how far from one a press still grabs it.
    pub point: f32,
    pub point_reach: f32,
    /// Samples along the curve, and its stroke width.
    pub curve_steps: usize,
    pub curve_width: f32,
    pub button_height: f32,
    /// The band's On/Off and Remove buttons, and its filter-type button.
    pub small_button: f32,
    pub type_button: f32,
    /// A labelled slider row: its height, label column, and value column.
    pub slider_row: f32,
    pub label_width: f32,
    pub value_width: f32,
    pub preset_row: f32,
}

impl Default for EqualizerMetrics {
    fn default() -> Self {
        Self {
            text: 13.0,
            caption: 11.0,
            line: 18.0,
            gap: 8.0,
            radius: 6.0,
            border: 1.0,
            graph_height: 150.0,
            graph_padding: 8.0,
            point: 8.0,
            point_reach: 14.0,
            curve_steps: 96,
            curve_width: 2.0,
            button_height: 28.0,
            small_button: 64.0,
            type_button: 86.0,
            slider_row: 24.0,
            label_width: 52.0,
            value_width: 64.0,
            preset_row: 26.0,
        }
    }
}

/// The hover menus beside the status icons.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct HoverMenuMetrics {
    /// Milliseconds the pointer must rest on an icon before its menu opens.
    pub open_ms: u64,
    /// Milliseconds with neither the icon nor the menu hovered before it
    /// closes; long enough to cross the gap between them.
    pub close_ms: u64,
    pub width: f32,
    /// Gap between the rail and the menu.
    pub offset: f32,
    pub padding: f32,
    pub radius: f32,
    pub border: f32,
    pub title: f32,
    pub text: f32,
    pub caption: f32,
    pub icon: f32,
    /// Line height as a multiple of the text size.
    pub line_height: f32,
    pub row_height: f32,
    pub row_gap: f32,
    pub section_gap: f32,
    pub gap: f32,
    pub close: f32,
    pub button_height: f32,
    pub button_radius: f32,
    /// Width reserved for the volume percentage.
    pub value_width: f32,
    /// Where values start in a label/value row, as a fraction of the width.
    pub stat_split: f32,
    /// Bluetooth devices listed before "+N more".
    pub max_devices: usize,
}

impl Default for HoverMenuMetrics {
    fn default() -> Self {
        Self {
            open_ms: 350,
            close_ms: 800,
            width: 280.0,
            offset: 4.0,
            padding: 12.0,
            radius: 12.0,
            border: 1.0,
            title: 14.0,
            text: 12.0,
            caption: 11.0,
            icon: 16.0,
            line_height: 1.45,
            row_height: 28.0,
            row_gap: 2.0,
            section_gap: 10.0,
            gap: 6.0,
            close: 20.0,
            button_height: 28.0,
            button_radius: 6.0,
            value_width: 38.0,
            stat_split: 0.38,
            max_devices: 6,
        }
    }
}

/// A tray item's context menu.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct TrayMenuMetrics {
    /// Width of the menu panel.
    pub width: f32,
    /// Height of one row.
    pub row_height: f32,
    /// Inner padding.
    pub padding: f32,
    /// Corner radius.
    pub radius: f32,
    /// Extra width added per nesting level.
    pub indent: f32,
    /// Font size for entries.
    pub text: f32,
    /// Maximum rows drawn before the list is truncated.
    pub max_rows: usize,
    /// Distance from the screen edge. The menu is anchored top-left, so the
    /// left and top margins place it; the others are kept for symmetry.
    pub margin_top: i32,
    pub margin_right: i32,
    pub margin_bottom: i32,
    pub margin_left: i32,
    /// Milliseconds after the pointer leaves both the menu and its icon
    /// before the menu closes (QML: 150).
    pub close_ms: u64,
}

impl Default for TrayMenuMetrics {
    fn default() -> Self {
        Self {
            width: 200.0,
            row_height: 28.0,
            padding: 6.0,
            radius: 6.0,
            indent: 12.0,
            text: 13.0,
            max_rows: 24,
            // Clears a 40 px vertical rail; a user with a top bar will set
            // margin_top instead.
            margin_top: 0,
            margin_right: 0,
            margin_bottom: 0,
            margin_left: 44,
            close_ms: 150,
        }
    }
}

/// Notification cards.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ToastMetrics {
    /// Surface width; cards are slightly narrower.
    pub surface_width: u32,
    /// Margin between the stack and the screen edge.
    pub margin: i32,
    /// Card width.
    pub card_width: f32,
    /// Gap between cards.
    pub gap: f32,
    /// Height of the card's header area.
    pub header_height: f32,
    /// Height added per body line.
    pub body_line_height: f32,
    /// Corner radius.
    pub radius: f32,
    /// Accent stripe width.
    pub stripe_width: f32,
    /// Icon size.
    pub icon: f32,
    /// Title size.
    pub title: f32,
    /// Body size.
    pub body: f32,
    /// Seconds a notification stays when the server specifies no timeout.
    pub default_seconds: f32,
    /// Maximum cards on screen at once.
    pub max_cards: usize,
    /// Slide-in distance.
    pub slide_distance: f32,
    /// Reveal duration in seconds.
    pub reveal_seconds: f32,
}

impl Default for ToastMetrics {
    fn default() -> Self {
        Self {
            surface_width: 344,
            margin: 12,
            card_width: 320.0,
            gap: 8.0,
            header_height: 52.0,
            body_line_height: 16.0,
            radius: 10.0,
            stripe_width: 3.0,
            icon: 12.0,
            title: 13.0,
            body: 11.0,
            default_seconds: 5.0,
            max_cards: 5,
            slide_distance: 24.0,
            reveal_seconds: 0.18,
        }
    }
}

/// The settings window and its navigation rail.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct WindowMetrics {
    /// Initial window width.
    pub width: u32,
    /// Initial window height.
    pub height: u32,
    /// Minimum window size.
    pub min_width: u32,
    pub min_height: u32,
    /// Navigation rail width.
    pub nav_width: f32,
    /// Height of the rail's header.
    pub nav_header: f32,
    /// Height of one navigation row.
    pub nav_row_height: f32,
    /// Vertical pitch between navigation rows.
    pub nav_row_spacing: f32,
    /// Inset of each row from the rail edge.
    pub nav_inset: f32,
    /// Gap between the header and the first row.
    pub nav_top_gap: f32,
    /// Radius of a selected row.
    pub nav_row_radius: f32,
    /// Padding inside the content area.
    pub content_padding: f32,
    /// Height reserved for a page heading.
    pub heading_height: f32,
}

impl Default for WindowMetrics {
    fn default() -> Self {
        Self {
            width: 520,
            height: 640,
            min_width: 360,
            min_height: 320,
            nav_width: 150.0,
            nav_header: 52.0,
            nav_row_height: 36.0,
            nav_row_spacing: 40.0,
            nav_inset: 8.0,
            nav_top_gap: 6.0,
            nav_row_radius: 8.0,
            content_padding: 20.0,
            heading_height: 40.0,
        }
    }
}

/// Sizing shared by control-centre pages.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct PageMetrics {
    /// Padding inside a card.
    pub card_padding: f32,
    /// Radius of a card or tile.
    pub card_radius: f32,
    /// Height of a normal list row.
    pub row_height: f32,
    /// Vertical pitch between list rows.
    pub row_spacing: f32,
    /// Height of a stat row.
    pub stat_height: f32,
    /// Vertical pitch between stat rows.
    pub stat_spacing: f32,
    /// Height of a button.
    pub button_height: f32,
    /// Radius of a button.
    pub button_radius: f32,
    /// Width of a wide button.
    pub button_width: f32,
    /// Track height of a slider.
    pub slider_track: f32,
    /// Diameter of a slider knob.
    pub slider_knob: f32,
    /// Width of a toggle switch.
    pub toggle_width: f32,
    /// Height of a toggle switch.
    pub toggle_height: f32,
    /// Gap between a section label and its content.
    pub section_gap: f32,
    /// Height of a wording row.
    pub section_label_height: f32,
}

impl Default for PageMetrics {
    fn default() -> Self {
        Self {
            card_padding: 14.0,
            card_radius: 10.0,
            row_height: 40.0,
            row_spacing: 44.0,
            stat_height: 32.0,
            stat_spacing: 38.0,
            button_height: 32.0,
            button_radius: 8.0,
            button_width: 140.0,
            slider_track: 6.0,
            slider_knob: 14.0,
            toggle_width: 44.0,
            toggle_height: 24.0,
            section_gap: 24.0,
            section_label_height: 20.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_sizes_follow_their_parts() {
        let metrics = WorkspacesMetrics {
            visible: 5,
            row_height: 20.0,
            gap: 3.0,
            ..Default::default()
        };
        assert_eq!(metrics.step(), 23.0);
        // Five rows and four gaps, not five gaps.
        assert_eq!(metrics.window_height(), 112.0);
    }

    #[test]
    fn defaults_are_parseable_from_empty_toml() {
        let metrics: Metrics = toml::from_str("").expect("defaults should parse");
        assert_eq!(metrics.workspaces.visible, 5);
        assert_eq!(metrics.typography.icon, 20.0);
    }

    #[test]
    fn partial_tables_keep_other_defaults() {
        let metrics: Metrics = toml::from_str(
            r#"
            [workspaces]
            visible = 7
            row_height = 24.0
            "#,
        )
        .expect("partial table should parse");
        assert_eq!(metrics.workspaces.visible, 7);
        assert_eq!(metrics.workspaces.row_height, 24.0);
        // Untouched fields keep their defaults.
        assert_eq!(metrics.workspaces.gap, 3.0);
        assert_eq!(metrics.typography.workspace, 15.0);
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let result: Result<Metrics, _> = toml::from_str(
            r#"
            [workspaces]
            row_heigth = 24.0
            "#,
        );
        assert!(result.is_err(), "a typo must not silently do nothing");
    }
}
