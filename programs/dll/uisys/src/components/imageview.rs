//! ImageView — displays a region for pixel data with a placeholder frame.
//!
//! The actual pixel blit is performed via a direct syscall by the caller.
//! This component renders the frame, background, and informational overlay.

use crate::draw;
use crate::theme;

/// Render the image view frame and background.
/// `data_ptr`: pointer to ARGB pixel data (informational; not blitted here).
/// `data_w`, `data_h`: dimensions of the source image.
///
/// If `data_ptr` is null, a placeholder is rendered instead.
pub extern "C" fn imageview_render(
    win: u32, x: i32, y: i32, w: u32, h: u32,
    data_ptr: *const u32, data_w: u32, data_h: u32,
) {
    // Background (dark fill behind image area)
    draw::fill_rect(win, x, y, w, h, theme::WINDOW_BG);

    // Border
    draw::draw_border(win, x, y, w, h, theme::CARD_BORDER);

    if data_ptr.is_null() || data_w == 0 || data_h == 0 {
        // Placeholder: draw a centered "No Image" label
        let label = b"No Image\0";
        let (tw, th) = draw::text_size(b"No Image");
        let text_x = x + (w as i32 - tw as i32) / 2;
        let text_y = y + (h as i32 - th as i32) / 2;
        draw::draw_text(win, text_x, text_y, theme::TEXT_DISABLED, label);

        // Cross lines for empty placeholder
        // Horizontal center line
        draw::fill_rect(win, x + 4, y + h as i32 / 2, w - 8, 1, theme::SEPARATOR);
        // Vertical center line
        draw::fill_rect(win, x + w as i32 / 2, y + 4, 1, h - 8, theme::SEPARATOR);
    }
    // When data_ptr is valid, caller is responsible for blitting pixels
    // via the appropriate window syscall after this frame is drawn.
}
