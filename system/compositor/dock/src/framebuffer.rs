//! Local ARGB framebuffer with 2D drawing primitives.
//!
//! Optimised for the dock's hot path: clear → shadow → pill → scaled icons.

use alloc::vec;
use alloc::vec::Vec;

use crate::theme::COLOR_TRANSPARENT;
use crate::types::Icon;

pub struct Framebuffer {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u32>,
}

impl Framebuffer {
    pub fn new(width: u32, height: u32) -> Self {
        Framebuffer {
            width,
            height,
            pixels: vec![COLOR_TRANSPARENT; (width as usize).saturating_mul(height as usize)],
        }
    }

    /// Clear the entire framebuffer to transparent (0x00000000).
    pub fn clear(&mut self) {
        // COLOR_TRANSPARENT is 0 — use write_bytes for fast memset
        unsafe {
            core::ptr::write_bytes(self.pixels.as_mut_ptr(), 0, self.pixels.len());
        }
    }

    /// Fill a rectangle, clipping once then iterating without per-pixel bounds checks.
    pub fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: u32) {
        let a = (color >> 24) & 0xFF;
        if a == 0 { return; }

        let fb_w = self.width as i32;
        let fb_h = self.height as i32;

        // Clip to framebuffer
        let x0 = x.max(0) as usize;
        let y0 = y.max(0) as usize;
        let x1 = (x + w as i32).min(fb_w) as usize;
        let y1 = (y + h as i32).min(fb_h) as usize;
        if x0 >= x1 || y0 >= y1 { return; }

        let stride = self.width as usize;

        if a >= 255 {
            // Opaque — fill rows directly (compiler can vectorise)
            for row in y0..y1 {
                let start = row * stride + x0;
                self.pixels[start..start + (x1 - x0)].fill(color);
            }
        } else {
            // Semi-transparent — alpha blend per pixel
            for row in y0..y1 {
                let start = row * stride + x0;
                let end = start + (x1 - x0);
                for p in &mut self.pixels[start..end] {
                    *p = alpha_blend(color, *p);
                }
            }
        }
    }

    /// Fill a rounded rectangle.
    pub fn fill_rounded_rect(&mut self, x: i32, y: i32, w: u32, h: u32, r: i32, color: u32) {
        let ru = r as u32;
        if ru == 0 || w < ru * 2 || h < ru * 2 {
            self.fill_rect(x, y, w, h, color);
            return;
        }
        // Centre body
        if h > ru * 2 {
            self.fill_rect(x, y + r, w, h - ru * 2, color);
        }
        // Top/bottom strips between corners
        if w > ru * 2 {
            self.fill_rect(x + r, y, w - ru * 2, ru, color);
            self.fill_rect(x + r, y + h as i32 - r, w - ru * 2, ru, color);
        }
        // Corner arcs
        let r2x4 = (2 * r) * (2 * r);
        for dy in 0..ru {
            let cy = 2 * dy as i32 + 1 - 2 * r;
            let cy2 = cy * cy;
            let mut fill_start = ru;
            for dx in 0..ru {
                let cx = 2 * dx as i32 + 1 - 2 * r;
                if cx * cx + cy2 <= r2x4 {
                    fill_start = dx;
                    break;
                }
            }
            let fill_width = ru - fill_start;
            if fill_width > 0 {
                let fs = fill_start as i32;
                self.fill_rect(x + fs, y + dy as i32, fill_width, 1, color);
                self.fill_rect(x + (w - ru) as i32, y + dy as i32, fill_width, 1, color);
                self.fill_rect(x + fs, y + (h as i32 - 1 - dy as i32), fill_width, 1, color);
                self.fill_rect(x + (w - ru) as i32, y + (h as i32 - 1 - dy as i32), fill_width, 1, color);
            }
        }
    }

    /// Fill a small circle (for running indicator dots).
    pub fn fill_circle(&mut self, cx: i32, cy: i32, r: i32, color: u32) {
        let r2 = r * r;
        for dy in -r..=r {
            let py = cy + dy;
            if py < 0 || py >= self.height as i32 { continue; }
            let dy2 = dy * dy;
            for dx in -r..=r {
                if dx * dx + dy2 <= r2 {
                    let px = cx + dx;
                    if px >= 0 && px < self.width as i32 {
                        self.pixels[(py as u32 * self.width + px as u32) as usize] = color;
                    }
                }
            }
        }
    }

    /// Blit an icon at 1:1 scale with fast path for fully in-bounds icons.
    pub fn blit_icon(&mut self, icon: &Icon, dst_x: i32, dst_y: i32) {
        let iw = icon.width as usize;
        let ih = icon.height as usize;
        let stride = self.width as usize;

        // Fast path: entirely within framebuffer
        if dst_x >= 0 && dst_y >= 0
            && (dst_x as usize + iw) <= self.width as usize
            && (dst_y as usize + ih) <= self.height as usize
        {
            let dx = dst_x as usize;
            let dy = dst_y as usize;
            for row in 0..ih {
                let dst_row = (dy + row) * stride + dx;
                let src_row = row * iw;
                for col in 0..iw {
                    let src_pixel = icon.pixels[src_row + col];
                    let a = (src_pixel >> 24) & 0xFF;
                    if a == 0 { continue; }
                    if a >= 255 {
                        self.pixels[dst_row + col] = src_pixel;
                    } else {
                        self.pixels[dst_row + col] = alpha_blend(src_pixel, self.pixels[dst_row + col]);
                    }
                }
            }
            return;
        }

        // Clipped path
        for row in 0..icon.height as i32 {
            let py = dst_y + row;
            if py < 0 || py >= self.height as i32 { continue; }
            for col in 0..icon.width as i32 {
                let px = dst_x + col;
                if px < 0 || px >= self.width as i32 { continue; }
                let src_pixel = icon.pixels[(row as u32 * icon.width + col as u32) as usize];
                let a = (src_pixel >> 24) & 0xFF;
                if a == 0 { continue; }
                let dst_idx = (py as u32 * self.width + px as u32) as usize;
                if a >= 255 {
                    self.pixels[dst_idx] = src_pixel;
                } else {
                    self.pixels[dst_idx] = alpha_blend(src_pixel, self.pixels[dst_idx]);
                }
            }
        }
    }

    /// Blit a scaled icon (nearest-neighbour) with fast path for in-bounds rendering.
    pub fn blit_icon_scaled(&mut self, icon: &Icon, dst_x: i32, dst_y: i32, dst_w: u32, dst_h: u32) {
        let src_w = icon.width;
        let src_h = icon.height;
        if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 { return; }

        let stride = self.width as usize;
        let src_stride = src_w as usize;

        // Fast path: entirely within framebuffer (no per-pixel bounds checks)
        if dst_x >= 0 && dst_y >= 0
            && (dst_x as u32 + dst_w) <= self.width
            && (dst_y as u32 + dst_h) <= self.height
        {
            let dx = dst_x as usize;
            let dy = dst_y as usize;
            for y in 0..dst_h as usize {
                let sy = y * src_h as usize / dst_h as usize;
                let dst_row = (dy + y) * stride + dx;
                let src_row = sy * src_stride;
                for x in 0..dst_w as usize {
                    let sx = x * src_w as usize / dst_w as usize;
                    let src_pixel = icon.pixels[src_row + sx];
                    let a = (src_pixel >> 24) & 0xFF;
                    if a == 0 { continue; }
                    if a >= 255 {
                        self.pixels[dst_row + x] = src_pixel;
                    } else {
                        self.pixels[dst_row + x] = alpha_blend(src_pixel, self.pixels[dst_row + x]);
                    }
                }
            }
            return;
        }

        // Clipped path
        for dy in 0..dst_h as i32 {
            let py = dst_y + dy;
            if py < 0 || py >= self.height as i32 { continue; }
            let sy = (dy as u32 * src_h / dst_h) as usize;
            for dx in 0..dst_w as i32 {
                let px = dst_x + dx;
                if px < 0 || px >= self.width as i32 { continue; }
                let sx = (dx as u32 * src_w / dst_w) as usize;
                let src_pixel = icon.pixels[sy * src_stride + sx];
                let a = (src_pixel >> 24) & 0xFF;
                if a == 0 { continue; }
                let dst_idx = (py as u32 * self.width + px as u32) as usize;
                if a >= 255 {
                    self.pixels[dst_idx] = src_pixel;
                } else {
                    self.pixels[dst_idx] = alpha_blend(src_pixel, self.pixels[dst_idx]);
                }
            }
        }
    }

    /// Render text into the framebuffer via the system font renderer.
    pub fn draw_text(&mut self, x: i32, y: i32, text: &str, color: u32) {
        anyos_std::ui::window::font_render_buf(
            crate::theme::FONT_ID, crate::theme::FONT_SIZE,
            &mut self.pixels, self.width, self.height, x, y, color, text,
        );
    }
}

/// Alpha-blend src over dst (ARGB format).
#[inline(always)]
pub fn alpha_blend(src: u32, dst: u32) -> u32 {
    let sa = (src >> 24) & 0xFF;
    let sr = (src >> 16) & 0xFF;
    let sg = (src >> 8) & 0xFF;
    let sb = src & 0xFF;
    let dr = (dst >> 16) & 0xFF;
    let dg = (dst >> 8) & 0xFF;
    let db = dst & 0xFF;
    let inv = 255 - sa;
    let or = (sr * sa + dr * inv) / 255;
    let og = (sg * sa + dg * inv) / 255;
    let ob = (sb * sa + db * inv) / 255;
    let oa = sa + ((dst >> 24) & 0xFF) * inv / 255;
    (oa << 24) | (or << 16) | (og << 8) | ob
}
