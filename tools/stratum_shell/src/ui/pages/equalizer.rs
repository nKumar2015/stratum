//! Equalizer page: the parametric EQ for the current output.
//!
//! Matching the QML `AudioEqualizerPanel` (and the EQ logic in `AudioMenu.qml`):
//!
//! - A response curve over 20 Hz–20 kHz (log scale) with one point per band.
//!   Dragging a point changes its gain only; the band's frequency stays fixed.
//!   The curve previews the drag and the gain is committed on release.
//! - Apply sends the bands; Reset returns to ten flat bands and applies them;
//!   "+ Band" adds one above the last; the selected band can be removed (never
//!   the last one), retyped, enabled or disabled, and have its frequency, gain,
//!   and Q set.
//! - A preamp.
//! - The output's presets, built-in ones marked; clicking one applies it. The
//!   current bands can be saved under a name, and custom presets deleted.
//!
//! Any edit makes the preset "Custom", as in QML. Each number has a slider
//! and a box beside it that can be typed into (QML had a typed field for the
//! frequency only).
//!
//! The EQ backend (`stratum_sys::managers::audio`) blocks, so everything goes
//! through the action worker, which publishes the presets and the outcome into
//! `SystemState::eq`.

use serde_json::{json, Value};

use crate::actions::SystemAction;
use crate::render::display_list::{Color, DisplayList, Paint, Path, PathCommand, Point, Rect};
use crate::ui::pages::heading;
use crate::ui::text_field::{self, FieldKey, FieldStyle, TextField};
use crate::ui::{self, controls, InputState, PaintCtx};

/// The ten bands Reset returns to, and the frequencies legacy presets use.
const DEFAULT_FREQUENCIES: [f64; 10] = [31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];
const FILTER_TYPES: [&str; 6] = ["peaking", "low_shelf", "high_shelf", "low_pass", "high_pass", "band_pass"];
const DEFAULT_Q: f64 = 0.707;

/// One parametric band.
#[derive(Clone, Debug, PartialEq)]
pub struct Band {
    pub frequency_hz: f64,
    pub gain_db: f64,
    pub q: f64,
    pub filter_type: String,
    pub enabled: bool,
}

impl Band {
    fn flat(frequency_hz: f64) -> Self {
        Self {
            frequency_hz,
            gain_db: 0.0,
            q: DEFAULT_Q,
            filter_type: "peaking".into(),
            enabled: true,
        }
    }

    fn from_json(value: &Value, index: usize) -> Self {
        Self {
            frequency_hz: value["frequency_hz"]
                .as_f64()
                .unwrap_or(DEFAULT_FREQUENCIES.get(index).copied().unwrap_or(1000.0)),
            gain_db: value["gain_db"].as_f64().unwrap_or(0.0),
            q: value["q"].as_f64().filter(|q| *q > 0.0).unwrap_or(DEFAULT_Q),
            filter_type: value["filter_type"].as_str().filter(|t| !t.is_empty()).unwrap_or("peaking").to_string(),
            enabled: value["enabled"].as_bool().unwrap_or(true),
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "frequency_hz": self.frequency_hz,
            "gain_db": self.gain_db,
            "q": self.q,
            "filter_type": self.filter_type,
            "enabled": self.enabled,
        })
    }
}

/// What the backend allows, from its `capabilities.parametric`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits {
    pub min_freq: f64,
    pub max_freq: f64,
    pub min_gain: f64,
    pub max_gain: f64,
    pub min_q: f64,
    pub max_q: f64,
    pub max_bands: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            min_freq: 20.0,
            max_freq: 20_000.0,
            min_gain: -24.0,
            max_gain: 24.0,
            min_q: 0.1,
            max_q: 10.0,
            max_bands: 24,
        }
    }
}

impl Limits {
    fn from_capabilities(value: &Value) -> Self {
        let p = &value["parametric"];
        let pair = |key: &str| {
            let range = p[key].as_array()?;
            Some((range.first()?.as_f64()?, range.get(1)?.as_f64()?)).filter(|(a, b)| b > a)
        };
        let d = Self::default();
        let (min_freq, max_freq) = pair("freq_range_hz").unwrap_or((d.min_freq, d.max_freq));
        let (min_gain, max_gain) = pair("gain_range_db").unwrap_or((d.min_gain, d.max_gain));
        let (min_q, max_q) = pair("q_range").unwrap_or((d.min_q, d.max_q));
        Self {
            min_freq,
            max_freq,
            min_gain,
            max_gain,
            min_q,
            max_q,
            max_bands: p["max_bands"].as_u64().map(|n| n as usize).unwrap_or(d.max_bands),
        }
    }

    /// Position of `freq` on the log-scale axis, 0..=1.
    pub fn norm_of(&self, freq: f64) -> f64 {
        let (lo, hi) = (self.min_freq.log10(), self.max_freq.log10());
        ((freq.clamp(self.min_freq, self.max_freq).log10() - lo) / (hi - lo)).clamp(0.0, 1.0)
    }

    /// The frequency at `norm` on the log-scale axis.
    pub fn freq_at(&self, norm: f64) -> f64 {
        let (lo, hi) = (self.min_freq.log10(), self.max_freq.log10());
        10f64.powf(lo + (hi - lo) * norm.clamp(0.0, 1.0))
    }
}

/// A preset as listed by the backend.
#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    pub name: String,
    pub builtin: bool,
    pub bands: Vec<Band>,
    pub preamp_db: f64,
}

/// The editable EQ state.
#[derive(Clone, Debug)]
pub struct EqModel {
    pub bands: Vec<Band>,
    pub preamp_db: f64,
    pub selected: usize,
    /// The preset the bands came from, or "Custom" once edited.
    pub preset: String,
    pub presets: Vec<Preset>,
    pub limits: Limits,
    /// Output the model belongs to.
    device: Option<String>,
    /// Bands were edited and not yet replaced by a preset.
    edited: bool,
    /// A preset whose bands to adopt once a snapshot lists it as active.
    awaiting: Option<String>,
}

impl Default for EqModel {
    fn default() -> Self {
        Self {
            bands: Vec::new(),
            preamp_db: 0.0,
            selected: 0,
            preset: "Flat".into(),
            presets: Vec::new(),
            limits: Limits::default(),
            device: None,
            edited: false,
            awaiting: None,
        }
    }
}

impl EqModel {
    /// Follow the current output and the worker's snapshot. Returns an action
    /// when the presets for a new output have to be loaded.
    pub fn sync(&mut self, device: &str, snapshot: &Value) -> Option<SystemAction> {
        if device.is_empty() {
            return None;
        }
        let mut request = None;
        if self.device.as_deref() != Some(device) {
            *self = Self {
                device: Some(device.to_string()),
                ..Self::default()
            };
            request = Some(SystemAction::EqRefresh {
                device: device.to_string(),
            });
        }
        if snapshot["device_id"].as_str() != Some(device) {
            return request;
        }
        self.limits = Limits::from_capabilities(&snapshot["capabilities"]);
        self.presets = snapshot["presets"]
            .as_array()
            .map(|list| list.iter().map(parse_preset).collect())
            .unwrap_or_default();
        let active = snapshot["active_preset"].as_str().unwrap_or("Flat").to_string();
        let adopt = match &self.awaiting {
            Some(name) => *name == active,
            None => !self.edited && (self.bands.is_empty() || self.preset != active),
        };
        if adopt {
            if let Some(preset) = self.presets.iter().find(|p| p.name == active).cloned() {
                self.bands = if preset.bands.is_empty() { flat_bands() } else { preset.bands };
                self.preamp_db = preset.preamp_db;
                self.preset = preset.name;
                self.edited = false;
                self.awaiting = None;
                self.selected = self.selected.min(self.bands.len().saturating_sub(1));
            } else if self.bands.is_empty() {
                self.bands = flat_bands();
            }
        }
        request
    }

    fn touch(&mut self) {
        self.edited = true;
        self.awaiting = None;
        self.preset = "Custom".into();
    }

    pub fn set_gain(&mut self, index: usize, gain: f64) {
        let limits = self.limits;
        if let Some(band) = self.bands.get_mut(index) {
            band.gain_db = gain.clamp(limits.min_gain, limits.max_gain);
            self.touch();
        }
    }

    pub fn set_frequency(&mut self, index: usize, freq: f64) {
        let limits = self.limits;
        if let Some(band) = self.bands.get_mut(index) {
            band.frequency_hz = freq.clamp(limits.min_freq, limits.max_freq);
            self.touch();
        }
    }

    pub fn set_q(&mut self, index: usize, q: f64) {
        let limits = self.limits;
        if let Some(band) = self.bands.get_mut(index) {
            band.q = q.clamp(limits.min_q, limits.max_q);
            self.touch();
        }
    }

    pub fn cycle_type(&mut self, index: usize) {
        if let Some(band) = self.bands.get_mut(index) {
            let at = FILTER_TYPES.iter().position(|t| *t == band.filter_type).unwrap_or(0);
            band.filter_type = FILTER_TYPES[(at + 1) % FILTER_TYPES.len()].to_string();
            self.touch();
        }
    }

    pub fn toggle_enabled(&mut self, index: usize) {
        if let Some(band) = self.bands.get_mut(index) {
            band.enabled = !band.enabled;
            self.touch();
        }
    }

    pub fn set_preamp(&mut self, db: f64) {
        self.preamp_db = db.clamp(self.limits.min_gain, self.limits.max_gain);
        self.touch();
    }

    /// Add a band 1.35× above the last (QML's rule), and select it.
    pub fn add_band(&mut self) {
        if self.bands.len() >= self.limits.max_bands {
            return;
        }
        let freq = self
            .bands
            .last()
            .map(|b| (b.frequency_hz * 1.35).round().min(self.limits.max_freq))
            .unwrap_or(1000.0);
        self.bands.push(Band::flat(freq));
        self.selected = self.bands.len() - 1;
        self.touch();
    }

    /// Remove a band; the last one stays.
    pub fn remove_band(&mut self, index: usize) {
        if self.bands.len() <= 1 || index >= self.bands.len() {
            return;
        }
        self.bands.remove(index);
        self.selected = self.selected.min(self.bands.len() - 1);
        self.touch();
    }

    /// Ten flat bands and no preamp. Returns the action applying them, which
    /// QML did at once.
    pub fn reset(&mut self) -> Option<SystemAction> {
        self.bands = flat_bands();
        self.preamp_db = 0.0;
        self.selected = self.selected.min(self.bands.len() - 1);
        self.touch();
        self.apply()
    }

    pub fn apply(&mut self) -> Option<SystemAction> {
        Some(SystemAction::EqApply {
            device: self.device.clone()?,
            bands: self.bands_json(),
            preamp_db: self.preamp_db,
        })
    }

    pub fn apply_preset(&mut self, name: &str) -> Option<SystemAction> {
        self.awaiting = Some(name.to_string());
        Some(SystemAction::EqApplyPreset {
            device: self.device.clone()?,
            name: name.to_string(),
        })
    }

    pub fn save(&mut self, name: &str) -> Option<SystemAction> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        // Saving makes the bands that preset.
        self.preset = name.to_string();
        self.edited = false;
        self.awaiting = Some(name.to_string());
        Some(SystemAction::EqSave {
            device: self.device.clone()?,
            name: name.to_string(),
            bands: self.bands_json(),
            preamp_db: self.preamp_db,
        })
    }

    /// Delete the current preset, when it is a custom one.
    pub fn delete_current(&mut self) -> Option<SystemAction> {
        let preset = self.presets.iter().find(|p| p.name == self.preset)?;
        if preset.builtin {
            return None;
        }
        Some(SystemAction::EqDelete {
            device: self.device.clone()?,
            name: preset.name.clone(),
        })
    }

    pub fn can_delete(&self) -> bool {
        self.presets.iter().any(|p| p.name == self.preset && !p.builtin)
    }

    pub fn bands_json(&self) -> Value {
        Value::Array(self.bands.iter().map(Band::to_json).collect())
    }

    /// The curve at `freq`, with `preview` substituting one band's gain. The
    /// same visual model as the QML graph: a bell in octaves for peaking and
    /// band-pass, a sigmoid for shelves and passes. It is a picture of the
    /// settings, not the filter's exact response.
    pub fn response_db(&self, freq: f64, preview: Option<(usize, f64)>) -> f64 {
        let mut total = self.preamp_db;
        for (i, band) in self.bands.iter().enumerate() {
            let gain = match preview {
                Some((index, gain)) if index == i => gain,
                _ => band.gain_db,
            };
            total += contribution(freq, band, gain, &self.limits);
        }
        total.clamp(self.limits.min_gain, self.limits.max_gain)
    }
}

fn contribution(freq: f64, band: &Band, gain: f64, limits: &Limits) -> f64 {
    if !band.enabled {
        return 0.0;
    }
    let f = freq.clamp(limits.min_freq, limits.max_freq);
    let center = band.frequency_hz.clamp(limits.min_freq, limits.max_freq);
    let q = band.q.clamp(0.1, 10.0);
    let octaves = (f / center).log2();
    let width = (1.2 / q).max(0.08);
    let bell = (-(octaves * octaves) / (2.0 * width * width)).exp();
    let slope = (q * 2.0).clamp(0.6, 12.0);
    let sig = 1.0 / (1.0 + (-octaves * slope).exp());
    match band.filter_type.as_str() {
        "low_shelf" | "low_pass" => gain * (1.0 - sig),
        "high_shelf" | "high_pass" => gain * sig,
        _ => gain * bell,
    }
}

fn flat_bands() -> Vec<Band> {
    DEFAULT_FREQUENCIES.iter().map(|f| Band::flat(*f)).collect()
}

fn parse_preset(value: &Value) -> Preset {
    let bands = value["parametric_bands"]
        .as_array()
        .filter(|b| !b.is_empty())
        .map(|list| list.iter().enumerate().map(|(i, b)| Band::from_json(b, i)).collect())
        .or_else(|| {
            // Legacy presets carry ten gains.
            value["bands"].as_array().map(|gains| {
                gains
                    .iter()
                    .enumerate()
                    .map(|(i, g)| Band {
                        gain_db: g.as_f64().unwrap_or(0.0),
                        ..Band::flat(DEFAULT_FREQUENCIES.get(i).copied().unwrap_or(1000.0))
                    })
                    .collect()
            })
        })
        .unwrap_or_default();
    Preset {
        name: value["name"].as_str().unwrap_or_default().to_string(),
        builtin: value["is_builtin"].as_bool().unwrap_or(false),
        bands,
        preamp_db: value["preamp_db"].as_f64().unwrap_or(0.0),
    }
}

fn format_freq(hz: f64) -> String {
    if hz >= 1000.0 {
        format!("{:.1} kHz", hz / 1000.0)
    } else {
        format!("{} Hz", hz.round())
    }
}

fn filter_label(kind: &str) -> &'static str {
    match kind {
        "low_shelf" => "Low shelf",
        "high_shelf" => "High shelf",
        "low_pass" => "Low pass",
        "high_pass" => "High pass",
        "band_pass" => "Band pass",
        _ => "Peaking",
    }
}

/// Which slider is being dragged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Drag {
    Point(usize),
    Preamp,
    Frequency,
    Gain,
    Q,
}

pub struct EqualizerPage {
    pub model: EqModel,
    drag: Option<Drag>,
    /// The gain a point is being dragged to, drawn until release.
    preview: Option<(usize, f64)>,
    /// The "save as" name field, while it is open.
    naming: Option<TextField>,
    /// A value box being typed into, and where it was last drawn (a click
    /// anywhere else commits it).
    editing: Option<Editing>,
    edit_rect: Option<Rect>,
}

/// A value box open for typing.
struct Editing {
    which: Drag,
    /// The band it belongs to; selecting another band drops the edit.
    band: usize,
    field: TextField,
}

/// A number typed into a value box. Units are optional: frequency in Hz,
/// where a `k` suffix means kHz ("1.2k"); gain and preamp in dB; Q plain.
/// `None` for anything that is not a number, which leaves the value as it
/// was.
fn parse_value(which: Drag, text: &str) -> Option<f64> {
    let text = text.trim().to_ascii_lowercase();
    let text = text.trim_end_matches("hz").trim_end_matches("db").trim();
    let (number, scale) = match text.strip_suffix('k') {
        Some(number) if which == Drag::Frequency => (number.trim(), 1000.0),
        _ => (text, 1.0),
    };
    let value: f64 = number.strip_prefix('+').unwrap_or(number).parse().ok()?;
    value.is_finite().then_some(value * scale)
}

impl EqualizerPage {
    pub fn new() -> Self {
        Self {
            model: EqModel::default(),
            drag: None,
            preview: None,
            naming: None,
            editing: None,
            edit_rect: None,
        }
    }

    /// Whether keys should come here: the preset name field or a value box
    /// is open.
    pub fn wants_keys(&self) -> bool {
        self.naming.is_some() || self.editing.is_some()
    }

    fn editing_which(&self) -> Option<Drag> {
        self.editing.as_ref().map(|e| e.which)
    }

    /// Set the value being typed, if it parses, and close its box. The
    /// setters clamp to the backend's limits.
    fn commit(&mut self, which: Drag, band: usize, text: &str) {
        let Some(value) = parse_value(which, text) else {
            return;
        };
        match which {
            Drag::Preamp => self.model.set_preamp((value * 10.0).round() / 10.0),
            Drag::Frequency => self.model.set_frequency(band, value.round()),
            Drag::Gain => self.model.set_gain(band, (value * 10.0).round() / 10.0),
            Drag::Q => self.model.set_q(band, (value * 100.0).round() / 100.0),
            Drag::Point(_) => {}
        }
    }

    fn commit_editing(&mut self) {
        if let Some(editing) = self.editing.take() {
            self.commit(editing.which, editing.band, editing.field.text());
        }
        self.edit_rect = None;
    }

    /// Typing into the preset name or a value box. Return saves or sets,
    /// Escape cancels.
    pub fn key(
        &mut self,
        keysym: smithay_client_toolkit::seat::keyboard::Keysym,
        text: Option<&str>,
        ctrl: bool,
        pending: &mut Vec<SystemAction>,
    ) {
        if let Some(editing) = self.editing.as_mut() {
            match editing.field.key(keysym, text, ctrl) {
                FieldKey::Submit(typed) => {
                    let (which, band) = (editing.which, editing.band);
                    self.editing = None;
                    self.edit_rect = None;
                    self.commit(which, band, typed.expose());
                }
                FieldKey::Escape => {
                    self.editing = None;
                    self.edit_rect = None;
                }
                _ => {}
            }
            return;
        }
        let Some(field) = self.naming.as_mut() else {
            return;
        };
        match field.key(keysym, text, ctrl) {
            FieldKey::Submit(name) => {
                self.naming = None;
                pending.extend(self.model.save(name.expose()));
            }
            FieldKey::Escape => self.naming = None,
            _ => {}
        }
    }

    pub fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        area: Rect,
        input: &InputState,
        clicked: bool,
        pending: &mut Vec<SystemAction>,
    ) {
        let m = ctx.metrics.equalizer.clone();
        let area = heading(ctx, list, area, "Equalizer");

        // Follow the current output.
        let audio = ctx.state.audio.get();
        let device = audio["default_sink"].as_str().unwrap_or_default().to_string();
        let device_label = audio["sinks"]
            .as_array()
            .and_then(|sinks| sinks.iter().find(|s| s["name"].as_str() == Some(&device)))
            .and_then(|s| s["description"].as_str())
            .unwrap_or(&device)
            .to_string();
        let snapshot = ctx.state.eq.get();
        pending.extend(self.model.sync(&device, &snapshot));

        let pointer = input.pointer.map(|(x, y)| Point::new(x, y));
        let released = !input.pressed;
        if released {
            if let (Some(Drag::Point(i)), Some((_, gain))) = (self.drag, self.preview) {
                self.model.set_gain(i, gain);
            }
            self.drag = None;
            self.preview = None;
        }
        // A click away from an open value box sets it, as leaving a field
        // did in QML; another band selected drops it.
        if self.editing.as_ref().is_some_and(|e| e.which != Drag::Preamp && e.band != self.model.selected) {
            self.editing = None;
            self.edit_rect = None;
        }
        if clicked && self.editing.is_some() && !pointer.zip(self.edit_rect).is_some_and(|(p, r)| r.contains(p)) {
            self.commit_editing();
        }
        let mut cursor = Cursor { x: area.x, y: area.y, width: area.width };
        let caption = |ctx: &mut PaintCtx<'_>, list: &mut DisplayList, rect: Rect, text: &str, color: Color| {
            ui::paint_text_vcentered(ctx, list, text, rect.x, rect, m.caption, ctx.metrics.typography.weight_regular, color);
        };

        if device.is_empty() {
            caption(ctx, list, cursor.take(m.line), "No output device", ctx.theme.text_muted);
            return;
        }
        let row = cursor.take(m.line);
        caption(ctx, list, row, &format!("{device_label} \u{b7} {}", self.model.preset), ctx.theme.text_muted);
        cursor.gap(m.gap);

        // The graph.
        let graph = cursor.take(m.graph_height);
        self.paint_graph(ctx, list, graph, pointer, input, clicked);
        cursor.gap(m.gap);

        // Apply / Reset / + Band.
        let row = cursor.take(m.button_height);
        let w = (row.width - m.gap * 2.0) / 3.0;
        let button = |i: f32| Rect { x: row.x + i * (w + m.gap), width: w, radius: m.radius, ..row };
        if controls::button(ctx, list, button(0.0), "Apply", input, clicked, true) {
            pending.extend(self.model.apply());
        }
        if controls::button(ctx, list, button(1.0), "Reset", input, clicked, false) {
            pending.extend(self.model.reset());
        }
        if controls::button(ctx, list, button(2.0), "+ Band", input, clicked, false) {
            self.model.add_band();
        }
        cursor.gap(m.gap);

        // Preamp.
        let preamp = self.model.preamp_db;
        let limits = self.model.limits;
        if let Some(v) = self.slider_row(ctx, list, &mut cursor, "Preamp", &format!("{preamp:+.1} dB"), format!("{preamp:.1}"), ((preamp - limits.min_gain) / (limits.max_gain - limits.min_gain)) as f32, Drag::Preamp, input, clicked) {
            self.model.set_preamp(limits.min_gain + v as f64 * (limits.max_gain - limits.min_gain));
        }
        cursor.gap(m.gap);

        // The selected band.
        let index = self.model.selected.min(self.model.bands.len().saturating_sub(1));
        if let Some(band) = self.model.bands.get(index).cloned() {
            let row = cursor.take(m.button_height);
            caption(ctx, list, row, &format!("Band {} of {}", index + 1, self.model.bands.len()), ctx.theme.text_main);
            let small = |i: f32, width: f32| Rect {
                x: row.x + row.width - (i + 1.0) * (width + m.gap) + m.gap,
                width,
                radius: m.radius,
                ..row
            };
            if self.model.bands.len() > 1 {
                if controls::button(ctx, list, small(0.0, m.small_button), "Remove", input, clicked, false) {
                    self.model.remove_band(index);
                }
            } else {
                disabled_button(ctx, list, small(0.0, m.small_button), "Remove");
            }
            if controls::button(ctx, list, small(1.0, m.small_button), if band.enabled { "On" } else { "Off" }, input, clicked, band.enabled) {
                self.model.toggle_enabled(index);
            }
            if controls::button(ctx, list, small(2.0, m.type_button), filter_label(&band.filter_type), input, clicked, false) {
                self.model.cycle_type(index);
            }
            cursor.gap(m.gap);
            let norm = limits.norm_of(band.frequency_hz) as f32;
            if let Some(v) = self.slider_row(ctx, list, &mut cursor, "Freq", &format_freq(band.frequency_hz), format!("{}", band.frequency_hz.round()), norm, Drag::Frequency, input, clicked) {
                self.model.set_frequency(index, limits.freq_at(v as f64).round());
            }
            let gain_norm = ((band.gain_db - limits.min_gain) / (limits.max_gain - limits.min_gain)) as f32;
            if let Some(v) = self.slider_row(ctx, list, &mut cursor, "Gain", &format!("{:+.1} dB", band.gain_db), format!("{:.1}", band.gain_db), gain_norm, Drag::Gain, input, clicked) {
                // Half-decibel steps.
                let gain = limits.min_gain + v as f64 * (limits.max_gain - limits.min_gain);
                self.model.set_gain(index, (gain * 2.0).round() / 2.0);
            }
            let q_norm = ((band.q - limits.min_q) / (limits.max_q - limits.min_q)) as f32;
            if let Some(v) = self.slider_row(ctx, list, &mut cursor, "Q", &format!("{:.2}", band.q), format!("{:.2}", band.q), q_norm, Drag::Q, input, clicked) {
                self.model.set_q(index, ((limits.min_q + v as f64 * (limits.max_q - limits.min_q)) * 100.0).round() / 100.0);
            }
        }
        cursor.gap(m.gap);

        // Presets.
        let row = cursor.take(m.line);
        caption(ctx, list, row, "Presets", ctx.theme.text_muted);
        let presets = self.model.presets.clone();
        for preset in &presets {
            let row = cursor.take(m.preset_row);
            if row.y + row.height > area.y + area.height - m.button_height - m.line - m.gap {
                break;
            }
            let selected = preset.name == self.model.preset;
            let hovered = pointer.is_some_and(|p| row.contains(p));
            if selected || hovered {
                list.push_rect(Rect { radius: m.radius, color: ctx.theme.bg_hover, ..row });
            }
            let label = Rect { x: row.x + m.gap, ..row };
            ui::paint_text_vcentered(ctx, list, &preset.name, label.x, label, m.text, if selected { ctx.metrics.typography.weight_bold } else { ctx.metrics.typography.weight_regular }, ctx.theme.text_main);
            if preset.builtin {
                let style = crate::text::TextStyle {
                    family: std::sync::Arc::from(ctx.theme.font.as_str()),
                    size: m.caption,
                    weight: ctx.metrics.typography.weight_regular,
                    color: ctx.theme.text_muted,
                };
                let w = ctx.text.measure("built-in", &style).0;
                ui::paint_text_vcentered(ctx, list, "built-in", row.x + row.width - w - m.gap, row, m.caption, ctx.metrics.typography.weight_regular, ctx.theme.text_muted);
            }
            if clicked && hovered {
                pending.extend(self.model.apply_preset(&preset.name));
            }
        }
        cursor.gap(m.gap);

        // Save / Delete, or the name field while saving.
        let row = cursor.take(m.button_height);
        if let Some(field) = &self.naming {
            let name_rect = Rect { width: row.width - m.small_button - m.gap, radius: m.radius, ..row };
            text_field::paint_frame(list, name_rect, m.border, ctx.theme.border_active, ctx.theme.bg_widget);
            field.paint(ctx, list, name_rect, FieldStyle { text: m.text, padding: m.gap, caret_width: m.border * 1.5, masked: false, placeholder: Some("Preset name") }, false);
            let save = Rect { x: row.x + row.width - m.small_button, width: m.small_button, radius: m.radius, ..row };
            if controls::button(ctx, list, save, "Save", input, clicked, true) {
                if let Some(name) = self.naming.as_mut().and_then(TextField::take) {
                    pending.extend(self.model.save(name.expose()));
                }
                self.naming = None;
            }
        } else {
            let w = (row.width - m.gap) / 2.0;
            if controls::button(ctx, list, Rect { width: w, radius: m.radius, ..row }, "Save as\u{2026}", input, clicked, false) {
                self.naming = Some(TextField::new());
            }
            // Built-in presets cannot be deleted.
            let delete = Rect { x: row.x + w + m.gap, width: w, radius: m.radius, ..row };
            if !self.model.can_delete() {
                disabled_button(ctx, list, delete, "Delete");
            } else if controls::button(ctx, list, delete, "Delete", input, clicked, false) {
                pending.extend(self.model.delete_current());
            }
        }
        cursor.gap(m.gap);

        let status = snapshot["status"].as_str().unwrap_or_default();
        if !status.is_empty() && snapshot["device_id"].as_str() == Some(&device) {
            let ok = snapshot["ok"].as_bool() != Some(false);
            caption(ctx, list, cursor.take(m.line), status, if ok { ctx.theme.text_muted } else { ctx.theme.error });
        }
    }

    /// A labelled slider with its value in a box that can be typed into:
    /// clicking the box opens it holding `prefill`. Returns the new 0..=1
    /// value while the slider is dragged.
    #[allow(clippy::too_many_arguments)]
    fn slider_row(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        cursor: &mut Cursor,
        label: &str,
        value: &str,
        prefill: String,
        norm: f32,
        which: Drag,
        input: &InputState,
        clicked: bool,
    ) -> Option<f32> {
        let m = ctx.metrics.equalizer.clone();
        let row = cursor.take(m.slider_row);
        ui::paint_text_vcentered(ctx, list, label, row.x, row, m.caption, ctx.metrics.typography.weight_regular, ctx.theme.text_muted);

        let value_box = Rect {
            x: row.x + row.width - m.value_width,
            width: m.value_width,
            radius: m.radius,
            ..row
        };
        let pointer_on_box = input.pointer.is_some_and(|(x, y)| value_box.contains(Point::new(x, y)));
        if clicked && pointer_on_box && self.drag.is_none() && self.editing_which() != Some(which) {
            self.editing = Some(Editing { which, band: self.model.selected, field: TextField::with_text(&prefill) });
        }
        let editing = self.editing.as_ref().filter(|e| e.which == which);
        let border = if editing.is_some() || pointer_on_box { ctx.theme.border_active } else { ctx.theme.border_inactive };
        text_field::paint_frame(list, value_box, m.border, border, ctx.theme.bg_widget);
        match editing {
            Some(editing) => {
                editing.field.paint(ctx, list, value_box, FieldStyle { text: m.caption, padding: m.gap, caret_width: m.border * 1.5, masked: false, placeholder: None }, false);
                self.edit_rect = Some(value_box);
            }
            None => {
                let style = crate::text::TextStyle {
                    family: std::sync::Arc::from(ctx.theme.font.as_str()),
                    size: m.caption,
                    weight: ctx.metrics.typography.weight_medium,
                    color: ctx.theme.text_main,
                };
                let w = ctx.text.measure(value, &style).0;
                let x = value_box.x + value_box.width - m.gap - w;
                ui::paint_text_vcentered(ctx, list, value, x, value_box, m.caption, style.weight, ctx.theme.text_main);
            }
        }

        let slider = Rect {
            x: row.x + m.label_width,
            width: row.width - m.label_width - m.value_width - m.gap,
            ..row
        };
        let pointer_in = input.pointer.is_some_and(|(x, y)| slider.contains(Point::new(x, y)));
        if input.pressed && pointer_in && self.drag.is_none() {
            self.drag = Some(which);
        }
        controls::slider(ctx, list, slider, norm, self.drag == Some(which), input)
    }

    fn paint_graph(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        graph: Rect,
        pointer: Option<Point>,
        input: &InputState,
        clicked: bool,
    ) {
        let m = ctx.metrics.equalizer.clone();
        let limits = self.model.limits;
        text_field::paint_frame(list, Rect { radius: m.radius, ..graph }, m.border, ctx.theme.border_inactive, ctx.theme.bg_widget);
        let plot = Rect {
            x: graph.x + m.graph_padding,
            y: graph.y + m.graph_padding,
            width: graph.width - m.graph_padding * 2.0,
            height: graph.height - m.graph_padding * 2.0,
            radius: 0.0,
            color: Color::TRANSPARENT,
        };
        let x_of = |freq: f64| plot.x + plot.width * limits.norm_of(freq) as f32;
        let y_of = |db: f64| plot.y + plot.height * (1.0 - ((db - limits.min_gain) / (limits.max_gain - limits.min_gain)) as f32);
        let db_at = |y: f32| limits.min_gain + (1.0 - ((y - plot.y) / plot.height).clamp(0.0, 1.0) as f64) * (limits.max_gain - limits.min_gain);

        // Grid: 0 dB, half the range either way, and decade lines.
        for db in [limits.min_gain / 2.0, 0.0, limits.max_gain / 2.0] {
            list.push_rect(Rect { x: plot.x, y: y_of(db), width: plot.width, height: m.border, radius: 0.0, color: if db == 0.0 { ctx.theme.border_active.with_alpha(0.5) } else { ctx.theme.border_inactive } });
        }
        for freq in [100.0, 1000.0, 10_000.0] {
            list.push_rect(Rect { x: x_of(freq), y: plot.y, width: m.border, height: plot.height, radius: 0.0, color: ctx.theme.border_inactive });
            let label = if freq >= 1000.0 { format!("{}k", freq / 1000.0) } else { format!("{freq}") };
            ui::paint_text_vcentered(ctx, list, &label, x_of(freq) + m.border * 3.0, Rect { y: plot.y + plot.height - m.caption * 1.3, height: m.caption * 1.3, ..plot }, m.caption * 0.85, ctx.metrics.typography.weight_regular, ctx.theme.text_muted);
        }

        // Start dragging the nearest point to a press, within reach.
        if clicked && self.drag.is_none() {
            if let Some(p) = pointer.filter(|p| graph.contains(*p)) {
                let nearest = self
                    .model
                    .bands
                    .iter()
                    .enumerate()
                    .map(|(i, b)| (i, (x_of(b.frequency_hz) - p.x).abs()))
                    .min_by(|a, b| a.1.total_cmp(&b.1));
                if let Some((i, _)) = nearest.filter(|(_, d)| *d <= m.point_reach) {
                    self.model.selected = i;
                    self.drag = Some(Drag::Point(i));
                }
            }
        }
        if let (Some(Drag::Point(i)), Some(p), true) = (self.drag, pointer, input.pressed) {
            // Gain only: the point's frequency stays where it is.
            let gain = (db_at(p.y) - self.model.preamp_db).clamp(limits.min_gain, limits.max_gain);
            self.preview = Some((i, (gain * 2.0).round() / 2.0));
        }

        // The curve.
        let steps = m.curve_steps.max(8);
        let mut commands = Vec::with_capacity(steps + 1);
        for s in 0..=steps {
            let norm = s as f64 / steps as f64;
            let point = Point::new(plot.x + plot.width * norm as f32, y_of(self.model.response_db(limits.freq_at(norm), self.preview)));
            commands.push(if s == 0 { PathCommand::MoveTo(point) } else { PathCommand::LineTo(point) });
        }
        list.push_path(Path::new(commands, Paint::Solid(ctx.theme.primary)).stroked(m.curve_width));

        // Points.
        for (i, band) in self.model.bands.iter().enumerate() {
            let gain = match self.preview {
                Some((index, gain)) if index == i => gain,
                _ => band.gain_db,
            };
            let center = Point::new(x_of(band.frequency_hz), y_of((gain + self.model.preamp_db).clamp(limits.min_gain, limits.max_gain)));
            let selected = i == self.model.selected;
            let size = if selected { m.point * 1.4 } else { m.point };
            let color = if !band.enabled {
                ctx.theme.text_muted
            } else if selected {
                ctx.theme.secondary
            } else {
                ctx.theme.primary
            };
            list.push_rect(Rect { x: center.x - size * 0.5, y: center.y - size * 0.5, width: size, height: size, radius: size * 0.5, color });
        }
    }
}

impl Default for EqualizerPage {
    fn default() -> Self {
        Self::new()
    }
}

/// A button that cannot be pressed now, drawn so it reads that way.
fn disabled_button(ctx: &mut PaintCtx<'_>, list: &mut DisplayList, rect: Rect, label: &str) {
    list.push_rect(Rect { color: ctx.theme.bg_widget.with_alpha(0.5), ..rect });
    ui::paint_text_centered(ctx, list, label, rect, ctx.metrics.typography.body, ctx.theme.text_muted.with_alpha(0.6));
}

struct Cursor {
    x: f32,
    y: f32,
    width: f32,
}

impl Cursor {
    fn take(&mut self, height: f32) -> Rect {
        let rect = Rect { x: self.x, y: self.y, width: self.width, height, radius: 0.0, color: Color::TRANSPARENT };
        self.y += height;
        rect
    }

    fn gap(&mut self, height: f32) {
        self.y += height;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_values_take_optional_units() {
        assert_eq!(parse_value(Drag::Frequency, "1000"), Some(1000.0));
        assert_eq!(parse_value(Drag::Frequency, "1.2k"), Some(1200.0));
        assert_eq!(parse_value(Drag::Frequency, " 2 kHz "), Some(2000.0));
        assert_eq!(parse_value(Drag::Frequency, "440 Hz"), Some(440.0));
        assert_eq!(parse_value(Drag::Gain, "+3.5 dB"), Some(3.5));
        assert_eq!(parse_value(Drag::Gain, "-2"), Some(-2.0));
        assert_eq!(parse_value(Drag::Gain, "2k"), None, "k only means kHz");
        assert_eq!(parse_value(Drag::Q, "0.71"), Some(0.71));
        assert_eq!(parse_value(Drag::Q, "loud"), None);
        assert_eq!(parse_value(Drag::Q, ""), None);
        assert_eq!(parse_value(Drag::Q, "inf"), None);
    }

    #[test]
    fn a_typed_value_is_rounded_and_clamped() {
        let mut page = EqualizerPage::new();
        page.model.reset();
        page.commit(Drag::Gain, 0, "3.46");
        assert_eq!(page.model.bands[0].gain_db, 3.5);
        page.commit(Drag::Gain, 0, "999");
        assert_eq!(page.model.bands[0].gain_db, page.model.limits.max_gain);
        page.commit(Drag::Frequency, 1, "1.5k");
        assert_eq!(page.model.bands[1].frequency_hz, 1500.0);
        let q = page.model.bands[2].q;
        page.commit(Drag::Q, 2, "not a number");
        assert_eq!(page.model.bands[2].q, q, "left as it was");
        page.commit(Drag::Preamp, 0, "-4");
        assert_eq!(page.model.preamp_db, -4.0);
    }

    fn snapshot(active: &str) -> Value {
        json!({
            "device_id": "speakers",
            "active_preset": active,
            "capabilities": {"parametric": {"gain_range_db": [-12.0, 12.0], "freq_range_hz": [20.0, 20000.0], "q_range": [0.1, 10.0], "max_bands": 12}},
            "presets": [
                {"name": "Flat", "is_builtin": true, "parametric_bands": [], "bands": [0,0,0,0,0,0,0,0,0,0], "preamp_db": 0.0},
                {"name": "Bass Boost", "is_builtin": true, "parametric_bands": [
                    {"frequency_hz": 60.0, "gain_db": 6.0, "q": 0.9, "filter_type": "low_shelf", "enabled": true}
                ], "preamp_db": -2.0},
                {"name": "Mine", "is_builtin": false, "parametric_bands": [
                    {"frequency_hz": 1000.0, "gain_db": 3.0, "q": 1.0, "filter_type": "peaking", "enabled": true}
                ], "preamp_db": 0.0},
            ],
        })
    }

    fn loaded(active: &str) -> EqModel {
        let mut model = EqModel::default();
        assert!(matches!(model.sync("speakers", &Value::Null), Some(SystemAction::EqRefresh { .. })));
        assert!(model.sync("speakers", &snapshot(active)).is_none());
        model
    }

    #[test]
    fn a_new_output_loads_its_active_preset() {
        let model = loaded("Bass Boost");
        assert_eq!(model.preset, "Bass Boost");
        assert_eq!(model.bands.len(), 1);
        assert_eq!(model.bands[0].filter_type, "low_shelf");
        assert_eq!(model.preamp_db, -2.0);
        assert_eq!(model.limits.max_gain, 12.0, "limits come from the backend");
    }

    #[test]
    fn a_legacy_flat_preset_becomes_ten_bands() {
        let model = loaded("Flat");
        assert_eq!(model.bands.len(), 10);
        assert!(model.bands.iter().all(|b| b.gain_db == 0.0));
    }

    #[test]
    fn edits_are_clamped_and_make_the_preset_custom() {
        let mut model = loaded("Mine");
        model.set_gain(0, 40.0);
        assert_eq!(model.bands[0].gain_db, 12.0);
        assert_eq!(model.preset, "Custom");
        model.set_frequency(0, 5.0);
        assert_eq!(model.bands[0].frequency_hz, 20.0);
        model.set_q(0, 99.0);
        assert_eq!(model.bands[0].q, 10.0);
        // A republished snapshot must not throw the edit away.
        model.sync("speakers", &snapshot("Mine"));
        assert_eq!(model.bands[0].gain_db, 12.0);
    }

    #[test]
    fn bands_are_added_above_the_last_and_the_last_one_stays() {
        let mut model = loaded("Mine");
        model.add_band();
        assert_eq!(model.bands[1].frequency_hz, 1350.0);
        assert_eq!(model.selected, 1);
        model.remove_band(0);
        model.remove_band(0);
        assert_eq!(model.bands.len(), 1, "never removes the last band");
        for _ in 0..30 {
            model.add_band();
        }
        assert_eq!(model.bands.len(), 12, "up to the backend's maximum");
    }

    #[test]
    fn applying_a_preset_adopts_its_bands_when_it_becomes_active() {
        let mut model = loaded("Mine");
        model.set_gain(0, 1.0);
        let action = model.apply_preset("Bass Boost");
        assert!(matches!(action, Some(SystemAction::EqApplyPreset { ref name, .. }) if name == "Bass Boost"));
        // Until the backend reports it active, the edit stays.
        model.sync("speakers", &snapshot("Mine"));
        assert_eq!(model.bands[0].frequency_hz, 1000.0);
        model.sync("speakers", &snapshot("Bass Boost"));
        assert_eq!(model.preset, "Bass Boost");
        assert_eq!(model.bands[0].frequency_hz, 60.0);
    }

    #[test]
    fn reset_is_ten_flat_bands_applied_at_once() {
        let mut model = loaded("Bass Boost");
        let Some(SystemAction::EqApply { bands, preamp_db, .. }) = model.reset() else {
            panic!("reset applies");
        };
        assert_eq!(bands.as_array().unwrap().len(), 10);
        assert_eq!(preamp_db, 0.0);
    }

    #[test]
    fn only_custom_presets_can_be_deleted() {
        let mut model = loaded("Bass Boost");
        assert!(!model.can_delete());
        assert!(model.delete_current().is_none());
        let mut model = loaded("Mine");
        assert!(matches!(model.delete_current(), Some(SystemAction::EqDelete { ref name, .. }) if name == "Mine"));
    }

    #[test]
    fn saving_needs_a_name() {
        let mut model = loaded("Mine");
        assert!(model.save("  ").is_none());
        assert!(matches!(model.save(" Night "), Some(SystemAction::EqSave { ref name, .. }) if name == "Night"));
        assert_eq!(model.preset, "Night");
    }

    #[test]
    fn the_curve_peaks_at_a_peaking_band_and_previews_drags() {
        let model = loaded("Mine");
        let at = model.response_db(1000.0, None);
        assert!((at - 3.0).abs() < 1e-6, "a bell peaks at its centre: {at}");
        assert!(model.response_db(100.0, None) < 0.5, "and falls away from it");
        let preview = model.response_db(1000.0, Some((0, -6.0)));
        assert!((preview + 6.0).abs() < 1e-6);
        // Disabled bands contribute nothing.
        let mut off = model.clone();
        off.toggle_enabled(0);
        assert_eq!(off.response_db(1000.0, None), 0.0);
    }

    #[test]
    fn the_axis_is_logarithmic() {
        let limits = Limits::default();
        assert!((limits.norm_of(20.0)).abs() < 1e-9);
        assert!((limits.norm_of(20_000.0) - 1.0).abs() < 1e-9);
        let mid = limits.freq_at(0.5);
        assert!((mid - 632.45).abs() < 0.1, "the middle of 20 Hz–20 kHz is ~632 Hz: {mid}");
        assert!((limits.norm_of(limits.freq_at(0.3)) - 0.3).abs() < 1e-9);
    }

    #[test]
    fn filter_types_cycle_through_the_backends_list() {
        let mut model = loaded("Mine");
        for expected in ["low_shelf", "high_shelf", "low_pass", "high_pass", "band_pass", "peaking"] {
            model.cycle_type(0);
            assert_eq!(model.bands[0].filter_type, expected);
        }
    }
}
