//! The screenshot viewer: look at a capture, draw on it, copy or save it.
//!
//! Matching `modules/ScreenshotViewer.qml` and its helpers:
//!
//! - A normal window titled "Screenshot Viewer", showing the capture mode,
//!   with clear, copy, save, save-as and close buttons. Copy, save and save-as
//!   show a busy glyph while running, and every button is disabled then.
//! - A pen row: colour presets (`[screenshot] pen_colors`), the pen size, and
//!   the zoom with its percentage.
//! - The image, fitted to the canvas. Ctrl+wheel zooms about the pointer
//!   (1×–8×); a middle-button or Ctrl drag pans, clamped so the image never
//!   leaves the canvas more than it must. The left button draws.
//! - Ctrl+Z undoes the last stroke, the clear button removes them all, and
//!   Escape or closing the window closes the viewer.
//! - Copy goes through `wl-copy`, and Save As asks the desktop portal's
//!   FileChooser. With strokes, the image is first composed at full
//!   resolution into a temporary file, which is removed once the action has
//!   finished.
//! - The result shows in a banner for `status_ms`; a failed action is also
//!   reported as a notification.
//!
//! One change from QML: the capture's own temporary file is removed when the
//! viewer closes or shows another capture. QML left every capture in the
//! runtime directory (which is memory) until logout, where nothing could
//! reach it again.
//!
//! Strokes are kept in image pixels, so zooming and panning never disturb
//! them and the export draws them exactly where they were drawn. A pen size
//! is in on-screen pixels at 100 % zoom, as in QML.

use std::sync::Arc;

use crate::render::display_list::{Backdrop, Color, DisplayList, Paint, Path, PathCommand, Point, Rect};
use crate::screenshot::Mode;
use crate::ui::{self, PaintCtx};

/// A freehand line, in image pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    pub color: Color,
    pub width: f32,
    pub points: Vec<(f32, f32)>,
}

/// A decoded capture.
#[derive(Clone, Debug)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<[u8]>,
}

/// What to do with the capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostAction {
    Copy,
    Save,
    SaveTo,
}

impl PostAction {
    fn done(self) -> &'static str {
        match self {
            PostAction::Copy => "Copied to clipboard",
            PostAction::Save | PostAction::SaveTo => "Saved screenshot",
        }
    }
}

/// What the shell has to do after input.
#[derive(Clone, Debug, PartialEq)]
pub enum ViewerAction {
    None,
    Redraw,
    Close,
    /// Run `action` on the image (with its strokes); `target` for save-to.
    Post { action: PostAction, target: Option<String> },
    /// Ask the portal where to save.
    SaveAs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Button {
    Clear,
    Copy,
    Save,
    SaveAs,
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slider {
    Pen,
    Zoom,
}

/// What a held button is doing.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Gesture {
    Drawing,
    Panning { last: Point },
    Sliding(Slider),
}

/// Rects of the last paint, for hit testing.
#[derive(Clone, Debug)]
struct Layout {
    buttons: Vec<(Button, Rect)>,
    swatches: Vec<Rect>,
    pen: Rect,
    zoom: Rect,
    canvas: Rect,
}

impl Default for Layout {
    fn default() -> Self {
        let empty = Rect { x: 0.0, y: 0.0, width: 0.0, height: 0.0, radius: 0.0, color: Color::TRANSPARENT };
        Self { buttons: Vec::new(), swatches: Vec::new(), pen: empty, zoom: empty, canvas: empty }
    }
}

pub struct ScreenshotViewer {
    open: bool,
    path: String,
    mode: Mode,
    /// Whether `path` is a capture of ours, to remove on close.
    owned: bool,
    image: Option<Image>,
    image_key: u64,
    strokes: Vec<Stroke>,
    color: usize,
    pen: u32,
    zoom: f32,
    pan: (f32, f32),
    gesture: Option<Gesture>,
    pointer: Option<Point>,
    working: bool,
    status: Option<(String, bool, std::time::Instant)>,
    layout: Layout,
    metrics: crate::ui::style::ScreenshotViewerMetrics,
    pen_max: u32,
    colors: Vec<Color>,
}

impl ScreenshotViewer {
    pub fn new(
        metrics: &crate::ui::style::ScreenshotViewerMetrics,
        config: &crate::config::ScreenshotConfig,
    ) -> Self {
        Self {
            open: false,
            path: String::new(),
            mode: Mode::Fullscreen,
            owned: false,
            image: None,
            image_key: 0,
            strokes: Vec::new(),
            color: 0,
            pen: config.pen_size,
            zoom: 1.0,
            pan: (0.0, 0.0),
            gesture: None,
            pointer: None,
            working: false,
            status: None,
            layout: Layout::default(),
            metrics: metrics.clone(),
            pen_max: config.pen_size_max,
            colors: config.colors(),
        }
    }

    pub fn configure(
        &mut self,
        metrics: &crate::ui::style::ScreenshotViewerMetrics,
        config: &crate::config::ScreenshotConfig,
    ) {
        self.metrics = metrics.clone();
        self.pen_max = config.pen_size_max.max(1);
        self.colors = config.colors();
        self.color = self.color.min(self.colors.len().saturating_sub(1));
        self.pen = self.pen.clamp(1, self.pen_max);
    }

    pub fn metrics(&self) -> &crate::ui::style::ScreenshotViewerMetrics {
        &self.metrics
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn strokes(&self) -> &[Stroke] {
        &self.strokes
    }

    pub fn image(&self) -> Option<&Image> {
        self.image.as_ref()
    }

    /// Show a capture, replacing whatever was shown. `owned` marks a
    /// temporary file of ours, removed when it is no longer shown.
    pub fn show(&mut self, path: String, mode: Mode, owned: bool, image: Image) {
        self.release_file();
        self.open = true;
        self.path = path;
        self.mode = mode;
        self.owned = owned;
        self.image = Some(image);
        self.image_key += 1;
        self.strokes.clear();
        self.zoom = 1.0;
        self.pan = (0.0, 0.0);
        self.gesture = None;
        self.status = None;
    }

    pub fn close(&mut self) {
        self.release_file();
        self.open = false;
        self.image = None;
        self.strokes.clear();
        // The capture is a full image; give its pages back.
        crate::memory::release_freed();
        self.gesture = None;
        self.status = None;
        self.working = false;
    }

    fn release_file(&mut self) {
        if self.owned && !self.path.is_empty() {
            let _ = std::fs::remove_file(&self.path);
        }
        self.owned = false;
    }

    /// An action is running (buttons are disabled), or has finished with
    /// `result`: the banner text, or an error.
    pub fn set_working(&mut self, working: bool) {
        self.working = working;
    }

    pub fn finished(&mut self, result: Result<PostAction, String>) {
        self.working = false;
        match result {
            Ok(action) => self.show_status(action.done().to_string(), false),
            Err(error) => self.show_status(error, true),
        }
    }

    pub fn show_status(&mut self, text: String, error: bool) {
        self.status = (!text.is_empty()).then(|| (text, error, std::time::Instant::now()));
    }

    /// Whether the banner is still up; drops it once `status_ms` has passed.
    pub fn tick(&mut self) -> bool {
        let expired = self.status.as_ref().is_some_and(|(_, _, at)| {
            at.elapsed() >= std::time::Duration::from_millis(self.metrics.status_ms)
        });
        if expired {
            self.status = None;
            return true;
        }
        false
    }

    // ----- geometry -----------------------------------------------------

    /// The image fitted into the canvas at 100 %: its scale, and its size.
    fn fit(&self, canvas: Rect) -> (f32, f32, f32) {
        let Some(image) = &self.image else {
            return (1.0, 0.0, 0.0);
        };
        let (w, h) = (image.width.max(1) as f32, image.height.max(1) as f32);
        let scale = (canvas.width / w).min(canvas.height / h).max(0.0);
        (scale, w * scale, h * scale)
    }

    /// Where the image is drawn, zoomed and panned.
    fn image_rect(&self, canvas: Rect) -> Rect {
        let (_, w, h) = self.fit(canvas);
        let (zw, zh) = (w * self.zoom, h * self.zoom);
        Rect {
            x: canvas.x + (canvas.width - zw) * 0.5 + self.pan.0,
            y: canvas.y + (canvas.height - zh) * 0.5 + self.pan.1,
            width: zw,
            height: zh,
            radius: 0.0,
            color: Color::TRANSPARENT,
        }
    }

    /// Keep the image covering as much of the canvas as it can.
    fn clamp_pan(&self, canvas: Rect, zoom: f32, pan: (f32, f32)) -> (f32, f32) {
        let (_, w, h) = self.fit(canvas);
        let axis = |size: f32, room: f32, value: f32| {
            if size * zoom <= room {
                0.0
            } else {
                let limit = (size * zoom - room) * 0.5;
                value.clamp(-limit, limit)
            }
        };
        (axis(w, canvas.width, pan.0), axis(h, canvas.height, pan.1))
    }

    /// Zoom to `zoom`, keeping the image point under `focus` where it is.
    fn set_zoom(&mut self, zoom: f32, focus: Point) {
        let canvas = self.layout.canvas;
        let (_, w, h) = self.fit(canvas);
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let m = &self.metrics;
        let next = zoom.clamp(m.zoom_min, m.zoom_max.max(m.zoom_min));
        let old = self.image_rect(canvas);
        let content = ((focus.x - old.x) / self.zoom.max(1e-4), (focus.y - old.y) / self.zoom.max(1e-4));
        let pos = (focus.x - content.0 * next, focus.y - content.1 * next);
        let centred = (
            canvas.x + (canvas.width - w * next) * 0.5,
            canvas.y + (canvas.height - h * next) * 0.5,
        );
        self.pan = self.clamp_pan(canvas, next, (pos.0 - centred.0, pos.1 - centred.1));
        self.zoom = next;
    }

    /// A canvas point in image pixels, clamped to the image.
    fn to_image(&self, point: Point) -> (f32, f32) {
        let canvas = self.layout.canvas;
        let rect = self.image_rect(canvas);
        let Some(image) = &self.image else {
            return (0.0, 0.0);
        };
        let per_px = rect.width / image.width.max(1) as f32;
        let x = (point.x - rect.x) / per_px.max(1e-6);
        let y = (point.y - rect.y) / per_px.max(1e-6);
        (
            x.clamp(0.0, (image.width.max(1) - 1) as f32),
            y.clamp(0.0, (image.height.max(1) - 1) as f32),
        )
    }

    // ----- input --------------------------------------------------------

    pub fn motion(&mut self, point: Point) -> ViewerAction {
        self.pointer = Some(point);
        match self.gesture {
            Some(Gesture::Drawing) => {
                let p = self.to_image(point);
                if let Some(stroke) = self.strokes.last_mut() {
                    let moved = stroke
                        .points
                        .last()
                        .is_none_or(|last| (last.0 - p.0).abs() + (last.1 - p.1).abs() >= 0.5);
                    if moved {
                        stroke.points.push(p);
                    }
                }
            }
            Some(Gesture::Panning { last }) => {
                let canvas = self.layout.canvas;
                let next = (self.pan.0 + point.x - last.x, self.pan.1 + point.y - last.y);
                self.pan = self.clamp_pan(canvas, self.zoom, next);
                self.gesture = Some(Gesture::Panning { last: point });
            }
            Some(Gesture::Sliding(slider)) => self.slide(slider, point),
            None => {}
        }
        ViewerAction::Redraw
    }

    pub fn leave(&mut self) {
        self.pointer = None;
    }

    /// A press. `middle` is the middle button; `ctrl` turns a left press on
    /// the image into a pan, as in QML.
    pub fn press(&mut self, point: Point, middle: bool, ctrl: bool) -> ViewerAction {
        self.pointer = Some(point);
        let on_canvas = self.layout.canvas.contains(point) && self.image.is_some();
        if middle || (ctrl && on_canvas) {
            if on_canvas {
                self.gesture = Some(Gesture::Panning { last: point });
            }
            return ViewerAction::None;
        }
        if let Some((button, _)) = self.layout.buttons.iter().find(|(_, r)| r.contains(point)) {
            if self.working {
                return ViewerAction::None;
            }
            return match button {
                Button::Clear => {
                    self.strokes.clear();
                    ViewerAction::Redraw
                }
                Button::Copy => ViewerAction::Post { action: PostAction::Copy, target: None },
                Button::Save => ViewerAction::Post { action: PostAction::Save, target: None },
                Button::SaveAs => ViewerAction::SaveAs,
                Button::Close => ViewerAction::Close,
            };
        }
        if let Some(index) = self.layout.swatches.iter().position(|r| r.contains(point)) {
            self.color = index;
            return ViewerAction::Redraw;
        }
        for (slider, rect) in [(Slider::Pen, self.layout.pen), (Slider::Zoom, self.layout.zoom)] {
            if rect.contains(point) && (slider == Slider::Pen || self.image.is_some()) {
                self.gesture = Some(Gesture::Sliding(slider));
                self.slide(slider, point);
                return ViewerAction::Redraw;
            }
        }
        if on_canvas {
            let (scale, _, _) = self.fit(self.layout.canvas);
            let color = self.colors.get(self.color).copied().unwrap_or(Color::WHITE);
            self.strokes.push(Stroke {
                color,
                width: self.pen as f32 / scale.max(1e-6),
                points: vec![self.to_image(point)],
            });
            self.gesture = Some(Gesture::Drawing);
            return ViewerAction::Redraw;
        }
        ViewerAction::None
    }

    pub fn release(&mut self) -> ViewerAction {
        match self.gesture.take() {
            Some(_) => ViewerAction::Redraw,
            None => ViewerAction::None,
        }
    }

    /// Wheel input on the canvas; only Ctrl+wheel zooms, as in QML.
    pub fn scroll(&mut self, point: Point, steps: f32, ctrl: bool) -> ViewerAction {
        if !ctrl || !self.layout.canvas.contains(point) || steps == 0.0 {
            return ViewerAction::None;
        }
        // Wheel up (negative steps) zooms in.
        let step = if steps < 0.0 { self.metrics.zoom_wheel_step } else { -self.metrics.zoom_wheel_step };
        self.set_zoom(self.zoom + step, point);
        ViewerAction::Redraw
    }

    pub fn key(&mut self, keysym: smithay_client_toolkit::seat::keyboard::Keysym, ctrl: bool) -> ViewerAction {
        use smithay_client_toolkit::seat::keyboard::Keysym;
        match keysym {
            Keysym::Escape => ViewerAction::Close,
            Keysym::z | Keysym::Z if ctrl && !self.working => {
                self.strokes.pop();
                self.gesture = None;
                ViewerAction::Redraw
            }
            _ => ViewerAction::None,
        }
    }

    fn slide(&mut self, slider: Slider, point: Point) {
        let rect = match slider {
            Slider::Pen => self.layout.pen,
            Slider::Zoom => self.layout.zoom,
        };
        let t = crate::ui::controls::fraction_in(rect, point.x);
        match slider {
            Slider::Pen => self.pen = 1 + (t * (self.pen_max - 1) as f32).round() as u32,
            Slider::Zoom => {
                let m = &self.metrics;
                let raw = m.zoom_min + t * (m.zoom_max - m.zoom_min);
                let step = m.zoom_slider_step.max(1e-3);
                let snapped = m.zoom_min + ((raw - m.zoom_min) / step).round() * step;
                let canvas = self.layout.canvas;
                let centre = Point::new(canvas.x + canvas.width * 0.5, canvas.y + canvas.height * 0.5);
                self.set_zoom(snapped, centre);
            }
        }
    }

    // ----- paint --------------------------------------------------------

    pub fn paint(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, surface: Rect) {
        let m = self.metrics.clone();
        let theme = ctx.theme;
        let mut layout = Layout::default();
        let inner = Rect {
            x: surface.x + m.padding,
            y: surface.y + m.padding,
            width: (surface.width - m.padding * 2.0).max(0.0),
            height: (surface.height - m.padding * 2.0).max(0.0),
            ..surface
        };

        // Title row: name, mode, then the buttons from the right.
        let row = Rect { height: m.button_height, ..inner };
        let title_style = crate::text::TextStyle {
            family: Arc::from(theme.font.as_str()),
            size: m.title,
            weight: ctx.metrics.typography.weight_bold,
            color: theme.text_main,
        };
        let title_width = ctx.text.measure("Screenshot Viewer", &title_style).0;
        ui::paint_text_vcentered(
            ctx,
            list,
            "Screenshot Viewer",
            row.x,
            row,
            m.title,
            ctx.metrics.typography.weight_bold,
            theme.text_main,
        );
        ui::paint_text_vcentered(
            ctx,
            list,
            &format!("Mode: {}", self.mode.as_str()),
            row.x + title_width + m.gap * 2.0,
            row,
            m.caption,
            400,
            theme.secondary,
        );
        let busy = "󰔛";
        let buttons = [
            (Button::Clear, "󰃢", theme.text_main),
            (Button::Copy, if self.working { busy } else { "󰆏" }, theme.text_main),
            (Button::Save, if self.working { busy } else { "󰆓" }, theme.text_main),
            (Button::SaveAs, if self.working { busy } else { "󰉋" }, theme.text_main),
            (Button::Close, "󰅖", theme.error),
        ];
        let mut x = row.x + row.width - m.button_width;
        let mut placed = Vec::new();
        for (button, glyph, color) in buttons.iter().rev() {
            let rect = Rect {
                x,
                y: row.y,
                width: m.button_width,
                height: m.button_height,
                radius: m.button_radius,
                color: Color::TRANSPARENT,
            };
            placed.push((*button, rect));
            let hovered = !self.working && self.pointer.is_some_and(|p| rect.contains(p));
            list.push_rect(Rect { color: if hovered { theme.bg_hover } else { theme.bg_widget }, ..rect });
            ui::paint_text_centered(ctx, list, glyph, rect, m.icon, *color);
            x -= m.button_width + m.gap;
        }
        layout.buttons = placed;

        // Pen row.
        let row = Rect { y: row.y + row.height + m.spacing, height: m.row_height, ..inner };
        let label = |ctx: &mut PaintCtx<'_>, list: &mut DisplayList, text: &str, x: f32, color: Color| {
            ui::paint_text_vcentered(ctx, list, text, x, row, m.caption, 400, color);
            x + ctx.measure(text, m.caption)
        };
        let mut x = label(ctx, list, "Annotation color", row.x, theme.tertiary) + m.gap;
        for (i, color) in self.colors.iter().enumerate() {
            let rect = Rect {
                x,
                y: row.y + (row.height - m.swatch) * 0.5,
                width: m.swatch,
                height: m.swatch,
                radius: m.swatch * 0.5,
                color: *color,
            };
            let selected = i == self.color;
            let (border, border_color) = if selected {
                (m.swatch_border_selected, theme.border_active)
            } else {
                (m.swatch_border, theme.border_inactive)
            };
            crate::ui::text_field::paint_frame(list, rect, border, border_color, *color);
            layout.swatches.push(rect);
            x += m.swatch + m.gap;
        }

        // From the right: zoom readout, slider, label; then the pen's.
        let readout = |ctx: &mut PaintCtx<'_>, list: &mut DisplayList, text: &str, right: f32| {
            ui::paint_text_vcentered(ctx, list, text, right - m.readout_width, row, m.caption, 400, theme.text_main);
            right - m.readout_width
        };
        let slider = |ctx: &mut PaintCtx<'_>, list: &mut DisplayList, right: f32, value: f32| {
            let rect = Rect { x: right - m.slider_width, width: m.slider_width, ..row };
            let input = crate::ui::InputState { pointer: None, pressed: false };
            crate::ui::controls::slider(ctx, list, rect, value, false, &input);
            rect
        };
        let right = row.x + row.width;
        let right = readout(ctx, list, &format!("{}%", (self.zoom * 100.0).round() as i32), right);
        let zoom_t = (self.zoom - m.zoom_min) / (m.zoom_max - m.zoom_min).max(1e-3);
        layout.zoom = slider(ctx, list, right - m.gap, zoom_t);
        // Labels clear the knob, which overhangs the track's ends.
        let overhang = ctx.metrics.page.slider_knob * 0.5;
        let zoom_label = ctx.measure("Zoom", m.caption);
        let right = layout.zoom.x - overhang - m.gap - zoom_label;
        label(ctx, list, "Zoom", right, theme.text_main);
        let right = readout(ctx, list, &self.pen.to_string(), right - m.gap * 2.0);
        let pen_t = (self.pen.saturating_sub(1)) as f32 / (self.pen_max.saturating_sub(1)).max(1) as f32;
        layout.pen = slider(ctx, list, right - m.gap, pen_t);
        let pen_label = ctx.measure("Pen", m.caption);
        label(ctx, list, "Pen", layout.pen.x - overhang - m.gap - pen_label, theme.text_main);

        // The image and its strokes, clipped to the canvas.
        let top = row.y + row.height + m.spacing;
        let canvas = Rect {
            y: top,
            height: (inner.y + inner.height - top).max(0.0),
            ..inner
        };
        layout.canvas = canvas;
        self.layout = layout;
        let rect = self.image_rect(canvas);
        if let Some(image) = &self.image {
            list.backdrop = Some(Backdrop {
                key: self.image_key,
                width: image.width,
                height: image.height,
                pixels: image.pixels.clone(),
                dest: Some(rect),
                clip: Some(canvas),
            });
            let per_px = rect.width / image.width.max(1) as f32;
            list.push_clip(canvas);
            for stroke in &self.strokes {
                if let Some(path) = stroke_path(stroke, rect, per_px) {
                    list.push_path(path);
                }
            }
            list.pop_clip();
        }

        if let Some((text, error, _)) = &self.status {
            let banner = Rect {
                y: canvas.y + canvas.height - m.status_height,
                height: m.status_height,
                radius: m.status_radius,
                color: if *error { theme.error } else { theme.success },
                ..canvas
            };
            list.push_rect(banner);
            ui::paint_text_centered_weighted(ctx, list, text, banner, m.caption, ctx.metrics.typography.weight_bold, theme.text_main);
        }
    }
}

/// A stroke as a path on screen; a single point is a dot.
fn stroke_path(stroke: &Stroke, image: Rect, per_px: f32) -> Option<Path> {
    let at = |(x, y): (f32, f32)| Point::new(image.x + x * per_px, image.y + y * per_px);
    let first = *stroke.points.first()?;
    let mut commands = vec![PathCommand::MoveTo(at(first))];
    if stroke.points.len() == 1 {
        // A zero-length segment draws as a round cap.
        commands.push(PathCommand::LineTo(at((first.0 + 0.01, first.1))));
    }
    commands.extend(stroke.points[1..].iter().map(|p| PathCommand::LineTo(at(*p))));
    Some(Path::new(commands, Paint::Solid(stroke.color)).stroked((stroke.width * per_px).max(0.5)).rounded())
}

/// Draw `strokes` into RGBA8 `pixels` (`width` × `height`), antialiased,
/// with round caps and joins. Each stroke covers a pixel once however many
/// of its segments overlap there, so a translucent pen would not darken at
/// joins.
pub fn rasterize(pixels: &mut [u8], width: u32, height: u32, strokes: &[Stroke]) {
    let (w, h) = (width as usize, height as usize);
    if pixels.len() < w * h * 4 {
        return;
    }
    for stroke in strokes {
        let radius = (stroke.width * 0.5).max(0.5);
        let points: Vec<(f32, f32)> = if stroke.points.len() == 1 {
            vec![stroke.points[0], stroke.points[0]]
        } else {
            stroke.points.clone()
        };
        if points.is_empty() {
            continue;
        }
        // The stroke's pixel bounds.
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for &(x, y) in &points {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        let bx0 = ((x0 - radius - 1.0).floor().max(0.0)) as usize;
        let by0 = ((y0 - radius - 1.0).floor().max(0.0)) as usize;
        let bx1 = ((x1 + radius + 1.0).ceil() as isize).clamp(0, w as isize) as usize;
        let by1 = ((y1 + radius + 1.0).ceil() as isize).clamp(0, h as isize) as usize;
        if bx1 <= bx0 || by1 <= by0 {
            continue;
        }
        let bw = bx1 - bx0;
        let mut coverage = vec![0f32; bw * (by1 - by0)];
        for pair in points.windows(2) {
            let ((ax, ay), (cx, cy)) = (pair[0], pair[1]);
            let sx0 = ((ax.min(cx) - radius - 1.0).floor() as isize).max(bx0 as isize) as usize;
            let sy0 = ((ay.min(cy) - radius - 1.0).floor() as isize).max(by0 as isize) as usize;
            let sx1 = ((ax.max(cx) + radius + 1.0).ceil() as isize).min(bx1 as isize) as usize;
            let sy1 = ((ay.max(cy) + radius + 1.0).ceil() as isize).min(by1 as isize) as usize;
            let (dx, dy) = (cx - ax, cy - ay);
            let len2 = dx * dx + dy * dy;
            for py in sy0..sy1 {
                for px in sx0..sx1 {
                    // Distance from the pixel centre to the segment.
                    let (qx, qy) = (px as f32 + 0.5, py as f32 + 0.5);
                    let t = if len2 > 0.0 { (((qx - ax) * dx + (qy - ay) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
                    let (ex, ey) = (qx - (ax + t * dx), qy - (ay + t * dy));
                    let d = (ex * ex + ey * ey).sqrt();
                    let c = (radius - d + 0.5).clamp(0.0, 1.0);
                    let cell = &mut coverage[(py - by0) * bw + (px - bx0)];
                    *cell = cell.max(c);
                }
            }
        }
        let color = stroke.color;
        let alpha = color.a.clamp(0.0, 1.0);
        let src = [color.r, color.g, color.b].map(|c| c.clamp(0.0, 1.0) * 255.0);
        for py in by0..by1 {
            for px in bx0..bx1 {
                let a = coverage[(py - by0) * bw + (px - bx0)] * alpha;
                if a <= 0.0 {
                    continue;
                }
                let i = (py * w + px) * 4;
                for (channel, s) in src.iter().enumerate() {
                    let d = pixels[i + channel] as f32;
                    pixels[i + channel] = (s * a + d * (1.0 - a)).round() as u8;
                }
                let d = pixels[i + 3] as f32;
                pixels[i + 3] = (255.0 * a + d * (1.0 - a)).round() as u8;
            }
        }
    }
}

/// Load a capture for the viewer.
pub fn load(path: &str) -> Result<Image, String> {
    let decoded = image::open(path).map_err(|err| format!("Failed to load image: {err}"))?.to_rgba8();
    let (width, height) = decoded.dimensions();
    Ok(Image { width, height, pixels: Arc::from(decoded.into_raw()) })
}

/// Compose `strokes` over `image` into a new temporary PNG.
pub fn export(image: &Image, strokes: &[Stroke]) -> Result<String, String> {
    use image::ImageEncoder;
    let mut pixels = image.pixels.to_vec();
    rasterize(&mut pixels, image.width, image.height, strokes);
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = format!("{dir}/stratum-screenshot-viewer-{}-{stamp}.png", std::process::id());
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|_| "Failed to prepare annotated image".to_string())?;
    let encoded = image::codecs::png::PngEncoder::new(std::io::BufWriter::new(file)).write_image(
        &pixels,
        image.width,
        image.height,
        image::ExtendedColorType::Rgba8,
    );
    if encoded.is_err() {
        let _ = std::fs::remove_file(&path);
        return Err("Failed to render annotated image".into());
    }
    Ok(path)
}

/// Copy `image` to the clipboard, save it to `~/Pictures/Screenshots`, or
/// save it to `target`. Runs on a worker thread.
pub fn post(action: PostAction, image: &str, target: Option<&str>) -> Result<PostAction, String> {
    let image = std::path::Path::new(image);
    match action {
        PostAction::Copy => copy_to_clipboard(image)?,
        PostAction::Save => {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
            let dir = std::path::Path::new(&home).join("Pictures/Screenshots");
            save_copy(image, &unique_path(&dir.join(save_name())))?;
        }
        PostAction::SaveTo => {
            let target = target.map(local_path).filter(|t| !t.is_empty()).ok_or("missing destination path")?;
            let mut path = std::path::PathBuf::from(&target);
            // A directory gets a generated name, which must not overwrite.
            let generated = target.ends_with('/') || path.is_dir();
            if generated {
                path = std::path::Path::new(target.trim_end_matches('/')).join(save_name());
            }
            if path.extension().and_then(|e| e.to_str()) != Some("png") {
                path.set_extension("png");
            }
            if generated {
                path = unique_path(&path);
            }
            save_copy(image, &path)?;
        }
    }
    Ok(action)
}

/// `Screenshot-<local time>.png`.
fn save_name() -> String {
    chrono::Local::now().format("Screenshot-%Y%m%d-%H%M%S.png").to_string()
}

/// `path`, or the first free `<stem>-<n>.png` beside it.
fn unique_path(path: &std::path::Path) -> std::path::PathBuf {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Screenshot");
    let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    std::iter::once(path.to_path_buf())
        .chain((1..10_000).map(|n| parent.join(format!("{stem}-{n}.png"))))
        .find(|candidate| !candidate.exists())
        .unwrap_or_else(|| path.to_path_buf())
}

fn save_copy(image: &std::path::Path, target: &std::path::Path) -> Result<(), String> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    std::fs::copy(image, target).map_err(|err| format!("failed to save screenshot: {err}"))?;
    Ok(())
}

fn copy_to_clipboard(image: &std::path::Path) -> Result<(), String> {
    use std::io::Write;
    let bytes = std::fs::read(image).map_err(|err| format!("failed to read image file: {err}"))?;
    let mut child = std::process::Command::new("wl-copy")
        .args(["--type", "image/png"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|err| format!("failed to run wl-copy: {err}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(&bytes).map_err(|err| format!("wl-copy stdin failed: {err}"))?;
    }
    // wl-copy forks to serve the selection; this waits for the parent only.
    match child.wait() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!("wl-copy failed: {status}")),
        Err(err) => Err(format!("wl-copy failed: {err}")),
    }
}

/// Ask the portal where to save. `Ok(None)` is a cancel.
pub fn portal_save_as(timeout: std::time::Duration) -> Result<Option<String>, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .map_err(|err| format!("Save As failed: {err}"))?;
    let chosen = runtime.block_on(async {
        tokio::time::timeout(timeout, portal_save_file("Save Screenshot As", &save_name())).await
    });
    let uri = chosen
        .map_err(|_| "Save As failed: portal request timed out".to_string())?
        .map_err(|err| format!("Save As failed: {err}"))?;
    match uri.map(|uri| local_path(&uri)) {
        Some(path) if path.is_empty() => Err("Save As failed: invalid destination".into()),
        chosen => Ok(chosen),
    }
}

/// The desktop portal's Save File dialog: the chosen URI, or `None` when it
/// was cancelled.
async fn portal_save_file(title: &str, name: &str) -> Result<Option<String>, String> {
    use futures_lite::StreamExt;
    use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
    const PORTAL: &str = "org.freedesktop.portal.Desktop";
    let error = |err: zbus::Error| err.to_string();

    let conn = zbus::Connection::session().await.map_err(error)?;
    // Subscribe before asking, on the path the request will have, or a quick
    // answer can arrive before anyone listens for it.
    let token = format!(
        "stratum{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_micros())
            .unwrap_or(0)
    );
    let sender = conn
        .unique_name()
        .map(|name| name.trim_start_matches(':').replace('.', "_"))
        .unwrap_or_default();
    let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
    let request = zbus::Proxy::new(&conn, PORTAL, path, "org.freedesktop.portal.Request")
        .await
        .map_err(error)?;
    let mut responses = request.receive_signal("Response").await.map_err(error)?;

    let options = std::collections::HashMap::from([
        ("handle_token", Value::from(token.as_str())),
        ("modal", Value::from(true)),
        ("current_name", Value::from(name)),
    ]);
    let chooser =
        zbus::Proxy::new(&conn, PORTAL, "/org/freedesktop/portal/desktop", "org.freedesktop.portal.FileChooser")
            .await
            .map_err(error)?;
    let _: OwnedObjectPath = chooser.call("SaveFile", &("", title, options)).await.map_err(error)?;

    let message = responses.next().await.ok_or("the portal dropped the request")?;
    let (code, results): (u32, std::collections::HashMap<String, OwnedValue>) =
        message.body().deserialize().map_err(error)?;
    match code {
        0 => {
            let uris = results
                .get("uris")
                .and_then(|uris| uris.try_clone().ok())
                .and_then(|uris| Vec::<String>::try_from(uris).ok())
                .unwrap_or_default();
            uris.into_iter().next().map(Some).ok_or_else(|| "the portal returned no file".into())
        }
        // Cancelled, or ended some other way without a choice.
        1 | 2 => Ok(None),
        code => Err(format!("unexpected portal response {code}")),
    }
}

/// A `file://` URI as a path, percent-decoded.
pub fn local_path(uri: &str) -> String {
    let value = uri.trim();
    let Some(rest) = value.strip_prefix("file://") else {
        return value.to_string();
    };
    let rest = format!("/{}", rest.trim_start_matches('/'));
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn viewer(width: u32, height: u32) -> ScreenshotViewer {
        let config = crate::config::Config::test_defaults();
        let mut v = ScreenshotViewer::new(&config.metrics.screenshot_viewer, &config.screenshot);
        v.show(
            String::new(),
            Mode::Region,
            false,
            Image { width, height, pixels: Arc::from(vec![0u8; (width * height * 4) as usize]) },
        );
        v.layout.canvas = Rect { x: 0.0, y: 100.0, width: 1000.0, height: 500.0, radius: 0.0, color: Color::TRANSPARENT };
        v
    }

    #[test]
    fn the_image_fits_and_centres() {
        let v = viewer(2000, 500);
        let r = v.image_rect(v.layout.canvas);
        assert_eq!((r.x, r.y, r.width, r.height), (0.0, 225.0, 1000.0, 250.0));
    }

    #[test]
    fn zooming_keeps_the_focus_point_still_and_clamps_the_pan() {
        let mut v = viewer(1000, 500);
        let focus = Point::new(750.0, 350.0);
        let before = v.to_image(focus);
        v.set_zoom(2.0, focus);
        let after = v.to_image(focus);
        assert!((before.0 - after.0).abs() < 0.01 && (before.1 - after.1).abs() < 0.01);
        // Zooming out to 1× recentres: there is nothing to pan.
        v.set_zoom(0.5, focus);
        assert_eq!((v.zoom, v.pan), (1.0, (0.0, 0.0)));
    }

    #[test]
    fn strokes_are_kept_in_image_pixels() {
        let mut v = viewer(2000, 1000);
        // 2000 px in 1000 logical: half a logical pixel per image pixel.
        v.pen = 4;
        v.press(Point::new(100.0, 150.0), false, false);
        v.motion(Point::new(200.0, 150.0));
        v.release();
        let stroke = &v.strokes()[0];
        assert_eq!(stroke.points, vec![(200.0, 100.0), (400.0, 100.0)]);
        assert_eq!(stroke.width, 8.0);
        assert!(v.key(smithay_client_toolkit::seat::keyboard::Keysym::z, true) == ViewerAction::Redraw);
        assert!(v.strokes().is_empty(), "Ctrl+Z undoes it");
    }

    #[test]
    fn a_ctrl_press_pans_instead_of_drawing() {
        let mut v = viewer(1000, 500);
        v.set_zoom(4.0, Point::new(500.0, 350.0));
        v.press(Point::new(500.0, 350.0), false, true);
        v.motion(Point::new(400.0, 350.0));
        v.release();
        assert!(v.strokes().is_empty());
        assert_eq!(v.pan.0, -100.0);
    }

    #[test]
    fn rasterized_strokes_cover_their_width_once() {
        let mut pixels = vec![0u8; 20 * 10 * 4];
        let red = Color::rgba(1.0, 0.0, 0.0, 1.0);
        let stroke = Stroke { color: red, width: 4.0, points: vec![(2.0, 5.0), (10.0, 5.0), (18.0, 5.0)] };
        rasterize(&mut pixels, 20, 10, &[stroke]);
        let at = |x: usize, y: usize| &pixels[(y * 20 + x) * 4..(y * 20 + x) * 4 + 4];
        assert_eq!(at(10, 5), &[255, 0, 0, 255], "on the line");
        assert_eq!(at(10, 0), &[0, 0, 0, 0], "outside its width");
        assert_eq!(at(10, 4)[0], 255, "within half the width");
    }

    #[test]
    fn save_to_names_directories_and_adds_the_extension() {
        let dir = std::env::temp_dir().join(format!("stratum-viewer-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("capture.png");
        std::fs::write(&source, b"png").unwrap();
        let source = source.to_str().unwrap();

        let named = dir.join("named");
        post(PostAction::SaveTo, source, Some(named.to_str().unwrap())).unwrap();
        assert_eq!(std::fs::read(dir.join("named.png")).unwrap(), b"png");

        // A directory gets a generated name, and a second save never
        // overwrites the first.
        let into = format!("file://{}/", dir.join("shots").display());
        std::fs::create_dir_all(dir.join("shots")).unwrap();
        post(PostAction::SaveTo, source, Some(&into)).unwrap();
        post(PostAction::SaveTo, source, Some(&into)).unwrap();
        assert_eq!(std::fs::read_dir(dir.join("shots")).unwrap().count(), 2);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn file_uris_become_paths() {
        assert_eq!(local_path("file:///home/a/My%20Shots/x.png"), "/home/a/My Shots/x.png");
        assert_eq!(local_path("/tmp/x.png"), "/tmp/x.png");
    }
}
