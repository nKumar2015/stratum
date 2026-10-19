//! The scrolling workspace strip.
//!
//! Hyprland numbers workspaces globally, but the bar shows them in pages of ten
//! with a five-row window that follows the active one. Each monitor has its own
//! active workspace, so the widget reads that from the per-output map rather
//! than from whatever output currently has focus.
//!
//! The window scroll animates: changing workspace moves the strip rather than
//! snapping, matching the QML shell's `OutBack` `NumberAnimation`.

use crate::config::Axis;
use crate::render::display_list::{Color, DisplayList, Rect};
use crate::ui::animation::{Easing, Tween};
use crate::ui::{self, Action, ClickCtx, InputState, PaintCtx, Widget};

/// Workspaces per page. Hyprland's ids are global, so a page is a fixed range
/// of ten rather than a contiguous allocation per monitor.
const PAGE: i32 = 10;

/// Rows visible in the window.
const VISIBLE: usize = 5;

/// Row height and gap, matching the QML `itemHeight`/`itemSpacing`.
const ROW_HEIGHT: f32 = 20.0;
const ROW_SPACING: f32 = 3.0;
const STEP: f32 = ROW_HEIGHT + ROW_SPACING;

/// How long the scroll takes. The QML shell used 400 ms with `OutBack`.
const SCROLL_SECONDS: f32 = 0.4;

/// One monitor's view of the strip.
///
/// The widget tree is shared by every bar surface, so all scrolling state has to
/// be keyed by output: a single `Tween` would be reset every time the next
/// monitor painted, and neither strip would ever animate.
#[derive(Default)]
struct OutputView {
    scroll: Tween,
    target_start: f32,
    /// Workspace the user last clicked, shown as active until the compositor
    /// catches up.
    pending: Option<i32>,
    cells: Vec<(Rect, i32)>,
    /// False until the first paint has seeded `scroll`.
    initialised: bool,
}

pub struct Workspaces {
    /// Per-output scroll and hit-test state, keyed by output name.
    views: std::collections::HashMap<String, OutputView>,
}

impl Workspaces {
    pub fn new() -> Self {
        Self {
            views: std::collections::HashMap::new(),
        }
    }

    /// Ids on the page containing `active`, and the index of `active` within it.
    fn page(active: i32) -> (Vec<i32>, usize) {
        if active <= 0 {
            return ((1..=PAGE).collect(), 0);
        }
        let start = ((active - 1) / PAGE) * PAGE + 1;
        let ids: Vec<i32> = (0..PAGE).map(|offset| start + offset).collect();
        let index = ids
            .iter()
            .position(|id| *id == active)
            // An active workspace outside the page (a named workspace, say)
            // should still land somewhere sane rather than showing row zero.
            .unwrap_or(0);
        (ids, index)
    }

    /// First visible row for `active`, biased two rows above centre.
    fn window_start(active: i32) -> f32 {
        let (ids, index) = Self::page(active);
        let max_start = ids.len().saturating_sub(VISIBLE);
        index.saturating_sub(2).min(max_start) as f32
    }

    /// Workspace id to treat as active for `output`.
    fn active_for(view: &OutputView, hypr: &crate::hyprland::HyprState, output: &str) -> i32 {
        let real = hypr.active.get(output).copied().unwrap_or(-1);
        match view.pending {
            // A click is a request; show it until the compositor agrees or the
            // state moves somewhere else.
            Some(id) if real != id => id,
            _ => real,
        }
    }
}

impl Default for Workspaces {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for Workspaces {
    fn height(&self, width: f32) -> Option<f32> {
        let _ = width;
        Some(VISIBLE as f32 * STEP - ROW_SPACING)
    }

    fn width(&self, height: f32) -> Option<f32> {
        // Five side-by-side cells in a horizontal bar.
        Some(VISIBLE as f32 * (height.min(ROW_HEIGHT) + 6.0))
    }

    fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        rect: Rect,
        _input: &InputState,
    ) {
        let hypr = ctx.state.hypr.get();
        let output = ctx.output_name.to_string();
        let view = self.views.entry(output.clone()).or_default();

        // First paint for this output: adopt the current window without
        // animating in from nowhere.
        if !view.initialised {
            view.initialised = true;
            view.scroll = Tween::new(0.0);
            view.scroll.snap(Self::window_start(
                hypr.active.get(&output).copied().unwrap_or(1),
            ));
        }

        let active = Self::active_for(view, &hypr, &output);
        // The compositor caught up: drop the optimistic value.
        if let Some(pending) = view.pending {
            if hypr.active.get(&output).copied() == Some(pending) {
                view.pending = None;
            }
        }

        let start = Self::window_start(active);
        if (start - view.target_start).abs() > f32::EPSILON {
            view.target_start = start;
            view.scroll.to(start.max(0.0), SCROLL_SECONDS, Easing::OutBack);
        }

        let (ids, _) = Self::page(active);
        let vertical = matches!(ctx.axis, Axis::Vertical);
        let offset = view.scroll.value();

        view.cells.clear();

        // Clip so rows outside the window never bleed into neighbouring
        // widgets; the scrolled content is taller than the widget.
        list.push_clip(rect);

        let origin = if vertical { rect.y } else { rect.x };
        let cross = if vertical { rect.x } else { rect.y };
        let extent = if vertical { rect.height } else { rect.width };
        let cell_length = if vertical { rect.width } else { extent / VISIBLE as f32 };

        for (index, id) in ids.iter().enumerate() {
            let along = origin + (index as f32 - offset) * STEP;
            let row = if vertical {
                Rect {
                    x: cross,
                    y: along,
                    width: cell_length,
                    height: ROW_HEIGHT,
                    radius: 0.0,
                    color: Color::TRANSPARENT,
                }
            } else {
                Rect {
                    x: along,
                    y: cross,
                    width: STEP,
                    height: rect.height,
                    radius: 0.0,
                    color: Color::TRANSPARENT,
                }
            };
            // Cull rows entirely outside the window; they cannot be drawn and
            // would otherwise widen `cells` without being clickable.
            if along + STEP < origin - STEP || along > origin + extent + STEP {
                continue;
            }

            let is_active = *id == active;
            let occupied = hypr.occupied.contains(id);
            let color = if is_active {
                ctx.theme.primary
            } else if occupied {
                ctx.theme.secondary
            } else {
                ctx.theme.text_muted
            };
            ui::paint_text_centered(ctx, list, &id.to_string(), row, 15.0, color);

            // Active marker: a short bar under the number, as in QML.
            if is_active {
                list.push_rect(Rect {
                    x: row.x + (row.width - 14.0) * 0.5,
                    y: row.y + row.height - 3.0,
                    width: 14.0,
                    height: 2.0,
                    radius: 1.0,
                    color: ctx.theme.primary,
                });
            }

            view.cells.push((row, *id));
        }

        list.pop_clip();
    }

    fn on_click(&mut self, ctx: &ClickCtx<'_>) -> Option<Action> {
        let view = self.views.get_mut(ctx.output_name)?;
        let id = view
            .cells
            .iter()
            .find(|(rect, _)| rect.contains(ctx.point))
            .map(|(_, id)| *id)?;
        // Show the request immediately; the compositor event will confirm it a
        // few milliseconds later and `paint` will reconcile.
        view.pending = Some(id);
        Some(Action::FocusWorkspace(id))
    }

    fn tick(&mut self, dt: f32) -> bool {
        let mut animating = false;
        for view in self.views.values_mut() {
            animating |= view.scroll.tick(dt);
        }
        animating
    }

    fn animating(&self) -> bool {
        self.views
            .values()
            .any(|view| view.scroll.is_animating())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_is_ten_ids_containing_active() {
        let (ids, index) = Workspaces::page(1);
        assert_eq!(ids, (1..=10).collect::<Vec<_>>());
        assert_eq!(index, 0);

        let (ids, index) = Workspaces::page(11);
        assert_eq!(ids, (11..=20).collect::<Vec<_>>());
        assert_eq!(index, 0);
    }

    #[test]
    fn window_keeps_active_two_rows_above_centre() {
        // The window starts at `index - 2`, clamped to the page bounds.
        // Row 1 is the top of the page; the window cannot scroll above it.
        assert_eq!(Workspaces::window_start(1), 0.0);
        assert_eq!(Workspaces::window_start(2), 0.0);
        assert_eq!(Workspaces::window_start(3), 0.0);
        assert_eq!(Workspaces::window_start(5), 2.0);
        // Near the bottom the window stops rather than scrolling past the page.
        assert_eq!(Workspaces::window_start(10), 5.0);
        // The next page starts its own window at zero again.
        assert_eq!(Workspaces::window_start(11), 0.0);
    }

    #[test]
    fn scroll_tween_settles_on_the_target() {
        let mut tween = Tween::new(0.0);
        tween.to(3.0, SCROLL_SECONDS, Easing::OutBack);
        for _ in 0..80 {
            tween.tick(0.016);
        }
        assert!((tween.value() - 3.0).abs() < 1e-3);
    }
}
