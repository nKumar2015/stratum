//! User configuration.
//!
//! Every default lives in one file, `config.default.toml`, installed beside
//! the binary as `share/stratum/config.default.toml`. The binary has none of
//! its own: every field is required, and a configuration missing one is
//! refused. The user's file is merged over the defaults (tables key by key,
//! anything else replaced), so it only needs the keys it changes; the Home
//! Manager module writes a complete one anyway.
//!
//! The defaults, first that exists:
//!   1. `$STRATUM_DEFAULTS`
//!   2. `<binary>/../share/stratum/config.default.toml`
//!   3. this crate's `config.default.toml` (debug builds only, for `cargo run`)
//!
//! The user's file, first that is set (it need not exist):
//!   1. `$STRATUM_CONFIG`
//!   2. `$XDG_CONFIG_HOME/stratum/config.toml`
//!   3. `~/.config/stratum/config.toml`

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::render::display_list::Color;

/// The whole configuration file.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Name of the active theme from [`Config::themes`].
    pub theme: String,
    /// Theme definitions. Colour values are `#rgb`, `#rrggbb` or `#rrggbbaa`.
    pub themes: BTreeMap<String, ThemeConfig>,
    pub sidebar: SidebarConfig,
    pub appearance: AppearanceConfig,
    pub behavior: BehaviorConfig,
    pub control_center: ControlCenterConfig,
    pub lock: LockConfig,
    pub screenshot: ScreenshotConfig,
    pub dashboard: DashboardConfig,
    pub render: RenderConfig,
    /// Sizes for everything the shell draws. Colours live in `themes`.
    pub metrics: crate::ui::style::Metrics,
}

impl Config {
    /// Merge `layers` in order, each over the ones before it, and check the
    /// result. Each layer is a file's path (for errors) and its text.
    pub fn from_layers(layers: &[(&Path, &str)]) -> Result<Self, ConfigError> {
        let mut merged = toml::Table::new();
        for (path, text) in layers {
            let table: toml::Table = toml::from_str(text).map_err(|err| ConfigError::Parse {
                path: path.to_path_buf(),
                message: err.to_string(),
            })?;
            merge(&mut merged, table);
        }
        let sources = layers
            .iter()
            .map(|(path, _)| path.display().to_string())
            .collect::<Vec<_>>()
            .join(" + ");
        let config: Config = toml::Value::Table(merged)
            .try_into()
            .map_err(|err: toml::de::Error| ConfigError::Invalid {
                sources: sources.clone(),
                message: err.to_string(),
            })?;
        config
            .validate()
            .map_err(|message| ConfigError::Invalid { sources, message })?;
        Ok(config)
    }

    /// What deserialising cannot check: values that must parse, and lists
    /// that must not be empty. Everything that reads them relies on this, so
    /// none of it needs a fallback.
    fn validate(&self) -> Result<(), String> {
        if self.themes.is_empty() {
            return Err("no themes are defined".into());
        }
        for (name, theme) in &self.themes {
            theme.to_theme().map_err(|err| format!("themes.{name}: {err}"))?;
        }
        if self.screenshot.pen_colors.is_empty() {
            return Err("screenshot.pen_colors is empty".into());
        }
        for colour in &self.screenshot.pen_colors {
            if parse_color(colour).is_none() {
                return Err(format!("screenshot.pen_colors: `{colour}` is not a colour"));
            }
        }
        let dashboard = &self.dashboard;
        if dashboard.tabs.is_empty() {
            return Err("dashboard.tabs is empty".into());
        }
        if dashboard.sample_ms == 0 || dashboard.history < 2 {
            return Err("dashboard: sample_ms must be positive and history at least 2".into());
        }
        if !(dashboard.screen_fraction > 0.0 && dashboard.screen_fraction <= 1.0) {
            return Err("dashboard.screen_fraction must be above 0 and at most 1".into());
        }
        for (i, tab) in dashboard.tabs.iter().enumerate() {
            if tab.name.trim().is_empty() {
                return Err(format!("dashboard.tabs[{i}] has no name"));
            }
            if dashboard.tab_index(&tab.name) != Some(i) {
                return Err(format!("dashboard.tabs: `{}` is named twice", tab.name));
            }
            let positive = |w: f32| !w.is_nan() && w > 0.0;
            if tab.rows.is_empty() || tab.rows.iter().any(|r| r.panels.is_empty()) {
                return Err(format!("dashboard tab `{}` has a row without panels, or no rows", tab.name));
            }
            if !tab.rows.iter().all(|r| positive(r.weight) && r.panels.iter().all(|p| positive(p.weight))) {
                return Err(format!("dashboard tab `{}`: every row and panel weight must be positive", tab.name));
            }
        }
        let weather = &dashboard.weather;
        if weather.latitude.is_some() != weather.longitude.is_some() {
            return Err("dashboard.weather: set both latitude and longitude, or neither".into());
        }
        if weather.latitude.is_none() && weather.zip.trim().is_empty() {
            return Err("dashboard.weather: zip is empty and no coordinates are set".into());
        }
        if weather.refresh_min == 0 || !(1..=48).contains(&weather.hours) || !(1..=16).contains(&weather.days) {
            return Err("dashboard.weather: refresh_min must be positive, hours 1-48 and days 1-16".into());
        }
        let calendar = &dashboard.calendar;
        if calendar.refresh_min == 0 || !(1..=60).contains(&calendar.agenda_days) {
            return Err("dashboard.calendar: refresh_min must be positive and agenda_days 1-60".into());
        }
        let timer = &dashboard.timer;
        let minutes = timer.presets_min.iter().chain([&timer.work_min, &timer.break_min, &timer.long_break_min]);
        if !(1..=6).contains(&timer.presets_min.len()) || timer.rounds == 0 || minutes.clone().any(|m| !(1..=24 * 60).contains(m)) {
            return Err("dashboard.timer: 1-6 presets, rounds positive, and every length 1-1440 minutes".into());
        }
        Ok(())
    }

    /// The active theme. An unknown name (a typo) shows the first theme
    /// rather than refusing the whole configuration.
    pub fn active_theme(&self) -> Theme {
        let named = self
            .themes
            .get_key_value(&self.theme)
            .or_else(|| self.themes.get_key_value(&self.theme.to_lowercase()))
            .or_else(|| self.themes.iter().next());
        let (name, theme) = named.expect("validated: at least one theme");
        let mut resolved = theme.resolved();
        resolved.name = name.clone();
        resolved
    }

    /// The shipped defaults alone, as tests see them.
    #[cfg(test)]
    pub fn test_defaults() -> Self {
        let text = include_str!("../config.default.toml");
        Self::from_layers(&[(Path::new("config.default.toml"), text)]).expect("shipped defaults are valid")
    }
}

/// Where the configuration comes from: the defaults, and the user's file
/// merged over them.
#[derive(Clone, Debug)]
pub struct ConfigSource {
    pub defaults: PathBuf,
    /// The user's file, which need not exist. `None` only without `$HOME`.
    pub user: Option<PathBuf>,
}

impl ConfigSource {
    pub fn locate() -> Result<Self, ConfigError> {
        let candidates = defaults_candidates();
        let defaults = candidates
            .iter()
            .find(|path| path.is_file())
            .cloned()
            .ok_or(ConfigError::NoDefaults { searched: candidates })?;
        Ok(Self {
            defaults,
            user: user_path(),
        })
    }

    /// The defaults with the user's file, when it exists, merged over them.
    pub fn load(&self) -> Result<Config, ConfigError> {
        let defaults = read(&self.defaults)?;
        match self.user.as_deref().filter(|path| path.is_file()) {
            Some(user) => {
                let text = read(user)?;
                Config::from_layers(&[(&self.defaults, &defaults), (user, &text)])
            }
            None => Config::from_layers(&[(&self.defaults, &defaults)]),
        }
    }

    /// The defaults alone, for a start whose user file is broken: better the
    /// default bar than none.
    pub fn load_defaults(&self) -> Result<Config, ConfigError> {
        Config::from_layers(&[(&self.defaults, &read(&self.defaults)?)])
    }

    /// Changes whenever the user's file does. The modification time alone is
    /// not enough: Home Manager makes the file a link into the Nix store,
    /// where every file has the same time, and a rebuild only re-points the
    /// link. The link's target changes with its contents, so it is part of
    /// the stamp. `None` while the file does not exist.
    pub fn user_stamp(&self) -> Option<(PathBuf, SystemTime)> {
        let target = std::fs::canonicalize(self.user.as_ref()?).ok()?;
        let modified = std::fs::metadata(&target).ok()?.modified().ok()?;
        Some((target, modified))
    }
}

/// Merge `over` into `base`: tables key by key, anything else replaced. A
/// list is replaced whole, so a user's `sidebar.widgets` is the whole bar.
fn merge(base: &mut toml::Table, over: toml::Table) {
    for (key, value) in over {
        match (base.get_mut(&key), value) {
            (Some(toml::Value::Table(base)), toml::Value::Table(over)) => merge(base, over),
            (_, value) => {
                base.insert(key, value);
            }
        }
    }
}

fn read(path: &Path) -> Result<String, ConfigError> {
    std::fs::read_to_string(path).map_err(|err| ConfigError::Io {
        path: path.to_path_buf(),
        message: err.to_string(),
    })
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn defaults_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    candidates.extend(env_path("STRATUM_DEFAULTS"));
    if let Some(prefix) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent()?.parent().map(Path::to_path_buf))
    {
        candidates.push(prefix.join("share/stratum/config.default.toml"));
    }
    #[cfg(debug_assertions)]
    candidates.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("config.default.toml"));
    candidates
}

fn user_path() -> Option<PathBuf> {
    if let Some(explicit) = env_path("STRATUM_CONFIG") {
        return Some(explicit);
    }
    // XDG_CONFIG_HOME when set, else ~/.config — both per the XDG base spec.
    if let Some(config_home) = env_path("XDG_CONFIG_HOME") {
        return Some(config_home.join("stratum/config.toml"));
    }
    env_path("HOME").map(|home| home.join(".config/stratum/config.toml"))
}

/// One named theme.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
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

    // Optional Material-style aliases. Each one left out is derived from a
    // colour above, so a theme only sets the ones it cares about. This is the
    // vocabulary loaf-fm uses and makes multi-surface theming coherent.
    pub surface_lowest: Option<String>,
    pub surface: Option<String>,
    pub surface_high: Option<String>,
    pub surface_highest: Option<String>,
    pub outline: Option<String>,
    pub on_primary: Option<String>,
}

impl ThemeConfig {
    /// The palette, or the first colour that does not parse.
    fn to_theme(&self) -> Result<Theme, String> {
        let c = |field: &str, value: &str| {
            parse_color(value).ok_or_else(|| format!("{field}: `{value}` is not a colour"))
        };
        let alias = |field: &str, value: &Option<String>, base: &str| {
            c(field, value.as_deref().unwrap_or(base))
        };
        Ok(Theme {
            bg_main: c("bg_main", &self.bg_main)?,
            bg_widget: c("bg_widget", &self.bg_widget)?,
            bg_hover: c("bg_hover", &self.bg_hover)?,
            bg_dark: c("bg_dark", &self.bg_dark)?,
            text_main: c("text_main", &self.text_main)?,
            text_muted: c("text_muted", &self.text_muted)?,
            primary: c("primary", &self.primary)?,
            secondary: c("secondary", &self.secondary)?,
            tertiary: c("tertiary", &self.tertiary)?,
            success: c("success", &self.success)?,
            warning: c("warning", &self.warning)?,
            error: c("error", &self.error)?,
            border_active: c("border_active", &self.border_active)?,
            border_inactive: c("border_inactive", &self.border_inactive)?,
            font: self.font.clone(),
            surface_lowest: alias("surface_lowest", &self.surface_lowest, &self.bg_dark)?,
            surface: alias("surface", &self.surface, &self.bg_main)?,
            surface_high: alias("surface_high", &self.surface_high, &self.bg_widget)?,
            surface_highest: alias("surface_highest", &self.surface_highest, &self.bg_hover)?,
            outline: alias("outline", &self.outline, &self.border_inactive)?,
            on_primary: alias("on_primary", &self.on_primary, &self.bg_main)?,
            name: String::new(),
        })
    }

    /// The palette of a theme from a loaded configuration, whose colours
    /// were checked when it loaded.
    pub(crate) fn resolved(&self) -> Theme {
        self.to_theme().expect("theme colours are validated when the config loads")
    }
}

/// Bar layout: placement, orientation, and which widgets to show, in order.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
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

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SidebarPosition {
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
    pub framed: bool,
    /// Vertical space this widget reserves. `None` uses the widget's intrinsic
    /// size, which is what most widgets want.
    pub height: Option<u32>,
    /// Children, for container widgets only; left out, there are none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<WidgetConfig>,
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
#[serde(deny_unknown_fields)]
pub struct AppearanceConfig {
    /// Corner radius used by framed widgets and menus.
    pub corner_radius: f32,
    /// Draw a border around framed widgets.
    pub framed_border: bool,
    /// That border's width in logical pixels.
    pub border_width: f32,
    /// Font for the sidebar's standalone icons (the launcher logo and the
    /// audio/Bluetooth/Wi-Fi glyphs). QML set no family on these, so
    /// fontconfig fell back to a proportional Nerd Font, whose icons are
    /// about half again as wide as a "Mono" variant's single-cell ones.
    pub icon_font: String,
}

impl AppearanceConfig {
    /// The width of a framed widget's border: none when borders are off.
    pub fn frame_border(&self) -> f32 {
        if self.framed_border {
            self.border_width
        } else {
            0.0
        }
    }
}

/// The paged settings surface.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlCenterConfig {
    /// Slide/fade duration in milliseconds. Zero opens instantly.
    pub animation_ms: u32,
    /// Reverse the wheel: turning it down scrolls the page up.
    pub invert_scroll: bool,
}

/// Runtime behaviour.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BehaviorConfig {
    /// Poll interval for widgets that are not push-driven, in milliseconds.
    pub poll_ms: u64,
    /// Watch the config file and reload on change.
    pub reload_on_change: bool,
    /// Own `org.kde.StatusNotifierWatcher` and display tray icons. Only one
    /// host can; turn it off to run beside another shell that owns it.
    pub tray: bool,
    /// Own `org.freedesktop.Notifications`. Only one server can; turn it off
    /// to run beside another notification daemon.
    pub notifications: bool,
    /// Register as the session's polkit authentication agent and show the
    /// password dialog. polkit allows one agent per session; turn it off
    /// while another agent holds it.
    pub polkit: bool,
}

/// The screenshot viewer's annotation pen.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScreenshotConfig {
    /// Pen colours offered, in order; the first is selected on open. These
    /// are drawn into the image, so they do not follow the theme.
    pub pen_colors: Vec<String>,
    /// Pen width on open, in on-screen pixels at 100 % zoom, and its maximum.
    pub pen_size: u32,
    pub pen_size_max: u32,
}

impl ScreenshotConfig {
    /// The pen colours, parsed. Loading checked that they all parse and that
    /// there is at least one.
    pub fn colors(&self) -> Vec<Color> {
        self.pen_colors.iter().filter_map(|c| parse_color(c)).collect()
    }
}

/// The dashboard overlay: how it samples, and what each tab shows.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DashboardConfig {
    /// Milliseconds between resource samples (CPU, memory, disk, GPUs).
    pub sample_ms: u64,
    /// Samples the graphs keep; times `sample_ms` is the span they cover.
    pub history: usize,
    /// Keep sampling while the dashboard is closed, so the graphs are full
    /// when it opens. Closed, samples are only recorded: nothing repaints.
    pub background_history: bool,
    /// The card's width as a share of the output's: everything in
    /// `[metrics.dashboard]` scales up to reach it, never down, and never
    /// past the screen's height.
    pub screen_fraction: f32,
    /// The tabs, in order. The dashboard always opens on the first.
    pub tabs: Vec<DashboardTab>,
    pub weather: WeatherConfig,
    pub calendar: CalendarConfig,
    pub timer: TimerConfig,
}

/// Calendar sync from ICS feeds. Fetched only while the dashboard is open,
/// at most every `refresh_min`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CalendarConfig {
    /// A file of ICS addresses, one per line (`#` starts a comment). The
    /// addresses are secrets, so they live in this file (a sops secret, say)
    /// rather than in the config, which Home Manager puts in the
    /// world-readable Nix store. A value with `://` is taken as the one
    /// address itself, for a config that is a secret (a sops template).
    /// Unset, there is no sync.
    pub urls_file: Option<String>,
    /// ICS addresses listed in the config itself (Google, Outlook, Canvas…),
    /// combined with `urls_file`'s. Only for a config that is a secret (a
    /// sops template filling each in with `config.sops.placeholder`): in a
    /// Home Manager config in the Nix store, every user could read them.
    pub urls: Option<Vec<String>>,
    pub refresh_min: u64,
    pub week_start: WeekStart,
    pub week_numbers: bool,
    /// Days the agenda looks ahead.
    pub agenda_days: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WeekStart {
    Monday,
    Sunday,
}

/// Where and how to forecast. Fetched only while the dashboard is open, at
/// most every `refresh_min`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WeatherConfig {
    /// A postal code, looked up once in `country` (ISO 3166 alpha-2:
    /// postal codes repeat across countries).
    pub zip: String,
    pub country: String,
    /// Set both to skip the postal code lookup.
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub units: stratum_sys::managers::weather::Units,
    pub refresh_min: u64,
    /// Hours and days forecast.
    pub hours: usize,
    pub days: usize,
}

impl DashboardConfig {
    /// The index of the tab called `name`, ignoring case.
    pub fn tab_index(&self, name: &str) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.name.eq_ignore_ascii_case(name))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DashboardTab {
    /// Shown in the tab bar, and the name `dashboard open <tab>` takes.
    pub name: String,
    /// A Nerd Font glyph before the name.
    pub icon: String,
    /// Top to bottom.
    pub rows: Vec<DashboardRow>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DashboardRow {
    /// Share of the tab's height, relative to the other rows' weights.
    pub weight: f32,
    /// Side by side, left to right.
    pub panels: Vec<DashboardPanel>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DashboardPanel {
    pub kind: PanelKind,
    /// Share of the tab's width, relative to the other panels' weights.
    pub weight: f32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PanelKind {
    /// Time, date, uptime, battery, device batteries and do-not-disturb.
    Glance,
    /// Now, the next hours and the next days.
    Weather,
    /// Events from the selected day (today unless one is picked) on.
    Agenda,
    /// The selected day's events alone, beside a month grid.
    Day,
    /// Load bars for CPU, GPUs, RAM and storage.
    Performance,
    /// The same, with history graphs, temperatures and network.
    Graphs,
    Calendar,
    Music,
    /// To-dos, kept in `$XDG_STATE_HOME/stratum/focus.json`.
    Todo,
    /// A free-form note, kept with the to-dos.
    Note,
    /// Countdown, stopwatch and Pomodoro.
    Timer,
}

/// The Focus tab's timer.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TimerConfig {
    /// Countdown buttons, in minutes.
    pub presets_min: Vec<u64>,
    /// Pomodoro: work, short and long breaks, and work rounds before a long
    /// break.
    pub work_min: u64,
    pub break_min: u64,
    pub long_break_min: u64,
    pub rounds: u32,
}

impl TimerConfig {
    pub fn pomodoro(&self) -> crate::dashboard::timer::Pomodoro {
        let minutes = |m: u64| std::time::Duration::from_secs(m * 60);
        crate::dashboard::timer::Pomodoro {
            work: minutes(self.work_min),
            short_break: minutes(self.break_min),
            long_break: minutes(self.long_break_min),
            rounds: self.rounds,
        }
    }
}

/// Which GPU the shell renders on. Read once at start; a reload does not move
/// the shell to another GPU.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RenderConfig {
    pub gpu: GpuPreference,
}

/// A GPU by vendor. `Auto` prefers an integrated GPU, then any other; a vendor
/// that is not present falls back to `Auto`.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum GpuPreference {
    Auto,
    Nvidia,
    Radeon,
    Intel,
}

/// The lock screen's artwork and authentication.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LockConfig {
    /// Wallpaper shown blurred behind the lock screen. `~/` is expanded. A
    /// missing file falls back to the theme's dark background.
    pub wallpaper: String,
    /// Round picture above the password field. Empty or missing shows a glyph.
    pub avatar: String,
    /// PAM service for the password check. Only used if `/etc/pam.d` has it;
    /// otherwise `stratum-shell`, then `login`, are tried, and the
    /// shell refuses to lock when none exists rather than lock with a service
    /// that can never succeed.
    pub pam_service: String,
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
    pub surface: Color,
    pub surface_high: Color,
    pub surface_highest: Color,
    /// Hairline colour for dividers and frame borders.
    pub outline: Color,
    /// Foreground that reads on `primary` (text on filled buttons).
    pub on_primary: Color,
    /// Name of the theme, for pages that display it.
    pub name: String,
}

/// Parse `#rgb`, `#rrggbb`, or `#rrggbbaa`.
fn parse_color(value: &str) -> Option<Color> {
    let hex = value.trim().strip_prefix('#')?;
    if !hex.is_ascii() {
        return None;
    }
    let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    let (r, g, b, a) = match hex.len() {
        3 => {
            let digit = |i: usize| u8::from_str_radix(&hex[i..i + 1], 16).ok().map(|c| c * 17);
            (digit(0)?, digit(1)?, digit(2)?, 255)
        }
        6 => (channel(0)?, channel(2)?, channel(4)?, 255),
        8 => (channel(0)?, channel(2)?, channel(4)?, channel(6)?),
        _ => return None,
    };
    Some(Color::rgba(
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
        a as f32 / 255.0,
    ))
}

#[derive(Debug)]
pub enum ConfigError {
    NoDefaults { searched: Vec<PathBuf> },
    Io { path: PathBuf, message: String },
    Parse { path: PathBuf, message: String },
    Invalid { sources: String, message: String },
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::NoDefaults { searched } => {
                let searched = searched
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "no config.default.toml found (searched {searched}); set STRATUM_DEFAULTS")
            }
            ConfigError::Io { path, message } => {
                write!(f, "failed to read {}: {}", path.display(), message)
            }
            ConfigError::Parse { path, message } => {
                write!(f, "invalid TOML in {}:\n{}", path.display(), message)
            }
            ConfigError::Invalid { sources, message } => {
                write!(f, "invalid configuration ({sources}):\n{message}")
            }
        }
    }
}

impl std::error::Error for ConfigError {}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULTS: &str = include_str!("../config.default.toml");

    fn with_user(user: &str) -> Result<Config, ConfigError> {
        Config::from_layers(&[(Path::new("defaults"), DEFAULTS), (Path::new("user"), user)])
    }

    #[test]
    fn shipped_defaults_are_complete() {
        let config = Config::test_defaults();
        assert_eq!(config.theme, "carbon");
        assert!(config.themes.contains_key("gruvbox"));
        assert!(config.sidebar.widgets.iter().any(|w| w.kind == WidgetKind::Clock));
    }

    #[test]
    fn a_key_missing_from_every_layer_is_an_error() {
        let mut table: toml::Table = toml::from_str(DEFAULTS).unwrap();
        let tray = table["metrics"]["tray"].as_table().unwrap().clone();
        let mut tray = tray;
        tray.remove("cell");
        table["metrics"].as_table_mut().unwrap().insert("tray".into(), tray.into());
        let text = toml::to_string(&table).unwrap();
        let err = Config::from_layers(&[(Path::new("defaults"), &text)]).unwrap_err();
        assert!(err.to_string().contains("cell"), "{err}");
    }

    #[test]
    fn user_tables_merge_over_the_defaults() {
        let config = with_user("[metrics.workspaces]\nvisible = 7\n").unwrap();
        assert_eq!(config.metrics.workspaces.visible, 7);
        // Untouched keys, in the same table and elsewhere, keep the defaults.
        let defaults = Config::test_defaults();
        assert_eq!(config.metrics.workspaces.gap, defaults.metrics.workspaces.gap);
        assert_eq!(config.metrics.typography.workspace, defaults.metrics.typography.workspace);
    }

    #[test]
    fn a_user_list_replaces_the_default_one() {
        let config = with_user(
            r##"
            theme = "gruvbox"

            [[sidebar.widgets]]
            kind = "clock"
            framed = false

            [[sidebar.widgets]]
            kind = "quick_panel"
            framed = true
            children = [
              { kind = "audio", framed = false },
              { kind = "battery", framed = false },
            ]
            "##,
        )
        .unwrap();
        let widgets = &config.sidebar.widgets;
        assert_eq!(widgets.len(), 2);
        assert_eq!(widgets[0].kind, WidgetKind::Clock);
        assert!(widgets[1].framed);
        assert_eq!(widgets[1].children[1].kind, WidgetKind::Battery);
    }

    #[test]
    fn a_user_theme_adds_to_the_shipped_ones() {
        let mut theme = toml::to_string(&Config::test_defaults().themes["carbon"]).unwrap();
        theme = format!("theme = \"mine\"\n[themes.mine]\n{theme}");
        let config = with_user(&theme).unwrap();
        assert!(config.themes.contains_key("mine"));
        assert!(config.themes.contains_key("carbon"));
        assert_eq!(config.active_theme().name, "mine");
    }

    #[test]
    fn rejects_unknown_keys() {
        assert!(with_user("[metrics.workspaces]\nrow_heigth = 24.0\n").is_err());
    }

    #[test]
    fn a_malformed_colour_is_refused_at_load() {
        let err = with_user("[themes.carbon]\nprimary = \"blue\"\n").unwrap_err();
        assert!(err.to_string().contains("themes.carbon: primary"), "{err}");
        assert!(with_user("[screenshot]\npen_colors = [\"#12345\"]\n").is_err());
        assert!(with_user("[screenshot]\npen_colors = []\n").is_err());
    }

    #[test]
    fn unknown_theme_name_shows_the_first_theme() {
        let mut config = Config::test_defaults();
        config.theme = "does-not-exist".into();
        let first = config.themes.keys().next().unwrap().clone();
        assert_eq!(config.active_theme().name, first);
    }

    #[test]
    fn parses_hex_with_and_without_alpha() {
        let c = parse_color("#ff0000").unwrap();
        assert_eq!((c.r, c.a), (1.0, 1.0));
        let c = parse_color("#00ff0080").unwrap();
        assert!((c.a - 0.502).abs() < 0.01);
        assert_eq!(parse_color("#0f0").unwrap().g, 1.0);
        assert!(parse_color("not-a-colour").is_none());
        assert!(parse_color("#12345").is_none());
    }

    #[test]
    fn round_trips_through_toml() {
        // What the Home Manager module writes is a serialised whole config;
        // it must load on its own, with no defaults under it.
        let config = Config::test_defaults();
        let text = toml::to_string(&config).expect("serialize");
        let parsed = Config::from_layers(&[(Path::new("written"), &text)]).expect("deserialize");
        assert_eq!(parsed.theme, config.theme);
        assert_eq!(parsed.sidebar.widgets.len(), config.sidebar.widgets.len());
    }

    #[test]
    fn the_stamp_follows_a_relinked_file() {
        let dir = std::env::temp_dir().join(format!("stratum-config-stamp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b, link) = (dir.join("a.toml"), dir.join("b.toml"), dir.join("config.toml"));
        // Same time on both, as every file in the Nix store has.
        let epoch = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1);
        for path in [&a, &b] {
            std::fs::write(path, "").unwrap();
            std::fs::File::options().write(true).open(path).unwrap().set_modified(epoch).unwrap();
        }
        let source = ConfigSource { defaults: PathBuf::new(), user: Some(link.clone()) };
        let _ = std::fs::remove_file(&link);
        assert_eq!(source.user_stamp(), None);
        std::os::unix::fs::symlink(&a, &link).unwrap();
        let first = source.user_stamp();
        std::fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(&b, &link).unwrap();
        let second = source.user_stamp();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(first.is_some());
        assert_ne!(first, second);
    }
}
