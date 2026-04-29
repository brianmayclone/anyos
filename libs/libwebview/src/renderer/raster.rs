use alloc::vec;
use alloc::vec::Vec;

use super::raster_utils::{
    alpha_blend, blit_image_scaled, cos_approx, darken_color, fill_dashed_buf,
    fill_rounded_border_buf, fill_rounded_rect_buf, fit_contain_size, fit_cover_size,
    interpolate_gradient_color, lighten_color, resolve_object_position_offset, sin_approx,
};
use super::{DrawCmd, DrawKind, ImageCache, MaskLayer};
use crate::style::{BackgroundImageVal, BackgroundRepeatVal, BackgroundSizeVal};

fn draw_ahem_string_buf(
    buf: *mut u32,
    stride: u32,
    buf_h: u32,
    x: i32,
    y: i32,
    color: u32,
    font_size: u16,
    text: &str,
) {
    let cell = font_size.max(1) as i32;
    let mut cursor_x = x;
    for ch in text.chars() {
        if !ch.is_whitespace() {
            fill_rect_buf(buf, stride, buf_h, cursor_x, y, cell, cell, color);
        }
        cursor_x += cell;
    }
}

fn blend_src_over(dst: u32, src: u32) -> u32 {
    let sa = (src >> 24) & 0xFF;
    if sa == 0 {
        return dst;
    }
    if sa == 255 {
        return src;
    }
    let da = (dst >> 24) & 0xFF;
    let inv = 255 - sa;
    let out_a = sa + da * inv / 255;
    let sr = (src >> 16) & 0xFF;
    let sg = (src >> 8) & 0xFF;
    let sb = src & 0xFF;
    let dr = (dst >> 16) & 0xFF;
    let dg = (dst >> 8) & 0xFF;
    let db = dst & 0xFF;
    let out_r = (sr * sa + dr * inv) / 255;
    let out_g = (sg * sa + dg * inv) / 255;
    let out_b = (sb * sa + db * inv) / 255;
    (out_a << 24) | (out_r << 16) | (out_g << 8) | out_b
}

fn lerp_u32(a: u32, b: u32, t: u32) -> u32 {
    (a * (255 - t) + b * t + 127) / 255
}

fn lerp_argb_premul(a: u32, b: u32, t: u32) -> u32 {
    let aa = (a >> 24) & 0xFF;
    let ba = (b >> 24) & 0xFF;
    let ar = (a >> 16) & 0xFF;
    let ag = (a >> 8) & 0xFF;
    let ab = a & 0xFF;
    let br = (b >> 16) & 0xFF;
    let bg = (b >> 8) & 0xFF;
    let bb = b & 0xFF;

    let out_a = lerp_u32(aa, ba, t);
    if out_a == 0 {
        return 0;
    }

    let ar_pm = ar * aa;
    let ag_pm = ag * aa;
    let ab_pm = ab * aa;
    let br_pm = br * ba;
    let bg_pm = bg * ba;
    let bb_pm = bb * ba;

    let r_pm = lerp_u32(ar_pm, br_pm, t);
    let g_pm = lerp_u32(ag_pm, bg_pm, t);
    let b_pm = lerp_u32(ab_pm, bb_pm, t);
    let r = ((r_pm + out_a / 2) / out_a).min(255);
    let g = ((g_pm + out_a / 2) / out_a).min(255);
    let b = ((b_pm + out_a / 2) / out_a).min(255);
    (out_a << 24) | (r << 16) | (g << 8) | b
}

fn draw_scaled_text_buf(
    buf: *mut u32,
    stride: u32,
    buf_h: u32,
    x: i32,
    y: i32,
    color: u32,
    font_id: u32,
    font_size: u16,
    scale_x_percent: i32,
    text: &str,
) {
    let scale = scale_x_percent.clamp(10, 400);
    let (measured_w, measured_h) = libfont_client::measure(font_id, font_size, text);
    let src_w = (measured_w as i32 + 4).max(1) as u32;
    let src_h = (measured_h.max(font_size as u32) + 4).max(1);
    let mut tmp = vec![0u32; (src_w as usize).saturating_mul(src_h as usize)];
    libfont_client::draw_string_buf(
        tmp.as_mut_ptr(),
        src_w,
        src_h,
        0,
        0,
        color,
        font_id,
        font_size,
        text,
    );

    let dst_w = ((src_w as i32 * scale + 50) / 100).max(1);
    let stride_usize = stride as usize;
    let buf_h_i32 = buf_h as i32;
    for dy in 0..(src_h as i32) {
        let out_y = y + dy;
        if out_y < 0 || out_y >= buf_h_i32 {
            continue;
        }
        for dx in 0..dst_w {
            let out_x = x + dx;
            if out_x < 0 || out_x >= stride as i32 {
                continue;
            }
            let src_center = ((dx * 100 + 50) / scale).saturating_sub(1);
            let sx0 = src_center.clamp(0, src_w as i32 - 1) as usize;
            let sx1 = (sx0 + 1).min(src_w as usize - 1);
            let frac = ((dx * 100 + 50) % scale) * 255 / scale;
            let row = dy as usize * src_w as usize;
            let src = lerp_argb_premul(tmp[row + sx0], tmp[row + sx1], frac as u32);
            if (src >> 24) == 0 {
                continue;
            }
            let dst_idx = out_y as usize * stride_usize + out_x as usize;
            unsafe {
                let dst = *buf.add(dst_idx);
                *buf.add(dst_idx) = blend_src_over(dst, src);
            }
        }
    }
}

fn rasterize_draw_cmd_basic(
    images: &ImageCache,
    cmd: &DrawCmd,
    buf: *mut u32,
    stride: u32,
    buf_h: u32,
    draw_y: i32,
    clip: (i32, i32, i32, i32),
) {
    match &cmd.kind {
        DrawKind::Rect { color } => {
            fill_rect_buf(buf, stride, buf_h, clip.0, clip.1, clip.2, clip.3, *color);
        }
        DrawKind::RoundedRect { color, radii } => {
            fill_rounded_rect_buf(
                buf, stride, buf_h, clip.0, clip.1, clip.2, clip.3, *color, *radii,
            );
        }
        DrawKind::RoundedBorder {
            color,
            radii,
            widths,
        } => {
            fill_rounded_border_buf(
                buf, stride, buf_h, clip.0, clip.1, clip.2, clip.3, *color, *radii, *widths,
            );
        }
        DrawKind::Triangle { color, p0, p1, p2 } => {
            fill_triangle_buf(buf, stride, buf_h, cmd.src_x, draw_y, *color, *p0, *p1, *p2);
        }
        DrawKind::DashedLine {
            color,
            dash_len,
            gap_len,
            vertical,
        } => {
            fill_dashed_buf(
                buf, stride, buf_h, clip.0, clip.1, clip.2, clip.3, *color, *dash_len, *gap_len,
                *vertical,
            );
        }
        DrawKind::Text {
            color,
            font_id,
            font_size,
            scale_x_percent,
            text,
        } => {
            #[cfg(feature = "host")]
            if std::env::var_os("SURF_DEBUG_PAINT_TEXT").is_some()
                && draw_y >= -200
                && draw_y < buf_h as i32
                && !text.trim().is_empty()
            {
                eprintln!(
                    "[libwebview] paint text x={} y={} w={} h={} color=0x{:08x} font={} size={} text={:?}",
                    cmd.src_x,
                    draw_y,
                    cmd.src_w,
                    cmd.src_h,
                    color,
                    font_id,
                    font_size,
                    text
                );
            }
            if crate::is_ahem_font_id(*font_id) {
                draw_ahem_string_buf(
                    buf, stride, buf_h, cmd.src_x, draw_y, *color, *font_size, text,
                );
            } else if *scale_x_percent != 100 {
                draw_scaled_text_buf(
                    buf,
                    stride,
                    buf_h,
                    cmd.src_x,
                    draw_y,
                    *color,
                    *font_id,
                    *font_size,
                    *scale_x_percent,
                    text,
                );
            } else {
                libfont_client::draw_string_buf(
                    buf, stride, buf_h, cmd.src_x, draw_y, *color, *font_id, *font_size, text,
                );
            }
        }
        DrawKind::Image {
            src,
            object_fit,
            object_position_x,
            object_position_x_is_percent,
            object_position_y,
            object_position_y_is_percent,
        } => {
            if let Some(entry) = images.get_ref(src) {
                if !entry.has_pixels() {
                    return;
                }
                blit_image_scaled(
                    buf,
                    stride,
                    buf_h,
                    cmd.src_x,
                    draw_y,
                    cmd.src_w,
                    cmd.src_h,
                    clip,
                    &entry.pixels,
                    entry.width,
                    entry.height,
                    *object_fit,
                    *object_position_x,
                    *object_position_x_is_percent,
                    *object_position_y,
                    *object_position_y_is_percent,
                );
            }
        }
    }
}

pub(super) fn rasterize_draw_cmd(
    images: &ImageCache,
    cmd: &DrawCmd,
    buf: *mut u32,
    stride: u32,
    buf_h: u32,
    tile_y_start: i32,
    draw_y: i32,
    clip: (i32, i32, i32, i32),
) {
    if cmd.rotations.is_empty() {
        rasterize_draw_cmd_basic(images, cmd, buf, stride, buf_h, draw_y, clip);
        return;
    }

    let sw = cmd.src_w.max(0);
    let sh = cmd.src_h.max(0);
    if sw == 0 || sh == 0 {
        return;
    }

    let mut scratch = vec![0u32; (sw as usize).saturating_mul(sh as usize)];
    let scratch_cmd = DrawCmd {
        x: 0,
        y: 0,
        w: sw,
        h: sh,
        src_x: 0,
        src_y: 0,
        src_w: sw,
        src_h: sh,
        kind: match &cmd.kind {
            DrawKind::Rect { color } => DrawKind::Rect { color: *color },
            DrawKind::RoundedRect { color, radii } => DrawKind::RoundedRect {
                color: *color,
                radii: *radii,
            },
            DrawKind::RoundedBorder {
                color,
                radii,
                widths,
            } => DrawKind::RoundedBorder {
                color: *color,
                radii: *radii,
                widths: *widths,
            },
            DrawKind::Triangle { color, p0, p1, p2 } => DrawKind::Triangle {
                color: *color,
                p0: *p0,
                p1: *p1,
                p2: *p2,
            },
            DrawKind::DashedLine {
                color,
                dash_len,
                gap_len,
                vertical,
            } => DrawKind::DashedLine {
                color: *color,
                dash_len: *dash_len,
                gap_len: *gap_len,
                vertical: *vertical,
            },
            DrawKind::Text {
                color,
                font_id,
                font_size,
                scale_x_percent,
                text,
            } => DrawKind::Text {
                color: *color,
                font_id: *font_id,
                font_size: *font_size,
                scale_x_percent: *scale_x_percent,
                text: text.clone(),
            },
            DrawKind::Image {
                src,
                object_fit,
                object_position_x,
                object_position_x_is_percent,
                object_position_y,
                object_position_y_is_percent,
            } => DrawKind::Image {
                src: src.clone(),
                object_fit: *object_fit,
                object_position_x: *object_position_x,
                object_position_x_is_percent: *object_position_x_is_percent,
                object_position_y: *object_position_y,
                object_position_y_is_percent: *object_position_y_is_percent,
            },
        },
        clip: None,
        masks: Vec::new(),
        rotations: Vec::new(),
    };
    rasterize_draw_cmd_basic(
        images,
        &scratch_cmd,
        scratch.as_mut_ptr(),
        sw as u32,
        sh as u32,
        0,
        (0, 0, sw, sh),
    );

    let (cx, cy, cw, ch) = clip;
    if cw <= 0 || ch <= 0 || buf.is_null() {
        return;
    }

    unsafe {
        for row in 0..ch {
            let dst_row = cy + row;
            if dst_row < 0 || dst_row >= buf_h as i32 {
                continue;
            }
            let doc_y = tile_y_start + dst_row;
            let dst_offset = dst_row as usize * stride as usize;
            for col in 0..cw {
                let doc_x = cx + col;
                let mut sx = doc_x as f32 + 0.5;
                let mut sy = doc_y as f32 + 0.5;
                for rot in cmd.rotations.iter().rev() {
                    let rad = rot.angle_deg100 as f32 / 100.0 * core::f32::consts::PI / 180.0;
                    let sin = sin_approx(rad);
                    let cos = cos_approx(rad);
                    let dx = sx - rot.origin_x as f32;
                    let dy = sy - rot.origin_y as f32;
                    sx = rot.origin_x as f32 + dx * cos + dy * sin;
                    sy = rot.origin_y as f32 - dx * sin + dy * cos;
                }

                let src_x = floor_f32(sx) - cmd.src_x;
                let src_y = floor_f32(sy) - cmd.src_y;
                if src_x < 0 || src_y < 0 || src_x >= sw || src_y >= sh {
                    continue;
                }
                let src = scratch[src_y as usize * sw as usize + src_x as usize];
                if (src >> 24) == 0 {
                    continue;
                }
                let idx = dst_offset + doc_x as usize;
                let prev = *buf.add(idx);
                *buf.add(idx) = src_over(src, prev);
            }
        }
    }
}

pub(super) fn rasterize_masked_cmd(
    images: &ImageCache,
    cmd: &DrawCmd,
    buf: *mut u32,
    stride: u32,
    buf_h: u32,
    draw_y: i32,
    tile_y_start: i32,
    clip: (i32, i32, i32, i32),
) {
    let (cx, cy, cw, ch) = clip;
    if cw <= 0 || ch <= 0 {
        return;
    }
    let mut scratch = vec![0u32; (cw as usize).saturating_mul(ch as usize)];
    rasterize_draw_cmd(
        images,
        cmd,
        scratch.as_mut_ptr(),
        cw as u32,
        ch as u32,
        tile_y_start + cy,
        draw_y - cy,
        (cmd.x - cx, draw_y - cy, cmd.w, cmd.h),
    );
    composite_masked_scratch(
        images,
        buf,
        stride,
        buf_h,
        cx,
        cy,
        cw,
        ch,
        tile_y_start,
        &scratch,
        &cmd.masks,
    );
}

fn composite_masked_scratch(
    images: &ImageCache,
    dst: *mut u32,
    stride: u32,
    buf_h: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    tile_y_start: i32,
    scratch: &[u32],
    masks: &[MaskLayer],
) {
    if dst.is_null() || w <= 0 || h <= 0 {
        return;
    }
    unsafe {
        for row in 0..h {
            let dst_row = y + row;
            if dst_row < 0 || dst_row >= buf_h as i32 {
                continue;
            }
            let base = row as usize * w as usize;
            let dst_offset = dst_row as usize * stride as usize;
            for col in 0..w {
                let src = scratch[base + col as usize];
                let src_a = (src >> 24) & 0xFF;
                if src_a == 0 {
                    continue;
                }
                let doc_x = x + col;
                let doc_y = tile_y_start + dst_row;
                let mask_a = combined_mask_alpha(images, doc_x, doc_y, masks);
                if mask_a == 0 {
                    continue;
                }
                let masked = apply_alpha_to_argb(src, mask_a);
                if (masked >> 24) & 0xFF == 0 {
                    continue;
                }
                let idx = dst_offset + (x + col) as usize;
                let prev = *dst.add(idx);
                *dst.add(idx) = src_over(masked, prev);
            }
        }
    }
}

fn combined_mask_alpha(images: &ImageCache, doc_x: i32, doc_y: i32, masks: &[MaskLayer]) -> u32 {
    let mut alpha = 255u32;
    for mask in masks {
        let (clip_x, clip_y, clip_w, clip_h) = mask.clip_rect;
        if doc_x < clip_x || doc_y < clip_y || doc_x >= clip_x + clip_w || doc_y >= clip_y + clip_h
        {
            return 0;
        }
        alpha = alpha * sample_mask_alpha(images, mask, doc_x, doc_y) / 255;
        if alpha == 0 {
            return 0;
        }
    }
    alpha
}

fn sample_mask_alpha(images: &ImageCache, mask: &MaskLayer, doc_x: i32, doc_y: i32) -> u32 {
    match &mask.image {
        BackgroundImageVal::None => 255,
        BackgroundImageVal::Url(src) => {
            let Some(entry) = images.get_ref(src) else {
                return 0;
            };
            if !entry.has_pixels() {
                return 0;
            }
            let (mx, my, mw, mh) = resolve_mask_image_rect(mask, Some((entry.width, entry.height)));
            if mw <= 0 || mh <= 0 {
                return 0;
            }
            let rel_x = doc_x - mx;
            let rel_y = doc_y - my;
            if !mask_repeats_at(mask.repeat, rel_x, rel_y, mw, mh) {
                return 0;
            }
            let tiled_x = wrap_repeat(rel_x, mw);
            let tiled_y = wrap_repeat(rel_y, mh);
            let sx = ((tiled_x as i64) * entry.width as i64 / mw as i64) as usize;
            let sy = ((tiled_y as i64) * entry.height as i64 / mh as i64) as usize;
            if sx >= entry.width as usize || sy >= entry.height as usize {
                return 0;
            }
            let idx = sy * entry.width as usize + sx;
            if idx >= entry.pixels.len() {
                return 0;
            }
            (entry.pixels[idx] >> 24) & 0xFF
        }
        BackgroundImageVal::LinearGradient { angle_deg, stops } => {
            let (mx, my, mw, mh) = resolve_mask_image_rect(mask, None);
            if mw <= 0 || mh <= 0 {
                return 0;
            }
            let rel_x = doc_x - mx;
            let rel_y = doc_y - my;
            if !mask_repeats_at(mask.repeat, rel_x, rel_y, mw, mh) {
                return 0;
            }
            let tiled_x = wrap_repeat(rel_x, mw);
            let tiled_y = wrap_repeat(rel_y, mh);
            let t = gradient_position(angle_deg, tiled_x, tiled_y, mw, mh);
            (interpolate_gradient_color(stops, t) >> 24) & 0xFF
        }
    }
}

fn resolve_mask_image_rect(
    mask: &MaskLayer,
    intrinsic: Option<(u32, u32)>,
) -> (i32, i32, i32, i32) {
    let (ox, oy, ow, oh) = mask.origin_rect;
    let (mw, mh) = match mask.size {
        BackgroundSizeVal::Auto => intrinsic
            .map(|(w, h)| (w as i32, h as i32))
            .unwrap_or((ow.max(1), oh.max(1))),
        BackgroundSizeVal::Cover => intrinsic
            .map(|(w, h)| fit_cover_size(ow.max(1), oh.max(1), w, h))
            .unwrap_or((ow.max(1), oh.max(1))),
        BackgroundSizeVal::Contain => intrinsic
            .map(|(w, h)| fit_contain_size(ow.max(1), oh.max(1), w, h))
            .unwrap_or((ow.max(1), oh.max(1))),
        BackgroundSizeVal::Explicit(w, h) => {
            let (intr_w, intr_h) = intrinsic
                .map(|(iw, ih)| (iw as i32, ih as i32))
                .unwrap_or((ow.max(1), oh.max(1)));
            let rw = if w < 0 {
                if h >= 0 && intr_h > 0 {
                    ((h as i64 * intr_w as i64) / intr_h as i64) as i32
                } else {
                    intr_w
                }
            } else {
                w
            };
            let rh = if h < 0 {
                if w >= 0 && intr_w > 0 {
                    ((w as i64 * intr_h as i64) / intr_w as i64) as i32
                } else {
                    intr_h
                }
            } else {
                h
            };
            (rw.max(1), rh.max(1))
        }
    };
    let free_x = (ow - mw).max(0);
    let free_y = (oh - mh).max(0);
    let px =
        ox + resolve_object_position_offset(free_x, mask.position_x, mask.position_x_is_percent);
    let py =
        oy + resolve_object_position_offset(free_y, mask.position_y, mask.position_y_is_percent);
    (px, py, mw, mh)
}

fn mask_repeats_at(repeat: BackgroundRepeatVal, rel_x: i32, rel_y: i32, w: i32, h: i32) -> bool {
    let repeat_x = matches!(
        repeat,
        BackgroundRepeatVal::Repeat | BackgroundRepeatVal::RepeatX
    );
    let repeat_y = matches!(
        repeat,
        BackgroundRepeatVal::Repeat | BackgroundRepeatVal::RepeatY
    );
    (repeat_x || (rel_x >= 0 && rel_x < w)) && (repeat_y || (rel_y >= 0 && rel_y < h))
}

fn wrap_repeat(pos: i32, len: i32) -> i32 {
    if len <= 0 {
        return 0;
    }
    let mut out = pos % len;
    if out < 0 {
        out += len;
    }
    out
}

fn gradient_position(angle_deg: &i32, x: i32, y: i32, w: i32, h: i32) -> i32 {
    let angle = (*angle_deg as f32).to_radians();
    let dir_x = cos_approx(angle);
    let dir_y = -sin_approx(angle);
    let nx = if w > 1 {
        x as f32 / (w - 1) as f32
    } else {
        0.0
    };
    let ny = if h > 1 {
        y as f32 / (h - 1) as f32
    } else {
        0.0
    };
    let proj = (nx - 0.5) * dir_x + (ny - 0.5) * dir_y;
    (((proj + 0.70710677) / 1.41421354) * 10000.0) as i32
}

fn apply_alpha_to_argb(color: u32, alpha: u32) -> u32 {
    let a = ((color >> 24) & 0xFF) * alpha / 255;
    (color & 0x00FFFFFF) | (a << 24)
}

fn src_over(src: u32, dst: u32) -> u32 {
    let sa = (src >> 24) & 0xFF;
    if sa == 0 {
        return dst;
    }
    if sa >= 255 {
        return src;
    }
    let da = (dst >> 24) & 0xFF;
    let inv_sa = 255 - sa;
    let out_a = sa + (da * inv_sa) / 255;
    let sr = (src >> 16) & 0xFF;
    let sg = (src >> 8) & 0xFF;
    let sb = src & 0xFF;
    let dr = (dst >> 16) & 0xFF;
    let dg = (dst >> 8) & 0xFF;
    let db = dst & 0xFF;
    let out_r = sr + (dr * inv_sa) / 255;
    let out_g = sg + (dg * inv_sa) / 255;
    let out_b = sb + (db * inv_sa) / 255;
    (out_a << 24) | (out_r.min(255) << 16) | (out_g.min(255) << 8) | out_b.min(255)
}

#[inline]
fn floor_f32(v: f32) -> i32 {
    let i = v as i32;
    if v < i as f32 {
        i - 1
    } else {
        i
    }
}

#[inline]
fn ceil_f32(v: f32) -> i32 {
    let i = v as i32;
    if v > i as f32 {
        i + 1
    } else {
        i
    }
}

// ═══════════════════════════════════════════════════════════════════════════

/// Fill a rectangle directly in the ARGB pixel buffer with clipping.
/// Parse a CSS color hex value (#RGB, #RRGGBB) into ARGB u32.
/// Parse "YYYY-MM-DD" into (year, month, day).
pub(super) fn parse_date_value(s: &str) -> Option<(u32, u32, u32)> {
    let b = s.as_bytes();
    // Expect at least YYYY-MM-DD (10 chars).
    if b.len() < 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let year = parse_uint(&b[0..4])?;
    let month = parse_uint(&b[5..7])?;
    let day = parse_uint(&b[8..10])?;
    Some((year, month, day))
}

/// Parse "HH:MM" into (hour, minute).
pub(super) fn parse_time_value(s: &str) -> Option<(u32, u32)> {
    let b = s.as_bytes();
    if b.len() < 5 || b[2] != b':' {
        return None;
    }
    let hour = parse_uint(&b[0..2])?;
    let minute = parse_uint(&b[3..5])?;
    Some((hour, minute))
}

fn parse_uint(b: &[u8]) -> Option<u32> {
    let mut n: u32 = 0;
    for &c in b {
        if c < b'0' || c > b'9' {
            return None;
        }
        n = n * 10 + (c - b'0') as u32;
    }
    Some(n)
}

pub fn parse_color_value(s: &str) -> u32 {
    let s = s.trim();
    let hex = if s.starts_with('#') { &s[1..] } else { s };
    let (r, g, b) = match hex.len() {
        3 => {
            let r = hex_nibble(hex.as_bytes()[0]) * 17;
            let g = hex_nibble(hex.as_bytes()[1]) * 17;
            let b = hex_nibble(hex.as_bytes()[2]) * 17;
            (r, g, b)
        }
        6 => {
            let r = hex_nibble(hex.as_bytes()[0]) * 16 + hex_nibble(hex.as_bytes()[1]);
            let g = hex_nibble(hex.as_bytes()[2]) * 16 + hex_nibble(hex.as_bytes()[3]);
            let b = hex_nibble(hex.as_bytes()[4]) * 16 + hex_nibble(hex.as_bytes()[5]);
            (r, g, b)
        }
        _ => (0, 0, 0),
    };
    0xFF000000 | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

fn hex_nibble(c: u8) -> u32 {
    match c {
        b'0'..=b'9' => (c - b'0') as u32,
        b'a'..=b'f' => (c - b'a' + 10) as u32,
        b'A'..=b'F' => (c - b'A' + 10) as u32,
        _ => 0,
    }
}

pub(super) fn fill_rect_buf(
    buf: *mut u32,
    stride: u32,
    buf_h: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    color: u32,
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

    let cw = (x1 - x0) as usize;
    let alpha = (color >> 24) & 0xFF;
    unsafe {
        for row in y0..y1 {
            let offset = row as usize * stride as usize + x0 as usize;
            let ptr = buf.add(offset);
            if alpha >= 255 {
                // Fast path: 4-pixel unrolled opaque fill.
                let mut i = 0usize;
                let cw4 = cw & !3;
                while i < cw4 {
                    *ptr.add(i) = color;
                    *ptr.add(i + 1) = color;
                    *ptr.add(i + 2) = color;
                    *ptr.add(i + 3) = color;
                    i += 4;
                }
                while i < cw {
                    *ptr.add(i) = color;
                    i += 1;
                }
            } else if alpha > 0 {
                let inv_a = 255 - alpha;
                let sr = (color >> 16) & 0xFF;
                let sg = (color >> 8) & 0xFF;
                let sb = color & 0xFF;
                for i in 0..cw {
                    let dst = *ptr.add(i);
                    let dr = (dst >> 16) & 0xFF;
                    let dg = (dst >> 8) & 0xFF;
                    let db = dst & 0xFF;
                    let r = (sr * alpha + dr * inv_a) / 255;
                    let g = (sg * alpha + dg * inv_a) / 255;
                    let b = (sb * alpha + db * inv_a) / 255;
                    *ptr.add(i) = 0xFF000000 | (r << 16) | (g << 8) | b;
                }
            }
        }
    }
}

fn fill_triangle_buf(
    buf: *mut u32,
    stride: u32,
    buf_h: u32,
    x: i32,
    y: i32,
    color: u32,
    p0: (i32, i32),
    p1: (i32, i32),
    p2: (i32, i32),
) {
    if buf.is_null() {
        return;
    }
    let min_x = x + p0.0.min(p1.0).min(p2.0);
    let max_x = x + p0.0.max(p1.0).max(p2.0);
    let min_y = y + p0.1.min(p1.1).min(p2.1);
    let max_y = y + p0.1.max(p1.1).max(p2.1);
    if max_x <= min_x || max_y <= min_y {
        return;
    }

    let s = stride as i32;
    let bh = buf_h as i32;
    let x0 = min_x.max(0);
    let y0 = min_y.max(0);
    let x1 = max_x.min(s - 1);
    let y1 = max_y.min(bh - 1);
    if x0 > x1 || y0 > y1 {
        return;
    }

    let a = (x + p0.0, y + p0.1);
    let b = (x + p1.0, y + p1.1);
    let c = (x + p2.0, y + p2.1);
    let area = edge_fn(a, b, c);
    if area == 0 {
        return;
    }

    unsafe {
        for py in y0..=y1 {
            let row = py as usize * stride as usize;
            for px in x0..=x1 {
                let p = (px, py);
                let w0 = edge_fn(b, c, p);
                let w1 = edge_fn(c, a, p);
                let w2 = edge_fn(a, b, p);
                let inside = if area > 0 {
                    w0 >= 0 && w1 >= 0 && w2 >= 0
                } else {
                    w0 <= 0 && w1 <= 0 && w2 <= 0
                };
                if !inside {
                    continue;
                }
                let idx = row + px as usize;
                let alpha = (color >> 24) & 0xFF;
                if alpha >= 255 {
                    *buf.add(idx) = color;
                } else if alpha > 0 {
                    let dst = *buf.add(idx);
                    *buf.add(idx) = src_over(color, dst);
                }
            }
        }
    }
}

#[inline]
fn edge_fn(a: (i32, i32), b: (i32, i32), p: (i32, i32)) -> i32 {
    (p.0 - a.0) * (b.1 - a.1) - (p.1 - a.1) * (b.0 - a.0)
}

/// Blit image pixels into the buffer with scaling and clipping.
fn blit_image_buf(
    buf: *mut u32,
    stride: u32,
    buf_h: u32,
    dx: i32,
    dy: i32,
    dw: i32,
    dh: i32,
    src: &[u32],
    src_w: u32,
    src_h: u32,
) {
    if dw <= 0 || dh <= 0 || src.is_empty() || src_w == 0 || src_h == 0 || buf.is_null() {
        return;
    }
    let s = stride as i32;
    let bh = buf_h as i32;

    let x0 = dx.max(0);
    let y0 = dy.max(0);
    let x1 = (dx + dw).min(s);
    let y1 = (dy + dh).min(bh);
    if x0 >= x1 || y0 >= y1 {
        return;
    }

    unsafe {
        for row in y0..y1 {
            let sy = ((row - dy) as u64 * src_h as u64 / dh as u64) as usize;
            if sy >= src_h as usize {
                continue;
            }
            let dst_offset = row as usize * stride as usize;
            let src_row = sy * src_w as usize;
            for col in x0..x1 {
                let sx = ((col - dx) as u64 * src_w as u64 / dw as u64) as usize;
                if sx >= src_w as usize {
                    continue;
                }
                let src_idx = src_row + sx;
                if src_idx >= src.len() {
                    continue;
                }
                let pixel = src[src_idx];
                let alpha = (pixel >> 24) & 0xFF;
                let dst_idx = dst_offset + col as usize;
                if alpha >= 255 {
                    *buf.add(dst_idx) = pixel;
                } else if alpha > 0 {
                    let dst = *buf.add(dst_idx);
                    let inv_a = 255 - alpha;
                    let r = (((pixel >> 16) & 0xFF) * alpha + ((dst >> 16) & 0xFF) * inv_a) / 255;
                    let g = (((pixel >> 8) & 0xFF) * alpha + ((dst >> 8) & 0xFF) * inv_a) / 255;
                    let b = ((pixel & 0xFF) * alpha + (dst & 0xFF) * inv_a) / 255;
                    *buf.add(dst_idx) = 0xFF000000 | (r << 16) | (g << 8) | b;
                }
            }
        }
    }
}

pub(super) fn blit_image_buf_clipped(
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
) {
    if dw <= 0 || dh <= 0 || src.is_empty() || src_w == 0 || src_h == 0 || buf.is_null() {
        return;
    }
    let s = stride as i32;
    let bh = buf_h as i32;
    let (clip_x, clip_y, clip_w, clip_h) = clip;
    let x0 = dx.max(clip_x).max(0);
    let y0 = dy.max(clip_y).max(0);
    let x1 = (dx + dw).min(clip_x + clip_w).min(s);
    let y1 = (dy + dh).min(clip_y + clip_h).min(bh);
    if x0 >= x1 || y0 >= y1 {
        return;
    }

    unsafe {
        for row in y0..y1 {
            let sy = ((row - dy) as u64 * src_h as u64 / dh as u64) as usize;
            if sy >= src_h as usize {
                continue;
            }
            let dst_offset = row as usize * stride as usize;
            let src_row = sy * src_w as usize;
            for col in x0..x1 {
                let sx = ((col - dx) as u64 * src_w as u64 / dw as u64) as usize;
                if sx >= src_w as usize {
                    continue;
                }
                let src_idx = src_row + sx;
                if src_idx >= src.len() {
                    continue;
                }
                let pixel = src[src_idx];
                let alpha = (pixel >> 24) & 0xFF;
                let dst_idx = dst_offset + col as usize;
                if alpha >= 255 {
                    *buf.add(dst_idx) = pixel;
                } else if alpha > 0 {
                    let dst = *buf.add(dst_idx);
                    let inv_a = 255 - alpha;
                    let r = (((pixel >> 16) & 0xFF) * alpha + ((dst >> 16) & 0xFF) * inv_a) / 255;
                    let g = (((pixel >> 8) & 0xFF) * alpha + ((dst >> 8) & 0xFF) * inv_a) / 255;
                    let b = ((pixel & 0xFF) * alpha + (dst & 0xFF) * inv_a) / 255;
                    *buf.add(dst_idx) = 0xFF000000 | (r << 16) | (g << 8) | b;
                }
            }
        }
    }
}
