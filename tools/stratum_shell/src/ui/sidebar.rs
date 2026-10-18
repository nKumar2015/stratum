//! The sidebar: widgets from the configuration, laid out on a rail.

use std::sync::Arc;

use crate::config::{Axis, Config, SidebarPosition, WidgetConfig, WidgetKind};
use crate::render::display_list::{Color, DisplayList, Rect, TextRun};
use crate::sys_state::SystemState;
use crate::ui::{self, InputState, PaintCtx, Widget};

/// The sidebar surface for one output.
pub struct Sidebar {
    pub config: Arc<Config>,
    pub width: f32,
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
            .map(|entry| build_widget(entry, Arc::clone(&config)))
            .collect();

        Self {
            width: sidebar.width as f32,
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
            let edge_x = match self.position {
                SidebarPosition::Left => surface.x + surface.width,
                _ => surface.x - 20.0,
            };
            ui::paint_inverted_corner(list, edge_x, surface.y, 20.0, false, ctx.theme.bg_main);
            ui::paint_inverted_corner(
                list,
                edge_x,
                surface.y + surface.height - 20.0,
                20.0,
                true,
                ctx.theme.bg_main,
            );
        }
    }

    /// Route a click to a widget, using the same geometry as the last paint.
    pub fn on_click(&mut self, point: crate::render::display_list::Point, input: &InputState) -> bool {
        self.widgets
            .iter_mut()
            .any(|widget| widget.on_click(point, input))
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
            .map(|widget| widget.height(content_width))
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
    ///
    /// The bar stretches across the whole edge, so flexible widgets (which
    /// return no width) split the leftover horizontal space.
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
            .map(|widget| widget.width(height))
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

fn build_widget(entry: &WidgetConfig, config: Arc<Config>) -> Box<dyn Widget> {
    match entry.kind {
        WidgetKind::Launcher => Box::new(LauncherWidget),
        WidgetKind::Workspaces => Box::new(WorkspacesWidget),
        WidgetKind::AppTitle => Box::new(AppTitleWidget {
            fallback: "Desktop".to_string(),
        }),
        WidgetKind::Tray => Box::new(SpacerWidget { height: 24.0 }),
        WidgetKind::QuickPanel => Box::new(QuickPanelWidget {
            framed: entry.framed,
            height: entry.height.unwrap_or(86) as f32,
            children: entry
                .children
                .iter()
                .map(|child| build_widget(child, Arc::clone(&config)))
                .collect(),
        }),
        WidgetKind::Audio => Box::new(IconWidget {
            glyph: "\u{f0581}".to_string(),
            kind: IconKind::Audio,
            size: 20.0,
        }),
        WidgetKind::Bluetooth => Box::new(IconWidget {
            glyph: "\u{f00b1}".to_string(),
            kind: IconKind::Bluetooth,
            size: 20.0,
        }),
        WidgetKind::Wifi => Box::new(IconWidget {
            glyph: "\u{f05a9}".to_string(),
            kind: IconKind::Wifi,
            size: 20.0,
        }),
        WidgetKind::Battery => Box::new(BatteryWidget),
        WidgetKind::Clock => Box::new(ClockWidget),
        WidgetKind::Spacer => Box::new(SpacerWidget {
            height: entry.height.unwrap_or(12) as f32,
        }),
    }
}

// ---------------------------------------------------------------------------

struct LauncherWidget;

impl Widget for LauncherWidget {
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
}

struct WorkspacesWidget;

impl Widget for WorkspacesWidget {
    fn height(&self, _width: f32) -> Option<f32> {
        // Room for ten workspaces plus gaps; the rail's flexible widget takes
        // whatever is left.
        Some(10.0 * 20.0 + 9.0 * 3.0)
    }

    fn width(&self, height: f32) -> Option<f32> {
        // Ten cells side by side in a horizontal bar.
        Some(10.0 * (height.min(24.0) + 6.0))
    }

    fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        rect: Rect,
        _input: &InputState,
    ) {
        let hypr = ctx.state.hypr.get();
        let active = hypr.active.get(ctx.output_name).copied().unwrap_or(-1);

        // Show the ten-workspace window around the active workspace, matching
        // the QML shell's grouping.
        let (ids, active_index) = if active > 0 {
            let start = ((active - 1) / 10) * 10 + 1;
            let ids: Vec<i32> = (0..10).map(|offset| start + offset).collect();
            let index = ids.iter().position(|id| *id == active).unwrap_or(0);
            (ids, index)
        } else {
            ((1..=10).collect(), 0)
        };

        let vertical = ctx.is_vertical();
        let step = if vertical { 23.0 } else { rect.height.min(24.0) + 6.0 };
        let cell = if vertical {
            Rect { height: 20.0, ..rect }
        } else {
            Rect {
                width: step,
                ..rect
            }
        };

        for (index, id) in ids.iter().enumerate() {
            let row = if vertical {
                Rect { y: rect.y + index as f32 * step, ..cell }
            } else {
                Rect { x: rect.x + index as f32 * step, ..cell }
            };
            let is_active = index == active_index;
            let occupied = hypr.occupied.contains(id);
            let color = if is_active {
                ctx.theme.primary
            } else if occupied {
                ctx.theme.secondary
            } else {
                ctx.theme.text_muted
            };
            ui::paint_text_centered(ctx, list, &id.to_string(), row, 15.0, color);
        }
    }
}

struct AppTitleWidget {
    /// Fallback shown when no window is focused.
    fallback: String,
}

impl Widget for AppTitleWidget {
    /// Flexible: consumes the leftover vertical space so the widgets after it
    /// sit at the bottom of the rail, as `Layout.fillHeight` did in QML.
    fn height(&self, _width: f32) -> Option<f32> {
        None
    }

    /// In a horizontal bar the title is ordinary inline text, and it is the
    /// widget that absorbs slack, so it reports no intrinsic width.
    fn width(&self, _height: f32) -> Option<f32> {
        None
    }

    fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        rect: Rect,
        _input: &InputState,
    ) {
        let live_title = ctx
            .state
            .hypr
            .get()
            .title
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| self.fallback.clone());

        if !ctx.is_vertical() {
            // Horizontal bar: a single line, left-aligned after a small inset.
            let style = crate::text::TextStyle {
                family: Arc::from(ctx.theme.font.as_str()),
                size: 13.0,
                weight: 500,
                color: ctx.theme.text_main,
            };
            let insets = 6.0;
            let available = rect.width - insets * 2.0;
            if available <= 0.0 {
                return;
            }
            let text = self.elide_text(&live_title, &style, ctx, available);
            if text.is_empty() {
                return;
            }
            ui::paint_text_vcentered(
                ctx,
                list,
                &text,
                rect.x + insets,
                rect,
                style.size,
                style.weight,
                style.color,
            );
            return;
        }

        // Vertical label, matching the QML app-title. The run advances upward;
        // `line_height` is its cross-axis extent, which must fit the rail.
        let mut size = 14.0_f32;
        let max_thickness = (rect.width - 12.0).max(6.0);

        // Shrink until the rotated line height fits the rail.
        while size > 7.0 && size * 1.25 > max_thickness {
            size -= 0.5;
        }

        let style = crate::text::TextStyle {
            family: Arc::from(ctx.theme.font.as_str()),
            size,
            weight: 600,
            color: ctx.theme.text_main,
        };

        // Prefer the focused window's title, like the QML app-title.
        let live = ctx
            .state
            .hypr
            .get()
            .title
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| self.fallback.clone());
        let full_title = live;

        // Truncate to whatever height the widget actually has.
        let available = rect.height - 16.0;
        let title = self.elide_text(&full_title, &style, ctx, available);
        if title.is_empty() {
            return;
        }

        let mut run = ctx.text.shape(&title, &style, 0.0, 0.0);
        let line_height = size * 1.2;

        // The run is centred in the widget, like QML's `anchors.centerIn` on the
        // rotated item. Rotated -90 degrees the run advances upward from its
        // baseline, so its vertical span is [baseline - length, baseline];
        // centring means baseline = centre + length/2.
        let length = ctx.text.measure(&title, &style).0;
        let baseline_y = rect.y + rect.height * 0.5 + length * 0.5;

        // Cross-axis: the line box spans +x from the baseline, so bias the
        // baseline by half the line height to centre it.
        run.transform = ui::vertical_text_transform(
            rect.x + (rect.width - line_height) * 0.5 + line_height,
            baseline_y,
        );
        list.push_text(run);
    }
}

impl AppTitleWidget {
    /// Shorten the title with an ellipsis until it fits `available` pixels of
    /// run length, mirroring QML's `elide: Text.ElideRight`.
    fn elide_text(
        &self,
        text: &str,
        style: &crate::text::TextStyle,
        ctx: &mut PaintCtx<'_>,
        available: f32,
    ) -> String {
        if available <= 0.0 {
            return String::new();
        }
        let full = ctx.text.measure(text, style).0;
        if full <= available {
            return text.to_string();
        }
        let ellipsis = "\u{2026}";
        let mut chars: Vec<char> = text.chars().collect();
        while !chars.is_empty() {
            chars.pop();
            let candidate: String = chars.iter().collect::<String>() + ellipsis;
            if ctx.text.measure(&candidate, style).0 <= available {
                return candidate;
            }
        }
        String::new()
    }
}

struct QuickPanelWidget {
    framed: bool,
    height: f32,
    children: Vec<Box<dyn Widget>>,
}

impl Widget for QuickPanelWidget {
    fn height(&self, _width: f32) -> Option<f32> {
        Some(self.height)
    }

    /// In a horizontal bar the panel sizes to its children side by side.
    fn width(&self, height: f32) -> Option<f32> {
        let slot = height - 8.0;
        let count = self.children.len().max(1) as f32;
        Some(slot * count + 6.0 * (count - 1.0) + 8.0)
    }

    fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        rect: Rect,
        input: &InputState,
    ) {
        if !ctx.is_vertical() {
            self.paint_horizontal(ctx, list, rect, input);
            return;
        }
        const PANEL_WIDTH: f32 = 30.0;
        const INNER_MARGIN: f32 = 6.0;
        const CHILD_SPACING: f32 = 4.0;

        // The panel is narrower than the rail and centred in it, matching the
        // QML layout (`quickPanelWidth: 30` inside a 40px rail).
        let width = PANEL_WIDTH.min(rect.width);
        let panel = Rect {
            x: rect.x + (rect.width - width) * 0.5,
            y: rect.y,
            width,
            height: rect.height,
            radius: ctx.appearance.corner_radius,
            color: ctx.theme.bg_widget,
        };
        if self.framed {
            ui::paint_frame(
                list,
                panel,
                ctx.theme.bg_widget,
                ctx.theme.border_active,
                ctx.appearance.framed_border,
            );
        }

        let count = self.children.len();
        if count == 0 {
            return;
        }

        // Vertical stack inside the frame with fixed spacing, like the QML
        // ColumnLayout with `quickPanelSpacing`.
        let inner_height = (panel.height - INNER_MARGIN * 2.0).max(0.0);
        let gaps = CHILD_SPACING * (count.saturating_sub(1)) as f32;
        let slot = ((inner_height - gaps) / count as f32).max(0.0);

        let mut y = panel.y + INNER_MARGIN;
        for child in self.children.iter_mut() {
            let slot_rect = Rect {
                x: panel.x,
                y,
                width: panel.width,
                height: slot,
                radius: 0.0,
                color: Color::TRANSPARENT,
            };
            child.paint(ctx, list, slot_rect, input);
            y += slot + CHILD_SPACING;
        }
    }
}

impl QuickPanelWidget {
    /// Row layout for a horizontal bar: children side by side inside the frame.
    fn paint_horizontal(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        rect: Rect,
        input: &InputState,
    ) {
        const INNER_MARGIN: f32 = 4.0;
        const CHILD_SPACING: f32 = 6.0;

        if self.framed {
            ui::paint_frame(
                list,
                rect,
                ctx.theme.bg_widget,
                ctx.theme.border_active,
                ctx.appearance.framed_border,
            );
        }

        let count = self.children.len();
        if count == 0 {
            return;
        }
        let inner_width = (rect.width - INNER_MARGIN * 2.0).max(0.0);
        let gaps = CHILD_SPACING * (count.saturating_sub(1)) as f32;
        let slot = ((inner_width - gaps) / count as f32).max(0.0);

        let mut x = rect.x + INNER_MARGIN;
        for child in self.children.iter_mut() {
            let slot_rect = Rect {
                x,
                y: rect.y,
                width: slot,
                height: rect.height,
                radius: 0.0,
                color: Color::TRANSPARENT,
            };
            child.paint(ctx, list, slot_rect, input);
            x += slot + CHILD_SPACING;
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum IconKind {
    Audio,
    Bluetooth,
    Wifi,
}

struct IconWidget {
    glyph: String,
    kind: IconKind,
    size: f32,
}

impl Widget for IconWidget {
    fn height(&self, _width: f32) -> Option<f32> {
        Some(self.size + 4.0)
    }

    fn width(&self, height: f32) -> Option<f32> {
        Some(height.max(self.size + 4.0))
    }

    fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        rect: Rect,
        _input: &InputState,
    ) {
        // Enabled state mirrors the real subsystem state; disabled icons use
        // the muted colour, matching the QML sidebar.
        let (enabled, color) = match self.kind {
            IconKind::Bluetooth => {
                let bt = ctx.state.bluetooth.get();
                let on = bt["state"].as_str() != Some("off");
                (on, if on { ctx.theme.primary } else { ctx.theme.text_muted })
            }
            IconKind::Wifi => {
                let net = ctx.state.net.get();
                let on = net["enabled"].as_bool().unwrap_or(true);
                (on, if on { ctx.theme.primary } else { ctx.theme.text_muted })
            }
            IconKind::Audio => {
                let audio = ctx.state.audio.get();
                let muted = audio["mute"].as_str() == Some("yes");
                (!muted, if muted { ctx.theme.error } else { ctx.theme.primary })
            }
        };
        let _ = enabled;
        ui::paint_text_centered(ctx, list, &self.glyph, rect, self.size, color);
    }
}

struct BatteryWidget;

impl Widget for BatteryWidget {
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
        // Live percentage and charging state from stratum_sys.
        let battery = ctx.state.battery.get();
        let percent = battery["battery"]["pct"]
            .as_u64()
            .or_else(|| battery["battery"]["percentage"].as_f64().map(|p| p as u64))
            .unwrap_or(0)
            .min(100) as u8;
        let state_text = battery["battery"]["state"].as_str().unwrap_or("");
        let charging = matches!(state_text, "charging" | "pending-charge" | "fully-charged");

        // Codepoints from the QML battery widget's level buckets.
        let glyph = if charging {
            "\u{f0084}"
        } else {
            match percent {
                90..=100 => "\u{f0079}",
                80..=89 => "\u{f007a}",
                70..=79 => "\u{f007b}",
                60..=69 => "\u{f007c}",
                50..=59 => "\u{f007d}",
                40..=49 => "\u{f007e}",
                30..=39 => "\u{f007f}",
                20..=29 => "\u{f0080}",
                10..=19 => "\u{f0081}",
                _ => "\u{f0082}",
            }
        };
        let color = if percent <= 20 {
            ctx.theme.error
        } else {
            ctx.theme.success
        };
        ui::paint_text_centered(ctx, list, glyph, rect, 24.0, color);
    }
}

struct ClockWidget;

impl Widget for ClockWidget {
    fn height(&self, _width: f32) -> Option<f32> {
        Some(64.0)
    }

    /// In a bar the clock is one line; width is measured at paint time, so use
    /// a stable estimate.
    fn width(&self, _height: f32) -> Option<f32> {
        Some(84.0)
    }

    fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        rect: Rect,
        _input: &InputState,
    ) {
        if !ctx.is_vertical() {
            // Single line: "04:20 PM".
            let now = chrono::Local::now();
            let text = now.format("%I:%M %p").to_string();
            let style = crate::text::TextStyle {
                family: Arc::from(ctx.theme.font.as_str()),
                size: 13.0,
                weight: 500,
                color: ctx.theme.primary,
            };
            let width = ctx.text.measure(&text, &style).0;
            let x = rect.x + (rect.width - width) * 0.5;
            ui::paint_text_vcentered(
                ctx,
                list,
                &text,
                x,
                rect,
                style.size,
                style.weight,
                style.color,
            );
            return;
        }

        // Three stacked lines: hour, minute, meridiem — matching the QML clock.
        let now = chrono::Local::now();
        let hour = now.format("%I").to_string();
        let minute = now.format("%M").to_string();
        let meridiem = now.format("%p").to_string();

        let line = rect.height / 3.0;
        let lines: [(String, Color); 3] = [
            (hour, ctx.theme.primary),
            (minute, ctx.theme.secondary),
            (meridiem, ctx.theme.tertiary),
        ];
        for (index, (value, color)) in lines.into_iter().enumerate() {
            let row = Rect {
                x: rect.x,
                y: rect.y + line * index as f32,
                width: rect.width,
                height: line,
                radius: 0.0,
                color: Color::TRANSPARENT,
            };
            ui::paint_text_centered(ctx, list, &value, row, 18.0, color);
        }
    }
}

struct SpacerWidget {
    height: f32,
}

impl Widget for SpacerWidget {
    fn height(&self, _width: f32) -> Option<f32> {
        Some(self.height)
    }

    fn width(&self, _height: f32) -> Option<f32> {
        Some(self.height)
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

/// Keep the unused-import lint quiet while `TextRun` is only used indirectly.
#[allow(dead_code)]
fn _keep(_: &TextRun) {}
