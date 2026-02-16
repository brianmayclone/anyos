//! Drawing helper functions for direct pixel-buffer rendering.
//!
//! All drawing operates on a `WinSurface` (pointer to an ARGB pixel buffer).
//! Shape rendering uses librender.dlib, text rendering uses libfont.dlib.
//! No kernel syscalls are needed for any drawing operations.

use crate::font_bitmap;

/// Surface descriptor for direct pixel-buffer rendering.
/// Apps allocate this (typically on stack) and cast the pointer to u32 for `win`.
#[repr(C)]
pub struct WinSurface {
    pub pixels: *mut u32,
    pub width: u32,
    pub height: u32,
}

#[inline(always)]
fn decode_surface(win: u32) -> &'static WinSurface {
    unsafe { &*(win as usize as *const WinSurface) }
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

// ── libfont DLL access (at fixed address 0x04200000) ────────────────

const LIBFONT_BASE: usize = 0x0420_0000;

/// Partial mirror of libfont's export table — only the fields we need.
#[repr(C)]
struct LibfontExportsPartial {
    _magic: [u8; 4],
    _version: u32,
    _num_exports: u32,
    _pad: u32,
    // offset 16
    _init: usize,
    _load_font: usize,
    _unload_font: usize,
    // offset 40
    measure_string: extern "C" fn(u32, u16, *const u8, u32, *mut u32, *mut u32),
    // offset 48
    draw_string_buf: extern "C" fn(*mut u32, u32, u32, i32, i32, u32, u32, u16, *const u8, u32),
}

#[inline(always)]
fn libfont() -> &'static LibfontExportsPartial {
    unsafe { &*(LIBFONT_BASE as *const LibfontExportsPartial) }
}

// ── Direct kernel syscall for GPU accel query ───────────────────────

const SYS_GPU_HAS_ACCEL: u32 = 135;

#[inline(always)]
fn syscall0(num: u32) -> u32 {
    let ret: u64;
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") num as u64 => ret,
            out("rcx") _,
            out("r11") _,
        );
    }
    ret as u32
}

// ── TTF rendering via libfont DLL ───────────────────────────────────

/// Default system font (sfpro.ttf, ID 0) and size for proportional text.
const DEFAULT_FONT_ID: u16 = 0;
const DEFAULT_FONT_SIZE: u16 = 13;

/// Render TTF text onto a WinSurface via libfont.dlib.
#[inline(always)]
fn render_ttf_surface(s: &WinSurface, x: i32, y: i32, color: u32, text: &[u8], font_id: u16, size: u16) {
    if text.is_empty() { return; }
    (libfont().draw_string_buf)(
        s.pixels, s.width, s.height,
        x, y, color,
        font_id as u32, size,
        text.as_ptr(), text.len() as u32,
    );
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
    let s = decode_surface(win);
    (librender().fill_rect)(s.pixels, s.width, s.height, x, y, w, h, color);
}

/// Draw a proportional-font string using TTF system font at default size (13px).
#[inline(always)]
pub fn draw_text(win: u32, x: i32, y: i32, color: u32, text: &[u8]) {
    draw_text_sized(win, x, y, color, text, DEFAULT_FONT_SIZE);
}

/// Draw text using TTF system font at the specified pixel size.
pub fn draw_text_sized(win: u32, x: i32, y: i32, color: u32, text: &[u8], size: u16) {
    let s = decode_surface(win);
    render_ttf_surface(s, x, y, color, text, DEFAULT_FONT_ID, size);
}

/// Draw monospace text.
#[inline(always)]
pub fn draw_text_mono(win: u32, x: i32, y: i32, color: u32, text: &[u8]) {
    let s = decode_surface(win);
    font_bitmap::draw_text_mono(s.pixels, s.width, s.height, x, y, text.as_ptr(), color);
}

/// Draw a filled rounded rectangle with AA.
pub fn fill_rounded_rect(win: u32, x: i32, y: i32, w: u32, h: u32, r: u32, color: u32) {
    let s = decode_surface(win);
    (librender().fill_rounded_rect_aa)(s.pixels, s.width, s.height, x, y, w, h, r as i32, color);
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
pub fn draw_text_ex(win: u32, x: i32, y: i32, color: u32, font_id: u16, size: u16, text: *const u8) {
    let s = decode_surface(win);
    let len = unsafe { c_strlen(text) };
    let text_slice = unsafe { core::slice::from_raw_parts(text, len) };
    render_ttf_surface(s, x, y, color, text_slice, font_id, size);
}

/// Measure text extent with a specific font via libfont.dlib.
pub fn measure_text(font_id: u16, size: u16, text: *const u8, text_len: u32, out_w: *mut u32, out_h: *mut u32) -> u32 {
    (libfont().measure_string)(font_id as u32, size, text, text_len, out_w, out_h);
    0
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

/// Query whether GPU acceleration is available (direct kernel syscall).
pub fn gpu_has_accel() -> u32 {
    syscall0(SYS_GPU_HAS_ACCEL)
}

// --- v2 extern "C" exports ---

/// Exported: fill a rounded rectangle with AA.
pub extern "C" fn fill_rounded_rect_aa(win: u32, x: i32, y: i32, w: u32, h: u32, r: u32, color: u32) {
    fill_rounded_rect(win, x, y, w, h, r, color);
}

/// Exported: draw text with explicit font and size.
pub extern "C" fn draw_text_with_font(win: u32, x: i32, y: i32, color: u32, size: u32, font_id: u16, text: *const u8, text_len: u32) {
    let s = decode_surface(win);
    let text_slice = unsafe { core::slice::from_raw_parts(text, text_len as usize) };
    render_ttf_surface(s, x, y, color, text_slice, font_id, size as u16);
}

/// Exported: measure text with a specific font.
pub extern "C" fn font_measure_export(font_id: u32, size: u16, text: *const u8, text_len: u32, out_w: *mut u32, out_h: *mut u32) -> u32 {
    measure_text(font_id as u16, size, text, text_len, out_w, out_h)
}

/// Exported: query GPU acceleration availability.
pub extern "C" fn gpu_has_accel_export() -> u32 {
    gpu_has_accel()
}
