//! The launcher widget.

use crate::render::display_list::{DisplayList, Rect};
use crate::ui::control_center::Page;
use crate::ui::{self, Action, ClickCtx, InputState, PaintCtx, Widget};

/// Launcher glyph, opening the control centre overview.
#[derive(Default)]
pub struct Launcher {
    /// Where the glyph was last painted. Needed for hit-testing: returning an
    /// action for every click made the launcher swallow the whole rail, so
    /// clicking the tray opened the settings window instead.
    rect: Option<Rect>,
}

impl Widget for Launcher {
    fn height(&self, _width: f32, metrics: &crate::ui::style::Metrics) -> Option<f32> {
        Some(metrics.bar.launcher)
    }

    fn width(&self, height: f32, metrics: &crate::ui::style::Metrics) -> Option<f32> {
        Some(height.max(metrics.bar.launcher))
    }

    fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        rect: Rect,
        _input: &InputState,
    ) {
        ui::paint_icon_centered(ctx, list, "\u{f313}", rect, ctx.metrics.typography.icon, ctx.theme.primary);
        self.rect = Some(rect);
    }

    fn on_click(&mut self, ctx: &ClickCtx<'_>) -> Option<Action> {
        let rect = self.rect?;
        rect.contains(ctx.point)
            .then_some(Action::OpenPanel(Page::Overview))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::display_list::{Color, Point};

    fn ctx(point: Point) -> ClickCtx<'static> {
        ClickCtx {
            point,
            output_name: "test",
            button: crate::ui::MouseButton::Left,
        }
    }

    #[test]
    fn clicks_outside_the_glyph_are_ignored() {
        let mut launcher = Launcher::default();
        // Before the first paint there is no geometry, so nothing is clickable.
        assert_eq!(launcher.on_click(&ctx(Point::new(5.0, 5.0))), None);

        launcher.rect = Some(Rect {
            x: 0.0,
            y: 0.0,
            width: 24.0,
            height: 24.0,
            radius: 0.0,
            color: Color::TRANSPARENT,
        });
        assert_eq!(
            launcher.on_click(&ctx(Point::new(12.0, 12.0))),
            Some(Action::OpenPanel(Page::Overview))
        );
        // This is the bug that made every rail click open the settings window.
        assert_eq!(launcher.on_click(&ctx(Point::new(12.0, 400.0))), None);
    }
}
