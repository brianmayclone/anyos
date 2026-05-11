//! Waiting / sleeping: waitpid, sleep_until, block_current_thread.

use super::{get_cpu_id, schedule, SCHEDULER};
use crate::task::thread::ThreadState;

#[derive(Clone, Copy)]
struct WaitThreadSnapshot {
    tid: u32,
    state: ThreadState,
    exit: u32,
    waiter: u32,
    retain: bool,
    last_cpu: usize,
    name: [u8; 32],
    args: [u8; 64],
    linux_fs_base: u64,
}

impl WaitThreadSnapshot {
    const fn empty() -> Self {
        Self {
            tid: 0,
            state: ThreadState::Terminated,
            exit: u32::MAX,
            waiter: 0,
            retain: false,
            last_cpu: 0,
            name: [0; 32],
            args: [0; 64],
            linux_fs_base: 0,
        }
    }
}

fn cstr_debug(bytes: &[u8]) -> &str {
    let len = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    core::str::from_utf8(&bytes[..len]).unwrap_or("?")
}

#[inline]
fn consume_exit_status(thread: &mut crate::task::thread::Thread) -> u32 {
    let code = thread.exit_code.unwrap_or(0);
    thread.exit_code = None;
    thread.exit_waiter_tid = None;
    thread.retain_exit_status = false;
    // Once waitpid/wait4 consumes a status, the child is reaped from the
    // parent's point of view. Keep the Thread object around until the deferred
    // reaper can safely drop its stack, but do not let future wait4(-1) calls
    // see it as a child and block forever.
    thread.parent_tid = 0;
    code
}

/// Emit a compact wait4 diagnostic from the scheduler's point of view.
///
/// This is intentionally serial-only and small: when shells get stuck in a
/// wait loop, the important question is whether the current thread still owns
/// any waitable children, and what state/exit-code they carry.
pub fn debug_wait4_snapshot(tag: &str, pid: i64, options: u64) {
    let mut child_count = 0u32;
    let mut waitable_count = 0u32;
    let mut current = WaitThreadSnapshot::empty();
    let mut children = [WaitThreadSnapshot::empty(); 8];
    let mut printed = 0usize;
    let cpu_id = get_cpu_id();
    let current_tid;
    {
        let guard = SCHEDULER.lock();
        let sched = match guard.as_ref() {
            Some(s) => s,
            None => {
                crate::serial_println!(
                    "licof linux wait4-debug {}: scheduler unavailable pid={} options={:#x}",
                    tag,
                    pid,
                    options
                );
                return;
            }
        };
        current_tid = sched.per_cpu[cpu_id].current_tid.unwrap_or(0);
        if let Some(t) = sched.threads.iter().find(|t| t.tid == current_tid) {
            let mut args = [0u8; 64];
            let args_len = t.args.iter().position(|&b| b == 0).unwrap_or(t.args.len());
            let args_copy_len = args_len.min(args.len() - 1);
            args[..args_copy_len].copy_from_slice(&t.args[..args_copy_len]);
            current = WaitThreadSnapshot {
                tid: t.tid,
                state: t.state,
                exit: t.exit_code.unwrap_or(u32::MAX),
                waiter: t.exit_waiter_tid.unwrap_or(0),
                retain: t.retain_exit_status,
                last_cpu: t.last_cpu,
                name: t.name,
                args,
                linux_fs_base: t.linux_fs_base,
            };
        }
        for t in sched.threads.iter() {
            if t.parent_tid == current_tid {
                child_count += 1;
                if t.state == ThreadState::Terminated && t.exit_code.is_some() {
                    waitable_count += 1;
                }
                if printed < children.len() {
                    let mut args = [0u8; 64];
                    let args_len = t.args.iter().position(|&b| b == 0).unwrap_or(t.args.len());
                    let args_copy_len = args_len.min(args.len() - 1);
                    args[..args_copy_len].copy_from_slice(&t.args[..args_copy_len]);
                    children[printed] = WaitThreadSnapshot {
                        tid: t.tid,
                        state: t.state,
                        exit: t.exit_code.unwrap_or(u32::MAX),
                        waiter: t.exit_waiter_tid.unwrap_or(0),
                        retain: t.retain_exit_status,
                        last_cpu: t.last_cpu,
                        name: t.name,
                        args,
                        linux_fs_base: t.linux_fs_base,
                    };
                    printed += 1;
                }
            }
        }
    }

    crate::serial_println!(
        "licof linux wait4-debug {}: cpu={} current_tid={} name='{}' args='{}' fs_base={:#x} pid={} options={:#x} children={} waitable={}",
        tag,
        cpu_id,
        current_tid,
        cstr_debug(&current.name),
        cstr_debug(&current.args),
        current.linux_fs_base,
        pid,
        options,
        child_count,
        waitable_count
    );

    for child in children.iter().take(printed) {
        crate::serial_println!(
            "licof linux wait4-debug {}: child tid={} name='{}' args='{}' fs_base={:#x} state={:?} exit={} waiter={} retain={} last_cpu={}",
            tag,
            child.tid,
            cstr_debug(&child.name),
            cstr_debug(&child.args),
            child.linux_fs_base,
            child.state,
            child.exit,
            child.waiter,
            child.retain,
            child.last_cpu
        );
    }
    if child_count as usize > children.len() {
        crate::serial_println!("licof linux wait4-debug {}: ... more children omitted", tag);
    }
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
                return if target.exit_code.is_some() {
                    consume_exit_status(target)
                } else {
                    u32::MAX
                };
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
            crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_WAITPID);
            let mut guard = SCHEDULER.lock();
            if let Some(sched) = guard.as_mut() {
                if let Some(target) = sched.threads.iter_mut().find(|t| t.tid == tid) {
                    if target.state == ThreadState::Terminated {
                        return if target.exit_code.is_some() {
                            consume_exit_status(target)
                        } else {
                            u32::MAX
                        };
                    }
                } else {
                    return u32::MAX;
                }
            }
        }
        crate::arch::hal::enable_interrupts();
        crate::arch::hal::halt();
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

        // Check if any wait-relevant children exist at all. A terminated child
        // whose exit_code is already consumed is no longer waitable.
        let has_children = sched.threads.iter().any(|t| {
            t.parent_tid == current_tid
                && (t.state != ThreadState::Terminated || t.exit_code.is_some())
        });
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
                let has_children = sched.threads.iter().any(|t| {
                    t.parent_tid == current_tid
                        && (t.state != ThreadState::Terminated || t.exit_code.is_some())
                });
                if !has_children {
                    return (u32::MAX, u32::MAX);
                }
            }
        }
        crate::arch::hal::enable_interrupts();
        crate::arch::hal::halt();
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
    let has_children = sched.threads.iter().any(|t| {
        t.parent_tid == current_tid && (t.state != ThreadState::Terminated || t.exit_code.is_some())
    });
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
            return if target.exit_code.is_some() {
                consume_exit_status(target)
            } else {
                u32::MAX
            };
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
