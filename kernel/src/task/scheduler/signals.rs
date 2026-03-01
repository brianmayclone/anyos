//! Signal helpers, parent/child TID management, thread existence checks.

use super::{get_cpu_id, SCHEDULER};
use crate::task::thread::ThreadState;
use alloc::vec::Vec;

/// Send a signal to a thread by TID. Returns true if the thread exists.
pub fn send_signal_to_thread(tid: u32, sig: u32) -> bool {
    let mut guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_mut() {
        if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
            thread.signals.send(sig);
            return true;
        }
    }
    false
}

/// Dequeue the lowest-numbered pending, unblocked signal for the current thread.
pub fn current_signal_dequeue() -> Option<u32> {
    let mut guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_mut() {
        let cpu = get_cpu_id();
        if let Some(idx) = sched.current_idx(cpu) {
            return sched.threads[idx].signals.dequeue();
        }
    }
    None
}

/// Get the handler address for a signal on the current thread.
pub fn current_signal_handler(sig: u32) -> u64 {
    let guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_ref() {
        let cpu = get_cpu_id();
        if let Some(idx) = sched.current_idx(cpu) {
            return sched.threads[idx].signals.get_handler(sig);
        }
    }
    crate::ipc::signal::SIG_DFL
}

/// Set a signal handler on the current thread. Returns the old handler.
pub fn current_signal_set_handler(sig: u32, handler: u64) -> u64 {
    let mut guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_mut() {
        let cpu = get_cpu_id();
        if let Some(idx) = sched.current_idx(cpu) {
            return sched.threads[idx].signals.set_handler(sig, handler);
        }
    }
    crate::ipc::signal::SIG_DFL
}

/// Get the current thread's blocked signal mask.
pub fn current_signal_get_blocked() -> u32 {
    let guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_ref() {
        let cpu = get_cpu_id();
        if let Some(idx) = sched.current_idx(cpu) {
            return sched.threads[idx].signals.blocked;
        }
    }
    0
}

/// Set the current thread's blocked signal mask. Returns the old mask.
pub fn current_signal_set_blocked(new_mask: u32) -> u32 {
    let mut guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_mut() {
        let cpu = get_cpu_id();
        if let Some(idx) = sched.current_idx(cpu) {
            let old = sched.threads[idx].signals.blocked;
            sched.threads[idx].signals.blocked = new_mask;
            return old;
        }
    }
    0
}

/// Check if the current thread has any pending, unblocked signals.
pub fn current_has_pending_signal() -> bool {
    let guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_ref() {
        let cpu = get_cpu_id();
        if let Some(idx) = sched.current_idx(cpu) {
            return sched.threads[idx].signals.has_pending();
        }
    }
    false
}

/// Set parent_tid on a thread (for fork/spawn child).
pub fn set_thread_parent_tid(tid: u32, parent: u32) {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_GET_THREAD_INFO);
    let mut guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_mut() {
        if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
            thread.parent_tid = parent;
        }
    }
}

/// Get the current thread's parent TID.
pub fn current_parent_tid() -> u32 {
    let guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_ref() {
        let cpu = get_cpu_id();
        if let Some(idx) = sched.current_idx(cpu) {
            return sched.threads[idx].parent_tid;
        }
    }
    0
}

/// Set signal state on a thread (for fork child — inherits handler table, clears pending).
pub fn set_thread_signals(tid: u32, signals: crate::ipc::signal::SignalState) {
    let mut guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_mut() {
        if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
            // Fork child inherits handler table and blocked mask, but pending signals are cleared
            thread.signals.handlers = signals.handlers;
            thread.signals.blocked = signals.blocked;
            thread.signals.pending = 0;
        }
    }
}

/// Check if a thread exists (for kill(pid, 0) semantics).
pub fn thread_exists(tid: u32) -> bool {
    let guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_ref() {
        return sched.threads.iter().any(|t| t.tid == tid && t.state != ThreadState::Terminated);
    }
    false
}

/// Get the parent_tid for a specific thread (for SIGCHLD delivery on exit).
pub fn get_thread_parent_tid(tid: u32) -> u32 {
    let guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_ref() {
        if let Some(thread) = sched.threads.iter().find(|t| t.tid == tid) {
            return thread.parent_tid;
        }
    }
    0
}

/// Collect TIDs of all live non-idle threads (for system shutdown).
///
/// Returns a Vec of TIDs for threads that are not idle and not yet terminated.
pub fn all_live_tids() -> Vec<u32> {
    let guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_ref() {
        sched.threads.iter()
            .filter(|t| !t.is_idle && t.state != ThreadState::Terminated)
            .map(|t| t.tid)
            .collect()
    } else {
        Vec::new()
    }
}
