//! What Stratum's shell and its apps share: the renderer (`render`), text
//! shaping (`text`), icons and desktop entries, the configuration with its
//! themes and metrics, and the small pieces of drawing that need no shell
//! state (colours, animation, graphs).
//!
//! The shell re-exports these modules at their old paths
//! (`crate::render`, `crate::config`, ...), so its code does not name this
//! crate.

pub mod animation;
pub mod apps;
pub mod app;
pub mod color;
pub mod config;
pub mod custom_themes;
pub mod desktop;
pub mod graph;
pub mod icons;
pub mod paths;
pub mod render;
pub mod secret;
pub mod settings_override;
pub mod style;
pub mod text;
pub mod theme_choice;
pub mod uri;
pub mod widgets;
