//! Local ARGB framebuffer with 2D drawing primitives for notification banners.

use alloc::vec;
use alloc::vec::Vec;

/// Fully transparent pixel.
const COLOR_TRANSPARENT: u32 = 0x00000000;

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

    /// Clear the entire framebuffer to transparent.
    pub fn clear(&mut self) {
        unsafe {
            core::ptr::write_bytes(self.pixels.as_mut_ptr(), 0, self.pixels.len());
        }
    }

    /// Fill a rectangle with clipping and alpha blending.
    pub fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: u32) {
        let a = (color >> 24) & 0xFF;
        if a == 0 { return; }

        let fb_w = self.width as i32;
        let fb_h = self.height as i32;
        let x0 = x.max(0) as usize;
        let y0 = y.max(0) as usize;
        let x1 = (x + w as i32).min(fb_w) as usize;
        let y1 = (y + h as i32).min(fb_h) as usize;
        if x0 >= x1 || y0 >= y1 { return; }

        let stride = self.width as usize;
        if a >= 255 {
            for row in y0..y1 {
                let start = row * stride + x0;
                self.pixels[start..start + (x1 - x0)].fill(color);
            }
        } else {
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

    /// Draw a 1px border outline of a rounded rectangle.
    pub fn stroke_rounded_rect(&mut self, x: i32, y: i32, w: u32, h: u32, r: i32, color: u32) {
        if w == 0 || h == 0 { return; }
        let ru = r as u32;

        // Top and bottom edges (between corners)
        if w > ru * 2 {
            self.fill_rect(x + r, y, w - ru * 2, 1, color);
            self.fill_rect(x + r, y + h as i32 - 1, w - ru * 2, 1, color);
        }
        // Left and right edges (between corners)
        if h > ru * 2 {
            self.fill_rect(x, y + r, 1, h - ru * 2, color);
            self.fill_rect(x + w as i32 - 1, y + r, 1, h - ru * 2, color);
        }

        if ru == 0 { return; }

        // Corner arcs (1px outline only)
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
            if fill_start < ru {
                let fs = fill_start as i32;
                // Top-left
                self.set_pixel(x + fs, y + dy as i32, color);
                // Top-right
                self.set_pixel(x + w as i32 - 1 - fs, y + dy as i32, color);
                // Bottom-left
                self.set_pixel(x + fs, y + h as i32 - 1 - dy as i32, color);
                // Bottom-right
                self.set_pixel(x + w as i32 - 1 - fs, y + h as i32 - 1 - dy as i32, color);
            }
        }
    }

    /// Set a single pixel with bounds checking.
    fn set_pixel(&mut self, x: i32, y: i32, color: u32) {
        if x >= 0 && y >= 0 && (x as u32) < self.width && (y as u32) < self.height {
            let idx = (y as u32 * self.width + x as u32) as usize;
            let a = (color >> 24) & 0xFF;
            if a >= 255 {
                self.pixels[idx] = color;
            } else if a > 0 {
                self.pixels[idx] = alpha_blend(color, self.pixels[idx]);
            }
        }
    }

    /// Blit a 16×16 ARGB icon at the given position.
    pub fn blit_icon_16(&mut self, icon: &[u32; 256], dst_x: i32, dst_y: i32) {
        let stride = self.width as usize;
        for row in 0..16i32 {
            let py = dst_y + row;
            if py < 0 || py >= self.height as i32 { continue; }
            for col in 0..16i32 {
                let px = dst_x + col;
                if px < 0 || px >= self.width as i32 { continue; }
                let src = icon[(row * 16 + col) as usize];
                let a = (src >> 24) & 0xFF;
                if a == 0 { continue; }
                let idx = py as usize * stride + px as usize;
                if a >= 255 {
                    self.pixels[idx] = src;
                } else {
                    self.pixels[idx] = alpha_blend(src, self.pixels[idx]);
                }
            }
        }
    }

    /// Render text into the framebuffer via the system font renderer.
    pub fn draw_text(&mut self, font_id: u16, font_size: u16, x: i32, y: i32, color: u32, text: &str) {
        anyos_std::ui::window::font_render_buf(
            font_id, font_size,
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
