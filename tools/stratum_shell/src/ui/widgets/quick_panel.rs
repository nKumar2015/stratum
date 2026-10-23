//! The quick-panel container.
//!
//! A framed group of widgets with its own inner padding and spacing. In a
//! vertical rail it is a narrow column centred in the rail; in a horizontal bar
//! it lays its children out in a row. This is the reusable version of the QML
//! `quickPanel*` group.

use crate::config::Axis;
use crate::render::display_list::{Color, DisplayList, Rect};
use crate::ui::{self, Action, ClickCtx, InputState, PaintCtx, Widget};

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
        let bar = &ctx.metrics.bar;
        let width = bar.group_width.min(rect.width);
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
                ctx.appearance.frame_border(),
                ctx.theme.border_active,
                ctx.theme.bg_widget,
            );
        }

        let count = self.children.len();
        if count == 0 {
            return;
        }
        let inner_height = (panel.height - bar.group_margin * 2.0).max(0.0);
        let gaps = bar.group_spacing * (count.saturating_sub(1)) as f32;
        let slot = ((inner_height - gaps) / count as f32).max(0.0);

        let mut y = panel.y + bar.group_margin;
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
            y += slot + bar.group_spacing;
        }
    }

    fn paint_horizontal(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        rect: Rect,
        input: &InputState,
    ) {
        if self.framed {
            ui::paint_frame(
                list,
                rect,
                ctx.appearance.frame_border(),
                ctx.theme.border_active,
                ctx.theme.bg_widget,
            );
        }

        let count = self.children.len();
        if count == 0 {
            return;
        }
        let bar = &ctx.metrics.bar;
        let inner = (rect.width - bar.group_margin * 2.0).max(0.0);
        let gaps = bar.group_spacing * (count.saturating_sub(1)) as f32;
        let slot = ((inner - gaps) / count as f32).max(0.0);

        let mut x = rect.x + bar.group_margin;
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
            x += slot + bar.group_spacing;
        }
    }
}

impl Widget for QuickPanel {
    fn height(&self, _width: f32, _metrics: &crate::ui::style::Metrics) -> Option<f32> {
        Some(self.height)
    }

    fn width(&self, height: f32, metrics: &crate::ui::style::Metrics) -> Option<f32> {
        let bar = &metrics.bar;
        let slot = height - bar.group_margin * 2.0;
        let count = self.children.len().max(1) as f32;
        Some(slot * count + bar.group_spacing * (count - 1.0) + bar.group_margin * 2.0)
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

    fn hover_target(
        &self,
        output: &str,
        point: crate::render::display_list::Point,
    ) -> Option<(crate::ui::hover_menu::HoverKind, Rect)> {
        self.children.iter().find_map(|child| child.hover_target(output, point))
    }

    fn tick(&mut self, dt: f32) -> bool {
        crate::ui::tick_all(&mut self.children, dt)
    }

    fn animating(&self) -> bool {
        self.children.iter().any(|child| child.animating())
    }
}
