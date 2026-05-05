//! Waiting / sleeping: waitpid, sleep_until, block_current_thread.

use super::{get_cpu_id, schedule, SCHEDULER};
use crate::task::thread::ThreadState;

#[inline]
fn consume_exit_status(thread: &mut crate::task::thread::Thread) -> u32 {
    let code = thread.exit_code.unwrap_or(0);
    thread.exit_code = None;
    thread.exit_waiter_tid = None;
    thread.retain_exit_status = false;
    code
}

/// Wait for a thread to terminate and return its exit code.
pub fn waitpid(tid: u32) -> u32 {
    {
        crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_WAITPID);
        let mut guard = SCHEDULER.lock();
        let cpu_id = get_cpu_id();
        let sched = match guard.as_mut() {
            Some(s) => s,
            None => return u32::MAX,
        };
        if let Some(target) = sched.threads.iter_mut().find(|t| t.tid == tid) {
            if target.state == ThreadState::Terminated {
                return consume_exit_status(target);
            }
        } else {
            return u32::MAX;
        }
        if let Some(current_tid) = sched.per_cpu[cpu_id].current_tid {
            if let Some(target) = sched.threads.iter_mut().find(|t| t.tid == tid) {
                target.exit_waiter_tid = Some(current_tid);
            }
            if let Some(idx) = sched.current_idx(cpu_id) {
                // CRITICAL: Set Blocked FIRST, then clear save_complete.
                // pick_eligible checks state==Ready first, so once Blocked
                // no other CPU will attempt to run this thread — even if
                // save_complete is momentarily stale.  The old order
                // (save_complete=0 then Blocked) left a window where the
                // thread was Ready with save_complete=0, allowing another
                // CPU to re-enqueue and potentially load a partially-saved
                // context.
                sched.threads[idx].last_cpu = cpu_id;
                sched.threads[idx].wake_at_tick = None;
                sched.threads[idx].state = ThreadState::Blocked;
                sched.threads[idx].context.save_complete = 0;
            }
        }
    }
    // Yield immediately instead of waiting up to 1ms for timer preemption.
    schedule();
    loop {
        {
            crate::arch::hal::enable_interrupts();
            crate::arch::hal::halt();
        }
        {
            crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_WAITPID);
            let mut guard = SCHEDULER.lock();
            if let Some(sched) = guard.as_mut() {
                if let Some(target) = sched.threads.iter_mut().find(|t| t.tid == tid) {
                    if target.state == ThreadState::Terminated {
                        return consume_exit_status(target);
                    }
                } else {
                    return u32::MAX;
                }
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
        let sched = match guard.as_mut() {
            Some(s) => s,
            None => return (u32::MAX, u32::MAX),
        };
        current_tid = match sched.per_cpu[cpu_id].current_tid {
            Some(t) => t,
            None => return (u32::MAX, u32::MAX),
        };

        // Check for already-terminated children
        if let Some(child_idx) = sched.threads.iter().position(|t| {
            t.parent_tid == current_tid
                && t.state == ThreadState::Terminated
                && t.exit_code.is_some()
        }) {
            let child_tid = sched.threads[child_idx].tid;
            let code = consume_exit_status(&mut sched.threads[child_idx]);
            return (child_tid, code);
        }

        // Check if any children exist at all
        let has_children = sched.threads.iter().any(|t| t.parent_tid == current_tid);
        if !has_children {
            return (u32::MAX, u32::MAX);
        }

        // Set blocking waiters on all non-terminated children so exit_current wakes us.
        for t in sched.threads.iter_mut() {
            if t.parent_tid == current_tid && t.state != ThreadState::Terminated {
                t.exit_waiter_tid = Some(current_tid);
            }
        }

        // Block current thread — state=Blocked first so no CPU picks it,
        // then clear save_complete (see waitpid for detailed rationale).
        if let Some(idx) = sched.current_idx(get_cpu_id()) {
            sched.threads[idx].last_cpu = get_cpu_id();
            sched.threads[idx].wake_at_tick = None;
            sched.threads[idx].state = ThreadState::Blocked;
            sched.threads[idx].context.save_complete = 0;
        }
    }
    // Yield immediately instead of waiting up to 1ms for timer preemption.
    schedule();
    loop {
        {
            crate::arch::hal::enable_interrupts();
            crate::arch::hal::halt();
        }
        {
            crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_WAITPID_ANY);
            let mut guard = SCHEDULER.lock();
            if let Some(sched) = guard.as_mut() {
                if let Some(child_idx) = sched.threads.iter().position(|t| {
                    t.parent_tid == current_tid
                        && t.state == ThreadState::Terminated
                        && t.exit_code.is_some()
                }) {
                    let child_tid = sched.threads[child_idx].tid;
                    let code = consume_exit_status(&mut sched.threads[child_idx]);
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
    let sched = match guard.as_mut() {
        Some(s) => s,
        None => return (u32::MAX, u32::MAX),
    };
    let current_tid = match sched.per_cpu[get_cpu_id()].current_tid {
        Some(t) => t,
        None => return (u32::MAX, u32::MAX),
    };
    if let Some(child_idx) = sched.threads.iter().position(|t| {
        t.parent_tid == current_tid && t.state == ThreadState::Terminated && t.exit_code.is_some()
    }) {
        let child_tid = sched.threads[child_idx].tid;
        let code = consume_exit_status(&mut sched.threads[child_idx]);
        return (child_tid, code);
    }
    let has_children = sched.threads.iter().any(|t| t.parent_tid == current_tid);
    if !has_children {
        (u32::MAX, u32::MAX)
    } else {
        for t in sched.threads.iter_mut() {
            if t.parent_tid == current_tid && t.state != ThreadState::Terminated {
                t.retain_exit_status = true;
            }
        }
        (u32::MAX - 1, u32::MAX - 1) // STILL_RUNNING equivalent
    }
}

/// Non-blocking check if a thread has terminated.
/// Also marks the target as retainable so the auto-reaper won't
/// discard the exit code before the caller can retrieve it.
pub fn try_waitpid(tid: u32) -> u32 {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_TRY_WAITPID);
    let mut guard = SCHEDULER.lock();
    let sched = match guard.as_mut() {
        Some(s) => s,
        None => return u32::MAX,
    };
    if let Some(target) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        if target.state == ThreadState::Terminated {
            return consume_exit_status(target);
        }
        if target.state == ThreadState::Stopped {
            return u32::MAX - 2; // Stopped by signal
        }
        target.retain_exit_status = true;
        return u32::MAX - 1; // Still running
    }
    u32::MAX // Not found
}

/// Mark the current thread Blocked with a wake deadline, but do NOT
/// invoke the scheduler. The caller must call [`schedule`] afterwards,
/// after dropping any subsystem locks.
///
/// This exists to close the missed-wakeup race in subsystems like
/// `tcp::recv`: the thread can publish its `waiting_tid` AND transition
/// to `Blocked` while still holding the subsystem lock, so an IRQ-driven
/// `try_wake_thread` call (which serializes through that same lock)
/// cannot observe a Ready thread.
pub fn prepare_to_block_until(wake_at: u32) {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_SLEEP_UNTIL);
    let mut guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    let sched = match guard.as_mut() {
        Some(s) => s,
        None => return,
    };
    if let Some(idx) = sched.current_idx(cpu_id) {
        sched.threads[idx].wake_at_tick = Some(wake_at);
        sched.threads[idx].last_cpu = cpu_id;
        sched.threads[idx].state = ThreadState::Blocked;
        sched.threads[idx].context.save_complete = 0;
        super::note_sleeper_deadline(wake_at);
    }
}

/// Block the current thread until the given PIT tick is reached.
pub fn sleep_until(wake_at: u32) {
    {
        crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_SLEEP_UNTIL);
        let mut guard = SCHEDULER.lock();
        let cpu_id = get_cpu_id();
        let sched = match guard.as_mut() {
            Some(s) => s,
            None => return,
        };
        if let Some(idx) = sched.current_idx(cpu_id) {
            // CRITICAL: Set Blocked first, then clear save_complete
            // (same rationale as waitpid — no window where state==Ready
            // with save_complete==0).
            sched.threads[idx].wake_at_tick = Some(wake_at);
            sched.threads[idx].last_cpu = cpu_id;
            sched.threads[idx].state = ThreadState::Blocked;
            sched.threads[idx].context.save_complete = 0;
            super::note_sleeper_deadline(wake_at);
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
        let sched = match guard.as_mut() {
            Some(s) => s,
            None => return,
        };
        if let Some(idx) = sched.current_idx(cpu_id) {
            // CRITICAL: Set Blocked first, then clear save_complete
            // (same rationale as waitpid).
            sched.threads[idx].last_cpu = cpu_id;
            sched.threads[idx].wake_at_tick = None;
            sched.threads[idx].state = ThreadState::Blocked;
            sched.threads[idx].context.save_complete = 0;
        }
    }
    schedule();
}
