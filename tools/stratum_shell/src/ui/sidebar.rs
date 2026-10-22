//! The sidebar: widgets from the configuration, laid out on a rail.
//!
//! This module owns placement only. Each widget lives in [`super::widgets`] and
//! knows how to measure, paint, and hit-test itself, so the same widget can be
//! used in a rail, a bar, or inside a container.

use std::sync::Arc;

use crate::config::{Axis, Config, SidebarPosition, WidgetConfig, WidgetKind};
use crate::render::display_list::{Color, DisplayList, Point, Rect};
use crate::ui::widgets::{
    AppTitle, Battery, Clock, Icon, IconKind, Launcher, QuickPanel, Spacer, Tray, Workspaces,
};
use crate::ui::{self, Action, ClickCtx, InputState, PaintCtx, Widget};

/// The sidebar surface for one output.
pub struct Sidebar {
    pub config: Arc<Config>,
    pub padding: f32,
    pub spacing: f32,
    pub position: SidebarPosition,
    pub axis: Axis,
    /// Thickness across the run: rail width or bar height.
    pub cross_size: f32,
    pub inverted_corners: bool,
    widgets: Vec<Box<dyn Widget>>,
}

impl Sidebar {
    pub fn new(config: Arc<Config>) -> Self {
        let sidebar = &config.sidebar;
        let widgets = sidebar
            .widgets
            .iter()
            .map(|entry| build_widget(entry))
            .collect();

        Self {
            padding: sidebar.padding as f32,
            spacing: sidebar.spacing as f32,
            position: sidebar.position,
            axis: sidebar.axis(),
            cross_size: if sidebar.is_vertical() {
                sidebar.width as f32
            } else {
                sidebar.height as f32
            },
            inverted_corners: sidebar.inverted_corners,
            widgets,
            config,
        }
    }

    /// Paint the whole rail into `list`, given the surface height.
    pub fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        surface: Rect,
        input: &InputState,
    ) {
        // Rail background.
        list.push_rect(surface);

        match self.axis {
            Axis::Vertical => self.paint_vertical(ctx, list, surface, input),
            Axis::Horizontal => self.paint_horizontal(ctx, list, surface, input),
        }

        if self.axis == Axis::Vertical && self.inverted_corners {
            // The surface is wider than the rail by `corner` on its inner
            // side; the corners go there.
            let corner = ctx.metrics.bar.corner_size;
            let mirror = self.position != SidebarPosition::Left;
            let edge_x = if mirror {
                surface.x - corner
            } else {
                surface.x + surface.width
            };
            ui::paint_inverted_corner(
                list,
                edge_x,
                surface.y,
                corner,
                false,
                mirror,
                ctx.theme.bg_main,
            );
            ui::paint_inverted_corner(
                list,
                edge_x,
                surface.y + surface.height - corner,
                corner,
                true,
                mirror,
                ctx.theme.bg_main,
            );
        }
    }

    /// Route a click to a widget, using the same geometry as the last paint.
    ///
    /// The output name is passed through because a shared widget tree is painted
    /// on every monitor: a workspace click belongs to the monitor it landed on.
    pub fn on_click(
        &mut self,
        point: Point,
        output_name: &str,
        input: &InputState,
    ) -> Option<Action> {
        self.on_click_with(point, output_name, input, ui::MouseButton::Left)
    }

    /// Route wheel input to the widget under `point` on `output`.
    pub fn on_scroll(&mut self, point: Point, output_name: &str, delta: f32) -> Option<Action> {
        self.widgets
            .iter_mut()
            .find_map(|widget| widget.on_scroll(output_name, point, delta))
    }

    /// The hover menu the widget under `point` on `output` opens, if any.
    pub fn hover_target(
        &self,
        output: &str,
        point: Point,
    ) -> Option<(crate::ui::hover_menu::HoverKind, crate::render::display_list::Rect)> {
        self.widgets.iter().find_map(|widget| widget.hover_target(output, point))
    }

    /// Route a click with an explicit button. The tray uses right-click for its
    /// context menu.
    pub fn on_click_with(
        &mut self,
        point: Point,
        output_name: &str,
        input: &InputState,
        button: ui::MouseButton,
    ) -> Option<Action> {
        let ctx = ClickCtx {
            point,
            output_name,
            input,
            button,
        };
        self.widgets
            .iter_mut()
            .find_map(|widget| widget.on_click(&ctx))
    }
    /// Advance widget animations. Returns true while anything is moving.
    pub fn tick(&mut self, dt: f32) -> bool {
        ui::tick_all(&mut self.widgets, dt)
    }

    /// Whether any widget is currently animating.
    ///
    /// Used by the event loop to decide the frame cadence without paying for a
    /// `tick` first.
    pub fn animating(&self) -> bool {
        self.widgets.iter().any(|widget| widget.animating())
    }

    /// Classic vertical rail: widgets stacked, flexible ones absorb the slack.
    fn paint_vertical(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        surface: Rect,
        input: &InputState,
    ) {
        let content_width = surface.width;
        let top = surface.y + self.padding;
        let bottom = surface.y + surface.height - self.padding;

        // First pass: intrinsic heights. Widgets returning `None` are flexible
        // and share whatever vertical space is left, like QML's
        // `Layout.fillHeight`.
        let heights: Vec<Option<f32>> = self
            .widgets
            .iter()
            .map(|widget| widget.height(content_width, ctx.metrics))
            .collect();

        let fixed_total: f32 = heights.iter().flatten().sum();
        let gaps = self.spacing * (self.widgets.len().saturating_sub(1)) as f32;
        let flexible_count = heights.iter().filter(|h| h.is_none()).count();
        let available = (bottom - top - fixed_total - gaps).max(0.0);
        let flexible_height = if flexible_count > 0 {
            available / flexible_count as f32
        } else {
            0.0
        };

        let mut cursor = top;
        for (widget, intrinsic) in self.widgets.iter_mut().zip(heights) {
            let height = intrinsic.unwrap_or(flexible_height);
            let rect = Rect {
                x: surface.x,
                y: cursor,
                width: content_width,
                height,
                radius: 0.0,
                color: Color::TRANSPARENT,
            };
            widget.paint(ctx, list, rect, input);
            cursor += height + self.spacing;
        }
    }

    /// Horizontal bar: widgets in a row, each as wide as it needs.
    fn paint_horizontal(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        surface: Rect,
        input: &InputState,
    ) {
        let height = surface.height;
        let left = surface.x + self.padding;
        let right = surface.x + surface.width - self.padding;

        let widths: Vec<Option<f32>> = self
            .widgets
            .iter()
            .map(|widget| widget.width(height, ctx.metrics))
            .collect();

        let fixed_total: f32 = widths.iter().flatten().sum();
        let gaps = self.spacing * (self.widgets.len().saturating_sub(1)) as f32;
        let flexible_count = widths.iter().filter(|w| w.is_none()).count();
        let available = (right - left - fixed_total - gaps).max(0.0);
        let flexible_width = if flexible_count > 0 {
            available / flexible_count as f32
        } else {
            0.0
        };

        let mut cursor = left;
        for (widget, intrinsic) in self.widgets.iter_mut().zip(widths) {
            let width = intrinsic.unwrap_or(flexible_width);
            let rect = Rect {
                x: cursor,
                y: surface.y,
                width,
                height,
                radius: 0.0,
                color: Color::TRANSPARENT,
            };
            widget.paint(ctx, list, rect, input);
            cursor += width + self.spacing;
        }
    }
}

/// Build one configured widget tree.
fn build_widget(entry: &WidgetConfig) -> Box<dyn Widget> {
    match entry.kind {
        WidgetKind::Launcher => Box::new(Launcher::default()),
        WidgetKind::Workspaces => Box::new(Workspaces::new()),
        WidgetKind::AppTitle => Box::new(AppTitle::new()),
        WidgetKind::Tray => Box::new(Tray::new()),
        WidgetKind::QuickPanel => Box::new(QuickPanel::new(
            entry.framed,
            entry.height.unwrap_or(86) as f32,
            entry.children.iter().map(build_widget).collect(),
        )),
        WidgetKind::Audio => Box::new(Icon::new(IconKind::Audio)),
        WidgetKind::Bluetooth => Box::new(Icon::new(IconKind::Bluetooth)),
        WidgetKind::Wifi => Box::new(Icon::new(IconKind::Wifi)),
        WidgetKind::Battery => Box::new(Battery::new()),
        WidgetKind::Clock => Box::new(Clock::new()),
        WidgetKind::Spacer => Box::new(Spacer {
            size: entry.height.unwrap_or(12) as f32,
        }),
    }
}
