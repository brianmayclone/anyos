//! Pre-auth login screen renderer for the VNC daemon.
//!
//! Renders a 640×480 ARGB login dialog into a caller-supplied pixel buffer.
//! The rendering is fully self-contained: it uses only `font.rs` for text and
//! does not depend on the compositor or any other OS service.
//!
//! # Layout (640×480)
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │                   (dark background)                      │
//! │         ┌─────────────────────────────────┐             │
//! │         │         anyOS VNC               │  (box)      │
//! │         │  Username: ________             │             │
//! │         │  Password: ********             │             │
//! │         │  [error message line]           │             │
//! │         └─────────────────────────────────┘             │
//! │                                                          │
//! └─────────────────────────────────────────────────────────┘
//! ```

use crate::font;

/// Width of the login framebuffer in pixels.
pub const LOGIN_W: usize = 640;
/// Height of the login framebuffer in pixels.
pub const LOGIN_H: usize = 480;

/// Total pixel count for the login screen.
pub const LOGIN_PIXELS: usize = LOGIN_W * LOGIN_H;

// ── Color palette ─────────────────────────────────────────────────────────────

const BG_DARK: u32 = 0xFF1C1C1E;     // macOS dark background
const BOX_BG: u32 = 0xFF2C2C2E;      // dialog box background
const BOX_BORDER: u32 = 0xFF3A3A3C;  // box border
const TEXT_WHITE: u32 = 0xFFFFFFFF;  // primary text
const TEXT_GRAY: u32 = 0xFF8E8E93;   // label text
const TEXT_RED: u32 = 0xFFFF453A;    // error message
const TEXT_BLUE: u32 = 0xFF0A84FF;   // title accent
const CURSOR_COLOR: u32 = 0xFF0A84FF; // text cursor

// ── Box geometry ──────────────────────────────────────────────────────────────

const BOX_W: usize = 360;
const BOX_H: usize = 220;
const BOX_X: usize = (LOGIN_W - BOX_W) / 2;  // 140
const BOX_Y: usize = (LOGIN_H - BOX_H) / 2;  // 130

/// State passed to [`render`] each time a new frame is needed.
pub struct LoginState<'a> {
    /// Username typed so far (byte slice, not null-terminated).
    pub username: &'a [u8],
    /// Number of password characters typed (displayed as `*`s).
    pub password_len: usize,
    /// Whether the cursor is currently in the username field.
    pub cursor_in_username: bool,
    /// Whether the cursor blink phase is "on".
    pub cursor_visible: bool,
    /// Optional error message (empty → no error shown).
    pub error_msg: &'a [u8],
}

// ── Rendering helpers ─────────────────────────────────────────────────────────

/// Fill a rectangle in an ARGB framebuffer.
fn fill_rect(fb: &mut [u32], stride: usize, x: usize, y: usize, w: usize, h: usize, color: u32) {
    for row in y..y.saturating_add(h) {
        for col in x..x.saturating_add(w) {
            let idx = row * stride + col;
            if idx < fb.len() {
                fb[idx] = color;
            }
        }
    }
}

/// Draw a 1-pixel border around a rectangle.
fn draw_border(fb: &mut [u32], stride: usize, x: usize, y: usize, w: usize, h: usize, color: u32) {
    // Top and bottom.
    fill_rect(fb, stride, x, y, w, 1, color);
    fill_rect(fb, stride, x, y + h - 1, w, 1, color);
    // Left and right.
    fill_rect(fb, stride, x, y, 1, h, color);
    fill_rect(fb, stride, x + w - 1, y, 1, h, color);
}

/// Draw a text input field box and its content.
///
/// Returns the x coordinate of the end of the typed text (for cursor placement).
fn draw_field(
    fb: &mut [u32],
    x: usize,
    y: usize,
    w: usize,
    content: &[u8],
    masked: bool,       // true → show asterisks
    has_cursor: bool,
    cursor_visible: bool,
) -> usize {
    let field_h = font::GLYPH_H + 6; // 14 px

    // Field background.
    fill_rect(fb, LOGIN_W, x, y, w, field_h, 0xFF1C1C1E);
    draw_border(fb, LOGIN_W, x, y, w, field_h, BOX_BORDER);

    // Render text (or asterisks) inside the field.
    let text_x = x + 4;
    let text_y = y + 3;
    let max_visible = (w - 8) / font::GLYPH_W;

    // For masked fields build an asterisk buffer.
    let mut star_buf = [b'*'; 128];
    let display = if masked {
        let n = content.len().min(128);
        &star_buf[..n]
    } else {
        &content[..content.len().min(max_visible)]
    };

    let end_x = font::draw_str(fb, LOGIN_W, display, text_x, text_y, TEXT_WHITE, 0xFF1C1C1E);

    // Cursor.
    if has_cursor && cursor_visible {
        fill_rect(fb, LOGIN_W, end_x, text_y, 2, font::GLYPH_H, CURSOR_COLOR);
    }

    end_x
}

// ── Public render function ────────────────────────────────────────────────────

/// Render the full login screen into `fb` (must be at least `LOGIN_PIXELS` elements).
///
/// Call this whenever state changes (keystroke, cursor blink).
pub fn render(fb: &mut [u32], state: &LoginState<'_>) {
    if fb.len() < LOGIN_PIXELS {
        return;
    }

    // 1. Dark background.
    fill_rect(fb, LOGIN_W, 0, 0, LOGIN_W, LOGIN_H, BG_DARK);

    // 2. Dialog box.
    fill_rect(fb, LOGIN_W, BOX_X, BOX_Y, BOX_W, BOX_H, BOX_BG);
    draw_border(fb, LOGIN_W, BOX_X, BOX_Y, BOX_W, BOX_H, BOX_BORDER);

    // 3. Title "anyOS VNC" centered at the top of the box.
    let title = b"anyOS VNC";
    let title_px = title.len() * font::GLYPH_W;
    let title_x = BOX_X + (BOX_W - title_px) / 2;
    let title_y = BOX_Y + 18;
    font::draw_str(fb, LOGIN_W, title, title_x, title_y, TEXT_BLUE, BOX_BG);

    // Separator line below title.
    fill_rect(fb, LOGIN_W, BOX_X + 16, title_y + font::GLYPH_H + 6, BOX_W - 32, 1, BOX_BORDER);

    // 4. "Username:" label.
    let label_x = BOX_X + 24;
    let field_x = label_x + 10 * font::GLYPH_W; // after "Username: "
    let field_w = BOX_W - 24 - 10 * font::GLYPH_W - 10;

    let uname_label_y = title_y + font::GLYPH_H + 18;
    font::draw_str(fb, LOGIN_W, b"Username:", label_x, uname_label_y, TEXT_GRAY, BOX_BG);
    draw_field(
        fb,
        field_x,
        uname_label_y - 3,
        field_w,
        state.username,
        false,
        state.cursor_in_username,
        state.cursor_visible,
    );

    // 5. "Password:" label.
    let pass_label_y = uname_label_y + font::GLYPH_H + 20;
    font::draw_str(fb, LOGIN_W, b"Password:", label_x, pass_label_y, TEXT_GRAY, BOX_BG);

    // Build a slice of the right length for the masked field.
    let star_buf = [b'*'; 128];
    let pw_display = &star_buf[..state.password_len.min(128)];
    draw_field(
        fb,
        field_x,
        pass_label_y - 3,
        field_w,
        pw_display,
        false, // already masked above
        !state.cursor_in_username,
        state.cursor_visible,
    );

    // 6. Error message (if any).
    if !state.error_msg.is_empty() {
        let err_y = pass_label_y + font::GLYPH_H + 20;
        let err_px = state.error_msg.len() * font::GLYPH_W;
        let err_x = if err_px < BOX_W { BOX_X + (BOX_W - err_px) / 2 } else { BOX_X + 8 };
        font::draw_str(fb, LOGIN_W, state.error_msg, err_x, err_y, TEXT_RED, BOX_BG);
    }

    // 7. Hint text at the bottom of the box.
    let hint = b"Press Enter to login, Esc to cancel";
    let hint_px = hint.len() * font::GLYPH_W;
    let hint_x = if hint_px < BOX_W { BOX_X + (BOX_W - hint_px) / 2 } else { BOX_X + 4 };
    let hint_y = BOX_Y + BOX_H - font::GLYPH_H - 10;
    font::draw_str(fb, LOGIN_W, hint, hint_x, hint_y, TEXT_GRAY, BOX_BG);
}
