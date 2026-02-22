//! 2D drawing primitives — rectangles, rounded rects, circles.

use crate::compositor::alpha_blend;

/// Fill a solid or alpha-blended rectangle.
/// Uses pre-clamped bounds and bulk slice::fill() for opaque fills.
pub(crate) fn fill_rect(
    pixels: &mut [u32],
    stride: u32,
    buf_h: u32,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    color: u32,
) {
    let a = (color >> 24) & 0xFF;
    if a == 0 { return; }
    // Pre-clamp bounds — eliminates per-pixel branch checks
    let x0 = x.max(0) as u32;
    let y0 = y.max(0) as u32;
    let x1 = ((x + w as i32) as u32).min(stride);
    let y1 = ((y + h as i32) as u32).min(buf_h);
    if x0 >= x1 || y0 >= y1 { return; }
    let cw = (x1 - x0) as usize;
    let plen = pixels.len();
    if a >= 255 {
        // Opaque: LLVM vectorizes slice::fill to rep stosd / SSE stores
        for row in y0..y1 {
            let off = (row * stride + x0) as usize;
            if off + cw <= plen {
                pixels[off..off + cw].fill(color);
            }
        }
    } else {
        for row in y0..y1 {
            let off = (row * stride + x0) as usize;
            for col in 0..cw {
                let i = off + col;
                if i < plen {
                    pixels[i] = alpha_blend(color, pixels[i]);
                }
            }
        }
    }
}

/// Fill a rounded rectangle (all four corners rounded).
pub(crate) fn fill_rounded_rect(
    pixels: &mut [u32],
    stride: u32,
    buf_h: u32,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    r: u32,
    color: u32,
) {
    if r == 0 || w < r * 2 || h < r * 2 {
        fill_rect(pixels, stride, buf_h, x, y, w, h, color);
        return;
    }
    // Center body
    if h > r * 2 {
        fill_rect(pixels, stride, buf_h, x, y + r as i32, w, h - r * 2, color);
    }
    // Top and bottom bands
    if w > r * 2 {
        fill_rect(pixels, stride, buf_h, x + r as i32, y, w - r * 2, r, color);
        fill_rect(
            pixels,
            stride,
            buf_h,
            x + r as i32,
            y + h as i32 - r as i32,
            w - r * 2,
            r,
            color,
        );
    }
    // Corners
    let r2x4 = (2 * r as i32) * (2 * r as i32);
    for dy in 0..r {
        let cy = 2 * dy as i32 + 1 - 2 * r as i32;
        let cy2 = cy * cy;
        let mut fill_start = r;
        for dx in 0..r {
            let cx = 2 * dx as i32 + 1 - 2 * r as i32;
            if cx * cx + cy2 <= r2x4 {
                fill_start = dx;
                break;
            }
        }
        let fill_width = r - fill_start;
        if fill_width > 0 {
            let fs = fill_start as i32;
            fill_rect(pixels, stride, buf_h, x + fs, y + dy as i32, fill_width, 1, color);
            fill_rect(
                pixels,
                stride,
                buf_h,
                x + (w - r) as i32,
                y + dy as i32,
                fill_width,
                1,
                color,
            );
            fill_rect(
                pixels,
                stride,
                buf_h,
                x + fs,
                y + h as i32 - 1 - dy as i32,
                fill_width,
                1,
                color,
            );
            fill_rect(
                pixels,
                stride,
                buf_h,
                x + (w - r) as i32,
                y + h as i32 - 1 - dy as i32,
                fill_width,
                1,
                color,
            );
        }
    }
}

/// Draw a 1px outline of a rounded rectangle.
pub(crate) fn draw_rounded_rect_outline(
    pixels: &mut [u32],
    stride: u32,
    buf_h: u32,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    r: u32,
    color: u32,
) {
    if w < 2 || h < 2 {
        return;
    }
    let r = r.min(w / 2).min(h / 2);

    let mut set_px = |px: i32, py: i32| {
        if px >= 0 && py >= 0 && (px as u32) < stride && (py as u32) < buf_h {
            let idx = (py as u32 * stride + px as u32) as usize;
            if idx < pixels.len() {
                pixels[idx] = color;
            }
        }
    };

    // Top edge (between rounded corners)
    for dx in r as i32..(w - r) as i32 {
        set_px(x + dx, y);
    }
    // Bottom edge
    for dx in r as i32..(w - r) as i32 {
        set_px(x + dx, y + h as i32 - 1);
    }
    // Left edge
    for dy in r as i32..(h - r) as i32 {
        set_px(x, y + dy);
    }
    // Right edge
    for dy in r as i32..(h - r) as i32 {
        set_px(x + w as i32 - 1, y + dy);
    }

    // Corner arcs
    let r2x4 = (2 * r as i32) * (2 * r as i32);
    for dy in 0..r {
        let cy = 2 * dy as i32 + 1 - 2 * r as i32;
        let cy2 = cy * cy;
        let mut fill_start = r;
        for dx in 0..r {
            let cx = 2 * dx as i32 + 1 - 2 * r as i32;
            if cx * cx + cy2 <= r2x4 {
                fill_start = dx;
                break;
            }
        }
        let fs = fill_start as i32;
        // Top-left
        set_px(x + fs, y + dy as i32);
        // Top-right
        set_px(x + w as i32 - 1 - fs, y + dy as i32);
        // Bottom-left
        set_px(x + fs, y + h as i32 - 1 - dy as i32);
        // Bottom-right
        set_px(x + w as i32 - 1 - fs, y + h as i32 - 1 - dy as i32);
    }
}

/// Fill rounded rect with only the top corners rounded.
pub(crate) fn fill_rounded_rect_top(
    pixels: &mut [u32],
    stride: u32,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    r: u32,
    color: u32,
) {
    if r == 0 || w < r * 2 {
        let buf_h = h;
        fill_rect(pixels, stride, buf_h + y as u32, x, y, w, h, color);
        return;
    }
    let buf_h = stride; // assume square-ish
    // Body below rounded top
    if h > r {
        fill_rect(pixels, stride, buf_h, x, y + r as i32, w, h - r, color);
    }
    // Top band
    if w > r * 2 {
        fill_rect(pixels, stride, buf_h, x + r as i32, y, w - r * 2, r, color);
    }
    // Top corners only
    let r2x4 = (2 * r as i32) * (2 * r as i32);
    for dy in 0..r {
        let cy = 2 * dy as i32 + 1 - 2 * r as i32;
        let cy2 = cy * cy;
        let mut fill_start = r;
        for dx in 0..r {
            let cx = 2 * dx as i32 + 1 - 2 * r as i32;
            if cx * cx + cy2 <= r2x4 {
                fill_start = dx;
                break;
            }
        }
        let fill_width = r - fill_start;
        if fill_width > 0 {
            let fs = fill_start as i32;
            fill_rect(pixels, stride, buf_h, x + fs, y + dy as i32, fill_width, 1, color);
            fill_rect(
                pixels,
                stride,
                buf_h,
                x + (w - r) as i32,
                y + dy as i32,
                fill_width,
                1,
                color,
            );
        }
    }
}

/// Integer square root (no floating point needed).
pub(crate) fn isqrt(n: i32) -> i32 {
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

/// Anti-aliased filled circle.
pub(crate) fn fill_circle(
    pixels: &mut [u32],
    stride: u32,
    buf_h: u32,
    cx: i32,
    cy: i32,
    r: i32,
    color: u32,
) {
    if r <= 0 {
        return;
    }
    let r_sq = r * r;
    let cr = (color >> 16) & 0xFF;
    let cg = (color >> 8) & 0xFF;
    let cb = color & 0xFF;
    let ca = (color >> 24) & 0xFF;

    for dy in -r..=r {
        let y = cy + dy;
        if y < 0 || y >= buf_h as i32 {
            continue;
        }
        let inner_sq = r_sq - dy * dy;
        let dx_max = isqrt(inner_sq);

        // Fill solid interior row
        if dx_max > 1 {
            fill_rect(
                pixels,
                stride,
                buf_h,
                cx - dx_max + 1,
                y,
                (dx_max * 2 - 1) as u32,
                1,
                color,
            );
        }

        // Alpha-blend edge pixels
        for &dx in &[-dx_max - 1, -dx_max, dx_max, dx_max + 1] {
            let px = cx + dx;
            if px < 0 || px >= stride as i32 {
                continue;
            }
            let dist_sq = dx * dx + dy * dy;
            if dist_sq >= (r + 1) * (r + 1) || dist_sq <= (r - 1) * (r - 1) {
                continue;
            }
            let alpha = 255 * (r_sq + r - dist_sq) / (2 * r);
            if alpha <= 0 {
                continue;
            }
            let a = (alpha.min(255) as u32 * ca / 255).min(255);
            let aa_color = (a << 24) | (cr << 16) | (cg << 8) | cb;
            let idx = (y as u32 * stride + px as u32) as usize;
            if idx < pixels.len() {
                if a >= 255 {
                    pixels[idx] = aa_color;
                } else {
                    pixels[idx] = alpha_blend(aa_color, pixels[idx]);
                }
            }
        }
    }
}
