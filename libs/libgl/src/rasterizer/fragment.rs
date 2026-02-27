//! Fragment processing: depth test and blending.

use crate::types::*;

/// Perform the depth test. Returns true if the fragment passes.
pub fn depth_test(frag_depth: f32, buffer_depth: f32, func: GLenum) -> bool {
    match func {
        GL_NEVER => false,
        GL_LESS => frag_depth < buffer_depth,
        GL_EQUAL => (frag_depth - buffer_depth).abs() < 1e-6,
        GL_LEQUAL => frag_depth <= buffer_depth,
        GL_GREATER => frag_depth > buffer_depth,
        GL_NOTEQUAL => (frag_depth - buffer_depth).abs() >= 1e-6,
        GL_GEQUAL => frag_depth >= buffer_depth,
        GL_ALWAYS => true,
        _ => true,
    }
}

/// Alpha blending: combine source (new fragment) with destination (framebuffer).
///
/// Both colors are ARGB u32. Returns blended ARGB u32.
pub fn blend(src: u32, dst: u32, src_factor: GLenum, dst_factor: GLenum) -> u32 {
    let sa = ((src >> 24) & 0xFF) as f32 / 255.0;
    let sr = ((src >> 16) & 0xFF) as f32 / 255.0;
    let sg = ((src >> 8) & 0xFF) as f32 / 255.0;
    let sb = (src & 0xFF) as f32 / 255.0;

    let da = ((dst >> 24) & 0xFF) as f32 / 255.0;
    let dr = ((dst >> 16) & 0xFF) as f32 / 255.0;
    let dg = ((dst >> 8) & 0xFF) as f32 / 255.0;
    let db = (dst & 0xFF) as f32 / 255.0;

    let sf = blend_factor(src_factor, sa, da);
    let df = blend_factor(dst_factor, sa, da);

    let out_r = clamp01(sr * sf + dr * df);
    let out_g = clamp01(sg * sf + dg * df);
    let out_b = clamp01(sb * sf + db * df);
    let out_a = clamp01(sa * sf + da * df);

    let ri = (out_r * 255.0) as u32;
    let gi = (out_g * 255.0) as u32;
    let bi = (out_b * 255.0) as u32;
    let ai = (out_a * 255.0) as u32;

    (ai << 24) | (ri << 16) | (gi << 8) | bi
}

/// Compute blend factor.
fn blend_factor(factor: GLenum, src_alpha: f32, dst_alpha: f32) -> f32 {
    match factor {
        GL_ZERO => 0.0,
        GL_ONE => 1.0,
        GL_SRC_ALPHA => src_alpha,
        GL_ONE_MINUS_SRC_ALPHA => 1.0 - src_alpha,
        GL_DST_ALPHA => dst_alpha,
        GL_ONE_MINUS_DST_ALPHA => 1.0 - dst_alpha,
        _ => 1.0,
    }
}

fn clamp01(x: f32) -> f32 {
    if x < 0.0 { 0.0 } else if x > 1.0 { 1.0 } else { x }
}
