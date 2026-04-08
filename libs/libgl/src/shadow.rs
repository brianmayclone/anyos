//! Shadow mapping support.
//!
//! Keeps shadow-map resource management and light-space matrix fitting out of
//! `lib.rs` so the rendering entry points stay focused on ABI concerns.

use crate::ctx;
use crate::drv_loader;
use crate::rasterizer;
use crate::types::*;

const DEFAULT_UP: [f32; 3] = [0.0, 1.0, 0.0];
const FALLBACK_UP: [f32; 3] = [0.0, 0.0, 1.0];
const SHADOW_BLUR_THRESHOLD: f32 = 0.0035;

#[inline(always)]
fn dot(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[inline(always)]
fn cross(a: &[f32; 3], b: &[f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[inline(always)]
fn sub(a: &[f32; 3], b: &[f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

#[inline(always)]
fn length(v: &[f32; 3]) -> f32 {
    rasterizer::math::sqrt(dot(v, v))
}

#[inline(always)]
fn normalize(v: &[f32; 3], fallback: [f32; 3]) -> [f32; 3] {
    let len = length(v);
    if len <= 1e-5 {
        fallback
    } else {
        [v[0] / len, v[1] / len, v[2] / len]
    }
}

#[inline(always)]
fn transform_point(m: &[f32; 16], p: &[f32; 3]) -> [f32; 3] {
    [
        m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12],
        m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13],
        m[2] * p[0] + m[6] * p[1] + m[10] * p[2] + m[14],
    ]
}

/// Build an orthographic projection matrix for directional-light shadowing.
pub fn ortho_matrix(l: f32, r: f32, b: f32, t: f32, n: f32, f: f32) -> [f32; 16] {
    let mut m = [0.0f32; 16];
    m[0] = 2.0 / (r - l);
    m[5] = 2.0 / (t - b);
    m[10] = -2.0 / (f - n);
    m[12] = -(r + l) / (r - l);
    m[13] = -(t + b) / (t - b);
    m[14] = -(f + n) / (f - n);
    m[15] = 1.0;
    m
}

/// Build a column-major look-at matrix.
pub fn look_at_matrix(eye: &[f32; 3], target: &[f32; 3], up_hint: &[f32; 3]) -> [f32; 16] {
    let forward = normalize(&sub(target, eye), [0.0, -1.0, 0.0]);
    let hint = if dot(&forward, up_hint).abs() > 0.98 {
        FALLBACK_UP
    } else {
        *up_hint
    };
    let right = normalize(&cross(&forward, &hint), [1.0, 0.0, 0.0]);
    let up = cross(&right, &forward);

    [
        right[0], up[0], -forward[0], 0.0,
        right[1], up[1], -forward[1], 0.0,
        right[2], up[2], -forward[2], 0.0,
        -dot(&right, eye),
        -dot(&up, eye),
        dot(&forward, eye),
        1.0,
    ]
}

/// Multiply two column-major 4x4 matrices.
pub fn mat4_mul(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut out = [0.0f32; 16];
    for col in 0..4 {
        for row in 0..4 {
            let mut s = 0.0f32;
            for k in 0..4 {
                s += a[k * 4 + row] * b[col * 4 + k];
            }
            out[col * 4 + row] = s;
        }
    }
    out
}

fn fit_directional_light_matrices(
    eye: &[f32; 3],
    target: &[f32; 3],
    radius: f32,
) -> ([f32; 16], [f32; 16], [f32; 16]) {
    let safe_radius = radius.max(1.0);
    let view = look_at_matrix(eye, target, &DEFAULT_UP);

    let mut min_x = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    let mut min_depth = f32::INFINITY;
    let mut max_depth = f32::NEG_INFINITY;

    for dx in [-safe_radius, safe_radius] {
        for dy in [-safe_radius, safe_radius] {
            for dz in [-safe_radius, safe_radius] {
                let corner = [target[0] + dx, target[1] + dy, target[2] + dz];
                let light_space = transform_point(&view, &corner);
                min_x = min_x.min(light_space[0]);
                max_x = max_x.max(light_space[0]);
                min_y = min_y.min(light_space[1]);
                max_y = max_y.max(light_space[1]);
                let depth = -light_space[2];
                min_depth = min_depth.min(depth);
                max_depth = max_depth.max(depth);
            }
        }
    }

    let xy_pad = (safe_radius * 0.15).max(0.5);
    let depth_pad = (safe_radius * 0.35).max(1.0);
    let left = min_x - xy_pad;
    let right = max_x + xy_pad;
    let bottom = min_y - xy_pad;
    let top = max_y + xy_pad;
    let near = (min_depth - depth_pad).max(0.1);
    let far = (max_depth + depth_pad).max(near + 1.0);

    let proj = ortho_matrix(left, right, bottom, top, near, far);
    let mvp = mat4_mul(&proj, &view);
    (view, proj, mvp)
}

fn blur_weighted_depth(
    src: &[f32],
    dst: &mut [f32],
    w: usize,
    h: usize,
    horizontal: bool,
) {
    const OFFSETS: [isize; 5] = [-2, -1, 0, 1, 2];
    const WEIGHTS: [f32; 5] = [1.0, 4.0, 6.0, 4.0, 1.0];

    for y in 0..h {
        for x in 0..w {
            let idx = y * w + x;
            let center = src[idx];
            let mut sum = 0.0f32;
            let mut total = 0.0f32;

            for i in 0..OFFSETS.len() {
                let off = OFFSETS[i];
                let sx = if horizontal {
                    (x as isize + off).clamp(0, w as isize - 1) as usize
                } else {
                    x
                };
                let sy = if horizontal {
                    y
                } else {
                    (y as isize + off).clamp(0, h as isize - 1) as usize
                };
                let sample = src[sy * w + sx];
                let diff = (sample - center).abs();
                if diff <= SHADOW_BLUR_THRESHOLD {
                    let weight = WEIGHTS[i];
                    sum += sample * weight;
                    total += weight;
                }
            }

            dst[idx] = if total > 0.0 { sum / total } else { center };
        }
    }
}

fn prefilter_shadow_map() {
    let c = ctx();
    if c.shadow_depth_tex_id == 0 {
        return;
    }

    let tex_id = c.shadow_depth_tex_id;
    let mut tmp = core::mem::take(&mut c.shadow_blur_tmp);
    if let Some(tex) = c.textures.get_mut(tex_id) {
        let w = tex.width as usize;
        let h = tex.height as usize;
        let count = w.saturating_mul(h);
        if count == 0 || tex.depth.len() != count {
            c.shadow_blur_tmp = tmp;
            return;
        }
        if tmp.len() != count {
            tmp.resize(count, 1.0);
        }
        blur_weighted_depth(&tex.depth, &mut tmp, w, h, true);
        blur_weighted_depth(&tmp, &mut tex.depth, w, h, false);
    }
    c.shadow_blur_tmp = tmp;
}

/// Ensure the internal shadow depth texture/FBO exist.
pub fn ensure_resources() {
    let c = ctx();
    if c.shadow_fbo_id != 0 {
        return;
    }

    let size = c.shadow_map_size;

    let mut tex = [0u32; 1];
    crate::glGenTextures(1, tex.as_mut_ptr());
    c.shadow_depth_tex_id = tex[0];
    crate::glBindTexture(GL_TEXTURE_2D, tex[0]);
    // Bilinear depth sampling softens the blocky PCF result noticeably in the
    // software path without needing a much larger shadow map.
    crate::glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as i32);
    crate::glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as i32);
    crate::glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as i32);
    crate::glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as i32);
    crate::glTexImage2D(
        GL_TEXTURE_2D,
        0,
        GL_DEPTH_COMPONENT as i32,
        size as i32,
        size as i32,
        0,
        GL_DEPTH_COMPONENT,
        GL_UNSIGNED_BYTE,
        core::ptr::null(),
    );

    let mut fbo = [0u32; 1];
    crate::glGenFramebuffers(1, fbo.as_mut_ptr());
    c.shadow_fbo_id = fbo[0];
    crate::glBindFramebuffer(GL_FRAMEBUFFER, fbo[0]);
    crate::glFramebufferTexture2D(
        GL_FRAMEBUFFER,
        GL_DEPTH_ATTACHMENT,
        GL_TEXTURE_2D,
        tex[0],
        0,
    );
    crate::glBindFramebuffer(GL_FRAMEBUFFER, 0);

    crate::serial_println!(
        "[libgl] shadow resources created: fbo={} tex={} size={}",
        c.shadow_fbo_id,
        c.shadow_depth_tex_id,
        size
    );
}

/// Start rendering into the internal shadow map.
pub fn begin_pass(
    lx: f32,
    ly: f32,
    lz: f32,
    tx: f32,
    ty: f32,
    tz: f32,
    radius: f32,
) -> u32 {
    ensure_resources();
    let c = ctx();
    if c.shadow_fbo_id == 0 {
        return 0;
    }

    let eye = [lx, ly, lz];
    let target = [tx, ty, tz];
    let (view, proj, mvp) = fit_directional_light_matrices(&eye, &target, radius);
    c.shadow_light_view = view;
    c.shadow_light_proj = proj;
    c.shadow_light_mvp = mvp;

    c.shadow_prev_viewport = [c.viewport_x, c.viewport_y, c.viewport_w, c.viewport_h];
    crate::glBindFramebuffer(GL_FRAMEBUFFER, c.shadow_fbo_id);
    crate::glViewport(0, 0, c.shadow_map_size as i32, c.shadow_map_size as i32);

    if unsafe { crate::USE_HW_BACKEND } {
        if let Some(drv) = drv_loader::drv() {
            if let Some(begin_fn) = drv.drv_shadow_begin {
                begin_fn(c.shadow_map_size, c.shadow_map_size);
            }
        }
    }

    c.shadow_pass_active = true;
    c.shadow_map_ready = false;
    1
}

/// Finish the shadow pass and restore the previous framebuffer/viewport.
pub fn end_pass() {
    let c = ctx();
    if !c.shadow_pass_active {
        return;
    }

    if unsafe { crate::USE_HW_BACKEND } {
        if let Some(drv) = drv_loader::drv() {
            if let Some(end_fn) = drv.drv_shadow_end {
                end_fn();
            }
        }
    }

    crate::glBindFramebuffer(GL_FRAMEBUFFER, 0);
    let prev = c.shadow_prev_viewport;
    crate::glViewport(prev[0], prev[1], prev[2], prev[3]);
    if !unsafe { crate::USE_HW_BACKEND } {
        prefilter_shadow_map();
    }
    c.shadow_pass_active = false;
    c.shadow_map_ready = true;
}
