//! A card of labelled setting rows (a switch, a stepped slider, chips or a
//! note), laid out as the Display page's monitor card: labels in a column
//! at least `editor_label_width` wide (wider for a long label, up to 45% of
//! the card), controls beside them, one `badge` tall each. The card is
//! given its rows' labels up front, which give its height and the label
//! column, so it is painted before them.

use crate::render::display_list::{DisplayList, Rect};
use crate::ui::pages::layout;
use crate::ui::{self, controls, InputState, PaintCtx};

pub struct SettingsCard {
    label_x: f32,
    label_w: f32,
    value_x: f32,
    value_w: f32,
    row_h: f32,
    gap: f32,
    y: f32,
    bottom: f32,
}

impl SettingsCard {
    /// Paint the card for rows labelled `labels` at `y` in `body`, with a
    /// title and caption on top.
    pub fn begin(
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        body: Rect,
        y: f32,
        title: (&str, &str),
        labels: &[&str],
    ) -> Self {
        let m = ctx.metrics.page.clone();
        let typo = ctx.metrics.typography.clone();
        let pad = m.card_padding;
        let gap = m.label_gap;
        let row_h = m.badge;
        let title_h = typo.line(typo.body) + typo.line(typo.caption);
        let height = pad * 2.0 + title_h + labels.len() as f32 * (gap + row_h);
        let card = layout::rect(body.x, y, body.width, height);
        layout::card(ctx, list, card, layout::Card::Plain);
        let widest = labels.iter().map(|l| ctx.measure(l, typo.body)).fold(0.0, f32::max);
        let label_w = m.editor_label_width.max(widest + gap * 2.0).min(body.width * 0.45);
        let value_x = body.x + pad + label_w;
        let title_rect = layout::rect(body.x + pad, y + pad, body.width - pad * 2.0, title_h);
        layout::two_lines(ctx, list, title_rect, title.0, title.1, ctx.theme.text_muted);
        Self {
            label_x: body.x + pad,
            label_w,
            value_x,
            value_w: body.x + body.width - pad - value_x,
            row_h,
            gap,
            y: title_rect.y + title_h,
            bottom: y + height,
        }
    }

    pub fn bottom(&self) -> f32 {
        self.bottom
    }

    /// Start a row labelled `label`: its top.
    fn row(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, label: &str) -> f32 {
        self.y += self.gap;
        let typo = ctx.metrics.typography.clone();
        let row = layout::rect(self.label_x, self.y, self.label_w, self.row_h);
        let label = ui::elide(ctx, label, typo.body, typo.weight_regular, self.label_w - self.gap);
        ui::paint_text_vcentered(ctx, list, &label, row.x, row, typo.body, typo.weight_regular, ctx.theme.text_main);
        let top = self.y;
        self.y += self.row_h;
        top
    }

    /// A switch, with an optional muted note beside it. Returns the new
    /// state when clicked.
    #[allow(clippy::too_many_arguments)]
    pub fn switch(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        label: &str,
        note: Option<&str>,
        on: bool,
        input: &InputState,
        clicked: bool,
    ) -> Option<bool> {
        let top = self.row(ctx, list, label);
        let m = ctx.metrics.page.clone();
        let typo = ctx.metrics.typography.clone();
        let y = top + (self.row_h - m.toggle_height) * 0.5;
        let changed = controls::switch(ctx, list, self.value_x, y, on, input, clicked);
        if let Some(note) = note {
            let x = self.value_x + m.toggle_width + self.gap;
            let width = (self.value_x + self.value_w - x).max(0.0);
            let note = ui::elide(ctx, note, typo.caption, typo.weight_regular, width);
            let row = layout::rect(x, top, width, self.row_h);
            ui::paint_text_vcentered(ctx, list, &note, x, row, typo.caption, typo.weight_regular, ctx.theme.text_muted);
        }
        changed
    }

    /// A slider over `labels`, showing the chosen one at its right. `drag`
    /// is the page's own state for this slider. Returns a different index
    /// while dragged to one.
    #[allow(clippy::too_many_arguments)]
    pub fn steps(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        label: &str,
        labels: &[String],
        index: usize,
        drag: &mut bool,
        input: &InputState,
        clicked: bool,
    ) -> Option<usize> {
        let top = self.row(ctx, list, label);
        let m = ctx.metrics.page.clone();
        let typo = ctx.metrics.typography.clone();
        let widest = labels.iter().map(|l| ctx.measure(l, typo.body)).fold(0.0, f32::max);
        let readout_w = widest + self.gap * 2.0;
        let knob = m.slider_knob * 0.5;
        let slider = layout::rect(self.value_x + knob, top, (self.value_w - readout_w - knob * 2.0).max(0.0), self.row_h);
        // The knob reaches past the track's ends; so does the grab.
        let grab = Rect { x: slider.x - knob, width: slider.width + knob * 2.0, ..slider };
        if input.pressed && clicked && layout::hovered(input, grab) {
            *drag = true;
        } else if !input.pressed {
            *drag = false;
        }
        let chosen = controls::stepped_slider(ctx, list, slider, index, labels.len(), *drag, input)
            .filter(|i| *i != index && *i < labels.len());
        let shown = labels.get(chosen.unwrap_or(index)).cloned().unwrap_or_default();
        let x = self.value_x + self.value_w - ctx.measure(&shown, typo.body);
        let row = layout::rect(x, top, self.value_x + self.value_w - x, self.row_h);
        ui::paint_text_vcentered(ctx, list, &shown, x, row, typo.body, typo.weight_regular, ctx.theme.text_main);
        chosen
    }

    /// Chips on one line, one of them selected. Returns a newly clicked one.
    #[allow(clippy::too_many_arguments)]
    pub fn chips(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        label: &str,
        labels: &[String],
        selected: usize,
        input: &InputState,
        clicked: bool,
    ) -> Option<usize> {
        let top = self.row(ctx, list, label);
        let (_, hit) = layout::chip_flow(ctx, list, self.value_x, top, self.value_w, labels, Some(selected), input, clicked);
        hit.filter(|i| *i != selected)
    }

    /// A muted line of text across the value column.
    pub fn note(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, label: &str, text: &str, color: crate::render::display_list::Color) {
        let top = self.row(ctx, list, label);
        let typo = ctx.metrics.typography.clone();
        let text = ui::elide(ctx, text, typo.body, typo.weight_regular, self.value_w);
        let row = layout::rect(self.value_x, top, self.value_w, self.row_h);
        ui::paint_text_vcentered(ctx, list, &text, self.value_x, row, typo.body, typo.weight_regular, color);
    }
}

/// A duration as a setting shows it: `Off` for 0, else seconds, minutes or
/// hours.
pub fn duration_label(seconds: u32) -> String {
    match seconds {
        0 => "Off".to_string(),
        s if s < 60 => format!("{s} s"),
        s if s % 3600 == 0 => format!("{} h", s / 3600),
        s if s % 60 == 0 => format!("{} min", s / 60),
        s => format!("{}:{:02}", s / 60, s % 60),
    }
}

/// `choices` with `current` among them (a value set in the config that is
/// not one of them), sorted, and the index of `current`.
pub fn with_current(choices: &[u32], current: u32) -> (Vec<u32>, usize) {
    let mut values = choices.to_vec();
    if !values.contains(&current) {
        values.push(current);
        values.sort_unstable();
    }
    let index = values.iter().position(|v| *v == current).unwrap_or(0);
    (values, index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_naturally() {
        assert_eq!(duration_label(0), "Off");
        assert_eq!(duration_label(30), "30 s");
        assert_eq!(duration_label(240), "4 min");
        assert_eq!(duration_label(7200), "2 h");
        assert_eq!(duration_label(330), "5:30");
    }

    #[test]
    fn a_configured_value_outside_the_choices_is_offered() {
        assert_eq!(with_current(&[0, 60, 300], 300), (vec![0, 60, 300], 2));
        assert_eq!(with_current(&[0, 60, 300], 330), (vec![0, 60, 300, 330], 3));
        assert_eq!(with_current(&[0, 60, 300], 45), (vec![0, 45, 60, 300], 1));
    }
}
