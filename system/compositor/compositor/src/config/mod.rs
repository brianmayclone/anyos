//! Compositor configuration.

mod display;
mod file;
mod launch;
mod resolution;
mod shortcuts;
mod theme;

pub use display::{read_font_smoothing, read_scale_factor, save_font_smoothing, save_scale_factor};
pub use launch::{launch_autostart, launch_login_services};
pub use resolution::{read_resolution, save_resolution, SavedResolution};
pub use shortcuts::{read_shortcuts, KeyboardShortcut, ShortcutAction};
pub use theme::{read_theme, save_theme, SavedTheme};
