//! Theme settings in compositor.conf.

use anyos_std::println;

use super::file::{read_string, register_manifest, write_string};

pub struct SavedTheme {
    pub mode: alloc::string::String,
    pub style: alloc::string::String,
}

pub fn read_theme() -> Option<SavedTheme> {
    register_manifest();
    let mode = read_string("theme/mode")?;
    let style = read_string("theme/style").unwrap_or_default();
    Some(SavedTheme { mode, style })
}

pub fn save_theme(mode: &str, style: &str) {
    register_manifest();
    if !write_string("theme/mode", mode) || !write_string("theme/style", style) {
        println!("compositor: FAILED to save theme");
    }
}
