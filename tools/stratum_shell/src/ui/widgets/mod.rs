//! Bar widgets, one module per widget.
//!
//! Each widget owns its own layout and hit-testing state so the sidebar is just
//! a stack of reusable parts. Adding a widget means adding a module here and a
//! match arm in [`super::sidebar::build_widget`]; nothing else changes.

pub mod app_title;
pub mod battery;
pub mod clock;
pub mod icon;
pub mod idle_inhibit;
pub mod night_light;
pub mod launcher;
pub mod quick_panel;
pub mod spacer;
pub mod tray;
pub mod workspaces;

pub use app_title::AppTitle;
pub use battery::Battery;
pub use clock::Clock;
pub use icon::{Icon, IconKind};
pub use idle_inhibit::IdleInhibit;
pub use night_light::NightLight;
pub use launcher::Launcher;
pub use quick_panel::QuickPanel;
pub use spacer::Spacer;
pub use tray::Tray;
pub use workspaces::Workspaces;
