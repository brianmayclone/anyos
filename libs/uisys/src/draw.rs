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
const SYS_GPU_HAS_HW_CURSOR: u32 = 138;

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

/// Query whether GPU hardware cursor is available (direct kernel syscall).
pub fn gpu_has_hw_cursor() -> u32 {
    syscall0(SYS_GPU_HAS_HW_CURSOR)
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

// ── Shadow rendering ─────────────────────────────────────────────────

/// Integer square root (Newton's method).
#[inline]
fn isqrt_u32(n: u32) -> u32 {
    if n == 0 { return 0; }
    let mut x = 1u32 << ((32 - n.leading_zeros() + 1) / 2);
    loop {
        let nx = (x + n / x) / 2;
        if nx >= x { return x; }
        x = nx;
    }
}

/// Integer square root for u64.
#[inline]
fn isqrt_u64(n: u64) -> u64 {
    if n == 0 { return 0; }
    let mut x = 1u64 << ((64 - n.leading_zeros() + 1) / 2);
    loop {
        let nx = (x + n / x) / 2;
        if nx >= x { return x; }
        x = nx;
    }
}

/// Alpha-blend a shadow pixel (pure black with given alpha) onto a destination pixel.
#[inline(always)]
fn shadow_blend(alpha: u32, dst: u32) -> u32 {
    if alpha == 0 { return dst; }
    let da = (dst >> 24) & 0xFF;
    let dr = (dst >> 16) & 0xFF;
    let dg = (dst >> 8) & 0xFF;
    let db = dst & 0xFF;
    let inv = 255 - alpha;
    let or = dr * inv / 255;
    let og = dg * inv / 255;
    let ob = db * inv / 255;
    let oa = alpha + da * inv / 255;
    (oa << 24) | (or << 16) | (og << 8) | ob
}

/// Signed distance from point (px,py) to an axis-aligned rectangle at (rx,ry,rw,rh).
/// Negative inside, positive outside.
#[inline]
fn rect_sdf(px: i32, py: i32, rx: i32, ry: i32, rw: i32, rh: i32) -> i32 {
    let dx = if px < rx { rx - px } else if px >= rx + rw { px - (rx + rw - 1) } else { 0 };
    let dy = if py < ry { ry - py } else if py >= ry + rh { py - (ry + rh - 1) } else { 0 };
    if dx == 0 && dy == 0 {
        let to_left = px - rx;
        let to_right = rx + rw - 1 - px;
        let to_top = py - ry;
        let to_bottom = ry + rh - 1 - py;
        -to_left.min(to_right).min(to_top).min(to_bottom)
    } else if dx > 0 && dy > 0 {
        isqrt_u32((dx * dx + dy * dy) as u32) as i32
    } else {
        dx.max(dy)
    }
}

/// Signed distance from point (px,py) to a rounded rectangle.
/// Negative inside, positive outside.
#[inline]
fn rounded_rect_sdf(px: i32, py: i32, rx: i32, ry: i32, rw: i32, rh: i32, r: i32) -> i32 {
    let r = r.min(rw / 2).min(rh / 2).max(0);
    let inner_x0 = rx + r;
    let inner_y0 = ry + r;
    let inner_x1 = rx + rw - r;
    let inner_y1 = ry + rh - r;
    let dx = if px < inner_x0 { inner_x0 - px } else if px >= inner_x1 { px - inner_x1 + 1 } else { 0 };
    let dy = if py < inner_y0 { inner_y0 - py } else if py >= inner_y1 { py - inner_y1 + 1 } else { 0 };
    if dx == 0 && dy == 0 {
        let to_left = px - rx;
        let to_right = rx + rw - 1 - px;
        let to_top = py - ry;
        let to_bottom = ry + rh - 1 - py;
        -to_left.min(to_right).min(to_top).min(to_bottom)
    } else if dx > 0 && dy > 0 {
        isqrt_u32((dx * dx + dy * dy) as u32) as i32 - r
    } else {
        dx.max(dy) - r
    }
}

/// Approximate signed distance from point (px,py) to an axis-aligned ellipse
/// centered at (cx,cy) with semi-axes (rx,ry). Negative inside, positive outside.
#[inline]
fn oval_sdf(px: i32, py: i32, cx: i32, cy: i32, rx: i32, ry: i32) -> i32 {
    if rx <= 0 || ry <= 0 { return i32::MAX; }
    let dx = (px - cx).abs() as i64;
    let dy = (py - cy).abs() as i64;
    let arx = rx as i64;
    let ary = ry as i64;
    // Map to normalized ellipse space: f = dx*ry, g = dy*rx
    let f = dx * ary;
    let g = dy * arx;
    let threshold = arx * ary;
    let len = isqrt_u64((f * f + g * g) as u64) as i64;
    // Approximate pixel distance: (len/threshold - 1) * min(rx,ry)
    let min_r = arx.min(ary);
    ((len - threshold) * min_r / threshold.max(1)) as i32
}

/// Core shadow drawing loop. Computes quadratic alpha falloff for pixels outside
/// the shape (dist > 0) and fills inside (dist <= 0) with full alpha so there's
/// no gap when the shadow is offset from the content drawn on top.
#[inline(always)]
fn draw_shadow_core<F: Fn(i32, i32) -> i32>(
    pixels: *mut u32, fb_w: u32, fb_h: u32,
    // Shadow bounding box (in screen coords)
    box_x: i32, box_y: i32, box_w: i32, box_h: i32,
    spread: i32, alpha: u32,
    sdf: F,
) {
    if alpha == 0 || spread <= 0 { return; }
    let s = spread as u32;

    // Clip to framebuffer
    let x0 = box_x.max(0);
    let y0 = box_y.max(0);
    let x1 = (box_x + box_w).min(fb_w as i32);
    let y1 = (box_y + box_h).min(fb_h as i32);

    let stride = fb_w as usize;
    for py in y0..y1 {
        let row_off = py as usize * stride;
        for px in x0..x1 {
            let dist = sdf(px, py);
            let a = if dist <= 0 {
                // Inside the shadow shape: full alpha
                alpha
            } else if dist < spread {
                // Outside with quadratic falloff
                let t = dist as u32;
                let inv = s - t;
                (alpha * inv * inv) / (s * s)
            } else {
                continue;
            };
            if a == 0 { continue; }
            let idx = row_off + px as usize;
            unsafe {
                let dst = *pixels.add(idx);
                *pixels.add(idx) = shadow_blend(a, dst);
            }
        }
    }
}

/// Draw a soft shadow for a rectangle.
/// (x,y,w,h) = content rect. Shadow is offset by (offset_x, offset_y).
/// `spread` = falloff distance in pixels. `alpha` = max opacity (0-255).
pub fn draw_shadow_rect_fn(win: u32, x: i32, y: i32, w: u32, h: u32,
    offset_x: i32, offset_y: i32, spread: i32, alpha: u32)
{
    let s = decode_surface(win);
    let sx = x + offset_x;
    let sy = y + offset_y;
    let sw = w as i32;
    let sh = h as i32;
    draw_shadow_core(
        s.pixels, s.width, s.height,
        sx - spread, sy - spread, sw + spread * 2, sh + spread * 2,
        spread, alpha,
        |px, py| rect_sdf(px, py, sx, sy, sw, sh),
    );
}

/// Draw a soft shadow for a rounded rectangle.
/// (x,y,w,h,r) = content rounded rect. Shadow is offset by (offset_x, offset_y).
pub fn draw_shadow_rounded_rect_fn(win: u32, x: i32, y: i32, w: u32, h: u32, r: i32,
    offset_x: i32, offset_y: i32, spread: i32, alpha: u32)
{
    let s = decode_surface(win);
    let sx = x + offset_x;
    let sy = y + offset_y;
    let sw = w as i32;
    let sh = h as i32;
    draw_shadow_core(
        s.pixels, s.width, s.height,
        sx - spread, sy - spread, sw + spread * 2, sh + spread * 2,
        spread, alpha,
        |px, py| rounded_rect_sdf(px, py, sx, sy, sw, sh, r),
    );
}

/// Draw a soft shadow for an oval/ellipse.
/// (cx,cy) = center, (rx,ry) = semi-axes. Shadow is offset by (offset_x, offset_y).
pub fn draw_shadow_oval_fn(win: u32, cx: i32, cy: i32, rx: i32, ry: i32,
    offset_x: i32, offset_y: i32, spread: i32, alpha: u32)
{
    let s = decode_surface(win);
    let scx = cx + offset_x;
    let scy = cy + offset_y;
    draw_shadow_core(
        s.pixels, s.width, s.height,
        scx - rx - spread, scy - ry - spread,
        (rx + spread) * 2, (ry + spread) * 2,
        spread, alpha,
        |px, py| oval_sdf(px, py, scx, scy, rx, ry),
    );
}

// --- Shadow extern "C" exports ---

pub extern "C" fn draw_shadow_rect_export(
    win: u32, x: i32, y: i32, w: u32, h: u32,
    offset_x: i32, offset_y: i32, spread: i32, alpha: u32,
) {
    draw_shadow_rect_fn(win, x, y, w, h, offset_x, offset_y, spread, alpha);
}

pub extern "C" fn draw_shadow_rounded_rect_export(
    win: u32, x: i32, y: i32, w: u32, h: u32, r: i32,
    offset_x: i32, offset_y: i32, spread: i32, alpha: u32,
) {
    draw_shadow_rounded_rect_fn(win, x, y, w, h, r, offset_x, offset_y, spread, alpha);
}

pub extern "C" fn draw_shadow_oval_export(
    win: u32, cx: i32, cy: i32, rx: i32, ry: i32,
    offset_x: i32, offset_y: i32, spread: i32, alpha: u32,
) {
    draw_shadow_oval_fn(win, cx, cy, rx, ry, offset_x, offset_y, spread, alpha);
}

// ── Blur rendering ───────────────────────────────────────────────────

/// Fast box blur applied to a rectangular region of a pixel buffer.
/// Uses two-pass (horizontal + vertical) separable box blur for O(n) per pixel.
/// `radius` is the blur kernel half-size (effective kernel = 2*radius+1).
/// `passes` controls quality: 1 = box blur, 2 = triangle-like, 3 ≈ Gaussian.
/// Only the region (x,y,w,h) is blurred; pixels outside are read but not written.
pub fn blur_rect_fn(win: u32, x: i32, y: i32, w: u32, h: u32, radius: u32, passes: u32) {
    let s = decode_surface(win);
    if w == 0 || h == 0 || radius == 0 || passes == 0 { return; }
    blur_region(s.pixels, s.width, s.height, x, y, w, h, radius, passes);
}

/// Fast box blur on a rounded rect region. Pixels outside the rounded rect
/// are not modified (preserves transparency for rounded corners).
pub fn blur_rounded_rect_fn(win: u32, x: i32, y: i32, w: u32, h: u32, r: i32, radius: u32, passes: u32) {
    let s = decode_surface(win);
    if w == 0 || h == 0 || radius == 0 || passes == 0 { return; }
    blur_region_rounded(s.pixels, s.width, s.height, x, y, w, h, r, radius, passes);
}

/// Core separable box blur on a rectangular region of a pixel buffer.
/// Two-pass (H then V) per pass iteration. Uses running sum for O(1) per pixel.
fn blur_region(pixels: *mut u32, fb_w: u32, fb_h: u32,
    rx: i32, ry: i32, rw: u32, rh: u32, radius: u32, passes: u32)
{
    // Clip region to framebuffer
    let x0 = rx.max(0) as usize;
    let y0 = ry.max(0) as usize;
    let x1 = ((rx + rw as i32) as usize).min(fb_w as usize);
    let y1 = ((ry + rh as i32) as usize).min(fb_h as usize);
    if x0 >= x1 || y0 >= y1 { return; }
    let w = x1 - x0;
    let h = y1 - y0;
    let stride = fb_w as usize;
    let r = radius as usize;
    let kernel = (2 * r + 1) as u32;

    // Temporary buffer for one scanline/column (stack-allocated, no heap in DLL)
    const MAX_BLUR_DIM: usize = 2048;
    let max_dim = w.max(h);
    if max_dim > MAX_BLUR_DIM { return; }
    let mut temp_buf = [0u32; MAX_BLUR_DIM];
    let temp = &mut temp_buf[..max_dim];

    for _ in 0..passes {
        // Horizontal pass: blur each row
        for row in y0..y1 {
            let row_off = row * stride;
            // Running sums for R, G, B, A
            let mut sr: u32 = 0;
            let mut sg: u32 = 0;
            let mut sb: u32 = 0;
            let mut sa: u32 = 0;

            // Initialize window: sum pixels [x0 - r .. x0 + r] (clamped)
            for i in 0..=(2 * r) {
                let sx = (x0 as i32 + i as i32 - r as i32).max(0).min(fb_w as i32 - 1) as usize;
                let px = unsafe { *pixels.add(row_off + sx) };
                sa += (px >> 24) & 0xFF;
                sr += (px >> 16) & 0xFF;
                sg += (px >> 8) & 0xFF;
                sb += px & 0xFF;
            }

            for col in 0..w {
                let cx = x0 + col;
                temp[col] = ((sa / kernel) << 24) | ((sr / kernel) << 16) | ((sg / kernel) << 8) | (sb / kernel);

                // Slide window: add right pixel, remove left pixel
                let add_x = (cx as i32 + r as i32 + 1).min(fb_w as i32 - 1).max(0) as usize;
                let rem_x = (cx as i32 - r as i32).max(0).min(fb_w as i32 - 1) as usize;
                let add_px = unsafe { *pixels.add(row_off + add_x) };
                let rem_px = unsafe { *pixels.add(row_off + rem_x) };
                sa += ((add_px >> 24) & 0xFF) - ((rem_px >> 24) & 0xFF);
                sr += ((add_px >> 16) & 0xFF) - ((rem_px >> 16) & 0xFF);
                sg += ((add_px >> 8) & 0xFF) - ((rem_px >> 8) & 0xFF);
                sb += (add_px & 0xFF) - (rem_px & 0xFF);
            }

            // Write back
            for col in 0..w {
                unsafe { *pixels.add(row_off + x0 + col) = temp[col]; }
            }
        }

        // Vertical pass: blur each column
        for col in x0..x1 {
            let mut sr: u32 = 0;
            let mut sg: u32 = 0;
            let mut sb: u32 = 0;
            let mut sa: u32 = 0;

            // Initialize window
            for i in 0..=(2 * r) {
                let sy = (y0 as i32 + i as i32 - r as i32).max(0).min(fb_h as i32 - 1) as usize;
                let px = unsafe { *pixels.add(sy * stride + col) };
                sa += (px >> 24) & 0xFF;
                sr += (px >> 16) & 0xFF;
                sg += (px >> 8) & 0xFF;
                sb += px & 0xFF;
            }

            for row in 0..h {
                let cy = y0 + row;
                temp[row] = ((sa / kernel) << 24) | ((sr / kernel) << 16) | ((sg / kernel) << 8) | (sb / kernel);

                let add_y = (cy as i32 + r as i32 + 1).min(fb_h as i32 - 1).max(0) as usize;
                let rem_y = (cy as i32 - r as i32).max(0).min(fb_h as i32 - 1) as usize;
                let add_px = unsafe { *pixels.add(add_y * stride + col) };
                let rem_px = unsafe { *pixels.add(rem_y * stride + col) };
                sa += ((add_px >> 24) & 0xFF) - ((rem_px >> 24) & 0xFF);
                sr += ((add_px >> 16) & 0xFF) - ((rem_px >> 16) & 0xFF);
                sg += ((add_px >> 8) & 0xFF) - ((rem_px >> 8) & 0xFF);
                sb += (add_px & 0xFF) - (rem_px & 0xFF);
            }

            // Write back
            for row in 0..h {
                unsafe { *pixels.add((y0 + row) * stride + col) = temp[row]; }
            }
        }
    }
}

/// Blur within a rounded rect region. Blurs the full bounding box, then restores
/// pixels outside the rounded rect to their original values.
fn blur_region_rounded(pixels: *mut u32, fb_w: u32, fb_h: u32,
    rx: i32, ry: i32, rw: u32, rh: u32, corner_r: i32, radius: u32, passes: u32)
{
    let x0 = rx.max(0) as usize;
    let y0 = ry.max(0) as usize;
    let x1 = ((rx + rw as i32) as usize).min(fb_w as usize);
    let y1 = ((ry + rh as i32) as usize).min(fb_h as usize);
    if x0 >= x1 || y0 >= y1 { return; }
    let w = x1 - x0;
    let h = y1 - y0;
    let stride = fb_w as usize;

    // Save corner pixels that are outside the rounded rect
    let cr = corner_r.max(0) as usize;
    if cr == 0 {
        // No rounding — just blur the full rect
        blur_region(pixels, fb_w, fb_h, rx, ry, rw, rh, radius, passes);
        return;
    }

    // Save the 4 corner regions (stack-allocated, no heap in DLL)
    // Max corner radius 32 → 1024 entries per corner × 4 = 16 KiB total
    const MAX_CORNER_R: usize = 32;
    if cr > MAX_CORNER_R { return; }
    let corner_size = cr * cr;
    let mut saved_tl = [0u32; MAX_CORNER_R * MAX_CORNER_R];
    let mut saved_tr = [0u32; MAX_CORNER_R * MAX_CORNER_R];
    let mut saved_bl = [0u32; MAX_CORNER_R * MAX_CORNER_R];
    let mut saved_br = [0u32; MAX_CORNER_R * MAX_CORNER_R];

    // Save top-left corner
    for dy in 0..cr.min(h) {
        for dx in 0..cr.min(w) {
            let idx = (y0 + dy) * stride + (x0 + dx);
            saved_tl[dy * cr + dx] = unsafe { *pixels.add(idx) };
        }
    }
    // Save top-right corner
    for dy in 0..cr.min(h) {
        for dx in 0..cr.min(w) {
            let sx = x1 - 1 - dx;
            if sx >= x0 {
                saved_tr[dy * cr + dx] = unsafe { *pixels.add((y0 + dy) * stride + sx) };
            }
        }
    }
    // Save bottom-left corner
    for dy in 0..cr.min(h) {
        let sy = y1 - 1 - dy;
        if sy >= y0 {
            for dx in 0..cr.min(w) {
                saved_bl[dy * cr + dx] = unsafe { *pixels.add(sy * stride + (x0 + dx)) };
            }
        }
    }
    // Save bottom-right corner
    for dy in 0..cr.min(h) {
        let sy = y1 - 1 - dy;
        if sy >= y0 {
            for dx in 0..cr.min(w) {
                let sx = x1 - 1 - dx;
                if sx >= x0 {
                    saved_br[dy * cr + dx] = unsafe { *pixels.add(sy * stride + sx) };
                }
            }
        }
    }

    // Blur the full bounding box
    blur_region(pixels, fb_w, fb_h, rx, ry, rw, rh, radius, passes);

    // Restore pixels outside the rounded rect corners
    let r2x4 = (2 * corner_r) * (2 * corner_r);
    for dy in 0..cr.min(h) {
        let cy = 2 * dy as i32 + 1 - 2 * corner_r;
        let cy2 = cy * cy;
        for dx in 0..cr.min(w) {
            let cx = 2 * dx as i32 + 1 - 2 * corner_r;
            if cx * cx + cy2 > r2x4 {
                // Outside the corner arc — restore original pixel
                // Top-left
                unsafe { *pixels.add((y0 + dy) * stride + (x0 + dx)) = saved_tl[dy * cr + dx]; }
                // Top-right
                let sx = x1 - 1 - dx;
                if sx >= x0 {
                    unsafe { *pixels.add((y0 + dy) * stride + sx) = saved_tr[dy * cr + dx]; }
                }
                // Bottom-left
                let sy = y1 - 1 - dy;
                if sy >= y0 {
                    unsafe { *pixels.add(sy * stride + (x0 + dx)) = saved_bl[dy * cr + dx]; }
                }
                // Bottom-right
                if sy >= y0 && sx >= x0 {
                    unsafe { *pixels.add(sy * stride + sx) = saved_br[dy * cr + dx]; }
                }
            }
        }
    }
}

// --- Blur extern "C" exports ---

pub extern "C" fn blur_rect_export(
    win: u32, x: i32, y: i32, w: u32, h: u32, radius: u32, passes: u32,
) {
    blur_rect_fn(win, x, y, w, h, radius, passes);
}

pub extern "C" fn blur_rounded_rect_export(
    win: u32, x: i32, y: i32, w: u32, h: u32, r: i32, radius: u32, passes: u32,
) {
    blur_rounded_rect_fn(win, x, y, w, h, r, radius, passes);
}
