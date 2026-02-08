//! RadioButton — circular selector with label.

use crate::draw;
use crate::theme;

const RADIO_SIZE: u32 = 18;
const INNER_DOT: u32 = 8;

/// Render a radio button with optional label.
/// selected: 0=unselected, nonzero=selected.
pub extern "C" fn radio_render(
    win: u32, x: i32, y: i32, selected: u32,
    label: *const u8, label_len: u32,
) {
    let r = RADIO_SIZE / 2;

    if selected != 0 {
        // Filled blue circle
        draw::fill_rounded_rect(win, x, y, RADIO_SIZE, RADIO_SIZE, r, theme::ACCENT);
        // White inner dot
        let dot_offset = (RADIO_SIZE - INNER_DOT) / 2;
        let dot_r = INNER_DOT / 2;
        draw::fill_rounded_rect(
            win,
            x + dot_offset as i32,
            y + dot_offset as i32,
            INNER_DOT,
            INNER_DOT,
            dot_r,
            theme::CHECK_MARK,
        );
    } else {
        // Empty circle with border
        draw::fill_rounded_rect(win, x, y, RADIO_SIZE, RADIO_SIZE, r, theme::CONTROL_BG);
        draw::draw_border(win, x, y, RADIO_SIZE, RADIO_SIZE, theme::INPUT_BORDER);
        // Re-draw inner area to hide square corners of border inside the circle
        let inset = 1u32;
        draw::fill_rounded_rect(
            win,
            x + inset as i32,
            y + inset as i32,
            RADIO_SIZE - inset * 2,
            RADIO_SIZE - inset * 2,
            r - inset,
            theme::CONTROL_BG,
        );
    }

    // Label text
    if !label.is_null() && label_len > 0 {
        let label_slice = unsafe { core::slice::from_raw_parts(label, label_len as usize + 1) };
        let text_x = x + RADIO_SIZE as i32 + 8;
        let text_y = y + (RADIO_SIZE as i32 - 16) / 2;
        draw::draw_text(win, text_x, text_y, theme::TEXT, label_slice);
    }
}

/// Hit test for radio button (includes label area).
pub extern "C" fn radio_hit_test(x: i32, y: i32, mx: i32, my: i32) -> u32 {
    let w = RADIO_SIZE as i32 + 100; // includes label area
    let h = RADIO_SIZE as i32;
    if mx >= x && mx < x + w && my >= y && my < y + h { 1 } else { 0 }
}
