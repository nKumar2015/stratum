//! The launcher widget.

use crate::render::display_list::{DisplayList, Rect};
use crate::ui::control_center::Page;
use crate::ui::{self, Action, ClickCtx, InputState, PaintCtx, Widget};

/// Launcher glyph, opening the control centre overview.
pub struct Launcher;

impl Widget for Launcher {
    fn height(&self, _width: f32) -> Option<f32> {
        Some(24.0)
    }

    fn width(&self, height: f32) -> Option<f32> {
        Some(height.max(24.0))
    }

    fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        rect: Rect,
        _input: &InputState,
    ) {
        ui::paint_text_centered(ctx, list, "\u{f313}", rect, 20.0, ctx.theme.primary);
    }

    fn on_click(&mut self, _ctx: &ClickCtx<'_>) -> Option<Action> {
        Some(Action::OpenPanel(Page::Overview))
    }
}
