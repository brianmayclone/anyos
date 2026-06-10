//! Per-process FD table helpers.
//!
//! The current-thread accessors reach the FD table through the per-CPU pointer
//! published at context switch (`with_current_fd_table`), so they do NOT take
//! the global scheduler lock — FD syscalls are the hottest LXE path and used
//! to serialize against scheduling on every CPU just to look up an FD. The
//! by-TID accessors still take the scheduler lock to find the thread, then
//! lock that thread's FD-table spinlock; they are cold (fork install, exit
//! cleanup) so the extra lock is irrelevant.

use super::{get_cpu_id, with_current_fd_table, SCHEDULER};
use crate::fs::fd_table::{FdEntry, FdKind, FdTable, MAX_FDS};
use alloc::vec::Vec;

/// Allocate an FD in the current thread's FD table.
pub fn current_fd_alloc(kind: FdKind) -> Option<u32> {
    with_current_fd_table(|t| t.alloc(kind)).flatten()
}

/// Close an FD in the current thread's FD table.
/// Returns the old FdKind for cleanup (decref, etc.), or None if invalid.
pub fn current_fd_close(fd: u32) -> Option<FdKind> {
    with_current_fd_table(|t| t.close(fd)).flatten()
}

/// Look up an FD in the current thread's FD table.
pub fn current_fd_get(fd: u32) -> Option<FdEntry> {
    with_current_fd_table(|t| t.get(fd).copied()).flatten()
}

/// Return the open file descriptor numbers for a thread.
pub fn thread_fd_numbers(tid: u32) -> Vec<u32> {
    let guard = SCHEDULER.lock();
    let Some(sched) = guard.as_ref() else {
        return Vec::new();
    };
    let Some(thread) = sched.threads.iter().find(|thread| thread.tid == tid) else {
        return Vec::new();
    };
    let table = thread.fd_table.lock();
    table
        .entries
        .iter()
        .enumerate()
        .filter_map(|(fd, entry)| {
            if matches!(entry.kind, FdKind::None) {
                None
            } else {
                Some(fd as u32)
            }
        })
        .collect()
}

/// Look up an FD in another thread's FD table.
pub fn thread_fd_get(tid: u32, fd: u32) -> Option<FdEntry> {
    let guard = SCHEDULER.lock();
    let sched = guard.as_ref()?;
    let thread = sched.threads.iter().find(|thread| thread.tid == tid)?;
    let entry = thread.fd_table.lock().get(fd).copied();
    entry
}

/// Duplicate old_fd to new_fd in the current thread's FD table.
/// Caller must handle closing new_fd first and incrementing refcounts.
pub fn current_fd_dup2(old_fd: u32, new_fd: u32) -> bool {
    with_current_fd_table(|t| t.dup2(old_fd, new_fd)).unwrap_or(false)
}

/// Allocate the lowest FD >= min_fd in the current thread's FD table.
pub fn current_fd_alloc_above(min_fd: u32, kind: FdKind) -> Option<u32> {
    with_current_fd_table(|t| t.alloc_above(min_fd, kind)).flatten()
}

/// Allocate an FD at a specific slot in the current thread's FD table.
pub fn current_fd_alloc_at(fd: u32, kind: FdKind) -> bool {
    with_current_fd_table(|t| t.alloc_at(fd, kind)).unwrap_or(false)
}

/// Set or clear the CLOEXEC flag on an FD in the current thread's FD table.
pub fn current_fd_set_cloexec(fd: u32, cloexec: bool) {
    with_current_fd_table(|t| t.set_cloexec(fd, cloexec));
}

/// Set or clear O_NONBLOCK on an FD in the current thread's FD table.
pub fn current_fd_set_nonblock(fd: u32, nonblock: bool) {
    with_current_fd_table(|t| t.set_nonblock(fd, nonblock));
}

/// Update the read/write cursor for an lxe Linux framebuffer FD.
pub fn current_fd_set_linux_fb_position(fd: u32, position: u32) -> bool {
    with_current_fd_table(|t| t.set_linux_fb_position(fd, position)).unwrap_or(false)
}

/// Update the cursor for a lxe Linux proc pseudo-file in the current FD table.
pub fn current_fd_set_linux_proc_position(fd: u32, position: u32) -> bool {
    with_current_fd_table(|t| t.set_linux_proc_position(fd, position)).unwrap_or(false)
}

/// Set the FD table on a thread (for fork child setup). The child is not yet
/// running, so replacing its `Arc` here is safe — the per-CPU pointer is only
/// published when the thread is first switched in.
pub fn set_thread_fd_table(tid: u32, table: FdTable) {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_GET_THREAD_INFO);
    let mut guard = SCHEDULER.lock();
    let sched = match guard.as_mut() {
        Some(s) => s,
        None => return,
    };
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.fd_table = alloc::sync::Arc::new(crate::sync::spinlock::Spinlock::new(table));
    }
}

/// Close all FDs in the current thread's FD table. Returns old FdKinds for cleanup.
pub fn current_fd_close_all() -> [FdKind; MAX_FDS] {
    let mut out = [FdKind::None; MAX_FDS];
    with_current_fd_table(|t| t.close_all(&mut out));
    out
}

/// Close all CLOEXEC FDs in the current thread's FD table. Returns old FdKinds.
pub fn current_fd_close_cloexec() -> [FdKind; MAX_FDS] {
    let mut out = [FdKind::None; MAX_FDS];
    with_current_fd_table(|t| t.close_cloexec(&mut out));
    out
}

/// Close all FDs for a specific thread (by TID). Returns old FdKinds for cleanup.
/// Used during sys_exit before destroying the page directory.
pub fn close_all_fds_for_thread(tid: u32) -> [FdKind; MAX_FDS] {
    let mut out = [FdKind::None; MAX_FDS];
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_GET_THREAD_INFO);
    let guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_ref() {
        if let Some(thread) = sched.threads.iter().find(|t| t.tid == tid) {
            thread.fd_table.lock().close_all(&mut out);
        }
    }
    out
}
