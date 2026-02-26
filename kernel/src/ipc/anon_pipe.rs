//! Anonymous (POSIX) pipes for inter-process communication.
//!
//! Each pipe has a 4 KiB circular buffer, separate read/write reference counts,
//! and a list of blocked reader/writer TIDs for blocking I/O.
//!
//! **SMP safety:**
//! - All operations go through the `PIPES` Spinlock.
//! - Blocking uses `save_complete = 0` before `Blocked` (per save_complete race fix).
//! - Never hold the lock during `serial_println!` or `schedule()`.

use alloc::collections::VecDeque;
use crate::sync::spinlock::Spinlock;
use core::sync::atomic::{AtomicU32, Ordering};

/// Maximum number of concurrent anonymous pipes system-wide.
const MAX_PIPES: usize = 64;

/// Pipe buffer capacity in bytes.
pub const PIPE_BUF_SIZE: usize = 4096;

/// Maximum blocked TIDs per side (readers or writers).
const MAX_BLOCKED: usize = 8;

static NEXT_PIPE_ID: AtomicU32 = AtomicU32::new(1);

/// A single anonymous pipe.
struct AnonPipe {
    id: u32,
    buffer: VecDeque<u8>,
    /// Number of open read-end FDs referencing this pipe.
    read_refs: u32,
    /// Number of open write-end FDs referencing this pipe.
    write_refs: u32,
    /// TIDs blocked waiting to read (pipe was empty).
    blocked_readers: [u32; MAX_BLOCKED],
    blocked_reader_count: usize,
    /// TIDs blocked waiting to write (pipe was full).
    blocked_writers: [u32; MAX_BLOCKED],
    blocked_writer_count: usize,
}

/// Fixed-size table of anonymous pipes. No heap allocation for the table itself.
static PIPES: Spinlock<[Option<AnonPipe>; MAX_PIPES]> = Spinlock::new([const { None }; MAX_PIPES]);

/// Create a new anonymous pipe. Returns the pipe_id (>0), or 0 on failure (table full).
pub fn create() -> u32 {
    let id = NEXT_PIPE_ID.fetch_add(1, Ordering::Relaxed);
    let pipe = AnonPipe {
        id,
        buffer: VecDeque::with_capacity(PIPE_BUF_SIZE),
        read_refs: 1,
        write_refs: 1,
        blocked_readers: [0; MAX_BLOCKED],
        blocked_reader_count: 0,
        blocked_writers: [0; MAX_BLOCKED],
        blocked_writer_count: 0,
    };

    let mut guard = PIPES.lock();
    for slot in guard.iter_mut() {
        if slot.is_none() {
            *slot = Some(pipe);
            return id;
        }
    }
    0 // table full
}

/// Increment the read-end reference count (for fork/dup).
pub fn incref_read(pipe_id: u32) {
    let mut guard = PIPES.lock();
    if let Some(pipe) = find_pipe(&mut guard, pipe_id) {
        pipe.read_refs += 1;
    }
}

/// Increment the write-end reference count (for fork/dup).
pub fn incref_write(pipe_id: u32) {
    let mut guard = PIPES.lock();
    if let Some(pipe) = find_pipe(&mut guard, pipe_id) {
        pipe.write_refs += 1;
    }
}

/// Decrement the read-end reference count.
/// If read_refs drops to 0, wake all blocked writers (they'll get EPIPE).
/// If both refs are 0, destroy the pipe.
pub fn decref_read(pipe_id: u32) {
    let mut writers_to_wake: [u32; MAX_BLOCKED] = [0; MAX_BLOCKED];
    let mut wake_count = 0;

    {
        let mut guard = PIPES.lock();
        let slot = find_pipe_slot(&mut guard, pipe_id);
        if let Some(Some(pipe)) = slot {
            if pipe.read_refs > 0 {
                pipe.read_refs -= 1;
            }
            if pipe.read_refs == 0 {
                // No more readers — wake blocked writers so they see EPIPE
                wake_count = pipe.blocked_writer_count;
                writers_to_wake[..wake_count].copy_from_slice(&pipe.blocked_writers[..wake_count]);
                pipe.blocked_writer_count = 0;
            }
            if pipe.read_refs == 0 && pipe.write_refs == 0 {
                *slot.unwrap() = None; // SAFETY: slot is Some from the if-let
            }
        }
    }

    // Wake outside the lock
    for i in 0..wake_count {
        crate::task::scheduler::wake_thread(writers_to_wake[i]);
    }
}

/// Decrement the write-end reference count.
/// If write_refs drops to 0, wake all blocked readers (they'll get EOF = 0 bytes).
/// If both refs are 0, destroy the pipe.
pub fn decref_write(pipe_id: u32) {
    let mut readers_to_wake: [u32; MAX_BLOCKED] = [0; MAX_BLOCKED];
    let mut wake_count = 0;

    {
        let mut guard = PIPES.lock();
        let slot = find_pipe_slot(&mut guard, pipe_id);
        if let Some(Some(pipe)) = slot {
            if pipe.write_refs > 0 {
                pipe.write_refs -= 1;
            }
            if pipe.write_refs == 0 {
                // No more writers — wake blocked readers so they see EOF
                wake_count = pipe.blocked_reader_count;
                readers_to_wake[..wake_count].copy_from_slice(&pipe.blocked_readers[..wake_count]);
                pipe.blocked_reader_count = 0;
            }
            if pipe.read_refs == 0 && pipe.write_refs == 0 {
                *slot.unwrap() = None;
            }
        }
    }

    for i in 0..wake_count {
        crate::task::scheduler::wake_thread(readers_to_wake[i]);
    }
}

/// Read from a pipe. Blocks if the buffer is empty and writers still exist.
/// Returns the number of bytes read, or 0 for EOF (no writers left).
///
/// **Must be called from a syscall context** (not from IRQ handler).
pub fn read(pipe_id: u32, buf: &mut [u8]) -> u32 {
    if buf.is_empty() {
        return 0;
    }

    loop {
        let readers_to_wake: [u32; MAX_BLOCKED] = [0; MAX_BLOCKED];
        let r_wake = 0;
        let mut writers_to_wake: [u32; MAX_BLOCKED] = [0; MAX_BLOCKED];
        let mut w_wake = 0;

        let result = {
            let mut guard = PIPES.lock();
            let pipe = match find_pipe(&mut guard, pipe_id) {
                Some(p) => p,
                None => return 0, // pipe destroyed
            };

            if !pipe.buffer.is_empty() {
                // Data available — drain up to buf.len()
                let n = buf.len().min(pipe.buffer.len());
                for i in 0..n {
                    buf[i] = pipe.buffer.pop_front().unwrap();
                }
                // Wake any blocked writers since we freed buffer space
                w_wake = pipe.blocked_writer_count;
                writers_to_wake[..w_wake].copy_from_slice(&pipe.blocked_writers[..w_wake]);
                pipe.blocked_writer_count = 0;
                Ok(n as u32)
            } else if pipe.write_refs == 0 {
                // EOF — no writers left
                Ok(0)
            } else {
                // Buffer empty, writers exist — need to block
                let tid = crate::task::scheduler::current_tid();
                if pipe.blocked_reader_count < MAX_BLOCKED {
                    pipe.blocked_readers[pipe.blocked_reader_count] = tid;
                    pipe.blocked_reader_count += 1;
                }
                Err(()) // signal: must block
            }
        };

        // Wake outside the lock
        for i in 0..w_wake {
            crate::task::scheduler::wake_thread(writers_to_wake[i]);
        }
        for i in 0..r_wake {
            crate::task::scheduler::wake_thread(readers_to_wake[i]);
        }

        match result {
            Ok(n) => return n,
            Err(()) => {
                // Block this thread and retry after wake
                crate::task::scheduler::block_current_thread();
            }
        }
    }
}

/// Write to a pipe. Blocks if the buffer is full and readers still exist.
/// Returns the number of bytes written, or u32::MAX if no readers (EPIPE).
///
/// **Must be called from a syscall context** (not from IRQ handler).
pub fn write(pipe_id: u32, data: &[u8]) -> u32 {
    if data.is_empty() {
        return 0;
    }

    let mut written = 0usize;

    loop {
        let mut readers_to_wake: [u32; MAX_BLOCKED] = [0; MAX_BLOCKED];
        let mut r_wake = 0;

        let result = {
            let mut guard = PIPES.lock();
            let pipe = match find_pipe(&mut guard, pipe_id) {
                Some(p) => p,
                None => return u32::MAX, // pipe destroyed (EPIPE)
            };

            if pipe.read_refs == 0 {
                // No readers — send SIGPIPE to current thread, return EPIPE
                let tid = crate::task::scheduler::current_tid();
                crate::task::scheduler::send_signal_to_thread(tid, crate::ipc::signal::SIGPIPE);
                return u32::MAX;
            }

            // Write as much as we can fit
            let space = PIPE_BUF_SIZE.saturating_sub(pipe.buffer.len());
            if space > 0 {
                let remaining = &data[written..];
                let n = remaining.len().min(space);
                for i in 0..n {
                    pipe.buffer.push_back(remaining[i]);
                }
                written += n;

                // Wake blocked readers since we added data
                r_wake = pipe.blocked_reader_count;
                readers_to_wake[..r_wake].copy_from_slice(&pipe.blocked_readers[..r_wake]);
                pipe.blocked_reader_count = 0;

                if written >= data.len() {
                    Ok(written as u32)
                } else {
                    // Buffer full but still have data — need to block and retry
                    let tid = crate::task::scheduler::current_tid();
                    if pipe.blocked_writer_count < MAX_BLOCKED {
                        pipe.blocked_writers[pipe.blocked_writer_count] = tid;
                        pipe.blocked_writer_count += 1;
                    }
                    Err(()) // signal: must block to write more
                }
            } else {
                // Buffer completely full — block
                let tid = crate::task::scheduler::current_tid();
                if pipe.blocked_writer_count < MAX_BLOCKED {
                    pipe.blocked_writers[pipe.blocked_writer_count] = tid;
                    pipe.blocked_writer_count += 1;
                }
                Err(()) // signal: must block
            }
        };

        // Wake outside the lock
        for i in 0..r_wake {
            crate::task::scheduler::wake_thread(readers_to_wake[i]);
        }

        match result {
            Ok(n) => return n,
            Err(()) => {
                // Block this thread and retry after wake
                crate::task::scheduler::block_current_thread();
            }
        }
    }
}

/// Return the number of bytes currently buffered in the pipe (non-blocking).
///
/// Returns 0 if the pipe does not exist or is empty.
/// Callers can use this together with `is_write_closed()` to distinguish
/// "empty but more data may arrive" from "empty and at EOF".
pub fn bytes_available(pipe_id: u32) -> u32 {
    let guard = PIPES.lock();
    guard.iter()
        .find_map(|slot| {
            slot.as_ref()
                .filter(|p| p.id == pipe_id)
                .map(|p| p.buffer.len() as u32)
        })
        .unwrap_or(0)
}

/// Return true if the write end of the pipe has been fully closed.
///
/// When the write end is closed and the buffer is empty, `read()` returns 0
/// (EOF).  A `poll()` caller can combine this with `bytes_available() == 0`
/// to report `POLLHUP`.
pub fn is_write_closed(pipe_id: u32) -> bool {
    let guard = PIPES.lock();
    guard.iter()
        .find_map(|slot| {
            slot.as_ref()
                .filter(|p| p.id == pipe_id)
                .map(|p| p.write_refs == 0)
        })
        .unwrap_or(true) // pipe not found → treat as closed
}

// ---- Internal helpers ----

fn find_pipe<'a>(slots: &'a mut [Option<AnonPipe>; MAX_PIPES], id: u32) -> Option<&'a mut AnonPipe> {
    slots.iter_mut()
        .find_map(|slot| slot.as_mut().filter(|p| p.id == id))
}

fn find_pipe_slot<'a>(slots: &'a mut [Option<AnonPipe>; MAX_PIPES], id: u32) -> Option<&'a mut Option<AnonPipe>> {
    slots.iter_mut()
        .find(|slot| slot.as_ref().map_or(false, |p| p.id == id))
}
