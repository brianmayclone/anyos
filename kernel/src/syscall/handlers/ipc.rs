//! Inter-process communication syscall handlers.
//!
//! Covers named pipes, the event bus (system + channel events),
//! shared memory, and compositor wake helper.

use super::helpers::{is_valid_user_ptr, read_user_str};

use crate::ipc::event_bus::{self, EventData};
use core::sync::atomic::Ordering;

// =========================================================================
// Pipes (SYS_PIPE_*)
// =========================================================================

/// sys_pipe_create - Create a new named pipe. arg1=name_ptr (null-terminated).
/// Returns pipe_id (always > 0).
pub fn sys_pipe_create(name_ptr: u32) -> u32 {
    let name = if name_ptr != 0 {
        unsafe { read_user_str(name_ptr) }
    } else {
        "unnamed"
    };
    crate::ipc::pipe::create(name)
}

/// sys_pipe_read - Read from a pipe. Returns bytes read, or u32::MAX if not found.
pub fn sys_pipe_read(pipe_id: u32, buf_ptr: u32, len: u32) -> u32 {
    if buf_ptr == 0 || len == 0 || !is_valid_user_ptr(buf_ptr as u64, len as u64) { return 0; }
    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, len as usize) };
    crate::ipc::pipe::read(pipe_id, buf)
}

/// sys_pipe_close - Destroy a pipe and free its buffer.
pub fn sys_pipe_close(pipe_id: u32) -> u32 {
    crate::ipc::pipe::close(pipe_id);
    0
}

/// sys_pipe_write - Write data to a pipe. Returns bytes written.
pub fn sys_pipe_write(pipe_id: u32, buf_ptr: u32, len: u32) -> u32 {
    if buf_ptr == 0 || len == 0 || !is_valid_user_ptr(buf_ptr as u64, len as u64) { return 0; }
    let buf = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, len as usize) };
    crate::ipc::pipe::write(pipe_id, buf)
}

/// sys_pipe_open - Open an existing pipe by name. Returns pipe_id or 0 if not found.
pub fn sys_pipe_open(name_ptr: u32) -> u32 {
    if name_ptr == 0 { return 0; }
    let name = unsafe { read_user_str(name_ptr) };
    crate::ipc::pipe::open(name)
}

/// sys_pipe_list - List all open pipes. Each entry is 80 bytes:
///   [0..4]   pipe_id (u32 LE)
///   [4..8]   buffered_bytes (u32 LE)
///   [8..72]  name (64 bytes, null-terminated)
///   [72..80] padding (zeroed)
/// Returns total pipe count.
pub fn sys_pipe_list(buf_ptr: u32, buf_size: u32) -> u32 {
    let pipes = crate::ipc::pipe::list();
    let count = pipes.len();
    if buf_ptr != 0 && buf_size > 0 && is_valid_user_ptr(buf_ptr as u64, buf_size as u64) {
        let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_size as usize) };
        let entry_size = 80usize;
        let max_entries = buf_size as usize / entry_size;
        for (i, pipe) in pipes.iter().enumerate().take(max_entries.min(count)) {
            let offset = i * entry_size;
            // Zero the entry first
            for b in &mut buf[offset..offset + entry_size] { *b = 0; }
            // pipe_id [0..4]
            buf[offset..offset + 4].copy_from_slice(&pipe.id.to_le_bytes());
            // buffered [4..8]
            buf[offset + 4..offset + 8].copy_from_slice(&(pipe.buffered as u32).to_le_bytes());
            // name [8..72]
            let nlen = pipe.name_len.min(63);
            buf[offset + 8..offset + 8 + nlen].copy_from_slice(&pipe.name[..nlen]);
        }
    }
    count as u32
}

// =========================================================================
// Event bus (SYS_EVT_*)
// =========================================================================

/// Subscribe to system events. ebx=filter (0=all). Returns sub_id.
pub fn sys_evt_sys_subscribe(filter: u32) -> u32 {
    event_bus::system_subscribe(filter)
}

/// Poll system event. ebx=sub_id, ecx=buf_ptr (20 bytes). Returns 1 if event, 0 if empty.
pub fn sys_evt_sys_poll(sub_id: u32, buf_ptr: u32) -> u32 {
    if let Some(evt) = event_bus::system_poll(sub_id) {
        if buf_ptr != 0 && is_valid_user_ptr(buf_ptr as u64, 20) {
            let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u32, 5) };
            buf.copy_from_slice(&evt.words);
        }
        1
    } else {
        0
    }
}

/// Unsubscribe from system events. ebx=sub_id.
pub fn sys_evt_sys_unsubscribe(sub_id: u32) -> u32 {
    event_bus::system_unsubscribe(sub_id);
    0
}

/// Create a module channel. ebx=name_ptr, ecx=name_len. Returns channel_id.
pub fn sys_evt_chan_create(name_ptr: u32, name_len: u32) -> u32 {
    let len = (name_len as usize).min(256);
    let name_bytes = unsafe { core::slice::from_raw_parts(name_ptr as *const u8, len) };
    event_bus::channel_create(name_bytes)
}

/// Subscribe to module channel. ebx=chan_id, ecx=filter. Returns sub_id.
pub fn sys_evt_chan_subscribe(chan_id: u32, filter: u32) -> u32 {
    event_bus::channel_subscribe(chan_id, filter)
}

/// Emit to module channel. ebx=chan_id, ecx=event_ptr (20 bytes). Returns 0.
pub fn sys_evt_chan_emit(chan_id: u32, event_ptr: u32) -> u32 {
    if event_ptr == 0 { return u32::MAX; }
    let words = unsafe { core::slice::from_raw_parts(event_ptr as *const u32, 5) };
    let evt = EventData { words: [words[0], words[1], words[2], words[3], words[4]] };
    event_bus::channel_emit(chan_id, evt);
    0
}

/// Poll module channel. ebx=chan_id, ecx=sub_id, edx=buf_ptr. Returns 1/0.
pub fn sys_evt_chan_poll(chan_id: u32, sub_id: u32, buf_ptr: u32) -> u32 {
    if let Some(evt) = event_bus::channel_poll(chan_id, sub_id) {
        if buf_ptr != 0 && is_valid_user_ptr(buf_ptr as u64, 20) {
            let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u32, 5) };
            buf.copy_from_slice(&evt.words);
        }
        1
    } else {
        0
    }
}

/// Unsubscribe from module channel. ebx=chan_id, ecx=sub_id.
pub fn sys_evt_chan_unsubscribe(chan_id: u32, sub_id: u32) -> u32 {
    event_bus::channel_unsubscribe(chan_id, sub_id);
    0
}

/// Destroy a module channel. ebx=chan_id.
pub fn sys_evt_chan_destroy(chan_id: u32) -> u32 {
    event_bus::channel_destroy(chan_id);
    0
}

/// Emit to a specific subscriber (unicast). ebx=chan_id, r10=sub_id, rdx=event_ptr.
pub fn sys_evt_chan_emit_to(chan_id: u32, sub_id: u32, event_ptr: u32) -> u32 {
    if event_ptr == 0 { return u32::MAX; }
    let words = unsafe { core::slice::from_raw_parts(event_ptr as *const u32, 5) };
    let evt = EventData { words: [words[0], words[1], words[2], words[3], words[4]] };
    event_bus::channel_emit_to(chan_id, sub_id, evt);
    0
}

/// Block until an event is available on a channel subscription, or timeout.
///
/// Returns 1 if events are available, 0 on timeout/spurious wake.
/// `timeout_ms` = `u32::MAX` means wait indefinitely (capped at 60s safety net).
pub fn sys_evt_chan_wait(chan_id: u32, sub_id: u32, timeout_ms: u32) -> u32 {
    // Fast path: events already queued — return immediately.
    if event_bus::channel_has_events(chan_id, sub_id) {
        return 1;
    }

    let tid = crate::task::scheduler::current_tid();

    // Register this thread as a waiter on the subscription.
    // Returns false if events arrived between our check and the registration.
    if !event_bus::channel_register_waiter(chan_id, sub_id, tid) {
        return 1; // Events arrived — don't block
    }

    // Compute wake tick. Cap indefinite waits at 60 seconds as safety net.
    let effective_ms = if timeout_ms == u32::MAX { 60_000 } else { timeout_ms };
    let pit_hz = crate::arch::x86::pit::TICK_HZ;
    let ticks = (effective_ms as u64 * pit_hz as u64 / 1000) as u32;
    let ticks = if ticks == 0 { 1 } else { ticks };
    let now = crate::arch::x86::pit::get_ticks();
    let wake_at = now.wrapping_add(ticks);

    // Block — thread sleeps until: emit wakes us, input IRQ wakes us, or timeout.
    crate::task::scheduler::sleep_until(wake_at);

    // Clean up waiter registration (may already be cleared by emit).
    event_bus::channel_unregister_waiter(chan_id, sub_id);

    // Check if events are available (handles spurious wakes gracefully).
    if event_bus::channel_has_events(chan_id, sub_id) {
        1
    } else {
        0 // Timeout or spurious wake — caller should check all event sources
    }
}

/// Wake the compositor's management thread if it is blocked.
///
/// Uses a two-tier approach for IRQ safety:
///   1. `try_wake_thread()` — non-blocking attempt (succeeds if SCHEDULER lock
///      is uncontended, which is the common case).
///   2. `deferred_wake()` — lock-free atomic enqueue; the next timer tick drains
///      deferred wakes under the already-held SCHEDULER lock.
///
/// This avoids ever calling the blocking `SCHEDULER.lock()` from IRQ context,
/// which could stall the IRQ handler on a contended lock and interact badly
/// with SMP context-switch timing (observed as GPF at irq_iret_done with
/// RSP outside kernel stack bounds).
pub fn wake_compositor_if_blocked() {
    let tid = super::COMPOSITOR_TID.load(Ordering::Relaxed);
    if tid != 0 {
        if !crate::task::scheduler::try_wake_thread(tid) {
            // Lock contended — defer to next timer tick (≤1ms latency).
            crate::task::scheduler::deferred_wake(tid);
        }
    }
}

// =========================================================================
// Shared memory (SYS_SHM_CREATE, SYS_SHM_MAP, SYS_SHM_UNMAP, SYS_SHM_DESTROY)
// =========================================================================

/// Create a shared memory region. ebx=size (bytes, rounded up to page).
/// Returns shm_id (>0) on success, 0 on failure.
pub fn sys_shm_create(size: u32) -> u32 {
    let tid = crate::task::scheduler::current_tid();
    match crate::ipc::shared_memory::create(size as usize, tid) {
        Some(id) => id,
        None => 0,
    }
}

/// Map a shared memory region into the caller's address space.
/// ebx=shm_id. Returns virtual address (u32) or 0 on failure.
pub fn sys_shm_map(shm_id: u32) -> u32 {
    crate::ipc::shared_memory::map_into_current(shm_id) as u32
}

/// Unmap a shared memory region from the caller's address space.
/// ebx=shm_id. Returns 0 on success, u32::MAX on failure.
pub fn sys_shm_unmap(shm_id: u32) -> u32 {
    if crate::ipc::shared_memory::unmap_from_current(shm_id) {
        0
    } else {
        u32::MAX
    }
}

/// Destroy a shared memory region (owner only).
/// ebx=shm_id. Returns 0 on success, u32::MAX on failure.
pub fn sys_shm_destroy(shm_id: u32) -> u32 {
    let tid = crate::task::scheduler::current_tid();
    if crate::ipc::shared_memory::destroy(shm_id, tid) {
        0
    } else {
        u32::MAX
    }
}
