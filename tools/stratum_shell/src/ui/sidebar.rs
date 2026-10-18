//! The sidebar: widgets from the configuration, laid out on a rail.

use std::sync::Arc;

use crate::config::{Config, SidebarPosition, WidgetConfig, WidgetKind};
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
        state: &SystemState,
    ) {
        // Rail background.
        list.push_rect(surface);

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

        let _ = state;

        // Scooped corners where the rail meets the screen edge.
        if self.inverted_corners {
            let edge_x = match self.position {
                SidebarPosition::Left => surface.x + surface.width,
                SidebarPosition::Right => surface.x - 20.0,
            };
            ui::paint_inverted_corner(
                list,
                edge_x,
                surface.y,
                20.0,
                false,
                ctx.theme.bg_main,
            );
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
}

fn build_widget(entry: &WidgetConfig, config: Arc<Config>) -> Box<dyn Widget> {
    match entry.kind {
        WidgetKind::Launcher => Box::new(LauncherWidget),
        WidgetKind::Workspaces => Box::new(WorkspacesWidget { count: 5, active: 1 }),
        WidgetKind::AppTitle => Box::new(AppTitleWidget {
            title: "Desktop".to_string(),
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
            color_role: ColorRole::Primary,
            size: 16.0,
        }),
        WidgetKind::Bluetooth => Box::new(IconWidget {
            glyph: "\u{f00b1}".to_string(),
            color_role: ColorRole::Primary,
            size: 16.0,
        }),
        WidgetKind::Wifi => Box::new(IconWidget {
            glyph: "\u{f05a9}".to_string(),
            color_role: ColorRole::Primary,
            size: 16.0,
        }),
        WidgetKind::Battery => Box::new(BatteryWidget {
            percent: 80,
            charging: false,
        }),
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

struct WorkspacesWidget {
    count: usize,
    active: usize,
}

impl Widget for WorkspacesWidget {
    fn height(&self, _width: f32) -> Option<f32> {
        Some(self.count as f32 * 20.0 + (self.count.saturating_sub(1)) as f32 * 3.0)
    }

    fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        rect: Rect,
        _input: &InputState,
    ) {
        for index in 0..self.count {
            let y = rect.y + index as f32 * 23.0;
            let label = (index + 1).to_string();
            let color = if index == self.active {
                ctx.theme.primary
            } else {
                ctx.theme.text_muted
            };
            let row = Rect {
                y,
                height: 20.0,
                ..rect
            };
            ui::paint_text_centered(ctx, list, &label, row, 15.0, color);
        }
    }
}

struct AppTitleWidget {
    title: String,
}

impl Widget for AppTitleWidget {
    /// Flexible: consumes the leftover vertical space so the widgets after it
    /// sit at the bottom of the rail, as `Layout.fillHeight` did in QML.
    fn height(&self, _width: f32) -> Option<f32> {
        None
    }

    fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        rect: Rect,
        _input: &InputState,
    ) {
        // Vertical label, matching the QML app-title. The run advances upward;
        // `line_height` is its cross-axis extent, which must fit the rail.
        let mut size = 12.0_f32;
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

        // Truncate to whatever height the widget actually has.
        let available = rect.height - 16.0;
        let title = self.elide(&style, ctx, available);
        if title.is_empty() {
            return;
        }

        let mut run = ctx.text.shape(&title, &style, 0.0, 0.0);
        let line_height = size * 1.2;
        let bottom = rect.y + rect.height - 8.0;
        // The rotation makes the run's cross-axis extent (line_height) span
        // +x from the baseline, so bias the baseline to centre that extent.
        run.transform = ui::vertical_text_transform(
            rect.x + (rect.width + line_height) * 0.5,
            bottom,
        );
        list.push_text(run);
    }
}

impl AppTitleWidget {
    /// Shorten the title with an ellipsis until it fits `available` pixels of
    /// run length, mirroring QML's `elide: Text.ElideRight`.
    fn elide(&self, style: &crate::text::TextStyle, ctx: &mut PaintCtx<'_>, available: f32) -> String {
        if available <= 0.0 {
            return String::new();
        }
        let full = ctx.text.measure(&self.title, style).0;
        if full <= available {
            return self.title.clone();
        }
        let ellipsis = "\u{2026}";
        let mut chars: Vec<char> = self.title.chars().collect();
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

    fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        rect: Rect,
        input: &InputState,
    ) {
        let radius = ctx.appearance.corner_radius;
        if self.framed {
            let inner = Rect {
                x: rect.x + 5.0,
                y: rect.y,
                width: rect.width - 10.0,
                height: rect.height,
                radius,
                color: ctx.theme.bg_widget,
            };
            ui::paint_frame(
                list,
                inner,
                ctx.theme.bg_widget,
                ctx.theme.border_active,
                ctx.appearance.framed_border,
            );
        }

        let count = self.children.len();
        if count == 0 {
            return;
        }
        let slot = rect.height / count as f32;
        for (index, child) in self.children.iter_mut().enumerate() {
            let slot_rect = Rect {
                x: rect.x,
                y: rect.y + slot * index as f32,
                width: rect.width,
                height: slot,
                radius: 0.0,
                color: Color::TRANSPARENT,
            };
            child.paint(ctx, list, slot_rect, input);
        }
    }
}

#[derive(Clone, Copy)]
enum ColorRole {
    Primary,
}

struct IconWidget {
    glyph: String,
    color_role: ColorRole,
    size: f32,
}

impl Widget for IconWidget {
    fn height(&self, _width: f32) -> Option<f32> {
        Some(self.size + 4.0)
    }

    fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        rect: Rect,
        _input: &InputState,
    ) {
        let color = match self.color_role {
            ColorRole::Primary => ctx.theme.primary,
        };
        ui::paint_text_centered(ctx, list, &self.glyph, rect, self.size, color);
    }
}

struct BatteryWidget {
    percent: u8,
    charging: bool,
}

impl Widget for BatteryWidget {
    fn height(&self, _width: f32) -> Option<f32> {
        Some(24.0)
    }

    fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        rect: Rect,
        _input: &InputState,
    ) {
        // Codepoints from the QML battery widget's level buckets.
        let glyph = if self.charging {
            "\u{f0084}"
        } else {
            match self.percent {
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
        let color = if self.percent <= 20 {
            ctx.theme.error
        } else {
            ctx.theme.success
        };
        ui::paint_text_centered(ctx, list, glyph, rect, 16.0, color);
    }
}

struct ClockWidget;

impl Widget for ClockWidget {
    fn height(&self, _width: f32) -> Option<f32> {
        Some(64.0)
    }

    fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        rect: Rect,
        _input: &InputState,
    ) {
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
