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
}

impl Default for BarMetrics {
    fn default() -> Self {
        Self {
            launcher: 24.0,
            group_margin: 6.0,
            group_spacing: 4.0,
            group_width: 30.0,
            corner_size: 20.0,
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
            icon: 22.0,
            title: 13.0,
            value: 12.0,
            hold_seconds: 1.2,
            fade_in_seconds: 0.13,
            fade_out_seconds: 0.22,
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
