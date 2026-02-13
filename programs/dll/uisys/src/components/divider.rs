//! Divider — 1px horizontal or vertical separator line.

use crate::draw;
use crate::theme;

/// Render a horizontal divider line.
pub extern "C" fn divider_render_h(win: u32, x: i32, y: i32, w: u32) {
    draw::fill_rect(win, x, y, w, 1, theme::SEPARATOR());
}

/// Render a vertical divider line.
pub extern "C" fn divider_render_v(win: u32, x: i32, y: i32, h: u32) {
    draw::fill_rect(win, x, y, 1, h, theme::SEPARATOR());
}
