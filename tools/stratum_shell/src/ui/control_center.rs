//! The control centre: a paged settings window for the shell.
//!
//! This module is window chrome only: the navigation rail, the page frame, and
//! the open/close animation. Each page lives in [`crate::ui::pages`] and owns
//! its own controls and interaction state, so a page can be tested and restyled
//! without touching the window.

use std::sync::Arc;

use crate::config::{Config, SidebarPosition};
use crate::render::display_list::{Color, DisplayList, Rect};
use crate::ui::pages::Pages;
use crate::ui::{InputState, PaintCtx};

pub use crate::ui::pages::Page;

/// Visual state of the control centre.
pub struct ControlCenter {
    pub open: bool,
    pub page: Page,
    config: Arc<Config>,
    /// Panel size in logical pixels.
    width: f32,
    /// Animation progress 0..1.
    pub progress: f32,
    pages: Pages,
}

impl ControlCenter {
    pub fn new(config: Arc<Config>) -> Self {
        let width = config.control_center.width as f32;
        Self {
            open: false,
            page: Page::Overview,
            config,
            width,
            progress: 0.0,
            pages: Pages::new(),
        }
    }

    /// Replace the configuration after a live reload or theme switch, keeping
    /// window state (page, open flag, animation) intact.
    pub fn set_config(&mut self, config: Arc<Config>) {
        self.width = config.control_center.width as f32;
        self.config = config;
    }

    /// Take any actions produced by the last paint.
    pub fn take_actions(&mut self) -> Vec<crate::actions::SystemAction> {
        self.pages.take_actions()
    }

    /// Open on `page`, or close if already open on that page.
    pub fn toggle(&mut self, page: Page) {
        if self.open && self.page == page {
            self.open = false;
        } else {
            let changed = !self.open || self.page != page;
            self.open = true;
            self.page = page;
            if changed {
                self.pages.on_page_shown(page);
            }
        }
    }

    /// Open on `page` without toggling. Used by IPC commands, whose contract is
    /// `open`, not "open unless already open".
    pub fn show(&mut self, page: Page) {
        if !self.open || self.page != page {
            self.pages.on_page_shown(page);
        }
        self.open = true;
        self.page = page;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    /// Advance the open/close animation. Returns true while animating.
    pub fn tick(&mut self, dt_seconds: f32) -> bool {
        let target = if self.open { 1.0 } else { 0.0 };
        let speed = if self.config.control_center.animation_ms == 0 {
            f32::INFINITY
        } else {
            1000.0 / self.config.control_center.animation_ms as f32
        };
        let step = (speed * dt_seconds).min(1.0);
        if (self.progress - target).abs() <= step {
            self.progress = target;
            false
        } else {
            self.progress += (target - self.progress).signum() * step;
            true
        }
    }

    /// True when the surface should be mapped (animating or open).
    pub fn visible(&self) -> bool {
        self.open || self.progress > 0.001
    }

    /// True while the open/close animation is still running.
    pub fn animating(&self) -> bool {
        let target = if self.open { 1.0 } else { 0.0 };
        (self.progress - target).abs() > 0.001 || (self.open && self.pages.animating(self.page))
    }

    /// Wheel scrolling for pages that have lists.
    pub fn on_scroll(&mut self, point: crate::render::display_list::Point, delta: f32) {
        if self.page == Page::Wifi {
            self.pages.wifi.on_scroll(point, delta);
        }
    }

    /// Paint into a normal window: no scrim and no slide, just the panel
    /// filling the window. Returns the content rect so clicks can be routed.
    pub fn paint_window(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        surface: Rect,
        input: &InputState,
    ) -> Rect {
        self.pages.begin_frame(input, ctx.dt);

        // Fill the whole window: `surface` may carry a transparent colour, so
        // paint an explicit background.
        list.push_rect(Rect {
            color: ctx.theme.bg_main,
            ..surface
        });
        let panel = Rect {
            x: 0.0,
            y: 0.0,
            width: surface.width,
            height: surface.height,
            radius: 0.0,
            color: ctx.theme.bg_main,
        };

        let nav = self.paint_nav(ctx, list, panel);
        let content = Rect {
            x: panel.x + nav + 1.0,
            y: panel.y,
            width: panel.width - nav - 1.0,
            height: panel.height,
            radius: 0.0,
            color: Color::TRANSPARENT,
        };
        self.paint_content(ctx, list, content, input);
        panel
    }

    /// Paint the overlay variant (kept for the panel-style presentation; the
    /// window is the default).
    pub fn paint(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        surface: Rect,
        input: &InputState,
    ) -> Rect {
        let alpha = 0.45 * self.progress;
        list.push_rect(Rect {
            color: Color::rgba(0.0, 0.0, 0.0, alpha),
            ..surface
        });

        let width = self.width.min(surface.width - 40.0);
        let height = (surface.height - 80.0).max(200.0);
        let slide = (1.0 - self.progress) * (width + 24.0);
        let x = match self.config.sidebar.position {
            SidebarPosition::Right => surface.x + surface.width - width - 12.0 + slide,
            _ => surface.x + 12.0 - slide,
        };
        let panel = Rect {
            x,
            y: surface.y + 40.0,
            width,
            height,
            radius: self.config.appearance.corner_radius,
            color: Color::TRANSPARENT,
        };
        crate::ui::paint_frame(
            list,
            panel,
            ctx.theme.bg_main,
            ctx.theme.border_inactive,
            true,
        );

        self.pages.begin_frame(input, ctx.dt);
        let nav = ctx.metrics.window.nav_width.min(132.0);
        let content = Rect {
            x: panel.x + nav + 8.0,
            y: panel.y,
            width: panel.width - nav - 8.0,
            height: panel.height,
            radius: 0.0,
            color: Color::TRANSPARENT,
        };
        self.paint_content(ctx, list, content, input);
        panel
    }

    /// Draw the navigation rail and its divider. Returns the rail width.
    fn paint_nav(&self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, panel: Rect) -> f32 {
        let win = &ctx.metrics.window;
        let nav_width = win.nav_width;
        let nav = Rect {
            x: panel.x,
            y: panel.y,
            width: nav_width,
            height: panel.height,
            radius: 0.0,
            color: Color::TRANSPARENT,
        };

        crate::ui::paint_text_vcentered(
            ctx,
            list,
            "Settings",
            nav.x + 18.0,
            Rect {
                height: win.nav_header,
                ..nav
            },
            ctx.metrics.typography.title,
            ctx.metrics.typography.weight_bold,
            ctx.theme.text_main,
        );

        let mut y = nav.y + win.nav_header + win.nav_top_gap;
        for page in Page::ALL {
            let row = Rect {
                x: nav.x + win.nav_inset,
                y,
                width: nav.width - win.nav_inset * 2.0,
                height: win.nav_row_height,
                radius: win.nav_row_radius,
                color: Color::TRANSPARENT,
            };
            let selected = page == self.page;
            if selected {
                list.push_rect(Rect {
                    color: ctx.theme.bg_hover,
                    ..row
                });
            }
            let color = if selected {
                ctx.theme.primary
            } else {
                ctx.theme.text_muted
            };
            crate::ui::paint_text_vcentered(
                ctx,
                list,
                page.glyph(),
                row.x + 10.0,
                row,
                ctx.metrics.typography.workspace,
                ctx.metrics.typography.weight_regular,
                color,
            );
            crate::ui::paint_text_vcentered(
                ctx,
                list,
                page.title(),
                row.x + 36.0,
                row,
                ctx.metrics.typography.body,
                ctx.metrics.typography.weight_regular,
                color,
            );
            y += win.nav_row_spacing;
        }

        list.push_rect(Rect {
            x: panel.x + nav_width,
            y: panel.y,
            width: 1.0,
            height: panel.height,
            radius: 0.0,
            color: ctx.theme.border_inactive,
        });
        nav_width
    }

    fn paint_content(
        &mut self,
        ctx: &mut PaintCtx<'_>,
        list: &mut DisplayList,
        area: Rect,
        input: &InputState,
    ) {
        let padding = ctx.metrics.window.content_padding;
        let inner = Rect {
            x: area.x + padding,
            y: area.y + padding,
            width: (area.width - padding * 2.0).max(0.0),
            height: (area.height - padding * 2.0).max(0.0),
            radius: 0.0,
            color: Color::TRANSPARENT,
        };
        self.pages.paint(self.page, ctx, list, inner, input);
    }

    /// Route a click in window coordinates. Returns an action for the shell.
    pub fn on_click(
        &mut self,
        point: crate::render::display_list::Point,
        surface: Rect,
        _input: &InputState,
    ) -> Option<crate::ui::Action> {
        if !self.open {
            return None;
        }

        // Navigation rail: the same geometry `paint_window` draws. These come
        // from the same metrics table, so the two cannot drift apart.
        let win = &self.config.metrics.window;
        let mut y = surface.y + win.nav_header + win.nav_top_gap;
        if point.x >= surface.x + win.nav_inset
            && point.x <= surface.x + win.nav_width - win.nav_inset
        {
            for page in Page::ALL {
                if point.y >= y && point.y <= y + win.nav_row_height {
                    if self.page != page {
                        self.page = page;
                        self.pages.on_page_shown(page);
                    }
                    return None;
                }
                y += win.nav_row_spacing;
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_opens_the_requested_page() {
        let config = Arc::new(Config::default());
        let mut center = ControlCenter::new(config);
        assert!(!center.open);

        center.toggle(Page::Battery);
        assert!(center.open);
        assert_eq!(center.page, Page::Battery);

        center.toggle(Page::Battery);
        assert!(!center.open);
    }

    #[test]
    fn show_is_not_a_toggle() {
        let config = Arc::new(Config::default());
        let mut center = ControlCenter::new(config);
        center.show(Page::Notifications);
        assert!(center.open);
        // A second `open` must leave it open; only `toggle` flips it.
        center.show(Page::Notifications);
        assert!(center.open);
        assert_eq!(center.page, Page::Notifications);
    }

    #[test]
    fn switching_page_while_open_stays_open() {
        let config = Arc::new(Config::default());
        let mut center = ControlCenter::new(config);
        center.toggle(Page::Audio);
        center.toggle(Page::Wifi);
        assert!(center.open);
        assert_eq!(center.page, Page::Wifi);
    }

    #[test]
    fn animation_reaches_its_target() {
        let config = Arc::new(Config::default());
        let mut center = ControlCenter::new(config);
        center.open = true;
        for _ in 0..100 {
            if !center.tick(0.016) {
                break;
            }
        }
        assert!((center.progress - 1.0).abs() < 1e-3);
        center.open = false;
        for _ in 0..100 {
            if !center.tick(0.016) {
                break;
            }
        }
        assert!(center.progress.abs() < 1e-3);
        assert!(!center.visible());
    }

    #[test]
    fn set_config_preserves_open_state_and_page() {
        let mut config = Config::default();
        config.theme = "carbon".into();
        let mut center = ControlCenter::new(Arc::new(config));
        center.toggle(Page::Display);
        assert!(center.open);

        let mut next = Config::default();
        next.theme = "gruvbox".into();
        next.control_center.width = 640;
        center.set_config(Arc::new(next));

        assert!(center.open, "open state must survive a config swap");
        assert_eq!(center.page, Page::Display, "page must survive");
        assert_eq!(center.width, 640.0, "new width should apply");
    }
}
