//! Where the theme switcher's slices go. Pure, so it is tested here.
//!
//! The slices are parallelograms in a horizontal band, each leaning right
//! at the top. The focused one is wide and the rest narrow; widths follow
//! a fractional focus, so while the band slides between two themes one
//! slice closes as the other opens, and the band stays centred on the
//! point between them.

use crate::render::display_list::{Color, Point, Rect};
use crate::ui::style::ThemeSwitcherMetrics;

/// A parallelogram: top left, top right, bottom right, bottom left.
pub type Quad = [Point; 4];

/// One slice as laid out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slice {
    /// Index in the filtered themes.
    pub index: usize,
    pub quad: Quad,
    /// 1 for the focused slice, falling to 0 a slice away.
    pub openness: f32,
}

/// The band within `area`, and the slices in it for `count` themes with
/// `focus` (fractional) centred. Slices wholly outside `area` are left out.
pub fn slices(count: usize, focus: f32, area: Rect, m: &ThemeSwitcherMetrics) -> (Rect, Vec<Slice>) {
    let height = area.height * m.band_height;
    // A little above the middle, leaving room for the label beneath.
    let top = area.y + (area.height - height) * 0.42;
    let band = Rect { x: area.x, y: top, width: area.width, height, radius: 0.0, color: Color::TRANSPARENT };
    if count == 0 {
        return (band, Vec::new());
    }
    let skew = m.skew * height;
    let wide = (m.focus_width * height).min(area.width * 0.6).max(m.slice_width);
    let openness = |i: usize| (1.0 - (i as f32 - focus).abs()).max(0.0);
    let widths: Vec<f32> = (0..count).map(|i| m.slice_width + (wide - m.slice_width) * openness(i)).collect();
    // Left edges along the band's middle line, before centring.
    let mut lefts = Vec::with_capacity(count);
    let mut x = 0.0;
    for w in &widths {
        lefts.push(x);
        x += w + m.slice_gap;
    }
    let centre = |i: usize| lefts[i] + widths[i] * 0.5;
    let f = focus.clamp(0.0, (count - 1) as f32);
    let (lo, hi) = (f.floor() as usize, f.ceil() as usize);
    let focus_centre = centre(lo) + (centre(hi) - centre(lo)) * (f - lo as f32);
    let shift = area.x + area.width * 0.5 - focus_centre;

    let bottom = top + height;
    let slices = (0..count)
        .filter_map(|i| {
            let (l, r) = (lefts[i] + shift, lefts[i] + widths[i] + shift);
            if r + skew * 0.5 < area.x || l - skew * 0.5 > area.x + area.width {
                return None;
            }
            let quad = [
                Point::new(l + skew * 0.5, top),
                Point::new(r + skew * 0.5, top),
                Point::new(r - skew * 0.5, bottom),
                Point::new(l - skew * 0.5, bottom),
            ];
            Some(Slice { index: i, quad, openness: openness(i) })
        })
        .collect();
    (band, slices)
}

/// Whether `p` is inside the convex `quad`.
pub fn contains(quad: &Quad, p: Point) -> bool {
    let mut sign = 0.0f32;
    for i in 0..4 {
        let (a, b) = (quad[i], quad[(i + 1) % 4]);
        let cross = (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
        if cross != 0.0 {
            if sign != 0.0 && cross.signum() != sign {
                return false;
            }
            sign = cross.signum();
        }
    }
    true
}

/// The part of `quad` between fractions `u0..u1` of its width (along its
/// slant) and `v0..v1` of its height.
pub fn sub_quad(quad: &Quad, (u0, u1): (f32, f32), (v0, v1): (f32, f32)) -> Quad {
    let [tl, tr, _, bl] = *quad;
    let at = |u: f32, v: f32| {
        Point::new(
            tl.x + (tr.x - tl.x) * u + (bl.x - tl.x) * v,
            tl.y + (tr.y - tl.y) * u + (bl.y - tl.y) * v,
        )
    };
    [at(u0, v0), at(u1, v0), at(u1, v1), at(u0, v1)]
}

/// Texture coordinates that show an image `size` upright in `quad`,
/// scaled to cover the quad's bounding box and centred in it: the slant
/// cuts the picture, it does not shear it.
pub fn cover_uv(quad: &Quad, size: (u32, u32)) -> [(f32, f32); 4] {
    let min_x = quad.iter().map(|p| p.x).fold(f32::INFINITY, f32::min);
    let max_x = quad.iter().map(|p| p.x).fold(f32::NEG_INFINITY, f32::max);
    let min_y = quad.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
    let max_y = quad.iter().map(|p| p.y).fold(f32::NEG_INFINITY, f32::max);
    let (bw, bh) = ((max_x - min_x).max(1.0), (max_y - min_y).max(1.0));
    let (iw, ih) = (size.0.max(1) as f32, size.1.max(1) as f32);
    let scale = (bw / iw).max(bh / ih);
    let (dw, dh) = (iw * scale, ih * scale);
    let (ox, oy) = (min_x + (bw - dw) * 0.5, min_y + (bh - dh) * 0.5);
    quad.map(|p| ((p.x - ox) / dw, (p.y - oy) / dh))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics() -> ThemeSwitcherMetrics {
        crate::config::Config::test_defaults().metrics.theme_switcher
    }

    fn area() -> Rect {
        Rect { x: 0.0, y: 0.0, width: 1920.0, height: 1080.0, radius: 0.0, color: Color::TRANSPARENT }
    }

    fn width(s: &Slice) -> f32 {
        s.quad[1].x - s.quad[0].x
    }

    fn mid_x(s: &Slice) -> f32 {
        (s.quad[0].x + s.quad[2].x) * 0.5
    }

    #[test]
    fn the_focused_slice_is_widest_and_centred() {
        let (_, slices) = slices(5, 2.0, area(), &metrics());
        let focused = slices.iter().find(|s| s.index == 2).unwrap();
        assert!(slices.iter().all(|s| s.index == 2 || width(s) < width(focused)));
        assert!((mid_x(focused) - 960.0).abs() < 0.01);
        assert_eq!(focused.openness, 1.0);
    }

    #[test]
    fn slices_never_overlap() {
        for focus in [0.0, 1.3, 2.5, 4.0] {
            let (_, slices) = slices(5, focus, area(), &metrics());
            for pair in slices.windows(2) {
                assert!(pair[0].quad[1].x <= pair[1].quad[0].x + 0.01, "{focus}");
            }
        }
    }

    #[test]
    fn sliding_is_continuous() {
        let m = metrics();
        let at = |f: f32| slices(6, f, area(), &m).1;
        // Just either side of a whole focus, everything is where it was.
        for focus in [1.0f32, 2.0, 3.0] {
            let (a, b) = (at(focus - 0.001), at(focus + 0.001));
            for (sa, sb) in a.iter().zip(&b) {
                assert_eq!(sa.index, sb.index);
                for (pa, pb) in sa.quad.iter().zip(&sb.quad) {
                    assert!((pa.x - pb.x).abs() < 1.0, "{focus}: {pa:?} {pb:?}");
                }
            }
        }
        // Halfway, the point between the two is centred.
        let half = at(1.5);
        let (one, two) = (half.iter().find(|s| s.index == 1).unwrap(), half.iter().find(|s| s.index == 2).unwrap());
        assert!(((mid_x(one) + mid_x(two)) * 0.5 - 960.0).abs() < 1.0);
        assert!((width(one) - width(two)).abs() < 0.01);
    }

    #[test]
    fn slices_far_off_screen_are_left_out() {
        let (_, slices) = slices(200, 100.0, area(), &metrics());
        assert!(slices.len() < 100);
        assert!(slices.iter().any(|s| s.index == 100));
    }

    #[test]
    fn hit_testing_follows_the_slant() {
        let quad = [Point::new(20.0, 0.0), Point::new(60.0, 0.0), Point::new(40.0, 100.0), Point::new(0.0, 100.0)];
        assert!(contains(&quad, Point::new(30.0, 50.0)));
        // Inside the bounding box but outside the slant.
        assert!(!contains(&quad, Point::new(5.0, 5.0)));
        assert!(!contains(&quad, Point::new(55.0, 95.0)));
    }

    #[test]
    fn sub_quads_follow_the_slant() {
        let quad = [Point::new(20.0, 0.0), Point::new(60.0, 0.0), Point::new(40.0, 100.0), Point::new(0.0, 100.0)];
        assert_eq!(sub_quad(&quad, (0.0, 1.0), (0.0, 1.0)), quad);
        let right_half = sub_quad(&quad, (0.5, 1.0), (0.0, 1.0));
        assert_eq!(right_half[0], Point::new(40.0, 0.0));
        assert_eq!(right_half[3], Point::new(20.0, 100.0));
    }

    #[test]
    fn an_image_covers_the_quad_upright() {
        let quad = [Point::new(20.0, 0.0), Point::new(60.0, 0.0), Point::new(40.0, 100.0), Point::new(0.0, 100.0)];
        // A wide image fills the height and is cropped at the sides.
        let uv = cover_uv(&quad, (200, 100));
        assert!((uv[0].1).abs() < 1e-6 && (uv[2].1 - 1.0).abs() < 1e-6);
        for (u, _) in uv {
            assert!((0.0..=1.0).contains(&u));
        }
        // Upright: the top edge's left corner is right of the bottom's.
        assert!(uv[0].0 > uv[3].0);
    }
}
