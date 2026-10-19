//! Fixed blank space.

use crate::render::display_list::{DisplayList, Rect};
use crate::ui::{InputState, PaintCtx, Widget};

/// Fixed blank space for grouping widgets.
pub struct Spacer {
    pub size: f32,
}

impl Widget for Spacer {
    fn height(&self, _width: f32) -> Option<f32> {
        Some(self.size)
    }

    fn width(&self, _height: f32) -> Option<f32> {
        Some(self.size)
    }

    fn paint(
        &mut self,
        _ctx: &mut PaintCtx<'_>,
        _list: &mut DisplayList,
        _rect: Rect,
        _input: &InputState,
    ) {
    }
}
