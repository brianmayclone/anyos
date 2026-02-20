//! Local ARGB framebuffer with 2D drawing primitives.

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

    pub fn clear(&mut self) {
        for p in &mut self.pixels {
            *p = COLOR_TRANSPARENT;
        }
    }

    pub fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: u32) {
        let a = (color >> 24) & 0xFF;
        for row in 0..h as i32 {
            let py = y + row;
            if py < 0 || py >= self.height as i32 { continue; }
            for col in 0..w as i32 {
                let px = x + col;
                if px < 0 || px >= self.width as i32 { continue; }
                let idx = (py as u32 * self.width + px as u32) as usize;
                if a >= 255 {
                    self.pixels[idx] = color;
                } else if a > 0 {
                    self.pixels[idx] = alpha_blend(color, self.pixels[idx]);
                }
            }
        }
    }

    pub fn fill_rounded_rect(&mut self, x: i32, y: i32, w: u32, h: u32, r: i32, color: u32) {
        let ru = r as u32;
        if ru == 0 || w < ru * 2 || h < ru * 2 {
            self.fill_rect(x, y, w, h, color);
            return;
        }
        if h > ru * 2 {
            self.fill_rect(x, y + r, w, h - ru * 2, color);
        }
        if w > ru * 2 {
            self.fill_rect(x + r, y, w - ru * 2, ru, color);
            self.fill_rect(x + r, y + h as i32 - r, w - ru * 2, ru, color);
        }
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

    pub fn fill_circle(&mut self, cx: i32, cy: i32, r: i32, color: u32) {
        for dy in -r..=r {
            for dx in -r..=r {
                if dx * dx + dy * dy <= r * r {
                    let px = cx + dx;
                    let py = cy + dy;
                    if px >= 0 && px < self.width as i32 && py >= 0 && py < self.height as i32 {
                        let idx = (py as u32 * self.width + px as u32) as usize;
                        self.pixels[idx] = color;
                    }
                }
            }
        }
    }

    pub fn blit_icon(&mut self, icon: &Icon, dst_x: i32, dst_y: i32) {
        for row in 0..icon.height as i32 {
            let py = dst_y + row;
            if py < 0 || py >= self.height as i32 { continue; }
            for col in 0..icon.width as i32 {
                let px = dst_x + col;
                if px < 0 || px >= self.width as i32 { continue; }
                let src_idx = (row as u32 * icon.width + col as u32) as usize;
                let src_pixel = icon.pixels[src_idx];
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

    pub fn blit_icon_scaled(&mut self, icon: &Icon, dst_x: i32, dst_y: i32, dst_w: u32, dst_h: u32) {
        let src_w = icon.width;
        let src_h = icon.height;
        if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 { return; }
        for dy in 0..dst_h as i32 {
            let py = dst_y + dy;
            if py < 0 || py >= self.height as i32 { continue; }
            let sy = (dy as u32 * src_h / dst_h) as usize;
            for dx in 0..dst_w as i32 {
                let px = dst_x + dx;
                if px < 0 || px >= self.width as i32 { continue; }
                let sx = (dx as u32 * src_w / dst_w) as usize;
                let src_idx = sy * src_w as usize + sx;
                let src_pixel = icon.pixels[src_idx];
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

    pub fn draw_text(&mut self, x: i32, y: i32, text: &str, color: u32) {
        anyos_std::ui::window::font_render_buf(
            crate::theme::FONT_ID, crate::theme::FONT_SIZE,
            &mut self.pixels, self.width, self.height, x, y, color, text,
        );
    }
}

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
