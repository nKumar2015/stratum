//! Widgets shared by the shell's pages and the apps: drawing helpers,
//! controls (switches, sliders), the text field, and the page layout
//! primitives (cards, rows, buttons, dropdowns).
//!
//! They draw through [`Ui`], which the shell's paint context and each app's
//! implement: the theme's colours, the metrics, and the text and icon
//! engines are all a widget needs.

pub mod controls;
pub mod draw;
pub mod layout;
pub mod text_field;

pub use draw::InputState;

use crate::config::{AppearanceConfig, Theme};
use crate::icons::IconTheme;
use crate::render::display_list::{Color, DisplayList};
use crate::style::Metrics;
use crate::text::{TextEngine, TextStyle};

/// What widgets paint with.
pub trait Ui {
    fn theme(&self) -> &Theme;
    fn appearance(&self) -> &AppearanceConfig;
    /// Every size, from `[metrics]`.
    fn metrics(&self) -> &Metrics;
    fn text(&mut self) -> &mut TextEngine;
    fn icons(&mut self) -> &mut IconTheme;

    /// A string's width in the theme font at `size`.
    fn measure(&mut self, value: &str, size: f32) -> f32 {
        let style = TextStyle { family: std::sync::Arc::from(self.theme().font.as_str()), size, weight: 400, color: Color::WHITE };
        self.text().measure(value, &style).0
    }

    /// Shape `value` with its baseline at `(x, y)` and add it to `list`.
    fn text_text(&mut self, list: &mut DisplayList, value: &str, x: f32, y: f32, style: TextStyle) {
        if value.is_empty() {
            return;
        }
        let run = self.text().shape(value, &style, x, y);
        list.push_text(run);
    }
}
