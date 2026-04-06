//! Current thread accessors — lock-based and lock-free variants.

use super::{SCHEDULER, PER_CPU_CURRENT_TID, PER_CPU_IS_USER,
            PER_CPU_HAS_THREAD, PER_CPU_THREAD_NAME, PER_CPU_IN_SCHEDULER,
            PER_CPU_STACK_BOTTOM, PER_CPU_STACK_TOP, PER_CPU_IDLE_STACK_TOP,
            PER_CPU_LAST_SYSCALL};
use crate::arch::hal::MAX_CPUS;
use crate::task::thread::ThreadState;
use core::sync::atomic::Ordering;

/// Get the current thread's TID (on the calling CPU).
///
/// Lock-free: reads from PER_CPU_CURRENT_TID which is kept in sync by the
/// scheduler during every context switch.  Previously this acquired the
/// SCHEDULER lock, which caused deadlocks when called from inside
/// serial_println! (OUTPUT_LOCK held → SCHEDULER lock = lock-order inversion).
pub fn current_tid() -> u32 {
    let cpu_id = crate::arch::hal::cpu_id();
    if cpu_id < MAX_CPUS { PER_CPU_CURRENT_TID[cpu_id].load(Ordering::Relaxed) } else { 0 }
}

/// Check if the current thread is a user process.
///
/// Lock-free: reads from PER_CPU_IS_USER (same rationale as current_tid).
pub fn is_current_thread_user() -> bool {
    let cpu_id = crate::arch::hal::cpu_id();
    if cpu_id < MAX_CPUS { PER_CPU_IS_USER[cpu_id].load(Ordering::Relaxed) } else { false }
}

/// Get the current thread's name.
///
/// Lock-free: reads from PER_CPU_THREAD_NAME (same rationale as current_tid).
pub fn current_thread_name() -> [u8; 32] {
    let cpu_id = crate::arch::hal::cpu_id();
    if cpu_id >= MAX_CPUS { return [0u8; 32]; }
    unsafe {
        let src = core::ptr::addr_of!(PER_CPU_THREAD_NAME[cpu_id]);
        core::ptr::read_volatile(src)
    }
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
        crate::serial_verbose_println!(
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

/// Record the last syscall number on this CPU (lock-free, called from dispatch).
pub fn set_last_syscall(cpu_id: usize, num: u32) {
    if cpu_id < MAX_CPUS {
        PER_CPU_LAST_SYSCALL[cpu_id].store(num, Ordering::Relaxed);
    }
}

/// Get the last syscall number executed on this CPU (lock-free crash diagnostics).
pub fn get_last_syscall(cpu_id: usize) -> u32 {
    if cpu_id < MAX_CPUS {
        PER_CPU_LAST_SYSCALL[cpu_id].load(Ordering::Relaxed)
    } else {
        0
    }
}

/// Check if a thread has any of the given capability bits set.
/// Uses try_lock to avoid deadlock when called from paths that may already
/// hold the SCHEDULER lock (e.g., SHM create during process cleanup).
/// Returns false if the lock cannot be acquired (conservative — treats
/// the thread as unprivileged, which just applies the stricter limit).
pub fn thread_has_cap(tid: u32, cap_mask: crate::task::capabilities::CapSet) -> bool {
    let guard = match SCHEDULER.try_lock() {
        Some(g) => g,
        None => return false, // Can't check — assume unprivileged
    };
    let sched = match guard.as_ref() { Some(s) => s, None => return false };
    for t in &sched.threads {
        if t.tid == tid {
            return (t.capabilities & cap_mask) != 0;
        }
    }
    false
}
