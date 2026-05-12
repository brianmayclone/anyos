//! Per-process FD table helpers.

use super::{get_cpu_id, SCHEDULER};
use crate::fs::fd_table::{FdEntry, FdKind, FdTable, MAX_FDS};
use alloc::vec::Vec;

/// Allocate an FD in the current thread's FD table.
pub fn current_fd_alloc(kind: FdKind) -> Option<u32> {
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut()?;
    let cpu = get_cpu_id();
    let idx = sched.current_idx(cpu)?;
    sched.threads[idx].fd_table.alloc(kind)
}

/// Close an FD in the current thread's FD table.
/// Returns the old FdKind for cleanup (decref, etc.), or None if invalid.
pub fn current_fd_close(fd: u32) -> Option<FdKind> {
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut()?;
    let cpu = get_cpu_id();
    let idx = sched.current_idx(cpu)?;
    sched.threads[idx].fd_table.close(fd)
}

/// Look up an FD in the current thread's FD table.
pub fn current_fd_get(fd: u32) -> Option<FdEntry> {
    let guard = SCHEDULER.lock();
    let sched = guard.as_ref()?;
    let cpu = get_cpu_id();
    let idx = sched.current_idx(cpu)?;
    sched.threads[idx].fd_table.get(fd).copied()
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
    thread
        .fd_table
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
    thread.fd_table.get(fd).copied()
}

/// Duplicate old_fd to new_fd in the current thread's FD table.
/// Caller must handle closing new_fd first and incrementing refcounts.
pub fn current_fd_dup2(old_fd: u32, new_fd: u32) -> bool {
    let mut guard = SCHEDULER.lock();
    let sched = match guard.as_mut() {
        Some(s) => s,
        None => return false,
    };
    let cpu = get_cpu_id();
    let idx = match sched.current_idx(cpu) {
        Some(i) => i,
        None => return false,
    };
    sched.threads[idx].fd_table.dup2(old_fd, new_fd)
}

/// Allocate the lowest FD >= min_fd in the current thread's FD table.
pub fn current_fd_alloc_above(min_fd: u32, kind: FdKind) -> Option<u32> {
    let mut guard = SCHEDULER.lock();
    let sched = match guard.as_mut() {
        Some(s) => s,
        None => return None,
    };
    let cpu = get_cpu_id();
    let idx = match sched.current_idx(cpu) {
        Some(i) => i,
        None => return None,
    };
    sched.threads[idx].fd_table.alloc_above(min_fd, kind)
}

/// Allocate an FD at a specific slot in the current thread's FD table.
pub fn current_fd_alloc_at(fd: u32, kind: FdKind) -> bool {
    let mut guard = SCHEDULER.lock();
    let sched = match guard.as_mut() {
        Some(s) => s,
        None => return false,
    };
    let cpu = get_cpu_id();
    let idx = match sched.current_idx(cpu) {
        Some(i) => i,
        None => return false,
    };
    sched.threads[idx].fd_table.alloc_at(fd, kind)
}

/// Set or clear the CLOEXEC flag on an FD in the current thread's FD table.
pub fn current_fd_set_cloexec(fd: u32, cloexec: bool) {
    let mut guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_mut() {
        let cpu = get_cpu_id();
        if let Some(idx) = sched.current_idx(cpu) {
            sched.threads[idx].fd_table.set_cloexec(fd, cloexec);
        }
    }
}

/// Set or clear O_NONBLOCK on an FD in the current thread's FD table.
pub fn current_fd_set_nonblock(fd: u32, nonblock: bool) {
    let mut guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_mut() {
        let cpu = get_cpu_id();
        if let Some(idx) = sched.current_idx(cpu) {
            sched.threads[idx].fd_table.set_nonblock(fd, nonblock);
        }
    }
}

/// Update the cursor for a licof Linux proc pseudo-file in the current FD table.
pub fn current_fd_set_linux_proc_position(fd: u32, position: u32) -> bool {
    let mut guard = SCHEDULER.lock();
    let sched = match guard.as_mut() {
        Some(sched) => sched,
        None => return false,
    };
    let cpu = get_cpu_id();
    let idx = match sched.current_idx(cpu) {
        Some(idx) => idx,
        None => return false,
    };
    sched.threads[idx]
        .fd_table
        .set_linux_proc_position(fd, position)
}

/// Set the FD table on a thread (for fork child setup).
pub fn set_thread_fd_table(tid: u32, table: FdTable) {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_GET_THREAD_INFO);
    let mut guard = SCHEDULER.lock();
    let sched = match guard.as_mut() {
        Some(s) => s,
        None => return,
    };
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.fd_table = table;
    }
}

/// Close all FDs in the current thread's FD table. Returns old FdKinds for cleanup.
pub fn current_fd_close_all() -> [FdKind; MAX_FDS] {
    let mut out = [FdKind::None; MAX_FDS];
    let mut guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_mut() {
        let cpu = get_cpu_id();
        if let Some(idx) = sched.current_idx(cpu) {
            sched.threads[idx].fd_table.close_all(&mut out);
        }
    }
    out
}

/// Close all CLOEXEC FDs in the current thread's FD table. Returns old FdKinds.
pub fn current_fd_close_cloexec() -> [FdKind; MAX_FDS] {
    let mut out = [FdKind::None; MAX_FDS];
    let mut guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_mut() {
        let cpu = get_cpu_id();
        if let Some(idx) = sched.current_idx(cpu) {
            sched.threads[idx].fd_table.close_cloexec(&mut out);
        }
    }
    out
}

/// Close all FDs for a specific thread (by TID). Returns old FdKinds for cleanup.
/// Used during sys_exit before destroying the page directory.
pub fn close_all_fds_for_thread(tid: u32) -> [FdKind; MAX_FDS] {
    let mut out = [FdKind::None; MAX_FDS];
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_GET_THREAD_INFO);
    let mut guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_mut() {
        if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
            thread.fd_table.close_all(&mut out);
        }
    }
    out
}
