//! Font manager — loads TTF fonts from disk, caches rasterized glyphs,
//! and provides text rendering into user-provided ARGB pixel buffers.
//!
//! Performance-critical: all `/ 255` replaced with `div255()` bit trick,
//! glyph cache uses hash table for O(1) lookup, FIR filter uses fixed-point.

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::ttf::TtfFont;
use crate::ttf_rasterizer;
use crate::png_decode;
use crate::syscall;

/// FontManager pointer — lives in per-process .bss (zero-initialized per process).
/// Each process gets its own copy via DLIB v3 per-process .bss support.
static mut FONT_MGR_PTR: *mut FontManager = core::ptr::null_mut();

/// Size-adaptive gamma correction LUTs for font coverage — make text appear
/// thicker and more readable on dark backgrounds (like macOS/Windows).
/// Three LUTs computed once during init():
///  - GAMMA_LUT_S: strong boost for small text (≤14px)
///  - GAMMA_LUT_M: moderate boost for medium text (15-24px)
///  - >24px uses raw coverage (no LUT needed)
static mut GAMMA_LUT_S: [u8; 256] = [0u8; 256];
static mut GAMMA_LUT_M: [u8; 256] = [0u8; 256];

/// Address of the `font_smoothing` field in the uisys DLL export struct.
/// 0 = no smoothing, 1 = greyscale AA, 2 = subpixel LCD.
const FONT_SMOOTHING_ADDR: *const u32 = 0x0400_0010 as *const u32;

/// Read the font smoothing mode from the shared uisys DLL page.
#[inline(always)]
fn read_font_smoothing() -> u32 {
    unsafe { core::ptr::read_volatile(FONT_SMOOTHING_ADDR) }
}

/// Maximum number of cached glyphs before LRU eviction.
/// Sized for HiDPI: at 200%+ scale, each font size generates a distinct cache
/// entry, doubling the working set compared to 100%.
const MAX_CACHE_SIZE: usize = 4096;

/// Hash table size for glyph cache lookup (must be power of 2).
const GLYPH_HASH_SIZE: usize = 4096;
const GLYPH_HASH_EMPTY: u16 = 0xFFFF;

/// System font IDs (must match kernel convention).
pub const SYSTEM_FONT_ID: u16 = 0;
pub const SYSTEM_FONT_BOLD: u16 = 1;
pub const SYSTEM_FONT_THIN: u16 = 2;
pub const SYSTEM_FONT_ITALIC: u16 = 3;
pub const SYSTEM_FONT_MONO: u16 = 4;
/// System emoji font (NotoColorEmoji), loaded as font ID 5.
pub const SYSTEM_FONT_EMOJI: u16 = 5;

/// Fast exact division by 255 (same as compositor's div255).
#[inline(always)]
fn div255(x: u32) -> u32 {
    (x + 1 + (x >> 8)) >> 8
}

struct LoadedFont {
    ttf: TtfFont,
    /// char→glyph cache for ASCII codepoints (avoids cmap4 binary search).
    /// 0xFFFF = not cached yet.
    ascii_glyph_cache: [u16; 128],
}

impl LoadedFont {
    fn new(ttf: TtfFont) -> Self {
        LoadedFont {
            ttf,
            ascii_glyph_cache: [0xFFFF; 128],
        }
    }

    /// Get glyph ID for a codepoint, using ASCII cache for common characters.
    fn char_to_glyph_cached(&mut self, codepoint: u32) -> u16 {
        if codepoint < 128 {
            let cached = self.ascii_glyph_cache[codepoint as usize];
            if cached != 0xFFFF {
                return cached;
            }
            let gid = self.ttf.char_to_glyph(codepoint);
            self.ascii_glyph_cache[codepoint as usize] = gid;
            gid
        } else {
            self.ttf.char_to_glyph(codepoint)
        }
    }
}

struct CachedGlyph {
    font_id: u16,
    glyph_id: u16,
    size: u16,
    subpixel: bool,
    /// True if `coverage` contains RGBA data (4 bytes per pixel) instead of greyscale.
    is_color: bool,
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
    /// Direct-mapped hash table for O(1) glyph cache lookup.
    /// Value = index into `cache` Vec, GLYPH_HASH_EMPTY = empty slot.
    glyph_hash: [u16; GLYPH_HASH_SIZE],
    access_counter: u32,
    subpixel_enabled: bool,
}

/// Compute hash table index from glyph cache key.
#[inline]
fn glyph_hash_index(font_id: u16, glyph_id: u16, size: u16, subpixel: bool) -> usize {
    let k = (font_id as u32) << 16 | (glyph_id as u32);
    let h = k.wrapping_mul(2654435761) ^ ((size as u32) << 1 | subpixel as u32);
    (h as usize) & (GLYPH_HASH_SIZE - 1)
}

impl FontManager {
    fn new() -> Self {
        FontManager {
            fonts: Vec::new(),
            cache: Vec::new(),
            glyph_hash: [GLYPH_HASH_EMPTY; GLYPH_HASH_SIZE],
            access_counter: 0,
            subpixel_enabled: false,
        }
    }

    fn add_font(&mut self, ttf: TtfFont) -> u16 {
        let font = LoadedFont::new(ttf);
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

    fn get_font_mut(&mut self, font_id: u16) -> Option<&mut LoadedFont> {
        self.fonts
            .get_mut(font_id as usize)
            .and_then(|slot| slot.as_mut())
    }

    fn get_font_or_fallback(&self, font_id: u16) -> Option<&TtfFont> {
        self.get_font(font_id)
            .or_else(|| if font_id != SYSTEM_FONT_ID { self.get_font(SYSTEM_FONT_ID) } else { None })
    }

    fn remove_font(&mut self, font_id: u16) {
        if let Some(slot) = self.fonts.get_mut(font_id as usize) {
            *slot = None;
        }
        // Invalidate all hash entries for this font, then remove from cache
        for i in (0..self.cache.len()).rev() {
            if self.cache[i].font_id == font_id {
                self.invalidate_hash_entry(i);
                self.cache.swap_remove(i);
                // Update hash entry for the element that was swapped in
                if i < self.cache.len() {
                    self.update_hash_entry(i);
                }
            }
        }
    }

    /// O(1) glyph cache lookup using hash table with linear-search fallback.
    fn find_cached(&mut self, font_id: u16, glyph_id: u16, size: u16, subpixel: bool) -> Option<usize> {
        let hash = glyph_hash_index(font_id, glyph_id, size, subpixel);
        let slot = self.glyph_hash[hash];

        // Fast path: direct hash hit
        if slot != GLYPH_HASH_EMPTY {
            let idx = slot as usize;
            if idx < self.cache.len() {
                let g = &self.cache[idx];
                if g.font_id == font_id && g.glyph_id == glyph_id && g.size == size && g.subpixel == subpixel {
                    self.access_counter += 1;
                    self.cache[idx].use_count = self.access_counter;
                    return Some(idx);
                }
            }
        }

        // Slow fallback: linear search (hash collision)
        for i in 0..self.cache.len() {
            let g = &self.cache[i];
            if g.font_id == font_id && g.glyph_id == glyph_id && g.size == size && g.subpixel == subpixel {
                self.access_counter += 1;
                self.cache[i].use_count = self.access_counter;
                // Update hash table for next time
                self.glyph_hash[hash] = i as u16;
                return Some(i);
            }
        }
        None
    }

    /// Invalidate the hash entry pointing to cache index `idx`.
    fn invalidate_hash_entry(&mut self, idx: usize) {
        let g = &self.cache[idx];
        let hash = glyph_hash_index(g.font_id, g.glyph_id, g.size, g.subpixel);
        if self.glyph_hash[hash] == idx as u16 {
            self.glyph_hash[hash] = GLYPH_HASH_EMPTY;
        }
    }

    /// Update the hash entry for cache index `idx` (after swap_remove moved an element).
    fn update_hash_entry(&mut self, idx: usize) {
        let g = &self.cache[idx];
        let hash = glyph_hash_index(g.font_id, g.glyph_id, g.size, g.subpixel);
        // Only update if this slot was pointing to the old (now moved) index,
        // or if it's empty (safe to claim).
        let old_slot = self.glyph_hash[hash];
        if old_slot == GLYPH_HASH_EMPTY || old_slot as usize >= self.cache.len() {
            self.glyph_hash[hash] = idx as u16;
        }
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
        // Invalidate hash entry for the evicted glyph
        self.invalidate_hash_entry(min_idx);
        self.cache.swap_remove(min_idx);
        // Update hash entry for the element that was swapped into min_idx
        if min_idx < self.cache.len() {
            self.update_hash_entry(min_idx);
        }
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
            is_color: false,
            width: bitmap.width, height: bitmap.height,
            x_offset: bitmap.x_offset, y_offset: bitmap.y_offset,
            advance: bitmap.advance,
            coverage: bitmap.coverage,
            use_count: self.access_counter,
        });
        // Store in hash table
        let hash = glyph_hash_index(font_id, glyph_id, size, subpixel);
        self.glyph_hash[hash] = idx as u16;
        idx
    }

    fn cache_color_glyph(
        &mut self, font_id: u16, glyph_id: u16, size: u16, subpixel: bool,
        width: u32, height: u32, x_offset: i32, y_offset: i32, advance: u32,
        rgba_data: Vec<u8>,
    ) -> usize {
        self.evict_if_needed();
        self.access_counter += 1;
        let idx = self.cache.len();
        self.cache.push(CachedGlyph {
            font_id, glyph_id, size, subpixel,
            is_color: true,
            width, height, x_offset, y_offset, advance,
            coverage: rgba_data,
            use_count: self.access_counter,
        });
        let hash = glyph_hash_index(font_id, glyph_id, size, subpixel);
        self.glyph_hash[hash] = idx as u16;
        idx
    }

    fn rasterize_and_cache(
        &mut self, font_id: u16, glyph_id: u16, size: u16, subpixel: bool,
    ) -> Option<usize> {
        let ttf = self.get_font(font_id)?;
        let units_per_em = ttf.units_per_em;
        let advance_fu = ttf.advance_width(glyph_id);
        let advance_px = (advance_fu as u32 * size as u32 + units_per_em as u32 / 2)
            / units_per_em as u32;

        // Try outline rasterization first
        if ttf.has_outlines {
            if let Some(outline) = ttf.glyph_outline(glyph_id) {
                let mut bitmap = if subpixel {
                    ttf_rasterizer::rasterize_subpixel(&outline, size as u32, units_per_em)?
                } else {
                    ttf_rasterizer::rasterize(&outline, size as u32, units_per_em)?
                };
                bitmap.advance = advance_px;
                return Some(self.cache_glyph(font_id, glyph_id, size, subpixel, bitmap));
            }
        }

        // Try bitmap glyph (CBDT/CBLC) — e.g. NotoColorEmoji
        if ttf.has_bitmaps {
            if let Some(bmp) = ttf.get_bitmap_glyph(glyph_id) {
                if let Some(png) = png_decode::decode_png(bmp.png_data) {
                    let scale_num = size as u32;
                    let scale_den = bmp.strike_ppem as u32;
                    if scale_den == 0 { return None; }
                    let scaled_w = ((png.width * scale_num) + scale_den / 2) / scale_den;
                    let scaled_h = ((png.height * scale_num) + scale_den / 2) / scale_den;
                    if scaled_w == 0 || scaled_h == 0 { return None; }

                    let rgba = if scaled_w == png.width && scaled_h == png.height {
                        png.data
                    } else {
                        png_decode::scale_rgba(&png.data, png.width, png.height, scaled_w, scaled_h)
                    };

                    let x_offset = (bmp.bearing_x as i32 * scale_num as i32) / scale_den as i32;
                    let y_offset = (bmp.bearing_y as i32 * scale_num as i32) / scale_den as i32;

                    return Some(self.cache_color_glyph(
                        font_id, glyph_id, size, subpixel,
                        scaled_w, scaled_h, x_offset, y_offset, advance_px, rgba,
                    ));
                }
            }
        }

        None
    }
}

fn line_height_internal(ttf: &TtfFont, size: u16) -> u32 {
    let ascent = ttf.ascent.unsigned_abs() as u32;
    let descent = ttf.descent.unsigned_abs() as u32;
    let line_gap = ttf.line_gap.max(0) as u32;
    (ascent + descent + line_gap) * size as u32 / ttf.units_per_em as u32
}

// ─── Gamma correction ────────────────────────────────────────────────────

/// Integer square root via Newton's method (for gamma LUT computation).
fn isqrt_u32(n: u32) -> u32 {
    if n == 0 { return 0; }
    let mut x = 1u32 << ((32 - n.leading_zeros() + 1) / 2);
    loop {
        let nx = (x + n / x) / 2;
        if nx >= x { return x; }
        x = nx;
    }
}

/// Build size-adaptive gamma LUTs. Called once from init().
///  - Strong (≤14px): (input + isqrt(input*255)) / 2 — ~50% boost on thin strokes
///  - Moderate (15-24px): (2*input + isqrt(input*255)) / 3 — ~33% boost
fn init_gamma_lut() {
    unsafe {
        GAMMA_LUT_S[0] = 0;
        GAMMA_LUT_M[0] = 0;
        for i in 1..256u32 {
            let sq = isqrt_u32(i * 255);
            GAMMA_LUT_S[i as usize] = ((i + sq + 1) / 2).min(255) as u8;
            GAMMA_LUT_M[i as usize] = ((i * 2 + sq + 1) / 3).min(255) as u8;
        }
    }
}

/// Select the appropriate gamma LUT for a given glyph size.
/// Returns a pointer to the 256-byte LUT, or None for large text (no correction).
#[inline(always)]
fn gamma_lut_for_size(size: u16) -> Option<&'static [u8; 256]> {
    if size <= 14 {
        Some(unsafe { &GAMMA_LUT_S })
    } else if size <= 24 {
        Some(unsafe { &GAMMA_LUT_M })
    } else {
        None
    }
}

// ─── State access ────────────────────────────────────────────────────────

fn get_mgr() -> Option<&'static mut FontManager> {
    unsafe {
        if FONT_MGR_PTR.is_null() {
            None
        } else {
            Some(&mut *FONT_MGR_PTR)
        }
    }
}

fn set_mgr_ptr(mgr: *mut FontManager) {
    unsafe {
        FONT_MGR_PTR = mgr;
    }
}

// ─── Public API (called from extern "C" exports) ────────────────────────

/// Initialize the font manager and load system fonts from embedded .rodata.
/// No disk I/O — fonts are compiled into the .so binary.
pub fn init() {
    if get_mgr().is_some() {
        return;
    }

    let mgr = Box::new(FontManager::new());
    let mgr_ptr = Box::into_raw(mgr);
    set_mgr_ptr(mgr_ptr);

    let mgr = unsafe { &mut *mgr_ptr };

    let embedded: [&'static [u8]; 5] = [
        crate::FONT_SFPRO,
        crate::FONT_SFPRO_BOLD,
        crate::FONT_SFPRO_THIN,
        crate::FONT_SFPRO_ITALIC,
        crate::FONT_ANDALE_MONO,
    ];

    for data in &embedded {
        if let Some(ttf) = TtfFont::parse_static(data) {
            mgr.add_font(ttf);
        } else {
            mgr.fonts.push(None);
        }
    }

    // Load emoji font (ID 5 = SYSTEM_FONT_EMOJI)
    if let Some(ttf) = TtfFont::parse_static(crate::FONT_EMOJI) {
        mgr.add_font(ttf);
    } else {
        mgr.fonts.push(None);
    }

    if syscall::gpu_has_accel() != 0 {
        mgr.subpixel_enabled = true;
    }

    init_gamma_lut();
}

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

    let actual_font_id = if mgr.get_font(font_id).is_some() { font_id } else { SYSTEM_FONT_ID };

    // Get UPM from font (immutable borrow)
    let upm = match mgr.get_font(actual_font_id) {
        Some(ttf) => ttf.units_per_em as u32,
        None => {
            let char_w = (size as u32 * 6) / 10;
            let w = text.chars().filter(|c| *c != '\n').count() as u32 * char_w;
            return (w, size as u32);
        }
    };

    let mut width = 0u32;
    let mut max_width = 0u32;
    let mut lines = 1u32;

    for ch in text.chars() {
        if ch == '\n' {
            max_width = max_width.max(width);
            width = 0;
            lines += 1;
            continue;
        }
        if ch == '\t' {
            // Use ASCII cache for space glyph
            if let Some(font) = mgr.get_font_mut(actual_font_id) {
                let space_gid = font.char_to_glyph_cached(b' ' as u32);
                let space_adv = font.ttf.advance_width(space_gid) as u32;
                width += space_adv * 4 * size as u32 / upm;
            }
            continue;
        }

        // Get glyph from primary font
        let gid = match mgr.get_font_mut(actual_font_id) {
            Some(font) => font.char_to_glyph_cached(ch as u32),
            None => 0,
        };

        // Fallback to emoji font if glyph missing
        if gid == 0 && actual_font_id != SYSTEM_FONT_EMOJI {
            if let Some(emoji_font) = mgr.get_font_mut(SYSTEM_FONT_EMOJI) {
                let emoji_gid = emoji_font.char_to_glyph_cached(ch as u32);
                if emoji_gid != 0 {
                    let adv = emoji_font.ttf.advance_width(emoji_gid) as u32;
                    let emoji_upm = emoji_font.ttf.units_per_em as u32;
                    width += (adv * size as u32 + emoji_upm / 2) / emoji_upm;
                    continue;
                }
            }
        }

        // Use primary font advance
        if let Some(font) = mgr.get_font_mut(actual_font_id) {
            let adv = font.ttf.advance_width(gid) as u32;
            width += (adv * size as u32 + upm / 2) / upm;
        }
    }
    max_width = max_width.max(width);

    let lh = match mgr.get_font(actual_font_id) {
        Some(ttf) => line_height_internal(ttf, size),
        None => size as u32,
    };
    (max_width, lines * lh)
}

/// Draw a string into an ARGB pixel buffer.
pub fn draw_string_buf(
    buf: *mut u32, buf_w: u32, buf_h: u32,
    x: i32, y: i32, color: u32,
    font_id: u16, size: u16, text: &str,
) {
    draw_string_buf_clipped(buf, buf_w, buf_h, x, y, color, font_id, size, text,
        0, 0, buf_w as i32, buf_h as i32);
}

/// Draw a string into an ARGB pixel buffer, clipped to a rectangle.
/// Clip rect is (clip_x, clip_y, clip_r, clip_b) — left/top/right/bottom in pixels.
pub fn draw_string_buf_clipped(
    buf: *mut u32, buf_w: u32, buf_h: u32,
    x: i32, y: i32, color: u32,
    font_id: u16, size: u16, text: &str,
    clip_x: i32, clip_y: i32, clip_r: i32, clip_b: i32,
) {
    let mgr = match ensure_init() {
        Some(m) => m,
        None => return,
    };

    // Clamp clip rect to buffer bounds
    let clip_x = clip_x.max(0);
    let clip_y = clip_y.max(0);
    let clip_r = clip_r.min(buf_w as i32);
    let clip_b = clip_b.min(buf_h as i32);
    if clip_x >= clip_r || clip_y >= clip_b { return; }

    // Read font smoothing mode from shared uisys page
    let smoothing = read_font_smoothing();
    let subpixel = smoothing == 2;
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

    let col_a = ((color >> 24) & 0xFF) as u32;
    let col_r = ((color >> 16) & 0xFF) as u8;
    let col_g = ((color >> 8) & 0xFF) as u8;
    let col_b = (color & 0xFF) as u8;

    for ch in text.chars() {
        // Early exit: cursor past right edge of clip rect
        if cx >= clip_r { break; }

        if ch == '\n' {
            cx = x;
            cy += lh;
            continue;
        }
        if ch == '\t' {
            cx += tab_advance;
            continue;
        }

        // Step 1: Get glyph ID from primary font
        let primary_gid = match mgr.get_font_mut(actual_font_id) {
            Some(f) => f.char_to_glyph_cached(ch as u32),
            None => continue,
        };

        // Step 2: Fallback to emoji font if glyph missing
        let (gid, render_font_id) = if primary_gid == 0 && actual_font_id != SYSTEM_FONT_EMOJI {
            match mgr.get_font_mut(SYSTEM_FONT_EMOJI) {
                Some(ef) => {
                    let eid = ef.char_to_glyph_cached(ch as u32);
                    if eid != 0 { (eid, SYSTEM_FONT_EMOJI) } else { (primary_gid, actual_font_id) }
                }
                None => (primary_gid, actual_font_id),
            }
        } else {
            (primary_gid, actual_font_id)
        };

        // Step 3: Compute advance width from the resolved font
        let advance_px = match mgr.get_font(render_font_id) {
            Some(ttf) => {
                let adv_fu = ttf.advance_width(gid);
                let font_upm = ttf.units_per_em as u32;
                (adv_fu as u32 * size as u32 + font_upm / 2) / font_upm
            }
            None => continue,
        };

        // Step 4: Rasterize/cache and draw
        let cache_idx = mgr.find_cached(render_font_id, gid, size, subpixel);
        let idx = if let Some(i) = cache_idx {
            i
        } else {
            match mgr.rasterize_and_cache(render_font_id, gid, size, subpixel) {
                Some(i) => i,
                None => {
                    cx += advance_px as i32;
                    continue;
                }
            }
        };

        let glyph = &mgr.cache[idx];
        if glyph.width > 0 && glyph.height > 0 {
            if glyph.is_color {
                // Color bitmap glyph (emoji)
                let gx = cx + glyph.x_offset;
                let gy = cy + ascent_px as i32 - glyph.y_offset;
                draw_glyph_color_buf(buf, buf_w, buf_h, gx, gy, glyph,
                    clip_x, clip_y, clip_r, clip_b);
            } else {
                let gx = cx + if glyph.subpixel { glyph.x_offset / 3 } else { glyph.x_offset };
                let gy = cy + ascent_px as i32 - glyph.y_offset;

                if subpixel && glyph.subpixel {
                    draw_glyph_subpixel_buf(buf, buf_w, buf_h, gx, gy, glyph, col_a, col_r, col_g, col_b,
                        clip_x, clip_y, clip_r, clip_b);
                } else {
                    draw_glyph_greyscale_buf(buf, buf_w, buf_h, gx, gy, glyph, col_a, col_r, col_g, col_b,
                        clip_x, clip_y, clip_r, clip_b, smoothing);
                }
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
    clip_x: i32, clip_y: i32, clip_r: i32, clip_b: i32,
    smoothing: u32,
) {
    let bw = glyph.width as i32;
    let bh = glyph.height as i32;
    let buf_len = sh as usize * sw as usize;
    let lut = if smoothing == 0 { None } else { gamma_lut_for_size(glyph.size) };

    for row in 0..bh {
        let py = y + row;
        if py < clip_y || py >= clip_b { continue; }
        for col in 0..bw {
            let px = x + col;
            if px < clip_x || px >= clip_r { continue; }
            let raw_cov = glyph.coverage[(row * bw + col) as usize];
            if raw_cov == 0 { continue; }

            // Mode 0 (no smoothing): binary threshold — full or nothing
            if smoothing == 0 {
                if raw_cov < 128 { continue; }
                let idx = (py as u32 * sw + px as u32) as usize;
                if idx >= buf_len { continue; }
                unsafe {
                    *buf.add(idx) = (col_a << 24) | ((col_r as u32) << 16) | ((col_g as u32) << 8) | col_b as u32;
                }
                continue;
            }

            let coverage = if let Some(tbl) = lut { tbl[raw_cov as usize] } else { raw_cov };
            let alpha = div255(coverage as u32 * col_a);
            if alpha == 0 { continue; }

            let idx = (py as u32 * sw + px as u32) as usize;
            if idx >= buf_len { continue; }
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
    clip_x: i32, clip_y: i32, clip_r: i32, clip_b: i32,
) {
    let pixel_w = (glyph.width / 3) as i32;
    let stride = glyph.width as usize;
    let bh = glyph.height as i32;
    let buf_len = sh as usize * sw as usize;
    let lut = gamma_lut_for_size(glyph.size);

    // Fixed-point reciprocal for / 10: (1 << 16) / 10 = 6553.6 → 6554
    const RECIP10: u32 = 6554;

    for row in 0..bh {
        let py = y + row;
        if py < clip_y || py >= clip_b { continue; }
        let row_start = row as usize * stride;
        if row_start + stride > glyph.coverage.len() { continue; }
        let cov_row = &glyph.coverage[row_start..row_start + stride];

        for col in 0..pixel_w {
            let px = x + col;
            if px < clip_x || px >= clip_r { continue; }
            let ci = col as usize * 3;
            let r_raw = cov_row[ci] as u32;
            let g_raw = cov_row[ci + 1] as u32;
            let b_raw = cov_row[ci + 2] as u32;
            if r_raw == 0 && g_raw == 0 && b_raw == 0 { continue; }

            // 5-tap FIR filter with fixed-point division (replaces / 10)
            let get = |i: usize| -> u32 {
                if i < stride { cov_row[i] as u32 } else { 0 }
            };
            let ci_i = ci as isize;
            let getl = |i: isize| -> u32 {
                if i >= 0 && (i as usize) < stride { cov_row[i as usize] as u32 } else { 0 }
            };
            let r_sum = getl(ci_i - 2) + getl(ci_i - 1) * 2 + r_raw * 4 + g_raw * 2 + b_raw + 5;
            let g_sum = getl(ci_i - 1) + r_raw * 2 + g_raw * 4 + b_raw * 2 + get(ci + 3) + 5;
            let b_sum = r_raw + g_raw * 2 + b_raw * 4 + get(ci + 3) * 2 + get(ci + 4) + 5;
            let r_val = ((r_sum * RECIP10) >> 16).min(255) as u8;
            let g_val = ((g_sum * RECIP10) >> 16).min(255) as u8;
            let b_val = ((b_sum * RECIP10) >> 16).min(255) as u8;
            let (r_filt, g_filt, b_filt) = if let Some(tbl) = lut {
                (tbl[r_val as usize], tbl[g_val as usize], tbl[b_val as usize])
            } else {
                (r_val, g_val, b_val)
            };

            let idx = (py as u32 * sw + px as u32) as usize;
            if idx >= buf_len { continue; }
            let dst = unsafe { *buf.add(idx) };
            let blended = subpixel_blend(dst, r_filt, g_filt, b_filt, col_r, col_g, col_b);
            unsafe { *buf.add(idx) = blended; }
        }
    }
}

/// Draw a color (RGBA) bitmap glyph — used for emoji.
fn draw_glyph_color_buf(
    buf: *mut u32, sw: u32, sh: u32,
    x: i32, y: i32, glyph: &CachedGlyph,
    clip_x: i32, clip_y: i32, clip_r: i32, clip_b: i32,
) {
    let bw = glyph.width as i32;
    let bh = glyph.height as i32;
    let buf_len = sh as usize * sw as usize;

    for row in 0..bh {
        let py = y + row;
        if py < clip_y || py >= clip_b { continue; }
        for col in 0..bw {
            let px = x + col;
            if px < clip_x || px >= clip_r { continue; }

            let si = (row * bw + col) as usize * 4;
            if si + 3 >= glyph.coverage.len() { continue; }
            let sr = glyph.coverage[si] as u32;
            let sg = glyph.coverage[si + 1] as u32;
            let sb = glyph.coverage[si + 2] as u32;
            let sa = glyph.coverage[si + 3] as u32;

            if sa == 0 { continue; }

            let idx = (py as u32 * sw + px as u32) as usize;
            if idx >= buf_len { continue; }
            if sa == 255 {
                unsafe { *buf.add(idx) = 0xFF000000 | (sr << 16) | (sg << 8) | sb; }
            } else {
                let dst = unsafe { *buf.add(idx) };
                let inv = 255 - sa;
                let dr = (dst >> 16) & 0xFF;
                let dg = (dst >> 8) & 0xFF;
                let db = dst & 0xFF;
                let nr = div255(sr * sa + dr * inv);
                let ng = div255(sg * sa + dg * inv);
                let nb = div255(sb * sa + db * inv);
                unsafe { *buf.add(idx) = 0xFF000000 | (nr << 16) | (ng << 8) | nb; }
            }
        }
    }
}

/// Division-free alpha blend for font glyph pixel.
#[inline(always)]
fn alpha_blend_pixel(dst: u32, alpha: u32, r: u8, g: u8, b: u8) -> u32 {
    let inv = 255 - alpha;
    let dr = (dst >> 16) & 0xFF;
    let dg = (dst >> 8) & 0xFF;
    let db = dst & 0xFF;
    let nr = div255(r as u32 * alpha + dr * inv);
    let ng = div255(g as u32 * alpha + dg * inv);
    let nb = div255(b as u32 * alpha + db * inv);
    0xFF000000 | (nr << 16) | (ng << 8) | nb
}

/// Division-free subpixel blend for LCD font rendering.
#[inline(always)]
fn subpixel_blend(dst: u32, r_cov: u8, g_cov: u8, b_cov: u8, col_r: u8, col_g: u8, col_b: u8) -> u32 {
    let dr = (dst >> 16) & 0xFF;
    let dg = (dst >> 8) & 0xFF;
    let db = dst & 0xFF;
    let ra = r_cov as u32;
    let ga = g_cov as u32;
    let ba = b_cov as u32;
    let nr = div255(col_r as u32 * ra + dr * (255 - ra));
    let ng = div255(col_g as u32 * ga + dg * (255 - ga));
    let nb = div255(col_b as u32 * ba + db * (255 - ba));
    0xFF000000 | (nr << 16) | (ng << 8) | nb
}
