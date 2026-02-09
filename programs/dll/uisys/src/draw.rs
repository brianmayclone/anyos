//! Drawing helper functions wrapping syscalls.

use crate::syscall;

/// Fill a rectangle.
#[inline(always)]
pub fn fill_rect(win: u32, x: i32, y: i32, w: u32, h: u32, color: u32) {
    syscall::win_fill_rect(win, x, y, w, h, color);
}

/// Draw a proportional-font string (null-terminated in caller memory).
/// Caller must provide a null-terminated buffer.
#[inline(always)]
pub fn draw_text(win: u32, x: i32, y: i32, color: u32, text: &[u8]) {
    // Text must be null-terminated; use the slice pointer directly.
    // The syscall reads until NUL.
    syscall::win_draw_text(win, x, y, color, text.as_ptr());
}

/// Draw monospace text.
#[inline(always)]
pub fn draw_text_mono(win: u32, x: i32, y: i32, color: u32, text: &[u8]) {
    syscall::win_draw_text_mono(win, x, y, color, text.as_ptr());
}

/// Draw a filled rounded rectangle via kernel AA syscall.
/// The kernel performs anti-aliased corner rendering, giving all components AA for free.
pub fn fill_rounded_rect(win: u32, x: i32, y: i32, w: u32, h: u32, r: u32, color: u32) {
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

/// Measure text extent with a specific font.
/// Returns 0 on success, non-zero on error.
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

/// Query whether GPU acceleration is available.
pub fn gpu_has_accel() -> u32 {
    syscall::gpu_has_accel()
}

// --- v2 extern "C" exports ---

/// Exported: fill a rounded rectangle with kernel-side AA.
/// Same signature as internal `fill_rounded_rect` but exposed as a named export.
pub extern "C" fn fill_rounded_rect_aa(win: u32, x: i32, y: i32, w: u32, h: u32, r: u32, color: u32) {
    fill_rounded_rect(win, x, y, w, h, r, color);
}

/// Exported: draw text with explicit font and size.
pub extern "C" fn draw_text_with_font(win: u32, x: i32, y: i32, color: u32, size: u32, font_id: u16, text: *const u8, _text_len: u32) {
    draw_text_ex(win, x, y, color, font_id, size as u16, text);
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
fn isqrt(n: u32) -> u32 {
    if n == 0 { return 0; }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}
