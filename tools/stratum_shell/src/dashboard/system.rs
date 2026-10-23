//! The System tab's graphs: CPU (with the package temperature), RAM, each
//! GPU (temperature and VRAM; sampled only while the dashboard is open, so
//! its graph starts then) and network down and up, with storage as a bar
//! and the NVMe temperature beneath. Load graphs span 0–100 %; the network
//! graph scales to its busiest reading.
//!
//! The graphs fill their panel: as many columns as `graph_column` allows,
//! and rows sharing the height that is left.

use std::collections::VecDeque;

use stratum_sys::managers::gpu::Load;
use stratum_sys::managers::sysmon::Usage;

use super::feed::Snapshot;
use super::{load_color, Dashboard};
use crate::render::display_list::{Color, DisplayList, Rect};
use crate::ui::graph::{self, GraphStyle};
use crate::ui::{self, PaintCtx};

/// The network graph's top edge never drops below this many bytes per
/// second, so an idle link draws a flat line, not noise blown up to full
/// height.
const NETWORK_FLOOR: f32 = 64.0 * 1024.0;

/// Bytes as GiB with one decimal.
pub fn gib(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / f64::from(1u32 << 30))
}

/// A rate in bytes per second, in the largest unit that keeps it above 1.
pub fn rate(bytes_per_second: f64) -> String {
    let units = ["B/s", "KB/s", "MB/s", "GB/s"];
    let mut value = bytes_per_second.max(0.0);
    let mut unit = 0;
    while value >= 1000.0 && unit < units.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 || value >= 100.0 {
        format!("{value:.0} {}", units[unit])
    } else {
        format!("{value:.1} {}", units[unit])
    }
}

fn celsius(t: Option<f32>) -> Option<String> {
    t.map(|t| format!("{t:.0}\u{b0}C"))
}

/// `count` cells laid out in `area`: as many columns as fit `min_column`
/// wide (at least one, at most `count`), rows sharing the height, `gap`
/// apart, filled left to right. A last row that is short of cells shares
/// the whole width among those it has, leaving no hole.
pub fn grid(area: Rect, count: usize, min_column: f32, gap: f32) -> Vec<Rect> {
    if count == 0 {
        return Vec::new();
    }
    let columns = (((area.width + gap) / (min_column + gap)).floor() as usize).clamp(1, count);
    let rows = count.div_ceil(columns);
    let height = (area.height - gap * (rows - 1) as f32) / rows as f32;
    (0..count)
        .map(|i| {
            let (row, column) = (i / columns, i % columns);
            let across = if row == rows - 1 { count - row * columns } else { columns };
            let width = (area.width - gap * (across - 1) as f32) / across as f32;
            Rect {
                x: area.x + column as f32 * (width + gap),
                y: area.y + row as f32 * (height + gap),
                width,
                height,
                ..area
            }
        })
        .collect()
}

/// The parts that are known, joined with a middle dot.
fn joined(parts: impl IntoIterator<Item = Option<String>>) -> String {
    parts.into_iter().flatten().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" \u{b7} ")
}

/// One graph row: what it shows and the series to draw.
struct Row<'a> {
    glyph: &'static str,
    label: String,
    detail: String,
    value: String,
    series: Vec<(&'a VecDeque<f32>, Color)>,
    max: f32,
}

impl Dashboard {
    pub(super) fn paint_graphs(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, panel: Rect, snapshot: &Snapshot) {
        let m = self.metrics.clone();
        let y = Self::title(ctx, list, panel, "System", &m);
        let x = panel.x + m.inner;
        let w = panel.width - m.inner * 2.0;
        let Some(sample) = snapshot.sample.as_ref() else {
            return;
        };
        let history = &snapshot.history;
        let theme = ctx.theme.clone();
        let mut rows = vec![Row {
            glyph: "\u{f2db}",
            label: "CPU".into(),
            detail: joined([celsius(sample.temperatures.cpu)]),
            value: sample.cpu_percent.map_or_else(|| "\u{2013}".into(), |p| format!("{}%", p.round())),
            series: vec![(&history.cpu, theme.primary)],
            max: 100.0,
        }];
        rows.push(Row {
            glyph: "\u{f035b}",
            label: "RAM".into(),
            detail: format!("{} / {} GiB", gib(sample.memory.used), gib(sample.memory.total)),
            value: format!("{}%", sample.memory.percent().round()),
            series: vec![(&history.memory, theme.tertiary)],
            max: 100.0,
        });
        for (gpu, series) in sample.gpus.iter().zip(&history.gpus) {
            let value = match gpu.load {
                Load::Percent(p) => format!("{}%", p.clamp(0, 100)),
                Load::Asleep => "Asleep".into(),
                Load::Unknown => "N/A".into(),
            };
            let vram = gpu.vram.map(|(used, total)| format!("{} / {} GiB", gib(used), gib(total)));
            rows.push(Row {
                glyph: "\u{f08ae}",
                label: gpu.name.clone(),
                detail: joined([celsius(gpu.temperature), vram]),
                value,
                series: vec![(series, theme.secondary)],
                max: 100.0,
            });
        }
        let busiest = history.rx.iter().chain(&history.tx).copied().fold(NETWORK_FLOOR, f32::max);
        rows.push(Row {
            glyph: "\u{f06f3}",
            label: "Network".into(),
            detail: String::new(),
            value: sample
                .network
                .map_or_else(|| "\u{2013}".into(), |n| format!("\u{2193} {}  \u{2191} {}", rate(n.rx), rate(n.tx))),
            series: vec![(&history.rx, theme.primary), (&history.tx, theme.secondary)],
            // Headroom, so the busiest reading does not ride the top edge.
            max: busiest * 1.1,
        });

        // Storage changes too slowly to graph: a bar along the bottom, and
        // the graphs share what is above it.
        let storage_y = panel.y + panel.height - m.inner - m.line - m.bar * 2.0;
        let area = Rect { x, y, width: w, height: storage_y - m.graph_gap - y, radius: 0.0, color: Color::TRANSPARENT };
        let cells = grid(area, rows.len(), m.graph_column, m.graph_gap);
        for (row, cell) in rows.into_iter().zip(cells) {
            let line = Rect { height: m.line, ..cell };
            self.paint_label(ctx, list, line, row.glyph, &row.label, &row.detail, &row.value);
            let graph_area = Rect {
                y: cell.y + m.line,
                height: (cell.height - m.line).max(m.graph),
                radius: m.bar * 0.5,
                color: theme.bg_main,
                ..cell
            };
            list.push_rect(graph_area);
            let style = GraphStyle { capacity: self.config.history, max: row.max, line: m.graph_line, fill: m.graph_fill };
            for (values, color) in row.series {
                graph::paint(list, graph_area, values, color, &style);
            }
        }
        let y = storage_y;
        let root = sample.root;
        let line = Rect { x, y, width: w, height: m.line, radius: 0.0, color: Color::TRANSPARENT };
        let detail = joined([Some(format!("{} / {} GiB", gib(root.used), gib(root.total))), celsius(sample.temperatures.storage)]);
        self.paint_label(ctx, list, line, "\u{f0a0}", "Storage", &detail, &format!("{}%", root.percent().round()));
        let track = Rect { x, y: y + m.line + m.bar * 0.5, width: w, height: m.bar, radius: m.bar * 0.5, color: theme.bg_hover };
        list.push_rect(track);
        let percent = f64::from(Usage::percent(&root));
        list.push_rect(Rect { width: w * (percent.clamp(0.0, 100.0) / 100.0) as f32, color: load_color(ctx, percent), ..track });
    }

    /// Glyph, label and a muted detail on the left; the value on the right.
    /// The detail gives way to the value when they would meet.
    #[allow(clippy::too_many_arguments)]
    fn paint_label(
        &self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        line: Rect,
        glyph: &str,
        label: &str,
        detail: &str,
        value: &str,
    ) {
        let m = &self.metrics;
        let (regular, medium) = (ctx.metrics.typography.weight_regular, ctx.metrics.typography.weight_medium);
        let indent = m.text * 1.6;
        ui::paint_text_vcentered(ctx, list, glyph, line.x, line, m.text, regular, ctx.theme.primary);
        let value_w = measure(ctx, value, m.text, medium);
        let right = line.x + line.width - value_w;
        ui::paint_text_vcentered(ctx, list, value, right, line, m.text, medium, ctx.theme.text_main);
        let label_room = right - line.x - indent - m.inner;
        let label = ui::elide(ctx, label, m.text, medium, label_room);
        ui::paint_text_vcentered(ctx, list, &label, line.x + indent, line, m.text, medium, ctx.theme.text_main);
        let detail_x = line.x + indent + measure(ctx, &label, m.text, medium) + m.inner;
        let detail = ui::elide(ctx, detail, m.caption, regular, right - detail_x - m.inner);
        ui::paint_text_vcentered(ctx, list, &detail, detail_x, line, m.caption, regular, ctx.theme.text_muted);
    }
}

fn measure(ctx: &mut PaintCtx<'_>, text: &str, size: f32, weight: u16) -> f32 {
    let style = crate::text::TextStyle {
        family: std::sync::Arc::from(ctx.theme.font.as_str()),
        size,
        weight,
        color: ctx.theme.text_main,
    };
    ctx.text.measure(text, &style).0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_pick_a_unit_that_keeps_them_readable() {
        assert_eq!(rate(0.0), "0 B/s");
        assert_eq!(rate(999.0), "999 B/s");
        assert_eq!(rate(2_100.0), "2.1 KB/s");
        assert_eq!(rate(450_000.0), "450 KB/s");
        assert_eq!(rate(12_340_000.0), "12.3 MB/s");
    }

    #[test]
    fn graphs_take_as_many_columns_as_fit() {
        let area = Rect { x: 0.0, y: 0.0, width: 700.0, height: 300.0, radius: 0.0, color: Color::TRANSPARENT };
        let cells = grid(area, 5, 320.0, 10.0);
        // Two columns of 345, three rows of 93.3.
        assert_eq!(cells.len(), 5);
        assert_eq!((cells[1].x, cells[1].width), (355.0, 345.0));
        assert_eq!(cells[2].y, cells[0].y + cells[0].height + 10.0);
        assert!((cells[4].y + cells[4].height - 300.0).abs() < 0.01, "the last row reaches the bottom");
        assert_eq!((cells[4].x, cells[4].width), (0.0, 700.0), "alone on its row, it spans it");
        // Narrower than one column still gets one; more room than graphs
        // gets no empty columns.
        assert_eq!(grid(Rect { width: 100.0, ..area }, 3, 320.0, 10.0)[2].x, 0.0);
        assert_eq!(grid(Rect { width: 2000.0, ..area }, 2, 320.0, 10.0)[1].width, 995.0);
        assert!(grid(area, 0, 320.0, 10.0).is_empty());
    }

    #[test]
    fn unknown_parts_are_left_out_of_a_detail() {
        assert_eq!(joined([Some("48\u{b0}C".into()), None, Some("2.0 / 8.0 GiB".into())]), "48\u{b0}C \u{b7} 2.0 / 8.0 GiB");
        assert_eq!(joined([None, Some(String::new())]), "");
    }
}
