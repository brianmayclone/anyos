//! Draw command dispatch.
//!
//! Implements `glDrawArrays` and `glDrawElements` by delegating to the
//! software rasterizer pipeline.

use crate::state::GlContext;
use crate::types::*;
use crate::rasterizer;

/// Execute glDrawArrays.
pub fn draw_arrays(ctx: &mut GlContext, mode: GLenum, first: GLint, count: GLsizei) {
    rasterizer::draw(ctx, mode, first, count);
}

/// Execute glDrawElements.
pub fn draw_elements(
    ctx: &mut GlContext,
    mode: GLenum,
    count: GLsizei,
    type_: GLenum,
    offset: usize,
) {
    rasterizer::draw_elements(ctx, mode, count, type_, offset);
}
