//! Current thread accessors — lock-based and lock-free variants.

use super::{get_cpu_id, SCHEDULER, PER_CPU_CURRENT_TID, PER_CPU_IS_USER,
            PER_CPU_HAS_THREAD, PER_CPU_THREAD_NAME, PER_CPU_IN_SCHEDULER,
            PER_CPU_STACK_BOTTOM, PER_CPU_STACK_TOP, PER_CPU_IDLE_STACK_TOP};
use crate::arch::hal::MAX_CPUS;
use crate::task::thread::ThreadState;
use core::sync::atomic::Ordering;

/// Get the current thread's TID (on the calling CPU).
pub fn current_tid() -> u32 {
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        return sched.per_cpu[cpu_id].current_tid.unwrap_or(0);
    }
    0
}

/// Check if the current thread is a user process.
pub fn is_current_thread_user() -> bool {
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current_idx(cpu_id) {
            return sched.threads[idx].is_user;
        }
    }
    false
}

/// Get the current thread's name.
pub fn current_thread_name() -> [u8; 32] {
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current_idx(cpu_id) {
            return sched.threads[idx].name;
        }
    }
    [0u8; 32]
}

/// Lock-free read of the current TID on this CPU.
pub fn debug_current_tid() -> u32 {
    let cpu_id = crate::arch::hal::cpu_id();
    if cpu_id < MAX_CPUS { PER_CPU_CURRENT_TID[cpu_id].load(Ordering::Relaxed) } else { 0 }
}

/// Lock-free check: is the current thread a user process?
pub fn debug_is_current_user() -> bool {
    let cpu_id = crate::arch::hal::cpu_id();
    PER_CPU_IS_USER[cpu_id].load(Ordering::Relaxed)
}

/// Lock-free read of the cached thread name for the current CPU.
pub fn debug_current_thread_name() -> [u8; 32] {
    let cpu_id = crate::arch::hal::cpu_id();
    if cpu_id >= MAX_CPUS { return [0u8; 32]; }
    unsafe {
        let src = core::ptr::addr_of!(PER_CPU_THREAD_NAME[cpu_id]);
        core::ptr::read_volatile(src)
    }
}

/// Lock-free check: does this CPU have an active thread running?
pub fn cpu_has_active_thread(cpu_id: usize) -> bool {
    if cpu_id < MAX_CPUS { PER_CPU_HAS_THREAD[cpu_id].load(Ordering::Relaxed) } else { false }
}

/// Lock-free check: is this CPU currently inside schedule_inner?
pub fn per_cpu_in_scheduler(cpu: usize) -> bool {
    if cpu < MAX_CPUS { PER_CPU_IN_SCHEDULER[cpu].load(Ordering::Relaxed) } else { false }
}

/// Get the idle thread's kernel stack top for a given CPU.
/// Used by AP init to switch from the small 16 KiB boot stack to the
/// idle thread's 512 KiB kernel stack for more headroom.
pub fn idle_stack_top(cpu_id: usize) -> u64 {
    if cpu_id < MAX_CPUS { PER_CPU_IDLE_STACK_TOP[cpu_id].load(Ordering::Relaxed) } else { 0 }
}

/// Lock-free read: current thread TID on this CPU (0 if none).
pub fn per_cpu_current_tid(cpu: usize) -> u32 {
    if cpu < MAX_CPUS { PER_CPU_CURRENT_TID[cpu].load(Ordering::Relaxed) } else { 0 }
}

/// Lock-free check: does this CPU have a non-idle thread?
pub fn per_cpu_has_thread(cpu: usize) -> bool {
    if cpu < MAX_CPUS { PER_CPU_HAS_THREAD[cpu].load(Ordering::Relaxed) } else { false }
}

/// Check the current thread's stack canary after a syscall.
pub fn check_current_stack_canary(syscall_num: u32) {
    let mut guard = SCHEDULER.lock();
    let sched = match guard.as_mut() { Some(s) => s, None => return };
    let cpu_id = crate::arch::hal::cpu_id();
    let tid = match sched.per_cpu[cpu_id].current_tid { Some(t) => t, None => return };
    let idx = match sched.current_idx(cpu_id) { Some(i) => i, None => return };
    if !sched.threads[idx].check_stack_canary() {
        crate::serial_println!(
            "STACK OVERFLOW after syscall {} in '{}' (TID={}) — killing",
            syscall_num, sched.threads[idx].name_str(), tid,
        );
        sched.threads[idx].state = ThreadState::Terminated;
        sched.threads[idx].exit_code = Some(139);
        sched.threads[idx].terminated_at_tick = Some(crate::arch::hal::timer_current_ticks());
    }
}

/// Lock-free check: is RSP within this CPU's current thread's kernel stack?
pub fn check_rsp_in_bounds(cpu_id: usize, rsp: u64) -> bool {
    let bottom = PER_CPU_STACK_BOTTOM[cpu_id].load(Ordering::Relaxed);
    let top = PER_CPU_STACK_TOP[cpu_id].load(Ordering::Relaxed);
    if bottom == 0 || top == 0 { return true; }
    rsp >= bottom && rsp <= top
}

/// Get per-CPU stack bounds (lock-free).
pub fn get_stack_bounds(cpu_id: usize) -> (u64, u64) {
    (PER_CPU_STACK_BOTTOM[cpu_id].load(Ordering::Relaxed),
     PER_CPU_STACK_TOP[cpu_id].load(Ordering::Relaxed))
}
