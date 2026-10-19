//! User configuration.
//!
//! The shell reads a single TOML file. Everything visual that a user is likely
//! to want to change lives here: colours, fonts, which widgets appear, their
//! order and spacing, and window placement.
//!
//! Resolution order (first match wins):
//!   1. `$STRATUM_CONFIG`
//!   2. `$XDG_CONFIG_HOME/stratum/config.toml`
//!   3. `~/.config/stratum/config.toml`
//!
//! A missing file is not an error: the built-in defaults are a complete,
//! sensible configuration (the Carbon theme and the standard sidebar).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::render::display_list::Color;

/// The whole configuration file.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Name of the active theme from [`Config::themes`].
    pub theme: String,
    /// Theme definitions. Colour values are `#rrggbb` or `#rrggbbaa`.
    pub themes: BTreeMap<String, ThemeConfig>,
    pub sidebar: SidebarConfig,
    pub appearance: AppearanceConfig,
    pub behavior: BehaviorConfig,
    pub control_center: ControlCenterConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: "carbon".to_string(),
            themes: builtin_themes(),
            sidebar: SidebarConfig::default(),
            appearance: AppearanceConfig::default(),
            behavior: BehaviorConfig::default(),
            control_center: ControlCenterConfig::default(),
        }
    }
}

impl Config {
    /// Load the configuration, falling back to defaults when no file exists.
    ///
    /// Parse errors are returned rather than silently ignored: a typo in a
    /// colour should be visible, not mysteriously ignored.
    pub fn load() -> Result<(Self, Option<PathBuf>), ConfigError> {
        let Some(path) = config_path() else {
            return Ok((Self::default(), None));
        };
        if !path.is_file() {
            return Ok((Self::default(), None));
        }
        let text = std::fs::read_to_string(&path).map_err(|err| ConfigError::Io {
            path: path.clone(),
            message: err.to_string(),
        })?;
        let config: Config = toml::from_str(&text).map_err(|err| ConfigError::Parse {
            path: path.clone(),
            message: err.to_string(),
        })?;
        Ok((config, Some(path)))
    }

    /// Resolve the active theme, falling back to the default when the name is
    /// unknown (a bad theme name must not blank the shell).
    pub fn active_theme(&self) -> Theme {
        let named = self
            .themes
            .get(&self.theme)
            .or_else(|| self.themes.get(&self.theme.to_lowercase()));
        match named {
            Some(theme) => theme.to_theme(),
            None => Theme::default(),
        }
    }
}

/// One named theme.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeConfig {
    pub bg_main: String,
    pub bg_widget: String,
    pub bg_hover: String,
    pub bg_dark: String,
    pub text_main: String,
    pub text_muted: String,
    pub primary: String,
    pub secondary: String,
    pub tertiary: String,
    pub success: String,
    pub warning: String,
    pub error: String,
    pub border_active: String,
    pub border_inactive: String,
    pub font: String,

    // Optional Material-style aliases. Each falls back to an existing colour,
    // so a config only needs to set the ones it cares about. This is the
    // vocabulary loaf-fm uses and makes multi-surface theming coherent.
    #[serde(default)]
    pub surface_lowest: Option<String>,
    #[serde(default)]
    pub surface_low: Option<String>,
    #[serde(default)]
    pub surface: Option<String>,
    #[serde(default)]
    pub surface_high: Option<String>,
    #[serde(default)]
    pub surface_highest: Option<String>,
    #[serde(default)]
    pub outline: Option<String>,
    #[serde(default)]
    pub on_primary: Option<String>,
    #[serde(default)]
    pub on_surface: Option<String>,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            bg_main: "#161616".into(),
            bg_widget: "#262626".into(),
            bg_hover: "#393939".into(),
            bg_dark: "#15161e".into(),
            text_main: "#f4f4f4".into(),
            text_muted: "#8d8d8d".into(),
            primary: "#4589ff".into(),
            secondary: "#a56eff".into(),
            tertiary: "#33b1ff".into(),
            success: "#42be64".into(),
            warning: "#f1c21b".into(),
            error: "#fa4d56".into(),
            border_active: "#4589ff".into(),
            border_inactive: "#393939".into(),
            font: "JetBrainsMono NFM".into(),
            surface_lowest: None,
            surface_low: None,
            surface: None,
            surface_high: None,
            surface_highest: None,
            outline: None,
            on_primary: None,
            on_surface: None,
        }
    }
}

impl ThemeConfig {
    fn to_theme(&self) -> Theme {
        // Fallbacks are literal colours: calling `Theme::default()` here would
        // recurse, because the default theme is built from a `ThemeConfig`.
        let f = Color::BLACK;
        Theme {
            bg_main: parse_color(&self.bg_main, f),
            bg_widget: parse_color(&self.bg_widget, f),
            bg_hover: parse_color(&self.bg_hover, f),
            bg_dark: parse_color(&self.bg_dark, f),
            text_main: parse_color(&self.text_main, f),
            text_muted: parse_color(&self.text_muted, f),
            primary: parse_color(&self.primary, f),
            secondary: parse_color(&self.secondary, f),
            tertiary: parse_color(&self.tertiary, f),
            success: parse_color(&self.success, f),
            warning: parse_color(&self.warning, f),
            error: parse_color(&self.error, f),
            border_active: parse_color(&self.border_active, f),
            border_inactive: parse_color(&self.border_inactive, f),
            font: self.font.clone(),
            surface_lowest: parse_color(
                self.surface_lowest.as_deref().unwrap_or(&self.bg_dark),
                f,
            ),
            surface_low: parse_color(self.surface_low.as_deref().unwrap_or(&self.bg_main), f),
            surface: parse_color(self.surface.as_deref().unwrap_or(&self.bg_main), f),
            surface_high: parse_color(self.surface_high.as_deref().unwrap_or(&self.bg_widget), f),
            surface_highest: parse_color(
                self.surface_highest.as_deref().unwrap_or(&self.bg_hover),
                f,
            ),
            outline: parse_color(
                self.outline.as_deref().unwrap_or(&self.border_inactive),
                f,
            ),
            on_primary: parse_color(self.on_primary.as_deref().unwrap_or(&self.bg_main), f),
            on_surface: parse_color(self.on_surface.as_deref().unwrap_or(&self.text_main), f),
        }
    }
}

/// Bar layout: placement, orientation, and which widgets to show, in order.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct SidebarConfig {
    /// Which screen edge the bar occupies.
    pub position: SidebarPosition,
    /// Orientation. `vertical` is the classic rail; `horizontal` is a top bar.
    /// When omitted it is inferred from `position`.
    pub axis: Option<Axis>,
    /// Rail width (vertical) in logical pixels.
    pub width: u32,
    /// Bar height (horizontal) in logical pixels.
    pub height: u32,
    /// Padding at both ends of the widget run.
    pub padding: u32,
    /// Gap between widgets.
    pub spacing: u32,
    /// Widgets to render, top to bottom. Removing an entry hides that widget.
    pub widgets: Vec<WidgetConfig>,
    /// Draw the scooped corner decoration where the rail meets the screen edge.
    pub inverted_corners: bool,
}

impl Default for SidebarConfig {
    fn default() -> Self {
        Self {
            position: SidebarPosition::Left,
            axis: None,
            width: 40,
            height: 32,
            padding: 12,
            spacing: 16,
            widgets: vec![
                WidgetConfig::simple(WidgetKind::Launcher),
                WidgetConfig::simple(WidgetKind::Workspaces),
                WidgetConfig::simple(WidgetKind::AppTitle),
                WidgetConfig::simple(WidgetKind::Tray),
                WidgetConfig::group(
                    WidgetKind::QuickPanel,
                    vec![
                        WidgetConfig::simple(WidgetKind::Audio),
                        WidgetConfig::simple(WidgetKind::Bluetooth),
                        WidgetConfig::simple(WidgetKind::Wifi),
                    ],
                ),
                WidgetConfig::simple(WidgetKind::Battery),
                WidgetConfig::simple(WidgetKind::Clock),
            ],
            inverted_corners: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SidebarPosition {
    #[default]
    Left,
    Right,
    Top,
    Bottom,
}

/// Orientation of the bar.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Axis {
    Vertical,
    Horizontal,
}

impl SidebarConfig {
    /// Effective orientation: explicit `axis`, else inferred from `position`.
    pub fn axis(&self) -> Axis {
        self.axis.unwrap_or(match self.position {
            SidebarPosition::Left | SidebarPosition::Right => Axis::Vertical,
            SidebarPosition::Top | SidebarPosition::Bottom => Axis::Horizontal,
        })
    }

    pub fn is_vertical(&self) -> bool {
        self.axis() == Axis::Vertical
    }
}

/// One widget entry in the sidebar.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WidgetConfig {
    pub kind: WidgetKind,
    /// Background box drawn behind the widget (the "quick panel" look).
    #[serde(default)]
    pub framed: bool,
    /// Vertical space this widget reserves. `None` uses the widget's intrinsic
    /// size, which is what most widgets want.
    #[serde(default)]
    pub height: Option<u32>,
    /// Children, for container widgets only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<WidgetConfig>,
}

impl WidgetConfig {
    fn simple(kind: WidgetKind) -> Self {
        Self {
            kind,
            framed: false,
            height: None,
            children: Vec::new(),
        }
    }

    fn group(kind: WidgetKind, children: Vec<WidgetConfig>) -> Self {
        Self {
            kind,
            framed: true,
            height: Some(86),
            children,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WidgetKind {
    Launcher,
    Workspaces,
    AppTitle,
    Tray,
    QuickPanel,
    Audio,
    Bluetooth,
    Wifi,
    Battery,
    Clock,
    Spacer,
}

/// Look-and-feel knobs that are not colours.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppearanceConfig {
    /// Base font size for body text, in logical pixels.
    pub font_size: f32,
    /// Corner radius used by framed widgets and menus.
    pub corner_radius: f32,
    /// Duration of hover and open/close animations, in milliseconds. Zero
    /// disables animation.
    pub animation_ms: u32,
    /// Draw a 1px border around framed widgets.
    pub framed_border: bool,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            font_size: 13.0,
            corner_radius: 15.0,
            animation_ms: 180,
            framed_border: true,
        }
    }
}

/// The paged settings surface.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ControlCenterConfig {
    /// Panel width in logical pixels.
    pub width: u32,
    /// Slide/fade duration in milliseconds. Zero opens instantly.
    pub animation_ms: u32,
    /// Which page to show when opened by something without a page of its own.
    pub default_page: String,
}

impl Default for ControlCenterConfig {
    fn default() -> Self {
        Self {
            width: 520,
            animation_ms: 160,
            default_page: "overview".into(),
        }
    }
}

/// Runtime behaviour.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct BehaviorConfig {
    /// Poll interval for widgets that are not push-driven, in milliseconds.
    pub poll_ms: u64,
    /// Watch the config file and reload on change.
    pub reload_on_change: bool,
    /// Own `org.kde.StatusNotifierWatcher` and display tray icons. Off by
    /// default for the same reason as notifications: only one host can.
    pub tray: bool,
    /// Own `org.freedesktop.Notifications`. Off by default so the shell can run
    /// beside another notification daemon (including Quickshell) during
    /// migration; enable it once this shell is the only one.
    pub notifications: bool,
}

impl Default for BehaviorConfig {
    fn default() -> Self {
        Self {
            poll_ms: 3000,
            reload_on_change: true,
            tray: false,
            notifications: false,
        }
    }
}

/// A resolved, ready-to-draw palette.
#[derive(Clone, Debug)]
pub struct Theme {
    pub bg_main: Color,
    pub bg_widget: Color,
    pub bg_hover: Color,
    pub bg_dark: Color,
    pub text_main: Color,
    pub text_muted: Color,
    pub primary: Color,
    pub secondary: Color,
    pub tertiary: Color,
    pub success: Color,
    pub warning: Color,
    pub error: Color,
    pub border_active: Color,
    pub border_inactive: Color,
    pub font: String,

    /// Material-style surface ramp: lowest (deepest) to highest (most raised).
    pub surface_lowest: Color,
    pub surface_low: Color,
    pub surface: Color,
    pub surface_high: Color,
    pub surface_highest: Color,
    /// Hairline colour for dividers and frame borders.
    pub outline: Color,
    /// Foreground that reads on `primary` (text on filled buttons).
    pub on_primary: Color,
    /// Foreground that reads on surface colours.
    pub on_surface: Color,
}

impl Default for Theme {
    fn default() -> Self {
        ThemeConfig::default().to_theme()
    }
}

/// Parse `#rgb`, `#rrggbb`, or `#rrggbbaa`, falling back when malformed.
fn parse_color(value: &str, fallback: Color) -> Color {
    let hex = value.trim().trim_start_matches('#');
    let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    match hex.len() {
        3 => {
            let expand = |c: u8| c * 17;
            let r = u8::from_str_radix(&hex[0..1], 16).ok().map(expand);
            let g = u8::from_str_radix(&hex[1..2], 16).ok().map(expand);
            let b = u8::from_str_radix(&hex[2..3], 16).ok().map(expand);
            match (r, g, b) {
                (Some(r), Some(g), Some(b)) => {
                    Color::rgba(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0)
                }
                _ => fallback,
            }
        }
        6 | 8 => {
            let (r, g, b) = (channel(0), channel(2), channel(4));
            let a = if hex.len() == 8 { channel(6) } else { Some(255) };
            match (r, g, b, a) {
                (Some(r), Some(g), Some(b), Some(a)) => Color::rgba(
                    r as f32 / 255.0,
                    g as f32 / 255.0,
                    b as f32 / 255.0,
                    a as f32 / 255.0,
                ),
                _ => fallback,
            }
        }
        _ => fallback,
    }
}

fn config_path() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("STRATUM_CONFIG") {
        if !explicit.is_empty() {
            return Some(PathBuf::from(explicit));
        }
    }
    // XDG_CONFIG_HOME when set, else ~/.config — both per the XDG base spec.
    if let Ok(config_home) = std::env::var("XDG_CONFIG_HOME") {
        if !config_home.is_empty() {
            return Some(Path::new(&config_home).join("stratum/config.toml"));
        }
    }
    std::env::var("HOME")
        .ok()
        .map(|home| Path::new(&home).join(".config/stratum/config.toml"))
}

/// The four themes the QML shell shipped with, keyed by lowercase name.
fn builtin_themes() -> BTreeMap<String, ThemeConfig> {
    let mut themes = BTreeMap::new();
    themes.insert(
        "carbon".into(),
        ThemeConfig {
            bg_main: "#161616".into(),
            bg_widget: "#262626".into(),
            bg_hover: "#393939".into(),
            bg_dark: "#15161e".into(),
            text_main: "#f4f4f4".into(),
            text_muted: "#8d8d8d".into(),
            primary: "#4589ff".into(),
            secondary: "#a56eff".into(),
            tertiary: "#33b1ff".into(),
            success: "#42be64".into(),
            warning: "#f1c21b".into(),
            error: "#fa4d56".into(),
            border_active: "#4589ff".into(),
            border_inactive: "#393939".into(),
            font: "JetBrainsMono NFM".into(),
            ..ThemeConfig::default()
        },
    );
    themes.insert(
        "gruvbox".into(),
        ThemeConfig {
            bg_main: "#1d2021".into(),
            bg_widget: "#282828".into(),
            bg_hover: "#3c3836".into(),
            bg_dark: "#141617".into(),
            text_main: "#ebdbb2".into(),
            text_muted: "#a89984".into(),
            primary: "#83a598".into(),
            secondary: "#d3869b".into(),
            tertiary: "#fe8019".into(),
            success: "#b8bb26".into(),
            warning: "#fabd2f".into(),
            error: "#fb4934".into(),
            border_active: "#83a598".into(),
            border_inactive: "#141617".into(),
            font: "JetBrainsMono NFM".into(),
            ..ThemeConfig::default()
        },
    );
    themes.insert(
        "rosepine".into(),
        ThemeConfig {
            bg_main: "#1f1d2e".into(),
            bg_widget: "#26233a".into(),
            bg_hover: "#403d52".into(),
            bg_dark: "#191724".into(),
            text_main: "#e0def4".into(),
            text_muted: "#908caa".into(),
            primary: "#c4a7e7".into(),
            secondary: "#ebbcba".into(),
            tertiary: "#31748f".into(),
            success: "#9ccfd8".into(),
            warning: "#f6c177".into(),
            error: "#eb6f92".into(),
            border_active: "#c4a7e7".into(),
            border_inactive: "#191724".into(),
            font: "JetBrainsMono NFM".into(),
            ..ThemeConfig::default()
        },
    );
    themes.insert(
        "tokyonight".into(),
        ThemeConfig {
            bg_main: "#1a1b26".into(),
            bg_widget: "#1f2335".into(),
            bg_hover: "#292e42".into(),
            bg_dark: "#15161e".into(),
            text_main: "#c0caf5".into(),
            text_muted: "#565f89".into(),
            primary: "#7aa2f7".into(),
            secondary: "#bb9af7".into(),
            tertiary: "#7dcfff".into(),
            success: "#9ece6a".into(),
            warning: "#e0af68".into(),
            error: "#f7768e".into(),
            border_active: "#7aa2f7".into(),
            border_inactive: "#15161e".into(),
            font: "JetBrainsMono NFM".into(),
            ..ThemeConfig::default()
        },
    );
    themes
}

#[derive(Debug)]
pub enum ConfigError {
    Io { path: PathBuf, message: String },
    Parse { path: PathBuf, message: String },
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io { path, message } => {
                write!(f, "failed to read {}: {}", path.display(), message)
            }
            ConfigError::Parse { path, message } => {
                write!(f, "invalid config at {}:\n{}", path.display(), message)
            }
        }
    }
}

impl std::error::Error for ConfigError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_cover_every_widget() {
        let config = Config::default();
        assert_eq!(config.theme, "carbon");
        assert!(config.themes.contains_key("gruvbox"));
        assert!(config.sidebar.widgets.iter().any(|w| w.kind == WidgetKind::Clock));
    }

    #[test]
    fn parses_hex_with_and_without_alpha() {
        let fallback = crate::render::display_list::Color::WHITE;
        let c = parse_color("#ff0000", fallback);
        assert_eq!(c.r, 1.0);
        assert_eq!(c.a, 1.0);

        let c = parse_color("#00ff0080", fallback);
        assert!((c.a - 0.502).abs() < 0.01);

        let c = parse_color("#0f0", fallback);
        assert_eq!(c.g, 1.0);
    }

    #[test]
    fn malformed_colour_falls_back() {
        let fallback = crate::render::display_list::Color::BLACK;
        assert_eq!(parse_color("not-a-colour", fallback).r, 0.0);
        assert_eq!(parse_color("#12345", fallback).r, 0.0);
    }

    #[test]
    fn unknown_theme_name_falls_back_to_default() {
        let mut config = Config::default();
        config.theme = "does-not-exist".into();
        let theme = config.active_theme();
        assert_eq!(theme.bg_main, Theme::default().bg_main);
    }

    #[test]
    fn round_trips_through_toml() {
        let config = Config::default();
        let text = toml::to_string(&config).expect("serialize");
        let parsed: Config = toml::from_str(&text).expect("deserialize");
        assert_eq!(parsed.theme, config.theme);
        assert_eq!(parsed.sidebar.widgets.len(), config.sidebar.widgets.len());
    }

    #[test]
    fn widget_order_and_framing_are_configurable() {
        let text = r##"
            theme = "gruvbox"

            [[sidebar.widgets]]
            kind = "clock"

            [[sidebar.widgets]]
            kind = "quick_panel"
            framed = true
            children = [
              { kind = "audio" },
              { kind = "battery" },
            ]
        "##;
        let config: Config = toml::from_str(text).expect("parse");
        let widgets = &config.sidebar.widgets;
        assert_eq!(widgets[0].kind, WidgetKind::Clock);
        assert_eq!(widgets[1].kind, WidgetKind::QuickPanel);
        assert!(widgets[1].framed);
        assert_eq!(widgets[1].children.len(), 2);
        assert_eq!(widgets[1].children[1].kind, WidgetKind::Battery);
    }

    #[test]
    fn rejects_unknown_keys() {
        let text = r#"theme = "carbon"
            [[sidebar.widgets]]
            kind = "clock"
            nonsense = true
        "#;
        assert!(toml::from_str::<Config>(text).is_err());
    }
}

#[cfg(test)]
mod example_config_tests {
    use super::*;

    /// The shipped example must parse through the real schema. This catches
    /// drift between the documentation and the code.
    #[test]
    fn shipped_example_config_is_valid() {
        let text = include_str!("../config.example.toml");
        let config: Config = toml::from_str(text).expect("example config must parse");
        assert!(config.themes.contains_key("carbon"));
        assert!(config.themes.len() >= 4);
        assert!(config.sidebar.widgets.iter().any(|w| w.kind == WidgetKind::Clock));
    }
}
