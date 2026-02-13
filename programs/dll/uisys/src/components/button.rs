//! Button — push button with 4 styles and 4 states.

use crate::draw;
use crate::theme;

/// style: 0=Default(gray), 1=Primary(blue), 2=Destructive(red), 3=Plain(text-only)
/// state: 0=Normal, 1=Hover, 2=Pressed, 3=Disabled
pub extern "C" fn button_render(
    win: u32, x: i32, y: i32, w: u32, h: u32,
    label: *const u8, label_len: u32,
    style: u8, state: u8,
) {
    let (bg, fg) = button_colors(style, state);

    // Background
    if style != 3 {
        // Filled button styles
        draw::fill_rounded_rect(win, x, y, w, h, theme::BUTTON_CORNER, bg);
    }

    // Label text (centered)
    if !label.is_null() && label_len > 0 {
        let label_slice = unsafe { core::slice::from_raw_parts(label, label_len as usize) };
        let (tw, th) = draw::text_size(label_slice);
        let text_x = x + (w as i32 - tw as i32) / 2;
        let text_y = y + (h as i32 - th as i32) / 2;
        draw::draw_text(win, text_x, text_y, fg, label_slice);
    }
}

/// Hit test: returns 1 if (mx,my) is inside the button bounds.
pub extern "C" fn button_hit_test(
    x: i32, y: i32, w: u32, h: u32,
    mx: i32, my: i32,
) -> u32 {
    if mx >= x && mx < x + w as i32 && my >= y && my < y + h as i32 {
        1
    } else {
        0
    }
}

/// Measure button: compute minimal size for label.
pub extern "C" fn button_measure(
    label: *const u8, label_len: u32,
    out_w: *mut u32, out_h: *mut u32,
) {
    if out_w.is_null() || out_h.is_null() { return; }
    let (tw, _) = if !label.is_null() && label_len > 0 {
        let label_slice = unsafe { core::slice::from_raw_parts(label, label_len as usize) };
        draw::text_size(label_slice)
    } else {
        (0, 0)
    };
    unsafe {
        *out_w = tw + theme::BUTTON_PADDING_H * 2;
        *out_h = theme::BUTTON_HEIGHT;
    }
}

fn button_colors(style: u8, state: u8) -> (u32, u32) {
    if state == 3 {
        // Disabled
        return (theme::CONTROL_BG(), theme::TEXT_DISABLED());
    }
    match style {
        1 => {
            // Primary (blue)
            let bg = match state {
                1 => theme::ACCENT_HOVER(),
                2 => 0xFF005EC4,
                _ => theme::ACCENT(),
            };
            (bg, 0xFFFFFFFF)
        }
        2 => {
            // Destructive (red)
            let bg = match state {
                1 => 0xFFFF5044,
                2 => 0xFFCC2F26,
                _ => theme::DESTRUCTIVE(),
            };
            (bg, 0xFFFFFFFF)
        }
        3 => {
            // Plain (text-only)
            let fg = match state {
                1 => theme::ACCENT_HOVER(),
                2 => 0xFF005EC4,
                _ => theme::ACCENT(),
            };
            (0, fg)
        }
        _ => {
            // Default (gray)
            let bg = match state {
                1 => theme::CONTROL_HOVER(),
                2 => theme::CONTROL_PRESSED(),
                _ => theme::CONTROL_BG(),
            };
            (bg, theme::TEXT())
        }
    }
}
