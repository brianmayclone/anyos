//! Alert — modal dialog with title, message, and action buttons.

use crate::draw;
use crate::theme;

/// Corner radius for the alert dialog.
const ALERT_CORNER: u32 = 10;
/// Padding inside the alert.
const ALERT_PADDING: u32 = 20;

/// Render the alert dialog background, title, and message.
pub extern "C" fn alert_render(
    win: u32, x: i32, y: i32, w: u32, h: u32,
    title: *const u8, title_len: u32,
    message: *const u8, message_len: u32,
) {
    // Shadow (offset dark rect behind the dialog)
    draw::fill_rounded_rect(win, x + 2, y + 2, w, h, ALERT_CORNER, 0xFF0A0A0A);

    // Dialog background
    draw::fill_rounded_rect(win, x, y, w, h, ALERT_CORNER, theme::CARD_BG());

    // Border
    draw::draw_border(win, x, y, w, h, theme::CARD_BORDER());

    // Title
    if !title.is_null() && title_len > 0 {
        let title_slice = unsafe { core::slice::from_raw_parts(title, title_len as usize) };
        let (tw, _) = draw::text_size(title_slice);
        let text_x = x + (w as i32 - tw as i32) / 2;
        let text_y = y + ALERT_PADDING as i32;
        draw::draw_text(win, text_x, text_y, theme::TEXT(), title_slice);
    }

    // Message
    if !message.is_null() && message_len > 0 {
        let msg_slice = unsafe { core::slice::from_raw_parts(message, message_len as usize) };
        let (tw, _) = draw::text_size(msg_slice);
        let text_x = x + (w as i32 - tw as i32) / 2;
        let text_y = y + ALERT_PADDING as i32 + 24;
        draw::draw_text(win, text_x, text_y, theme::TEXT_SECONDARY(), msg_slice);
    }
}

/// Render an alert action button.
/// `style`: 0=Default(gray), 1=Primary(blue), 2=Destructive(red)
/// `state`: 0=Normal, 1=Hover, 2=Pressed
pub extern "C" fn alert_render_button(
    win: u32, x: i32, y: i32, w: u32, h: u32,
    label: *const u8, label_len: u32,
    style: u8, state: u8,
) {
    let (bg, fg) = alert_button_colors(style, state);

    // Button background
    draw::fill_rounded_rect(win, x, y, w, h, theme::BUTTON_CORNER, bg);

    // Label (centered)
    if !label.is_null() && label_len > 0 {
        let label_slice = unsafe { core::slice::from_raw_parts(label, label_len as usize) };
        let (tw, th) = draw::text_size(label_slice);
        let text_x = x + (w as i32 - tw as i32) / 2;
        let text_y = y + (h as i32 - th as i32) / 2;
        draw::draw_text(win, text_x, text_y, fg, label_slice);
    }
}

/// Determine button colors based on style and state.
fn alert_button_colors(style: u8, state: u8) -> (u32, u32) {
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
