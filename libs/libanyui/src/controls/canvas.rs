//! Canvas — pixel drawing surface with software rendering primitives.
//!
//! The Canvas owns an ARGB pixel buffer. Clients draw into it using
//! functions like `clear`, `fill_rect`, `draw_line`, `draw_circle`, etc.
//! The buffer is blitted to the window's SHM surface during rendering.

use alloc::vec;
use alloc::vec::Vec;
use crate::control::{Control, ControlBase, ControlKind, EventResponse};

pub struct Canvas {
    pub(crate) base: ControlBase,
    pub pixels: Vec<u32>,
}

impl Canvas {
    pub fn new(base: ControlBase) -> Self {
        let size = (base.w * base.h) as usize;
        Self {
            pixels: vec![0xFF000000; size], // opaque black
            base,
        }
    }

    // ── Drawing primitives ───────────────────────────────────────────

    #[inline]
    fn stride(&self) -> u32 { self.base.w }

    #[inline]
    fn height(&self) -> u32 { self.base.h }

    pub fn set_pixel(&mut self, x: i32, y: i32, color: u32) {
        let w = self.stride();
        let h = self.height();
        if x >= 0 && y >= 0 && (x as u32) < w && (y as u32) < h {
            self.pixels[y as usize * w as usize + x as usize] = color;
        }
    }

    pub fn clear(&mut self, color: u32) {
        self.pixels.fill(color);
    }

    pub fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: u32) {
        let stride = self.stride() as i32;
        let buf_h = self.height() as i32;
        let x0 = x.max(0);
        let y0 = y.max(0);
        let x1 = (x + w as i32).min(stride);
        let y1 = (y + h as i32).min(buf_h);

        for row in y0..y1 {
            let start = row as usize * stride as usize + x0 as usize;
            let end = start + (x1 - x0) as usize;
            if end <= self.pixels.len() {
                self.pixels[start..end].fill(color);
            }
        }
    }

    pub fn draw_line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: u32) {
        // Bresenham's line algorithm
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx: i32 = if x0 < x1 { 1 } else { -1 };
        let sy: i32 = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        let mut cx = x0;
        let mut cy = y0;

        loop {
            self.set_pixel(cx, cy, color);
            if cx == x1 && cy == y1 { break; }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                cx += sx;
            }
            if e2 <= dx {
                err += dx;
                cy += sy;
            }
        }
    }

    pub fn draw_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: u32, thickness: u32) {
        let t = thickness as i32;
        // Top edge
        self.fill_rect(x, y, w, thickness, color);
        // Bottom edge
        self.fill_rect(x, y + h as i32 - t, w, thickness, color);
        // Left edge
        self.fill_rect(x, y + t, thickness, h.saturating_sub(2 * thickness), color);
        // Right edge
        self.fill_rect(x + w as i32 - t, y + t, thickness, h.saturating_sub(2 * thickness), color);
    }

    pub fn draw_circle(&mut self, cx: i32, cy: i32, radius: i32, color: u32) {
        // Midpoint circle algorithm
        let mut x = 0i32;
        let mut y = radius;
        let mut d = 1 - radius;

        while x <= y {
            self.set_pixel(cx + x, cy + y, color);
            self.set_pixel(cx - x, cy + y, color);
            self.set_pixel(cx + x, cy - y, color);
            self.set_pixel(cx - x, cy - y, color);
            self.set_pixel(cx + y, cy + x, color);
            self.set_pixel(cx - y, cy + x, color);
            self.set_pixel(cx + y, cy - x, color);
            self.set_pixel(cx - y, cy - x, color);

            if d < 0 {
                d += 2 * x + 3;
            } else {
                d += 2 * (x - y) + 5;
                y -= 1;
            }
            x += 1;
        }
    }

    pub fn fill_circle(&mut self, cx: i32, cy: i32, radius: i32, color: u32) {
        let mut x = 0i32;
        let mut y = radius;
        let mut d = 1 - radius;

        while x <= y {
            // Draw horizontal lines for each octant pair
            self.draw_hline(cx - x, cx + x, cy + y, color);
            self.draw_hline(cx - x, cx + x, cy - y, color);
            self.draw_hline(cx - y, cx + y, cy + x, color);
            self.draw_hline(cx - y, cx + y, cy - x, color);

            if d < 0 {
                d += 2 * x + 3;
            } else {
                d += 2 * (x - y) + 5;
                y -= 1;
            }
            x += 1;
        }
    }

    #[inline]
    fn draw_hline(&mut self, x0: i32, x1: i32, y: i32, color: u32) {
        let stride = self.stride() as i32;
        let buf_h = self.height() as i32;
        if y < 0 || y >= buf_h { return; }
        let lx = x0.max(0);
        let rx = x1.min(stride - 1);
        for x in lx..=rx {
            self.pixels[y as usize * stride as usize + x as usize] = color;
        }
    }
}

impl Control for Canvas {
    fn base(&self) -> &ControlBase { &self.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.base }
    fn kind(&self) -> ControlKind { ControlKind::Canvas }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        if self.pixels.is_empty() { return; }
        let x = ax + self.base.x;
        let y = ay + self.base.y;
        // Blit our pixel buffer directly to the surface
        crate::draw::blit_buffer(surface, x, y, self.base.w, self.base.h, &self.pixels);
    }

    fn is_interactive(&self) -> bool { true }

    fn handle_mouse_down(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        EventResponse::CONSUMED
    }

    fn handle_mouse_up(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        EventResponse::CONSUMED
    }

    fn handle_click(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        EventResponse::CLICK
    }
}
