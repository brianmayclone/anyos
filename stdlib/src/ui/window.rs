//! Window manager — create, draw, events, present.

use crate::raw::*;

// Window event types
pub const EVENT_KEY_DOWN: u32 = 1;
pub const EVENT_KEY_UP: u32 = 2;
pub const EVENT_RESIZE: u32 = 3;
pub const EVENT_MOUSE_DOWN: u32 = 4;
pub const EVENT_MOUSE_UP: u32 = 5;
pub const EVENT_MOUSE_MOVE: u32 = 6;

// Window creation flags
pub const WIN_FLAG_NOT_RESIZABLE: u32 = 0x01;
pub const WIN_FLAG_BORDERLESS: u32 = 0x02;
pub const WIN_FLAG_ALWAYS_ON_TOP: u32 = 0x04;

/// Create a window. Returns window_id or u32::MAX.
pub fn create(title: &str, x: u16, y: u16, w: u16, h: u16) -> u32 {
    create_ex(title, x, y, w, h, 0)
}

/// Create a window with flags. Returns window_id or u32::MAX.
/// flags: bit 0 = WIN_FLAG_NOT_RESIZABLE, bit 1 = WIN_FLAG_BORDERLESS, bit 2 = WIN_FLAG_ALWAYS_ON_TOP
pub fn create_ex(title: &str, x: u16, y: u16, w: u16, h: u16, flags: u32) -> u32 {
    let mut title_buf = [0u8; 129];
    let len = title.len().min(128);
    title_buf[..len].copy_from_slice(&title.as_bytes()[..len]);
    title_buf[len] = 0;
    let pos = (x as u32) | ((y as u32) << 16);
    let size = (w as u32) | ((h as u32) << 16);
    syscall5(SYS_WIN_CREATE, title_buf.as_ptr() as u32, pos, size, flags, 0)
}

/// Destroy a window.
pub fn destroy(window_id: u32) -> u32 {
    syscall1(SYS_WIN_DESTROY, window_id)
}

/// Set window title.
pub fn set_title(window_id: u32, title: &str) -> u32 {
    let mut buf = [0u8; 129];
    let len = title.len().min(128);
    buf[..len].copy_from_slice(&title.as_bytes()[..len]);
    buf[len] = 0;
    syscall3(SYS_WIN_SET_TITLE, window_id, buf.as_ptr() as u32, 0)
}

/// Poll window event. Returns 1 if event received, 0 if none.
/// Event format: [type:u32, p1:u32, p2:u32, p3:u32, p4:u32]
pub fn get_event(window_id: u32, event: &mut [u32; 5]) -> u32 {
    syscall2(SYS_WIN_GET_EVENT, window_id, event.as_mut_ptr() as u32)
}

/// Fill a rectangle in a window.
pub fn fill_rect(window_id: u32, x: i16, y: i16, w: u16, h: u16, color: u32) -> u32 {
    let mut params = [0u8; 12];
    params[0..2].copy_from_slice(&x.to_le_bytes());
    params[2..4].copy_from_slice(&y.to_le_bytes());
    params[4..6].copy_from_slice(&w.to_le_bytes());
    params[6..8].copy_from_slice(&h.to_le_bytes());
    params[8..12].copy_from_slice(&color.to_le_bytes());
    syscall2(SYS_WIN_FILL_RECT, window_id, params.as_ptr() as u32)
}

/// Draw text in a window.
pub fn draw_text(window_id: u32, x: i16, y: i16, color: u32, text: &str) -> u32 {
    let mut text_buf = [0u8; 257];
    let len = text.len().min(256);
    text_buf[..len].copy_from_slice(&text.as_bytes()[..len]);
    text_buf[len] = 0;

    let mut params = [0u8; 12];
    params[0..2].copy_from_slice(&x.to_le_bytes());
    params[2..4].copy_from_slice(&y.to_le_bytes());
    params[4..8].copy_from_slice(&color.to_le_bytes());
    let text_ptr = text_buf.as_ptr() as u32;
    params[8..12].copy_from_slice(&text_ptr.to_le_bytes());
    syscall2(SYS_WIN_DRAW_TEXT, window_id, params.as_ptr() as u32)
}

/// Draw text in a window using the monospace bitmap font (8x16 fixed-width).
/// Suitable for terminal output and code display.
pub fn draw_text_mono(window_id: u32, x: i16, y: i16, color: u32, text: &str) -> u32 {
    let mut text_buf = [0u8; 257];
    let len = text.len().min(256);
    text_buf[..len].copy_from_slice(&text.as_bytes()[..len]);
    text_buf[len] = 0;

    let mut params = [0u8; 12];
    params[0..2].copy_from_slice(&x.to_le_bytes());
    params[2..4].copy_from_slice(&y.to_le_bytes());
    params[4..8].copy_from_slice(&color.to_le_bytes());
    let text_ptr = text_buf.as_ptr() as u32;
    params[8..12].copy_from_slice(&text_ptr.to_le_bytes());
    syscall2(SYS_WIN_DRAW_TEXT_MONO, window_id, params.as_ptr() as u32)
}

/// Present (flush) window drawing to compositor.
pub fn present(window_id: u32) -> u32 {
    syscall1(SYS_WIN_PRESENT, window_id)
}

/// Get window content size. Returns [width, height].
pub fn get_size(window_id: u32) -> Option<(u32, u32)> {
    let mut buf = [0u32; 2];
    if syscall2(SYS_WIN_GET_SIZE, window_id, buf.as_mut_ptr() as u32) == 0 {
        Some((buf[0], buf[1]))
    } else {
        None
    }
}

/// Blit ARGB pixel data (u32 per pixel, 0xAARRGGBB) to window content surface.
pub fn blit(window_id: u32, x: i16, y: i16, w: u16, h: u16, data: &[u32]) -> u32 {
    let mut params = [0u8; 12];
    params[0..2].copy_from_slice(&x.to_le_bytes());
    params[2..4].copy_from_slice(&y.to_le_bytes());
    params[4..6].copy_from_slice(&w.to_le_bytes());
    params[6..8].copy_from_slice(&h.to_le_bytes());
    let data_ptr = data.as_ptr() as u32;
    params[8..12].copy_from_slice(&data_ptr.to_le_bytes());
    syscall2(SYS_WIN_BLIT, window_id, params.as_ptr() as u32)
}

/// List open windows. Returns number of windows.
/// Each entry in buf is 64 bytes: [id:u32, title_len:u32, title:56bytes]
pub fn list_windows(buf: &mut [u8]) -> u32 {
    let max_entries = (buf.len() / 64) as u32;
    syscall2(SYS_WIN_LIST, buf.as_mut_ptr() as u32, max_entries)
}

/// Focus/raise a window by ID.
pub fn focus(window_id: u32) -> u32 {
    syscall1(SYS_WIN_FOCUS, window_id)
}

/// Get screen dimensions. Returns (width, height).
pub fn screen_size() -> (u32, u32) {
    let mut buf = [0u32; 2];
    syscall1(SYS_SCREEN_SIZE, buf.as_mut_ptr() as u32);
    (buf[0], buf[1])
}
