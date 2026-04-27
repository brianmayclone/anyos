use super::raster::{blit_image_buf_clipped, fill_rect_buf};
use super::*;

pub(super) fn resolve_axis_origin(start: i32, size: i32, value: i32, is_percent: bool) -> i32 {
    if is_percent {
        start + (size as i64 * value as i64 / 10000) as i32
    } else {
        start + value
    }
}

pub(super) fn resolve_object_position_offset(free_space: i32, value: i32, is_percent: bool) -> i32 {
    if is_percent {
        (free_space as i64 * value as i64 / 10000) as i32
    } else {
        value
    }
}

pub(super) fn fit_contain_size(dw: i32, dh: i32, src_w: u32, src_h: u32) -> (i32, i32) {
    let sw = src_w as i64;
    let sh = src_h as i64;
    let dw64 = dw as i64;
    let dh64 = dh as i64;
    if sw * dh64 > sh * dw64 {
        (dw, (sh * dw64 / sw).max(1) as i32)
    } else {
        ((sw * dh64 / sh).max(1) as i32, dh)
    }
}

pub(super) fn fit_cover_size(dw: i32, dh: i32, src_w: u32, src_h: u32) -> (i32, i32) {
    let sw = src_w as i64;
    let sh = src_h as i64;
    let dw64 = dw as i64;
    let dh64 = dh as i64;
    if sw * dh64 < sh * dw64 {
        (dw, (sh * dw64 / sw).max(1) as i32)
    } else {
        ((sw * dh64 / sh).max(1) as i32, dh)
    }
}

/// Blit image with object-fit semantics.
pub(super) fn blit_image_scaled(
    buf: *mut u32,
    stride: u32,
    buf_h: u32,
    dx: i32,
    dy: i32,
    dw: i32,
    dh: i32,
    clip: (i32, i32, i32, i32),
    src: &[u32],
    src_w: u32,
    src_h: u32,
    fit: crate::style::ObjectFit,
    pos_x: i32,
    pos_x_is_percent: bool,
    pos_y: i32,
    pos_y_is_percent: bool,
) {
    use crate::style::ObjectFit;
    if dw <= 0 || dh <= 0 || src.is_empty() || src_w == 0 || src_h == 0 {
        return;
    }
    match fit {
        ObjectFit::Fill => {
            blit_image_buf_clipped(buf, stride, buf_h, dx, dy, dw, dh, clip, src, src_w, src_h);
        }
        ObjectFit::Contain | ObjectFit::ScaleDown => {
            let (fw, fh) = fit_contain_size(dw, dh, src_w, src_h);
            let (fw, fh) = if matches!(fit, ObjectFit::ScaleDown)
                && src_w as i32 <= dw
                && src_h as i32 <= dh
            {
                (src_w as i32, src_h as i32)
            } else {
                (fw, fh)
            };
            let ox = dx + resolve_object_position_offset(dw - fw, pos_x, pos_x_is_percent);
            let oy = dy + resolve_object_position_offset(dh - fh, pos_y, pos_y_is_percent);
            blit_image_buf_clipped(buf, stride, buf_h, ox, oy, fw, fh, clip, src, src_w, src_h);
        }
        ObjectFit::Cover => {
            let (fw, fh) = fit_cover_size(dw, dh, src_w, src_h);
            let ox = dx + resolve_object_position_offset(dw - fw, pos_x, pos_x_is_percent);
            let oy = dy + resolve_object_position_offset(dh - fh, pos_y, pos_y_is_percent);
            blit_image_buf_clipped(buf, stride, buf_h, ox, oy, fw, fh, clip, src, src_w, src_h);
        }
        ObjectFit::None => {
            let nw = src_w as i32;
            let nh = src_h as i32;
            let ox = dx + resolve_object_position_offset(dw - nw, pos_x, pos_x_is_percent);
            let oy = dy + resolve_object_position_offset(dh - nh, pos_y, pos_y_is_percent);
            blit_image_buf_clipped(buf, stride, buf_h, ox, oy, nw, nh, clip, src, src_w, src_h);
        }
    }
}

// ---------------------------------------------------------------------------
// Gradient and shadow helpers
// ---------------------------------------------------------------------------

/// Interpolate a color along a gradient at position `t` (0..10000).
pub(super) fn interpolate_gradient_color(stops: &[crate::style::GradientStop], t: i32) -> u32 {
    if stops.is_empty() {
        return 0xFF000000;
    }
    if stops.len() == 1 {
        return stops[0].color;
    }

    // Find the two stops that bracket `t`.
    let t_clamped = t.max(0).min(10000);
    let mut prev = &stops[0];
    for stop in &stops[1..] {
        if stop.position >= t_clamped {
            // Interpolate between prev and stop.
            let range = stop.position - prev.position;
            if range <= 0 {
                return stop.color;
            }
            let frac = ((t_clamped - prev.position) * 255 / range) as u32;
            return lerp_color(prev.color, stop.color, frac);
        }
        prev = stop;
    }
    stops.last().map(|s| s.color).unwrap_or(0xFF000000)
}

/// Linear interpolation between two ARGB colors. `frac` is 0..255.
fn lerp_color(c0: u32, c1: u32, frac: u32) -> u32 {
    let inv = 255 - frac;
    let a = (((c0 >> 24) & 0xFF) * inv + ((c1 >> 24) & 0xFF) * frac) / 255;
    let r = (((c0 >> 16) & 0xFF) * inv + ((c1 >> 16) & 0xFF) * frac) / 255;
    let g = (((c0 >> 8) & 0xFF) * inv + ((c1 >> 8) & 0xFF) * frac) / 255;
    let b = ((c0 & 0xFF) * inv + (c1 & 0xFF) * frac) / 255;
    (a << 24) | (r << 16) | (g << 8) | b
}

/// Apply an alpha value (0..255) to an existing color.
pub(super) fn alpha_blend(color: u32, alpha: u32) -> u32 {
    let existing_a = (color >> 24) & 0xFF;
    let new_a = (existing_a * alpha / 255).min(255);
    (new_a << 24) | (color & 0x00FFFFFF)
}

/// Darken a color by a percentage (0..100).
pub(super) fn darken_color(color: u32, amount: u32) -> u32 {
    let a = (color >> 24) & 0xFF;
    let r = ((color >> 16) & 0xFF) * (100 - amount) / 100;
    let g = ((color >> 8) & 0xFF) * (100 - amount) / 100;
    let b = (color & 0xFF) * (100 - amount) / 100;
    (a << 24) | (r << 16) | (g << 8) | b
}

/// Lighten a color by a percentage (0..100).
pub(super) fn lighten_color(color: u32, amount: u32) -> u32 {
    let a = (color >> 24) & 0xFF;
    let r = (((color >> 16) & 0xFF) + (255 - ((color >> 16) & 0xFF)) * amount / 100).min(255);
    let g = (((color >> 8) & 0xFF) + (255 - ((color >> 8) & 0xFF)) * amount / 100).min(255);
    let b = ((color & 0xFF) + (255 - (color & 0xFF)) * amount / 100).min(255);
    (a << 24) | (r << 16) | (g << 8) | b
}

// ---------------------------------------------------------------------------
// Rounded rectangle rendering
// ---------------------------------------------------------------------------

/// Fill a rounded rectangle. `radii` = [top-left, top-right, bottom-right, bottom-left].
/// Uses a simple per-pixel distance check against corner circles.
pub(super) fn fill_rounded_rect_buf(
    buf: *mut u32,
    stride: u32,
    buf_h: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    color: u32,
    radii: [i32; 4],
) {
    if w <= 0 || h <= 0 || buf.is_null() {
        return;
    }
    let s = stride as i32;
    let bh = buf_h as i32;

    let x0 = x.max(0);
    let y0 = y.max(0);
    let x1 = (x + w).min(s);
    let y1 = (y + h).min(bh);
    if x0 >= x1 || y0 >= y1 {
        return;
    }

    let alpha = (color >> 24) & 0xFF;
    if alpha == 0 {
        return;
    }

    let max_radius = (w.min(h) / 2).max(0);
    let [rtl, rtr, rbr, rbl] = [
        radii[0].max(0).min(max_radius),
        radii[1].max(0).min(max_radius),
        radii[2].max(0).min(max_radius),
        radii[3].max(0).min(max_radius),
    ];

    unsafe {
        for row in y0..y1 {
            let ry = row - y; // relative y within rect
            let offset = row as usize * stride as usize;
            for col in x0..x1 {
                let rx = col - x; // relative x within rect

                // Check if this pixel falls inside a rounded corner.
                let inside = is_inside_rounded_rect(rx, ry, w, h, rtl, rtr, rbr, rbl);
                if !inside {
                    continue;
                }

                let dst_idx = offset + col as usize;
                if alpha >= 255 {
                    *buf.add(dst_idx) = color;
                } else {
                    let dst = *buf.add(dst_idx);
                    let inv_a = 255 - alpha;
                    let r = (((color >> 16) & 0xFF) * alpha + ((dst >> 16) & 0xFF) * inv_a) / 255;
                    let g = (((color >> 8) & 0xFF) * alpha + ((dst >> 8) & 0xFF) * inv_a) / 255;
                    let b = ((color & 0xFF) * alpha + (dst & 0xFF) * inv_a) / 255;
                    *buf.add(dst_idx) = 0xFF000000 | (r << 16) | (g << 8) | b;
                }
            }
        }
    }
}

/// Check if point (px, py) relative to rect origin is inside a rounded rect.
#[inline]
fn is_inside_rounded_rect(
    px: i32,
    py: i32,
    w: i32,
    h: i32,
    rtl: i32,
    rtr: i32,
    rbr: i32,
    rbl: i32,
) -> bool {
    // Top-left corner
    if px < rtl && py < rtl {
        let dx = rtl - px;
        let dy = rtl - py;
        return dx * dx + dy * dy <= rtl * rtl;
    }
    // Top-right corner
    if px >= w - rtr && py < rtr {
        let dx = px - (w - rtr - 1);
        let dy = rtr - py;
        return dx * dx + dy * dy <= rtr * rtr;
    }
    // Bottom-right corner
    if px >= w - rbr && py >= h - rbr {
        let dx = px - (w - rbr - 1);
        let dy = py - (h - rbr - 1);
        return dx * dx + dy * dy <= rbr * rbr;
    }
    // Bottom-left corner
    if px < rbl && py >= h - rbl {
        let dx = rbl - px;
        let dy = py - (h - rbl - 1);
        return dx * dx + dy * dy <= rbl * rbl;
    }
    true
}

// ---------------------------------------------------------------------------
// Dashed / dotted border rendering
// ---------------------------------------------------------------------------

/// Fill a dashed/dotted line pattern within the given rect.
pub(super) fn fill_dashed_buf(
    buf: *mut u32,
    stride: u32,
    buf_h: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    color: u32,
    dash_len: i32,
    gap_len: i32,
    vertical: bool,
) {
    if w <= 0 || h <= 0 || buf.is_null() || dash_len <= 0 {
        return;
    }
    let cycle = dash_len + gap_len;
    if cycle <= 0 {
        return;
    }

    if vertical {
        // Vertical dashed line: iterate along height, fill dash segments.
        let mut pos = 0;
        while pos < h {
            let seg_len = dash_len.min(h - pos);
            if seg_len > 0 {
                fill_rect_buf(buf, stride, buf_h, x, y + pos, w, seg_len, color);
            }
            pos += cycle;
        }
    } else {
        // Horizontal dashed line: iterate along width, fill dash segments.
        let mut pos = 0;
        while pos < w {
            let seg_len = dash_len.min(w - pos);
            if seg_len > 0 {
                fill_rect_buf(buf, stride, buf_h, x + pos, y, seg_len, h, color);
            }
            pos += cycle;
        }
    }
}

// ---------------------------------------------------------------------------
// Trig approximations (no_std)
// ---------------------------------------------------------------------------

/// Sine approximation using Bhaskara I's formula. Input in radians.
pub(super) fn sin_approx(x: f32) -> f32 {
    // Normalize to [0, 2*PI)
    let pi = core::f32::consts::PI;
    let two_pi = 2.0 * pi;
    let mut a = x % two_pi;
    if a < 0.0 {
        a += two_pi;
    }

    let sign = if a > pi {
        a -= pi;
        -1.0
    } else {
        1.0
    };

    // Bhaskara I approximation for [0, PI]:
    // sin(x) ≈ 16x(PI-x) / (5*PI^2 - 4x(PI-x))
    let num = 16.0 * a * (pi - a);
    let den = 5.0 * pi * pi - 4.0 * a * (pi - a);
    if den.abs() < 0.001 {
        return 0.0;
    }
    sign * num / den
}

/// Cosine approximation: cos(x) = sin(x + PI/2).
pub(super) fn cos_approx(x: f32) -> f32 {
    sin_approx(x + core::f32::consts::FRAC_PI_2)
}
