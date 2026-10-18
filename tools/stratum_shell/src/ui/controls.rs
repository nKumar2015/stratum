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

    let track = Rect {
        x: rect.x,
        y: rect.y + rect.height * 0.5 - 3.0,
        width: rect.width,
        height: 6.0,
        radius: 3.0,
        color: ctx.theme.bg_hover,
    };
    list.push_rect(track);
    list.push_rect(Rect {
        width: rect.width * value,
        color: ctx.theme.primary,
        ..track
    });

    let knob_x = rect.x + rect.width * value;
    list.push_rect(Rect {
        x: knob_x - 7.0,
        y: rect.y + rect.height * 0.5 - 7.0,
        width: 14.0,
        height: 14.0,
        radius: 7.0,
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
        13.0,
        500,
        ctx.theme.text_main,
    );

    let sw = Rect {
        x: rect.x + rect.width - 44.0,
        y: rect.y + rect.height * 0.5 - 12.0,
        width: 44.0,
        height: 24.0,
        radius: 12.0,
        color: if on {
            ctx.theme.primary
        } else {
            ctx.theme.bg_hover
        },
    };
    list.push_rect(sw);
    let knob = if on { sw.x + 22.0 } else { sw.x + 2.0 };
    list.push_rect(Rect {
        x: knob,
        y: sw.y + 2.0,
        width: 20.0,
        height: 20.0,
        radius: 10.0,
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
        size: 13.0,
        weight: 600,
        color: text_color,
    };
    let width = ctx.text.measure(label, &style).0;
    ui::paint_text_vcentered(
        ctx,
        list,
        label,
        rect.x + (rect.width - width) * 0.5,
        rect,
        13.0,
        600,
        text_color,
    );

    let hit = input
        .pointer
        .map(|(x, y)| rect.contains(crate::render::display_list::Point::new(x, y)))
        .unwrap_or(false);
    hit && clicked
}

/// A selectable list row (devices, networks).
#[allow(clippy::too_many_arguments)]
pub fn selectable_row(
    ctx: &mut PaintCtx<'_>,
    list: &mut DisplayList,
    rect: Rect,
    title: &str,
    subtitle: &str,
    selected: bool,
    input: &InputState,
    clicked: bool,
) -> bool {
    if selected {
        list.push_rect(Rect {
            color: ctx.theme.bg_hover,
            ..rect
        });
    }
    ui::paint_text_vcentered(
        ctx,
        list,
        title,
        rect.x + 10.0,
        Rect {
            height: rect.height * 0.55,
            ..rect
        },
        13.0,
        500,
        ctx.theme.text_main,
    );
    if !subtitle.is_empty() {
        ui::paint_text_vcentered(
            ctx,
            list,
            subtitle,
            rect.x + 10.0,
            Rect {
                y: rect.y + rect.height * 0.5,
                height: rect.height * 0.5,
                ..rect
            },
            11.0,
            400,
            ctx.theme.text_muted,
        );
    }
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
