//! The System tab's panels: history graphs and the busiest processes.
//!
//! - Graphs: CPU (with the package temperature), each GPU (temperature and
//!   VRAM; sampled only while the dashboard is open, so its graph starts
//!   then), RAM, network down and up, and storage as a bar with the NVMe
//!   temperature. Load graphs span 0–100 %; the network graph scales to its
//!   busiest reading.
//! - Processes: the busiest by CPU share of the whole machine, so they add
//!   up to the CPU row, with resident memory. Its title runs
//!   `[dashboard] process_monitor` (btop in a terminal by default).

use std::collections::VecDeque;

use stratum_sys::managers::gpu::Load;
use stratum_sys::managers::sysmon::{Process, Usage};

use super::feed::Snapshot;
use super::{load_color, Button, Dashboard};
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

/// A process's CPU share: a decimal below 10 %, where it still means
/// something.
pub fn cpu_share(percent: f32) -> String {
    if percent < 10.0 {
        format!("{percent:.1}%")
    } else {
        format!("{percent:.0}%")
    }
}

/// A process's memory: MiB below a GiB.
pub fn memory(bytes: u64) -> String {
    if bytes < 1 << 30 {
        format!("{} MiB", bytes >> 20)
    } else {
        format!("{} GiB", gib(bytes))
    }
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
        let mut y = Self::title(ctx, list, panel, "System", &m);
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
        rows.push(Row {
            glyph: "\u{f035b}",
            label: "RAM".into(),
            detail: format!("{} / {} GiB", gib(sample.memory.used), gib(sample.memory.total)),
            value: format!("{}%", sample.memory.percent().round()),
            series: vec![(&history.memory, theme.tertiary)],
            max: 100.0,
        });
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

        for row in rows {
            let line = Rect { x, y, width: w, height: m.line, radius: 0.0, color: Color::TRANSPARENT };
            self.paint_label(ctx, list, line, row.glyph, &row.label, &row.detail, &row.value);
            y += m.line;
            let area = Rect { x, y, width: w, height: m.graph, radius: m.bar * 0.5, color: theme.bg_main };
            list.push_rect(area);
            let style = GraphStyle { capacity: self.config.history, max: row.max, line: m.graph_line, fill: m.graph_fill };
            for (values, color) in row.series {
                graph::paint(list, area, values, color, &style);
            }
            y += m.graph + m.graph_gap;
        }

        // Storage changes too slowly to graph.
        let root = sample.root;
        let line = Rect { x, y, width: w, height: m.line, radius: 0.0, color: Color::TRANSPARENT };
        let detail = joined([Some(format!("{} / {} GiB", gib(root.used), gib(root.total))), celsius(sample.temperatures.storage)]);
        self.paint_label(ctx, list, line, "\u{f0a0}", "Storage", &detail, &format!("{}%", root.percent().round()));
        y += m.line;
        let track = Rect { x, y: y + m.bar * 0.5, width: w, height: m.bar, radius: m.bar * 0.5, color: theme.bg_hover };
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

    pub(super) fn paint_processes(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        panel: Rect,
        processes: &[Process],
    ) {
        let m = self.metrics.clone();
        let y0 = Self::title(ctx, list, panel, "Processes", &m);
        let x = panel.x + m.inner;
        let w = panel.width - m.inner * 2.0;
        if !self.config.process_monitor.is_empty() {
            // The whole title row opens the monitor; a glyph says so.
            let title = Rect { x: panel.x, y: panel.y, width: panel.width, height: y0 - panel.y, radius: 0.0, color: Color::TRANSPARENT };
            let glyph_row = Rect { y: panel.y + m.inner, height: m.line * 1.2, ..title };
            let glyph = "\u{f08e}";
            let gw = measure(ctx, glyph, m.text, ctx.metrics.typography.weight_regular);
            ui::paint_text_vcentered(ctx, list, glyph, x + w - gw, glyph_row, m.text, ctx.metrics.typography.weight_regular, ctx.theme.text_muted);
            self.buttons.push((title, Button::ProcessMonitor));
        }
        let regular = ctx.metrics.typography.weight_regular;
        let columns = [w * 0.6, w * 0.18, w * 0.22];
        let header = Rect { x, y: y0, width: w, height: m.line, radius: 0.0, color: Color::TRANSPARENT };
        self.paint_process_row(ctx, list, header, &columns, ["Name", "CPU", "Memory"], m.caption, ctx.theme.text_muted);
        let mut y = y0 + m.line;
        if processes.is_empty() {
            let row = Rect { y, ..header };
            ui::paint_text_vcentered(ctx, list, "Measuring\u{2026}", x, row, m.caption, regular, ctx.theme.text_muted);
            return;
        }
        let bottom = panel.y + panel.height - m.inner;
        for process in processes {
            if y + m.line > bottom {
                break;
            }
            let row = Rect { y, ..header };
            let cpu = cpu_share(process.cpu_percent);
            let mem = memory(process.memory);
            self.paint_process_row(ctx, list, row, &columns, [&process.name, &cpu, &mem], m.text, ctx.theme.text_main);
            y += m.line;
        }
    }

    /// A name column, then right-aligned numeric columns.
    #[allow(clippy::too_many_arguments)]
    fn paint_process_row(
        &self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        row: Rect,
        columns: &[f32; 3],
        cells: [&str; 3],
        size: f32,
        color: Color,
    ) {
        let weight = ctx.metrics.typography.weight_regular;
        let name = ui::elide(ctx, cells[0], size, weight, columns[0] - self.metrics.inner);
        ui::paint_text_vcentered(ctx, list, &name, row.x, row, size, weight, color);
        let mut right = row.x + columns[0];
        for (cell, width) in cells[1..].iter().zip(&columns[1..]) {
            right += width;
            let cw = measure(ctx, cell, size, weight);
            ui::paint_text_vcentered(ctx, list, cell, right - cw, row, size, weight, color);
        }
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
    fn process_columns_format_for_their_size() {
        assert_eq!(cpu_share(0.44), "0.4%");
        assert_eq!(cpu_share(12.6), "13%");
        assert_eq!(memory(300 << 20), "300 MiB");
        assert_eq!(memory(3 << 30), "3.0 GiB");
    }

    #[test]
    fn unknown_parts_are_left_out_of_a_detail() {
        assert_eq!(joined([Some("48\u{b0}C".into()), None, Some("2.0 / 8.0 GiB".into())]), "48\u{b0}C \u{b7} 2.0 / 8.0 GiB");
        assert_eq!(joined([None, Some(String::new())]), "");
    }
}
