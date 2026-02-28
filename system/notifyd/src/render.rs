//! Notification banner rendering — iOS-style notification cards.
//!
//! Each banner: rounded rect background with 1px border, optional 16×16 icon,
//! title (bold 13px), "now" label (11px right-aligned), and message (11px, max 2 lines).

use crate::framebuffer::Framebuffer;
use crate::Notification;

// ── Layout Constants ────────────────────────────────────────────────────────

/// Banner width in logical pixels.
pub const BANNER_W: u32 = 320;
/// Banner height in logical pixels.
pub const BANNER_H: u32 = 72;
/// Corner radius.
pub const BANNER_RADIUS: i32 = 12;
/// Gap between stacked banners.
pub const STACK_GAP: u32 = 8;
/// Margin from top of canvas to first banner.
pub const MARGIN_TOP: u32 = 8;
/// Margin from the right edge of the canvas to the banner.
pub const MARGIN_RIGHT: u32 = 12;
/// Maximum number of visible banners.
pub const MAX_VISIBLE: usize = 4;

// ── Font IDs (match libfont registry) ───────────────────────────────────────

const FONT_REGULAR: u16 = 0;  // SF Pro
const FONT_BOLD: u16 = 1;     // SF Pro Bold

// ── Theme Colors ────────────────────────────────────────────────────────────

/// Check if light theme is active.
fn is_light() -> bool {
    libanyui_client::theme::is_light()
}

/// Banner background color (semi-transparent).
fn color_banner_bg() -> u32 {
    if is_light() { 0xF0F5F5F7 } else { 0xF02C2C2E }
}

/// Banner border color.
fn color_banner_border() -> u32 {
    if is_light() { 0x30000000 } else { 0x30FFFFFF }
}

/// Title text color.
fn color_title() -> u32 {
    if is_light() { 0xFF1C1C1E } else { 0xFFFFFFFF }
}

/// Message text color.
fn color_message() -> u32 {
    if is_light() { 0xFF3C3C43 } else { 0xFFAEAEB2 }
}

/// "now" label color.
fn color_timestamp() -> u32 {
    if is_light() { 0xFF8E8E93 } else { 0xFF8E8E93 }
}

// ── Rendering ───────────────────────────────────────────────────────────────

/// Render all visible notifications into the framebuffer.
/// Each banner slides horizontally (x_offset) and is stacked vertically by slot (y_pos).
pub fn render_all(fb: &mut Framebuffer, notifications: &[Notification]) {
    fb.clear();

    for notif in notifications.iter().take(MAX_VISIBLE) {
        if !notif.visible { continue; }
        render_banner(fb, notif, notif.x_offset, notif.y_pos());
    }
}

/// Render a single notification banner at the given position.
fn render_banner(fb: &mut Framebuffer, notif: &Notification, x: i32, y: i32) {
    let w = BANNER_W;
    let h = BANNER_H;

    // Background
    fb.fill_rounded_rect(x, y, w, h, BANNER_RADIUS, color_banner_bg());

    // 1px border outline
    fb.stroke_rounded_rect(x, y, w, h, BANNER_RADIUS, color_banner_border());

    // Content area starts after padding
    let pad_x = 12i32;
    let pad_y = 10i32;
    let mut cx = x + pad_x;
    let cy = y + pad_y;

    // Icon (16×16, left-aligned)
    if let Some(ref icon) = notif.icon {
        fb.blit_icon_16(icon, cx, cy);
        cx += 16 + 8; // icon width + gap
    }

    // Title (bold, 13px) — single line
    let title_y = cy;
    let title = if notif.title_len > 0 {
        core::str::from_utf8(&notif.title[..notif.title_len]).unwrap_or("")
    } else {
        ""
    };
    if !title.is_empty() {
        fb.draw_text(FONT_BOLD, 13, cx, title_y, color_title(), title);
    }

    // "now" label (right-aligned, same line as title)
    let now_text = "now";
    let (now_w, _) = anyos_std::ui::window::font_measure(FONT_REGULAR, 11, now_text);
    let now_x = x + w as i32 - pad_x - now_w as i32;
    fb.draw_text(FONT_REGULAR, 11, now_x, title_y + 1, color_timestamp(), now_text);

    // Message (regular, 11px) — below title, up to 2 lines
    let msg = if notif.msg_len > 0 {
        core::str::from_utf8(&notif.msg[..notif.msg_len]).unwrap_or("")
    } else {
        ""
    };
    if !msg.is_empty() {
        let msg_y = title_y + 18;
        let max_w = (w as i32 - pad_x * 2 - if notif.icon.is_some() { 24 } else { 0 }) as u32;

        // Simple line wrapping: find last space within max_w for first line
        let (line1, line2) = wrap_text(msg, FONT_REGULAR, 11, max_w);
        fb.draw_text(FONT_REGULAR, 11, cx, msg_y, color_message(), line1);
        if !line2.is_empty() {
            fb.draw_text(FONT_REGULAR, 11, cx, msg_y + 16, color_message(), line2);
        }
    }
}

/// Simple word-wrap: split text into at most 2 lines that fit within `max_w` pixels.
/// Returns (line1, line2) where line2 may be empty.
fn wrap_text<'a>(text: &'a str, font_id: u16, font_size: u16, max_w: u32) -> (&'a str, &'a str) {
    let (full_w, _) = anyos_std::ui::window::font_measure(font_id, font_size, text);
    if (full_w as u32) <= max_w {
        return (text, "");
    }

    // Find the last space that keeps line 1 within max_w
    let bytes = text.as_bytes();
    let mut best_split = 0;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b' ' {
            let prefix = &text[..i];
            let (pw, _) = anyos_std::ui::window::font_measure(font_id, font_size, prefix);
            if (pw as u32) <= max_w {
                best_split = i;
            } else {
                break;
            }
        }
    }

    if best_split == 0 {
        // No space found — hard break at max_w
        for i in (1..text.len()).rev() {
            if text.is_char_boundary(i) {
                let prefix = &text[..i];
                let (pw, _) = anyos_std::ui::window::font_measure(font_id, font_size, prefix);
                if (pw as u32) <= max_w {
                    return (prefix, &text[i..]);
                }
            }
        }
        (text, "")
    } else {
        let line1 = &text[..best_split];
        let rest = &text[best_split + 1..]; // skip the space
        // Truncate line 2 if too long (add "..." ellipsis)
        (line1, rest)
    }
}
