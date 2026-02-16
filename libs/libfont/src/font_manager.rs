//! Font manager — loads TTF fonts from disk, caches rasterized glyphs,
//! and provides text rendering into user-provided ARGB pixel buffers.
//!
//! State is stored on the process heap; the pointer lives in per-process .bss.

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::ttf::TtfFont;
use crate::ttf_rasterizer;
use crate::syscall;

/// FontManager pointer — lives in per-process .bss (zero-initialized per process).
/// Each process gets its own copy via DLIB v3 per-process .bss support.
static mut FONT_MGR_PTR: *mut FontManager = core::ptr::null_mut();

/// Maximum number of cached glyphs before LRU eviction.
const MAX_CACHE_SIZE: usize = 512;

/// System font IDs (must match kernel convention).
pub const SYSTEM_FONT_ID: u16 = 0;
pub const SYSTEM_FONT_BOLD: u16 = 1;
pub const SYSTEM_FONT_THIN: u16 = 2;
pub const SYSTEM_FONT_ITALIC: u16 = 3;

struct LoadedFont {
    ttf: TtfFont,
}

struct CachedGlyph {
    font_id: u16,
    glyph_id: u16,
    size: u16,
    subpixel: bool,
    width: u32,
    height: u32,
    x_offset: i32,
    y_offset: i32,
    advance: u32,
    coverage: Vec<u8>,
    use_count: u32,
}

pub struct FontManager {
    fonts: Vec<Option<LoadedFont>>,
    cache: Vec<CachedGlyph>,
    access_counter: u32,
    subpixel_enabled: bool,
}

impl FontManager {
    fn new() -> Self {
        FontManager {
            fonts: Vec::new(),
            cache: Vec::new(),
            access_counter: 0,
            subpixel_enabled: false,
        }
    }

    fn add_font(&mut self, ttf: TtfFont) -> u16 {
        let font = LoadedFont { ttf };
        for (i, slot) in self.fonts.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(font);
                return i as u16;
            }
        }
        let id = self.fonts.len() as u16;
        self.fonts.push(Some(font));
        id
    }

    fn get_font(&self, font_id: u16) -> Option<&TtfFont> {
        self.fonts
            .get(font_id as usize)
            .and_then(|slot| slot.as_ref())
            .map(|f| &f.ttf)
    }

    fn get_font_or_fallback(&self, font_id: u16) -> Option<&TtfFont> {
        self.get_font(font_id)
            .or_else(|| if font_id != SYSTEM_FONT_ID { self.get_font(SYSTEM_FONT_ID) } else { None })
    }

    fn remove_font(&mut self, font_id: u16) {
        if let Some(slot) = self.fonts.get_mut(font_id as usize) {
            *slot = None;
        }
        self.cache.retain(|g| g.font_id != font_id);
    }

    fn find_cached(&mut self, font_id: u16, glyph_id: u16, size: u16, subpixel: bool) -> Option<usize> {
        for (i, g) in self.cache.iter_mut().enumerate() {
            if g.font_id == font_id && g.glyph_id == glyph_id && g.size == size && g.subpixel == subpixel {
                self.access_counter += 1;
                g.use_count = self.access_counter;
                return Some(i);
            }
        }
        None
    }

    fn evict_if_needed(&mut self) {
        if self.cache.len() < MAX_CACHE_SIZE {
            return;
        }
        let mut min_use = u32::MAX;
        let mut min_idx = 0;
        for (i, g) in self.cache.iter().enumerate() {
            if g.use_count < min_use {
                min_use = g.use_count;
                min_idx = i;
            }
        }
        self.cache.swap_remove(min_idx);
    }

    fn cache_glyph(
        &mut self, font_id: u16, glyph_id: u16, size: u16, subpixel: bool,
        bitmap: ttf_rasterizer::GlyphBitmap,
    ) -> usize {
        self.evict_if_needed();
        self.access_counter += 1;
        let idx = self.cache.len();
        self.cache.push(CachedGlyph {
            font_id, glyph_id, size, subpixel,
            width: bitmap.width, height: bitmap.height,
            x_offset: bitmap.x_offset, y_offset: bitmap.y_offset,
            advance: bitmap.advance,
            coverage: bitmap.coverage,
            use_count: self.access_counter,
        });
        idx
    }

    fn rasterize_and_cache(
        &mut self, font_id: u16, glyph_id: u16, size: u16, subpixel: bool,
    ) -> Option<usize> {
        let ttf = self.get_font(font_id)?;
        let outline = ttf.glyph_outline(glyph_id)?;
        let units_per_em = ttf.units_per_em;
        let advance_fu = ttf.advance_width(glyph_id);
        let advance_px = (advance_fu as u32 * size as u32 + units_per_em as u32 / 2)
            / units_per_em as u32;

        let mut bitmap = if subpixel {
            ttf_rasterizer::rasterize_subpixel(&outline, size as u32, units_per_em)?
        } else {
            ttf_rasterizer::rasterize(&outline, size as u32, units_per_em)?
        };

        bitmap.advance = advance_px;

        Some(self.cache_glyph(font_id, glyph_id, size, subpixel, bitmap))
    }
}

fn line_height_internal(ttf: &TtfFont, size: u16) -> u32 {
    let ascent = ttf.ascent.unsigned_abs() as u32;
    let descent = ttf.descent.unsigned_abs() as u32;
    let line_gap = ttf.line_gap.max(0) as u32;
    (ascent + descent + line_gap) * size as u32 / ttf.units_per_em as u32
}

// ─── State access ────────────────────────────────────────────────────────

/// Get the FontManager pointer from per-process .bss, or None if not initialized.
fn get_mgr() -> Option<&'static mut FontManager> {
    unsafe {
        if FONT_MGR_PTR.is_null() {
            None
        } else {
            Some(&mut *FONT_MGR_PTR)
        }
    }
}

/// Store the FontManager pointer in per-process .bss.
fn set_mgr_ptr(mgr: *mut FontManager) {
    unsafe {
        FONT_MGR_PTR = mgr;
    }
}

// ─── Public API (called from extern "C" exports) ────────────────────────

/// Initialize the font manager and load system fonts from disk.
pub fn init() {
    // Don't double-init
    if get_mgr().is_some() {
        return;
    }

    let mgr = Box::new(FontManager::new());
    let mgr_ptr = Box::into_raw(mgr);
    set_mgr_ptr(mgr_ptr);

    let mgr = unsafe { &mut *mgr_ptr };

    // Load system font variants
    let font_paths: [&[u8]; 4] = [
        b"/System/fonts/sfpro.ttf",
        b"/System/fonts/sfpro-bold.ttf",
        b"/System/fonts/sfpro-thin.ttf",
        b"/System/fonts/sfpro-italic.ttf",
    ];

    for path in &font_paths {
        match syscall::read_file(path) {
            Some(data) => {
                if let Some(ttf) = TtfFont::parse(data) {
                    mgr.add_font(ttf);
                } else {
                    mgr.fonts.push(None);
                }
            }
            None => {
                mgr.fonts.push(None);
            }
        }
    }

    // Auto-detect GPU acceleration and enable subpixel rendering if available
    if syscall::gpu_has_accel() != 0 {
        mgr.subpixel_enabled = true;
    }
}

/// Ensure the font manager is initialized (auto-init on first call).
fn ensure_init() -> Option<&'static mut FontManager> {
    if get_mgr().is_none() {
        init();
    }
    get_mgr()
}

/// Load a custom TTF font from disk. Returns font_id or u32::MAX.
pub fn load_font(path: &[u8]) -> u32 {
    let mgr = match ensure_init() {
        Some(m) => m,
        None => return u32::MAX,
    };
    let data = match syscall::read_file(path) {
        Some(d) => d,
        None => return u32::MAX,
    };
    match TtfFont::parse(data) {
        Some(ttf) => mgr.add_font(ttf) as u32,
        None => u32::MAX,
    }
}

/// Unload a font by ID. Cannot unload system font (ID 0).
pub fn unload_font(font_id: u16) {
    if font_id == SYSTEM_FONT_ID {
        return;
    }
    if let Some(mgr) = get_mgr() {
        mgr.remove_font(font_id);
    }
}

/// Set subpixel (LCD) rendering mode.
pub fn set_subpixel(enabled: bool) {
    if let Some(mgr) = ensure_init() {
        mgr.subpixel_enabled = enabled;
    }
}

/// Get line height for a font at a given size.
pub fn line_height(font_id: u16, size: u16) -> u32 {
    let mgr = match ensure_init() {
        Some(m) => m,
        None => return size as u32,
    };
    if let Some(ttf) = mgr.get_font_or_fallback(font_id) {
        return line_height_internal(ttf, size);
    }
    size as u32
}

/// Measure the pixel dimensions of a text string.
pub fn measure_string(text: &str, font_id: u16, size: u16) -> (u32, u32) {
    let mgr = match ensure_init() {
        Some(m) => m,
        None => {
            let char_w = (size as u32 * 6) / 10;
            let w = text.chars().filter(|c| *c != '\n').count() as u32 * char_w;
            return (w, size as u32);
        }
    };

    if let Some(ttf) = mgr.get_font_or_fallback(font_id) {
        let mut width = 0u32;
        let mut max_width = 0u32;
        let mut lines = 1u32;
        let upm = ttf.units_per_em as u32;

        for ch in text.chars() {
            if ch == '\n' {
                max_width = max_width.max(width);
                width = 0;
                lines += 1;
                continue;
            }
            if ch == '\t' {
                let space_gid = ttf.char_to_glyph(b' ' as u32);
                let space_adv = ttf.advance_width(space_gid) as u32;
                width += space_adv * 4 * size as u32 / upm;
                continue;
            }
            let gid = ttf.char_to_glyph(ch as u32);
            let adv = ttf.advance_width(gid) as u32;
            width += (adv * size as u32 + upm / 2) / upm;
        }
        max_width = max_width.max(width);
        let lh = line_height_internal(ttf, size);
        return (max_width, lines * lh);
    }

    let char_w = (size as u32 * 6) / 10;
    let w = text.chars().filter(|c| *c != '\n').count() as u32 * char_w;
    (w, size as u32)
}

/// Draw a string into an ARGB pixel buffer.
pub fn draw_string_buf(
    buf: *mut u32, buf_w: u32, buf_h: u32,
    x: i32, y: i32, color: u32,
    font_id: u16, size: u16, text: &str,
) {
    let mgr = match ensure_init() {
        Some(m) => m,
        None => return,
    };

    let subpixel = mgr.subpixel_enabled;
    let actual_font_id = if mgr.get_font(font_id).is_some() { font_id } else { SYSTEM_FONT_ID };
    let (upm, ascent_px, lh, tab_advance) = {
        let ttf = match mgr.get_font(actual_font_id) {
            Some(t) => t,
            None => return,
        };
        let upm = ttf.units_per_em as u32;
        let ascent_px = (ttf.ascent.unsigned_abs() as u32 * size as u32) / upm;
        let lh = line_height_internal(ttf, size) as i32;
        let space_gid = ttf.char_to_glyph(b' ' as u32);
        let space_adv = ttf.advance_width(space_gid) as u32;
        let tab_advance = (space_adv * 4 * size as u32 / upm) as i32;
        (upm, ascent_px, lh, tab_advance)
    };

    let mut cx = x;
    let mut cy = y;

    // Extract color components
    let col_a = ((color >> 24) & 0xFF) as u32;
    let col_r = ((color >> 16) & 0xFF) as u8;
    let col_g = ((color >> 8) & 0xFF) as u8;
    let col_b = (color & 0xFF) as u8;

    for ch in text.chars() {
        if ch == '\n' {
            cx = x;
            cy += lh;
            continue;
        }
        if ch == '\t' {
            cx += tab_advance;
            continue;
        }

        let (gid, advance_px) = {
            let ttf = match mgr.get_font(actual_font_id) {
                Some(t) => t,
                None => continue,
            };
            let gid = ttf.char_to_glyph(ch as u32);
            let adv_fu = ttf.advance_width(gid);
            (gid, (adv_fu as u32 * size as u32 + upm / 2) / upm)
        };

        let cache_idx = mgr.find_cached(actual_font_id, gid, size, subpixel);
        let idx = if let Some(i) = cache_idx {
            i
        } else {
            match mgr.rasterize_and_cache(actual_font_id, gid, size, subpixel) {
                Some(i) => i,
                None => {
                    cx += advance_px as i32;
                    continue;
                }
            }
        };

        let glyph = &mgr.cache[idx];
        if glyph.width > 0 && glyph.height > 0 {
            let gx = cx + if glyph.subpixel { glyph.x_offset / 3 } else { glyph.x_offset };
            let gy = cy + ascent_px as i32 - glyph.y_offset;

            if subpixel && glyph.subpixel {
                draw_glyph_subpixel_buf(buf, buf_w, buf_h, gx, gy, glyph, col_a, col_r, col_g, col_b);
            } else {
                draw_glyph_greyscale_buf(buf, buf_w, buf_h, gx, gy, glyph, col_a, col_r, col_g, col_b);
            }
        }

        cx += advance_px as i32;
    }
}

// ─── Internal rendering ──────────────────────────────────────────────────

fn draw_glyph_greyscale_buf(
    buf: *mut u32, sw: u32, sh: u32,
    x: i32, y: i32, glyph: &CachedGlyph,
    col_a: u32, col_r: u8, col_g: u8, col_b: u8,
) {
    let bw = glyph.width as i32;
    let bh = glyph.height as i32;
    let sw_i = sw as i32;
    let sh_i = sh as i32;

    for row in 0..bh {
        let py = y + row;
        if py < 0 || py >= sh_i { continue; }
        for col in 0..bw {
            let px = x + col;
            if px < 0 || px >= sw_i { continue; }
            let coverage = glyph.coverage[(row * bw + col) as usize];
            if coverage == 0 { continue; }
            let alpha = (coverage as u32 * col_a) / 255;
            if alpha == 0 { continue; }

            let idx = (py as u32 * sw + px as u32) as usize;
            let dst = unsafe { *buf.add(idx) };
            let blended = alpha_blend_pixel(dst, alpha, col_r, col_g, col_b);
            unsafe { *buf.add(idx) = blended; }
        }
    }
}

fn draw_glyph_subpixel_buf(
    buf: *mut u32, sw: u32, sh: u32,
    x: i32, y: i32, glyph: &CachedGlyph,
    _col_a: u32, col_r: u8, col_g: u8, col_b: u8,
) {
    let pixel_w = (glyph.width / 3) as i32;
    let stride = glyph.width as usize;
    let bh = glyph.height as i32;
    let sw_i = sw as i32;
    let sh_i = sh as i32;

    for row in 0..bh {
        let py = y + row;
        if py < 0 || py >= sh_i { continue; }
        let row_start = row as usize * stride;
        if row_start + stride > glyph.coverage.len() { continue; }
        let cov_row = &glyph.coverage[row_start..row_start + stride];

        for col in 0..pixel_w {
            let px = x + col;
            if px < 0 || px >= sw_i { continue; }
            let ci = col as usize * 3;
            let r_raw = cov_row[ci] as u32;
            let g_raw = cov_row[ci + 1] as u32;
            let b_raw = cov_row[ci + 2] as u32;
            if r_raw == 0 && g_raw == 0 && b_raw == 0 { continue; }

            // 5-tap FIR filter for LCD color fringe reduction
            let get = |i: usize| -> u32 {
                if i < stride { cov_row[i] as u32 } else { 0 }
            };
            let ci_i = ci as isize;
            let getl = |i: isize| -> u32 {
                if i >= 0 && (i as usize) < stride { cov_row[i as usize] as u32 } else { 0 }
            };
            let r_filt = (getl(ci_i - 2) + getl(ci_i - 1) * 2 + r_raw * 4 + g_raw * 2 + b_raw + 5) / 10;
            let g_filt = (getl(ci_i - 1) + r_raw * 2 + g_raw * 4 + b_raw * 2 + get(ci + 3) + 5) / 10;
            let b_filt = (r_raw + g_raw * 2 + b_raw * 4 + get(ci + 3) * 2 + get(ci + 4) + 5) / 10;

            let idx = (py as u32 * sw + px as u32) as usize;
            let dst = unsafe { *buf.add(idx) };
            let blended = subpixel_blend(dst, r_filt as u8, g_filt as u8, b_filt as u8, col_r, col_g, col_b);
            unsafe { *buf.add(idx) = blended; }
        }
    }
}

#[inline]
fn alpha_blend_pixel(dst: u32, alpha: u32, r: u8, g: u8, b: u8) -> u32 {
    let inv = 255 - alpha;
    let dr = (dst >> 16) & 0xFF;
    let dg = (dst >> 8) & 0xFF;
    let db = dst & 0xFF;
    let nr = (r as u32 * alpha + dr * inv) / 255;
    let ng = (g as u32 * alpha + dg * inv) / 255;
    let nb = (b as u32 * alpha + db * inv) / 255;
    0xFF000000 | (nr << 16) | (ng << 8) | nb
}

#[inline]
fn subpixel_blend(dst: u32, r_cov: u8, g_cov: u8, b_cov: u8, col_r: u8, col_g: u8, col_b: u8) -> u32 {
    let dr = (dst >> 16) & 0xFF;
    let dg = (dst >> 8) & 0xFF;
    let db = dst & 0xFF;
    let ra = r_cov as u32;
    let ga = g_cov as u32;
    let ba = b_cov as u32;
    let nr = (col_r as u32 * ra + dr * (255 - ra)) / 255;
    let ng = (col_g as u32 * ga + dg * (255 - ga)) / 255;
    let nb = (col_b as u32 * ba + db * (255 - ba)) / 255;
    0xFF000000 | (nr << 16) | (ng << 8) | nb
}
