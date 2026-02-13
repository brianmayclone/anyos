//! Named kernel pipes -- byte-buffer IPC with string names for discoverability.
//!
//! Pipes serve as unidirectional byte streams identified by human-readable names.
//! Any process can open, read from, or write to a pipe by name, making them useful
//! for stdout/stdin routing, application-level IPC, and system monitoring channels.

use alloc::vec::Vec;
use crate::sync::spinlock::Spinlock;

const MAX_PIPE_NAME: usize = 64;

struct Pipe {
    id: u32,
    name: [u8; MAX_PIPE_NAME],
    name_len: usize,
    buffer: Vec<u8>,
}

impl Pipe {
    fn name_str(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("???")
    }
}

static PIPES: Spinlock<Vec<Pipe>> = Spinlock::new(Vec::new());
static NEXT_PIPE_ID: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(1);

/// Create a new named pipe. Returns a unique pipe ID (always > 0).
pub fn create(name: &str) -> u32 {
    let id = NEXT_PIPE_ID.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    let mut name_buf = [0u8; MAX_PIPE_NAME];
    let len = name.len().min(MAX_PIPE_NAME - 1);
    name_buf[..len].copy_from_slice(&name.as_bytes()[..len]);

    let mut pipes = PIPES.lock();
    pipes.push(Pipe { id, name: name_buf, name_len: len, buffer: Vec::new() });
    id
}

/// Open an existing pipe by name. Returns pipe_id or 0 if not found.
pub fn open(name: &str) -> u32 {
    let pipes = PIPES.lock();
    for pipe in pipes.iter() {
        if pipe.name_str() == name {
            return pipe.id;
        }
    }
    0
}

/// Write data into a pipe buffer. Returns bytes written, or u32::MAX if pipe not found.
pub fn write(pipe_id: u32, data: &[u8]) -> u32 {
    let mut pipes = PIPES.lock();
    if let Some(pipe) = pipes.iter_mut().find(|p| p.id == pipe_id) {
        pipe.buffer.extend_from_slice(data);
        data.len() as u32
    } else {
        u32::MAX
    }
}

/// Read available data from a pipe. Returns bytes read, or u32::MAX if pipe not found.
/// Non-blocking: returns 0 if the pipe is empty.
pub fn read(pipe_id: u32, buf: &mut [u8]) -> u32 {
    let mut pipes = PIPES.lock();
    if let Some(pipe) = pipes.iter_mut().find(|p| p.id == pipe_id) {
        let n = pipe.buffer.len().min(buf.len());
        buf[..n].copy_from_slice(&pipe.buffer[..n]);
        pipe.buffer.drain(..n);
        n as u32
    } else {
        u32::MAX
    }
}

/// Clear a pipe's buffer (for overwrite-style pipes like cpu_load).
pub fn clear(pipe_id: u32) {
    let mut pipes = PIPES.lock();
    if let Some(pipe) = pipes.iter_mut().find(|p| p.id == pipe_id) {
        pipe.buffer.clear();
    }
}

/// Close and destroy a pipe, freeing its buffer.
pub fn close(pipe_id: u32) {
    let mut pipes = PIPES.lock();
    pipes.retain(|p| p.id != pipe_id);
}

/// Snapshot of a pipe's state for debug listing and inspection.
pub struct PipeInfo {
    /// Unique pipe identifier.
    pub id: u32,
    /// Pipe name as a null-terminated byte array.
    pub name: [u8; MAX_PIPE_NAME],
    /// Length of the name in bytes (excluding null terminator).
    pub name_len: usize,
    /// Number of bytes currently in the pipe's buffer.
    pub buffered: usize,
}

/// Lock-free check if the pipe lock is currently held.
pub fn is_pipe_locked() -> bool {
    PIPES.is_locked()
}

/// List all open pipes (for debug/inspection).
pub fn list() -> Vec<PipeInfo> {
    let pipes = PIPES.lock();
    pipes.iter().map(|p| PipeInfo {
        id: p.id,
        name: p.name,
        name_len: p.name_len,
        buffered: p.buffer.len(),
    }).collect()
}
