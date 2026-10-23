//! Interactive controls for the settings window.
//!
//! These are immediate-mode helpers rather than retained widgets: a page
//! computes a control's rect, paints it, and asks whether the pointer is
//! interacting. Page code stays linear and there is no second widget tree.

use crate::render::display_list::{DisplayList, Rect};
use crate::ui::{self, InputState, PaintCtx};

/// Draw a horizontal slider. Returns a new 0..=1 value while dragging.
pub fn slider(
    ctx: &mut PaintCtx<'_>,
    list: &mut DisplayList,
    rect: Rect,
    value: f32,
    dragging: bool,
    input: &InputState,
) -> Option<f32> {
    let value = value.clamp(0.0, 1.0);
    let page = &ctx.metrics.page;
    let track_height = page.slider_track;
    let knob = page.slider_knob;

    let track = Rect {
        x: rect.x,
        y: rect.y + rect.height * 0.5 - track_height * 0.5,
        width: rect.width,
        height: track_height,
        radius: track_height * 0.5,
        // The top of the surface ramp, so the track shows on a card too.
        color: ctx.theme.surface_highest,
    };
    list.push_rect(track);
    list.push_rect(Rect {
        width: rect.width * value,
        color: ctx.theme.primary,
        ..track
    });

    let knob_x = rect.x + rect.width * value;
    list.push_rect(Rect {
        x: knob_x - knob * 0.5,
        y: rect.y + rect.height * 0.5 - knob * 0.5,
        width: knob,
        height: knob,
        radius: knob * 0.5,
        color: ctx.theme.text_main,
    });

    if !dragging {
        return None;
    }
    let (px, _) = input.pointer?;
    Some(((px - rect.x) / rect.width.max(1.0)).clamp(0.0, 1.0))
}

/// Draw a labelled switch. Returns the new state when it is clicked.
pub fn toggle(
    ctx: &mut PaintCtx<'_>,
    list: &mut DisplayList,
    rect: Rect,
    label: &str,
    on: bool,
    input: &InputState,
    clicked: bool,
) -> Option<bool> {
    ui::paint_text_vcentered(
        ctx,
        list,
        label,
        rect.x,
        rect,
        ctx.metrics.typography.body,
        ctx.metrics.typography.weight_medium,
        ctx.theme.text_main,
    );

    let page = &ctx.metrics.page;
    let width = page.toggle_width;
    let height = page.toggle_height;
    let inset = 2.0;
    let knob_size = height - inset * 2.0;
    let sw = Rect {
        x: rect.x + rect.width - width,
        y: rect.y + rect.height * 0.5 - height * 0.5,
        width,
        height,
        radius: height * 0.5,
        color: if on {
            ctx.theme.primary
        } else {
            ctx.theme.bg_hover
        },
    };
    list.push_rect(sw);
    let knob_x = if on {
        sw.x + width - knob_size - inset
    } else {
        sw.x + inset
    };
    list.push_rect(Rect {
        x: knob_x,
        y: sw.y + inset,
        width: knob_size,
        height: knob_size,
        radius: knob_size * 0.5,
        color: ctx.theme.text_main,
    });

    let hit = input
        .pointer
        .map(|(x, y)| sw.contains(crate::render::display_list::Point::new(x, y)))
        .unwrap_or(false);
    (hit && clicked).then_some(!on)
}

/// Draw a full-width button. Returns true when clicked.
pub fn button(
    ctx: &mut PaintCtx<'_>,
    list: &mut DisplayList,
    rect: Rect,
    label: &str,
    input: &InputState,
    clicked: bool,
    accent: bool,
) -> bool {
    let fill = if accent {
        ctx.theme.primary
    } else {
        ctx.theme.bg_widget
    };
    list.push_rect(Rect { color: fill, ..rect });

    let text_color = if accent {
        ctx.theme.bg_main
    } else {
        ctx.theme.text_main
    };
    let style = crate::text::TextStyle {
        family: std::sync::Arc::from(ctx.theme.font.as_str()),
        size: ctx.metrics.typography.body,
        weight: ctx.metrics.typography.weight_bold,
        color: text_color,
    };
    let width = ctx.text.measure(label, &style).0;
    ui::paint_text_vcentered(
        ctx,
        list,
        label,
        rect.x + (rect.width - width) * 0.5,
        rect,
        ctx.metrics.typography.body,
        ctx.metrics.typography.weight_bold,
        text_color,
    );

    let hit = input
        .pointer
        .map(|(x, y)| rect.contains(crate::render::display_list::Point::new(x, y)))
        .unwrap_or(false);
    hit && clicked
}

/// Convert a pointer position inside `rect` to a 0..=1 fraction along x.
pub fn fraction_in(rect: Rect, x: f32) -> f32 {
    ((x - rect.x) / rect.width.max(1.0)).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Rect {
        Rect {
            x: 100.0,
            y: 0.0,
            width: 200.0,
            height: 20.0,
            radius: 0.0,
            color: crate::render::display_list::Color::TRANSPARENT,
        }
    }

    #[test]
    fn fraction_maps_and_clamps() {
        let r = rect();
        assert!((fraction_in(r, 100.0) - 0.0).abs() < 1e-6);
        assert!((fraction_in(r, 200.0) - 0.5).abs() < 1e-6);
        assert!((fraction_in(r, 400.0) - 1.0).abs() < 1e-6);
        assert!((fraction_in(r, 0.0) - 0.0).abs() < 1e-6);
    }
}
