//! History graphs: a line over a shaded area, newest reading at the right.
//!
//! A graph spans a fixed number of readings, so a short history fills in
//! from the right and the line scrolls left as readings arrive, instead of
//! stretching across the width.

use std::collections::VecDeque;

use crate::render::display_list::{Color, DisplayList, Paint, Path, PathCommand, Point, Rect};

/// How a graph is drawn.
#[derive(Clone, Copy, Debug)]
pub struct GraphStyle {
    /// Readings across the full width.
    pub capacity: usize,
    /// The value at the top edge; larger values are clipped to it.
    pub max: f32,
    pub line: f32,
    /// Opacity of the area under the line, relative to the line's colour.
    pub fill: f32,
}

/// The line's points inside `rect`, inset by half the line width at the
/// top and bottom so the stroke is not cut off.
pub fn points(rect: Rect, values: &VecDeque<f32>, style: &GraphStyle) -> Vec<Point> {
    let step = rect.width / style.capacity.saturating_sub(1).max(1) as f32;
    let inset = style.line * 0.5;
    let height = (rect.height - style.line).max(0.0);
    let right = rect.x + rect.width;
    let shown = values.len().min(style.capacity);
    values
        .iter()
        .skip(values.len() - shown)
        .enumerate()
        .map(|(i, v)| {
            let level = if style.max > 0.0 { (v / style.max).clamp(0.0, 1.0) } else { 0.0 };
            Point::new(right - (shown - 1 - i) as f32 * step, rect.y + inset + height * (1.0 - level))
        })
        .collect()
}

/// Draw `values` in `color`. Fewer than two readings draw nothing.
pub fn paint(list: &mut DisplayList, rect: Rect, values: &VecDeque<f32>, color: Color, style: &GraphStyle) {
    let points = points(rect, values, style);
    let (Some(first), Some(last)) = (points.first().copied(), points.last().copied()) else {
        return;
    };
    if points.len() < 2 {
        return;
    }
    let bottom = rect.y + rect.height;
    let mut area = vec![PathCommand::MoveTo(Point::new(first.x, bottom))];
    area.extend(points.iter().map(|p| PathCommand::LineTo(*p)));
    area.extend([PathCommand::LineTo(Point::new(last.x, bottom)), PathCommand::Close]);
    list.push_path(Path::new(area, Paint::Solid(color.with_alpha(color.a * style.fill))));

    let mut line = vec![PathCommand::MoveTo(first)];
    line.extend(points[1..].iter().map(|p| PathCommand::LineTo(*p)));
    list.push_path(Path::new(line, Paint::Solid(color)).stroked(style.line));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Rect {
        Rect { x: 10.0, y: 0.0, width: 100.0, height: 21.0, radius: 0.0, color: Color::TRANSPARENT }
    }

    #[test]
    fn the_newest_reading_is_at_the_right_edge() {
        let style = GraphStyle { capacity: 11, max: 100.0, line: 1.0, fill: 0.2 };
        let values = VecDeque::from([0.0, 50.0, 100.0]);
        let points = points(rect(), &values, &style);
        assert_eq!(points.iter().map(|p| p.x).collect::<Vec<_>>(), [90.0, 100.0, 110.0]);
        // Inset half a line: 0 sits at 20.5, 100 at 0.5.
        assert_eq!(points.iter().map(|p| p.y).collect::<Vec<_>>(), [20.5, 10.5, 0.5]);
    }

    #[test]
    fn readings_beyond_the_capacity_or_the_max_are_dropped_or_clipped() {
        let style = GraphStyle { capacity: 2, max: 10.0, line: 1.0, fill: 0.2 };
        let values = VecDeque::from([1.0, 2.0, 50.0]);
        let points = points(rect(), &values, &style);
        assert_eq!(points.len(), 2);
        assert_eq!(points[0].x, 10.0);
        assert_eq!(points[1].y, 0.5, "clipped to the top");
    }
}
