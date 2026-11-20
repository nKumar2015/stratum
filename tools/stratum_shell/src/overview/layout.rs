//! Where the overview puts its workspaces and their windows. Pure, so it
//! is tested on its own.

use crate::render::display_list::{Color, Rect};

/// The workspaces' cards: `count` of them with the output's `aspect`
/// (width / height), as large as fit in `area` with `gap` between them and
/// `label` above each for its name. Rows are centred, the last one too.
/// Returns the cards and the number of columns.
pub fn grid(count: usize, area: Rect, aspect: f32, gap: f32, label: f32) -> (Vec<Rect>, usize) {
    if count == 0 || area.width <= 0.0 || area.height <= 0.0 || aspect <= 0.0 {
        return (Vec::new(), 1);
    }
    // The column count giving the widest cards.
    let (columns, width) = (1..=count)
        .map(|columns| {
            let rows = count.div_ceil(columns);
            let by_width = (area.width - gap * (columns - 1) as f32) / columns as f32;
            let by_height = (area.height - (label + gap) * rows as f32 + gap) / rows as f32 * aspect;
            (columns, by_width.min(by_height))
        })
        .fold((1, f32::MIN), |best, next| if next.1 > best.1 { next } else { best });
    let width = width.max(1.0);
    let height = width / aspect;
    let rows = count.div_ceil(columns);
    let total_height = (label + height) * rows as f32 + gap * (rows - 1) as f32;
    let top = area.y + (area.height - total_height) * 0.5;
    let mut cards = Vec::with_capacity(count);
    for row in 0..rows {
        let in_row = (count - row * columns).min(columns);
        let row_width = width * in_row as f32 + gap * (in_row - 1) as f32;
        let left = area.x + (area.width - row_width) * 0.5;
        let y = top + row as f32 * (label + height + gap) + label;
        for column in 0..in_row {
            cards.push(Rect {
                x: left + column as f32 * (width + gap),
                y,
                width,
                height,
                radius: 0.0,
                color: Color::TRANSPARENT,
            });
        }
    }
    (cards, columns)
}

/// A window at `window` (x, y, width, height in the layout) on a monitor at
/// `monitor`, drawn in `card`.
pub fn place(window: (i32, i32, i32, i32), monitor: (i32, i32, i32, i32), card: Rect) -> Rect {
    let scale = card.width / monitor.2.max(1) as f32;
    Rect {
        x: card.x + (window.0 - monitor.0) as f32 * scale,
        y: card.y + (window.1 - monitor.1) as f32 * scale,
        width: window.2 as f32 * scale,
        height: window.3 as f32 * scale,
        radius: 0.0,
        color: Color::TRANSPARENT,
    }
}

/// The card a key moves to from `from` in a grid of `count` cards with
/// `columns` columns: left and right step through every card in order, up
/// and down keep the column where the row has one.
pub fn step(from: usize, count: usize, columns: usize, dx: i32, dy: i32) -> usize {
    if count == 0 {
        return 0;
    }
    let columns = columns.max(1);
    if dx != 0 {
        return (from as i64 + dx as i64).rem_euclid(count as i64) as usize;
    }
    let (row, column) = (from / columns, from % columns);
    let rows = count.div_ceil(columns);
    let target = (row as i64 + dy as i64).clamp(0, rows as i64 - 1) as usize;
    (target * columns + column).min(count - 1)
}

/// Texture coordinates that cover `rect` with a picture of `size`,
/// cropping its middle as a wallpaper is.
pub fn cover_uv(rect: Rect, size: (u32, u32)) -> [(f32, f32); 4] {
    let (w, h) = (size.0.max(1) as f32, size.1.max(1) as f32);
    let (rect_aspect, image_aspect) = (rect.width / rect.height.max(1.0), w / h);
    let (u, v) = if image_aspect > rect_aspect {
        let keep = rect_aspect / image_aspect;
        ((1.0 - keep) * 0.5, 0.0)
    } else {
        let keep = image_aspect / rect_aspect;
        (0.0, (1.0 - keep) * 0.5)
    };
    [(u, v), (1.0 - u, v), (1.0 - u, 1.0 - v), (u, 1.0 - v)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(width: f32, height: f32) -> Rect {
        Rect { x: 0.0, y: 0.0, width, height, radius: 0.0, color: Color::TRANSPARENT }
    }

    #[test]
    fn ten_workspaces_on_a_wide_output_take_the_widest_cards() {
        let (cards, columns) = grid(10, area(1488.0, 888.0), 1.6, 28.0, 21.0);
        assert_eq!(cards.len(), 10);
        assert_eq!(columns, 4, "four columns of three rows beat five of two");
        let card = cards[0];
        assert!((card.width / card.height - 1.6).abs() < 1e-3);
        // Everything inside the area.
        for card in &cards {
            assert!(card.x >= 0.0 && card.x + card.width <= 1488.0 + 1e-3);
            assert!(card.y >= 21.0 && card.y + card.height <= 888.0 + 1e-3);
        }
    }

    #[test]
    fn the_last_row_is_centred() {
        let (cards, columns) = grid(3, area(1000.0, 1000.0), 1.0, 10.0, 0.0);
        assert_eq!(columns, 2);
        let middle = cards[2].x + cards[2].width * 0.5;
        assert!((middle - 500.0).abs() < 1e-3);
    }

    #[test]
    fn one_workspace_fills_the_area_by_its_shape() {
        let (cards, _) = grid(1, area(1600.0, 800.0), 1.6, 20.0, 20.0);
        assert!((cards[0].height - 780.0).abs() < 1e-3, "height-bound: {:?}", cards[0]);
    }

    #[test]
    fn windows_scale_with_the_card() {
        let card = Rect { x: 100.0, y: 50.0, width: 400.0, height: 250.0, radius: 0.0, color: Color::TRANSPARENT };
        let placed = place((1640, 440, 800, 500), (1600, 0, 1600, 1000), card);
        assert_eq!((placed.x, placed.y, placed.width, placed.height), (110.0, 160.0, 200.0, 125.0));
    }

    #[test]
    fn keys_walk_the_grid() {
        // 10 cards, 4 columns: rows of 4, 4, 2.
        assert_eq!(step(3, 10, 4, 1, 0), 4, "right runs on to the next row");
        assert_eq!(step(9, 10, 4, 1, 0), 0, "and wraps");
        assert_eq!(step(0, 10, 4, -1, 0), 9);
        assert_eq!(step(1, 10, 4, 0, 1), 5);
        assert_eq!(step(7, 10, 4, 0, 1), 9, "a short last row keeps to its end");
        assert_eq!(step(1, 10, 4, 0, -1), 1, "the top row stays");
    }

    #[test]
    fn a_wide_picture_is_cropped_at_its_sides() {
        let rect = Rect { x: 0.0, y: 0.0, width: 100.0, height: 100.0, radius: 0.0, color: Color::TRANSPARENT };
        let uv = cover_uv(rect, (200, 100));
        assert_eq!(uv[0], (0.25, 0.0));
        assert_eq!(uv[2], (0.75, 1.0));
    }
}
