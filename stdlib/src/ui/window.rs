//! Window manager — create, draw, events, present.

use crate::raw::*;

// Window event types
pub const EVENT_KEY_DOWN: u32 = 1;
pub const EVENT_KEY_UP: u32 = 2;
pub const EVENT_RESIZE: u32 = 3;
pub const EVENT_MOUSE_DOWN: u32 = 4;
pub const EVENT_MOUSE_UP: u32 = 5;
pub const EVENT_MOUSE_MOVE: u32 = 6;
pub const EVENT_WINDOW_CLOSE: u32 = 8;

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
    syscall5(SYS_WIN_CREATE, title_buf.as_ptr() as u64, pos as u64, size as u64, flags as u64, 0)
}

/// Destroy a window.
pub fn destroy(window_id: u32) -> u32 {
    syscall1(SYS_WIN_DESTROY, window_id as u64)
}

/// Set window title.
pub fn set_title(window_id: u32, title: &str) -> u32 {
    let mut buf = [0u8; 129];
    let len = title.len().min(128);
    buf[..len].copy_from_slice(&title.as_bytes()[..len]);
    buf[len] = 0;
    syscall3(SYS_WIN_SET_TITLE, window_id as u64, buf.as_ptr() as u64, 0)
}

/// Poll window event. Returns 1 if event received, 0 if none.
/// Event format: [type:u32, p1:u32, p2:u32, p3:u32, p4:u32]
pub fn get_event(window_id: u32, event: &mut [u32; 5]) -> u32 {
    syscall2(SYS_WIN_GET_EVENT, window_id as u64, event.as_mut_ptr() as u64)
}

/// Fill a rectangle in a window.
pub fn fill_rect(window_id: u32, x: i16, y: i16, w: u16, h: u16, color: u32) -> u32 {
    let mut params = [0u8; 12];
    params[0..2].copy_from_slice(&x.to_le_bytes());
    params[2..4].copy_from_slice(&y.to_le_bytes());
    params[4..6].copy_from_slice(&w.to_le_bytes());
    params[6..8].copy_from_slice(&h.to_le_bytes());
    params[8..12].copy_from_slice(&color.to_le_bytes());
    syscall2(SYS_WIN_FILL_RECT, window_id as u64, params.as_ptr() as u64)
}

/// Draw text in a window.
pub fn draw_text(window_id: u32, x: i16, y: i16, color: u32, text: &str) -> u32 {
    let mut text_buf = [0u8; 257];
    let len = text.len().min(256);
    text_buf[..len].copy_from_slice(&text.as_bytes()[..len]);
    text_buf[len] = 0;

    // params: [x:i16, y:i16, color:u32, text_ptr:u64] = 16 bytes
    // Kernel reads text_ptr as u32 from bytes 8-11 (lower 32 bits, works for <4GB pointers)
    let mut params = [0u8; 16];
    params[0..2].copy_from_slice(&x.to_le_bytes());
    params[2..4].copy_from_slice(&y.to_le_bytes());
    params[4..8].copy_from_slice(&color.to_le_bytes());
    let text_ptr = text_buf.as_ptr() as u64;
    params[8..16].copy_from_slice(&text_ptr.to_le_bytes());
    syscall2(SYS_WIN_DRAW_TEXT, window_id as u64, params.as_ptr() as u64)
}

/// Draw text in a window using the monospace bitmap font (8x16 fixed-width).
/// Suitable for terminal output and code display.
pub fn draw_text_mono(window_id: u32, x: i16, y: i16, color: u32, text: &str) -> u32 {
    let mut text_buf = [0u8; 257];
    let len = text.len().min(256);
    text_buf[..len].copy_from_slice(&text.as_bytes()[..len]);
    text_buf[len] = 0;

    // params: [x:i16, y:i16, color:u32, text_ptr:u64] = 16 bytes
    let mut params = [0u8; 16];
    params[0..2].copy_from_slice(&x.to_le_bytes());
    params[2..4].copy_from_slice(&y.to_le_bytes());
    params[4..8].copy_from_slice(&color.to_le_bytes());
    let text_ptr = text_buf.as_ptr() as u64;
    params[8..16].copy_from_slice(&text_ptr.to_le_bytes());
    syscall2(SYS_WIN_DRAW_TEXT_MONO, window_id as u64, params.as_ptr() as u64)
}

/// Present (flush) window drawing to compositor.
pub fn present(window_id: u32) -> u32 {
    syscall1(SYS_WIN_PRESENT, window_id as u64)
}

/// Get window content size. Returns [width, height].
pub fn get_size(window_id: u32) -> Option<(u32, u32)> {
    let mut buf = [0u32; 2];
    if syscall2(SYS_WIN_GET_SIZE, window_id as u64, buf.as_mut_ptr() as u64) == 0 {
        Some((buf[0], buf[1]))
    } else {
        None
    }
}

/// Blit ARGB pixel data (u32 per pixel, 0xAARRGGBB) to window content surface.
pub fn blit(window_id: u32, x: i16, y: i16, w: u16, h: u16, data: &[u32]) -> u32 {
    // params: [x:i16, y:i16, w:u16, h:u16, data_ptr:u64] = 16 bytes
    // Kernel reads data_ptr as u32 from bytes 8-11 (lower 32 bits, works for <4GB pointers)
    let mut params = [0u8; 16];
    params[0..2].copy_from_slice(&x.to_le_bytes());
    params[2..4].copy_from_slice(&y.to_le_bytes());
    params[4..6].copy_from_slice(&w.to_le_bytes());
    params[6..8].copy_from_slice(&h.to_le_bytes());
    let data_ptr = data.as_ptr() as u64;
    params[8..16].copy_from_slice(&data_ptr.to_le_bytes());
    syscall2(SYS_WIN_BLIT, window_id as u64, params.as_ptr() as u64)
}

/// List open windows. Returns number of windows.
/// Each entry in buf is 64 bytes: [id:u32, title_len:u32, title:56bytes]
pub fn list_windows(buf: &mut [u8]) -> u32 {
    let max_entries = (buf.len() / 64) as u32;
    syscall2(SYS_WIN_LIST, buf.as_mut_ptr() as u64, max_entries as u64)
}

/// Focus/raise a window by ID.
pub fn focus(window_id: u32) -> u32 {
    syscall1(SYS_WIN_FOCUS, window_id as u64)
}

/// Get screen dimensions. Returns (width, height).
pub fn screen_size() -> (u32, u32) {
    let mut buf = [0u32; 2];
    syscall1(SYS_SCREEN_SIZE, buf.as_mut_ptr() as u64);
    (buf[0], buf[1])
}

/// Set display resolution. Returns true on success.
pub fn set_resolution(width: u32, height: u32) -> bool {
    syscall2(SYS_SET_RESOLUTION, width as u64, height as u64) == 0
}

/// List supported display resolutions. Returns a Vec of (width, height).
pub fn list_resolutions() -> alloc::vec::Vec<(u32, u32)> {
    let mut buf = [0u32; 32]; // 16 modes max, 2 u32 each
    let count = syscall2(SYS_LIST_RESOLUTIONS, buf.as_mut_ptr() as u64, (buf.len() * 4) as u64);
    let mut modes = alloc::vec::Vec::new();
    for i in 0..count as usize {
        if i * 2 + 1 < buf.len() {
            modes.push((buf[i * 2], buf[i * 2 + 1]));
        }
    }
    modes
}

/// Load a TTF font from a file path. Returns font_id or None.
pub fn font_load(path: &str) -> Option<u32> {
    let result = syscall2(SYS_FONT_LOAD, path.as_ptr() as u64, path.len() as u64);
    if result == u32::MAX { None } else { Some(result) }
}

/// Unload a previously loaded font.
pub fn font_unload(font_id: u32) {
    syscall1(SYS_FONT_UNLOAD, font_id as u64);
}

/// Measure text dimensions with a specific font and size.
pub fn font_measure(font_id: u16, size: u16, text: &str) -> (u32, u32) {
    let mut out_w: u32 = 0;
    let mut out_h: u32 = 0;
    let mut params = [0u8; 20];
    params[0..2].copy_from_slice(&font_id.to_le_bytes());
    params[2..4].copy_from_slice(&size.to_le_bytes());
    let text_ptr = text.as_ptr() as u32;
    let text_len = text.len() as u32;
    params[4..8].copy_from_slice(&text_ptr.to_le_bytes());
    params[8..12].copy_from_slice(&text_len.to_le_bytes());
    let w_ptr = &mut out_w as *mut u32 as u32;
    let h_ptr = &mut out_h as *mut u32 as u32;
    params[12..16].copy_from_slice(&w_ptr.to_le_bytes());
    params[16..20].copy_from_slice(&h_ptr.to_le_bytes());
    syscall1(SYS_FONT_MEASURE, params.as_ptr() as u64);
    (out_w, out_h)
}

/// Draw text with explicit font_id and size.
pub fn draw_text_ex(window_id: u32, x: i16, y: i16, color: u32, font_id: u16, size: u16, text: &str) -> u32 {
    let mut text_buf = [0u8; 257];
    let len = text.len().min(256);
    text_buf[..len].copy_from_slice(&text.as_bytes()[..len]);
    text_buf[len] = 0;

    // params: [x:i16, y:i16, color:u32, font_id:u16, size:u16, text_ptr:u32] = 16 bytes
    let mut params = [0u8; 16];
    params[0..2].copy_from_slice(&x.to_le_bytes());
    params[2..4].copy_from_slice(&y.to_le_bytes());
    params[4..8].copy_from_slice(&color.to_le_bytes());
    params[8..10].copy_from_slice(&font_id.to_le_bytes());
    params[10..12].copy_from_slice(&size.to_le_bytes());
    let text_ptr = text_buf.as_ptr() as u32;
    params[12..16].copy_from_slice(&text_ptr.to_le_bytes());
    syscall2(SYS_WIN_DRAW_TEXT_EX, window_id as u64, params.as_ptr() as u64)
}

/// Fill a rounded rectangle (auto-AA on hardware-accelerated GPUs).
pub fn fill_rounded_rect(window_id: u32, x: i16, y: i16, w: u16, h: u16, radius: u16, color: u32) -> u32 {
    let mut params = [0u8; 16];
    params[0..2].copy_from_slice(&x.to_le_bytes());
    params[2..4].copy_from_slice(&y.to_le_bytes());
    params[4..6].copy_from_slice(&w.to_le_bytes());
    params[6..8].copy_from_slice(&h.to_le_bytes());
    params[8..10].copy_from_slice(&radius.to_le_bytes());
    params[10..12].copy_from_slice(&0u16.to_le_bytes()); // padding
    params[12..16].copy_from_slice(&color.to_le_bytes());
    syscall2(SYS_WIN_FILL_ROUNDED_RECT, window_id as u64, params.as_ptr() as u64)
}

/// Query if GPU hardware acceleration is available.
pub fn gpu_has_accel() -> bool {
    syscall0(SYS_GPU_HAS_ACCEL) != 0
}

/// Set the desktop wallpaper from decoded ARGB pixel data.
pub fn set_wallpaper(w: u32, h: u32, pixels: &[u32], mode: u32) -> u32 {
    let mut params = [0u8; 20];
    params[0..4].copy_from_slice(&w.to_le_bytes());
    params[4..8].copy_from_slice(&h.to_le_bytes());
    let ptr = pixels.as_ptr() as u32;
    params[8..12].copy_from_slice(&ptr.to_le_bytes());
    let count = (w * h) as u32;
    params[12..16].copy_from_slice(&count.to_le_bytes());
    params[16..20].copy_from_slice(&mode.to_le_bytes());
    syscall1(SYS_SET_WALLPAPER, params.as_ptr() as u64)
}

/// Get GPU driver name. Returns empty string if no GPU driver is registered.
pub fn gpu_name() -> alloc::string::String {
    let mut buf = [0u8; 64];
    let len = syscall2(SYS_GPU_INFO, buf.as_mut_ptr() as u64, buf.len() as u64);
    if len > 0 {
        let actual_len = (len as usize).min(63);
        alloc::string::String::from(
            core::str::from_utf8(&buf[..actual_len]).unwrap_or("Unknown")
        )
    } else {
        alloc::string::String::from("Software (VESA VBE)")
    }
}
