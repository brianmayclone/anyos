//! High-level 2D renderer providing geometric primitives on top of a RenderSurface.

use crate::color::Color;
use crate::rect::Rect;
use crate::surface::RenderSurface;

/// Integer square root (no floating point needed).
fn isqrt(n: i32) -> i32 {
    if n <= 0 {
        return 0;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

/// 2D renderer that draws primitives onto a surface.
pub struct Renderer<'a> {
    surface: &'a mut RenderSurface,
}

impl<'a> Renderer<'a> {
    pub fn new(surface: &'a mut RenderSurface) -> Self {
        Renderer { surface }
    }

    pub fn clear(&mut self, color: Color) {
        self.surface.fill(color);
    }

    pub fn fill_rect(&mut self, rect: Rect, color: Color) {
        self.surface.fill_rect(rect, color);
    }

    pub fn draw_rect(&mut self, rect: Rect, color: Color, thickness: u32) {
        let t = thickness as i32;
        self.surface.fill_rect(Rect::new(rect.x, rect.y, rect.width, thickness), color);
        self.surface.fill_rect(
            Rect::new(rect.x, rect.bottom() - t, rect.width, thickness),
            color,
        );
        self.surface.fill_rect(Rect::new(rect.x, rect.y, thickness, rect.height), color);
        self.surface.fill_rect(
            Rect::new(rect.right() - t, rect.y, thickness, rect.height),
            color,
        );
    }

    pub fn draw_line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: Color) {
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        let mut x = x0;
        let mut y = y0;

        loop {
            self.surface.put_pixel(x, y, color);
            if x == x1 && y == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    pub fn draw_circle(&mut self, cx: i32, cy: i32, radius: i32, color: Color) {
        let mut x = radius;
        let mut y = 0;
        let mut err = 1 - radius;

        while x >= y {
            self.surface.put_pixel(cx + x, cy + y, color);
            self.surface.put_pixel(cx - x, cy + y, color);
            self.surface.put_pixel(cx + x, cy - y, color);
            self.surface.put_pixel(cx - x, cy - y, color);
            self.surface.put_pixel(cx + y, cy + x, color);
            self.surface.put_pixel(cx - y, cy + x, color);
            self.surface.put_pixel(cx + y, cy - x, color);
            self.surface.put_pixel(cx - y, cy - x, color);

            y += 1;
            if err < 0 {
                err += 2 * y + 1;
            } else {
                x -= 1;
                err += 2 * (y - x) + 1;
            }
        }
    }

    pub fn fill_circle(&mut self, cx: i32, cy: i32, radius: i32, color: Color) {
        for dy in -radius..=radius {
            let dx = isqrt(radius * radius - dy * dy);
            let y = cy + dy;
            self.surface.fill_rect(
                Rect::new(cx - dx, y, (dx * 2 + 1) as u32, 1),
                color,
            );
        }
    }

    pub fn fill_rounded_rect(&mut self, rect: Rect, radius: i32, color: Color) {
        let r = radius.min(rect.width as i32 / 2).min(rect.height as i32 / 2);
        if r <= 0 {
            self.surface.fill_rect(rect, color);
            return;
        }
        let ru = r as u32;

        if rect.height > ru * 2 {
            self.surface.fill_rect(
                Rect::new(rect.x, rect.y + r, rect.width, rect.height - ru * 2),
                color,
            );
        }
        if rect.width > ru * 2 {
            self.surface.fill_rect(
                Rect::new(rect.x + r, rect.y, rect.width - ru * 2, ru),
                color,
            );
            self.surface.fill_rect(
                Rect::new(rect.x + r, rect.bottom() - r, rect.width - ru * 2, ru),
                color,
            );
        }

        let r2x4 = (2 * r) * (2 * r);
        for dy in 0..r {
            let cy = 2 * dy + 1 - 2 * r;
            let cy2 = cy * cy;
            let mut fill_start = r;
            for dx in 0..r {
                let cx = 2 * dx + 1 - 2 * r;
                if cx * cx + cy2 <= r2x4 {
                    fill_start = dx;
                    break;
                }
            }
            let fill_width = (r - fill_start) as u32;
            if fill_width > 0 {
                self.surface.fill_rect(
                    Rect::new(rect.x + fill_start, rect.y + dy, fill_width, 1),
                    color,
                );
                self.surface.fill_rect(
                    Rect::new(rect.right() - r, rect.y + dy, fill_width, 1),
                    color,
                );
                self.surface.fill_rect(
                    Rect::new(rect.x + fill_start, rect.bottom() - 1 - dy, fill_width, 1),
                    color,
                );
                self.surface.fill_rect(
                    Rect::new(rect.right() - r, rect.bottom() - 1 - dy, fill_width, 1),
                    color,
                );
            }
        }
    }

    pub fn fill_rounded_rect_aa(&mut self, rect: Rect, radius: i32, color: Color) {
        let r = radius.min(rect.width as i32 / 2).min(rect.height as i32 / 2);
        if r <= 0 {
            self.surface.fill_rect(rect, color);
            return;
        }
        let ru = r as u32;

        if rect.height > ru * 2 {
            self.surface.fill_rect(
                Rect::new(rect.x, rect.y + r, rect.width, rect.height - ru * 2),
                color,
            );
        }
        if rect.width > ru * 2 {
            self.surface.fill_rect(
                Rect::new(rect.x + r, rect.y, rect.width - ru * 2, ru),
                color,
            );
            self.surface.fill_rect(
                Rect::new(rect.x + r, rect.bottom() - r, rect.width - ru * 2, ru),
                color,
            );
        }

        let r2x4 = (2 * r) * (2 * r);
        let transition = 3 * r;
        for dy in 0..r {
            let cy = 2 * dy + 1 - 2 * r;
            let cy2 = cy * cy;

            let mut fill_start = r;
            for dx in 0..r {
                let cx = 2 * dx + 1 - 2 * r;
                let dist_sq = cx * cx + cy2;
                if dist_sq <= r2x4 - transition {
                    fill_start = dx;
                    break;
                }
            }

            let fill_width = (r - fill_start) as u32;
            if fill_width > 0 {
                self.surface.fill_rect(
                    Rect::new(rect.x + fill_start, rect.y + dy, fill_width, 1),
                    color,
                );
                self.surface.fill_rect(
                    Rect::new(rect.right() - r, rect.y + dy, fill_width, 1),
                    color,
                );
                self.surface.fill_rect(
                    Rect::new(rect.x + fill_start, rect.bottom() - 1 - dy, fill_width, 1),
                    color,
                );
                self.surface.fill_rect(
                    Rect::new(rect.right() - r, rect.bottom() - 1 - dy, fill_width, 1),
                    color,
                );
            }

            for dx in 0..fill_start {
                let cx = 2 * dx + 1 - 2 * r;
                let dist_sq = cx * cx + cy2;

                if dist_sq >= r2x4 + transition {
                    continue;
                }

                let alpha = if dist_sq <= r2x4 - transition {
                    255i32
                } else {
                    255 * (r2x4 + transition - dist_sq) / (2 * transition)
                };

                if alpha <= 0 {
                    continue;
                }
                let a = (alpha.min(255) as u32 * color.a as u32 / 255) as u8;
                let aa_color = Color::with_alpha(a, color.r, color.g, color.b);

                self.surface.put_pixel(rect.x + dx, rect.y + dy, aa_color);
                self.surface.put_pixel(rect.right() - 1 - dx, rect.y + dy, aa_color);
                self.surface.put_pixel(rect.x + dx, rect.bottom() - 1 - dy, aa_color);
                self.surface.put_pixel(rect.right() - 1 - dx, rect.bottom() - 1 - dy, aa_color);
            }
        }
    }

    pub fn fill_circle_aa(&mut self, cx: i32, cy: i32, radius: i32, color: Color) {
        if radius <= 0 {
            return;
        }
        for dy in -radius..=radius {
            let y = cy + dy;
            let inner_sq = radius * radius - dy * dy;
            let dx_max = isqrt(inner_sq);

            if dx_max > 1 {
                self.surface.fill_rect(
                    Rect::new(cx - dx_max + 1, y, (dx_max * 2 - 1) as u32, 1),
                    color,
                );
            }

            let r_sq = radius * radius;
            for &dx in &[-dx_max - 1, -dx_max, dx_max, dx_max + 1] {
                let dist_sq = dx * dx + dy * dy;
                if dist_sq >= (radius + 1) * (radius + 1) {
                    continue;
                }
                if dist_sq <= (radius - 1) * (radius - 1) {
                    continue;
                }
                let alpha = 255 * (r_sq + radius - dist_sq) / (2 * radius);
                if alpha <= 0 {
                    continue;
                }
                let a = (alpha.min(255) as u32 * color.a as u32 / 255) as u8;
                let aa_color = Color::with_alpha(a, color.r, color.g, color.b);
                self.surface.put_pixel(cx + dx, y, aa_color);
            }
        }
    }

    pub fn draw_circle_aa(&mut self, cx: i32, cy: i32, radius: i32, color: Color) {
        if radius <= 0 {
            self.surface.put_pixel(cx, cy, color);
            return;
        }
        let mut x = radius;
        let mut y = 0;
        let r_sq = radius * radius;

        while x >= y {
            let dist_sq = x * x + y * y;
            let err = dist_sq - r_sq;
            let outer_alpha = if radius > 0 {
                255 - (err.abs() * 255 / (2 * radius)).min(255)
            } else {
                255
            };
            let inner_alpha = 255 - outer_alpha;

            if outer_alpha > 0 {
                let a = (outer_alpha.min(255) as u32 * color.a as u32 / 255) as u8;
                let c = Color::with_alpha(a, color.r, color.g, color.b);
                self.surface.put_pixel(cx + x, cy + y, c);
                self.surface.put_pixel(cx - x, cy + y, c);
                self.surface.put_pixel(cx + x, cy - y, c);
                self.surface.put_pixel(cx - x, cy - y, c);
                self.surface.put_pixel(cx + y, cy + x, c);
                self.surface.put_pixel(cx - y, cy + x, c);
                self.surface.put_pixel(cx + y, cy - x, c);
                self.surface.put_pixel(cx - y, cy - x, c);
            }
            if inner_alpha > 0 && x > y {
                let a = (inner_alpha.min(255) as u32 * color.a as u32 / 255) as u8;
                let c = Color::with_alpha(a, color.r, color.g, color.b);
                self.surface.put_pixel(cx + x - 1, cy + y, c);
                self.surface.put_pixel(cx - x + 1, cy + y, c);
                self.surface.put_pixel(cx + x - 1, cy - y, c);
                self.surface.put_pixel(cx - x + 1, cy - y, c);
                self.surface.put_pixel(cx + y, cy + x - 1, c);
                self.surface.put_pixel(cx - y, cy + x - 1, c);
                self.surface.put_pixel(cx + y, cy - x + 1, c);
                self.surface.put_pixel(cx - y, cy - x + 1, c);
            }

            y += 1;
            if 2 * (x * x + y * y - r_sq) + 1 > 0 {
                x -= 1;
            }
        }
    }

    pub fn draw_rounded_rect_aa(&mut self, rect: Rect, radius: i32, color: Color) {
        let w = rect.width as i32;
        let h = rect.height as i32;
        if w <= 0 || h <= 0 { return; }
        let r = radius.min(w / 2).min(h / 2).max(0);
        for px in (rect.x + r)..(rect.x + w - r) {
            self.surface.put_pixel(px, rect.y, color);
        }
        for px in (rect.x + r)..(rect.x + w - r) {
            self.surface.put_pixel(px, rect.y + h - 1, color);
        }
        for py in (rect.y + r)..(rect.y + h - r) {
            self.surface.put_pixel(rect.x, py, color);
        }
        for py in (rect.y + r)..(rect.y + h - r) {
            self.surface.put_pixel(rect.x + w - 1, py, color);
        }
        if r <= 0 { return; }
        let r2x4 = (2 * r) * (2 * r);
        let fill_transition = 3 * r;
        let half_width = 2 * r;
        for dy in 0..r {
            let cy = 2 * dy + 1 - 2 * r;
            let cy2 = cy * cy;
            for dx in 0..r {
                let cx = 2 * dx + 1 - 2 * r;
                let dist_sq = cx * cx + cy2;

                if dist_sq >= r2x4 + fill_transition {
                    continue;
                }
                if dist_sq <= r2x4 - half_width * 2 {
                    continue;
                }

                let err = dist_sq - r2x4;
                let alpha = 255 - (err.abs() * 255 / (half_width * 2)).min(255);
                if alpha <= 0 { continue; }
                let a = (alpha.min(255) as u32 * color.a as u32 / 255) as u8;
                let aa_color = Color::with_alpha(a, color.r, color.g, color.b);

                self.surface.put_pixel(rect.x + dx, rect.y + dy, aa_color);
                self.surface.put_pixel(rect.x + w - 1 - dx, rect.y + dy, aa_color);
                self.surface.put_pixel(rect.x + dx, rect.y + h - 1 - dy, aa_color);
                self.surface.put_pixel(rect.x + w - 1 - dx, rect.y + h - 1 - dy, aa_color);
            }
        }
    }

    pub fn fill_gradient_h(&mut self, rect: Rect, left: Color, right: Color) {
        for x in 0..rect.width {
            let t = x as u32 * 255 / rect.width.max(1);
            let color = Color::new(
                ((left.r as u32 * (255 - t) + right.r as u32 * t) / 255) as u8,
                ((left.g as u32 * (255 - t) + right.g as u32 * t) / 255) as u8,
                ((left.b as u32 * (255 - t) + right.b as u32 * t) / 255) as u8,
            );
            self.surface.fill_rect(
                Rect::new(rect.x + x as i32, rect.y, 1, rect.height),
                color,
            );
        }
    }

    pub fn fill_gradient_v(&mut self, rect: Rect, top: Color, bottom: Color) {
        for y in 0..rect.height {
            let t = y as u32 * 255 / rect.height.max(1);
            let color = Color::new(
                ((top.r as u32 * (255 - t) + bottom.r as u32 * t) / 255) as u8,
                ((top.g as u32 * (255 - t) + bottom.g as u32 * t) / 255) as u8,
                ((top.b as u32 * (255 - t) + bottom.b as u32 * t) / 255) as u8,
            );
            self.surface.fill_rect(
                Rect::new(rect.x, rect.y + y as i32, rect.width, 1),
                color,
            );
        }
    }
}
