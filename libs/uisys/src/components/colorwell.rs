//! ColorWell — color swatch display square with border.

use crate::draw;
use crate::theme;

const WELL_CORNER: u32 = 4;
const BORDER_WIDTH: u32 = 1;

/// Render a color well (swatch).
/// size: width and height of the square swatch.
/// color: the fill color to display (0xAARRGGBB).
pub extern "C" fn colorwell_render(win: u32, x: i32, y: i32, size: u32, color: u32) {
    // Outer border
    draw::fill_rounded_rect(win, x, y, size, size, WELL_CORNER, theme::INPUT_BORDER());

    // Inner color fill (inset by border width)
    let inner = BORDER_WIDTH;
    let inner_size = if size > inner * 2 { size - inner * 2 } else { size };
    let inner_corner = if WELL_CORNER > inner { WELL_CORNER - inner } else { 0 };
    draw::fill_rounded_rect(
        win,
        x + inner as i32,
        y + inner as i32,
        inner_size,
        inner_size,
        inner_corner,
        color,
    );
}
