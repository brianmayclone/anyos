//! Tag — colored pill label (chip) with optional close button.

use crate::draw;
use crate::theme;

const TAG_HEIGHT: u32 = 24;
const TAG_PADDING_H: u32 = 10;
const TAG_CORNER: u32 = 12;
const CLOSE_SIZE: u32 = 16;
const CLOSE_MARGIN: u32 = 4;

/// Render a tag/chip.
/// show_close: if nonzero, render a small "x" close button on the right.
pub extern "C" fn tag_render(
    win: u32, x: i32, y: i32,
    text: *const u8, text_len: u32,
    bg_color: u32, text_color: u32,
    show_close: u32,
) {
    if text.is_null() || text_len == 0 { return; }

    let text_slice = unsafe { core::slice::from_raw_parts(text, text_len as usize) };
    let (text_w, th) = draw::text_size(text_slice);
    let close_extra = if show_close != 0 { CLOSE_SIZE + CLOSE_MARGIN } else { 0 };
    let w = TAG_PADDING_H + text_w + close_extra + TAG_PADDING_H;

    // Pill background
    draw::fill_rounded_rect(win, x, y, w, TAG_HEIGHT, TAG_CORNER, bg_color);

    // Text
    let text_x = x + TAG_PADDING_H as i32;
    let text_y = y + (TAG_HEIGHT as i32 - th as i32) / 2;
    draw::draw_text(win, text_x, text_y, text_color, text_slice);

    // Close button "x"
    if show_close != 0 {
        let cx = x + (w - TAG_PADDING_H - CLOSE_SIZE) as i32;
        let cy = y + (TAG_HEIGHT as i32 - CLOSE_SIZE as i32) / 2;
        let mid = CLOSE_SIZE as i32 / 2;
        draw::fill_rect(win, cx + 3, cy + mid, CLOSE_SIZE - 6, 1, text_color);
        draw::fill_rect(win, cx + mid, cy + 3, 1, CLOSE_SIZE - 6, text_color);
    }
}

/// Hit test: returns 1 if (mx,my) is inside the tag bounds.
pub extern "C" fn tag_hit_test(
    x: i32, y: i32,
    text_len: u32, show_close: u32,
    mx: i32, my: i32,
) -> u32 {
    let text_w = text_len * 7; // approximate for hit testing
    let close_extra = if show_close != 0 { CLOSE_SIZE + CLOSE_MARGIN } else { 0 };
    let w = (TAG_PADDING_H + text_w + close_extra + TAG_PADDING_H) as i32;
    let h = TAG_HEIGHT as i32;
    if mx >= x && mx < x + w && my >= y && my < y + h { 1 } else { 0 }
}

/// Hit test for the close button only. Returns 1 if (mx,my) is on the close "x".
pub extern "C" fn tag_close_hit_test(
    x: i32, y: i32,
    text_len: u32,
    mx: i32, my: i32,
) -> u32 {
    let text_w = text_len * 7; // approximate for hit testing
    let w = TAG_PADDING_H + text_w + CLOSE_SIZE + CLOSE_MARGIN + TAG_PADDING_H;
    let cx = x + (w - TAG_PADDING_H - CLOSE_SIZE) as i32;
    let cy = y + (TAG_HEIGHT as i32 - CLOSE_SIZE as i32) / 2;
    if mx >= cx && mx < cx + CLOSE_SIZE as i32 && my >= cy && my < cy + CLOSE_SIZE as i32 {
        1
    } else {
        0
    }
}
