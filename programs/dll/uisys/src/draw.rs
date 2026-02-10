//! Drawing helper functions with dual-mode support.
//!
//! When `win` is a small integer (< SURFACE_THRESHOLD), it's a kernel window ID
//! and we use the old syscall path. When `win >= SURFACE_THRESHOLD`, it's a pointer
//! to a `WinSurface` struct and we draw directly to the pixel buffer using
//! librender (for shapes) and font_bitmap (for text).

use crate::syscall;
use crate::font_bitmap;

/// Threshold: if `win` >= this value, interpret as a pointer to WinSurface.
/// Kernel window IDs are small integers (typically < 1000).
const SURFACE_THRESHOLD: u32 = 0x0100_0000;

/// Surface descriptor for direct pixel-buffer rendering.
/// Apps allocate this (typically on stack) and cast the pointer to u32 for `win`.
#[repr(C)]
pub struct WinSurface {
    pub pixels: *mut u32,
    pub width: u32,
    pub height: u32,
}

// ── librender DLL access (at fixed address 0x04300000) ──────────────

const LIBRENDER_BASE: usize = 0x0430_0000;

/// Partial mirror of librender's export table — only the fields we need.
#[repr(C)]
struct LibrenderExportsPartial {
    _magic: [u8; 4],
    _version: u32,
    _num_exports: u32,
    _pad: u32,
    // offset 16: Surface operations
    fill_rect: extern "C" fn(*mut u32, u32, u32, i32, i32, u32, u32, u32),
    _fill_surface: usize,
    _put_pixel: usize,
    _get_pixel: usize,
    _blit_rect: usize,
    _put_pixel_subpixel: usize,
    // offset 64: Renderer primitives
    _fill_rounded_rect: usize,
    fill_rounded_rect_aa: extern "C" fn(*mut u32, u32, u32, i32, i32, u32, u32, i32, u32),
}

#[inline(always)]
fn librender() -> &'static LibrenderExportsPartial {
    unsafe { &*(LIBRENDER_BASE as *const LibrenderExportsPartial) }
}

#[inline(always)]
fn is_surface(win: u32) -> bool {
    win >= SURFACE_THRESHOLD
}

#[inline(always)]
fn decode_surface(win: u32) -> &'static WinSurface {
    unsafe { &*(win as usize as *const WinSurface) }
}

// ── TTF rendering via kernel syscall ────────────────────────────────

/// Default system font (sfpro.ttf, ID 0) and size for proportional text.
const DEFAULT_FONT_ID: u16 = 0;
const DEFAULT_FONT_SIZE: u16 = 13;

/// Render TTF text onto a WinSurface via SYS_FONT_RENDER_BUF.
fn render_ttf_surface(s: &WinSurface, x: i32, y: i32, color: u32, text: &[u8], font_id: u16, size: u16) {
    if text.is_empty() { return; }
    let mut params = [0u8; 36];
    let buf_ptr = s.pixels as u32;
    params[0..4].copy_from_slice(&buf_ptr.to_le_bytes());
    params[4..8].copy_from_slice(&s.width.to_le_bytes());
    params[8..12].copy_from_slice(&s.height.to_le_bytes());
    params[12..16].copy_from_slice(&x.to_le_bytes());
    params[16..20].copy_from_slice(&y.to_le_bytes());
    params[20..24].copy_from_slice(&color.to_le_bytes());
    params[24..26].copy_from_slice(&font_id.to_le_bytes());
    params[26..28].copy_from_slice(&size.to_le_bytes());
    let text_ptr = text.as_ptr() as u32;
    let text_len = text.len() as u32;
    params[28..32].copy_from_slice(&text_ptr.to_le_bytes());
    params[32..36].copy_from_slice(&text_len.to_le_bytes());
    syscall::font_render_buf(params.as_ptr() as u32);
}

/// Compute length of a null-terminated C string.
unsafe fn c_strlen(s: *const u8) -> usize {
    let mut len = 0;
    while *s.add(len) != 0 { len += 1; }
    len
}

// ── Drawing functions ───────────────────────────────────────────────

/// Fill a rectangle.
#[inline(always)]
pub fn fill_rect(win: u32, x: i32, y: i32, w: u32, h: u32, color: u32) {
    if is_surface(win) {
        let s = decode_surface(win);
        (librender().fill_rect)(s.pixels, s.width, s.height, x, y, w, h, color);
    } else {
        syscall::win_fill_rect(win, x, y, w, h, color);
    }
}

/// Draw a proportional-font string using TTF system font at default size (13px).
#[inline(always)]
pub fn draw_text(win: u32, x: i32, y: i32, color: u32, text: &[u8]) {
    draw_text_sized(win, x, y, color, text, DEFAULT_FONT_SIZE);
}

/// Draw text using TTF system font at the specified pixel size.
pub fn draw_text_sized(win: u32, x: i32, y: i32, color: u32, text: &[u8], size: u16) {
    if is_surface(win) {
        let s = decode_surface(win);
        render_ttf_surface(s, x, y, color, text, DEFAULT_FONT_ID, size);
    } else {
        syscall::win_draw_text(win, x, y, color, text.as_ptr());
    }
}

/// Draw monospace text.
#[inline(always)]
pub fn draw_text_mono(win: u32, x: i32, y: i32, color: u32, text: &[u8]) {
    if is_surface(win) {
        let s = decode_surface(win);
        font_bitmap::draw_text_mono(s.pixels, s.width, s.height, x, y, text.as_ptr(), color);
    } else {
        syscall::win_draw_text_mono(win, x, y, color, text.as_ptr());
    }
}

/// Draw a filled rounded rectangle with AA.
pub fn fill_rounded_rect(win: u32, x: i32, y: i32, w: u32, h: u32, r: u32, color: u32) {
    if is_surface(win) {
        let s = decode_surface(win);
        (librender().fill_rounded_rect_aa)(s.pixels, s.width, s.height, x, y, w, h, r as i32, color);
    } else {
        // Pack params: [x:i16, y:i16, w:u16, h:u16, radius:u16, _pad:u16, color:u32] = 16 bytes
        let params: [u8; 16] = unsafe {
            let mut p = [0u8; 16];
            let px = x as i16;
            let py = y as i16;
            let pw = w as u16;
            let ph = h as u16;
            let pr = r as u16;
            core::ptr::copy_nonoverlapping(px.to_le_bytes().as_ptr(), p.as_mut_ptr(), 2);
            core::ptr::copy_nonoverlapping(py.to_le_bytes().as_ptr(), p.as_mut_ptr().add(2), 2);
            core::ptr::copy_nonoverlapping(pw.to_le_bytes().as_ptr(), p.as_mut_ptr().add(4), 2);
            core::ptr::copy_nonoverlapping(ph.to_le_bytes().as_ptr(), p.as_mut_ptr().add(6), 2);
            core::ptr::copy_nonoverlapping(pr.to_le_bytes().as_ptr(), p.as_mut_ptr().add(8), 2);
            // _pad at offset 10 is already zero
            core::ptr::copy_nonoverlapping(color.to_le_bytes().as_ptr(), p.as_mut_ptr().add(12), 4);
            p
        };
        syscall::win_fill_rounded_rect(win, params.as_ptr() as u32);
    }
}

/// Draw a 1px border rectangle.
pub fn draw_border(win: u32, x: i32, y: i32, w: u32, h: u32, color: u32) {
    fill_rect(win, x, y, w, 1, color);                     // top
    fill_rect(win, x, y + h as i32 - 1, w, 1, color);     // bottom
    fill_rect(win, x, y, 1, h, color);                     // left
    fill_rect(win, x + w as i32 - 1, y, 1, h, color);     // right
}

/// Draw a 1px rounded border following the same arc as fill_rounded_rect.
pub fn draw_rounded_border(win: u32, x: i32, y: i32, w: u32, h: u32, r: u32, color: u32) {
    if r == 0 || w < r * 2 || h < r * 2 {
        draw_border(win, x, y, w, h, color);
        return;
    }
    // Top edge (between corners)
    if w > r * 2 {
        fill_rect(win, x + r as i32, y, w - r * 2, 1, color);
        // Bottom edge
        fill_rect(win, x + r as i32, y + h as i32 - 1, w - r * 2, 1, color);
    }
    // Left edge (between corners)
    if h > r * 2 {
        fill_rect(win, x, y + r as i32, 1, h - r * 2, color);
        // Right edge
        fill_rect(win, x + w as i32 - 1, y + r as i32, 1, h - r * 2, color);
    }
    // Corner arcs — draw only the outermost pixel on each scanline
    let r2x4 = (2 * r as i32) * (2 * r as i32);
    for dy in 0..r {
        let cy = 2 * dy as i32 + 1 - 2 * r as i32;
        let cy2 = cy * cy;
        let mut fill_start = r;
        for dx in 0..r {
            let cx = 2 * dx as i32 + 1 - 2 * r as i32;
            if cx * cx + cy2 <= r2x4 {
                fill_start = dx;
                break;
            }
        }
        if fill_start < r {
            let fs = fill_start as i32;
            // Top-left arc pixel
            fill_rect(win, x + fs, y + dy as i32, 1, 1, color);
            // Top-right arc pixel
            fill_rect(win, x + (w - 1 - fill_start) as i32, y + dy as i32, 1, 1, color);
            // Bottom-left arc pixel
            fill_rect(win, x + fs, y + (h - 1 - dy) as i32, 1, 1, color);
            // Bottom-right arc pixel
            fill_rect(win, x + (w - 1 - fill_start) as i32, y + (h - 1 - dy) as i32, 1, 1, color);
        }
    }
}

/// Draw text with explicit font and size selection.
/// In surface mode, renders TTF text via kernel syscall.
pub fn draw_text_ex(win: u32, x: i32, y: i32, color: u32, font_id: u16, size: u16, text: *const u8) {
    if is_surface(win) {
        let s = decode_surface(win);
        let len = unsafe { c_strlen(text) };
        let text_slice = unsafe { core::slice::from_raw_parts(text, len) };
        render_ttf_surface(s, x, y, color, text_slice, font_id, size);
    } else {
        // Pack params: [x:i16, y:i16, color:u32, font_id:u16, size:u16, text_ptr:u32] = 16 bytes
        let params: [u8; 16] = unsafe {
            let mut p = [0u8; 16];
            let px = x as i16;
            let py = y as i16;
            core::ptr::copy_nonoverlapping(px.to_le_bytes().as_ptr(), p.as_mut_ptr(), 2);
            core::ptr::copy_nonoverlapping(py.to_le_bytes().as_ptr(), p.as_mut_ptr().add(2), 2);
            core::ptr::copy_nonoverlapping(color.to_le_bytes().as_ptr(), p.as_mut_ptr().add(4), 4);
            core::ptr::copy_nonoverlapping(font_id.to_le_bytes().as_ptr(), p.as_mut_ptr().add(8), 2);
            core::ptr::copy_nonoverlapping(size.to_le_bytes().as_ptr(), p.as_mut_ptr().add(10), 2);
            let tp = text as u32;
            core::ptr::copy_nonoverlapping(tp.to_le_bytes().as_ptr(), p.as_mut_ptr().add(12), 4);
            p
        };
        syscall::win_draw_text_ex(win, params.as_ptr() as u32);
    }
}

/// Measure text extent with a specific font.
/// Returns 0 on success, non-zero on error.
/// Note: This always uses the kernel syscall since it doesn't take a `win` parameter.
pub fn measure_text(font_id: u16, size: u16, text: *const u8, text_len: u32, out_w: *mut u32, out_h: *mut u32) -> u32 {
    // Pack params: [font_id:u16, size:u16, text_ptr:u32, text_len:u32, out_w_ptr:u32, out_h_ptr:u32] = 20 bytes
    let params: [u8; 20] = unsafe {
        let mut p = [0u8; 20];
        core::ptr::copy_nonoverlapping(font_id.to_le_bytes().as_ptr(), p.as_mut_ptr(), 2);
        core::ptr::copy_nonoverlapping(size.to_le_bytes().as_ptr(), p.as_mut_ptr().add(2), 2);
        let tp = text as u32;
        core::ptr::copy_nonoverlapping(tp.to_le_bytes().as_ptr(), p.as_mut_ptr().add(4), 4);
        core::ptr::copy_nonoverlapping(text_len.to_le_bytes().as_ptr(), p.as_mut_ptr().add(8), 4);
        let wp = out_w as u32;
        core::ptr::copy_nonoverlapping(wp.to_le_bytes().as_ptr(), p.as_mut_ptr().add(12), 4);
        let hp = out_h as u32;
        core::ptr::copy_nonoverlapping(hp.to_le_bytes().as_ptr(), p.as_mut_ptr().add(16), 4);
        p
    };
    syscall::font_measure(params.as_ptr() as u32)
}

/// Convenience: measure text extent using the default system font.
/// Returns (width, height) in pixels.
pub fn text_size(text: &[u8]) -> (u32, u32) {
    // Strip trailing NUL if present (measure actual visible text)
    let len = if !text.is_empty() && text[text.len() - 1] == 0 {
        text.len() - 1
    } else {
        text.len()
    };
    if len == 0 {
        return (0, DEFAULT_FONT_SIZE as u32);
    }
    let mut w = 0u32;
    let mut h = 0u32;
    measure_text(
        DEFAULT_FONT_ID, DEFAULT_FONT_SIZE,
        text.as_ptr(), len as u32,
        &mut w, &mut h,
    );
    (w, h)
}

/// Convenience: measure text width of the first `n` bytes using the default system font.
pub fn text_width_n(text: &[u8], n: usize) -> u32 {
    if n == 0 { return 0; }
    let len = n.min(text.len());
    let mut w = 0u32;
    let mut h = 0u32;
    measure_text(
        DEFAULT_FONT_ID, DEFAULT_FONT_SIZE,
        text.as_ptr(), len as u32,
        &mut w, &mut h,
    );
    w
}

/// Query whether GPU acceleration is available.
pub fn gpu_has_accel() -> u32 {
    syscall::gpu_has_accel()
}

// --- v2 extern "C" exports ---

/// Exported: fill a rounded rectangle with AA.
pub extern "C" fn fill_rounded_rect_aa(win: u32, x: i32, y: i32, w: u32, h: u32, r: u32, color: u32) {
    fill_rounded_rect(win, x, y, w, h, r, color);
}

/// Exported: draw text with explicit font and size.
pub extern "C" fn draw_text_with_font(win: u32, x: i32, y: i32, color: u32, size: u32, font_id: u16, text: *const u8, text_len: u32) {
    if is_surface(win) {
        let s = decode_surface(win);
        let text_slice = unsafe { core::slice::from_raw_parts(text, text_len as usize) };
        render_ttf_surface(s, x, y, color, text_slice, font_id, size as u16);
    } else {
        draw_text_ex(win, x, y, color, font_id, size as u16, text);
    }
}

/// Exported: measure text with a specific font.
pub extern "C" fn font_measure_export(font_id: u32, size: u16, text: *const u8, text_len: u32, out_w: *mut u32, out_h: *mut u32) -> u32 {
    measure_text(font_id as u16, size, text, text_len, out_w, out_h)
}

/// Exported: query GPU acceleration availability.
pub extern "C" fn gpu_has_accel_export() -> u32 {
    gpu_has_accel()
}

/// Integer square root.
fn _isqrt(n: u32) -> u32 {
    if n == 0 { return 0; }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}
