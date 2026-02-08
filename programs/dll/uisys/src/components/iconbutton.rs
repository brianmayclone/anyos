//! IconButton — icon-only button with optional round shape.

use crate::draw;
use crate::theme;

/// Render an icon button (background only; icon drawn by caller).
/// size: width and height of the button.
/// state: 0=Normal, 1=Hover, 2=Pressed, 3=Disabled.
/// round: if nonzero, render as a circle; otherwise as a rounded rect.
pub extern "C" fn iconbutton_render(
    win: u32, x: i32, y: i32, size: u32,
    state: u8, round: u32,
) {
    let (bg, _fg) = iconbutton_colors(state);
    let corner = if round != 0 { size / 2 } else { 6 };

    // Background
    draw::fill_rounded_rect(win, x, y, size, size, corner, bg);

    // Subtle border for normal/hover states
    if state < 3 {
        draw::draw_border(win, x, y, size, size, theme::CARD_BORDER);
    }
}

/// Hit test for icon button.
pub extern "C" fn iconbutton_hit_test(
    x: i32, y: i32, size: u32,
    mx: i32, my: i32,
) -> u32 {
    let s = size as i32;
    if mx >= x && mx < x + s && my >= y && my < y + s { 1 } else { 0 }
}

fn iconbutton_colors(state: u8) -> (u32, u32) {
    match state {
        1 => (theme::CONTROL_HOVER, theme::TEXT),
        2 => (theme::CONTROL_PRESSED, theme::TEXT),
        3 => (theme::CONTROL_PRESSED, theme::TEXT_DISABLED),
        _ => (theme::CONTROL_BG, theme::TEXT),
    }
}
