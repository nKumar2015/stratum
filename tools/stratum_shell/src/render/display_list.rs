//! A backend-agnostic draw list.
//!
//! The vocabulary is deliberately small: filled/stroked paths, rounded rects,
//! gradients, text runs, and images. Layout and widgets emit these; the
//! renderer turns them into GPU work. Nothing here knows about Wayland or wgpu.

#![allow(dead_code)] // Forward-looking API lands in M3 (widgets).

use std::sync::Arc;

/// Straight RGBA, 0..=1, interpreted as sRGB.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rect {
    pub const EMPTY: Rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 0.0,
        height: 0.0,
        radius: 0.0,
        color: Color::TRANSPARENT,
    };

    /// True when `point` lies inside this rectangle.
    pub fn contains(self, point: Point) -> bool {
        point.x >= self.x
            && point.x <= self.x + self.width
            && point.y >= self.y
            && point.y <= self.y + self.height
    }
}

impl Color {
    pub const TRANSPARENT: Self = Self::rgba(0.0, 0.0, 0.0, 0.0);
    pub const WHITE: Self = Self::rgba(1.0, 1.0, 1.0, 1.0);
    pub const BLACK: Self = Self::rgba(0.0, 0.0, 0.0, 1.0);

    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub fn from_hex(hex: u32, alpha: f32) -> Self {
        Self {
            r: ((hex >> 16) & 0xff) as f32 / 255.0,
            g: ((hex >> 8) & 0xff) as f32 / 255.0,
            b: (hex & 0xff) as f32 / 255.0,
            a: alpha,
        }
    }

    pub fn to_array(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }

    /// Multiply alpha, used for fades.
    pub fn with_alpha(self, a: f32) -> Self {
        Self { a: a.clamp(0.0, 1.0), ..self }
    }
}

/// A point in surface-local logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// A 2D affine transform.
///
/// Applied when vertices are built, so transformed glyphs, paths, and rects all
/// cost the same as untransformed ones — this is the "transform-only" tier of
/// the motion plan: no re-rasterization or re-tessellation.
#[derive(Clone, Copy, Debug)]
pub struct Transform {
    /// `[m11, m12, m21, m22, tx, ty]`, column-major like CSS `matrix(a,b,c,d,e,f)`.
    pub m: [f32; 6],
}

impl Transform {
    pub const IDENTITY: Self = Self {
        m: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
    };

    pub const fn translate(x: f32, y: f32) -> Self {
        Self {
            m: [1.0, 0.0, 0.0, 1.0, x, y],
        }
    }

    pub const fn scale(sx: f32, sy: f32) -> Self {
        Self {
            m: [sx, 0.0, 0.0, sy, 0.0, 0.0],
        }
    }

    /// Rotate clockwise by `radians` about the origin.
    pub fn rotate(radians: f32) -> Self {
        let (s, c) = radians.sin_cos();
        Self {
            m: [c, s, -s, c, 0.0, 0.0],
        }
    }

    /// Rotate about `(px, py)`, matching the QML `rotation` + centered pivot
    /// idiom used for the vertical app-title label.
    pub fn rotate_about(radians: f32, px: f32, py: f32) -> Self {
        Self::translate(px, py).then(Self::rotate(radians)).then(Self::translate(-px, -py))
    }

    /// `self` applied first, then `other`.
    pub fn then(self, other: Self) -> Self {
        let [a1, b1, c1, d1, e1, f1] = self.m;
        let [a2, b2, c2, d2, e2, f2] = other.m;
        Self {
            m: [
                a2 * a1 + c2 * b1,
                b2 * a1 + d2 * b1,
                a2 * c1 + c2 * d1,
                b2 * c1 + d2 * d1,
                a2 * e1 + c2 * f1 + e2,
                b2 * e1 + d2 * f1 + f2,
            ],
        }
    }

    pub fn apply(self, x: f32, y: f32) -> (f32, f32) {
        let [a, b, c, d, e, f] = self.m;
        (a * x + c * y + e, b * x + d * y + f)
    }

    /// True when this transform only translates (the common case, worth a fast
    /// path).
    pub fn is_identity(self) -> bool {
        self.m == Self::IDENTITY.m
    }
}

/// A filled rounded rectangle. The fast path: SDF, no tessellation.
#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub radius: f32,
    pub color: Color,
}

/// How a path is filled.
#[derive(Clone, Debug)]
pub enum Paint {
    Solid(Color),
    LinearGradient {
        from: Point,
        to: Point,
        stops: Vec<(f32, Color)>,
    },
    RadialGradient {
        center: Point,
        radius: f32,
        stops: Vec<(f32, Color)>,
    },
}

impl Paint {
    pub fn solid(color: Color) -> Self {
        Paint::Solid(color)
    }
}

/// A path segment, in draw order. Curves are cubic/quintic-free for now:
/// lines, quadratics, and cubics cover authored artwork and UI shapes.
#[derive(Clone, Debug, PartialEq)]
pub enum PathCommand {
    MoveTo(Point),
    LineTo(Point),
    QuadTo(Point, Point),
    CubicTo(Point, Point, Point),
    Close,
}

/// An arbitrary filled or stroked path.
///
/// Paths are tessellated once per distinct geometry and cached by the renderer;
/// animated shapes should use SDF primitives (see the motion plan) rather than
/// rebuilding paths every frame.
#[derive(Clone, Debug)]
pub struct Path {
    pub commands: Vec<PathCommand>,
    pub paint: Paint,
    /// `None` means fill; otherwise the stroke width in logical pixels.
    pub stroke: Option<f32>,
    /// Applied around the path origin when vertices are built.
    pub transform: Transform,
}

impl Path {
    pub fn new(commands: Vec<PathCommand>, paint: Paint) -> Self {
        Self {
            commands,
            paint,
            stroke: None,
            transform: Transform::IDENTITY,
        }
    }

    pub fn stroked(mut self, width: f32) -> Self {
        self.stroke = Some(width);
        self
    }

    pub fn with_transform(mut self, transform: Transform) -> Self {
        self.transform = transform;
        self
    }

    /// Axis-aligned bounds, used for damage tracking and culling.
    pub fn bounds(&self) -> (f32, f32, f32, f32) {
        let (mut min_x, mut min_y) = (f32::MAX, f32::MAX);
        let (mut max_x, mut max_y) = (f32::MIN, f32::MIN);
        for cmd in &self.commands {
            let points: &[Point] = match cmd {
                PathCommand::MoveTo(p) | PathCommand::LineTo(p) => std::slice::from_ref(p),
                PathCommand::QuadTo(a, b) => &[*a, *b],
                PathCommand::CubicTo(a, b, c) => &[*a, *b, *c],
                PathCommand::Close => &[],
            };
            for p in points {
                min_x = min_x.min(p.x);
                min_y = min_y.min(p.y);
                max_x = max_x.max(p.x);
                max_y = max_y.max(p.y);
            }
        }
        let pad = self.stroke.unwrap_or(0.0) * 0.5;
        if min_x > max_x {
            return (0.0, 0.0, 0.0, 0.0);
        }
        (min_x - pad, min_y - pad, max_x + pad, max_y + pad)
    }
}

/// A shaped text run ready for the renderer.
///
/// `glyphs` are positioned by the shaping pass; the renderer only rasterizes
/// and composites them, so a text field can re-shape without touching the GPU
/// unless its glyph set changes.
#[derive(Clone, Debug)]
pub struct TextRun {
    pub x: f32,
    pub y: f32,
    /// Font size in logical pixels.
    pub size: f32,
    pub color: Color,
    pub family: Arc<str>,
    pub weight: u16,
    pub glyphs: Vec<PositionedGlyph>,
    /// Applied around the run origin when vertices are built.
    pub transform: Transform,
    /// Scissor in effect when the run was pushed, if any.
    pub clip: Option<Rect>,
}

#[derive(Clone, Copy, Debug)]
pub struct PositionedGlyph {
    pub glyph_id: u16,
    /// Font chosen during shaping (fallback may pick a different face per
    /// glyph, so this cannot be inferred from the run).
    pub font_id: fontdb::ID,
    /// Glyph hitbox origin in line space, as reported by shaping.
    pub x: f32,
    pub y: f32,
    /// Fractional offsets (in em) cosmic-text applies on top of `x`/`y`.
    pub x_offset: f32,
    pub y_offset: f32,
}

/// A decoded image (icons, notification art, screenshots).
#[derive(Clone, Debug)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    /// Premultiplied RGBA8.
    pub pixels: Arc<[u8]>,
    pub x: f32,
    pub y: f32,
    pub width_logical: f32,
    pub height_logical: f32,
}

#[derive(Clone, Debug)]
pub enum DrawCommand {
    Rect(Rect),
    Path(Path),
    Image(Image),
    /// Restrict everything drawn after this to `rect`.
    PushClip(Rect),
    /// Restore the clip in effect before the matching [`DrawCommand::PushClip`].
    PopClip,
}

/// The frame's contents.
///
/// Text is kept separately because it needs the font/glyph caches, which live
/// with the renderer rather than in this data structure. A text run records the
/// clip in effect when it was pushed, so text obeys the same scissor as the
/// geometry around it.
#[derive(Clone, Debug)]
pub struct DisplayList {
    pub clear: Color,
    pub commands: Vec<DrawCommand>,
    pub texts: Vec<TextRun>,
    clip_stack: Vec<Rect>,
}

impl Default for DisplayList {
    fn default() -> Self {
        Self {
            clear: Color::TRANSPARENT,
            commands: Vec::new(),
            texts: Vec::new(),
            clip_stack: Vec::new(),
        }
    }
}

impl DisplayList {
    pub fn new(clear: Color) -> Self {
        Self {
            clear,
            commands: Vec::new(),
            texts: Vec::new(),
            clip_stack: Vec::new(),
        }
    }

    pub fn push_rect(&mut self, rect: Rect) {
        self.commands.push(DrawCommand::Rect(rect));
    }

    pub fn push_path(&mut self, path: Path) {
        self.commands.push(DrawCommand::Path(path));
    }

    pub fn push_image(&mut self, image: Image) {
        self.commands.push(DrawCommand::Image(image));
    }

    pub fn push_text(&mut self, mut run: TextRun) {
        // Nested clips intersect, so a child can never draw outside its parent.
        run.clip = self.clip();
        self.texts.push(run);
    }

    /// Restrict subsequent drawing to `rect` until [`Self::pop_clip`].
    pub fn push_clip(&mut self, rect: Rect) {
        let rect = match self.clip_stack.last() {
            Some(parent) => intersect(*parent, rect),
            None => rect,
        };
        self.clip_stack.push(rect);
        self.commands.push(DrawCommand::PushClip(rect));
    }

    /// End the most recent clip.
    pub fn pop_clip(&mut self) {
        if self.clip_stack.pop().is_some() {
            self.commands.push(DrawCommand::PopClip);
        }
    }

    /// The clip currently in effect, if any.
    pub fn clip(&self) -> Option<Rect> {
        self.clip_stack.last().copied()
    }

    pub fn is_empty(&self) -> bool {
        self.commands.is_empty() && self.texts.is_empty()
    }
}

/// Intersection of two rects; an empty rect when they do not overlap.
fn intersect(a: Rect, b: Rect) -> Rect {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let right = (a.x + a.width).min(b.x + b.width);
    let bottom = (a.y + a.height).min(b.y + b.height);
    Rect {
        x,
        y,
        width: (right - x).max(0.0),
        height: (bottom - y).max(0.0),
        radius: 0.0,
        color: Color::TRANSPARENT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_maps_to_srgb_channels() {
        let c = Color::from_hex(0x4589ff, 1.0);
        assert!((c.r - 69.0 / 255.0).abs() < 1e-6);
        assert!((c.g - 137.0 / 255.0).abs() < 1e-6);
        assert!((c.b - 1.0).abs() < 1e-6);
        assert_eq!(c.a, 1.0);
    }

    #[test]
    fn alpha_is_preserved_and_default_is_transparent() {
        assert_eq!(Color::from_hex(0x000000, 0.5).a, 0.5);
        assert_eq!(DisplayList::default().clear, Color::TRANSPARENT);
        assert!(DisplayList::default().is_empty());
    }

    #[test]
    fn path_bounds_include_stroke() {
        let path = Path::new(
            vec![
                PathCommand::MoveTo(Point::new(10.0, 20.0)),
                PathCommand::LineTo(Point::new(30.0, 40.0)),
            ],
            Paint::solid(Color::WHITE),
        )
        .stroked(4.0);
        let (x, y, w, h) = path.bounds();
        assert_eq!((x, y), (8.0, 18.0));
        assert_eq!((w, h), (32.0, 42.0));
    }

    #[test]
    fn empty_path_has_zero_bounds() {
        let path = Path::new(vec![PathCommand::Close], Paint::solid(Color::WHITE));
        assert_eq!(path.bounds(), (0.0, 0.0, 0.0, 0.0));
    }

    #[test]
    fn draw_commands_preserve_order() {
        let mut list = DisplayList::new(Color::TRANSPARENT);
        list.push_rect(Rect {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
            radius: 0.0,
            color: Color::WHITE,
        });
        list.push_path(Path::new(
            vec![PathCommand::MoveTo(Point::new(0.0, 0.0)), PathCommand::Close],
            Paint::solid(Color::BLACK),
        ));
        assert_eq!(list.commands.len(), 2);
        assert!(matches!(list.commands[0], DrawCommand::Rect(_)));
        assert!(matches!(list.commands[1], DrawCommand::Path(_)));
    }
}

#[cfg(test)]
mod transform_tests {
    use super::*;

    #[test]
    fn rotate_minus_90_turns_x_into_up() {
        let t = Transform::rotate(-std::f32::consts::FRAC_PI_2);
        // A point 10 to the right should map to 10 up (y negative).
        let (x, y) = t.apply(10.0, 0.0);
        assert!(x.abs() < 1e-4, "x was {x}");
        assert!((y + 10.0).abs() < 1e-4, "y was {y}");
    }

    #[test]
    fn translate_then_rotate_order() {
        // rotate then translate: glyphs advance upward from the pivot.
        let t = Transform::rotate(-std::f32::consts::FRAC_PI_2)
            .then(Transform::translate(20.0, 900.0));
        let (x, y) = t.apply(10.0, 0.0);
        assert!((x - 20.0).abs() < 1e-4, "x was {x}");
        assert!((y - 890.0).abs() < 1e-4, "y was {y}");
    }

    #[test]
    fn scale_is_applied_after_transform() {
        let t = Transform::rotate(-std::f32::consts::FRAC_PI_2);
        let (x, y) = t.apply(10.0, 0.0);
        assert!((x * 2.0).abs() < 1e-3);
        assert!((y * 2.0 + 20.0).abs() < 1e-3);
    }
}
