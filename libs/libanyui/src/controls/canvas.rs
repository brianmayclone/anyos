//! Canvas — pixel drawing surface with software rendering primitives.
//!
//! The Canvas owns an ARGB pixel buffer. Clients draw into it using
//! functions like `clear`, `fill_rect`, `draw_line`, `draw_circle`, etc.
//! The buffer is blitted to the window's SHM surface during rendering.
//!
//! When `interactive` is true, mouse move events are tracked and fire
//! EVENT_CHANGE callbacks, enabling drag-to-draw behavior.

use alloc::vec;
use alloc::vec::Vec;
use crate::control::{Control, ControlBase, ControlKind, EventResponse};

pub struct Canvas {
    pub(crate) base: ControlBase,
    pub pixels: Vec<u32>,
    /// Last known mouse position (local coordinates).
    pub last_mouse_x: i32,
    pub last_mouse_y: i32,
    /// Mouse button state from last mouse_down (0 = none, 1 = left, 2 = right).
    pub mouse_button: u32,
    /// When true, handle_mouse_move fires EVENT_CHANGE for drag-drawing.
    pub interactive: bool,
}

impl Canvas {
    pub fn new(base: ControlBase) -> Self {
        let size = (base.w * base.h) as usize;
        Self {
            pixels: vec![0xFF000000; size], // opaque black
            base,
            last_mouse_x: 0,
            last_mouse_y: 0,
            mouse_button: 0,
            interactive: false,
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

    /// Read a single pixel. Returns 0 if out of bounds.
    pub fn get_pixel(&self, x: i32, y: i32) -> u32 {
        let w = self.stride();
        let h = self.height();
        if x >= 0 && y >= 0 && (x as u32) < w && (y as u32) < h {
            self.pixels[y as usize * w as usize + x as usize]
        } else {
            0
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

    /// Draw a line with configurable thickness using filled circles at each point.
    pub fn draw_thick_line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: u32, thickness: u32) {
        if thickness <= 1 {
            self.draw_line(x0, y0, x1, y1, color);
            return;
        }
        let radius = (thickness as i32) / 2;
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx: i32 = if x0 < x1 { 1 } else { -1 };
        let sy: i32 = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        let mut cx = x0;
        let mut cy = y0;

        loop {
            self.fill_circle(cx, cy, radius, color);
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

    /// Draw an ellipse outline using midpoint ellipse algorithm.
    pub fn draw_ellipse(&mut self, cx: i32, cy: i32, rx: i32, ry: i32, color: u32) {
        if rx <= 0 || ry <= 0 { return; }
        let rx2 = (rx as i64) * (rx as i64);
        let ry2 = (ry as i64) * (ry as i64);
        let mut x = 0i32;
        let mut y = ry;
        let mut px = 0i64;
        let mut py = 2 * rx2 * (y as i64);

        // Region 1
        let mut p = ry2 - rx2 * (ry as i64) + rx2 / 4;
        while px < py {
            self.set_pixel(cx + x, cy + y, color);
            self.set_pixel(cx - x, cy + y, color);
            self.set_pixel(cx + x, cy - y, color);
            self.set_pixel(cx - x, cy - y, color);
            x += 1;
            px += 2 * ry2;
            if p < 0 {
                p += ry2 + px;
            } else {
                y -= 1;
                py -= 2 * rx2;
                p += ry2 + px - py;
            }
        }

        // Region 2
        p = ry2 * ((x as i64) * (x as i64) + (x as i64))
            + rx2 * ((y as i64 - 1) * (y as i64 - 1)) - rx2 * ry2;
        while y >= 0 {
            self.set_pixel(cx + x, cy + y, color);
            self.set_pixel(cx - x, cy + y, color);
            self.set_pixel(cx + x, cy - y, color);
            self.set_pixel(cx - x, cy - y, color);
            y -= 1;
            py -= 2 * rx2;
            if p > 0 {
                p += rx2 - py;
            } else {
                x += 1;
                px += 2 * ry2;
                p += rx2 - py + px;
            }
        }
    }

    /// Draw a filled ellipse using horizontal scanlines.
    pub fn fill_ellipse(&mut self, cx: i32, cy: i32, rx: i32, ry: i32, color: u32) {
        if rx <= 0 || ry <= 0 { return; }
        let rx2 = (rx as i64) * (rx as i64);
        let ry2 = (ry as i64) * (ry as i64);
        let mut x = 0i32;
        let mut y = ry;
        let mut px = 0i64;
        let mut py = 2 * rx2 * (y as i64);

        // Region 1
        let mut p = ry2 - rx2 * (ry as i64) + rx2 / 4;
        let mut last_y = y + 1;
        while px < py {
            if y != last_y {
                self.draw_hline(cx - x, cx + x, cy + y, color);
                self.draw_hline(cx - x, cx + x, cy - y, color);
                last_y = y;
            }
            x += 1;
            px += 2 * ry2;
            if p < 0 {
                p += ry2 + px;
            } else {
                self.draw_hline(cx - x, cx + x, cy + y, color);
                self.draw_hline(cx - x, cx + x, cy - y, color);
                y -= 1;
                py -= 2 * rx2;
                p += ry2 + px - py;
                last_y = y;
            }
        }

        // Region 2
        p = ry2 * ((x as i64) * (x as i64) + (x as i64))
            + rx2 * ((y as i64 - 1) * (y as i64 - 1)) - rx2 * ry2;
        while y >= 0 {
            self.draw_hline(cx - x, cx + x, cy + y, color);
            self.draw_hline(cx - x, cx + x, cy - y, color);
            y -= 1;
            py -= 2 * rx2;
            if p > 0 {
                p += rx2 - py;
            } else {
                x += 1;
                px += 2 * ry2;
                p += rx2 - py + px;
            }
        }
    }

    /// Iterative scanline flood fill. Replaces `target_color` with `fill_color`.
    pub fn flood_fill(&mut self, x: i32, y: i32, fill_color: u32) {
        let w = self.stride() as i32;
        let h = self.height() as i32;
        if x < 0 || y < 0 || x >= w || y >= h { return; }

        let target_color = self.get_pixel(x, y);
        if target_color == fill_color { return; }

        let mut stack: Vec<(i32, i32)> = Vec::new();
        stack.push((x, y));

        while let Some((sx, sy)) = stack.pop() {
            let mut lx = sx;
            while lx > 0 && self.get_pixel(lx - 1, sy) == target_color {
                lx -= 1;
            }
            let mut cx = lx;
            let mut above = false;
            let mut below = false;
            while cx < w && self.get_pixel(cx, sy) == target_color {
                self.set_pixel(cx, sy, fill_color);
                if sy > 0 {
                    let c = self.get_pixel(cx, sy - 1) == target_color;
                    if c && !above { stack.push((cx, sy - 1)); }
                    above = c;
                }
                if sy < h - 1 {
                    let c = self.get_pixel(cx, sy + 1) == target_color;
                    if c && !below { stack.push((cx, sy + 1)); }
                    below = c;
                }
                cx += 1;
            }
        }
    }

    /// Copy pixel data from a source slice into the canvas buffer.
    pub fn copy_pixels_from(&mut self, src: &[u32]) {
        let len = src.len().min(self.pixels.len());
        self.pixels[..len].copy_from_slice(&src[..len]);
        self.base.mark_dirty();
    }

    /// Copy canvas pixel data into a destination slice. Returns count copied.
    pub fn copy_pixels_to(&self, dst: &mut [u32]) -> usize {
        let len = dst.len().min(self.pixels.len());
        dst[..len].copy_from_slice(&self.pixels[..len]);
        len
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

    fn set_size(&mut self, w: u32, h: u32) {
        let b = self.base_mut();
        if b.w != w || b.h != h {
            b.w = w;
            b.h = h;
            b.mark_dirty();
            // Resize pixel buffer to match new dimensions
            let expected = (w * h) as usize;
            if expected > 0 && self.pixels.len() != expected {
                self.pixels.resize(expected, 0xFF000000);
            }
        }
    }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        if self.pixels.is_empty() { return; }
        let b = self.base();
        let p = crate::draw::scale_bounds(ax, ay, b.x, b.y, b.w, b.h);
        // Canvas pixel buffer is in logical resolution. At 1x the sizes
        // match and we do a fast 1:1 copy. At higher DPI we nearest-neighbor
        // upscale from logical (b.w × b.h) to physical (p.w × p.h).
        if b.w == p.w && b.h == p.h {
            crate::draw::blit_buffer(surface, p.x, p.y, b.w, b.h, &self.pixels);
        } else {
            crate::draw::blit_buffer_scaled(surface, p.x, p.y, p.w, p.h, b.w, b.h, &self.pixels);
        }
    }

    fn is_interactive(&self) -> bool { true }

    fn handle_mouse_down(&mut self, lx: i32, ly: i32, button: u32) -> EventResponse {
        self.last_mouse_x = lx;
        self.last_mouse_y = ly;
        self.mouse_button = button;
        self.base.mark_dirty();
        EventResponse::CLICK
    }

    fn handle_mouse_move(&mut self, lx: i32, ly: i32) -> EventResponse {
        self.last_mouse_x = lx;
        self.last_mouse_y = ly;
        if self.interactive {
            self.base.mark_dirty();
            EventResponse::CHANGED
        } else {
            EventResponse::CONSUMED
        }
    }

    fn handle_mouse_up(&mut self, lx: i32, ly: i32, _button: u32) -> EventResponse {
        self.last_mouse_x = lx;
        self.last_mouse_y = ly;
        self.mouse_button = 0;
        self.base.mark_dirty();
        EventResponse::CONSUMED
    }

    fn handle_click(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        EventResponse::CLICK
    }
}
