//! The emoji picker's grid: results in rows of `columns` cells, each group
//! under a header line, and how the selection moves through it. Pure; the
//! launcher paints it.

/// One line of the grid, top to bottom.
#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    /// A section header, naming a group.
    Header(String),
    /// Results `start..end`, at most `columns` of them.
    Cells { start: usize, end: usize },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Grid {
    pub lines: Vec<Line>,
    pub columns: usize,
}

impl Grid {
    /// Lay out `groups`, each a name and how many consecutive results it
    /// holds. A group with no name has no header.
    pub fn build(groups: &[(Option<String>, usize)], columns: usize) -> Self {
        let columns = columns.max(1);
        let mut lines = Vec::new();
        let mut start = 0;
        for (name, count) in groups {
            if *count == 0 {
                continue;
            }
            if let Some(name) = name {
                lines.push(Line::Header(name.clone()));
            }
            let end = start + count;
            let mut row = start;
            while row < end {
                lines.push(Line::Cells { start: row, end: (row + columns).min(end) });
                row += columns;
            }
            start = end;
        }
        Self { lines, columns }
    }

    /// The line holding result `index`, and its column.
    pub fn position(&self, index: usize) -> Option<(usize, usize)> {
        self.lines.iter().enumerate().find_map(|(line, l)| match l {
            Line::Cells { start, end } if (*start..*end).contains(&index) => Some((line, index - start)),
            _ => None,
        })
    }

    /// The result above (`-1`) or below (`1`) `index`, `steps` rows away,
    /// in the same column or the last one of a shorter row. Headers are
    /// skipped; the selection stops at the first and last rows.
    pub fn vertical(&self, index: usize, steps: isize) -> usize {
        let Some((mut line, column)) = self.position(index) else {
            return index;
        };
        let mut target = index;
        let mut moved = 0;
        while moved < steps.unsigned_abs() {
            let next = if steps < 0 { line.checked_sub(1) } else { Some(line + 1) };
            let Some(next) = next.filter(|n| *n < self.lines.len()) else {
                break;
            };
            line = next;
            if let Line::Cells { start, end } = self.lines[line] {
                target = (start + column).min(end - 1);
                moved += 1;
            }
        }
        target
    }

    /// The top of each line, given the heights of headers and rows, and the
    /// total height.
    pub fn offsets(&self, header: f32, row: f32) -> (Vec<f32>, f32) {
        let mut y = 0.0;
        let tops = self
            .lines
            .iter()
            .map(|line| {
                let top = y;
                y += match line {
                    Line::Header(_) => header,
                    Line::Cells { .. } => row,
                };
                top
            })
            .collect();
        (tops, y)
    }

    /// The scroll offset that shows result `index`'s row in a view `height`
    /// tall, moving `scroll` as little as possible. A row right under its
    /// header shows the header too.
    pub fn scroll_to(&self, index: usize, scroll: f32, height: f32, header: f32, row: f32) -> f32 {
        let Some((line, _)) = self.position(index) else {
            return scroll;
        };
        let (tops, total) = self.offsets(header, row);
        let mut top = tops[line];
        if line > 0 && matches!(self.lines[line - 1], Line::Header(_)) {
            top = tops[line - 1];
        }
        let bottom = tops[line] + row;
        let scroll = if top < scroll {
            top
        } else if bottom > scroll + height {
            bottom - height
        } else {
            scroll
        };
        scroll.clamp(0.0, (total - height).max(0.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> Grid {
        // Faces: 0..5 in rows of 3; Animals: 5..7.
        Grid::build(&[(Some("Faces".into()), 5), (Some("Animals".into()), 2)], 3)
    }

    #[test]
    fn groups_get_headers_and_rows() {
        assert_eq!(
            grid().lines,
            [
                Line::Header("Faces".into()),
                Line::Cells { start: 0, end: 3 },
                Line::Cells { start: 3, end: 5 },
                Line::Header("Animals".into()),
                Line::Cells { start: 5, end: 7 },
            ]
        );
        assert_eq!(Grid::build(&[(None, 4)], 3).lines.len(), 2, "no header without a name");
    }

    #[test]
    fn up_and_down_keep_the_column_and_skip_headers() {
        let g = grid();
        assert_eq!(g.vertical(1, 1), 4, "down a row");
        assert_eq!(g.vertical(2, 1), 4, "into a shorter row: its last cell");
        assert_eq!(g.vertical(4, 1), 6, "over the header");
        assert_eq!(g.vertical(6, -1), 4);
        assert_eq!(g.vertical(0, -1), 0, "stops at the top");
        assert_eq!(g.vertical(5, 5), 5, "and at the bottom");
        assert_eq!(g.vertical(0, 2), 5, "two rows");
    }

    #[test]
    fn scrolling_shows_the_selection_and_its_header() {
        let g = grid(); // tops: 0, 10, 50, 90, 100; total 140
        assert_eq!(g.offsets(10.0, 40.0).1, 140.0);
        assert_eq!(g.scroll_to(5, 0.0, 60.0, 10.0, 40.0), 80.0, "bottom row fully in view");
        assert_eq!(g.scroll_to(0, 80.0, 60.0, 10.0, 40.0), 0.0, "first row with its header");
        assert_eq!(g.scroll_to(3, 0.0, 100.0, 10.0, 40.0), 0.0, "already visible");
    }
}
