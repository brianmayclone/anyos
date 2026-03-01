//! Debug / trace syscall handlers (anyTrace).
//!
//! Provides process debugging primitives: attach/detach, suspend/resume,
//! register and memory inspection, software breakpoints, single-step,
//! memory map queries, debug event polling, and extended thread info.
//!
//! All handlers require `CAP_DEBUG`.

use super::helpers::is_valid_user_ptr;
use crate::task::scheduler;
#[allow(unused_imports)]
use crate::memory::virtual_mem;

// ---- Debug event types (must match userspace constants) ----

/// Thread was suspended by an INT3 software breakpoint hit.
const DEBUG_EVENT_BREAKPOINT: u32 = 1;
/// Thread completed a single-step (#DB with TF).
const DEBUG_EVENT_SINGLE_STEP: u32 = 2;
/// Thread exited while debug-attached.
const DEBUG_EVENT_EXIT: u32 = 3;

// =========================================================================
// SYS_DEBUG_ATTACH (300) — Attach debugger to a target thread
// =========================================================================

/// Attach the calling thread as debugger of target `tid`.
///
/// The target thread is suspended (Blocked) so the debugger can inspect it.
/// Returns 0 on success, u32::MAX on error.
pub fn sys_debug_attach(tid: u32) -> u32 {
    let caller_tid = scheduler::current_tid();

    // Cannot attach to self
    if tid == caller_tid || tid == 0 {
        return u32::MAX;
    }

    scheduler::debug_attach(caller_tid, tid)
}

// =========================================================================
// SYS_DEBUG_DETACH (301) — Detach debugger from a target thread
// =========================================================================

/// Detach from a previously attached thread, removing all breakpoints
/// and resuming execution.
///
/// Returns 0 on success, u32::MAX on error.
pub fn sys_debug_detach(tid: u32) -> u32 {
    let caller_tid = scheduler::current_tid();
    scheduler::debug_detach(caller_tid, tid)
}

// =========================================================================
// SYS_DEBUG_SUSPEND (302) — Suspend a debug-attached thread
// =========================================================================

/// Suspend the target thread (must be attached by caller).
///
/// Returns 0 on success, u32::MAX on error.
pub fn sys_debug_suspend(tid: u32) -> u32 {
    let caller_tid = scheduler::current_tid();
    scheduler::debug_suspend(caller_tid, tid)
}

// =========================================================================
// SYS_DEBUG_RESUME (303) — Resume a suspended debug-attached thread
// =========================================================================

/// Resume a suspended thread. If single_step is pending, RFLAGS.TF is set.
///
/// Returns 0 on success, u32::MAX on error.
pub fn sys_debug_resume(tid: u32) -> u32 {
    let caller_tid = scheduler::current_tid();
    scheduler::debug_resume(caller_tid, tid)
}

// =========================================================================
// SYS_DEBUG_GET_REGS (304) — Read target thread's register state
// =========================================================================

/// Copy the target thread's CpuContext into a user buffer.
///
/// Args: tid, buf_ptr (u64 address as two u32 halves packed in arg2/arg3),
///       size in bytes.
/// The buffer receives the first `min(size, 160)` bytes of the CpuContext
/// (16 GPRs + RIP + RSP + RFLAGS + CR3 = 20 * 8 = 160 bytes).
///
/// Returns number of bytes written, or u32::MAX on error.
pub fn sys_debug_get_regs(tid: u32, buf_lo: u32, buf_hi_and_size: u32) -> u32 {
    let caller_tid = scheduler::current_tid();
    // buf_lo = lower 32 bits of user buffer address
    // buf_hi_and_size: upper 16 bits = high bits of addr, lower 16 bits = size
    // Actually, syscall args are u32. For 64-bit pointers we use two args.
    // Convention: buf = (arg3 << 32) | arg2, size comes from the buffer itself.
    // Simpler: arg1=tid, arg2=buf_ptr (truncated to u32), arg3=size
    // Since user processes are in lower-half (<0x8000_0000_0000), u32 is enough
    // for most allocations. But mmap can be above 32-bit range.
    // Use the standard pattern: buf_ptr = arg2 as u64, size = arg3.
    let buf_ptr = buf_lo as u64;
    let size = buf_hi_and_size;

    if !is_valid_user_ptr(buf_ptr, size as u64) {
        return u32::MAX;
    }

    scheduler::debug_get_regs(caller_tid, tid, buf_ptr, size)
}

// =========================================================================
// SYS_DEBUG_SET_REGS (305) — Write target thread's register state
// =========================================================================

/// Overwrite the target thread's CpuContext from a user buffer.
///
/// Returns 0 on success, u32::MAX on error.
pub fn sys_debug_set_regs(tid: u32, buf_ptr: u32, size: u32) -> u32 {
    let caller_tid = scheduler::current_tid();
    let buf = buf_ptr as u64;

    if !is_valid_user_ptr(buf, size as u64) {
        return u32::MAX;
    }

    scheduler::debug_set_regs(caller_tid, tid, buf, size)
}

// =========================================================================
// SYS_DEBUG_READ_MEM (306) — Read memory from target process
// =========================================================================

/// Read `size` bytes from the target thread's address space at `addr`.
///
/// Args: tid, addr (u32 — lower 32 bits), size, buf_ptr.
/// Uses CR3-switch pattern for cross-process memory access.
///
/// Returns number of bytes read, or u32::MAX on error.
pub fn sys_debug_read_mem(tid: u32, addr: u32, size: u32, buf_ptr: u32) -> u32 {
    let caller_tid = scheduler::current_tid();
    let target_addr = addr as u64;
    let buf = buf_ptr as u64;
    let len = size.min(4096); // Max 4 KiB per call

    if !is_valid_user_ptr(buf, len as u64) {
        return u32::MAX;
    }

    scheduler::debug_read_mem(caller_tid, tid, target_addr, buf, len)
}

// =========================================================================
// SYS_DEBUG_WRITE_MEM (307) — Write memory in target process
// =========================================================================

/// Write up to 256 bytes into the target thread's address space.
///
/// Returns number of bytes written, or u32::MAX on error.
pub fn sys_debug_write_mem(tid: u32, addr: u32, size: u32, buf_ptr: u32) -> u32 {
    let caller_tid = scheduler::current_tid();
    let target_addr = addr as u64;
    let buf = buf_ptr as u64;
    let len = size.min(256); // Max 256 bytes per call

    if !is_valid_user_ptr(buf, len as u64) {
        return u32::MAX;
    }

    scheduler::debug_write_mem(caller_tid, tid, target_addr, buf, len)
}

// =========================================================================
// SYS_DEBUG_SET_BREAKPOINT (308) — Set a software breakpoint
// =========================================================================

/// Set a software breakpoint at the given address in the target's address space.
/// Reads the original byte, replaces it with INT3 (0xCC).
///
/// Args: tid, addr (u32 — lower 32 bits of virtual address).
/// Returns 0 on success, u32::MAX on error.
pub fn sys_debug_set_breakpoint(tid: u32, addr: u32) -> u32 {
    let caller_tid = scheduler::current_tid();
    scheduler::debug_set_breakpoint(caller_tid, tid, addr as u64)
}

// =========================================================================
// SYS_DEBUG_CLR_BREAKPOINT (309) — Clear a software breakpoint
// =========================================================================

/// Remove a software breakpoint, restoring the original byte.
///
/// Returns 0 on success, u32::MAX on error.
pub fn sys_debug_clr_breakpoint(tid: u32, addr: u32) -> u32 {
    let caller_tid = scheduler::current_tid();
    scheduler::debug_clr_breakpoint(caller_tid, tid, addr as u64)
}

// =========================================================================
// SYS_DEBUG_SINGLE_STEP (310) — Single-step one instruction
// =========================================================================

/// Mark the target for single-step: on resume, RFLAGS.TF will be set
/// so the CPU executes exactly one instruction and triggers #DB.
///
/// Returns 0 on success, u32::MAX on error.
pub fn sys_debug_single_step(tid: u32) -> u32 {
    let caller_tid = scheduler::current_tid();
    scheduler::debug_single_step(caller_tid, tid)
}

// =========================================================================
// SYS_DEBUG_GET_MEM_MAP (311) — Get target's virtual memory map
// =========================================================================

/// Walk the target's page tables (PML4→PDPT→PD→PT) and return an array
/// of contiguous memory regions with their flags.
///
/// Each entry: (start_addr: u64, end_addr: u64, flags: u64) = 24 bytes.
/// Returns number of regions written, or u32::MAX on error.
pub fn sys_debug_get_mem_map(tid: u32, buf_ptr: u32, size: u32) -> u32 {
    let caller_tid = scheduler::current_tid();
    let buf = buf_ptr as u64;

    if !is_valid_user_ptr(buf, size as u64) {
        return u32::MAX;
    }

    scheduler::debug_get_mem_map(caller_tid, tid, buf, size)
}

// =========================================================================
// SYS_DEBUG_WAIT_EVENT (312) — Poll for debug events
// =========================================================================

/// Non-blocking poll: check if the target has a pending debug event.
///
/// If an event exists, writes (event_type: u32, addr: u64) = 12 bytes
/// to the user buffer and clears the event.
///
/// Returns the event type (1=breakpoint, 2=single_step, 3=exit),
/// or 0 if no event pending, or u32::MAX on error.
pub fn sys_debug_wait_event(tid: u32, buf_ptr: u32, size: u32) -> u32 {
    let caller_tid = scheduler::current_tid();
    let buf = buf_ptr as u64;

    if buf != 0 && !is_valid_user_ptr(buf, size as u64) {
        return u32::MAX;
    }

    scheduler::debug_wait_event(caller_tid, tid, buf, size)
}

// =========================================================================
// SYS_THREAD_INFO_EX (313) — Extended thread information
// =========================================================================

/// Retrieve extended information about a thread: parent_tid, cwd,
/// fd_count, brk, mmap_next, last_cpu, rip, rsp, cr3.
///
/// Returns number of bytes written, or u32::MAX on error.
pub fn sys_thread_info_ex(tid: u32, buf_ptr: u32, size: u32) -> u32 {
    let buf = buf_ptr as u64;

    if !is_valid_user_ptr(buf, size as u64) {
        return u32::MAX;
    }

    scheduler::thread_info_ex(tid, buf, size)
}
