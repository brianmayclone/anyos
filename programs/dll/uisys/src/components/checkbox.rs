//! Checkbox — square check box with checkmark.

use crate::draw;
use crate::theme;

/// Render a checkbox with optional label.
/// state: 0=unchecked, 1=checked, 2=indeterminate
pub extern "C" fn checkbox_render(
    win: u32, x: i32, y: i32, state: u8,
    label: *const u8, label_len: u32,
) {
    let size = theme::CHECKBOX_SIZE;

    if state == 1 || state == 2 {
        // Filled blue background
        draw::fill_rounded_rect(win, x, y, size, size, 3, theme::ACCENT);

        if state == 1 {
            // Checkmark: draw two small lines
            // Simplified checkmark using filled rects
            let cx = x + 4;
            let cy = y + (size as i32) / 2;
            draw::fill_rect(win, cx, cy, 3, 2, theme::CHECK_MARK);
            draw::fill_rect(win, cx + 2, cy - 3, 2, 5, theme::CHECK_MARK);
            draw::fill_rect(win, cx + 4, cy - 5, 2, 5, theme::CHECK_MARK);
            draw::fill_rect(win, cx + 6, cy - 7, 2, 4, theme::CHECK_MARK);
        } else {
            // Indeterminate: dash
            let dash_y = y + (size as i32 - 2) / 2;
            draw::fill_rect(win, x + 4, dash_y, size - 8, 2, theme::CHECK_MARK);
        }
    } else {
        // Empty box with border
        draw::fill_rounded_rect(win, x, y, size, size, 3, theme::CONTROL_BG);
        draw::draw_border(win, x, y, size, size, theme::INPUT_BORDER);
    }

    // Label text
    if !label.is_null() && label_len > 0 {
        let label_slice = unsafe { core::slice::from_raw_parts(label, label_len as usize + 1) };
        let text_x = x + size as i32 + 8;
        let text_y = y + (size as i32 - 16) / 2;
        draw::draw_text(win, text_x, text_y, theme::TEXT, label_slice);
    }
}

/// Hit test for checkbox (includes label area for usability).
pub extern "C" fn checkbox_hit_test(x: i32, y: i32, mx: i32, my: i32) -> u32 {
    let size = theme::CHECKBOX_SIZE as i32;
    // Hit area includes some space for the label
    if mx >= x && mx < x + size + 100 && my >= y && my < y + size { 1 } else { 0 }
}
