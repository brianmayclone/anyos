//! Named kernel pipes -- byte-buffer IPC with string names for discoverability.
//!
//! Pipes serve as unidirectional byte streams identified by human-readable names.
//! Any process can open, read from, or write to a pipe by name, making them useful
//! for stdout/stdin routing, application-level IPC, and system monitoring channels.

use crate::sync::spinlock::Spinlock;
use alloc::collections::VecDeque;
use alloc::vec::Vec;

const MAX_PIPE_NAME: usize = 64;
const MAX_PIPE_BUFFER: usize = 256 * 1024;
const ATOMIC_WRITE_LIMIT: usize = 64 * 1024;

struct Pipe {
    id: u32,
    name: [u8; MAX_PIPE_NAME],
    name_len: usize,
    buffer: VecDeque<u8>,
}

impl Pipe {
    fn name_str(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("???")
    }
}

static PIPES: Spinlock<Vec<Pipe>> = Spinlock::new(Vec::new());
static NEXT_PIPE_ID: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(1);

/// Create a new named pipe. Returns a unique pipe ID (always > 0).
///
/// If a stale pipe with the same name exists, replace it.  The old
/// implementation allowed duplicate names, which made `open(name)` return the
/// first stale pipe and could silently route replies into a dead buffer.
pub fn create(name: &str) -> u32 {
    let id = NEXT_PIPE_ID.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    let mut name_buf = [0u8; MAX_PIPE_NAME];
    let len = name.len().min(MAX_PIPE_NAME - 1);
    name_buf[..len].copy_from_slice(&name.as_bytes()[..len]);

    let mut pipes = PIPES.lock();
    pipes.retain(|p| !pipe_name_eq(p, &name_buf, len));
    pipes.push(Pipe {
        id,
        name: name_buf,
        name_len: len,
        buffer: VecDeque::new(),
    });
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

/// Write data into a pipe buffer. Returns bytes written, 0 when a bounded pipe
/// cannot accept an atomic message yet, or u32::MAX if the pipe is not found.
///
/// Small writes are all-or-nothing so line/RPC messages cannot be silently
/// truncated.  Very large writes may be accepted partially, matching the old
/// byte-stream behavior used by stdout-style pipes.
pub fn write(pipe_id: u32, data: &[u8]) -> u32 {
    if data.is_empty() {
        return 0;
    }

    let mut pipes = PIPES.lock();
    if let Some(pipe) = pipes.iter_mut().find(|p| p.id == pipe_id) {
        let available = MAX_PIPE_BUFFER.saturating_sub(pipe.buffer.len());
        if data.len() <= ATOMIC_WRITE_LIMIT && available < data.len() {
            return 0;
        }
        let n = data.len().min(available);
        if n == 0 {
            return 0;
        }
        pipe.buffer.extend(&data[..n]);
        n as u32
    } else {
        u32::MAX
    }
}

/// Read available data from a pipe. Returns bytes read, or u32::MAX if pipe not found.
/// Non-blocking: returns 0 if the pipe is empty.
pub fn read(pipe_id: u32, buf: &mut [u8]) -> u32 {
    if buf.is_empty() {
        return 0;
    }

    let mut pipes = PIPES.lock();
    if let Some(pipe) = pipes.iter_mut().find(|p| p.id == pipe_id) {
        let n = pipe.buffer.len().min(buf.len());
        let (front, back) = pipe.buffer.as_slices();
        let from_front = n.min(front.len());
        buf[..from_front].copy_from_slice(&front[..from_front]);
        if from_front < n {
            let from_back = n - from_front;
            buf[from_front..n].copy_from_slice(&back[..from_back]);
        }
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
    pipes
        .iter()
        .map(|p| PipeInfo {
            id: p.id,
            name: p.name,
            name_len: p.name_len,
            buffered: p.buffer.len(),
        })
        .collect()
}

fn pipe_name_eq(pipe: &Pipe, name: &[u8; MAX_PIPE_NAME], name_len: usize) -> bool {
    pipe.name_len == name_len && pipe.name[..name_len] == name[..name_len]
}
