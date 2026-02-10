//! ARGB8888 pixel surface backed by a raw pointer — no heap allocation.
//!
//! All rendering operates on caller-provided pixel buffers via raw pointers.
//! This is the userspace equivalent of the kernel's `Surface` type.

use crate::color::Color;
use crate::rect::Rect;

/// A pixel surface backed by a raw ARGB8888 buffer.
///
/// Does NOT own the pixel data — the caller is responsible for the buffer
/// lifetime. All methods are safe as long as the buffer has at least
/// `width * height` u32 elements.
pub struct RenderSurface {
    pub pixels: *mut u32,
    pub width: u32,
    pub height: u32,
}

impl RenderSurface {
    /// Create a surface descriptor from raw parts.
    ///
    /// # Safety
    /// `pixels` must point to a buffer of at least `width * height` u32 elements.
    pub unsafe fn from_raw(pixels: *mut u32, width: u32, height: u32) -> Self {
        RenderSurface { pixels, width, height }
    }

    /// Set a pixel with alpha blending. Out-of-bounds coordinates are silently ignored.
    pub fn put_pixel(&mut self, x: i32, y: i32, color: Color) {
        if x >= 0 && x < self.width as i32 && y >= 0 && y < self.height as i32 {
            let idx = (y as u32 * self.width + x as u32) as usize;
            unsafe {
                if color.a == 255 {
                    *self.pixels.add(idx) = color.to_u32();
                } else if color.a > 0 {
                    let dst = Color::from_u32(*self.pixels.add(idx));
                    *self.pixels.add(idx) = color.blend_over(dst).to_u32();
                }
            }
        }
    }

    /// Set a pixel without alpha blending (raw write). OOB silently ignored.
    pub fn set_pixel_raw(&mut self, x: i32, y: i32, color: Color) {
        if x >= 0 && x < self.width as i32 && y >= 0 && y < self.height as i32 {
            let idx = (y as u32 * self.width + x as u32) as usize;
            unsafe {
                *self.pixels.add(idx) = color.to_u32();
            }
        }
    }

    /// Set a pixel with LCD subpixel rendering.
    pub fn put_pixel_subpixel(&mut self, x: i32, y: i32, r_cov: u8, g_cov: u8, b_cov: u8, color: Color) {
        if x < 0 || x >= self.width as i32 || y < 0 || y >= self.height as i32 {
            return;
        }
        let idx = (y as u32 * self.width + x as u32) as usize;
        unsafe {
            let dst = Color::from_u32(*self.pixels.add(idx));
            let r = (color.r as u32 * r_cov as u32 + dst.r as u32 * (255 - r_cov as u32)) / 255;
            let g = (color.g as u32 * g_cov as u32 + dst.g as u32 * (255 - g_cov as u32)) / 255;
            let b = (color.b as u32 * b_cov as u32 + dst.b as u32 * (255 - b_cov as u32)) / 255;
            *self.pixels.add(idx) = Color::new(r as u8, g as u8, b as u8).to_u32();
        }
    }

    /// Read a pixel. Returns `Color::TRANSPARENT` for out-of-bounds coordinates.
    pub fn get_pixel(&self, x: i32, y: i32) -> Color {
        if x >= 0 && x < self.width as i32 && y >= 0 && y < self.height as i32 {
            let idx = (y as u32 * self.width + x as u32) as usize;
            unsafe { Color::from_u32(*self.pixels.add(idx)) }
        } else {
            Color::TRANSPARENT
        }
    }

    /// Fill the entire surface with a solid color (no blending).
    pub fn fill(&mut self, color: Color) {
        let val = color.to_u32();
        let len = (self.width * self.height) as usize;
        unsafe {
            for i in 0..len {
                *self.pixels.add(i) = val;
            }
        }
    }

    /// Fill a rectangle with the given color. Opaque colors use direct writes;
    /// semi-transparent colors are alpha-blended per pixel.
    pub fn fill_rect(&mut self, rect: Rect, color: Color) {
        let x0 = rect.x.max(0) as u32;
        let y0 = rect.y.max(0) as u32;
        let x1 = (rect.right() as u32).min(self.width);
        let y1 = (rect.bottom() as u32).min(self.height);

        let val = color.to_u32();
        for y in y0..y1 {
            let row_base = (y * self.width) as usize;
            if color.a == 255 {
                for x in x0..x1 {
                    unsafe { *self.pixels.add(row_base + x as usize) = val; }
                }
            } else {
                for x in x0..x1 {
                    let idx = row_base + x as usize;
                    unsafe {
                        let dst = Color::from_u32(*self.pixels.add(idx));
                        *self.pixels.add(idx) = color.blend_over(dst).to_u32();
                    }
                }
            }
        }
    }

    /// Blit a rectangular region from `src` onto this surface.
    ///
    /// `src_opaque`: if true, uses memcpy fast path (no alpha blending).
    pub fn blit_rect(
        &mut self,
        src: *const u32,
        sw: u32,
        sh: u32,
        src_rect: Rect,
        dx: i32,
        dy: i32,
        src_opaque: bool,
    ) {
        // Clip source rect to source surface bounds
        let sr_x0 = src_rect.x.max(0) as u32;
        let sr_y0 = src_rect.y.max(0) as u32;
        let sr_x1 = (src_rect.right() as u32).min(sw);
        let sr_y1 = (src_rect.bottom() as u32).min(sh);
        if sr_x0 >= sr_x1 || sr_y0 >= sr_y1 { return; }

        let mut copy_x = dx + (sr_x0 as i32 - src_rect.x);
        let mut copy_y = dy + (sr_y0 as i32 - src_rect.y);
        let mut src_sx = sr_x0;
        let mut src_sy = sr_y0;
        let mut copy_w = (sr_x1 - sr_x0) as i32;
        let mut copy_h = (sr_y1 - sr_y0) as i32;

        if copy_x < 0 { src_sx += (-copy_x) as u32; copy_w += copy_x; copy_x = 0; }
        if copy_y < 0 { src_sy += (-copy_y) as u32; copy_h += copy_y; copy_y = 0; }
        if copy_x + copy_w > self.width as i32 { copy_w = self.width as i32 - copy_x; }
        if copy_y + copy_h > self.height as i32 { copy_h = self.height as i32 - copy_y; }
        if copy_w <= 0 || copy_h <= 0 { return; }

        let cw = copy_w as usize;
        for row in 0..copy_h as u32 {
            let sy = src_sy + row;
            let dy_row = copy_y as u32 + row;
            let src_start = (sy * sw + src_sx) as usize;
            let dst_start = (dy_row * self.width + copy_x as u32) as usize;

            unsafe {
                let src_row = core::slice::from_raw_parts(src.add(src_start), cw);
                let dst_row = core::slice::from_raw_parts_mut(self.pixels.add(dst_start), cw);

                if src_opaque {
                    dst_row.copy_from_slice(src_row);
                } else {
                    // Opaque span batching
                    let mut i = 0;
                    while i < cw {
                        let span_start = i;
                        while i < cw && (src_row[i] >> 24) >= 255 {
                            i += 1;
                        }
                        if i > span_start {
                            dst_row[span_start..i].copy_from_slice(&src_row[span_start..i]);
                        }
                        while i < cw && (src_row[i] >> 24) < 255 {
                            let sp = src_row[i];
                            let a = sp >> 24;
                            if a > 0 {
                                let sc = Color::from_u32(sp);
                                let dc = Color::from_u32(dst_row[i]);
                                dst_row[i] = sc.blend_over(dc).to_u32();
                            }
                            i += 1;
                        }
                    }
                }
            }
        }
    }
}
