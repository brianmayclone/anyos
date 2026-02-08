use crate::graphics::color::Color;
use crate::graphics::rect::Rect;
use crate::graphics::surface::Surface;

/// Integer square root (no floating point needed)
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

/// 2D renderer that draws primitives onto a surface
pub struct Renderer<'a> {
    surface: &'a mut Surface,
}

impl<'a> Renderer<'a> {
    pub fn new(surface: &'a mut Surface) -> Self {
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
        // Top edge
        self.surface.fill_rect(Rect::new(rect.x, rect.y, rect.width, thickness), color);
        // Bottom edge
        self.surface.fill_rect(
            Rect::new(rect.x, rect.bottom() - t, rect.width, thickness),
            color,
        );
        // Left edge
        self.surface.fill_rect(Rect::new(rect.x, rect.y, thickness, rect.height), color);
        // Right edge
        self.surface.fill_rect(
            Rect::new(rect.right() - t, rect.y, thickness, rect.height),
            color,
        );
    }

    pub fn draw_line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: Color) {
        // Bresenham's line algorithm
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
        // Midpoint circle algorithm
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

        // Fill center rectangle (excluding corners)
        self.surface.fill_rect(
            Rect::new(rect.x + r, rect.y, rect.width - 2 * r as u32, rect.height),
            color,
        );
        // Fill left and right strips
        self.surface.fill_rect(
            Rect::new(rect.x, rect.y + r, r as u32, rect.height - 2 * r as u32),
            color,
        );
        self.surface.fill_rect(
            Rect::new(rect.right() - r, rect.y + r, r as u32, rect.height - 2 * r as u32),
            color,
        );

        // Fill corners with quarter circles
        for dy in 0..r {
            let dx = isqrt(r * r - dy * dy);
            // Top-left
            self.surface.fill_rect(
                Rect::new(rect.x + r - dx, rect.y + r - dy, dx as u32, 1),
                color,
            );
            // Top-right
            self.surface.fill_rect(
                Rect::new(rect.right() - r, rect.y + r - dy, dx as u32, 1),
                color,
            );
            // Bottom-left
            self.surface.fill_rect(
                Rect::new(rect.x + r - dx, rect.bottom() - r + dy, dx as u32, 1),
                color,
            );
            // Bottom-right
            self.surface.fill_rect(
                Rect::new(rect.right() - r, rect.bottom() - r + dy, dx as u32, 1),
                color,
            );
        }
    }

    /// Draw a horizontal gradient
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

    /// Draw a vertical gradient
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
