//! A tachometer dial: an arc split into zones, ticks, and a needle the
//! pointer can drag. The battery menu chooses the power mode with it, one
//! zone per mode, from economy on the left to the redline on the right.
//!
//! Angles are in degrees, counter-clockwise from the +x axis with y up (as
//! in maths, not screen space). The dial sweeps clockwise over the top from
//! [`START`] (lower left) through 90° (straight up) to `START - SWEEP`
//! (lower right), leaving the bottom open like a car's tachometer.
//!
//! The geometry is pure and tested; [`paint`] only draws it.

use crate::render::display_list::{Color, DisplayList, Paint, Path, PathCommand, Point, Rect};

/// Where the sweep starts (lower left).
pub const START: f32 = 210.0;
/// How far it goes, clockwise.
pub const SWEEP: f32 = 240.0;
/// Degrees between ticks.
const TICK_STEP: f32 = 10.0;
/// Degrees left empty between zones on the arc.
const ZONE_GAP: f32 = 2.0;

/// The angle `t` (0 to 1) of the way along the sweep.
pub fn angle_at(t: f32) -> f32 {
    START - SWEEP * t.clamp(0.0, 1.0)
}

/// How far along the sweep (0 to 1) `angle` is.
pub fn fraction(angle: f32) -> f32 {
    ((START - angle) / SWEEP).clamp(0.0, 1.0)
}

/// The point at `angle` on a circle of `radius` about `centre`, in screen
/// space (y down).
pub fn point(centre: Point, radius: f32, angle: f32) -> Point {
    let a = angle.to_radians();
    Point::new(centre.x + radius * a.cos(), centre.y - radius * a.sin())
}

/// The dial angle the pointer at `at` points to, seen from `centre`. Below
/// the centre, in the gap the dial leaves open, it is held at the nearer
/// end.
pub fn pointer_angle(centre: Point, at: Point) -> f32 {
    let mut angle = (centre.y - at.y).atan2(at.x - centre.x).to_degrees();
    // (-180, 180] to [-90, 270): the sweep, 210° down to -30°, is then one
    // unbroken range, and the gap is (210, 270) and [-90, -30).
    if angle < -90.0 {
        angle += 360.0;
    }
    angle.clamp(START - SWEEP, START)
}

/// Which of `zones` equal zones `angle` is in.
pub fn zone_at(angle: f32, zones: usize) -> usize {
    let zones = zones.max(1);
    ((fraction(angle) * zones as f32) as usize).min(zones - 1)
}

/// The angle in the middle of zone `index` of `zones`.
pub fn zone_centre(index: usize, zones: usize) -> f32 {
    let zones = zones.max(1) as f32;
    angle_at((index as f32 + 0.5) / zones)
}

/// An arc from `from` to `to` degrees as a polyline, a point every few
/// degrees (the renderer has no arcs).
pub fn arc(centre: Point, radius: f32, from: f32, to: f32) -> Vec<PathCommand> {
    let steps = ((from - to).abs() / 3.0).ceil().max(1.0) as usize;
    (0..=steps)
        .map(|i| {
            let p = point(centre, radius, from + (to - from) * i as f32 / steps as f32);
            if i == 0 {
                PathCommand::MoveTo(p)
            } else {
                PathCommand::LineTo(p)
            }
        })
        .collect()
}

/// Sizes and colours for [`paint`].
pub struct Style {
    pub radius: f32,
    /// The arc's width.
    pub band: f32,
    pub tick: f32,
    pub needle: f32,
    pub hub: f32,
    /// One per zone, left to right.
    pub zones: Vec<Color>,
    /// The zone the needle points at, drawn at full strength.
    pub lit: usize,
    /// Behind the other zones, which are mixed toward it.
    pub track: Color,
    pub ticks: Color,
    pub hub_fill: Color,
}

/// Draw the arc, ticks and a needle at `needle` degrees.
pub fn paint(list: &mut DisplayList, centre: Point, needle: f32, style: &Style) {
    let n = style.zones.len().max(1);
    let per = SWEEP / n as f32;
    for (i, color) in style.zones.iter().enumerate() {
        let from = START - per * i as f32 - if i == 0 { 0.0 } else { ZONE_GAP * 0.5 };
        let to = START - per * (i + 1) as f32 + if i + 1 == n { 0.0 } else { ZONE_GAP * 0.5 };
        let color = if i == style.lit { *color } else { crate::ui::pages::layout::mix(style.track, *color, 0.35) };
        list.push_path(Path::new(arc(centre, style.radius, from, to), Paint::Solid(color)).stroked(style.band));
    }

    // Ticks inside the arc: longer where zones meet and at the ends.
    let inner = style.radius - style.band * 0.5 - style.tick * 0.5;
    let count = (SWEEP / TICK_STEP).round() as usize;
    for i in 0..=count {
        let angle = START - TICK_STEP * i as f32;
        let major = ((START - angle) / per).fract().abs() < 1e-3 || i == count;
        let length = if major { style.tick * 2.0 } else { style.tick };
        let a = point(centre, inner, angle);
        let b = point(centre, inner - length, angle);
        let width = if major { style.needle * 0.6 } else { style.needle * 0.4 };
        list.push_path(Path::new(vec![PathCommand::MoveTo(a), PathCommand::LineTo(b)], Paint::Solid(style.ticks)).stroked(width));
    }

    // The needle, with a short tail past the hub, in the colour of the
    // zone it points at.
    let color = style.zones.get(zone_at(needle, n)).copied().unwrap_or(style.ticks);
    let tip = point(centre, style.radius - style.band - style.tick, needle);
    let tail = point(centre, style.hub * 0.9, needle + 180.0);
    list.push_path(Path::new(vec![PathCommand::MoveTo(tail), PathCommand::LineTo(tip)], Paint::Solid(color)).stroked(style.needle));
    let hub = Rect {
        x: centre.x - style.hub * 0.5,
        y: centre.y - style.hub * 0.5,
        width: style.hub,
        height: style.hub,
        radius: style.hub * 0.5,
        color,
    };
    list.push_rect(hub);
    let dot = style.hub * 0.4;
    list.push_rect(Rect {
        x: centre.x - dot * 0.5,
        y: centre.y - dot * 0.5,
        width: dot,
        height: dot,
        radius: dot * 0.5,
        color: style.hub_fill,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const C: Point = Point { x: 100.0, y: 100.0 };

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    #[test]
    fn three_zones_split_the_sweep() {
        assert!(close(zone_centre(0, 3), 170.0));
        assert!(close(zone_centre(1, 3), 90.0), "the middle one points straight up");
        assert!(close(zone_centre(2, 3), 10.0));
        assert_eq!(zone_at(205.0, 3), 0);
        assert_eq!(zone_at(131.0, 3), 0);
        assert_eq!(zone_at(129.0, 3), 1);
        assert_eq!(zone_at(-30.0, 3), 2, "the far end is in the last zone");
    }

    #[test]
    fn the_pointer_sets_the_angle_and_the_gap_holds_the_ends() {
        assert!(close(pointer_angle(C, Point::new(100.0, 20.0)), 90.0), "above");
        assert!(close(pointer_angle(C, Point::new(20.0, 100.0)), 180.0), "left");
        assert!(close(pointer_angle(C, Point::new(180.0, 100.0)), 0.0), "right");
        // Below and a little left: in the gap, so the left end.
        assert!(close(pointer_angle(C, Point::new(90.0, 180.0)), START));
        // Below and a little right: the right end.
        assert!(close(pointer_angle(C, Point::new(110.0, 180.0)), START - SWEEP));
        // Lower left but on the dial.
        let a = pointer_angle(C, point(C, 50.0, 200.0));
        assert!(close(a, 200.0), "{a}");
    }

    #[test]
    fn points_are_in_screen_space() {
        let up = point(C, 10.0, 90.0);
        assert!(close(up.x, 100.0) && close(up.y, 90.0), "up is towards smaller y");
        let arc = arc(C, 10.0, 180.0, 0.0);
        assert!(matches!(arc.first(), Some(PathCommand::MoveTo(p)) if close(p.x, 90.0)));
        assert!(matches!(arc.last(), Some(PathCommand::LineTo(p)) if close(p.x, 110.0)));
    }
}
