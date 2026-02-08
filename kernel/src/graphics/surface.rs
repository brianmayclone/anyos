use crate::graphics::color::Color;
use crate::graphics::rect::Rect;
use alloc::vec::Vec;

/// A pixel surface (ARGB8888 bitmap)
pub struct Surface {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u32>, // ARGB8888
    /// Hint: all pixels are fully opaque (alpha == 255). Enables fast blit path.
    pub opaque: bool,
}

impl Surface {
    pub fn new(width: u32, height: u32) -> Self {
        let size = (width * height) as usize;
        Surface {
            width,
            height,
            pixels: alloc::vec![0u32; size],
            opaque: false,
        }
    }

    pub fn new_with_color(width: u32, height: u32, color: Color) -> Self {
        let size = (width * height) as usize;
        Surface {
            width,
            height,
            pixels: alloc::vec![color.to_u32(); size],
            opaque: color.a == 255,
        }
    }

    pub fn put_pixel(&mut self, x: i32, y: i32, color: Color) {
        if x >= 0 && x < self.width as i32 && y >= 0 && y < self.height as i32 {
            let idx = (y as u32 * self.width + x as u32) as usize;
            if color.a == 255 {
                self.pixels[idx] = color.to_u32();
            } else if color.a > 0 {
                let dst = Color::from_u32(self.pixels[idx]);
                self.pixels[idx] = color.blend_over(dst).to_u32();
            }
        }
    }

    pub fn get_pixel(&self, x: i32, y: i32) -> Color {
        if x >= 0 && x < self.width as i32 && y >= 0 && y < self.height as i32 {
            let idx = (y as u32 * self.width + x as u32) as usize;
            Color::from_u32(self.pixels[idx])
        } else {
            Color::TRANSPARENT
        }
    }

    pub fn fill(&mut self, color: Color) {
        let val = color.to_u32();
        for pixel in self.pixels.iter_mut() {
            *pixel = val;
        }
    }

    pub fn fill_rect(&mut self, rect: Rect, color: Color) {
        let x0 = rect.x.max(0) as u32;
        let y0 = rect.y.max(0) as u32;
        let x1 = (rect.right() as u32).min(self.width);
        let y1 = (rect.bottom() as u32).min(self.height);

        let val = color.to_u32();
        for y in y0..y1 {
            let row_start = (y * self.width + x0) as usize;
            let row_end = (y * self.width + x1) as usize;
            if color.a == 255 {
                for pixel in &mut self.pixels[row_start..row_end] {
                    *pixel = val;
                }
            } else {
                for pixel in &mut self.pixels[row_start..row_end] {
                    let dst = Color::from_u32(*pixel);
                    *pixel = color.blend_over(dst).to_u32();
                }
            }
        }
    }

    /// Blit another surface onto this one at (dx, dy).
    /// Automatically uses fast path for opaque sources.
    pub fn blit(&mut self, src: &Surface, dx: i32, dy: i32) {
        if src.opaque {
            self.blit_opaque(src, dx, dy);
            return;
        }
        for sy in 0..src.height as i32 {
            let ty = dy + sy;
            if ty < 0 || ty >= self.height as i32 {
                continue;
            }
            for sx in 0..src.width as i32 {
                let tx = dx + sx;
                if tx < 0 || tx >= self.width as i32 {
                    continue;
                }
                let src_idx = (sy as u32 * src.width + sx as u32) as usize;
                let src_color = Color::from_u32(src.pixels[src_idx]);
                self.put_pixel(tx, ty, src_color);
            }
        }
    }

    /// Fast blit for fully opaque surfaces. Row-by-row slice copy, no alpha blending.
    fn blit_opaque(&mut self, src: &Surface, dx: i32, dy: i32) {
        let sx0 = if dx < 0 { (-dx) as u32 } else { 0 };
        let sy0 = if dy < 0 { (-dy) as u32 } else { 0 };
        let dx0 = dx.max(0) as u32;
        let dy0 = dy.max(0) as u32;

        let copy_w = if sx0 >= src.width || dx0 >= self.width {
            return;
        } else {
            (src.width - sx0).min(self.width - dx0) as usize
        };
        let copy_h = if sy0 >= src.height || dy0 >= self.height {
            return;
        } else {
            (src.height - sy0).min(self.height - dy0)
        };

        for row in 0..copy_h {
            let src_y = sy0 + row;
            let dst_y = dy0 + row;
            let src_start = (src_y * src.width + sx0) as usize;
            let dst_start = (dst_y * self.width + dx0) as usize;
            self.pixels[dst_start..dst_start + copy_w]
                .copy_from_slice(&src.pixels[src_start..src_start + copy_w]);
        }
    }

    /// Blit a region of the source surface.
    /// Automatically uses fast path for opaque sources.
    /// Non-opaque path uses row-based blitting with inline alpha checks.
    pub fn blit_rect(&mut self, src: &Surface, src_rect: Rect, dx: i32, dy: i32) {
        if src.opaque {
            self.blit_rect_opaque(src, src_rect, dx, dy);
            return;
        }
        // Clip source rect to source surface bounds
        let sr_x0 = src_rect.x.max(0) as u32;
        let sr_y0 = src_rect.y.max(0) as u32;
        let sr_x1 = (src_rect.right() as u32).min(src.width);
        let sr_y1 = (src_rect.bottom() as u32).min(src.height);
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
            let src_start = (sy * src.width + src_sx) as usize;
            let dst_start = (dy_row * self.width + copy_x as u32) as usize;
            let src_row = &src.pixels[src_start..src_start + cw];
            let dst_row = &mut self.pixels[dst_start..dst_start + cw];

            for i in 0..cw {
                let sp = src_row[i];
                let a = sp >> 24;
                if a >= 255 {
                    dst_row[i] = sp;
                } else if a > 0 {
                    let sc = Color::from_u32(sp);
                    let dc = Color::from_u32(dst_row[i]);
                    dst_row[i] = sc.blend_over(dc).to_u32();
                }
            }
        }
    }

    /// Fast blit_rect for fully opaque surfaces. Row-by-row slice copy.
    fn blit_rect_opaque(&mut self, src: &Surface, src_rect: Rect, dx: i32, dy: i32) {
        // Clip source rect to source surface bounds
        let sr_x0 = src_rect.x.max(0) as u32;
        let sr_y0 = src_rect.y.max(0) as u32;
        let sr_x1 = (src_rect.right() as u32).min(src.width);
        let sr_y1 = (src_rect.bottom() as u32).min(src.height);
        if sr_x0 >= sr_x1 || sr_y0 >= sr_y1 {
            return;
        }

        // Compute offset adjustments for clipping against destination
        let mut copy_x = dx + (sr_x0 as i32 - src_rect.x);
        let mut copy_y = dy + (sr_y0 as i32 - src_rect.y);
        let mut src_sx = sr_x0;
        let mut src_sy = sr_y0;
        let mut copy_w = (sr_x1 - sr_x0) as i32;
        let mut copy_h = (sr_y1 - sr_y0) as i32;

        // Clip left
        if copy_x < 0 {
            src_sx += (-copy_x) as u32;
            copy_w += copy_x;
            copy_x = 0;
        }
        // Clip top
        if copy_y < 0 {
            src_sy += (-copy_y) as u32;
            copy_h += copy_y;
            copy_y = 0;
        }
        // Clip right
        if copy_x + copy_w > self.width as i32 {
            copy_w = self.width as i32 - copy_x;
        }
        // Clip bottom
        if copy_y + copy_h > self.height as i32 {
            copy_h = self.height as i32 - copy_y;
        }

        if copy_w <= 0 || copy_h <= 0 {
            return;
        }
        let copy_w = copy_w as usize;

        for row in 0..copy_h as u32 {
            let sy = src_sy + row;
            let dy_row = copy_y as u32 + row;
            let src_start = (sy * src.width + src_sx) as usize;
            let dst_start = (dy_row * self.width + copy_x as u32) as usize;
            self.pixels[dst_start..dst_start + copy_w]
                .copy_from_slice(&src.pixels[src_start..src_start + copy_w]);
        }
    }
}
