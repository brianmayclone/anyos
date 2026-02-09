//! Font manager — loads TTF fonts from disk at runtime, caches rasterized
//! glyphs, and provides the unified text rendering API for the kernel.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::graphics::color::Color;
use crate::graphics::surface::Surface;
use crate::graphics::ttf::TtfFont;
use crate::graphics::ttf_rasterizer;
use crate::sync::spinlock::Spinlock;

/// Global flag: GPU acceleration is enabled (set by compositor).
/// Font manager reads this to decide greyscale vs LCD subpixel rendering.
static GPU_ACCEL: AtomicBool = AtomicBool::new(false);

/// Global font manager singleton.
static FONT_MGR: Spinlock<Option<FontManager>> = Spinlock::new(None);

/// Maximum number of cached glyphs before LRU eviction.
const MAX_CACHE_SIZE: usize = 512;

/// System font ID (always 0).
pub const SYSTEM_FONT_ID: u16 = 0;

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
    use_count: u32, // for LRU eviction
}

pub struct FontManager {
    fonts: Vec<Option<LoadedFont>>,
    cache: Vec<CachedGlyph>,
    access_counter: u32,
}

impl FontManager {
    fn new() -> Self {
        FontManager {
            fonts: Vec::new(),
            cache: Vec::new(),
            access_counter: 0,
        }
    }

    fn add_font(&mut self, ttf: TtfFont) -> u16 {
        let font = LoadedFont { ttf };
        // Find an empty slot or append
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

    fn remove_font(&mut self, font_id: u16) {
        if let Some(slot) = self.fonts.get_mut(font_id as usize) {
            *slot = None;
        }
        // Evict cached glyphs for this font
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
        // Find least recently used entry
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
        &mut self,
        font_id: u16,
        glyph_id: u16,
        size: u16,
        subpixel: bool,
        bitmap: ttf_rasterizer::GlyphBitmap,
    ) -> usize {
        self.evict_if_needed();
        self.access_counter += 1;
        let idx = self.cache.len();
        self.cache.push(CachedGlyph {
            font_id,
            glyph_id,
            size,
            subpixel,
            width: bitmap.width,
            height: bitmap.height,
            x_offset: bitmap.x_offset,
            y_offset: bitmap.y_offset,
            advance: bitmap.advance,
            coverage: bitmap.coverage,
            use_count: self.access_counter,
        });
        idx
    }

    fn rasterize_and_cache(
        &mut self,
        font_id: u16,
        glyph_id: u16,
        size: u16,
        subpixel: bool,
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

        // Override advance with properly scaled value
        bitmap.advance = advance_px;

        Some(self.cache_glyph(font_id, glyph_id, size, subpixel, bitmap))
    }
}

// ─── Public API ──────────────────────────────────────────────────────────

/// Set the GPU acceleration flag (called by compositor).
pub fn set_gpu_accel(enabled: bool) {
    GPU_ACCEL.store(enabled, Ordering::Relaxed);
}

/// Check if GPU acceleration is enabled.
pub fn gpu_accel_enabled() -> bool {
    GPU_ACCEL.load(Ordering::Relaxed)
}

/// Initialize the font manager and load the system font from disk.
pub fn init() {
    let mut mgr = FontManager::new();

    // Load system font from /system/fonts/sfpro.ttf
    match crate::fs::vfs::read_file_to_vec("/system/fonts/sfpro.ttf") {
        Ok(data) => {
            if let Some(ttf) = TtfFont::parse(data) {
                let id = mgr.add_font(ttf);
                crate::serial_println!(
                    "[OK] Font manager: sfpro.ttf loaded (font_id={}, {} glyphs)",
                    id,
                    mgr.get_font(id).map(|f| f.num_glyphs).unwrap_or(0)
                );
            } else {
                crate::serial_println!("[WARN] Font manager: failed to parse sfpro.ttf");
            }
        }
        Err(e) => {
            crate::serial_println!("[WARN] Font manager: sfpro.ttf not found ({:?}), using bitmap fallback", e);
        }
    }

    let mut guard = FONT_MGR.lock();
    *guard = Some(mgr);
}

/// Check if the font manager is initialized and has at least the system font.
pub fn is_ready() -> bool {
    let guard = FONT_MGR.lock();
    if let Some(ref mgr) = *guard {
        mgr.get_font(SYSTEM_FONT_ID).is_some()
    } else {
        false
    }
}

/// Load a TTF font from disk. Returns font_id on success.
pub fn load_font(path: &str) -> Option<u16> {
    let data = crate::fs::vfs::read_file_to_vec(path).ok()?;
    let ttf = TtfFont::parse(data)?;
    let mut guard = FONT_MGR.lock();
    let mgr = guard.as_mut()?;
    Some(mgr.add_font(ttf))
}

/// Unload a font by ID. Cannot unload system font (ID 0).
pub fn unload_font(font_id: u16) {
    if font_id == SYSTEM_FONT_ID {
        return;
    }
    let mut guard = FONT_MGR.lock();
    if let Some(ref mut mgr) = *guard {
        mgr.remove_font(font_id);
    }
}

/// Get line height for a font at a given size (in pixels).
pub fn line_height(font_id: u16, size: u16) -> u32 {
    let guard = FONT_MGR.lock();
    if let Some(ref mgr) = *guard {
        if let Some(ttf) = mgr.get_font(font_id) {
            let ascent = ttf.ascent.unsigned_abs() as u32;
            let descent = ttf.descent.unsigned_abs() as u32;
            let line_gap = ttf.line_gap.max(0) as u32;
            return (ascent + descent + line_gap) * size as u32 / ttf.units_per_em as u32;
        }
    }
    size as u32 // fallback
}

/// Measure the pixel dimensions of a text string.
pub fn measure_string(text: &str, font_id: u16, size: u16) -> (u32, u32) {
    let guard = FONT_MGR.lock();
    if let Some(ref mgr) = *guard {
        if let Some(ttf) = mgr.get_font(font_id) {
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
                    // 4 spaces worth
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
    }
    // Fallback estimate
    let char_w = (size as u32 * 6) / 10;
    let w = text.chars().filter(|c| *c != '\n').count() as u32 * char_w;
    (w, size as u32)
}

fn line_height_internal(ttf: &TtfFont, size: u16) -> u32 {
    let ascent = ttf.ascent.unsigned_abs() as u32;
    let descent = ttf.descent.unsigned_abs() as u32;
    let line_gap = ttf.line_gap.max(0) as u32;
    (ascent + descent + line_gap) * size as u32 / ttf.units_per_em as u32
}

/// Draw a string onto a surface at (x, y) with the given color, font, and size.
pub fn draw_string(
    surface: &mut Surface,
    x: i32,
    y: i32,
    text: &str,
    color: Color,
    font_id: u16,
    size: u16,
) {
    let subpixel = GPU_ACCEL.load(Ordering::Relaxed);
    let mut guard = FONT_MGR.lock();
    let mgr = match guard.as_mut() {
        Some(m) => m,
        None => return,
    };
    // Extract font metadata in limited scope (drop immutable borrow before mutable ops)
    let (upm, ascent_px, lh, tab_advance) = {
        let ttf = match mgr.get_font(font_id) {
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

        // Get glyph info in limited scope (borrow dropped before mutable methods)
        let (gid, advance_px) = {
            let ttf = match mgr.get_font(font_id) {
                Some(t) => t,
                None => continue,
            };
            let gid = ttf.char_to_glyph(ch as u32);
            let adv_fu = ttf.advance_width(gid);
            (gid, (adv_fu as u32 * size as u32 + upm / 2) / upm)
        };

        // Try cache first
        let cache_idx = mgr.find_cached(font_id, gid, size, subpixel);
        let idx = if let Some(i) = cache_idx {
            i
        } else {
            // Rasterize and cache
            match mgr.rasterize_and_cache(font_id, gid, size, subpixel) {
                Some(i) => i,
                None => {
                    cx += advance_px as i32;
                    continue;
                }
            }
        };

        let glyph = &mgr.cache[idx];
        if glyph.width > 0 && glyph.height > 0 {
            // For subpixel glyphs, x_offset is in 3× subpixel space — divide by 3
            let gx = cx + if glyph.subpixel { glyph.x_offset / 3 } else { glyph.x_offset };
            let gy = cy + ascent_px as i32 - glyph.y_offset;

            if subpixel && glyph.subpixel {
                draw_glyph_subpixel(surface, gx, gy, glyph, color);
            } else {
                draw_glyph_greyscale(surface, gx, gy, glyph, color);
            }
        }

        cx += advance_px as i32;
    }
}

/// Draw a single character. Returns advance width in pixels.
pub fn draw_char(
    surface: &mut Surface,
    x: i32,
    y: i32,
    ch: char,
    color: Color,
    font_id: u16,
    size: u16,
) -> u32 {
    let subpixel = GPU_ACCEL.load(Ordering::Relaxed);
    let mut guard = FONT_MGR.lock();
    let mgr = match guard.as_mut() {
        Some(m) => m,
        None => return size as u32 / 2,
    };
    // Extract font data in limited scope (drop immutable borrow before mutable ops)
    let (upm, ascent_px, gid, advance_px) = {
        let ttf = match mgr.get_font(font_id) {
            Some(t) => t,
            None => return size as u32 / 2,
        };
        let upm = ttf.units_per_em as u32;
        let ascent_px = (ttf.ascent.unsigned_abs() as u32 * size as u32) / upm;
        let gid = ttf.char_to_glyph(ch as u32);
        let adv_fu = ttf.advance_width(gid);
        let advance_px = (adv_fu as u32 * size as u32 + upm / 2) / upm;
        (upm, ascent_px, gid, advance_px)
    };

    let cache_idx = mgr.find_cached(font_id, gid, size, subpixel);
    let idx = if let Some(i) = cache_idx {
        i
    } else {
        match mgr.rasterize_and_cache(font_id, gid, size, subpixel) {
            Some(i) => i,
            None => return advance_px,
        }
    };

    let glyph = &mgr.cache[idx];
    if glyph.width > 0 && glyph.height > 0 {
        // For subpixel glyphs, x_offset is in 3× subpixel space — divide by 3
        let gx = x + if glyph.subpixel { glyph.x_offset / 3 } else { glyph.x_offset };
        let gy = y + ascent_px as i32 - glyph.y_offset;

        if subpixel && glyph.subpixel {
            draw_glyph_subpixel(surface, gx, gy, glyph, color);
        } else {
            draw_glyph_greyscale(surface, gx, gy, glyph, color);
        }
    }

    advance_px
}

// ─── Internal rendering ──────────────────────────────────────────────────

fn draw_glyph_greyscale(surface: &mut Surface, x: i32, y: i32, glyph: &CachedGlyph, color: Color) {
    let bw = glyph.width as i32;
    let bh = glyph.height as i32;
    let sw = surface.width as i32;
    let sh = surface.height as i32;

    for row in 0..bh {
        let py = y + row;
        if py < 0 || py >= sh {
            continue;
        }
        for col in 0..bw {
            let px = x + col;
            if px < 0 || px >= sw {
                continue;
            }
            let coverage = glyph.coverage[(row * bw + col) as usize];
            if coverage == 0 {
                continue;
            }
            let alpha = (coverage as u32 * color.a as u32) / 255;
            if alpha == 0 {
                continue;
            }
            let blended = Color::with_alpha(alpha as u8, color.r, color.g, color.b);
            surface.put_pixel(px, py, blended);
        }
    }
}

fn draw_glyph_subpixel(surface: &mut Surface, x: i32, y: i32, glyph: &CachedGlyph, color: Color) {
    // Subpixel coverage: glyph.width is 3× the output pixel width (one byte per subpixel).
    // Coverage row stride = glyph.width, output pixel count = glyph.width / 3.
    let pixel_w = (glyph.width / 3) as i32;
    let stride = glyph.width as usize;
    let bh = glyph.height as i32;
    let sw = surface.width as i32;
    let sh = surface.height as i32;

    for row in 0..bh {
        let py = y + row;
        if py < 0 || py >= sh {
            continue;
        }
        let row_start = row as usize * stride;
        if row_start + stride > glyph.coverage.len() {
            continue;
        }
        let cov_row = &glyph.coverage[row_start..row_start + stride];

        for col in 0..pixel_w {
            let px = x + col;
            if px < 0 || px >= sw {
                continue;
            }
            let ci = col as usize * 3;
            let r_raw = cov_row[ci] as u32;
            let g_raw = cov_row[ci + 1] as u32;
            let b_raw = cov_row[ci + 2] as u32;
            if r_raw == 0 && g_raw == 0 && b_raw == 0 {
                continue;
            }

            // LCD color fringe reduction: 5-tap horizontal FIR filter.
            // Weights [1, 2, 4, 2, 1] / 10 — wider kernel significantly
            // reduces visible red/blue halos while preserving subpixel
            // positioning benefit.
            let get = |i: usize| -> u32 {
                if i < stride { cov_row[i] as u32 } else { 0 }
            };
            let ci_i = ci as isize;
            let getl = |i: isize| -> u32 {
                if i >= 0 && (i as usize) < stride { cov_row[i as usize] as u32 } else { 0 }
            };
            // R subpixel (ci): neighbors at ci-2, ci-1, ci+1(=g), ci+2(=b)
            let r_filt = (getl(ci_i - 2) + getl(ci_i - 1) * 2 + r_raw * 4 + g_raw * 2 + b_raw + 5) / 10;
            // G subpixel (ci+1): neighbors at ci-1, ci(=r), ci+2(=b), ci+3
            let g_filt = (getl(ci_i - 1) + r_raw * 2 + g_raw * 4 + b_raw * 2 + get(ci + 3) + 5) / 10;
            // B subpixel (ci+2): neighbors at ci(=r), ci+1(=g), ci+3, ci+4
            let b_filt = (r_raw + g_raw * 2 + b_raw * 4 + get(ci + 3) * 2 + get(ci + 4) + 5) / 10;

            surface.put_pixel_subpixel(px, py, r_filt as u8, g_filt as u8, b_filt as u8, color);
        }
    }
}
