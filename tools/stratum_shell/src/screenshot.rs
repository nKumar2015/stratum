//! The screenshot overlay: freeze the screen, pick what to capture, capture it.
//!
//! Matching `modules/ScreenshotToolbar.qml`:
//!
//! - `stratum-cli screenshot start` covers every output with an overlay that
//!   shows a frozen frame of that output, so what is picked is what was on
//!   screen when the shot was asked for.
//! - Hovering highlights the window under the pointer (the most recently
//!   focused one where windows overlap), or the whole output where there is
//!   none. A click captures that window, grown by `window_margin` on every
//!   side, or the whole output. Pressing and moving past `drag_threshold`
//!   selects a region instead, captured on release.
//! - A crosshair follows the pointer, and the selection's size is shown
//!   beside it (the geometry readout).
//! - Capturing cuts the pick out of the frozen frame, so the image is what
//!   the user saw while picking: running `grim` again caught whatever had
//!   moved since. Only without a frame does it hide the overlay, wait
//!   `capture_delay_ms` for the compositor to present the screen without it,
//!   and run `grim`. The image then opens in the screenshot viewer. A failed
//!   capture reports it as a notification and goes back to picking, as QML
//!   did.
//! - Escape closes it.
//!
//! The frozen frames come from `grim -t ppm` on stdout, decoded straight into
//! the backdrop texture: QML wrote a PNG to disk and loaded it back. Window
//! geometry comes from the Hyprland state the shell already keeps, where QML
//! spawned `hyprctl` every 45 ms while the pointer moved.

use std::collections::HashMap;
use std::sync::Arc;

use crate::render::display_list::{Backdrop, Color, DisplayList, Point, Rect};
use crate::ui::{self, PaintCtx};

/// What the user picked, as the viewer is told (`mode` in the QML payload).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Window,
    Region,
    Fullscreen,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Window => "window",
            Mode::Region => "region",
            Mode::Fullscreen => "fullscreen",
        }
    }
}

/// A capture to run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Capture {
    /// `grim -g "<x>,<y> <w>x<h>"`, in global layout coordinates.
    Geometry { x: i32, y: i32, width: i32, height: i32, mode: Mode },
    /// `grim -o <output>`.
    Output { output: String },
}

impl Capture {
    pub fn mode(&self) -> Mode {
        match self {
            Capture::Geometry { mode, .. } => *mode,
            Capture::Output { .. } => Mode::Fullscreen,
        }
    }
}

/// A window's rect in global layout coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlobalRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Closed,
    Picking,
    /// Hidden while the capture runs.
    Capturing,
}

pub struct Screenshot {
    phase: Phase,
    /// Where it was started, which takes the keyboard (for Escape).
    origin: String,
    /// Each output's frozen frame, once `grim` has delivered it. An output's
    /// overlay is shown only once its frame is here.
    frames: HashMap<String, Backdrop>,
    /// The pointer's output and position on it.
    pointer: Option<(String, Point)>,
    /// Where the left button went down, and on which output.
    press: Option<(String, Point)>,
    /// The region being dragged, once the press has moved far enough.
    drag: Option<Rect>,
    /// The window under the pointer, in global coordinates.
    hover: Option<GlobalRect>,
    next_key: u64,
    metrics: crate::ui::style::ScreenshotMetrics,
}

impl Screenshot {
    pub fn new(metrics: &crate::ui::style::ScreenshotMetrics) -> Self {
        Self {
            phase: Phase::Closed,
            origin: String::new(),
            frames: HashMap::new(),
            pointer: None,
            press: None,
            drag: None,
            hover: None,
            next_key: 1,
            metrics: metrics.clone(),
        }
    }

    pub fn set_metrics(&mut self, metrics: &crate::ui::style::ScreenshotMetrics) {
        self.metrics = metrics.clone();
    }

    pub fn metrics(&self) -> &crate::ui::style::ScreenshotMetrics {
        &self.metrics
    }

    pub fn is_open(&self) -> bool {
        self.phase != Phase::Closed
    }

    /// Start picking. Returns false when already open, so a repeated keybind
    /// does not freeze the screen again (QML ignored it too).
    pub fn open_on(&mut self, output: &str) -> bool {
        if self.is_open() {
            return false;
        }
        self.phase = Phase::Picking;
        self.origin = output.to_string();
        self.reset_selection();
        true
    }

    pub fn close(&mut self) {
        self.phase = Phase::Closed;
        // A full frame per output is tens of megabytes; nothing keeps them.
        let had_frames = !self.frames.is_empty();
        self.frames.clear();
        self.reset_selection();
        if had_frames {
            crate::memory::release_freed();
        }
    }

    fn reset_selection(&mut self) {
        self.press = None;
        self.drag = None;
        self.hover = None;
    }

    /// The output that takes the keyboard while open.
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// Whether `output`'s overlay should be on screen.
    pub fn shows(&self, output: &str) -> bool {
        self.phase == Phase::Picking && self.frames.contains_key(output)
    }

    /// A frozen frame arrived for `output`. Premultiplied RGBA; a screen is
    /// opaque, so it already is.
    pub fn frozen(&mut self, output: &str, width: u32, height: u32, pixels: Vec<u8>) {
        if self.phase == Phase::Closed {
            return;
        }
        let key = self.next_key;
        self.next_key += 1;
        self.frames.insert(
            output.to_string(),
            Backdrop { key, width, height, pixels: Arc::from(pixels), dest: None, clip: None },
        );
    }

    /// The pointer moved on `output` to `point`; `window` is the window under
    /// it, if any. Returns whether anything visible changed.
    pub fn motion(&mut self, output: &str, point: Point, window: Option<GlobalRect>) -> bool {
        if self.phase != Phase::Picking {
            return false;
        }
        self.pointer = Some((output.to_string(), point));
        if let Some((press_output, press)) = &self.press {
            if press_output == output {
                let (dx, dy) = (point.x - press.x, point.y - press.y);
                let threshold = self.metrics.drag_threshold;
                if self.drag.is_some() || dx.abs() >= threshold || dy.abs() >= threshold {
                    self.drag = Some(Rect {
                        x: press.x.min(point.x),
                        y: press.y.min(point.y),
                        width: dx.abs().max(1.0),
                        height: dy.abs().max(1.0),
                        radius: 0.0,
                        color: Color::TRANSPARENT,
                    });
                }
            }
        } else {
            self.hover = window;
        }
        true
    }

    /// The pointer left `output`.
    pub fn leave(&mut self, output: &str) {
        if self.pointer.as_ref().is_some_and(|(o, _)| o == output) {
            self.pointer = None;
            if self.press.is_none() {
                self.hover = None;
            }
        }
    }

    pub fn press(&mut self, output: &str, point: Point) {
        if self.phase != Phase::Picking || !self.frames.contains_key(output) {
            return;
        }
        self.press = Some((output.to_string(), point));
        self.drag = None;
    }

    /// The left button came up. `origin` is the output's top-left in global
    /// coordinates. Returns the capture to run, if the release completes one.
    pub fn release(&mut self, output: &str, origin: (i32, i32)) -> Option<Capture> {
        let (press_output, _) = self.press.take()?;
        if self.phase != Phase::Picking || press_output != output {
            self.drag = None;
            return None;
        }
        let capture = if let Some(region) = self.drag.take() {
            Capture::Geometry {
                x: (region.x + origin.0 as f32).round() as i32,
                y: (region.y + origin.1 as f32).round() as i32,
                width: region.width.round().max(1.0) as i32,
                height: region.height.round().max(1.0) as i32,
                mode: Mode::Region,
            }
        } else if let Some(window) = self.hover {
            let margin = self.metrics.window_margin.max(0.0).round() as i32;
            Capture::Geometry {
                x: window.x - margin,
                y: window.y - margin,
                width: (window.width + margin * 2).max(1),
                height: (window.height + margin * 2).max(1),
                mode: Mode::Window,
            }
        } else {
            Capture::Output { output: output.to_string() }
        };
        self.phase = Phase::Capturing;
        self.reset_selection();
        Some(capture)
    }

    /// `capture` cut out of the frame frozen on `output`, whose top-left is
    /// `origin` and logical size `size` in global layout coordinates.
    /// Premultiplied RGBA. `None` without a frame, or when none of the
    /// capture is on it.
    pub fn crop_frozen(
        &self,
        output: &str,
        origin: (i32, i32),
        size: (i32, i32),
        capture: &Capture,
    ) -> Option<(u32, u32, Vec<u8>)> {
        let frame = self.frames.get(output)?;
        crop(frame.width, frame.height, &frame.pixels, origin, size, capture)
    }

    /// A capture failed: pick again, over the same frozen frames.
    pub fn capture_failed(&mut self) {
        if self.phase == Phase::Capturing {
            self.phase = Phase::Picking;
        }
    }

    /// Paint `output`'s overlay. `origin` is its top-left in global
    /// coordinates, to place the hovered window.
    pub fn paint(
        &self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        surface: Rect,
        output: &str,
        origin: (i32, i32),
    ) {
        let Some(frame) = self.frames.get(output) else {
            return;
        };
        list.backdrop = Some(frame.clone());
        let m = &self.metrics;
        let theme = ctx.theme;
        let pointer = self
            .pointer
            .as_ref()
            .filter(|(o, _)| o == output)
            .map(|(_, p)| *p);
        let dragging_here = self.press.as_ref().is_some_and(|(o, _)| o == output);

        // What a release would capture, outlined.
        let selection = if let (Some(region), true) = (self.drag, dragging_here) {
            Some((Rect { radius: m.region_radius, ..region }, m.fill_alpha))
        } else if pointer.is_some() && self.drag.is_none() {
            Some(match self.hover {
                Some(w) => (
                    Rect {
                        x: (w.x - origin.0) as f32,
                        y: (w.y - origin.1) as f32,
                        width: w.width as f32,
                        height: w.height as f32,
                        radius: m.window_radius,
                        color: Color::TRANSPARENT,
                    },
                    m.fill_alpha,
                ),
                None => (Rect { radius: 0.0, ..surface }, m.output_fill_alpha),
            })
        } else {
            None
        };
        if let Some((rect, alpha)) = selection {
            list.push_rect(Rect { color: theme.primary.with_alpha(alpha), ..rect });
            let inset = m.border * 0.5;
            let outline = Rect {
                x: rect.x + inset,
                y: rect.y + inset,
                width: (rect.width - m.border).max(0.0),
                height: (rect.height - m.border).max(0.0),
                radius: (rect.radius - inset).max(0.0),
                ..rect
            };
            ui::stroke_rounded(list, outline, theme.primary, m.border);
            self.paint_readout(ctx, list, surface, rect);
        }

        if let Some(p) = pointer {
            self.paint_crosshair(ctx, list, p);
        }
    }

    /// The selection's size in a pill below it, or above it (then inside)
    /// when there is no room.
    fn paint_readout(&self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, surface: Rect, rect: Rect) {
        let m = &self.metrics;
        let text = format!("{} × {}", rect.width.round() as i32, rect.height.round() as i32);
        let width = ctx.measure(&text, m.label) + m.label_padding * 2.0;
        let below = rect.y + rect.height + m.label_gap;
        let y = if below + m.label_height <= surface.y + surface.height {
            below
        } else if rect.y - m.label_gap - m.label_height >= surface.y {
            rect.y - m.label_gap - m.label_height
        } else {
            rect.y + rect.height - m.label_gap - m.label_height
        };
        let x = rect.x.clamp(surface.x, (surface.x + surface.width - width).max(surface.x));
        let pill = Rect {
            x,
            y,
            width,
            height: m.label_height,
            radius: m.label_radius,
            color: ctx.theme.bg_main,
        };
        list.push_rect(pill);
        ui::paint_text_centered(ctx, list, &text, pill, m.label, ctx.theme.text_main);
    }

    /// A dark cross with a light outline, readable over any frame.
    fn paint_crosshair(&self, ctx: &PaintCtx<'_>, list: &mut DisplayList, p: Point) {
        let m = &self.metrics;
        let (x, y) = (p.x.round(), p.y.round());
        let bar = |arm: f32, width: f32, vertical: bool, color: Color| {
            let (w, h) = if vertical { (width, arm * 2.0 + 1.0) } else { (arm * 2.0 + 1.0, width) };
            Rect { x: x + 0.5 - w * 0.5, y: y + 0.5 - h * 0.5, width: w, height: h, radius: 0.0, color }
        };
        let (arm, width, outline) = (m.crosshair_arm, m.crosshair_width, m.crosshair_outline);
        for vertical in [true, false] {
            list.push_rect(bar(arm + outline, width + outline * 2.0, vertical, ctx.theme.text_main));
        }
        for vertical in [true, false] {
            list.push_rect(bar(arm, width, vertical, ctx.theme.bg_dark));
        }
    }
}

/// Freeze `output`: its current frame as opaque RGBA8, at its buffer size.
pub fn freeze(output: &str) -> Result<(u32, u32, Vec<u8>), String> {
    let result = std::process::Command::new("grim")
        .args(["-t", "ppm", "-o", output, "-"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|err| format!("grim: {err}"))?;
    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr).trim().to_string();
        return Err(if stderr.is_empty() { "Could not freeze screen".into() } else { stderr });
    }
    decode_ppm(&result.stdout).ok_or_else(|| "Invalid freeze frame".to_string())
}

/// Decode a binary (P6, 8-bit) PPM into RGBA8.
pub fn decode_ppm(data: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let mut pos = 0;
    let mut field = || -> Option<&[u8]> {
        // Whitespace and `#` comments separate header fields.
        loop {
            match data.get(pos)? {
                b'#' => {
                    while *data.get(pos)? != b'\n' {
                        pos += 1;
                    }
                }
                c if c.is_ascii_whitespace() => pos += 1,
                _ => break,
            }
        }
        let start = pos;
        while data.get(pos).is_some_and(|c| !c.is_ascii_whitespace()) {
            pos += 1;
        }
        Some(&data[start..pos])
    };
    if field()? != b"P6" {
        return None;
    }
    let mut number = || -> Option<u32> { std::str::from_utf8(field()?).ok()?.parse().ok() };
    let (width, height, max) = (number()?, number()?, number()?);
    if max != 255 || width == 0 || height == 0 {
        return None;
    }
    // Exactly one whitespace byte ends the header.
    let body = data.get(pos + 1..)?;
    let count = (width as usize).checked_mul(height as usize)?;
    if body.len() < count * 3 {
        return None;
    }
    let mut rgba = Vec::with_capacity(count * 4);
    for px in body[..count * 3].chunks_exact(3) {
        rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
    }
    Some((width, height, rgba))
}

/// Cut `capture` out of a `width`×`height` RGBA frame of an output at
/// `origin` with logical size `size`. The rect is clamped to the frame, so a
/// window that overhangs the output keeps only its part on it.
fn crop(
    width: u32,
    height: u32,
    pixels: &[u8],
    origin: (i32, i32),
    size: (i32, i32),
    capture: &Capture,
) -> Option<(u32, u32, Vec<u8>)> {
    if size.0 <= 0 || size.1 <= 0 || pixels.len() < (width * height * 4) as usize {
        return None;
    }
    let (x, y, w, h) = match capture {
        Capture::Geometry { x, y, width, height, .. } => (x - origin.0, y - origin.1, *width, *height),
        Capture::Output { .. } => (0, 0, size.0, size.1),
    };
    // Frame pixels per logical pixel: 1.6 on an output scaled by 1.6.
    let (sx, sy) = (width as f32 / size.0 as f32, height as f32 / size.1 as f32);
    let edge = |v: i32, scale: f32, limit: u32| ((v as f32 * scale).round().max(0.0) as u32).min(limit);
    let (left, right) = (edge(x, sx, width), edge(x + w, sx, width));
    let (top, bottom) = (edge(y, sy, height), edge(y + h, sy, height));
    if right <= left || bottom <= top {
        return None;
    }
    let row_bytes = ((right - left) * 4) as usize;
    let mut out = Vec::with_capacity(row_bytes * (bottom - top) as usize);
    for row in top..bottom {
        let start = ((row * width + left) * 4) as usize;
        out.extend_from_slice(&pixels[start..start + row_bytes]);
    }
    Some((right - left, bottom - top, out))
}

/// Write an opaque RGBA image to a new temporary PNG and return its path.
/// The viewer owns the file from then on.
pub fn save_png(width: u32, height: u32, pixels: &[u8]) -> Result<String, String> {
    let path = temp_path().ok_or("failed to allocate output file")?;
    image::save_buffer(&path, pixels, width, height, image::ExtendedColorType::Rgba8).map_err(|err| {
        let _ = std::fs::remove_file(&path);
        format!("failed to write the capture: {err}")
    })?;
    Ok(path)
}

/// Run `capture` with `grim` into a new temporary PNG and return its path.
/// The viewer owns the file from then on.
pub fn capture(capture: &Capture) -> Result<String, String> {
    let path = temp_path().ok_or("failed to allocate output file")?;
    let mut command = std::process::Command::new("grim");
    match capture {
        Capture::Geometry { x, y, width, height, .. } => {
            command.args(["-g", &format!("{x},{y} {width}x{height}")]);
        }
        Capture::Output { output } => {
            command.args(["-o", output]);
        }
    }
    let ok = command
        .arg(&path)
        .stdin(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    if !ok || size == 0 {
        let _ = std::fs::remove_file(&path);
        return Err(if ok { "capture produced empty file" } else { "grim capture failed" }.into());
    }
    Ok(path)
}

/// A failure, reported as a critical notification.
pub fn failure(summary: &str, body: &str) -> crate::actions::SystemAction {
    tracing::warn!("screenshot: {summary}: {body}");
    crate::actions::SystemAction::Notify {
        app: "Screenshot",
        icon: "",
        urgency: 2,
        summary: summary.into(),
        body: body.into(),
    }
}

/// A fresh file in the runtime directory (the tmpfs QML used too), created
/// exclusively so two captures cannot share it.
pub(crate) fn temp_path() -> Option<String> {
    let dir = crate::paths::runtime_dir().display().to_string();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    for attempt in 0..16 {
        let path = format!("{dir}/stratum-screenshot-{}-{stamp}-{attempt}.png", std::process::id());
        if std::fs::OpenOptions::new().write(true).create_new(true).open(&path).is_ok() {
            return Some(path);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opened() -> Screenshot {
        let mut shot = Screenshot::new(&crate::config::Config::test_defaults().metrics.screenshot);
        assert!(shot.open_on("A"));
        shot.frozen("A", 2, 2, vec![0; 16]);
        shot
    }

    /// A frame whose every pixel holds its own coordinates: `[x, y, 0, 255]`.
    fn coordinate_frame(width: u32, height: u32) -> Vec<u8> {
        (0..height)
            .flat_map(|y| (0..width).flat_map(move |x| [x as u8, y as u8, 0, 255]))
            .collect()
    }

    fn region(x: i32, y: i32, width: i32, height: i32) -> Capture {
        Capture::Geometry { x, y, width, height, mode: Mode::Region }
    }

    #[test]
    fn a_region_is_cut_from_the_frozen_frame_at_the_output_scale() {
        // 10x5 logical at scale 1.6 is a 16x8 frame, left of the origin as
        // HDMI-A-1 is.
        let frame = coordinate_frame(16, 8);
        let (w, h, pixels) = crop(16, 8, &frame, (-10, 0), (10, 5), &region(-5, 0, 5, 5)).unwrap();
        assert_eq!((w, h), (8, 8));
        assert_eq!(&pixels[..4], &[8, 0, 0, 255], "starts 5 logical px in");
        assert_eq!(&pixels[pixels.len() - 4..], &[15, 7, 0, 255]);
    }

    #[test]
    fn a_crop_keeps_only_what_is_on_the_output() {
        let frame = coordinate_frame(4, 4);
        let (w, h, pixels) = crop(4, 4, &frame, (0, 0), (4, 4), &region(-2, 2, 4, 9)).unwrap();
        assert_eq!((w, h), (2, 2));
        assert_eq!(&pixels[..4], &[0, 2, 0, 255]);
        assert_eq!(crop(4, 4, &frame, (0, 0), (4, 4), &region(5, 0, 2, 2)), None);
    }

    #[test]
    fn an_output_capture_is_the_whole_frame() {
        let frame = coordinate_frame(3, 2);
        let output = Capture::Output { output: "A".into() };
        assert_eq!(crop(3, 2, &frame, (7, 7), (3, 2), &output), Some((3, 2, frame.clone())));
    }

    #[test]
    fn an_output_shows_only_once_frozen() {
        let mut shot = Screenshot::new(&crate::config::Config::test_defaults().metrics.screenshot);
        shot.open_on("A");
        assert!(!shot.shows("A"));
        shot.frozen("A", 1, 1, vec![0; 4]);
        assert!(shot.shows("A"));
        assert!(!shot.shows("B"));
        assert!(!shot.open_on("A"), "a second start is ignored");
        shot.close();
        assert!(!shot.shows("A"));
        shot.frozen("A", 1, 1, vec![0; 4]);
        assert!(!shot.shows("A"), "a late frame after close is dropped");
    }

    #[test]
    fn a_click_on_a_window_captures_it_with_a_margin() {
        let mut shot = opened();
        let window = GlobalRect { x: 1700, y: 100, width: 400, height: 300 };
        shot.motion("A", Point::new(200.0, 150.0), Some(window));
        shot.press("A", Point::new(200.0, 150.0));
        assert_eq!(
            shot.release("A", (1600, 0)),
            Some(Capture::Geometry { x: 1695, y: 95, width: 410, height: 310, mode: Mode::Window })
        );
        assert!(!shot.shows("A"), "hidden while capturing");
    }

    #[test]
    fn a_click_with_no_window_captures_the_output() {
        let mut shot = opened();
        shot.motion("A", Point::new(5.0, 5.0), None);
        shot.press("A", Point::new(5.0, 5.0));
        // Moving less than the threshold is still a click.
        shot.motion("A", Point::new(10.0, 9.0), None);
        assert_eq!(shot.release("A", (0, 0)), Some(Capture::Output { output: "A".into() }));
    }

    #[test]
    fn a_drag_captures_the_region_in_global_coordinates() {
        let mut shot = opened();
        shot.press("A", Point::new(300.0, 200.0));
        shot.motion("A", Point::new(100.4, 260.0), None);
        assert_eq!(
            shot.release("A", (-2560, 0)),
            Some(Capture::Geometry { x: -2460, y: 200, width: 200, height: 60, mode: Mode::Region })
        );
    }

    #[test]
    fn a_failed_capture_returns_to_picking() {
        let mut shot = opened();
        shot.press("A", Point::new(1.0, 1.0));
        assert!(shot.release("A", (0, 0)).is_some());
        shot.capture_failed();
        assert!(shot.shows("A"));
    }

    #[test]
    fn a_release_without_a_press_does_nothing() {
        let mut shot = opened();
        assert_eq!(shot.release("A", (0, 0)), None);
        shot.press("B", Point::new(1.0, 1.0));
        assert_eq!(shot.release("A", (0, 0)), None, "B has no frame, so no press");
    }

    #[test]
    fn ppm_decodes_with_comments() {
        let mut data = b"P6\n# grim\n2 1\n255\n".to_vec();
        data.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
        assert_eq!(decode_ppm(&data), Some((2, 1, vec![1, 2, 3, 255, 4, 5, 6, 255])));
        assert_eq!(decode_ppm(b"P6\n2 1\n255\n\x01\x02"), None, "truncated");
        assert_eq!(decode_ppm(b"P5\n1 1\n255\n\x01"), None, "greyscale");
    }
}
