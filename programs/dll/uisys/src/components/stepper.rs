//! Stepper — plus/minus increment buttons with value display.

use crate::draw;
use crate::theme;

const STEPPER_BTN_W: u32 = 28;
const STEPPER_BTN_H: u32 = 28;
const VALUE_WIDTH: u32 = 40;
const STEPPER_CORNER: u32 = 6;

/// Render a stepper control: [ - ] value [ + ]
/// value: current integer value.
/// min, max: allowed range (used to gray out buttons at limits).
pub extern "C" fn stepper_render(
    win: u32, x: i32, y: i32,
    value: i32, min: i32, max: i32,
) {
    let minus_enabled = value > min;
    let plus_enabled = value < max;

    // Minus button
    let minus_bg = if minus_enabled { theme::CONTROL_BG() } else { theme::CONTROL_PRESSED() };
    let minus_fg = if minus_enabled { theme::TEXT() } else { theme::TEXT_DISABLED() };
    draw::fill_rounded_rect(win, x, y, STEPPER_BTN_W, STEPPER_BTN_H, STEPPER_CORNER, minus_bg);
    // Draw "-" sign
    let dash_w = 10u32;
    let dash_x = x + (STEPPER_BTN_W as i32 - dash_w as i32) / 2;
    let dash_y = y + (STEPPER_BTN_H as i32 / 2);
    draw::fill_rect(win, dash_x, dash_y, dash_w, 2, minus_fg);

    // Value display
    let val_x = x + STEPPER_BTN_W as i32;
    draw::fill_rect(win, val_x, y, VALUE_WIDTH, STEPPER_BTN_H, theme::INPUT_BG());
    // Format value
    let mut buf = [0u8; 16];
    let text_len = format_i32(value, &mut buf);
    let (text_w, th) = draw::text_size(&buf[..text_len]);
    let text_x = val_x + (VALUE_WIDTH as i32 - text_w as i32) / 2;
    let text_y = y + (STEPPER_BTN_H as i32 - th as i32) / 2;
    draw::draw_text(win, text_x, text_y, theme::TEXT(), &buf[..text_len + 1]);

    // Plus button
    let plus_x = x + STEPPER_BTN_W as i32 + VALUE_WIDTH as i32;
    let plus_bg = if plus_enabled { theme::CONTROL_BG() } else { theme::CONTROL_PRESSED() };
    let plus_fg = if plus_enabled { theme::TEXT() } else { theme::TEXT_DISABLED() };
    draw::fill_rounded_rect(win, plus_x, y, STEPPER_BTN_W, STEPPER_BTN_H, STEPPER_CORNER, plus_bg);
    // Draw "+" sign
    let bar_w = 10u32;
    let bar_h = 2u32;
    let cx = plus_x + (STEPPER_BTN_W as i32 - bar_w as i32) / 2;
    let cy = y + STEPPER_BTN_H as i32 / 2;
    draw::fill_rect(win, cx, cy, bar_w, bar_h, plus_fg);
    // Vertical bar of "+"
    let vx = plus_x + (STEPPER_BTN_W as i32 / 2);
    let vy = y + (STEPPER_BTN_H as i32 - bar_w as i32) / 2;
    draw::fill_rect(win, vx, vy, bar_h, bar_w, plus_fg);
}

/// Hit test for the plus button. Returns 1 if (mx,my) is on "+".
pub extern "C" fn stepper_hit_test_plus(x: i32, y: i32, mx: i32, my: i32) -> u32 {
    let plus_x = x + STEPPER_BTN_W as i32 + VALUE_WIDTH as i32;
    if mx >= plus_x && mx < plus_x + STEPPER_BTN_W as i32
        && my >= y && my < y + STEPPER_BTN_H as i32
    {
        1
    } else {
        0
    }
}

/// Hit test for the minus button. Returns 1 if (mx,my) is on "-".
pub extern "C" fn stepper_hit_test_minus(x: i32, y: i32, mx: i32, my: i32) -> u32 {
    if mx >= x && mx < x + STEPPER_BTN_W as i32
        && my >= y && my < y + STEPPER_BTN_H as i32
    {
        1
    } else {
        0
    }
}

/// Format an i32 into a decimal string (null-terminated).
/// Returns the number of characters written (not counting NUL).
fn format_i32(val: i32, buf: &mut [u8; 16]) -> usize {
    if val == 0 {
        buf[0] = b'0';
        buf[1] = 0;
        return 1;
    }
    let negative = val < 0;
    let mut abs_val = if negative {
        if val == i32::MIN {
            2147483648u32
        } else {
            (-val) as u32
        }
    } else {
        val as u32
    };

    let mut tmp = [0u8; 11];
    let mut i = 0usize;
    while abs_val > 0 && i < 11 {
        tmp[i] = b'0' + (abs_val % 10) as u8;
        abs_val /= 10;
        i += 1;
    }

    let mut pos = 0usize;
    if negative {
        buf[pos] = b'-';
        pos += 1;
    }
    for j in 0..i {
        buf[pos] = tmp[i - 1 - j];
        pos += 1;
    }
    buf[pos] = 0;
    pos
}
