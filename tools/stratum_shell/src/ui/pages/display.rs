//! Display page: font size, output, and theme selection.

use crate::actions::SystemAction;
use crate::render::display_list::{Color, DisplayList, Rect};
use crate::ui::controls;
use crate::ui::pages::{heading, section_label, stat_row, Optimistic};
use crate::ui::{InputState, PaintCtx};

pub struct DisplayPage {
    theme: Optimistic<String>,
}

impl DisplayPage {
    pub fn new() -> Self {
        Self {
            theme: Optimistic::new(),
        }
    }

    pub fn tick(&mut self, dt: f32) {
        self.theme.tick(dt);
    }

    pub fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        area: Rect,
        input: &InputState,
        clicked: bool,
        pending: &mut Vec<SystemAction>,
    ) {
        let area = heading(ctx, list, area, "Display");
        let current = ctx.theme.name.clone();
        let selected_theme = self.theme.view(current);

        stat_row(
            ctx,
            list,
            area,
            0,
            "Font size",
            &ctx.appearance.font_size.to_string(),
        );
        stat_row(ctx, list, area, 1, "Output", ctx.output_name);

        let mut y = area.y + 84.0;
        section_label(ctx, list, area, y, "Theme");
        y += 24.0;

        // Themes come from the shell configuration, so the list grows as the
        // user adds palettes without any code change.
        let mut names: Vec<String> = ctx.themes.keys().cloned().collect();
        names.sort();
        for name in names {
            let row = Rect {
                x: area.x,
                y,
                width: area.width.min(320.0),
                height: ctx.metrics.page.row_height,
                radius: ctx.metrics.page.button_radius,
                color: Color::TRANSPARENT,
            };
            let selected = name == selected_theme;
            if controls::selectable_row(ctx, list, row, &name, "", selected, input, clicked)
                && !selected
            {
                self.theme.set(name.clone());
                pending.push(SystemAction::SetTheme(name));
            }
            y += ctx.metrics.page.stat_spacing;
        }
    }
}

impl Default for DisplayPage {
    fn default() -> Self {
        Self::new()
    }
}
