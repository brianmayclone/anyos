//! Preemptive round-robin scheduler with SMP support.
//!
//! Driven by per-CPU LAPIC timer interrupts, the scheduler picks the highest-priority
//! ready thread on each tick and performs a context switch. Each CPU has its own
//! `current` thread, idle context, and FPU state. The ready queue is shared across
//! all CPUs (protected by a single Spinlock).

use crate::memory::address::PhysAddr;
use crate::sync::spinlock::Spinlock;
use crate::task::context::CpuContext;
use crate::task::thread::{FxState, Thread, ThreadState};
use crate::arch::x86::smp::MAX_CPUS;
use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

static SCHEDULER: Spinlock<Option<Scheduler>> = Spinlock::new(None);

/// Lock-free debug variable: tracks the currently running thread TID.
/// Updated by schedule() before context switch. Read by exception handler
/// without acquiring any locks (safe for use in fault handlers).
static mut DEBUG_CURRENT_TID: u32 = 0;


/// Total scheduler ticks (incremented every schedule() call).
static TOTAL_SCHED_TICKS: AtomicU32 = AtomicU32::new(0);
/// Idle scheduler ticks (incremented when no thread is running).
static IDLE_SCHED_TICKS: AtomicU32 = AtomicU32::new(0);

/// Per-CPU total ticks (incremented on each timer tick, per CPU).
static PER_CPU_TOTAL: [AtomicU32; MAX_CPUS] = {
    const INIT: AtomicU32 = AtomicU32::new(0);
    [INIT; MAX_CPUS]
};
/// Per-CPU idle ticks (incremented when that CPU has no thread to run).
static PER_CPU_IDLE: [AtomicU32; MAX_CPUS] = {
    const INIT: AtomicU32 = AtomicU32::new(0);
    [INIT; MAX_CPUS]
};

/// Per-CPU flag: true when a thread is actively running on this CPU.
/// Used to correctly count idle ticks even when the scheduler lock is contended.
static PER_CPU_HAS_THREAD: [AtomicBool; MAX_CPUS] = {
    const INIT: AtomicBool = AtomicBool::new(false);
    [INIT; MAX_CPUS]
};

/// Core scheduler state: thread list, ready queue, and per-CPU contexts.
///
/// Per-CPU arrays are heap-allocated (`Vec`) to avoid placing ~11 KiB on the
/// stack during construction, which overflows the BSP boot stack in debug builds.
pub struct Scheduler {
    /// All threads known to the scheduler (running, ready, blocked, terminated).
    ///
    /// Each thread is heap-allocated in a `Box` so that its address is **stable**
    /// across `Vec` reallocations and `remove()` shifts.  This is critical because
    /// `schedule_inner()` extracts raw pointers to `CpuContext` / `FxState` under
    /// the lock and uses them *after* releasing it (for `context_switch`).  If
    /// another CPU pushes a new thread (causing reallocation) or reaps a dead one
    /// (causing element shifts) in that window, inline `Vec<Thread>` storage would
    /// invalidate those pointers — leading to corrupted RIP, page faults, or GPFs.
    threads: Vec<Box<Thread>>,
    /// Indices into `threads` for threads eligible to run, in FIFO order.
    ready_queue: VecDeque<usize>,
    /// Per-CPU: index of the currently executing thread, or None if idle.
    current: Vec<Option<usize>>,
    /// Per-CPU: CPU context to return to when no threads are runnable (hlt loop).
    idle_context: Vec<CpuContext>,
    /// Per-CPU: FPU/SSE state for the idle context.
    idle_fpu_state: Vec<FxState>,
}

impl Scheduler {
    fn new() -> Self {
        let mut idle_fpu = Vec::with_capacity(MAX_CPUS);
        for _ in 0..MAX_CPUS {
            idle_fpu.push(FxState::new_default());
        }
        Scheduler {
            threads: Vec::with_capacity(128),
            ready_queue: VecDeque::new(),
            current: alloc::vec![None; MAX_CPUS],
            idle_context: alloc::vec![CpuContext::default(); MAX_CPUS],
            idle_fpu_state: idle_fpu,
        }
    }

    fn add_thread(&mut self, thread: Thread) -> u32 {
        let tid = thread.tid;
        let idx = self.threads.len();
        self.threads.push(Box::new(thread));
        self.ready_queue.push_back(idx);
        tid
    }

    /// Add a thread in Blocked state without putting it in the ready queue.
    /// Used by `spawn_blocked()` to prevent SMP races — the thread can't be
    /// picked up by any CPU until explicitly woken via `wake_thread()`.
    fn add_thread_blocked(&mut self, mut thread: Thread) -> u32 {
        let tid = thread.tid;
        thread.state = ThreadState::Blocked;
        self.threads.push(Box::new(thread));
        tid
    }

    /// Remove terminated threads whose exit code has been consumed, or orphan
    /// threads that have been terminated for >2 seconds without a waiter.
    /// Frees kernel stacks and page directories. Fixes all index references.
    fn reap_terminated(&mut self) {
        let current_tick = crate::arch::x86::pit::get_ticks();
        let mut i = 0;
        while i < self.threads.len() {
            if self.threads[i].state == ThreadState::Terminated {
                // Reap if:
                // 1. exit_code consumed by waitpid/try_waitpid (original behavior), OR
                // 2. Orphan: no waiter registered + grace period expired (~2 sec)
                let consumed = self.threads[i].exit_code.is_none();
                let auto_reap = self.threads[i].waiting_tid.is_none()
                    && self.threads[i].terminated_at_tick
                        .map(|t| current_tick.wrapping_sub(t) > 200)
                        .unwrap_or(false);

                if consumed || auto_reap {
                    let removed_idx = i;
                    self.threads.remove(removed_idx);
                    // Fix ready_queue: remove stale index, shift indices above removed
                    self.ready_queue.retain(|&idx| idx != removed_idx);
                    for idx in self.ready_queue.iter_mut() {
                        if *idx > removed_idx {
                            *idx -= 1;
                        }
                    }
                    // Fix ALL per-CPU current indices
                    for cpu in 0..MAX_CPUS {
                        if let Some(ref mut cur) = self.current[cpu] {
                            if *cur == removed_idx {
                                self.current[cpu] = None;
                            } else if *cur > removed_idx {
                                *cur -= 1;
                            }
                        }
                    }
                    // Don't increment i — next thread shifted into this slot
                } else {
                    i += 1;
                }
            } else {
                i += 1;
            }
        }
    }

    fn pick_next(&mut self) -> Option<usize> {
        // Simple round-robin with priority: pick highest priority ready thread
        let mut best: Option<(usize, u8)> = None;
        let mut best_pos = 0;

        for (pos, &idx) in self.ready_queue.iter().enumerate() {
            let thread = &self.threads[idx];
            if thread.state == ThreadState::Ready {
                match best {
                    None => {
                        best = Some((idx, thread.priority));
                        best_pos = pos;
                    }
                    Some((_, bp)) if thread.priority > bp => {
                        best = Some((idx, thread.priority));
                        best_pos = pos;
                    }
                    _ => {}
                }
            }
        }

        if let Some((idx, _)) = best {
            self.ready_queue.remove(best_pos);
            Some(idx)
        } else {
            None
        }
    }
}

/// Initialize the global scheduler. Must be called once before any threads are spawned.
pub fn init() {
    let mut sched = SCHEDULER.lock();
    *sched = Some(Scheduler::new());
    crate::serial_println!("[OK] Scheduler initialized (SMP-aware, {} CPUs max)", MAX_CPUS);
}

/// Get total scheduler ticks (for CPU load calculation).
pub fn total_sched_ticks() -> u32 {
    TOTAL_SCHED_TICKS.load(Ordering::Relaxed)
}

/// Get idle scheduler ticks (for CPU load calculation).
pub fn idle_sched_ticks() -> u32 {
    IDLE_SCHED_TICKS.load(Ordering::Relaxed)
}

/// Get per-CPU total ticks.
pub fn per_cpu_total_ticks(cpu: usize) -> u32 {
    if cpu < MAX_CPUS {
        PER_CPU_TOTAL[cpu].load(Ordering::Relaxed)
    } else {
        0
    }
}

/// Get per-CPU idle ticks.
pub fn per_cpu_idle_ticks(cpu: usize) -> u32 {
    if cpu < MAX_CPUS {
        PER_CPU_IDLE[cpu].load(Ordering::Relaxed)
    } else {
        0
    }
}

/// Create a new kernel thread and add it to the ready queue.
/// Returns the assigned TID. Emits an `EVT_PROCESS_SPAWNED` event.
pub fn spawn(entry: extern "C" fn(), priority: u8, name: &str) -> u32 {
    let tid = {
        let thread = Thread::new(entry, priority, name);
        let mut sched = SCHEDULER.lock();
        let sched = sched.as_mut().expect("Scheduler not initialized");
        let tid = sched.add_thread(thread);
        crate::serial_println!("  Spawned thread '{}' (TID={})", name, tid);
        tid
    };

    // Pack the first 12 chars of the thread name into 3 u32 words (little-endian)
    // so that subscribers (e.g. the dock) can identify the process.
    let name_bytes = name.as_bytes();
    let mut p2: u32 = 0;
    let mut p3: u32 = 0;
    let mut p4: u32 = 0;
    for i in 0..name_bytes.len().min(12) {
        let word = match i / 4 {
            0 => &mut p2,
            1 => &mut p3,
            _ => &mut p4,
        };
        *word |= (name_bytes[i] as u32) << ((i % 4) * 8);
    }
    crate::ipc::event_bus::system_emit(crate::ipc::event_bus::EventData::new(
        crate::ipc::event_bus::EVT_PROCESS_SPAWNED, tid, p2, p3, p4,
    ));

    tid
}

/// Spawn a kernel thread in Blocked state (not added to the ready queue).
///
/// The thread will NOT run on any CPU until [`wake_thread`] is called with the
/// returned TID.  This prevents SMP races where an AP picks up the thread before
/// the caller has finished setting up user info, pending program data, and args.
pub fn spawn_blocked(entry: extern "C" fn(), priority: u8, name: &str) -> u32 {
    let tid = {
        let thread = Thread::new(entry, priority, name);
        let mut sched = SCHEDULER.lock();
        let sched = sched.as_mut().expect("Scheduler not initialized");
        let tid = sched.add_thread_blocked(thread);
        crate::serial_println!("  Spawned thread '{}' (TID={}, blocked)", name, tid);
        tid
    };

    // Emit process-spawned event (same as spawn)
    let name_bytes = name.as_bytes();
    let mut p2: u32 = 0;
    let mut p3: u32 = 0;
    let mut p4: u32 = 0;
    for i in 0..name_bytes.len().min(12) {
        let word = match i / 4 {
            0 => &mut p2,
            1 => &mut p3,
            _ => &mut p4,
        };
        *word |= (name_bytes[i] as u32) << ((i % 4) * 8);
    }
    crate::ipc::event_bus::system_emit(crate::ipc::event_bus::EventData::new(
        crate::ipc::event_bus::EVT_PROCESS_SPAWNED, tid, p2, p3, p4,
    ));

    tid
}

/// Called from the timer interrupt (PIT or LAPIC) to perform preemptive scheduling.
/// Increments CPU accounting counters (total ticks, idle ticks, per-thread cpu_ticks).
pub fn schedule_tick() {
    schedule_inner(true);
}

/// Voluntary yield: reschedule without incrementing CPU accounting counters.
/// Used by kernel threads (cpu_monitor busy-loop), syscalls (yield, sleep), etc.
pub fn schedule() {
    schedule_inner(false);
}

/// Get the current CPU index, always reading from the LAPIC.
fn get_cpu_id() -> usize {
    let c = crate::arch::x86::smp::current_cpu_id() as usize;
    if c < MAX_CPUS { c } else { 0 }
}

fn schedule_inner(from_timer: bool) {
    // Get the CPU ID for this core
    let cpu_id = get_cpu_id();

    // Extract context switch parameters under the lock, then release before switching
    // Tuple: (old_cpu_ctx, new_cpu_ctx, old_fpu_ptr, new_fpu_ptr)
    let switch_info: Option<(*mut CpuContext, *const CpuContext, *mut u8, *const u8)>;

    // Only increment counters on timer-driven scheduling
    if from_timer {
        TOTAL_SCHED_TICKS.fetch_add(1, Ordering::Relaxed);
        PER_CPU_TOTAL[cpu_id].fetch_add(1, Ordering::Relaxed);
    }

    let mut guard = match SCHEDULER.try_lock() {
        Some(s) => s,
        None => {
            // Lock contended — still count idle if this CPU has no running thread
            if from_timer && !PER_CPU_HAS_THREAD[cpu_id].load(Ordering::Relaxed) {
                IDLE_SCHED_TICKS.fetch_add(1, Ordering::Relaxed);
                PER_CPU_IDLE[cpu_id].fetch_add(1, Ordering::Relaxed);
            }
            return;
        }
    };

    {
        let sched = match guard.as_mut() {
            Some(s) => s,
            None => return, // guard drops normally (restores IF) — fine for early return
        };

        // Only CPU 0 (BSP) reaps terminated threads and wakes sleepers
        // to avoid redundant work and PIT-dependency
        if cpu_id == 0 {
            sched.reap_terminated();

            // Wake blocked threads whose sleep timer has expired
            if from_timer {
                let current_tick = crate::arch::x86::pit::get_ticks();
                for idx in 0..sched.threads.len() {
                    if sched.threads[idx].state == ThreadState::Blocked {
                        if let Some(wake_tick) = sched.threads[idx].wake_at_tick {
                            if current_tick.wrapping_sub(wake_tick) < 0x8000_0000 {
                                sched.threads[idx].state = ThreadState::Ready;
                                sched.threads[idx].wake_at_tick = None;
                                if !sched.ready_queue.contains(&idx) {
                                    sched.ready_queue.push_back(idx);
                                }
                            }
                        }
                    }
                }
            }
        }

        // Track CPU ticks for the currently running thread on THIS CPU
        if from_timer {
            if let Some(current_idx) = sched.current[cpu_id] {
                if sched.threads[current_idx].state == ThreadState::Running {
                    sched.threads[current_idx].cpu_ticks += 1;
                }
            } else {
                // No thread running on this CPU = idle
                IDLE_SCHED_TICKS.fetch_add(1, Ordering::Relaxed);
                PER_CPU_IDLE[cpu_id].fetch_add(1, Ordering::Relaxed);
            }
        }

        // Put current thread on THIS CPU back to ready
        if let Some(current_idx) = sched.current[cpu_id] {
            let thread = &mut sched.threads[current_idx];
            if thread.state == ThreadState::Running {
                thread.state = ThreadState::Ready;
                sched.ready_queue.push_back(current_idx);
            }
        }

        // Pick next thread for THIS CPU
        switch_info = if let Some(next_idx) = sched.pick_next() {
            let prev_idx = sched.current[cpu_id];
            sched.current[cpu_id] = Some(next_idx);
            sched.threads[next_idx].state = ThreadState::Running;
            PER_CPU_HAS_THREAD[cpu_id].store(true, Ordering::Relaxed);

            // Update lock-free debug TID
            unsafe { DEBUG_CURRENT_TID = sched.threads[next_idx].tid; }

            // Update this CPU's TSS RSP0 and SYSCALL per-CPU kernel RSP
            let kstack_top = sched.threads[next_idx].kernel_stack_top();
            crate::arch::x86::tss::set_kernel_stack_for_cpu(cpu_id, kstack_top);
            crate::arch::x86::syscall_msr::set_kernel_rsp(kstack_top);

            if let Some(prev_idx) = prev_idx {
                if prev_idx != next_idx {
                    let old_ctx = &mut sched.threads[prev_idx].context as *mut CpuContext;
                    let new_ctx = &sched.threads[next_idx].context as *const CpuContext;
                    let old_fpu = sched.threads[prev_idx].fpu_state.data.as_mut_ptr();
                    let new_fpu = sched.threads[next_idx].fpu_state.data.as_ptr();
                    Some((old_ctx, new_ctx, old_fpu, new_fpu))
                } else {
                    None // Same thread, no switch needed
                }
            } else {
                // No thread was running — switch from idle
                let idle_ctx = &mut sched.idle_context[cpu_id] as *mut CpuContext;
                let new_ctx = &sched.threads[next_idx].context as *const CpuContext;
                let old_fpu = sched.idle_fpu_state[cpu_id].data.as_mut_ptr();
                let new_fpu = sched.threads[next_idx].fpu_state.data.as_ptr();
                Some((idle_ctx, new_ctx, old_fpu, new_fpu))
            }
        } else {
            // No ready threads — this CPU is idle
            if from_timer {
                IDLE_SCHED_TICKS.fetch_add(1, Ordering::Relaxed);
                PER_CPU_IDLE[cpu_id].fetch_add(1, Ordering::Relaxed);
            }

            // If the current thread is no longer runnable
            // (e.g. Terminated or Blocked), switch back to the idle context
            if let Some(current_idx) = sched.current[cpu_id] {
                if sched.threads[current_idx].state != ThreadState::Running {
                    sched.current[cpu_id] = None;
                    PER_CPU_HAS_THREAD[cpu_id].store(false, Ordering::Relaxed);
                    let old_ctx = &mut sched.threads[current_idx].context as *mut CpuContext;
                    let idle_ctx = &sched.idle_context[cpu_id] as *const CpuContext;
                    let old_fpu = sched.threads[current_idx].fpu_state.data.as_mut_ptr();
                    let new_fpu = sched.idle_fpu_state[cpu_id].data.as_ptr();
                    Some((old_ctx, idle_ctx, old_fpu, new_fpu))
                } else {
                    None
                }
            } else {
                None
            }
        };
        // sched borrow ends here (inner scope)
    }

    // CRITICAL: Release the lock WITHOUT restoring IF. This keeps interrupts
    // disabled from lock acquisition all the way through context_switch.
    guard.release_no_irq_restore();

    // Context switch with the lock released AND interrupts still disabled.
    // context_switch.asm clears IF in restored RFLAGS to prevent races.
    if let Some((old_ctx, new_ctx, old_fpu, new_fpu)) = switch_info {
        // Safety check: validate RIP before switching
        let new_rip = unsafe { (*new_ctx).rip };
        let new_rsp = unsafe { (*new_ctx).rsp };
        let new_cr3 = unsafe { (*new_ctx).cr3 };
        if new_rip < 0xFFFF_FFFF_8010_0000 {
            crate::serial_println!(
                "BUG: context_switch to bad RIP={:#018x} RSP={:#018x} CR3={:#018x} CPU{}",
                new_rip, new_rsp, new_cr3, cpu_id,
            );
            // Recover: re-acquire lock and fix scheduler state
            {
                let mut guard = SCHEDULER.try_lock();
                if let Some(ref mut guard) = guard {
                    if let Some(sched) = guard.as_mut() {
                        if let Some(current_idx) = sched.current[cpu_id] {
                            sched.threads[current_idx].state = ThreadState::Terminated;
                            sched.current[cpu_id] = None;
                        }
                    }
                }
            }
            // Re-enable interrupts before returning (don't leave IF=0)
            unsafe { core::arch::asm!("sti"); }
            return;
        }
        // Save current FPU/SSE state, load new thread's FPU/SSE state
        unsafe {
            core::arch::asm!("fxsave [{}]", in(reg) old_fpu, options(nostack, preserves_flags));
            core::arch::asm!("fxrstor [{}]", in(reg) new_fpu, options(nostack, preserves_flags));
        }
        unsafe { crate::task::context::context_switch(old_ctx, new_ctx); }
    }

    // Re-enable interrupts. try_lock disabled IF, release_no_irq_restore kept
    // it disabled through context_switch. For timer-preempted paths this is
    // harmless (IRET will also restore IF). For voluntary schedule() calls
    // (e.g. sys_sleep) this is CRITICAL — without it, IF stays 0 permanently,
    // timer ticks stop, and sleep() loops forever.
    unsafe { core::arch::asm!("sti"); }
}

/// Get the current thread's TID (on the calling CPU).
pub fn current_tid() -> u32 {
    let cpu_id = get_cpu_id();
    let guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current[cpu_id] {
            return sched.threads[idx].tid;
        }
    }
    0
}

/// Check if the current thread is a user process (has its own page directory).
/// Returns true even when temporarily executing kernel code (syscall, trampoline).
pub fn is_current_thread_user() -> bool {
    let cpu_id = get_cpu_id();
    let guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current[cpu_id] {
            return sched.threads[idx].is_user;
        }
    }
    false
}

/// Get the current thread's name (for diagnostic messages).
pub fn current_thread_name() -> [u8; 32] {
    let cpu_id = get_cpu_id();
    let guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current[cpu_id] {
            return sched.threads[idx].name;
        }
    }
    [0u8; 32]
}

/// Lock-free read of the last-scheduled thread TID. Safe to call from
/// exception handlers even if the SCHEDULER lock is held.
pub fn debug_current_tid() -> u32 {
    unsafe { DEBUG_CURRENT_TID }
}

/// Configure a thread as a user process (set page directory, CR3, brk, is_user).
pub fn set_thread_user_info(tid: u32, pd: PhysAddr, brk: u32) {
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.page_directory = Some(pd);
        thread.context.cr3 = pd.as_u64();
        thread.is_user = true;
        thread.brk = brk;
    }
}

/// Set the architecture mode (Native64/Compat32) for a thread.
pub fn set_thread_arch_mode(tid: u32, mode: crate::task::thread::ArchMode) {
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.arch_mode = mode;
    }
}

/// Get the current thread's page directory (if it's a user process).
pub fn current_thread_page_directory() -> Option<PhysAddr> {
    let cpu_id = get_cpu_id();
    let guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current[cpu_id] {
            return sched.threads[idx].page_directory;
        }
    }
    None
}

/// Get the current thread's program break address.
pub fn current_thread_brk() -> u32 {
    let cpu_id = get_cpu_id();
    let guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current[cpu_id] {
            return sched.threads[idx].brk;
        }
    }
    0
}

/// Set the current thread's program break address.
pub fn set_current_thread_brk(brk: u32) {
    let cpu_id = get_cpu_id();
    let mut guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_mut() {
        if let Some(idx) = sched.current[cpu_id] {
            sched.threads[idx].brk = brk;
        }
    }
}

/// Terminate the current thread with an exit code.
/// Wakes any thread waiting via waitpid.
pub fn exit_current(code: u32) {
    let cpu_id = get_cpu_id();
    let tid;
    {
        let mut guard = SCHEDULER.lock();
        let sched = guard.as_mut().expect("Scheduler not initialized");

        tid = sched.current[cpu_id].map(|idx| sched.threads[idx].tid).unwrap_or(0);

        if let Some(current_idx) = sched.current[cpu_id] {
            sched.threads[current_idx].state = ThreadState::Terminated;
            sched.threads[current_idx].exit_code = Some(code);
            sched.threads[current_idx].terminated_at_tick =
                Some(crate::arch::x86::pit::get_ticks());
            // Clear page_directory field (sys_exit already freed the actual pages)
            sched.threads[current_idx].page_directory = None;

            // Wake any thread that is waiting on us
            if let Some(waiter_tid) = sched.threads[current_idx].waiting_tid {
                if let Some(waiter_idx) = sched.threads.iter().position(|t| t.tid == waiter_tid) {
                    if sched.threads[waiter_idx].state == ThreadState::Blocked {
                        sched.threads[waiter_idx].state = ThreadState::Ready;
                        if !sched.ready_queue.contains(&waiter_idx) {
                            sched.ready_queue.push_back(waiter_idx);
                        }
                    }
                }
            }
        }
        // Lock released here
    }

    crate::ipc::event_bus::system_emit(crate::ipc::event_bus::EventData::new(
        crate::ipc::event_bus::EVT_PROCESS_EXITED, tid, code, 0, 0,
    ));

    // Switch to another thread (never returns for the terminated thread)
    schedule();

    // In case schedule() returns (shouldn't happen for terminated threads)
    loop {
        unsafe { core::arch::asm!("hlt"); }
    }
}

/// Kill a thread by TID. Returns 0 on success, u32::MAX on error.
/// Cannot kill the compositor thread (TID 3) or idle (TID 0).
pub fn kill_thread(tid: u32) -> u32 {
    if tid == 0 {
        return u32::MAX; // Can't kill idle
    }

    let mut pd_to_destroy: Option<PhysAddr> = None;
    let mut is_current = false;
    let cpu_id = get_cpu_id();

    {
        let mut guard = SCHEDULER.lock();
        let sched = guard.as_mut().expect("Scheduler not initialized");

        // Find the target thread
        let target_idx = match sched.threads.iter().position(|t| t.tid == tid) {
            Some(idx) => idx,
            None => return u32::MAX, // Not found
        };

        // Protect system threads: compositor (TID 3) and cpu_monitor
        if tid == 3 {
            return u32::MAX;
        }

        // Check if killing current thread (on this CPU)
        is_current = sched.current[cpu_id] == Some(target_idx);

        // Mark as terminated
        sched.threads[target_idx].state = ThreadState::Terminated;
        sched.threads[target_idx].exit_code = Some(u32::MAX - 1); // killed
        sched.threads[target_idx].terminated_at_tick =
            Some(crate::arch::x86::pit::get_ticks());

        // Remove from ready queue
        sched.ready_queue.retain(|&idx| idx != target_idx);

        // If this is a user process, remember PD for cleanup
        if let Some(pd) = sched.threads[target_idx].page_directory {
            pd_to_destroy = Some(pd);
            sched.threads[target_idx].page_directory = None;
        }

        // Wake any thread that is waiting on us
        if let Some(waiter_tid) = sched.threads[target_idx].waiting_tid {
            if let Some(waiter_idx) = sched.threads.iter().position(|t| t.tid == waiter_tid) {
                if sched.threads[waiter_idx].state == ThreadState::Blocked {
                    sched.threads[waiter_idx].state = ThreadState::Ready;
                    if !sched.ready_queue.contains(&waiter_idx) {
                        sched.ready_queue.push_back(waiter_idx);
                    }
                }
            }
        }

        if is_current {
            sched.current[cpu_id] = None;
        }
    }

    // Destroy user page directory outside the scheduler lock
    if let Some(pd) = pd_to_destroy {
        // Switch to kernel CR3 first if killing current
        if is_current {
            let kernel_cr3 = crate::memory::virtual_mem::kernel_cr3();
            unsafe { core::arch::asm!("mov cr3, {}", in(reg) kernel_cr3); }
        }
        crate::memory::virtual_mem::destroy_user_page_directory(pd);
    }

    crate::ipc::event_bus::system_emit(crate::ipc::event_bus::EventData::new(
        crate::ipc::event_bus::EVT_PROCESS_EXITED, tid, u32::MAX - 1, 0, 0,
    ));

    // If we killed the current thread, switch away
    if is_current {
        schedule();
        loop { unsafe { core::arch::asm!("hlt"); } }
    }

    0
}

/// Wait for a thread to terminate and return its exit code.
/// If called from a scheduled thread, properly blocks and yields CPU.
/// If called from the idle context (kernel_main), busy-waits.
pub fn waitpid(tid: u32) -> u32 {
    let cpu_id = get_cpu_id();
    // Register as a waiter and block if we're a scheduled thread
    {
        let mut guard = SCHEDULER.lock();
        let sched = guard.as_mut().expect("Scheduler not initialized");

        // Check if already terminated
        if let Some(target) = sched.threads.iter_mut().find(|t| t.tid == tid) {
            if target.state == ThreadState::Terminated {
                let code = target.exit_code.unwrap_or(0);
                // Clear exit_code so reap_terminated() can free this thread
                target.exit_code = None;
                return code;
            }
        } else {
            return u32::MAX; // Thread not found
        }

        // If we're a scheduled thread, properly block and register as waiter
        if let Some(current_idx) = sched.current[cpu_id] {
            let current_tid = sched.threads[current_idx].tid;
            // Tell the target to wake us when it terminates
            if let Some(target) = sched.threads.iter_mut().find(|t| t.tid == tid) {
                target.waiting_tid = Some(current_tid);
            }
            // Block ourselves — scheduler won't pick us until we're woken
            sched.threads[current_idx].state = ThreadState::Blocked;
        }
        // Lock released here
    }

    // Wait for the target thread to terminate.
    loop {
        unsafe { core::arch::asm!("sti; hlt"); }

        {
            let mut guard = SCHEDULER.lock();
            if let Some(sched) = guard.as_mut() {
                if let Some(target) = sched.threads.iter_mut().find(|t| t.tid == tid) {
                    if target.state == ThreadState::Terminated {
                        let code = target.exit_code.unwrap_or(0);
                        // Clear exit_code so reap_terminated() can free this thread
                        target.exit_code = None;
                        return code;
                    }
                } else {
                    return u32::MAX;
                }
            }
        }
    }
}

/// Non-blocking check if a thread has terminated.
/// Returns exit code if terminated, u32::MAX if not found, u32::MAX-1 if still running.
pub fn try_waitpid(tid: u32) -> u32 {
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    if let Some(target) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        if target.state == ThreadState::Terminated {
            let code = target.exit_code.unwrap_or(0);
            target.exit_code = None;
            return code;
        }
        return u32::MAX - 1; // Still running
    }
    u32::MAX // Not found
}

/// Set command-line arguments for a thread (before it starts running).
pub fn set_thread_args(tid: u32, args: &str) {
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        let bytes = args.as_bytes();
        let len = bytes.len().min(255);
        thread.args[..len].copy_from_slice(&bytes[..len]);
        thread.args[len] = 0;
    }
}

/// Get the current thread's command-line arguments.
pub fn current_thread_args(buf: &mut [u8]) -> usize {
    let cpu_id = get_cpu_id();
    let guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current[cpu_id] {
            let args = &sched.threads[idx].args;
            let len = args.iter().position(|&b| b == 0).unwrap_or(256);
            let copy_len = len.min(buf.len());
            buf[..copy_len].copy_from_slice(&args[..copy_len]);
            return copy_len;
        }
    }
    0
}

/// Set a thread's stdout pipe ID (0 = no pipe).
pub fn set_thread_stdout_pipe(tid: u32, pipe_id: u32) {
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.stdout_pipe = pipe_id;
    }
}

/// Get the current thread's stdout pipe ID (0 = no pipe).
pub fn current_thread_stdout_pipe() -> u32 {
    let cpu_id = get_cpu_id();
    let guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current[cpu_id] {
            return sched.threads[idx].stdout_pipe;
        }
    }
    0
}

/// Snapshot of a thread's state, used by the `ps` / sysinfo syscall.
pub struct ThreadInfo {
    pub tid: u32,
    pub priority: u8,
    pub state: &'static str,
    pub name: alloc::string::String,
    pub cpu_ticks: u32,
    /// Architecture mode: 0 = Native64, 1 = Compat32.
    pub arch_mode: u8,
}

/// List all live threads (for `ps` command). Terminated threads are excluded.
pub fn list_threads() -> Vec<ThreadInfo> {
    let guard = SCHEDULER.lock();
    let mut result = Vec::new();
    if let Some(sched) = guard.as_ref() {
        for thread in &sched.threads {
            if thread.state == ThreadState::Terminated {
                continue; // Don't show dead threads
            }
            let state_str = match thread.state {
                ThreadState::Ready => "ready",
                ThreadState::Running => "running",
                ThreadState::Blocked => "blocked",
                ThreadState::Terminated => unreachable!(),
            };
            result.push(ThreadInfo {
                tid: thread.tid,
                priority: thread.priority,
                state: state_str,
                name: alloc::string::String::from(thread.name_str()),
                cpu_ticks: thread.cpu_ticks,
                arch_mode: thread.arch_mode as u8,
            });
        }
    }
    result
}

/// Block the current thread until the given PIT tick count is reached.
pub fn sleep_until(wake_at: u32) {
    let cpu_id = get_cpu_id();
    {
        let mut guard = SCHEDULER.lock();
        let sched = guard.as_mut().expect("Scheduler not initialized");
        if let Some(current_idx) = sched.current[cpu_id] {
            sched.threads[current_idx].wake_at_tick = Some(wake_at);
            sched.threads[current_idx].state = ThreadState::Blocked;
        }
    }
    schedule();
}

/// Block the current thread unconditionally (no wake condition).
///
/// The caller must arrange for [`wake_thread`] to be called later to unblock
/// the thread.  Used by [`crate::sync::mutex::Mutex`] and
/// [`crate::sync::semaphore::Semaphore`] for scheduler-integrated blocking.
pub fn block_current_thread() {
    let cpu_id = get_cpu_id();
    {
        let mut guard = SCHEDULER.lock();
        let sched = guard.as_mut().expect("Scheduler not initialized");
        if let Some(idx) = sched.current[cpu_id] {
            sched.threads[idx].state = ThreadState::Blocked;
        }
    }
    schedule();
}

/// Set the priority of a thread by TID.
pub fn set_thread_priority(tid: u32, priority: u8) {
    let mut guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_mut() {
        if let Some(idx) = sched.threads.iter().position(|t| t.tid == tid) {
            sched.threads[idx].priority = priority;
        }
    }
}

/// Wake a blocked thread by TID, moving it back to the ready queue.
///
/// If the thread is not in `Blocked` state this is a no-op.
pub fn wake_thread(tid: u32) {
    let mut guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_mut() {
        if let Some(idx) = sched.threads.iter().position(|t| t.tid == tid) {
            if sched.threads[idx].state == ThreadState::Blocked {
                sched.threads[idx].state = ThreadState::Ready;
                if !sched.ready_queue.contains(&idx) {
                    sched.ready_queue.push_back(idx);
                }
            }
        }
    }
}

/// Enter the scheduler loop (called from kernel_main, becomes idle thread)
pub fn run() -> ! {
    unsafe { core::arch::asm!("sti"); }
    loop {
        unsafe { core::arch::asm!("hlt"); }
    }
}
