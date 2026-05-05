//! Compositor configuration.

mod display;
mod file;
mod input;
mod launch;
mod resolution;
mod shortcuts;
mod theme;

pub use display::{
    read_font_smoothing, read_scale_auto, read_scale_factor, save_font_smoothing, save_scale_factor,
};
pub use input::{
    cursor_scale_percent, cursor_size_index, left_handed_enabled, natural_scroll_enabled,
    pointer_speed_percent, read_cursor_size, read_double_click_ms, read_left_handed,
    read_natural_scroll, read_pointer_speed, refresh_input_settings,
    refresh_input_settings_check_cursor, refresh_natural_scroll, save_cursor_size,
    save_double_click_ms, save_left_handed, save_natural_scroll, save_pointer_speed,
};
pub use launch::{launch_autostart, launch_login_services, launch_required_services};
pub use resolution::{read_resolution, save_resolution, SavedResolution};
pub use shortcuts::{read_shortcuts, KeyboardShortcut, ShortcutAction};
pub use theme::{read_theme, save_theme, SavedTheme};
