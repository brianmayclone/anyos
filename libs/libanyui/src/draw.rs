//! Drawing functions for direct pixel-buffer rendering.
//!
//! All drawing operates on a `Surface` (pointer to an ARGB pixel buffer).
//! Shape rendering delegates to librender.dlib, text rendering to libfont.so.
//! Bitmap font rendering uses the embedded font_bitmap module.

use crate::font_bitmap;

/// SHM window surface — pixel buffer + dimensions + clip rect.
#[derive(Clone, Copy)]
pub struct Surface {
    pub pixels: *mut u32,
    pub width: u32,
    pub height: u32,
    /// Clip rectangle — drawing outside this region is discarded.
    pub clip_x: i32,
    pub clip_y: i32,
    pub clip_w: u32,
    pub clip_h: u32,
}

impl Surface {
    /// Create a surface with clip set to full bounds.
    pub fn new(pixels: *mut u32, width: u32, height: u32) -> Self {
        Self { pixels, width, height, clip_x: 0, clip_y: 0, clip_w: width, clip_h: height }
    }

    /// Return a copy with clip rect intersected with the given region.
    pub fn with_clip(&self, x: i32, y: i32, w: u32, h: u32) -> Self {
        let cx0 = self.clip_x.max(x);
        let cy0 = self.clip_y.max(y);
        let cx1 = (self.clip_x + self.clip_w as i32).min(x + w as i32);
        let cy1 = (self.clip_y + self.clip_h as i32).min(y + h as i32);
        Surface {
            pixels: self.pixels,
            width: self.width,
            height: self.height,
            clip_x: cx0,
            clip_y: cy0,
            clip_w: (cx1 - cx0).max(0) as u32,
            clip_h: (cy1 - cy0).max(0) as u32,
        }
    }
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

// ── libfont.so dynamic resolution ────────────────────────────────────

type MeasureFn = extern "C" fn(u32, u16, *const u8, u32, *mut u32, *mut u32);
type DrawFn = extern "C" fn(*mut u32, u32, u32, i32, i32, u32, u32, u16, *const u8, u32);

static mut FONT_MEASURE: Option<MeasureFn> = None;
static mut FONT_DRAW: Option<DrawFn> = None;

/// Ensure libfont.so is loaded and symbols are resolved.
fn ensure_libfont() {
    unsafe {
        if FONT_MEASURE.is_some() { return; }
        let base = crate::syscall::dll_load(b"/Libraries/libfont.so");
        if base == 0 { return; }
        FONT_MEASURE = resolve_sym(base, b"font_measure_string");
        FONT_DRAW = resolve_sym(base, b"font_draw_string_buf");
    }
}

/// Mini ELF64 symbol resolver — resolves a single symbol from a loaded .so.
unsafe fn resolve_sym<T: Copy>(base: u64, name: &[u8]) -> Option<T> {
    // ELF64 header
    let ehdr = base as *const u8;
    if *ehdr != 0x7F || *ehdr.add(1) != b'E' || *ehdr.add(2) != b'L' || *ehdr.add(3) != b'F' {
        return None;
    }
    let e_phoff = *(ehdr.add(32) as *const u64);
    let e_phnum = *(ehdr.add(56) as *const u16);

    // Find PT_DYNAMIC
    let mut dynamic_va: u64 = 0;
    for i in 0..e_phnum as usize {
        let ph = (base + e_phoff + (i as u64) * 56) as *const u8;
        let p_type = *(ph as *const u32);
        if p_type == 2 { // PT_DYNAMIC
            dynamic_va = *(ph.add(16) as *const u64); // p_vaddr
            break;
        }
    }
    if dynamic_va == 0 { return None; }

    // Walk .dynamic for DT_SYMTAB(6), DT_STRTAB(5), DT_HASH(4)
    let mut symtab: u64 = 0;
    let mut strtab: u64 = 0;
    let mut hash: u64 = 0;
    let dyn_ptr = dynamic_va as *const u8;
    for i in 0..128 {
        let entry = dyn_ptr.add(i * 16);
        let d_tag = *(entry as *const i64);
        let d_val = *(entry.add(8) as *const u64);
        match d_tag {
            6 => symtab = d_val,
            5 => strtab = d_val,
            4 => hash = d_val,
            0 => break,
            _ => {}
        }
    }
    if symtab == 0 || strtab == 0 || hash == 0 { return None; }

    // ELF hash lookup
    let nbuckets = *(hash as *const u32);
    let buckets = (hash as *const u32).add(2);
    let chains = buckets.add(nbuckets as usize);

    let h = elf_hash(name);
    let mut idx = *buckets.add((h % nbuckets) as usize);
    while idx != 0 {
        // Elf64Sym: st_name(4) st_info(1) st_other(1) st_shndx(2) st_value(8) st_size(8) = 24 bytes
        let sym = (symtab + idx as u64 * 24) as *const u8;
        let st_name = *(sym as *const u32);
        let st_value = *(sym.add(8) as *const u64);
        if st_value != 0 && cstr_eq(strtab as *const u8, st_name as usize, name) {
            return Some(core::mem::transmute_copy::<u64, T>(&st_value));
        }
        idx = *chains.add(idx as usize);
    }
    None
}

/// SysV ELF hash function.
fn elf_hash(name: &[u8]) -> u32 {
    let mut h: u32 = 0;
    for &b in name {
        h = (h << 4).wrapping_add(b as u32);
        let g = h & 0xF000_0000;
        if g != 0 { h ^= g >> 24; }
        h &= !g;
    }
    h
}

/// Compare a symbol name from strtab with a byte slice.
unsafe fn cstr_eq(strtab: *const u8, offset: usize, name: &[u8]) -> bool {
    let s = strtab.add(offset);
    for (i, &b) in name.iter().enumerate() {
        if *s.add(i) != b { return false; }
    }
    *s.add(name.len()) == 0
}

// ── Constants ───────────────────────────────────────────────────────

/// Default system font (sfpro.ttf, ID 0).
const DEFAULT_FONT_ID: u16 = 0;
const DEFAULT_FONT_SIZE: u16 = 13;

// ── Drawing functions ───────────────────────────────────────────────

/// Fill a rectangle on a surface via librender, clipped to the surface's clip rect.
#[inline(always)]
pub fn fill_rect(s: &Surface, x: i32, y: i32, w: u32, h: u32, color: u32) {
    let x0 = x.max(s.clip_x);
    let y0 = y.max(s.clip_y);
    let x1 = (x + w as i32).min(s.clip_x + s.clip_w as i32);
    let y1 = (y + h as i32).min(s.clip_y + s.clip_h as i32);
    if x0 >= x1 || y0 >= y1 { return; }
    (librender().fill_rect)(s.pixels, s.width, s.height, x0, y0, (x1 - x0) as u32, (y1 - y0) as u32, color);
}

/// Fill a rounded rectangle with antialiasing via librender.
/// Skipped entirely if fully outside the clip rect.
pub fn fill_rounded_rect(s: &Surface, x: i32, y: i32, w: u32, h: u32, r: u32, color: u32) {
    // Skip if fully outside clip rect
    if x + w as i32 <= s.clip_x || y + h as i32 <= s.clip_y
        || x >= s.clip_x + s.clip_w as i32 || y >= s.clip_y + s.clip_h as i32
    {
        return;
    }
    (librender().fill_rounded_rect_aa)(s.pixels, s.width, s.height, x, y, w, h, r as i32, color);
}

/// Draw a 1px border rectangle.
pub fn draw_border(s: &Surface, x: i32, y: i32, w: u32, h: u32, color: u32) {
    fill_rect(s, x, y, w, 1, color);                     // top
    fill_rect(s, x, y + h as i32 - 1, w, 1, color);     // bottom
    fill_rect(s, x, y, 1, h, color);                     // left
    fill_rect(s, x + w as i32 - 1, y, 1, h, color);     // right
}

/// Draw a 1px rounded border following the same arc as fill_rounded_rect.
pub fn draw_rounded_border(s: &Surface, x: i32, y: i32, w: u32, h: u32, r: u32, color: u32) {
    if r == 0 || w < r * 2 || h < r * 2 {
        draw_border(s, x, y, w, h, color);
        return;
    }
    // Top edge (between corners)
    if w > r * 2 {
        fill_rect(s, x + r as i32, y, w - r * 2, 1, color);
        fill_rect(s, x + r as i32, y + h as i32 - 1, w - r * 2, 1, color);
    }
    // Left edge (between corners)
    if h > r * 2 {
        fill_rect(s, x, y + r as i32, 1, h - r * 2, color);
        fill_rect(s, x + w as i32 - 1, y + r as i32, 1, h - r * 2, color);
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
            fill_rect(s, x + fs, y + dy as i32, 1, 1, color);
            fill_rect(s, x + (w - 1 - fill_start) as i32, y + dy as i32, 1, 1, color);
            fill_rect(s, x + fs, y + (h - 1 - dy) as i32, 1, 1, color);
            fill_rect(s, x + (w - 1 - fill_start) as i32, y + (h - 1 - dy) as i32, 1, 1, color);
        }
    }
}

// ── Text rendering ─────────────────────────────────────────────────

/// Render TTF text onto a surface via libfont.so.
/// Skipped if the text line is fully outside the clip rect.
#[inline(always)]
fn render_ttf(s: &Surface, x: i32, y: i32, color: u32, text: &[u8], font_id: u16, size: u16) {
    if text.is_empty() { return; }
    let text_h = size as i32 + 4; // approximate line height
    // Skip if fully outside clip rect
    if y + text_h <= s.clip_y || y >= s.clip_y + s.clip_h as i32
        || x >= s.clip_x + s.clip_w as i32
    {
        return;
    }
    ensure_libfont();
    if let Some(draw) = unsafe { FONT_DRAW } {
        draw(
            s.pixels, s.width, s.height,
            x, y, color,
            font_id as u32, size,
            text.as_ptr(), text.len() as u32,
        );
    }
}

/// Draw text using the default system font at 13px.
#[inline(always)]
pub fn draw_text(s: &Surface, x: i32, y: i32, color: u32, text: &[u8]) {
    render_ttf(s, x, y, color, text, DEFAULT_FONT_ID, DEFAULT_FONT_SIZE);
}

/// Draw text using the default system font at a specific size.
pub fn draw_text_sized(s: &Surface, x: i32, y: i32, color: u32, text: &[u8], size: u16) {
    render_ttf(s, x, y, color, text, DEFAULT_FONT_ID, size);
}

/// Draw text with explicit font ID and size.
pub fn draw_text_ex(s: &Surface, x: i32, y: i32, color: u32, text: &[u8], font_id: u16, size: u16) {
    render_ttf(s, x, y, color, text, font_id, size);
}

/// Draw monospace text using the embedded bitmap font.
#[inline(always)]
pub fn draw_text_mono(s: &Surface, x: i32, y: i32, color: u32, text: &[u8]) {
    if y + 16 <= s.clip_y || y >= s.clip_y + s.clip_h as i32
        || x >= s.clip_x + s.clip_w as i32 { return; }
    font_bitmap::draw_text_mono(s.pixels, s.width, s.height, x, y, text, color);
}

/// Draw proportional text using the embedded bitmap font.
#[inline(always)]
pub fn draw_text_bitmap(s: &Surface, x: i32, y: i32, color: u32, text: &[u8]) {
    if y + 16 <= s.clip_y || y >= s.clip_y + s.clip_h as i32
        || x >= s.clip_x + s.clip_w as i32 { return; }
    font_bitmap::draw_text(s.pixels, s.width, s.height, x, y, text, color);
}

// ── Text measurement ───────────────────────────────────────────────

/// Measure text extent using a specific font via libfont.so.
/// Returns (width, height) in pixels.
pub fn measure_text_ex(text: &[u8], font_id: u16, size: u16) -> (u32, u32) {
    let len = if !text.is_empty() && text[text.len() - 1] == 0 {
        text.len() - 1
    } else {
        text.len()
    };
    if len == 0 {
        return (0, size as u32);
    }
    ensure_libfont();
    let mut w = 0u32;
    let mut h = 0u32;
    if let Some(measure) = unsafe { FONT_MEASURE } {
        measure(font_id as u32, size, text.as_ptr(), len as u32, &mut w, &mut h);
    }
    (w, h)
}

/// Measure text extent using the default system font (13px).
pub fn text_size(text: &[u8]) -> (u32, u32) {
    measure_text_ex(text, DEFAULT_FONT_ID, DEFAULT_FONT_SIZE)
}

/// Measure text extent using the default system font at a specific size.
pub fn text_size_at(text: &[u8], size: u16) -> (u32, u32) {
    measure_text_ex(text, DEFAULT_FONT_ID, size)
}

/// Measure text width of the first `n` bytes using the default system font.
pub fn text_width_n(text: &[u8], n: usize) -> u32 {
    if n == 0 { return 0; }
    let len = n.min(text.len());
    ensure_libfont();
    let mut w = 0u32;
    let mut h = 0u32;
    if let Some(measure) = unsafe { FONT_MEASURE } {
        measure(
            DEFAULT_FONT_ID as u32, DEFAULT_FONT_SIZE,
            text.as_ptr(), len as u32,
            &mut w, &mut h,
        );
    }
    w
}

// ── Shadow rendering ───────────────────────────────────────────────

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

/// Signed distance from point (px,py) to an axis-aligned rectangle.
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

/// Signed distance from point to a rounded rectangle.
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

/// Approximate signed distance from point to an axis-aligned ellipse.
#[inline]
fn oval_sdf(px: i32, py: i32, cx: i32, cy: i32, rx: i32, ry: i32) -> i32 {
    if rx <= 0 || ry <= 0 { return i32::MAX; }
    let dx = (px - cx).abs() as i64;
    let dy = (py - cy).abs() as i64;
    let arx = rx as i64;
    let ary = ry as i64;
    let f = dx * ary;
    let g = dy * arx;
    let threshold = arx * ary;
    let len = isqrt_u64((f * f + g * g) as u64) as i64;
    let min_r = arx.min(ary);
    ((len - threshold) * min_r / threshold.max(1)) as i32
}

/// Core shadow drawing loop with quadratic alpha falloff.
#[inline(always)]
fn draw_shadow_core<F: Fn(i32, i32) -> i32>(
    pixels: *mut u32, fb_w: u32, fb_h: u32,
    box_x: i32, box_y: i32, box_w: i32, box_h: i32,
    spread: i32, alpha: u32,
    sdf: F,
) {
    if alpha == 0 || spread <= 0 { return; }
    let s = spread as u32;
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
                alpha
            } else if dist < spread {
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
pub fn draw_shadow_rect(s: &Surface, x: i32, y: i32, w: u32, h: u32,
    offset_x: i32, offset_y: i32, spread: i32, alpha: u32)
{
    let sx = x + offset_x;
    let sy = y + offset_y;
    let sw = w as i32;
    let sh = h as i32;
    let bx = sx - spread;
    let by = sy - spread;
    let bw = sw + spread * 2;
    let bh = sh + spread * 2;
    // Skip if fully outside clip rect
    if bx + bw <= s.clip_x || by + bh <= s.clip_y
        || bx >= s.clip_x + s.clip_w as i32 || by >= s.clip_y + s.clip_h as i32
    {
        return;
    }
    draw_shadow_core(
        s.pixels, s.width, s.height,
        bx, by, bw, bh,
        spread, alpha,
        |px, py| rect_sdf(px, py, sx, sy, sw, sh),
    );
}

/// Draw a soft shadow for a rounded rectangle.
pub fn draw_shadow_rounded_rect(s: &Surface, x: i32, y: i32, w: u32, h: u32, r: i32,
    offset_x: i32, offset_y: i32, spread: i32, alpha: u32)
{
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
pub fn draw_shadow_oval(s: &Surface, cx: i32, cy: i32, rx: i32, ry: i32,
    offset_x: i32, offset_y: i32, spread: i32, alpha: u32)
{
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

// ── Blur rendering ─────────────────────────────────────────────────

/// Fast box blur applied to a rectangular region of a surface.
/// Uses two-pass (H + V) separable box blur for O(1) per pixel.
pub fn blur_rect(s: &Surface, x: i32, y: i32, w: u32, h: u32, radius: u32, passes: u32) {
    if w == 0 || h == 0 || radius == 0 || passes == 0 { return; }
    blur_region(s.pixels, s.width, s.height, x, y, w, h, radius, passes);
}

/// Fast box blur on a rounded rect region. Pixels outside the rounded rect
/// are not modified (preserves transparency for rounded corners).
pub fn blur_rounded_rect(s: &Surface, x: i32, y: i32, w: u32, h: u32, r: i32, radius: u32, passes: u32) {
    if w == 0 || h == 0 || radius == 0 || passes == 0 { return; }
    blur_region_rounded(s.pixels, s.width, s.height, x, y, w, h, r, radius, passes);
}

/// Blit a pixel buffer onto the surface at (x, y), clipped to the surface's clip rect.
pub fn blit_buffer(s: &Surface, x: i32, y: i32, w: u32, h: u32, src: &[u32]) {
    if w == 0 || h == 0 || src.is_empty() { return; }
    let sw = s.width as i32;
    let sh = s.height as i32;
    // Compute effective clip bounds (intersection of surface bounds and clip rect)
    let clip_x0 = s.clip_x.max(0);
    let clip_y0 = s.clip_y.max(0);
    let clip_x1 = (s.clip_x + s.clip_w as i32).min(sw);
    let clip_y1 = (s.clip_y + s.clip_h as i32).min(sh);
    for row in 0..h as i32 {
        let dy = y + row;
        if dy < clip_y0 || dy >= clip_y1 { continue; }
        let src_off = row as usize * w as usize;
        let x0 = x.max(clip_x0);
        let x1 = (x + w as i32).min(clip_x1);
        if x0 >= x1 { continue; }
        let skip = (x0 - x) as usize;
        let count = (x1 - x0) as usize;
        if src_off + skip + count > src.len() { continue; }
        unsafe {
            let dst = s.pixels.add(dy as usize * s.width as usize + x0 as usize);
            core::ptr::copy_nonoverlapping(src.as_ptr().add(src_off + skip), dst, count);
        }
    }
}

/// Core separable box blur.
fn blur_region(pixels: *mut u32, fb_w: u32, fb_h: u32,
    rx: i32, ry: i32, rw: u32, rh: u32, radius: u32, passes: u32)
{
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

    const MAX_BLUR_DIM: usize = 2048;
    let max_dim = w.max(h);
    if max_dim > MAX_BLUR_DIM { return; }
    let mut temp_buf = [0u32; MAX_BLUR_DIM];
    let temp = &mut temp_buf[..max_dim];

    for _ in 0..passes {
        // Horizontal pass
        for row in y0..y1 {
            let row_off = row * stride;
            let (mut sr, mut sg, mut sb, mut sa) = (0u32, 0u32, 0u32, 0u32);
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
                let add_x = (cx as i32 + r as i32 + 1).min(fb_w as i32 - 1).max(0) as usize;
                let rem_x = (cx as i32 - r as i32).max(0).min(fb_w as i32 - 1) as usize;
                let add_px = unsafe { *pixels.add(row_off + add_x) };
                let rem_px = unsafe { *pixels.add(row_off + rem_x) };
                sa += ((add_px >> 24) & 0xFF) - ((rem_px >> 24) & 0xFF);
                sr += ((add_px >> 16) & 0xFF) - ((rem_px >> 16) & 0xFF);
                sg += ((add_px >> 8) & 0xFF) - ((rem_px >> 8) & 0xFF);
                sb += (add_px & 0xFF) - (rem_px & 0xFF);
            }
            for col in 0..w {
                unsafe { *pixels.add(row_off + x0 + col) = temp[col]; }
            }
        }

        // Vertical pass
        for col in x0..x1 {
            let (mut sr, mut sg, mut sb, mut sa) = (0u32, 0u32, 0u32, 0u32);
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
            for row in 0..h {
                unsafe { *pixels.add((y0 + row) * stride + col) = temp[row]; }
            }
        }
    }
}

/// Blur within a rounded rect region.
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
    let cr = corner_r.max(0) as usize;

    if cr == 0 {
        blur_region(pixels, fb_w, fb_h, rx, ry, rw, rh, radius, passes);
        return;
    }

    const MAX_CORNER_R: usize = 32;
    if cr > MAX_CORNER_R { return; }

    let mut saved_tl = [0u32; MAX_CORNER_R * MAX_CORNER_R];
    let mut saved_tr = [0u32; MAX_CORNER_R * MAX_CORNER_R];
    let mut saved_bl = [0u32; MAX_CORNER_R * MAX_CORNER_R];
    let mut saved_br = [0u32; MAX_CORNER_R * MAX_CORNER_R];

    // Save corner pixels
    for dy in 0..cr.min(h) {
        for dx in 0..cr.min(w) {
            saved_tl[dy * cr + dx] = unsafe { *pixels.add((y0 + dy) * stride + (x0 + dx)) };
        }
    }
    for dy in 0..cr.min(h) {
        for dx in 0..cr.min(w) {
            let sx = x1 - 1 - dx;
            if sx >= x0 {
                saved_tr[dy * cr + dx] = unsafe { *pixels.add((y0 + dy) * stride + sx) };
            }
        }
    }
    for dy in 0..cr.min(h) {
        let sy = y1 - 1 - dy;
        if sy >= y0 {
            for dx in 0..cr.min(w) {
                saved_bl[dy * cr + dx] = unsafe { *pixels.add(sy * stride + (x0 + dx)) };
            }
        }
    }
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
                unsafe { *pixels.add((y0 + dy) * stride + (x0 + dx)) = saved_tl[dy * cr + dx]; }
                let sx = x1 - 1 - dx;
                if sx >= x0 {
                    unsafe { *pixels.add((y0 + dy) * stride + sx) = saved_tr[dy * cr + dx]; }
                }
                let sy = y1 - 1 - dy;
                if sy >= y0 {
                    unsafe { *pixels.add(sy * stride + (x0 + dx)) = saved_bl[dy * cr + dx]; }
                }
                if sy >= y0 && sx >= x0 {
                    unsafe { *pixels.add(sy * stride + sx) = saved_br[dy * cr + dx]; }
                }
            }
        }
    }
}
