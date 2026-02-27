//! Vertex/Index buffer objects (VBO/EBO).
//!
//! Manages named buffer objects: `glGenBuffers`, `glDeleteBuffers`, `glBindBuffer`,
//! `glBufferData`, `glBufferSubData`. Data is stored as raw byte `Vec<u8>`.

use alloc::vec::Vec;
use crate::types::*;

/// A GL buffer object holding raw byte data.
pub struct GlBuffer {
    pub data: Vec<u8>,
    pub usage: GLenum,
}

/// Storage for all buffer objects.
pub struct BufferStore {
    /// Slot 0 is unused (id 0 = unbound). Slots 1..N hold buffer objects.
    slots: Vec<Option<GlBuffer>>,
    next_id: u32,
}

impl BufferStore {
    /// Create an empty buffer store.
    pub fn new() -> Self {
        Self {
            slots: Vec::new(),
            next_id: 1,
        }
    }

    /// Generate `n` buffer names, writing them to `ids`.
    pub fn gen(&mut self, n: i32, ids: &mut [u32]) {
        for i in 0..(n as usize).min(ids.len()) {
            let id = self.next_id;
            self.next_id += 1;
            // Ensure slot exists
            while self.slots.len() <= id as usize {
                self.slots.push(None);
            }
            self.slots[id as usize] = Some(GlBuffer {
                data: Vec::new(),
                usage: GL_STATIC_DRAW,
            });
            ids[i] = id;
        }
    }

    /// Delete buffer objects by id.
    pub fn delete(&mut self, n: i32, ids: &[u32]) {
        for i in 0..(n as usize).min(ids.len()) {
            let id = ids[i] as usize;
            if id > 0 && id < self.slots.len() {
                self.slots[id] = None;
            }
        }
    }

    /// Get a reference to a buffer's data by id.
    pub fn get(&self, id: u32) -> Option<&GlBuffer> {
        if id == 0 { return None; }
        self.slots.get(id as usize).and_then(|s| s.as_ref())
    }

    /// Get a mutable reference to a buffer by id.
    pub fn get_mut(&mut self, id: u32) -> Option<&mut GlBuffer> {
        if id == 0 { return None; }
        self.slots.get_mut(id as usize).and_then(|s| s.as_mut())
    }

    /// Upload data into a buffer (glBufferData).
    pub fn buffer_data(&mut self, id: u32, data: &[u8], usage: GLenum) {
        if let Some(buf) = self.get_mut(id) {
            buf.data.clear();
            buf.data.extend_from_slice(data);
            buf.usage = usage;
        }
    }

    /// Update a sub-region of a buffer (glBufferSubData).
    pub fn buffer_sub_data(&mut self, id: u32, offset: usize, data: &[u8]) {
        if let Some(buf) = self.get_mut(id) {
            let end = offset + data.len();
            if end <= buf.data.len() {
                buf.data[offset..end].copy_from_slice(data);
            }
        }
    }
}
