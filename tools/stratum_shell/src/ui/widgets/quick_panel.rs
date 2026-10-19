//! The quick-panel container.
//!
//! A framed group of widgets with its own inner padding and spacing. In a
//! vertical rail it is a narrow column centred in the rail; in a horizontal bar
//! it lays its children out in a row. This is the reusable version of the QML
//! `quickPanel*` group.

use crate::config::Axis;
use crate::render::display_list::{Color, DisplayList, Rect};
use crate::ui::{self, Action, ClickCtx, InputState, PaintCtx, Widget};

/// Rail width used by the QML shell (`quickPanelWidth`).
const PANEL_WIDTH: f32 = 30.0;
const INNER_MARGIN: f32 = 6.0;
const CHILD_SPACING: f32 = 4.0;

pub struct QuickPanel {
    pub framed: bool,
    pub height: f32,
    pub children: Vec<Box<dyn Widget>>,
}

impl QuickPanel {
    pub fn new(framed: bool, height: f32, children: Vec<Box<dyn Widget>>) -> Self {
        Self {
            framed,
            height,
            children,
        }
    }

    fn paint_vertical(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        rect: Rect,
        input: &InputState,
    ) {
        // The panel is narrower than the rail and centred in it.
        let width = PANEL_WIDTH.min(rect.width);
        let panel = Rect {
            x: rect.x + (rect.width - width) * 0.5,
            y: rect.y,
            width,
            height: rect.height,
            radius: ctx.appearance.corner_radius,
            color: ctx.theme.bg_widget,
        };
        if self.framed {
            ui::paint_frame(
                list,
                panel,
                ctx.theme.bg_widget,
                ctx.theme.border_active,
                ctx.appearance.framed_border,
            );
        }

        let count = self.children.len();
        if count == 0 {
            return;
        }
        let inner_height = (panel.height - INNER_MARGIN * 2.0).max(0.0);
        let gaps = CHILD_SPACING * (count.saturating_sub(1)) as f32;
        let slot = ((inner_height - gaps) / count as f32).max(0.0);

        let mut y = panel.y + INNER_MARGIN;
        for child in &mut self.children {
            let slot_rect = Rect {
                x: panel.x,
                y,
                width: panel.width,
                height: slot,
                radius: 0.0,
                color: Color::TRANSPARENT,
            };
            child.paint(ctx, list, slot_rect, input);
            y += slot + CHILD_SPACING;
        }
    }

    fn paint_horizontal(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        rect: Rect,
        input: &InputState,
    ) {
        const HORIZONTAL_MARGIN: f32 = 4.0;
        const HORIZONTAL_SPACING: f32 = 6.0;

        if self.framed {
            ui::paint_frame(
                list,
                rect,
                ctx.theme.bg_widget,
                ctx.theme.border_active,
                ctx.appearance.framed_border,
            );
        }

        let count = self.children.len();
        if count == 0 {
            return;
        }
        let inner = (rect.width - HORIZONTAL_MARGIN * 2.0).max(0.0);
        let gaps = HORIZONTAL_SPACING * (count.saturating_sub(1)) as f32;
        let slot = ((inner - gaps) / count as f32).max(0.0);

        let mut x = rect.x + HORIZONTAL_MARGIN;
        for child in &mut self.children {
            let slot_rect = Rect {
                x,
                y: rect.y,
                width: slot,
                height: rect.height,
                radius: 0.0,
                color: Color::TRANSPARENT,
            };
            child.paint(ctx, list, slot_rect, input);
            x += slot + HORIZONTAL_SPACING;
        }
    }
}

impl Widget for QuickPanel {
    fn height(&self, _width: f32) -> Option<f32> {
        Some(self.height)
    }

    fn width(&self, height: f32) -> Option<f32> {
        let slot = height - 8.0;
        let count = self.children.len().max(1) as f32;
        Some(slot * count + 6.0 * (count - 1.0) + 8.0)
    }

    fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        rect: Rect,
        input: &InputState,
    ) {
        match ctx.axis {
            Axis::Vertical => self.paint_vertical(ctx, list, rect, input),
            Axis::Horizontal => self.paint_horizontal(ctx, list, rect, input),
        }
    }

    fn on_click(&mut self, ctx: &ClickCtx<'_>) -> Option<Action> {
        self.children.iter_mut().find_map(|child| child.on_click(ctx))
    }

    fn tick(&mut self, dt: f32) -> bool {
        crate::ui::tick_all(&mut self.children, dt)
    }

    fn animating(&self) -> bool {
        self.children.iter().any(|child| child.animating())
    }
}
