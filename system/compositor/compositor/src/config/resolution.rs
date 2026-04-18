//! Resolution persistence in compositor.conf.

use anyos_std::println;

use super::file::{read_i64, register_manifest, write_i64};

pub struct SavedResolution {
    pub width: u32,
    pub height: u32,
}

pub fn read_resolution() -> Option<SavedResolution> {
    register_manifest();
    let width = read_i64("resolution/width").and_then(|v| u32::try_from(v).ok());
    let height = read_i64("resolution/height").and_then(|v| u32::try_from(v).ok());

    match (width, height) {
        (Some(w), Some(h)) if w >= 1024 && h >= 768 => Some(SavedResolution { width: w, height: h }),
        _ => None,
    }
}

pub fn save_resolution(width: u32, height: u32) {
    register_manifest();
    if !write_i64("resolution/width", width as i64) || !write_i64("resolution/height", height as i64) {
        println!("compositor: FAILED to save resolution");
    }
}
