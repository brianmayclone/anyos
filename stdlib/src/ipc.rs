//! Inter-process communication — named pipes and event bus.

use crate::raw::*;

// ─── Named Pipes ──────────────────────────────────────────────────────

/// Create a named pipe. Returns pipe_id (always > 0).
pub fn pipe_create(name: &str) -> u32 {
    let mut buf = [0u8; 65];
    let len = name.len().min(64);
    buf[..len].copy_from_slice(&name.as_bytes()[..len]);
    buf[len] = 0;
    syscall1(SYS_PIPE_CREATE, buf.as_ptr() as u64)
}

/// Read from a pipe. Returns bytes read, 0 if empty, u32::MAX if not found.
pub fn pipe_read(pipe_id: u32, buf: &mut [u8]) -> u32 {
    syscall3(SYS_PIPE_READ, pipe_id as u64, buf.as_mut_ptr() as u64, buf.len() as u64)
}

/// Write to a pipe. Returns bytes written, u32::MAX if not found.
pub fn pipe_write(pipe_id: u32, data: &[u8]) -> u32 {
    syscall3(SYS_PIPE_WRITE, pipe_id as u64, data.as_ptr() as u64, data.len() as u64)
}

/// Open an existing pipe by name. Returns pipe_id or 0 if not found.
pub fn pipe_open(name: &str) -> u32 {
    let mut buf = [0u8; 65];
    let len = name.len().min(64);
    buf[..len].copy_from_slice(&name.as_bytes()[..len]);
    buf[len] = 0;
    syscall1(SYS_PIPE_OPEN, buf.as_ptr() as u64)
}

/// Close and destroy a pipe.
pub fn pipe_close(pipe_id: u32) -> u32 {
    syscall1(SYS_PIPE_CLOSE, pipe_id as u64)
}

// ─── Event Bus — System Events ────────────────────────────────────────

/// Subscribe to system events. filter=0 means all events. Returns sub_id.
pub fn evt_sys_subscribe(filter: u32) -> u32 {
    syscall1(SYS_EVT_SYS_SUBSCRIBE, filter as u64)
}

/// Poll for next system event. Returns true if an event was written to buf.
pub fn evt_sys_poll(sub_id: u32, buf: &mut [u32; 5]) -> bool {
    syscall2(SYS_EVT_SYS_POLL, sub_id as u64, buf.as_mut_ptr() as u64) == 1
}

/// Unsubscribe from system events.
pub fn evt_sys_unsubscribe(sub_id: u32) {
    syscall1(SYS_EVT_SYS_UNSUBSCRIBE, sub_id as u64);
}

// ─── Event Bus — Module Channels ──────────────────────────────────────

/// Create a named module channel. Returns channel_id (hash).
pub fn evt_chan_create(name: &str) -> u32 {
    syscall2(SYS_EVT_CHAN_CREATE, name.as_ptr() as u64, name.len() as u64)
}

/// Subscribe to a module channel. filter=0 means all. Returns sub_id.
pub fn evt_chan_subscribe(channel_id: u32, filter: u32) -> u32 {
    syscall2(SYS_EVT_CHAN_SUBSCRIBE, channel_id as u64, filter as u64)
}

/// Emit an event to a module channel.
pub fn evt_chan_emit(channel_id: u32, event: &[u32; 5]) {
    syscall2(SYS_EVT_CHAN_EMIT, channel_id as u64, event.as_ptr() as u64);
}

/// Poll for next event on a module channel subscription.
pub fn evt_chan_poll(channel_id: u32, sub_id: u32, buf: &mut [u32; 5]) -> bool {
    syscall3(SYS_EVT_CHAN_POLL, channel_id as u64, sub_id as u64, buf.as_mut_ptr() as u64) == 1
}

/// Unsubscribe from a module channel.
pub fn evt_chan_unsubscribe(channel_id: u32, sub_id: u32) {
    syscall2(SYS_EVT_CHAN_UNSUBSCRIBE, channel_id as u64, sub_id as u64);
}

/// Destroy a module channel.
pub fn evt_chan_destroy(channel_id: u32) {
    syscall1(SYS_EVT_CHAN_DESTROY, channel_id as u64);
}
