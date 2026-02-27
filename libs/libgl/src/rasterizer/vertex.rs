//! Vertex attribute fetching from VBOs.
//!
//! Reads vertex data from bound buffer objects according to the
//! vertex attribute pointer configuration.

use crate::state::GlContext;
use crate::types::*;

/// Fetch a single attribute value for a given vertex.
///
/// Used by the HW draw path to pack vertex data into an interleaved buffer.
pub fn fetch_single_attribute(
    ctx: &GlContext,
    size: i32,
    typ: GLenum,
    stride: i32,
    offset: usize,
    buffer_id: u32,
    vertex_index: u32,
) -> [f32; 4] {
    if size == 0 || buffer_id == 0 {
        return [0.0, 0.0, 0.0, 1.0];
    }

    let buf = match ctx.buffers.get(buffer_id) {
        Some(b) => &b.data,
        None => return [0.0, 0.0, 0.0, 1.0],
    };

    let elem_size = match typ {
        GL_FLOAT => 4,
        GL_SHORT | GL_UNSIGNED_SHORT => 2,
        GL_BYTE | GL_UNSIGNED_BYTE => 1,
        GL_INT | GL_UNSIGNED_INT => 4,
        _ => 4,
    };
    let actual_stride = if stride == 0 { size * elem_size } else { stride };
    let base = offset + (vertex_index as i32 * actual_stride) as usize;

    let mut val = [0.0f32, 0.0, 0.0, 1.0];
    for c in 0..(size as usize).min(4) {
        let off = base + c * elem_size as usize;
        val[c] = match typ {
            GL_FLOAT => {
                if off + 3 < buf.len() {
                    f32::from_le_bytes([buf[off], buf[off+1], buf[off+2], buf[off+3]])
                } else { 0.0 }
            }
            GL_UNSIGNED_BYTE => {
                if off < buf.len() { buf[off] as f32 / 255.0 } else { 0.0 }
            }
            GL_BYTE => {
                if off < buf.len() { (buf[off] as i8) as f32 / 127.0 } else { 0.0 }
            }
            GL_UNSIGNED_SHORT => {
                if off + 1 < buf.len() {
                    let v = u16::from_le_bytes([buf[off], buf[off+1]]);
                    v as f32 / 65535.0
                } else { 0.0 }
            }
            GL_SHORT => {
                if off + 1 < buf.len() {
                    let v = i16::from_le_bytes([buf[off], buf[off+1]]);
                    v as f32 / 32767.0
                } else { 0.0 }
            }
            _ => 0.0,
        };
    }
    val
}

/// Fetch all attributes for a single vertex into a caller-provided buffer.
///
/// Writes one `[f32; 4]` per attribute into `out[0..attrib_info.len()]`.
/// **Zero heap allocation** — writes directly into the caller's stack buffer.
#[inline]
pub fn fetch_attributes_into(
    ctx: &GlContext,
    attrib_info: &[(i32, i32, GLenum, i32, usize, u32)],
    vertex_index: u32,
    out: &mut [[f32; 4]],
) {
    for (i, &(_loc, size, typ, stride, offset, buffer_id)) in attrib_info.iter().enumerate() {
        if i >= out.len() { break; }

        if size == 0 || buffer_id == 0 {
            out[i] = [0.0, 0.0, 0.0, 1.0];
            continue;
        }

        let buf = match ctx.buffers.get(buffer_id) {
            Some(b) => &b.data,
            None => {
                out[i] = [0.0, 0.0, 0.0, 1.0];
                continue;
            }
        };

        let elem_size: i32 = match typ {
            GL_FLOAT => 4,
            GL_SHORT | GL_UNSIGNED_SHORT => 2,
            GL_BYTE | GL_UNSIGNED_BYTE => 1,
            GL_INT | GL_UNSIGNED_INT => 4,
            _ => 4,
        };
        let actual_stride = if stride == 0 { size * elem_size } else { stride };
        let base = offset + (vertex_index as i32 * actual_stride) as usize;

        let mut val = [0.0f32, 0.0, 0.0, 1.0]; // w defaults to 1.0
        let n = (size as usize).min(4);

        // Fast path for GL_FLOAT (most common)
        if typ == GL_FLOAT {
            for c in 0..n {
                let off = base + c * 4;
                if off + 3 < buf.len() {
                    val[c] = f32::from_le_bytes([buf[off], buf[off+1], buf[off+2], buf[off+3]]);
                }
            }
        } else {
            for c in 0..n {
                let off = base + c * elem_size as usize;
                val[c] = match typ {
                    GL_UNSIGNED_BYTE => {
                        if off < buf.len() { buf[off] as f32 / 255.0 } else { 0.0 }
                    }
                    GL_BYTE => {
                        if off < buf.len() { (buf[off] as i8) as f32 / 127.0 } else { 0.0 }
                    }
                    GL_UNSIGNED_SHORT => {
                        if off + 1 < buf.len() {
                            let v = u16::from_le_bytes([buf[off], buf[off+1]]);
                            v as f32 / 65535.0
                        } else { 0.0 }
                    }
                    GL_SHORT => {
                        if off + 1 < buf.len() {
                            let v = i16::from_le_bytes([buf[off], buf[off+1]]);
                            v as f32 / 32767.0
                        } else { 0.0 }
                    }
                    _ => 0.0,
                };
            }
        }
        out[i] = val;
    }
}

/// Fetch all attribute values for a single vertex (returns Vec, legacy API).
///
/// Kept for backward compatibility with the HW draw path.
pub fn fetch_attributes(
    ctx: &GlContext,
    attrib_info: &[(i32, i32, GLenum, i32, usize, u32)],
    vertex_index: u32,
) -> alloc::vec::Vec<[f32; 4]> {
    let mut out = alloc::vec::Vec::with_capacity(attrib_info.len());
    out.resize(attrib_info.len(), [0.0, 0.0, 0.0, 1.0]);
    fetch_attributes_into(ctx, attrib_info, vertex_index, &mut out);
    out
}
