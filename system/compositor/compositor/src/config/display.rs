//! Display settings in compositor.conf.

use anyos_std::println;

use super::file::{read_bool, read_i64, register_manifest, write_bool, write_i64};

pub fn read_font_smoothing() -> Option<u32> {
    register_manifest();
    read_i64("display/font_smoothing")
        .and_then(|v| u32::try_from(v).ok())
        .map(|v| v.min(2))
}

pub fn save_font_smoothing(mode: u32) {
    register_manifest();
    if !write_i64("display/font_smoothing", mode.min(2) as i64) {
        println!("compositor: FAILED to save font smoothing");
    }
}

pub fn read_scale_factor() -> Option<u32> {
    register_manifest();
    read_i64("display/scale")
        .and_then(|v| u32::try_from(v).ok())
        .map(|v| v.clamp(100, 300))
}

pub fn read_scale_auto() -> bool {
    register_manifest();
    read_bool("display/scale_auto").unwrap_or(true)
}

pub fn save_scale_factor(percent: u32) {
    let clamped = percent.max(100).min(300);
    let rounded = ((clamped + 12) / 25) * 25;
    register_manifest();
    if !write_i64("display/scale", rounded as i64) {
        println!("compositor: FAILED to save scale");
    }
    if !write_bool("display/scale_auto", false) {
        println!("compositor: FAILED to save scale auto flag");
    }
}
