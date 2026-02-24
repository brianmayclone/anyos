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

/// Emit an event to a module channel (broadcast to all subscribers).
pub fn evt_chan_emit(channel_id: u32, event: &[u32; 5]) {
    syscall2(SYS_EVT_CHAN_EMIT, channel_id as u64, event.as_ptr() as u64);
}

/// Emit an event to a specific subscriber on a module channel (unicast).
pub fn evt_chan_emit_to(channel_id: u32, sub_id: u32, event: &[u32; 5]) {
    syscall3(SYS_EVT_CHAN_EMIT_TO, channel_id as u64, sub_id as u64, event.as_ptr() as u64);
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

/// Block until an event is available on a channel subscription, or timeout.
///
/// Returns 1 if events are available, 0 on timeout/spurious wake.
/// `timeout_ms` = `u32::MAX` means wait indefinitely (kernel caps at 60s safety net).
pub fn evt_chan_wait(channel_id: u32, sub_id: u32, timeout_ms: u32) -> u32 {
    syscall3(SYS_EVT_CHAN_WAIT, channel_id as u64, sub_id as u64, timeout_ms as u64)
}

// ─── Shared Memory ──────────────────────────────────────────────────

/// Create a shared memory region. Returns shm_id (>0) or 0 on failure.
pub fn shm_create(size: u32) -> u32 {
    syscall1(SYS_SHM_CREATE, size as u64)
}

/// Map a shared memory region into the caller's address space.
/// Returns virtual address or 0 on failure.
pub fn shm_map(shm_id: u32) -> u32 {
    syscall1(SYS_SHM_MAP, shm_id as u64)
}

/// Unmap a shared memory region. Returns 0 on success.
pub fn shm_unmap(shm_id: u32) -> u32 {
    syscall1(SYS_SHM_UNMAP, shm_id as u64)
}

/// Destroy a shared memory region (owner only). Returns 0 on success.
pub fn shm_destroy(shm_id: u32) -> u32 {
    syscall1(SYS_SHM_DESTROY, shm_id as u64)
}

// ─── Compositor-Privileged ──────────────────────────────────────────

/// Register calling process as the compositor. Returns 0 on success.
pub fn register_compositor() -> u32 {
    syscall0(SYS_REGISTER_COMPOSITOR)
}

/// Framebuffer mapping info returned by [`map_framebuffer`].
#[repr(C)]
pub struct FbMapInfo {
    pub fb_addr: u32,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
}

/// Map the GPU framebuffer into the compositor's address space.
/// Returns Some(FbMapInfo) on success, None on failure.
pub fn map_framebuffer() -> Option<FbMapInfo> {
    let mut info = FbMapInfo { fb_addr: 0, width: 0, height: 0, pitch: 0 };
    let ret = syscall1(SYS_MAP_FRAMEBUFFER, &mut info as *mut FbMapInfo as u64);
    if ret == 0 { Some(info) } else { None }
}

/// Submit GPU acceleration commands. Returns number of commands executed.
/// Each command is [u32; 9]: { cmd_type, args[0..8] }.
pub fn gpu_command(cmds: &[[u32; 9]]) -> u32 {
    if cmds.is_empty() { return 0; }
    syscall2(SYS_GPU_COMMAND, cmds.as_ptr() as u64, cmds.len() as u64)
}

/// Query total GPU VRAM size in bytes. Compositor-only.
/// Returns 0 if no GPU driver or caller is not compositor.
pub fn gpu_vram_size() -> u32 {
    syscall0(SYS_GPU_VRAM_SIZE)
}

/// Map VRAM pages into a target app's address space. Compositor-only.
/// Returns the user VA (0x18000000) on success, 0 on failure.
pub fn vram_map(target_tid: u32, vram_byte_offset: u32, num_bytes: u32) -> u32 {
    syscall3(SYS_VRAM_MAP, target_tid as u64, vram_byte_offset as u64, num_bytes as u64)
}

/// Register the compositor's back buffer for GPU DMA transfers (GMR).
/// After success (returns 0), the GPU reads directly from the back buffer
/// via DMA instead of requiring a CPU memcpy to VRAM.
pub fn gpu_register_backbuffer(buf_ptr: u32, buf_size: u32) -> u32 {
    syscall2(SYS_GPU_REGISTER_BACKBUFFER, buf_ptr as u64, buf_size as u64)
}

/// Poll raw input events. Returns number of events written to buf.
/// Each event is [u32; 5]: { event_type, arg0, arg1, arg2, arg3 }.
pub fn input_poll(buf: &mut [[u32; 5]]) -> u32 {
    if buf.is_empty() { return 0; }
    syscall2(SYS_INPUT_POLL, buf.as_mut_ptr() as u64, buf.len() as u64)
}

/// Take over cursor from kernel splash mode. Compositor-only.
/// Returns the splash cursor position (x, y) and disables kernel cursor tracking.
/// All pending mouse events from boot are drained to prevent double-application.
pub fn cursor_takeover() -> (i32, i32) {
    let packed = syscall0(SYS_CURSOR_TAKEOVER);
    let x = (packed >> 16) as i16 as i32;
    let y = packed as u16 as i32;
    (x, y)
}
