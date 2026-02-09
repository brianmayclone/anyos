//! High-level 2D renderer providing geometric primitives (lines, circles,
//! rounded rectangles, gradients) on top of a borrowed [`Surface`].

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
    /// Wrap a surface for drawing. The renderer borrows the surface mutably.
    pub fn new(surface: &'a mut Surface) -> Self {
        Renderer { surface }
    }

    /// Fill the entire surface with a solid color.
    pub fn clear(&mut self, color: Color) {
        self.surface.fill(color);
    }

    /// Fill a rectangle with a solid color.
    pub fn fill_rect(&mut self, rect: Rect, color: Color) {
        self.surface.fill_rect(rect, color);
    }

    /// Draw a rectangular outline with the given border thickness.
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

    /// Draw a line between two points using Bresenham's algorithm.
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

    /// Draw a circle outline using the midpoint circle algorithm.
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

    /// Fill a solid circle centered at (cx, cy).
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

    /// Fill a rectangle with rounded corners of the given radius.
    pub fn fill_rounded_rect(&mut self, rect: Rect, radius: i32, color: Color) {
        let r = radius.min(rect.width as i32 / 2).min(rect.height as i32 / 2);
        if r <= 0 {
            self.surface.fill_rect(rect, color);
            return;
        }
        let ru = r as u32;

        // 3 non-overlapping body rects
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

        // Corner fills using pixel-center test for accuracy
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
                // Top-left
                self.surface.fill_rect(
                    Rect::new(rect.x + fill_start, rect.y + dy, fill_width, 1),
                    color,
                );
                // Top-right: mirror horizontally
                self.surface.fill_rect(
                    Rect::new(rect.right() - r, rect.y + dy, fill_width, 1),
                    color,
                );
                // Bottom-left: mirror vertically
                self.surface.fill_rect(
                    Rect::new(rect.x + fill_start, rect.bottom() - 1 - dy, fill_width, 1),
                    color,
                );
                // Bottom-right: mirror both
                self.surface.fill_rect(
                    Rect::new(rect.right() - r, rect.bottom() - 1 - dy, fill_width, 1),
                    color,
                );
            }
        }
    }

    // ─── Anti-aliased variants (used when gpu_accel is true) ──────────

    /// Fill a rounded rectangle with anti-aliased edges.
    /// Body fill is identical to `fill_rounded_rect`; only corner arc pixels
    /// are rendered with alpha coverage for smooth edges.
    pub fn fill_rounded_rect_aa(&mut self, rect: Rect, radius: i32, color: Color) {
        let r = radius.min(rect.width as i32 / 2).min(rect.height as i32 / 2);
        if r <= 0 {
            self.surface.fill_rect(rect, color);
            return;
        }
        let ru = r as u32;

        // 3 non-overlapping body rects (same as aliased version)
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

        // AA corner fills: compute alpha coverage for each pixel
        let r2x4 = (2 * r) * (2 * r); // (2r)^2
        let transition = 3 * r; // ~1.5 pixel transition zone for smoother AA
        for dy in 0..r {
            let cy = 2 * dy + 1 - 2 * r;
            let cy2 = cy * cy;

            // Find first fully-inside pixel for fast fill
            let mut fill_start = r;
            for dx in 0..r {
                let cx = 2 * dx + 1 - 2 * r;
                let dist_sq = cx * cx + cy2;
                if dist_sq <= r2x4 - transition {
                    fill_start = dx;
                    break;
                }
            }

            // Fill fully-inside span for all 4 corners
            let fill_width = (r - fill_start) as u32;
            if fill_width > 0 {
                // Top-left
                self.surface.fill_rect(
                    Rect::new(rect.x + fill_start, rect.y + dy, fill_width, 1),
                    color,
                );
                // Top-right
                self.surface.fill_rect(
                    Rect::new(rect.right() - r, rect.y + dy, fill_width, 1),
                    color,
                );
                // Bottom-left
                self.surface.fill_rect(
                    Rect::new(rect.x + fill_start, rect.bottom() - 1 - dy, fill_width, 1),
                    color,
                );
                // Bottom-right
                self.surface.fill_rect(
                    Rect::new(rect.right() - r, rect.bottom() - 1 - dy, fill_width, 1),
                    color,
                );
            }

            // AA edge pixels (from 0 to fill_start)
            for dx in 0..fill_start {
                let cx = 2 * dx + 1 - 2 * r;
                let dist_sq = cx * cx + cy2;

                if dist_sq >= r2x4 + transition {
                    continue; // fully outside
                }

                // Linear interpolation across 1-pixel boundary
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

                // Top-left corner
                self.surface.put_pixel(rect.x + dx, rect.y + dy, aa_color);
                // Top-right corner (mirror horizontally)
                self.surface.put_pixel(rect.right() - 1 - dx, rect.y + dy, aa_color);
                // Bottom-left corner (mirror vertically)
                self.surface.put_pixel(rect.x + dx, rect.bottom() - 1 - dy, aa_color);
                // Bottom-right corner (mirror both)
                self.surface.put_pixel(rect.right() - 1 - dx, rect.bottom() - 1 - dy, aa_color);
            }
        }
    }

    /// Fill a solid circle with anti-aliased edges.
    pub fn fill_circle_aa(&mut self, cx: i32, cy: i32, radius: i32, color: Color) {
        if radius <= 0 {
            return;
        }
        for dy in -radius..=radius {
            let y = cy + dy;
            // Compute horizontal extent using integer sqrt
            let inner_sq = radius * radius - dy * dy;
            let dx_max = isqrt(inner_sq);

            // Fill the fully interior span
            if dx_max > 1 {
                self.surface.fill_rect(
                    Rect::new(cx - dx_max + 1, y, (dx_max * 2 - 1) as u32, 1),
                    color,
                );
            }

            // AA edge pixels: check +-dx_max and +-dx_max-1 boundary
            // The exact boundary is at sqrt(r^2 - dy^2)
            // Use distance-based coverage: dist_sq = dx^2 + dy^2, compare to r^2
            let r_sq = radius * radius;
            for &dx in &[-dx_max - 1, -dx_max, dx_max, dx_max + 1] {
                let dist_sq = dx * dx + dy * dy;
                if dist_sq >= (radius + 1) * (radius + 1) {
                    continue; // far outside
                }
                if dist_sq <= (radius - 1) * (radius - 1) {
                    // Fully inside — already covered by fill_rect
                    continue;
                }
                // Linear coverage based on distance to boundary
                // dist = sqrt(dist_sq), boundary at radius
                // Use approximation: coverage ≈ (radius + 0.5 - approx_dist) * 255
                // Since sqrt isn't available, use: alpha = 255 * (r_sq + radius - dist_sq) / (2*radius)
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

    /// Draw a circle outline with anti-aliased edges (Wu's algorithm).
    pub fn draw_circle_aa(&mut self, cx: i32, cy: i32, radius: i32, color: Color) {
        if radius <= 0 {
            self.surface.put_pixel(cx, cy, color);
            return;
        }
        // Wu's circle: walk from 45° and draw 8 octants with coverage
        let mut x = radius;
        let mut y = 0;
        let r_sq = radius * radius;

        while x >= y {
            // For each (x, y), compute coverage of the "main" pixel
            let dist_sq = x * x + y * y;
            // Error from ideal circle
            let err = dist_sq - r_sq;
            // Approximate coverage: map error to alpha
            // When err=0, pixel is exactly on circle → full alpha
            // When |err| ~ radius, pixel is ~0.5px away → reduced alpha
            let outer_alpha = if radius > 0 {
                255 - (err.abs() * 255 / (2 * radius)).min(255)
            } else {
                255
            };
            let inner_alpha = 255 - outer_alpha;

            if outer_alpha > 0 {
                let a = (outer_alpha.min(255) as u32 * color.a as u32 / 255) as u8;
                let c = Color::with_alpha(a, color.r, color.g, color.b);
                // 8 octants
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
                // Inner pixels (x-1 or y+1 depending on octant)
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

    /// Draw a 1px rounded rectangle outline with anti-aliased corners.
    pub fn draw_rounded_rect_aa(&mut self, rect: Rect, radius: i32, color: Color) {
        let w = rect.width as i32;
        let h = rect.height as i32;
        if w <= 0 || h <= 0 { return; }
        let r = radius.min(w / 2).min(h / 2).max(0);
        // Straight edges (excluding corner regions)
        // Top
        for px in (rect.x + r)..(rect.x + w - r) {
            self.surface.put_pixel(px, rect.y, color);
        }
        // Bottom
        for px in (rect.x + r)..(rect.x + w - r) {
            self.surface.put_pixel(px, rect.y + h - 1, color);
        }
        // Left
        for py in (rect.y + r)..(rect.y + h - r) {
            self.surface.put_pixel(rect.x, py, color);
        }
        // Right
        for py in (rect.y + r)..(rect.y + h - r) {
            self.surface.put_pixel(rect.x + w - 1, py, color);
        }
        if r <= 0 { return; }
        // AA corner arcs: outline ring at the arc boundary, clipped to the
        // fill's outer extent so we never draw beyond the window shape.
        let r2x4 = (2 * r) * (2 * r);
        let fill_transition = 3 * r; // must match fill_rounded_rect_aa
        let half_width = 2 * r; // outline ring half-width in 2× coords (~1px)
        for dy in 0..r {
            let cy = 2 * dy + 1 - 2 * r;
            let cy2 = cy * cy;
            for dx in 0..r {
                let cx = 2 * dx + 1 - 2 * r;
                let dist_sq = cx * cx + cy2;

                // Clip to fill boundary (don't draw outside the filled shape)
                if dist_sq >= r2x4 + fill_transition {
                    continue;
                }
                // Skip pixels well inside the shape (not part of outline)
                if dist_sq <= r2x4 - half_width * 2 {
                    continue;
                }

                // Coverage: peak at the arc boundary, fading symmetrically
                let err = dist_sq - r2x4;
                let alpha = 255 - (err.abs() * 255 / (half_width * 2)).min(255);
                if alpha <= 0 { continue; }
                let a = (alpha.min(255) as u32 * color.a as u32 / 255) as u8;
                let aa_color = Color::with_alpha(a, color.r, color.g, color.b);

                // Top-left
                self.surface.put_pixel(rect.x + dx, rect.y + dy, aa_color);
                // Top-right
                self.surface.put_pixel(rect.x + w - 1 - dx, rect.y + dy, aa_color);
                // Bottom-left
                self.surface.put_pixel(rect.x + dx, rect.y + h - 1 - dy, aa_color);
                // Bottom-right
                self.surface.put_pixel(rect.x + w - 1 - dx, rect.y + h - 1 - dy, aa_color);
            }
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
