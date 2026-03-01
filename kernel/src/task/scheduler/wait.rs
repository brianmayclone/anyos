//! Waiting / sleeping: waitpid, sleep_until, block_current_thread.

use super::{get_cpu_id, SCHEDULER, schedule};
use crate::task::thread::ThreadState;

/// Wait for a thread to terminate and return its exit code.
pub fn waitpid(tid: u32) -> u32 {
    {
        crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_WAITPID);
        let mut guard = SCHEDULER.lock();
        let cpu_id = get_cpu_id();
        let sched = guard.as_mut().expect("Scheduler not initialized");
        if let Some(target) = sched.threads.iter_mut().find(|t| t.tid == tid) {
            if target.state == ThreadState::Terminated {
                let code = target.exit_code.unwrap_or(0);
                target.exit_code = None;
                return code;
            }
        } else { return u32::MAX; }
        if let Some(current_tid) = sched.per_cpu[cpu_id].current_tid {
            if let Some(target) = sched.threads.iter_mut().find(|t| t.tid == tid) {
                target.waiting_tid = Some(current_tid);
            }
            if let Some(idx) = sched.current_idx(cpu_id) {
                // CRITICAL: Mark context as unsaved BEFORE setting Blocked.
                // Without this, another CPU can wake this thread (→ Ready)
                // and load its stale saved context while we're still
                // physically executing on its stack → two CPUs on same stack → crash.
                sched.threads[idx].context.save_complete = 0;
                sched.threads[idx].state = ThreadState::Blocked;
            }
        }
    }
    // Yield immediately instead of waiting up to 1ms for timer preemption.
    schedule();
    loop {
        { crate::arch::hal::enable_interrupts(); crate::arch::hal::halt(); }
        {
            crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_WAITPID);
            let mut guard = SCHEDULER.lock();
            if let Some(sched) = guard.as_mut() {
                if let Some(target) = sched.threads.iter_mut().find(|t| t.tid == tid) {
                    if target.state == ThreadState::Terminated {
                        let code = target.exit_code.unwrap_or(0);
                        target.exit_code = None;
                        return code;
                    }
                } else { return u32::MAX; }
            }
        }
    }
}

/// Wait for ANY child of the current thread to terminate.
/// Returns (child_tid, exit_code), or (u32::MAX, u32::MAX) if no children.
pub fn waitpid_any() -> (u32, u32) {
    let current_tid;
    {
        crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_WAITPID_ANY);
        let mut guard = SCHEDULER.lock();
        let cpu_id = get_cpu_id();
        let sched = guard.as_mut().expect("Scheduler not initialized");
        current_tid = match sched.per_cpu[cpu_id].current_tid {
            Some(t) => t,
            None => return (u32::MAX, u32::MAX),
        };

        // Check for already-terminated children
        if let Some(child_idx) = sched.threads.iter().position(|t|
            t.parent_tid == current_tid && t.state == ThreadState::Terminated
                && t.exit_code.is_some()
        ) {
            let child_tid = sched.threads[child_idx].tid;
            let code = sched.threads[child_idx].exit_code.unwrap_or(0);
            sched.threads[child_idx].exit_code = None;
            return (child_tid, code);
        }

        // Check if any children exist at all
        let has_children = sched.threads.iter().any(|t| t.parent_tid == current_tid);
        if !has_children {
            return (u32::MAX, u32::MAX);
        }

        // Set waiting_tid on all non-terminated children so exit_current wakes us
        for t in sched.threads.iter_mut() {
            if t.parent_tid == current_tid && t.state != ThreadState::Terminated {
                t.waiting_tid = Some(current_tid);
            }
        }

        // Block current thread
        if let Some(idx) = sched.current_idx(get_cpu_id()) {
            sched.threads[idx].context.save_complete = 0;
            sched.threads[idx].state = ThreadState::Blocked;
        }
    }
    // Yield immediately instead of waiting up to 1ms for timer preemption.
    schedule();
    loop {
        { crate::arch::hal::enable_interrupts(); crate::arch::hal::halt(); }
        {
            crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_WAITPID_ANY);
            let mut guard = SCHEDULER.lock();
            if let Some(sched) = guard.as_mut() {
                if let Some(child_idx) = sched.threads.iter().position(|t|
                    t.parent_tid == current_tid && t.state == ThreadState::Terminated
                        && t.exit_code.is_some()
                ) {
                    let child_tid = sched.threads[child_idx].tid;
                    let code = sched.threads[child_idx].exit_code.unwrap_or(0);
                    sched.threads[child_idx].exit_code = None;
                    return (child_tid, code);
                }
                // No children at all → ECHILD
                let has_children = sched.threads.iter().any(|t| t.parent_tid == current_tid);
                if !has_children {
                    return (u32::MAX, u32::MAX);
                }
            }
        }
    }
}

/// Non-blocking wait for any child (used by WNOHANG).
/// Returns (child_tid, exit_code), or (u32::MAX-1, u32::MAX-1) if children exist but none terminated.
pub fn try_waitpid_any() -> (u32, u32) {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_TRY_WAITPID_ANY);
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    let current_tid = match sched.per_cpu[get_cpu_id()].current_tid {
        Some(t) => t,
        None => return (u32::MAX, u32::MAX),
    };
    if let Some(child_idx) = sched.threads.iter().position(|t|
        t.parent_tid == current_tid && t.state == ThreadState::Terminated
            && t.exit_code.is_some()
    ) {
        let child_tid = sched.threads[child_idx].tid;
        let code = sched.threads[child_idx].exit_code.unwrap_or(0);
        sched.threads[child_idx].exit_code = None;
        // Mark waiting so reaper doesn't reclaim before caller checks
        sched.threads[child_idx].waiting_tid = Some(current_tid);
        return (child_tid, code);
    }
    let has_children = sched.threads.iter().any(|t| t.parent_tid == current_tid);
    if !has_children {
        (u32::MAX, u32::MAX)
    } else {
        (u32::MAX - 1, u32::MAX - 1) // STILL_RUNNING equivalent
    }
}

/// Non-blocking check if a thread has terminated.
/// Also marks the target with `waiting_tid` so the auto-reaper won't
/// discard the exit code before the caller can retrieve it.
pub fn try_waitpid(tid: u32) -> u32 {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_TRY_WAITPID);
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    let caller_tid = sched.per_cpu[get_cpu_id()].current_tid.unwrap_or(0);
    if let Some(target) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        if target.state == ThreadState::Terminated {
            let code = target.exit_code.unwrap_or(0);
            target.exit_code = None;
            return code;
        }
        // Mark that someone is polling for this thread's exit —
        // prevents auto-reap from discarding the exit code.
        if target.waiting_tid.is_none() {
            target.waiting_tid = Some(caller_tid);
        }
        return u32::MAX - 1; // Still running
    }
    u32::MAX // Not found
}

/// Block the current thread until the given PIT tick is reached.
pub fn sleep_until(wake_at: u32) {
    {
        crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_SLEEP_UNTIL);
        let mut guard = SCHEDULER.lock();
        let cpu_id = get_cpu_id();
        let sched = guard.as_mut().expect("Scheduler not initialized");
        if let Some(idx) = sched.current_idx(cpu_id) {
            // CRITICAL: Mark context as unsaved before Blocked (same race as waitpid).
            sched.threads[idx].context.save_complete = 0;
            sched.threads[idx].wake_at_tick = Some(wake_at);
            sched.threads[idx].state = ThreadState::Blocked;
        }
    }
    schedule();
}

/// Block the current thread unconditionally (no wake condition).
pub fn block_current_thread() {
    {
        crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_BLOCK_CURRENT);
        let mut guard = SCHEDULER.lock();
        let cpu_id = get_cpu_id();
        let sched = guard.as_mut().expect("Scheduler not initialized");
        if let Some(idx) = sched.current_idx(cpu_id) {
            // CRITICAL: Mark context as unsaved before Blocked (same race as waitpid).
            sched.threads[idx].context.save_complete = 0;
            sched.threads[idx].state = ThreadState::Blocked;
        }
    }
    schedule();
}
