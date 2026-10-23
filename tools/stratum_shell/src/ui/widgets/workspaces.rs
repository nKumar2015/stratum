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

/// Length of the scroll animation, from the configured duration.
fn scroll_seconds(metrics: &crate::ui::style::WorkspacesMetrics) -> f32 {
    metrics.scroll_ms as f32 / 1000.0
}

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
    /// The widget's rect, the workspace shown as active, and its page at the
    /// last paint, for wheel input.
    rect: Option<Rect>,
    active: i32,
    page: Vec<i32>,
    /// Wheel travel not yet turned into a switch. Touchpads send many small
    /// deltas; one switch per notch's worth keeps them from racing ahead.
    wheel: f32,
    /// False until the first paint has seeded `scroll`.
    initialised: bool,
}

/// Wheel travel per workspace step: one notch of a mouse wheel.
const WHEEL_NOTCH: f32 = 120.0;

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
    fn page(active: i32, metrics: &crate::ui::style::WorkspacesMetrics) -> (Vec<i32>, usize) {
        let size = metrics.page_size.max(1);
        if active <= 0 {
            return ((1..=size).collect(), 0);
        }
        let start = ((active - 1) / size) * size + 1;
        let ids: Vec<i32> = (0..size).map(|offset| start + offset).collect();
        let index = ids
            .iter()
            .position(|id| *id == active)
            // An active workspace outside the page (a named workspace, say)
            // should still land somewhere sane rather than showing row zero.
            .unwrap_or(0);
        (ids, index)
    }

    /// First visible row for `active`, biased above centre.
    fn window_start(active: i32, metrics: &crate::ui::style::WorkspacesMetrics) -> f32 {
        let (ids, index) = Self::page(active, metrics);
        let max_start = ids.len().saturating_sub(metrics.visible);
        index.saturating_sub(metrics.bias).min(max_start) as f32
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

    /// The workspace one wheel step from `active` within the page `ids`: QML's
    /// wheel stopped at the first and last ids rather than wrapping or paging.
    fn wheel_target(ids: &[i32], active: i32, down: bool) -> Option<i32> {
        let index = ids.iter().position(|id| *id == active)?;
        let next = if down {
            ids.get(index + 1)
        } else {
            index.checked_sub(1).and_then(|i| ids.get(i))
        };
        next.copied()
    }
}

impl Default for Workspaces {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for Workspaces {
    fn height(&self, width: f32, metrics: &crate::ui::style::Metrics) -> Option<f32> {
        let _ = width;
        Some(metrics.workspaces.window_height())
    }

    fn width(&self, height: f32, metrics: &crate::ui::style::Metrics) -> Option<f32> {
        let ws = &metrics.workspaces;
        // Side-by-side cells in a horizontal bar.
        Some(ws.visible as f32 * ws.cell_pitch.min(height.max(ws.row_height)))
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
        let ws = &ctx.metrics.workspaces;

        // First paint for this output: adopt the current window without
        // animating in from nowhere.
        if !view.initialised {
            view.initialised = true;
            view.scroll = Tween::new(0.0);
            view.scroll.snap(Self::window_start(
                hypr.active.get(&output).copied().unwrap_or(1),
                ws,
            ));
        }

        let active = Self::active_for(view, &hypr, &output);
        view.rect = Some(rect);
        view.active = active;
        // The compositor caught up: drop the optimistic value.
        if let Some(pending) = view.pending {
            if hypr.active.get(&output).copied() == Some(pending) {
                view.pending = None;
            }
        }

        let start = Self::window_start(active, ws);
        if (start - view.target_start).abs() > f32::EPSILON {
            view.target_start = start;
            view.scroll
                .to(start.max(0.0), scroll_seconds(ws), Easing::OutBack);
        }

        let (ids, _) = Self::page(active, ws);
        view.page.clone_from(&ids);
        let vertical = matches!(ctx.axis, Axis::Vertical);
        let offset = view.scroll.value();

        view.cells.clear();

        // Clip so rows outside the window never bleed into neighbouring
        // widgets; the scrolled content is taller than the widget.
        list.push_clip(rect);

        let origin = if vertical { rect.y } else { rect.x };
        let cross = if vertical { rect.x } else { rect.y };
        let extent = if vertical { rect.height } else { rect.width };
        let cell_length = if vertical {
            rect.width
        } else {
            extent / ws.visible.max(1) as f32
        };

        for (index, id) in ids.iter().enumerate() {
            let along = origin + (index as f32 - offset) * ws.step();
            let row = if vertical {
                Rect {
                    x: cross,
                    y: along,
                    width: cell_length,
                    height: ws.row_height,
                    radius: 0.0,
                    color: Color::TRANSPARENT,
                }
            } else {
                Rect {
                    x: along,
                    y: cross,
                    width: ws.step(),
                    height: rect.height,
                    radius: 0.0,
                    color: Color::TRANSPARENT,
                }
            };
            // Cull rows entirely outside the window; they cannot be drawn and
            // would otherwise widen `cells` without being clickable.
            if along + ws.step() < origin - ws.step() || along > origin + extent + ws.step() {
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
            ui::paint_text_centered_weighted(
                ctx,
                list,
                &id.to_string(),
                row,
                ctx.metrics.typography.workspace,
                ctx.metrics.typography.weight_bold,
                color,
            );

            // Active marker: a short bar under the number, as in QML.
            if is_active {
                list.push_rect(Rect {
                    x: row.x + (row.width - ws.marker_width) * 0.5,
                    y: row.y + row.height - ws.marker_height,
                    width: ws.marker_width,
                    height: ws.marker_height,
                    radius: ws.marker_height * 0.5,
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

    fn on_scroll(
        &mut self,
        output: &str,
        point: crate::render::display_list::Point,
        delta: f32,
    ) -> Option<Action> {
        let view = self.views.get_mut(output)?;
        if !view.rect.is_some_and(|r| r.contains(point)) {
            return None;
        }
        // A reversal starts afresh rather than first undoing the old travel.
        if view.wheel.signum() != delta.signum() {
            view.wheel = 0.0;
        }
        view.wheel += delta;
        if view.wheel.abs() < WHEEL_NOTCH {
            return None;
        }
        let down = view.wheel > 0.0;
        view.wheel = 0.0;
        // `pending` is folded into `active` at paint, so repeated notches
        // before the compositor answers keep stepping.
        let id = Self::wheel_target(&view.page, view.active, down)?;
        view.pending = Some(id);
        view.active = id;
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
        let m = crate::config::Config::test_defaults().metrics.workspaces;
        let (ids, index) = Workspaces::page(1, &m);
        assert_eq!(ids, (1..=10).collect::<Vec<_>>());
        assert_eq!(index, 0);

        let (ids, index) = Workspaces::page(11, &m);
        assert_eq!(ids, (11..=20).collect::<Vec<_>>());
        assert_eq!(index, 0);
    }

    #[test]
    fn window_keeps_active_two_rows_above_centre() {
        let m = crate::config::Config::test_defaults().metrics.workspaces;
        // The window starts at `index - bias`, clamped to the page bounds.
        // Row 1 is the top of the page; the window cannot scroll above it.
        assert_eq!(Workspaces::window_start(1, &m), 0.0);
        assert_eq!(Workspaces::window_start(2, &m), 0.0);
        assert_eq!(Workspaces::window_start(3, &m), 0.0);
        assert_eq!(Workspaces::window_start(5, &m), 2.0);
        // Near the bottom the window stops rather than scrolling past the page.
        assert_eq!(Workspaces::window_start(10, &m), 5.0);
        // The next page starts its own window at zero again.
        assert_eq!(Workspaces::window_start(11, &m), 0.0);
    }

    #[test]
    fn window_follows_configured_metrics() {
        // A seven-row window with a bias of three places the active row
        // accordingly, so the metrics table really drives layout.
        let m = crate::ui::style::WorkspacesMetrics {
            visible: 7,
            bias: 3,
            ..crate::config::Config::test_defaults().metrics.workspaces
        };
        // Active id 6 has index 5; biased three rows above gives 2.
        assert_eq!(Workspaces::window_start(6, &m), 2.0);
        // Ten ids with seven visible leaves three rows of slack, so the last
        // row can still sit at the bottom of the window.
        assert_eq!(Workspaces::window_start(10, &m), 3.0);
    }

    #[test]
    fn scroll_tween_settles_on_the_target() {
        let mut tween = Tween::new(0.0);
        tween.to(3.0, scroll_seconds(&crate::config::Config::test_defaults().metrics.workspaces), Easing::OutBack);
        for _ in 0..80 {
            tween.tick(0.016);
        }
        assert!((tween.value() - 3.0).abs() < 1e-3);
    }

    /// The bug behind "they are not monitor specific": every surface read the
    /// focused monitor's workspace, so both rails showed the same page.
    #[test]
    fn each_output_shows_its_own_active_workspace() {
        let mut hypr = crate::hyprland::HyprState::default();
        hypr.active.insert("eDP-2".into(), 1);
        hypr.active.insert("HDMI-A-1".into(), 11);

        let m = crate::config::Config::test_defaults().metrics.workspaces;
        let left = hypr.active.get("eDP-2").copied().unwrap_or(-1);
        let right = hypr.active.get("HDMI-A-1").copied().unwrap_or(-1);
        assert_eq!(Workspaces::page(left, &m).0, (1..=10).collect::<Vec<_>>());
        assert_eq!(Workspaces::page(right, &m).0, (11..=20).collect::<Vec<_>>());
        assert_ne!(left, right, "monitors must not share an active workspace");
    }

    /// A click must be visible before the compositor's event arrives, which is
    /// what "when I click the number it changes, but the widget doesn't update"
    /// was describing.
    #[test]
    fn pending_click_overrides_the_compositor_until_it_agrees() {
        let view = OutputView {
            pending: Some(7),
            ..Default::default()
        };
        let mut hypr = crate::hyprland::HyprState::default();
        hypr.active.insert("eDP-2".into(), 3);

        // The compositor has not caught up yet, so the request is shown.
        assert_eq!(Workspaces::active_for(&view, &hypr, "eDP-2"), 7);

        // Once it agrees, painting picks up the real value.
        hypr.active.insert("eDP-2".into(), 7);
        assert_eq!(Workspaces::active_for(&view, &hypr, "eDP-2"), 7);
    }

    #[test]
    fn wheel_steps_within_the_page_and_stops_at_its_ends() {
        let ids: Vec<i32> = (11..=20).collect();
        assert_eq!(Workspaces::wheel_target(&ids, 14, true), Some(15));
        assert_eq!(Workspaces::wheel_target(&ids, 14, false), Some(13));
        assert_eq!(Workspaces::wheel_target(&ids, 20, true), None);
        assert_eq!(Workspaces::wheel_target(&ids, 11, false), None);
        // A named workspace is not on the page; the wheel does nothing.
        assert_eq!(Workspaces::wheel_target(&ids, -98, true), None);
    }

    #[test]
    fn wheel_switches_once_per_notch() {
        let mut widget = Workspaces::new();
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            width: 40.0,
            height: 115.0,
            radius: 0.0,
            color: Color::TRANSPARENT,
        };
        widget.views.insert(
            "eDP-2".into(),
            OutputView {
                rect: Some(rect),
                active: 3,
                page: (1..=10).collect(),
                ..Default::default()
            },
        );
        let at = crate::render::display_list::Point::new(10.0, 10.0);

        // Touchpad-sized deltas accumulate to one notch before switching.
        assert!(widget.on_scroll("eDP-2", at, 60.0).is_none());
        assert!(matches!(widget.on_scroll("eDP-2", at, 60.0), Some(Action::FocusWorkspace(4))));
        // Another notch steps again before the compositor has answered.
        assert!(matches!(widget.on_scroll("eDP-2", at, 120.0), Some(Action::FocusWorkspace(5))));
        assert!(matches!(widget.on_scroll("eDP-2", at, -120.0), Some(Action::FocusWorkspace(4))));
        // Outside the strip, or on another output, nothing happens.
        let outside = crate::render::display_list::Point::new(10.0, 200.0);
        assert!(widget.on_scroll("eDP-2", outside, 120.0).is_none());
        assert!(widget.on_scroll("HDMI-A-1", at, 120.0).is_none());
    }

    #[test]
    fn scroll_state_is_independent_per_output() {
        let mut widget = Workspaces::new();
        let mut left = OutputView::default();
        let mut right = OutputView::default();
        left.scroll.snap(5.0);
        right.scroll.snap(0.0);
        widget.views.insert("eDP-2".into(), left);
        widget.views.insert("HDMI-A-1".into(), right);

        assert_eq!(widget.views["eDP-2"].scroll.value(), 5.0);
        assert_eq!(widget.views["HDMI-A-1"].scroll.value(), 0.0);
    }
}
