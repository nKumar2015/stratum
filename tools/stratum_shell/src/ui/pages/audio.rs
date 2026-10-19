//! Audio page: volume, mute, and output selection.
//!
//! The slider is optimistic: dragging writes the pointer value straight into the
//! page so the knob tracks the cursor at frame rate, while the volume change is
//! sent to the worker. Without this the knob would follow the system snapshot,
//! which arrives a frame or two late and makes the slider feel sticky.

use crate::actions::SystemAction;
use crate::render::display_list::{Color, DisplayList, Rect};
use crate::ui::controls;
use crate::ui::pages::{heading, section_label, Optimistic};
use crate::ui::{self, InputState, PaintCtx};

/// Commits the slider sends per drag are throttled to one percent steps.
pub struct AudioPage {
    dragging: bool,
    /// Value the pointer is currently asking for.
    drag_value: Option<f32>,
    muted: Optimistic<bool>,
}

impl AudioPage {
    pub fn new() -> Self {
        Self {
            dragging: false,
            drag_value: None,
            muted: Optimistic::new(),
        }
    }

    pub fn tick(&mut self, dt: f32) {
        self.muted.tick(dt);
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
        let area = heading(ctx, list, area, "Audio");

        let audio = ctx.state.audio.get();
        let real_muted = audio["mute"].as_str() == Some("yes");
        let muted = self.muted.view(real_muted);
        let real_volume = audio["volume"]
            .as_str()
            .and_then(|v| v.trim_end_matches('%').parse::<f32>().ok())
            .unwrap_or(0.0);

        let slider_rect = Rect {
            x: area.x,
            y: area.y + 4.0,
            width: area.width,
            height: 24.0,
            radius: 0.0,
            color: Color::TRANSPARENT,
        };
        let pointer_inside = input
            .pointer
            .map(|(x, y)| slider_rect.contains(crate::render::display_list::Point::new(x, y)))
            .unwrap_or(false);

        if input.pressed && pointer_inside && !self.dragging {
            self.dragging = true;
        } else if !input.pressed {
            self.dragging = false;
            self.drag_value = None;
        }

        // While dragging the knob follows the pointer, not the (lagging)
        // snapshot. `value` is the fraction the slider draws.
        let fraction = if let Some(drag) = self.drag_value {
            drag
        } else {
            (real_volume / 100.0).clamp(0.0, 1.5) / 1.5
        };

        if let Some(new_fraction) =
            controls::slider(ctx, list, slider_rect, fraction, self.dragging, input)
        {
            self.drag_value = Some(new_fraction);
            // The slider covers 0..150%; convert back to a percentage.
            let percent = (new_fraction * 150.0).round() as i64;
            let shown = (real_volume.round() as i64).max(0);
            if percent != shown {
                pending.push(SystemAction::SetVolume(percent));
            }
        }

        let volume_text = if muted {
            "Muted".to_string()
        } else {
            format!("{}%", (fraction * 150.0).round() as i64)
        };
        ui::paint_text_vcentered(
            ctx,
            list,
            &volume_text,
            area.x + area.width - 56.0,
            Rect {
                y: area.y + 30.0,
                height: 18.0,
                ..area
            },
            12.0,
            500,
            ctx.theme.text_muted,
        );

        let mute_rect = Rect {
            x: area.x,
            y: area.y + 56.0,
            width: area.width,
            height: 30.0,
            radius: 0.0,
            color: Color::TRANSPARENT,
        };
        if let Some(next) = controls::toggle(ctx, list, mute_rect, "Muted", muted, input, clicked) {
            self.muted.set(next);
            pending.push(SystemAction::SetMuted(next));
        }

        let mut y = area.y + 100.0;
        section_label(ctx, list, area, y, "Outputs");
        y += 24.0;

        let current_output = audio["default_sink"]
            .as_str()
            .or_else(|| audio["default"]["sink"].as_str())
            .unwrap_or("")
            .to_string();

        if let Some(sinks) = audio["sinks"].as_array() {
            for sink in sinks.iter().take(6) {
                let name = sink["name"].as_str().unwrap_or("");
                let label = sink["description"].as_str().unwrap_or(name);
                let row = Rect {
                    x: area.x,
                    y,
                    width: area.width,
                    height: 40.0,
                    radius: 8.0,
                    color: Color::TRANSPARENT,
                };
                let selected = name == current_output;
                if controls::selectable_row(
                    ctx, list, row, label, name, selected, input, clicked,
                ) && !selected
                {
                    pending.push(SystemAction::SetOutput(name.to_string()));
                }
                y += 44.0;
            }
        }
    }
}

impl Default for AudioPage {
    fn default() -> Self {
        Self::new()
    }
}
