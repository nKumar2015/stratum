//! Layout metrics: every size the shell draws with.
//!
//! Colours live in [`crate::config::Theme`]; this module is the other half of
//! styling. Nothing here knows how to draw — it is pure data, deserialised from
//! the `[metrics]` table in the config file, so adjusting a row height or a font
//! size means editing one number rather than hunting through widget code.
//!
//! Widgets read these through `PaintCtx::metrics`. Every field is required:
//! the defaults, which reproduce the QML shell's proportions, live only in
//! `config.default.toml` (see [`crate::config`]).

use serde::{Deserialize, Serialize};

/// Everything sizing-related.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
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

/// Font sizes by role. All in logical pixels.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
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

/// Sizing shared by the sidebar and any container inside it.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
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

/// The scrolling workspace strip.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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

/// The volume/brightness overlay.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
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

/// The power menu overlay.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
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

/// The lock screen. Defaults follow `modules/LockScreen.qml`.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
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

/// The polkit authentication dialog. Defaults follow `modules/PolkitDialog.qml`.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
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

/// A notification card, shared by the toasts and the notifications page.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
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

/// The theme switcher overlay. Defaults follow `modules/ThemeSwitcher.qml`.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
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

/// The dashboard overlay.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DashboardMetrics {
    /// Distance from the top of the screen, and the card's padding.
    pub margin: f32,
    pub padding: f32,
    /// Gap between panels, and padding inside one.
    pub gap: f32,
    pub inner: f32,
    pub radius: f32,
    pub border: f32,
    /// The card, padding included; the same on every tab, so switching tabs
    /// does not resize it.
    pub width: f32,
    pub panel_height: f32,
    /// The tab bar above the panels, and the gap between its tabs.
    pub tab_height: f32,
    pub tab_gap: f32,
    /// A history graph's least height (graphs grow to fill their panel),
    /// the least width of a column of them, its line width, the opacity of
    /// the area under the line, and the gap between graphs.
    pub graph: f32,
    pub graph_column: f32,
    pub graph_line: f32,
    pub graph_fill: f32,
    pub graph_gap: f32,
    /// The glance strip's time, and the weather panel's current glyph and
    /// temperature.
    pub clock: f32,
    pub weather_icon: f32,
    pub weather_temp: f32,
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

impl DashboardMetrics {
    /// Every length times `s`; opacities kept. Destructured whole, so a new
    /// field cannot be left out by accident.
    pub fn scaled(&self, s: f32) -> Self {
        let Self {
            margin,
            padding,
            gap,
            inner,
            radius,
            border,
            width,
            panel_height,
            tab_height,
            tab_gap,
            graph,
            graph_column,
            graph_line,
            graph_fill,
            graph_gap,
            clock,
            weather_icon,
            weather_temp,
            title,
            text,
            caption,
            line,
            bar,
            cell,
            cell_gap,
            button,
            art,
            scrim_alpha,
        } = self.clone();
        Self {
            margin: margin * s,
            padding: padding * s,
            gap: gap * s,
            inner: inner * s,
            radius: radius * s,
            border: border * s,
            width: width * s,
            panel_height: panel_height * s,
            tab_height: tab_height * s,
            tab_gap: tab_gap * s,
            graph: graph * s,
            graph_column: graph_column * s,
            graph_line: graph_line * s,
            graph_fill,
            graph_gap: graph_gap * s,
            clock: clock * s,
            weather_icon: weather_icon * s,
            weather_temp: weather_temp * s,
            title: title * s,
            text: text * s,
            caption: caption * s,
            line: line * s,
            bar: bar * s,
            cell: cell * s,
            cell_gap: cell_gap * s,
            button: button * s,
            art: art * s,
            scrim_alpha,
        }
    }

    /// How much to scale for a `width` × `height` surface: enough for the
    /// card to span `fraction` of the width, no more than keeps it (with its
    /// top margin and as much below) inside the height, and never below 1.
    pub fn scale_for(&self, width: f32, height: f32, fraction: f32) -> f32 {
        let card = self.padding * 2.0 + self.tab_height + self.gap + self.panel_height;
        let across = fraction * width / self.width;
        let down = height / (card + self.margin * 2.0);
        across.min(down).max(1.0)
    }
}

/// The screenshot overlay. Defaults are `modules/ScreenshotToolbar.qml`'s.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
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

/// The Bluetooth and Network pages: a status card, a toolbar, the list, and
/// a detail panel beside it. Defaults are the QML menus'.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DevicePageMetrics {
    /// Between the loading dot and its label.
    pub gap: f32,
    /// A device or network row, and what an expanded row adds below it.
    pub row_height: f32,
    pub expanded: f32,
    /// The password field: text size, padding, caret width, corner radius
    /// and border.
    pub field_text: f32,
    pub field_padding: f32,
    pub caret_width: f32,
    pub radius: f32,
    pub border: f32,
    /// A scrolling title (QML `MarqueeLabel`): the gap before it repeats,
    /// and its speed in px/s.
    pub marquee_gap: f32,
    pub marquee_speed: f32,
    /// The Audio page's status line, and how long a result stays shown.
    pub footer_height: f32,
    pub status_ms: u64,
    /// The loading dot's size and its pulse period.
    pub loading_dot: f32,
    pub loading_ms: u64,
}

/// The Audio page: volume, now playing, and the output and input lists.
/// Defaults are `modules/AudioMenu.qml`'s Playback tab.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AudioPageMetrics {
    /// Between controls in a row (slider and readout, media buttons).
    pub gap: f32,
    /// Volume cards: the narrowest before output and input stack, and their
    /// height.
    pub volume_min_width: f32,
    pub volume_height: f32,
    /// A volume slider and its readout.
    pub slider_height: f32,
    pub readout_width: f32,
    /// The now-playing card: art, and text sizes.
    pub art: f32,
    pub title: f32,
    pub text: f32,
    pub caption: f32,
    pub line: f32,
    /// Previous / play-pause / next.
    pub media_button: f32,
    pub media_icon: f32,
    /// Height of an output or input device row.
    pub device_height: f32,
    /// How long a chosen device shows as selected before the system agrees
    /// (QML's optimistic guard).
    pub pending_ms: u64,
}

/// The screenshot viewer window. Defaults are `modules/ScreenshotViewer.qml`'s.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
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

/// The Equalizer page of the settings window.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EqualizerMetrics {
    /// The name and value fields' text, and the slider labels.
    pub text: f32,
    pub caption: f32,
    /// Between the selected band's header and its sliders, and between
    /// sliders.
    pub gap: f32,
    /// The value and name fields: corner radius and border.
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
    /// A labelled slider row: its height, label column, and value column.
    pub slider_row: f32,
    pub label_width: f32,
    pub value_width: f32,
    /// Preset cards: the narrowest before the grid drops a column, and
    /// their height.
    pub preset_min_width: f32,
    pub preset_height: f32,
}

/// The hover menus beside the status icons.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
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

/// A tray item's context menu.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TrayMenuMetrics {
    /// The narrowest and widest the panel may be. In between, it fits its
    /// widest label, as QML's did; past `max_width` labels are cut short.
    pub width: f32,
    pub max_width: f32,
    /// Space before a label, and kept free after the widest one.
    pub label_inset: f32,
    /// Room for the marker after an entry with a submenu.
    pub chevron_width: f32,
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
    /// Milliseconds after the pointer leaves both the menu and its icon
    /// before the menu closes (QML: 150).
    pub close_ms: u64,
}

/// Notification cards.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
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

/// The settings window and its navigation rail.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
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
    /// Left inset of the rail's "Settings" title.
    pub nav_title_x: f32,
    /// Offsets of a row's glyph and label from the row's left edge.
    pub nav_icon_x: f32,
    pub nav_label_x: f32,
    /// Padding inside the content area.
    pub content_padding: f32,
    /// Distance one wheel notch scrolls a page.
    pub scroll_step: f32,
}

/// Sizing shared by control-centre pages.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PageMetrics {
    /// Padding inside a card.
    pub card_padding: f32,
    /// Radius of a card or tile.
    pub card_radius: f32,
    /// Height of a stat row (the Display page's preview).
    pub stat_height: f32,
    /// Height of a button.
    pub button_height: f32,
    /// Radius of a button.
    pub button_radius: f32,
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
    /// Side of a colour swatch (theme rows, the preview's status dots).
    pub swatch: f32,
    /// Gap between swatches.
    pub swatch_gap: f32,
    /// Height of the Display page's theme preview card.
    pub preview_height: f32,
    /// Widest the page column grows; a wider window centres it.
    pub max_width: f32,
    /// The page header: title and summary sizes, the gap between them, and
    /// the space below the header.
    pub title: f32,
    pub summary: f32,
    pub header_gap: f32,
    pub header_bottom: f32,
    /// Between sections of a page, and between a section label and its cards.
    pub section_spacing: f32,
    pub label_gap: f32,
    /// Card grids: the gap between cells and the narrowest a card may be
    /// before the grid drops a column.
    pub grid_gap: f32,
    pub card_min_width: f32,
    /// Status cards: height, the round badge behind the glyph, glyph size,
    /// the large value, and the meter along the bottom. Cards with the badge
    /// above their text grow past their height to keep `badge_gap` below it.
    pub stat_card_height: f32,
    pub badge: f32,
    pub badge_icon: f32,
    pub badge_gap: f32,
    pub value: f32,
    pub meter: f32,
    /// Quick toggle tiles: narrowest tile, and its height.
    pub tile_min_width: f32,
    pub tile_height: f32,
    /// Action tiles (the Power page): narrowest tile, and its height.
    pub action_min_width: f32,
    pub action_height: f32,
    /// Info cards (a label over a value).
    pub info_height: f32,
    /// The Battery page's charge card: height, the large percentage, and
    /// the thickness of its meter.
    pub hero_height: f32,
    pub hero_value: f32,
    pub hero_meter: f32,
    /// Display page theme cards: narrowest card, and its height.
    pub theme_min_width: f32,
    pub theme_height: f32,
    /// The card a page shows when it has nothing to list.
    pub empty_height: f32,
    /// How long an action tile waits for its confirming second click.
    pub confirm_ms: u64,
    /// Outline width of a card, and of a highlighted one.
    pub outline: f32,
    pub outline_active: f32,
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
            ..crate::config::Config::test_defaults().metrics.workspaces
        };
        assert_eq!(metrics.step(), 23.0);
        // Five rows and four gaps, not five gaps.
        assert_eq!(metrics.window_height(), 112.0);
    }

    #[test]
    fn a_missing_field_is_an_error() {
        let result: Result<TrayMetrics, _> = toml::from_str("cell = 24.0\ngap = 4.0\nvertical_limit = 6\n");
        assert!(result.is_err(), "the binary has no default to fill it with");
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let result: Result<TrayMetrics, _> = toml::from_str(
            "cell = 24.0\ngap = 4.0\nvertical_limit = 6\nhorizontal_limit = 8\ncel = 1.0\n",
        );
        assert!(result.is_err(), "a typo must not silently do nothing");
    }
}
