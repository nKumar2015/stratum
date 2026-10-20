//! The tray strip.
//!
//! Items come from the StatusNotifier watcher. Tray icons arrive either as a
//! themed icon name or as raw ARGB pixmaps; the icon theme and pixmap decoders
//! live in `crate::icons` and `crate::tray`, so this widget only places them.

use crate::render::display_list::{DisplayList, Rect};
use crate::ui::{self, Action, ClickCtx, InputState, PaintCtx, Widget};

pub struct Tray {
    /// Hit rects and the item each one belongs to, from the last paint.
    cells: Vec<(Rect, crate::tray::TrayItem)>,
}

impl Tray {
    pub fn new() -> Self {
        Self { cells: Vec::new() }
    }
}

impl Default for Tray {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for Tray {
    fn height(&self, _width: f32, metrics: &crate::ui::style::Metrics) -> Option<f32> {
        Some(metrics.tray.cell)
    }

    fn width(&self, height: f32, metrics: &crate::ui::style::Metrics) -> Option<f32> {
        Some((height.max(metrics.tray.cell) + metrics.tray.gap) * 1.4)
    }

    fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        rect: Rect,
        _input: &InputState,
    ) {
        let items = ctx.state.tray.get();
        self.cells.clear();
        if items.is_empty() {
            return;
        }

        let vertical = ctx.is_vertical();
        let mut cursor = if vertical { rect.y } else { rect.x };
        // Vertical rails show one column, bars show a row. Both bound the count
        // so a misbehaving application cannot push the rest of the bar off
        // screen.
        let tray = &ctx.metrics.tray;
        let limit = if vertical {
            tray.vertical_limit
        } else {
            tray.horizontal_limit
        };

        for item in items.iter().take(limit) {
            let cell = if vertical {
                Rect {
                    x: rect.x,
                    y: cursor,
                    width: rect.width,
                    height: tray.cell,
                    radius: 0.0,
                    color: crate::render::display_list::Color::TRANSPARENT,
                }
            } else {
                Rect {
                    x: cursor,
                    y: rect.y,
                    width: tray.cell,
                    height: rect.height,
                    radius: 0.0,
                    color: crate::render::display_list::Color::TRANSPARENT,
                }
            };
            self.paint_item(ctx, list, cell, item);
            self.cells.push((cell, item.clone()));
            cursor += tray.cell + tray.gap;
        }
    }

    fn on_click(&mut self, ctx: &ClickCtx<'_>) -> Option<Action> {
        let (rect, item) = self
            .cells
            .iter()
            .find(|(rect, _)| rect.contains(ctx.point))?;
        match ctx.button {
            crate::ui::MouseButton::Left => Some(Action::ActivateTray(item.service.clone())),
            // Middle-click is SNI's SecondaryActivate, which some apps use to
            // toggle a window.
            crate::ui::MouseButton::Middle => {
                Some(Action::SecondaryActivateTray(item.service.clone()))
            }
            crate::ui::MouseButton::Right => {
                item.menu.as_ref().map(|path| Action::OpenTrayMenu {
                    service: item.service.clone(),
                    path: path.clone(),
                    y: rect.y,
                })
            }
        }
    }
}

impl Tray {
    fn paint_item(
        &self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        cell: Rect,
        item: &crate::tray::TrayItem,
    ) {
        let tray = &ctx.metrics.tray;
        let color = match item.status.as_str() {
            "NeedsAttention" => ctx.theme.warning,
            "Passive" => ctx.theme.text_muted,
            _ => ctx.theme.text_main,
        };

        // Prefer the real icon: a pixmap if the item supplied one, otherwise the
        // themed icon name. Only fall back to a letter when neither resolves, so
        // the bar never shows an empty cell.
        if let Some((width, height, pixels)) = item
            .icon_pixmaps
            .iter()
            .max_by_key(|(w, h, _)| (*w as u64) * (*h as u64))
        {
            if *width > 0 && *height > 0 && pixels.len() >= (*width * *height * 4) as usize {
                list.push_image(crate::render::display_list::Image {
                    width: *width,
                    height: *height,
                    pixels: std::sync::Arc::from(pixels.as_slice()),
                    x: cell.x + (cell.width - tray.cell) * 0.5,
                    y: cell.y + (cell.height - tray.cell) * 0.5,
                    width_logical: tray.cell,
                    height_logical: tray.cell,
                });
                return;
            }
        }

        if let Some(icon) = ctx.icons.load_icon(&item.icon_name, tray.cell as u32, 2) {
            list.push_image(crate::render::display_list::Image {
                width: icon.width,
                height: icon.height,
                pixels: icon.pixels,
                x: cell.x + (cell.width - tray.cell) * 0.5,
                y: cell.y + (cell.height - tray.cell) * 0.5,
                width_logical: tray.cell,
                height_logical: tray.cell,
            });
            return;
        }

        let label = item
            .title
            .chars()
            .next()
            .or_else(|| item.id.chars().next())
            .unwrap_or('?')
            .to_string();
        ui::paint_text_centered(ctx, list, &label, cell, ctx.metrics.typography.tray, color);
    }
}
