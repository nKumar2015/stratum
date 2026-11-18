//! Display page: the font and the output. Themes moved to the Theme page;
//! this page is kept for display settings to come.

use crate::actions::SystemAction;
use crate::render::display_list::{DisplayList, Rect};
use crate::ui::pages::layout;
use crate::ui::{InputState, PaintCtx};

pub struct DisplayPage;

impl DisplayPage {
    pub fn new() -> Self {
        Self
    }

    /// Returns the bottom of the page.
    pub fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        area: Rect,
        _input: &InputState,
        _clicked: bool,
        _pending: &mut Vec<SystemAction>,
    ) -> f32 {
        let m = ctx.metrics.page.clone();
        let area = layout::column(area, m.max_width);
        let summary = ctx.output_name.to_string();
        let (_, body) = layout::header(ctx, list, area, "Display", &summary);

        // The text and output facts.
        let y = layout::section(ctx, list, body, body.y, "Text and output");
        let facts = [
            ("Font", ctx.theme.font.clone()),
            ("Output", ctx.output_name.to_string()),
        ];
        let cols = layout::columns(body.width, m.card_min_width, m.grid_gap, facts.len());
        let cols = layout::even_columns(cols, facts.len());
        let cells = layout::cells(body.x, y, body.width, facts.len(), cols, m.info_height, m.grid_gap);
        for ((label, value), cell) in facts.iter().zip(&cells) {
            layout::info_card(ctx, list, *cell, label, value);
        }
        y + layout::grid_height(facts.len(), cols, m.info_height, m.grid_gap)
    }
}

impl Default for DisplayPage {
    fn default() -> Self {
        Self::new()
    }
}
