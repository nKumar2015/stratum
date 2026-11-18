//! Interactive controls for the settings window.
//!
//! These are immediate-mode helpers rather than retained widgets: a page
//! computes a control's rect, paints it, and asks whether the pointer is
//! interacting. Page code stays linear and there is no second widget tree.

use crate::render::display_list::{DisplayList, Rect};
use crate::ui::{InputState, PaintCtx};

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
    Some(fraction_in(rect, px))
}

/// A slider that only stops at `count` evenly spaced steps, each marked on
/// the track. Returns the step nearest the pointer while dragging. With a
/// single step there is nothing to choose and it never returns one.
pub fn stepped_slider(
    ctx: &mut PaintCtx<'_>,
    list: &mut DisplayList,
    rect: Rect,
    index: usize,
    count: usize,
    dragging: bool,
    input: &InputState,
) -> Option<usize> {
    let page = &ctx.metrics.page;
    let track_height = page.slider_track;
    let knob = page.slider_knob;
    let last = count.saturating_sub(1).max(1) as f32;
    let at = |i: usize| rect.x + rect.width * (i as f32 / last).min(1.0);
    let cy = rect.y + rect.height * 0.5;

    let track = Rect {
        x: rect.x,
        y: cy - track_height * 0.5,
        width: rect.width,
        height: track_height,
        radius: track_height * 0.5,
        color: ctx.theme.surface_highest,
    };
    list.push_rect(track);
    list.push_rect(Rect { width: at(index) - rect.x, color: ctx.theme.primary, ..track });
    // A mark per step, in the colour that reads on the part of the track
    // it sits on.
    let dot = track_height * 0.5;
    for i in 0..count {
        let color = if i <= index { ctx.theme.on_primary } else { ctx.theme.text_muted };
        list.push_rect(Rect { x: at(i) - dot * 0.5, y: cy - dot * 0.5, width: dot, height: dot, radius: dot * 0.5, color });
    }
    list.push_rect(Rect {
        x: at(index) - knob * 0.5,
        y: cy - knob * 0.5,
        width: knob,
        height: knob,
        radius: knob * 0.5,
        color: ctx.theme.text_main,
    });

    if !dragging || count < 2 {
        return None;
    }
    let (px, _) = input.pointer?;
    Some((fraction_in(rect, px) * last).round() as usize)
}

/// A switch, its top-left at `(x, y)`. Returns the new state when it is
/// clicked.
pub fn switch(
    ctx: &mut PaintCtx<'_>,
    list: &mut DisplayList,
    x: f32,
    y: f32,
    on: bool,
    input: &InputState,
    clicked: bool,
) -> Option<bool> {
    let page = &ctx.metrics.page;
    let width = page.toggle_width;
    let height = page.toggle_height;
    let inset = 2.0;
    let knob_size = height - inset * 2.0;
    let sw = Rect {
        x,
        y,
        width,
        height,
        radius: height * 0.5,
        color: if on {
            ctx.theme.primary
        } else {
            ctx.theme.surface_highest
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
