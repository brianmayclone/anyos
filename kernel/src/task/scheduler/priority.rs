//! Priority, wake, critical-thread, capability, and identity management.

use super::{get_cpu_id, clamp_priority, SCHEDULER};

/// Set the priority of a thread by TID (clamped to 0–127).
pub fn set_thread_priority(tid: u32, priority: u8) {
    let priority = clamp_priority(priority, "set_thread_priority");
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_SET_THREAD_PRIORITY);
    let mut guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_mut() {
        if let Some(idx) = sched.find_idx(tid) {
            sched.threads[idx].priority = priority;
        }
    }
}

/// Wake a blocked thread by TID.
pub fn wake_thread(tid: u32) {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_WAKE_THREAD);
    let mut guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_mut() {
        sched.wake_thread_inner(tid);
    }
}

/// Try to wake a blocked thread by TID (non-blocking).
///
/// Uses `try_lock()` to avoid spinning on the SCHEDULER lock. Returns `true`
/// if the thread was woken, `false` if the lock was contended (caller should
/// retry later or use the deferred-wake mechanism).
///
/// Safe to call from IRQ context — never blocks.
pub fn try_wake_thread(tid: u32) -> bool {
    if let Some(mut guard) = SCHEDULER.try_lock() {
        if let Some(sched) = guard.as_mut() {
            sched.wake_thread_inner(tid);
        }
        true
    } else {
        false
    }
}

/// Mark a thread as critical (will not be killed by RSP recovery).
pub fn set_thread_critical(tid: u32) {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_GET_THREAD_INFO);
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.critical = true;
        crate::serial_println!("  Thread '{}' (TID={}) marked as critical", thread.name_str(), tid);
    }
}

/// Get the capability bitmask for the currently running thread.
pub fn current_thread_capabilities() -> crate::task::capabilities::CapSet {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_GET_THREAD_INFO);
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    let sched = guard.as_ref().expect("Scheduler not initialized");
    if let Some(idx) = sched.current_idx(cpu_id) {
        return sched.threads[idx].capabilities;
    }
    0
}

/// Set the capability bitmask for a thread (called by loader after spawn).
pub fn set_thread_capabilities(tid: u32, caps: crate::task::capabilities::CapSet) {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_GET_THREAD_INFO);
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.capabilities = caps;
    }
}

/// Get the user ID of the currently running thread.
pub fn current_thread_uid() -> u16 {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_GET_THREAD_INFO);
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    let sched = guard.as_ref().expect("Scheduler not initialized");
    if let Some(idx) = sched.current_idx(cpu_id) {
        return sched.threads[idx].uid;
    }
    0
}

/// Get the group ID of the currently running thread.
pub fn current_thread_gid() -> u16 {
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    let sched = guard.as_ref().expect("Scheduler not initialized");
    if let Some(idx) = sched.current_idx(cpu_id) {
        return sched.threads[idx].gid;
    }
    0
}

/// Set the user and group IDs for a specific thread.
pub fn set_thread_identity(tid: u32, uid: u16, gid: u16) {
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.uid = uid;
        thread.gid = gid;
    }
}

/// Set uid/gid on ALL threads that share the same page_directory as the given thread.
/// Used by SYS_AUTHENTICATE to propagate identity to all threads in a process.
pub fn set_process_identity(tid: u32, uid: u16, gid: u16) {
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    // Find the page directory of the target thread
    let pd = {
        let thread = match sched.threads.iter().find(|t| t.tid == tid) {
            Some(t) => t,
            None => return,
        };
        thread.page_directory
    };
    // Update all threads sharing the same PD (same process)
    for thread in sched.threads.iter_mut() {
        if thread.page_directory == pd {
            thread.uid = uid;
            thread.gid = gid;
        }
    }
}
