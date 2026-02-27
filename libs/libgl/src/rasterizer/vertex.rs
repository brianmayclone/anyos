//! Vertex attribute fetching from VBOs.
//!
//! Reads vertex data from bound buffer objects according to the
//! vertex attribute pointer configuration.

use alloc::vec::Vec;
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

/// Fetch all attribute values for a single vertex.
///
/// Returns a vector of `[f32; 4]`, one per attribute.
pub fn fetch_attributes(
    ctx: &GlContext,
    attrib_info: &[(i32, i32, GLenum, i32, usize, u32)],
    vertex_index: u32,
) -> Vec<[f32; 4]> {
    let mut result = Vec::with_capacity(attrib_info.len());

    for &(_loc, size, typ, stride, offset, buffer_id) in attrib_info {
        if size == 0 || buffer_id == 0 {
            result.push([0.0, 0.0, 0.0, 1.0]);
            continue;
        }

        let buf = match ctx.buffers.get(buffer_id) {
            Some(b) => &b.data,
            None => {
                result.push([0.0, 0.0, 0.0, 1.0]);
                continue;
            }
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

        let mut val = [0.0f32, 0.0, 0.0, 1.0]; // w defaults to 1.0
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
        result.push(val);
    }

    result
}
