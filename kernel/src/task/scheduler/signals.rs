//! Signal helpers, parent/child TID management, thread existence checks.

use super::{get_cpu_id, schedule, SCHEDULER};
use crate::task::thread::ThreadState;
use alloc::vec::Vec;

/// Send a signal to a thread by TID. Returns true if the thread exists.
pub fn send_signal_to_thread(tid: u32, sig: u32) -> bool {
    use crate::ipc::signal::{SIGCONT, SIGSTOP, SIGTSTP, SIG_DFL};

    let mut guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_mut() {
        let idx = match sched.threads.iter().position(|t| t.tid == tid) {
            Some(i) => i,
            None => return false,
        };

        let handler = sched.threads[idx].signals.get_handler(sig);

        // SIGTSTP/SIGSTOP: stop the thread (default action)
        if (sig == SIGTSTP || sig == SIGSTOP) && handler == SIG_DFL {
            let state = sched.threads[idx].state;
            if state == ThreadState::Ready
                || state == ThreadState::Running
                || state == ThreadState::Blocked
            {
                // If the thread was sleeping (Blocked with wake_at_tick), save the
                // remaining ticks so SIGCONT can resume the sleep correctly.
                if state == ThreadState::Blocked {
                    if let Some(wake_tick) = sched.threads[idx].wake_at_tick {
                        let now = crate::arch::hal::timer_current_ticks();
                        let remaining = wake_tick.wrapping_sub(now);
                        // Only save if the sleep hasn't already expired
                        if remaining < 0x8000_0000 {
                            sched.threads[idx].sleep_remaining = remaining;
                        } else {
                            sched.threads[idx].sleep_remaining = 0;
                        }
                        sched.threads[idx].wake_at_tick = None;
                    }
                }
                sched.threads[idx].state = ThreadState::Stopped;
                crate::serial_verbose_println!(
                    "Signal {}: stopped T{} (was {:?})",
                    sig,
                    tid,
                    state
                );
                // Wake any waitpid waiter so it can see the stopped state
                if let Some(waiter_tid) = sched.threads[idx].exit_waiter_tid {
                    sched.wake_thread_inner(waiter_tid);
                }
            }
            return true;
        }

        // SIGCONT: resume a stopped thread
        if sig == SIGCONT {
            if sched.threads[idx].state == ThreadState::Stopped {
                // If the thread had remaining sleep time, restore it as Blocked
                // with a new wake_at_tick instead of making it immediately Ready.
                if sched.threads[idx].sleep_remaining > 0 {
                    let now = crate::arch::hal::timer_current_ticks();
                    sched.threads[idx].wake_at_tick =
                        Some(now.wrapping_add(sched.threads[idx].sleep_remaining));
                    sched.threads[idx].sleep_remaining = 0;
                    sched.threads[idx].state = ThreadState::Blocked;
                    crate::serial_verbose_println!(
                        "Signal {}: continued T{} (re-sleeping)",
                        sig,
                        tid
                    );
                } else {
                    sched.threads[idx].state = ThreadState::Ready;
                    let cpu = sched.threads[idx].affinity_cpu;
                    let n = sched.num_cpus();
                    let target = if cpu < n { cpu } else { 0 };
                    let priority = sched.threads[idx].priority;
                    sched.per_cpu[target].run_queue.enqueue(tid, priority);
                    crate::serial_verbose_println!("Signal {}: continued T{}", sig, tid);
                }
            }
            // SIGCONT also clears pending stop signals
            sched.threads[idx].signals.pending &= !((1 << SIGTSTP) | (1 << SIGSTOP));
            return true;
        }

        sched.threads[idx].signals.send(sig);
        return true;
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

/// Detach a child from its parent. Only succeeds if `parent_tid` matches.
/// Sets the child's parent_tid to 0 so it won't be cascade-killed.
/// Returns 0 on success, u32::MAX on error.
pub fn detach_child(parent_tid: u32, child_tid: u32) -> u32 {
    let mut guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_mut() {
        if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == child_tid) {
            if thread.parent_tid == parent_tid {
                thread.parent_tid = 0;
                return 0;
            }
        }
    }
    u32::MAX
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
        return sched
            .threads
            .iter()
            .any(|t| t.tid == tid && t.state != ThreadState::Terminated);
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

/// Stop the current thread (SIGTSTP/SIGSTOP default action).
/// Sets state to Stopped and yields the CPU. The thread will not be
/// scheduled again until it receives SIGCONT.
pub fn stop_current_thread(sig: u32) {
    {
        let mut guard = SCHEDULER.lock();
        let cpu_id = get_cpu_id();
        let sched = match guard.as_mut() {
            Some(s) => s,
            None => return,
        };
        if let Some(idx) = sched.current_idx(cpu_id) {
            crate::serial_verbose_println!(
                "Signal {}: stopping current T{}",
                sig,
                sched.threads[idx].tid
            );
            sched.threads[idx].state = ThreadState::Stopped;
            sched.threads[idx].context.save_complete = 0;
            // Wake any waitpid waiter so it can see the stopped state
            if let Some(waiter_tid) = sched.threads[idx].exit_waiter_tid {
                sched.wake_thread_inner(waiter_tid);
            }
        }
    }
    schedule();
}

/// Check if a thread is in Stopped state.
pub fn is_thread_stopped(tid: u32) -> bool {
    let guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_ref() {
        if let Some(thread) = sched.threads.iter().find(|t| t.tid == tid) {
            return thread.state == ThreadState::Stopped;
        }
    }
    false
}

/// Collect TIDs of all live non-idle threads (for system shutdown).
///
/// Returns a Vec of TIDs for threads that are not idle and not yet terminated.
pub fn all_live_tids() -> Vec<u32> {
    let guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_ref() {
        sched
            .threads
            .iter()
            .filter(|t| !t.is_idle && t.state != ThreadState::Terminated)
            .map(|t| t.tid)
            .collect()
    } else {
        Vec::new()
    }
}
