//! Monitor configuration: what the Display page edits, how it becomes
//! Hyprland's monitor rules, and the layouts kept for each set of monitors.
//!
//! Hyprland's own rules come from the user's (Nix-generated, read-only)
//! config. A layout applied from the shell is a runtime `hl.monitor` rule
//! sent with `eval`, which `hyprctl reload` discards, so the shell keeps it
//! in `$XDG_STATE_HOME/stratum/displays.json`, keyed by the set of monitors
//! connected, and applies it again whenever that set reappears or the config
//! is reloaded (`display_manager.rs`).
//!
//! Everything here is pure: parsing, geometry and the saved file's model.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// A video mode: size in pixels and refresh rate in Hz.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Mode {
    pub width: u32,
    pub height: u32,
    pub refresh: f32,
}

impl Mode {
    /// `2560x1600@165.00Hz`, as `availableModes` lists them.
    pub fn parse(text: &str) -> Option<Mode> {
        let text = text.trim().trim_end_matches("Hz");
        let (size, refresh) = text.split_once('@')?;
        let (width, height) = size.split_once('x')?;
        Some(Mode { width: width.parse().ok()?, height: height.parse().ok()?, refresh: refresh.parse().ok()? })
    }

    /// The same mode, allowing for a reported rate of 165.002 against a
    /// listed 165.00.
    pub fn same(&self, other: &Mode) -> bool {
        self.width == other.width && self.height == other.height && (self.refresh - other.refresh).abs() < 0.5
    }

    pub fn same_size(&self, other: &Mode) -> bool {
        self.width == other.width && self.height == other.height
    }

    /// What `hl.monitor`'s `mode` takes.
    pub fn spec(&self) -> String {
        format!("{}x{}@{:.2}", self.width, self.height, self.refresh)
    }

    pub fn size_label(&self) -> String {
        format!("{}\u{d7}{}", self.width, self.height)
    }

    pub fn refresh_label(&self) -> String {
        let rounded = self.refresh.round();
        if (self.refresh - rounded).abs() < 0.01 {
            format!("{rounded} Hz")
        } else {
            format!("{:.2} Hz", self.refresh)
        }
    }
}

/// How one monitor is set up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub mode: Mode,
    pub x: i32,
    pub y: i32,
    pub scale: f32,
    /// Hyprland's transform: 0-3 rotate by 90° steps, 4-7 flip then rotate.
    pub transform: u8,
    pub enabled: bool,
    pub vrr: bool,
}

impl Settings {
    /// The size it takes in the layout: the mode over the scale, turned
    /// with the transform.
    pub fn logical_size(&self) -> (i32, i32) {
        let scale = if self.scale > 0.0 { self.scale } else { 1.0 };
        let w = (self.mode.width as f32 / scale).round() as i32;
        let h = (self.mode.height as f32 / scale).round() as i32;
        if self.transform % 2 == 1 { (h, w) } else { (w, h) }
    }

    pub fn area(&self) -> Area {
        let (w, h) = self.logical_size();
        Area { x: self.x, y: self.y, w, h }
    }

    /// Whether applying `self` over `current` would change anything.
    pub fn differs(&self, current: &Settings) -> bool {
        if self.enabled != current.enabled {
            return true;
        }
        if !self.enabled {
            return false;
        }
        !self.mode.same(&current.mode)
            || self.x != current.x
            || self.y != current.y
            || (self.scale - current.scale).abs() > 0.005
            || self.transform != current.transform
            || self.vrr != current.vrr
    }

    /// The `hl.monitor` call that applies these settings to `output`.
    pub fn lua_rule(self, output: &str) -> String {
        let output = lua_string(output);
        if !self.enabled {
            return format!("hl.monitor({{ output = {output}, disabled = true }})");
        }
        format!(
            "hl.monitor({{ output = {output}, mode = \"{}\", position = \"{}x{}\", scale = {}, transform = {}, vrr = {} }})",
            self.mode.spec(),
            self.x,
            self.y,
            format_scale(self.scale),
            self.transform,
            u8::from(self.vrr),
        )
    }
}

/// A Lua string literal for `text`.
fn lua_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A scale as few digits as it needs: `1.6`, not `1.60000002`.
pub fn format_scale(scale: f32) -> String {
    let text = format!("{scale:.3}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// A connected monitor as Hyprland reports it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Monitor {
    /// The connector: `eDP-1`, `HDMI-A-1`.
    pub name: String,
    /// Make, model and serial; what identifies the monitor across ports.
    pub description: String,
    pub current: Settings,
    pub modes: Vec<Mode>,
    pub vrr_capable: bool,
}

impl Monitor {
    /// What a saved layout knows this monitor by: its description, which
    /// follows it from port to port, or its name when it has none.
    pub fn ident(&self) -> &str {
        if self.description.trim().is_empty() { &self.name } else { &self.description }
    }

    /// The sizes it offers, largest first.
    pub fn sizes(&self) -> Vec<Mode> {
        let mut sizes: Vec<Mode> = Vec::new();
        for mode in &self.modes {
            if !sizes.iter().any(|m| m.same_size(mode)) {
                sizes.push(*mode);
            }
        }
        sizes.sort_by(|a, b| (b.width * b.height).cmp(&(a.width * a.height)).then(b.width.cmp(&a.width)));
        sizes
    }

    /// The modes of one size, fastest first.
    pub fn rates(&self, size: &Mode) -> Vec<Mode> {
        let mut rates: Vec<Mode> = self.modes.iter().filter(|m| m.same_size(size)).copied().collect();
        rates.sort_by(|a, b| b.refresh.total_cmp(&a.refresh));
        rates.dedup_by(|a, b| a.same(b));
        rates
    }
}

/// Every monitor in a `j/monitors all` reply, disabled ones included.
pub fn parse_monitors(json: &str) -> Vec<Monitor> {
    let Ok(serde_json::Value::Array(list)) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|m| {
            let name = m["name"].as_str()?.to_string();
            let modes: Vec<Mode> = m["availableModes"]
                .as_array()
                .map(|modes| modes.iter().filter_map(|v| v.as_str().and_then(Mode::parse)).collect())
                .unwrap_or_default();
            let enabled = m["disabled"].as_bool() != Some(true);
            let reported = Mode {
                width: m["width"].as_u64().unwrap_or(0) as u32,
                height: m["height"].as_u64().unwrap_or(0) as u32,
                refresh: m["refreshRate"].as_f64().unwrap_or(0.0) as f32,
            };
            // The listed mode it is in, so a rate of 165.002 reads 165.00; a
            // disabled monitor reports nothing useful, so it takes its best.
            let mode = modes
                .iter()
                .find(|listed| listed.same(&reported))
                .copied()
                .or_else(|| (reported.width > 0).then_some(reported))
                .or_else(|| modes.first().copied())
                .unwrap_or_default();
            Some(Monitor {
                name,
                description: m["description"].as_str().unwrap_or_default().to_string(),
                current: Settings {
                    mode,
                    x: m["x"].as_i64().unwrap_or(0) as i32,
                    y: m["y"].as_i64().unwrap_or(0) as i32,
                    scale: m["scale"].as_f64().unwrap_or(1.0) as f32,
                    transform: m["transform"].as_u64().unwrap_or(0).min(7) as u8,
                    enabled,
                    vrr: m["vrr"].as_bool().unwrap_or(false),
                },
                modes,
                vrr_capable: m["hardwareDetails"]["vrrCapable"].as_bool().unwrap_or(false),
            })
        })
        .collect()
}

/// The scales of `candidates` that give `mode` a whole logical size (what
/// Hyprland accepts without adjusting), with `current` always among them.
pub fn scales_for(mode: &Mode, candidates: &[f32], current: f32) -> Vec<f32> {
    let whole = |v: f32| (v - v.round()).abs() < 0.01;
    let mut scales: Vec<f32> = candidates
        .iter()
        .copied()
        .filter(|s| *s > 0.0 && whole(mode.width as f32 / s) && whole(mode.height as f32 / s))
        .collect();
    if !scales.iter().any(|s| (s - current).abs() < 0.005) {
        scales.push(current);
    }
    scales.sort_by(f32::total_cmp);
    scales
}

/// A rectangle in the layout, in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Area {
    pub fn overlaps(&self, other: &Area) -> bool {
        self.x < other.x + other.w && other.x < self.x + self.w && self.y < other.y + other.h && other.y < self.y + self.h
    }

    /// The smallest area holding all of `areas`.
    pub fn bounds(areas: impl IntoIterator<Item = Area>) -> Option<Area> {
        let mut iter = areas.into_iter();
        let first = iter.next()?;
        let (mut x0, mut y0, mut x1, mut y1) = (first.x, first.y, first.x + first.w, first.y + first.h);
        for a in iter {
            x0 = x0.min(a.x);
            y0 = y0.min(a.y);
            x1 = x1.max(a.x + a.w);
            y1 = y1.max(a.y + a.h);
        }
        Some(Area { x: x0, y: y0, w: x1 - x0, h: y1 - y0 })
    }
}

/// Where a monitor of `size` dropped at `wanted` goes: against an edge of
/// one of `others`, overlapping none, as near `wanted` as that allows. Along
/// the edge it slides freely, but snaps to the other's ends and centre when
/// within `snap`. With no others it stays where it was put.
pub fn place(size: (i32, i32), wanted: (i32, i32), others: &[Area], snap: i32) -> (i32, i32) {
    let (w, h) = size;
    if others.is_empty() {
        return wanted;
    }
    let snap_to = |value: i32, targets: [i32; 3]| {
        targets
            .into_iter()
            .filter(|t| (t - value).abs() <= snap)
            .min_by_key(|t| (t - value).abs())
            .unwrap_or(value)
    };
    let mut candidates = Vec::new();
    for o in others {
        // Left or right of `o`: slide vertically while still sharing an edge.
        let y = wanted.1.clamp(o.y - h + 1, o.y + o.h - 1);
        let y = snap_to(y, [o.y, o.y + o.h - h, o.y + (o.h - h) / 2]);
        candidates.push((o.x + o.w, y));
        candidates.push((o.x - w, y));
        // Above or below: slide horizontally.
        let x = wanted.0.clamp(o.x - w + 1, o.x + o.w - 1);
        let x = snap_to(x, [o.x, o.x + o.w - w, o.x + (o.w - w) / 2]);
        candidates.push((x, o.y + o.h));
        candidates.push((x, o.y - h));
    }
    let dist = |(x, y): (i32, i32)| {
        let (dx, dy) = ((x - wanted.0) as i64, (y - wanted.1) as i64);
        dx * dx + dy * dy
    };
    candidates
        .into_iter()
        .filter(|&(x, y)| {
            let area = Area { x, y, w, h };
            !others.iter().any(|o| o.overlaps(&area))
        })
        .min_by_key(|c| dist(*c))
        // Every side blocked (it cannot be, with finite monitors): keep it.
        .unwrap_or(wanted)
}

/// The order to send rules in: monitors being enabled or kept on first, so
/// there is never a moment with every monitor off.
pub fn apply_order(changes: &mut [(String, Settings)]) {
    changes.sort_by_key(|(_, s)| !s.enabled);
}

/// The layout saved for one set of monitors.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    /// The identities of the monitors connected, sorted.
    pub monitors: Vec<String>,
    /// Each monitor's settings, by identity.
    pub settings: BTreeMap<String, Settings>,
}

/// The saved layouts.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    pub profiles: Vec<Profile>,
}

/// The sorted identities of `monitors`, which name their profile.
pub fn set_of(monitors: &[Monitor]) -> Vec<String> {
    let mut set: Vec<String> = monitors.iter().map(|m| m.ident().to_string()).collect();
    set.sort();
    set
}

impl Saved {
    pub fn profile(&self, monitors: &[Monitor]) -> Option<&Profile> {
        let set = set_of(monitors);
        self.profiles.iter().find(|p| p.monitors == set)
    }

    /// Keep `settings` (by connector name) as the layout for `monitors`.
    pub fn remember(&mut self, monitors: &[Monitor], settings: &[(String, Settings)]) {
        let set = set_of(monitors);
        let by_ident = monitors
            .iter()
            .filter_map(|m| {
                let s = settings.iter().find(|(name, _)| *name == m.name).map(|(_, s)| *s)?;
                Some((m.ident().to_string(), s))
            })
            .collect();
        self.profiles.retain(|p| p.monitors != set);
        self.profiles.push(Profile { monitors: set, settings: by_ident });
    }

    /// Forget the layout for `monitors`. Returns whether there was one.
    pub fn forget(&mut self, monitors: &[Monitor]) -> bool {
        let set = set_of(monitors);
        let before = self.profiles.len();
        self.profiles.retain(|p| p.monitors != set);
        self.profiles.len() != before
    }

    /// The rules that apply the saved layout, by connector name, in the
    /// order to send them: none when it is already in place, else one for
    /// every monitor. Sending only the one that differs is not enough: a
    /// monitor the config places `auto` is moved by Hyprland whenever
    /// another one moves.
    pub fn plan(&self, monitors: &[Monitor]) -> Vec<(String, Settings)> {
        let Some(profile) = self.profile(monitors) else {
            return Vec::new();
        };
        let mut changes: Vec<(String, Settings)> = monitors
            .iter()
            .filter_map(|m| Some((m.name.clone(), *profile.settings.get(m.ident())?)))
            .collect();
        let current = |name: &str| monitors.iter().find(|m| m.name == name).map(|m| m.current);
        if !changes.iter().any(|(n, s)| current(n).is_some_and(|c| s.differs(&c))) {
            return Vec::new();
        }
        // Never switch every monitor off, whatever the file says.
        let stays_on = |m: &Monitor| changes.iter().find(|(n, _)| *n == m.name).map_or(m.current.enabled, |(_, s)| s.enabled);
        if !monitors.is_empty() && !monitors.iter().any(stays_on) {
            return Vec::new();
        }
        apply_order(&mut changes);
        changes
    }
}

fn path() -> Option<std::path::PathBuf> {
    crate::paths::state_file("displays.json")
}

pub fn load() -> Saved {
    let Some(path) = path() else { return Saved::default() };
    let Ok(text) = std::fs::read_to_string(&path) else { return Saved::default() };
    serde_json::from_str(&text).unwrap_or_else(|err| {
        tracing::warn!("ignoring unreadable {}: {err}", path.display());
        Saved::default()
    })
}

pub fn store(saved: &Saved) -> Result<(), String> {
    let path = path().ok_or("no state directory ($HOME is unset)")?;
    let text = serde_json::to_string_pretty(saved).map_err(|err| err.to_string())?;
    crate::paths::write_atomic(&path, text).map_err(|err| format!("{}: {err}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONITORS: &str = r#"[
      {"name":"eDP-1","description":"BOE 0x0B38 0x00000003","width":2560,"height":1600,"refreshRate":165.002,
       "x":0,"y":0,"scale":1.6,"transform":0,"disabled":false,"vrr":false,
       "availableModes":["2560x1600@165.00Hz","2560x1600@60.00Hz","1920x1200@165.00Hz"],
       "hardwareDetails":{"vrrCapable":true}},
      {"name":"HDMI-A-1","description":"Microstep MSI G271CQR CC3H212601039","width":2560,"height":1440,
       "refreshRate":144.0,"x":-2560,"y":0,"scale":1.0,"transform":0,"disabled":false,"vrr":false,
       "availableModes":["2560x1440@144.00Hz","2560x1440@59.95Hz","1920x1080@60.00Hz"],
       "hardwareDetails":{"vrrCapable":false}}
    ]"#;

    fn monitors() -> Vec<Monitor> {
        parse_monitors(MONITORS)
    }

    #[test]
    fn monitors_parse_with_their_listed_mode() {
        let m = monitors();
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].current.mode.refresh, 165.0, "the listed rate, not 165.002");
        assert_eq!(m[0].current.logical_size(), (1600, 1000));
        assert!(m[0].vrr_capable && !m[1].vrr_capable);
        assert_eq!(m[1].current.x, -2560);
        assert_eq!(m[1].ident(), "Microstep MSI G271CQR CC3H212601039");
    }

    #[test]
    fn modes_group_into_sizes_and_rates() {
        let m = &monitors()[0];
        let sizes = m.sizes();
        assert_eq!(sizes.iter().map(Mode::size_label).collect::<Vec<_>>(), ["2560\u{d7}1600", "1920\u{d7}1200"]);
        let rates = m.rates(&sizes[0]);
        assert_eq!(rates.iter().map(Mode::refresh_label).collect::<Vec<_>>(), ["165 Hz", "60 Hz"]);
        assert_eq!(Mode::parse("2560x1440@59.95Hz").unwrap().refresh_label(), "59.95 Hz");
    }

    #[test]
    fn rules_are_lua_hyprland_accepts() {
        let s = monitors()[0].current;
        assert_eq!(
            s.lua_rule("eDP-1"),
            "hl.monitor({ output = \"eDP-1\", mode = \"2560x1600@165.00\", position = \"0x0\", scale = 1.6, transform = 0, vrr = 0 })"
        );
        let off = Settings { enabled: false, ..s };
        assert_eq!(off.lua_rule("a\"b"), "hl.monitor({ output = \"a\\\"b\", disabled = true })");
    }

    #[test]
    fn only_whole_logical_sizes_are_offered() {
        let mode = Mode { width: 2560, height: 1600, refresh: 60.0 };
        let scales = scales_for(&mode, &[1.0, 1.25, 1.5, 1.6, 2.0], 1.6);
        assert_eq!(scales, [1.0, 1.25, 1.6, 2.0], "1.5 gives 1706.67 pixels");
        assert!(scales_for(&mode, &[1.0], 1.333).contains(&1.333), "the current scale stays");
    }

    #[test]
    fn rotation_swaps_the_logical_size() {
        let s = Settings { mode: Mode { width: 1920, height: 1080, refresh: 60.0 }, scale: 1.0, transform: 1, enabled: true, ..Default::default() };
        assert_eq!(s.logical_size(), (1080, 1920));
    }

    #[test]
    fn a_dropped_monitor_snaps_against_an_edge_without_overlapping() {
        let laptop = Area { x: 0, y: 0, w: 1600, h: 1000 };
        // Dropped overlapping the laptop's right half: pushed to its right edge.
        assert_eq!(place((2560, 1440), (900, 30), &[laptop], 50), (1600, 0), "and snapped to the top");
        // Far away to the left: brought back against the left edge.
        assert_eq!(place((2560, 1440), (-4000, 300), &[laptop], 50), (-2560, 300));
        // Below, near the centre: snaps to centred.
        assert_eq!(place((1000, 500), (290, 1200), &[laptop], 20), (300, 1000));
        // Nothing else to stand against.
        assert_eq!(place((10, 10), (5, 5), &[], 20), (5, 5));
    }

    #[test]
    fn placement_avoids_every_other_monitor() {
        let a = Area { x: 0, y: 0, w: 1000, h: 1000 };
        let b = Area { x: 1000, y: 0, w: 1000, h: 1000 };
        let (x, y) = place((1000, 1000), (1000, 100), &[a, b], 50);
        let area = Area { x, y, w: 1000, h: 1000 };
        assert!(!area.overlaps(&a) && !area.overlaps(&b), "{x},{y}");
    }

    #[test]
    fn a_saved_layout_applies_whole_when_anything_differs_and_survives_a_port_change() {
        let mut m = monitors();
        let mut saved = Saved::default();
        let mut moved = m[1].current;
        moved.x = 1600;
        saved.remember(&m, &[("eDP-1".into(), m[0].current), ("HDMI-A-1".into(), moved)]);
        let plan = saved.plan(&m);
        assert_eq!(plan, [("eDP-1".to_string(), m[0].current), ("HDMI-A-1".to_string(), moved)]);
        // The same monitor on another port is the same profile.
        m[1].name = "DP-3".into();
        assert_eq!(saved.plan(&m)[1].0, "DP-3");
        // Once applied, there is nothing left to do.
        m[1].current = moved;
        assert!(saved.plan(&m).is_empty());
        // Another set of monitors has no profile.
        assert!(saved.plan(&m[..1]).is_empty());
        assert!(saved.forget(&m) && saved.profiles.is_empty());
    }

    #[test]
    fn a_saved_layout_never_turns_every_monitor_off() {
        let m = monitors();
        let mut saved = Saved::default();
        let off = |s: Settings| Settings { enabled: false, ..s };
        saved.remember(&m, &[("eDP-1".into(), off(m[0].current)), ("HDMI-A-1".into(), off(m[1].current))]);
        assert!(saved.plan(&m).is_empty());
    }

    #[test]
    fn enabling_is_sent_before_disabling() {
        let s = monitors()[0].current;
        let mut changes = vec![("a".to_string(), Settings { enabled: false, ..s }), ("b".to_string(), s)];
        apply_order(&mut changes);
        assert_eq!(changes[0].0, "b");
    }

    #[test]
    fn the_saved_file_round_trips() {
        let m = monitors();
        let mut saved = Saved::default();
        saved.remember(&m, &[("eDP-1".into(), m[0].current), ("HDMI-A-1".into(), m[1].current)]);
        let text = serde_json::to_string(&saved).unwrap();
        assert_eq!(serde_json::from_str::<Saved>(&text).unwrap(), saved);
    }
}
