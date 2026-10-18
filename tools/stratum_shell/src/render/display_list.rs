//! A minimal, backend-agnostic draw list.
//!
//! M1 only needs solid and rounded rectangles. The vocabulary grows as widgets
//! land; keeping it declarative means the renderer never sees layout or state.

/// Straight RGBA, 0..=1, interpreted as sRGB.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const TRANSPARENT: Self = Self::rgba(0.0, 0.0, 0.0, 0.0);

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
}

/// A filled rounded rectangle in surface-local logical pixels.
#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub radius: f32,
    pub color: Color,
}

#[derive(Clone, Debug)]
pub struct DisplayList {
    pub clear: Color,
    pub rects: Vec<Rect>,
}

impl Default for DisplayList {
    fn default() -> Self {
        Self {
            clear: Color::TRANSPARENT,
            rects: Vec::new(),
        }
    }
}

impl DisplayList {
    pub fn new(clear: Color) -> Self {
        Self {
            clear,
            rects: Vec::new(),
        }
    }

    pub fn push_rect(&mut self, rect: Rect) {
        self.rects.push(rect);
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
        assert!(DisplayList::default().rects.is_empty());
    }

    #[test]
    fn rectangles_are_appended_in_order() {
        let mut list = DisplayList::new(Color::TRANSPARENT);
        let rect = Rect {
            x: 1.0,
            y: 2.0,
            width: 3.0,
            height: 4.0,
            radius: 1.0,
            color: Color::rgba(1.0, 0.0, 0.0, 1.0),
        };
        list.push_rect(rect);
        assert_eq!(list.rects.len(), 1);
        assert_eq!(list.rects[0].width, 3.0);
    }
}
