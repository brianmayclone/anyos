//! Preemptive round-robin scheduler with priority support.
//!
//! Driven by the PIT timer interrupt, the scheduler picks the highest-priority ready thread
//! on each tick and performs a context switch. Threads can be spawned, blocked, waited on,
//! and killed. The idle context (kernel_main's `hlt` loop) runs when no threads are ready.

use crate::memory::address::PhysAddr;
use crate::sync::spinlock::Spinlock;
use crate::task::context::CpuContext;
use crate::task::thread::{FxState, Thread, ThreadState};
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

static SCHEDULER: Spinlock<Option<Scheduler>> = Spinlock::new(None);

/// Lock-free debug variable: tracks the currently running thread TID.
/// Updated by schedule() before context switch. Read by exception handler
/// without acquiring any locks (safe for use in fault handlers).
static mut DEBUG_CURRENT_TID: u32 = 0;

/// Total scheduler ticks (incremented every schedule() call).
static TOTAL_SCHED_TICKS: AtomicU32 = AtomicU32::new(0);
/// Idle scheduler ticks (incremented when no thread is running).
static IDLE_SCHED_TICKS: AtomicU32 = AtomicU32::new(0);

/// Core scheduler state: thread list, ready queue, and the idle context.
pub struct Scheduler {
    /// All threads known to the scheduler (running, ready, blocked, terminated).
    threads: Vec<Thread>,
    /// Indices into `threads` for threads eligible to run, in FIFO order.
    ready_queue: VecDeque<usize>,
    /// Index of the currently executing thread, or `None` if idle.
    current: Option<usize>,
    /// CPU context to return to when no threads are runnable (kernel_main's hlt loop).
    idle_context: CpuContext,
    /// FPU/SSE state for the idle context.
    idle_fpu_state: FxState,
}

impl Scheduler {
    fn new() -> Self {
        Scheduler {
            threads: Vec::new(),
            ready_queue: VecDeque::new(),
            current: None,
            idle_context: CpuContext::default(),
            idle_fpu_state: FxState::new_default(),
        }
    }

    fn add_thread(&mut self, thread: Thread) -> u32 {
        let tid = thread.tid;
        let idx = self.threads.len();
        self.threads.push(thread);
        self.ready_queue.push_back(idx);
        tid
    }

    /// Remove terminated threads whose exit code has been consumed (waiting_tid cleared).
    /// Frees kernel stacks and page directories. Fixes all index references.
    fn reap_terminated(&mut self) {
        let mut i = 0;
        while i < self.threads.len() {
            if self.threads[i].state == ThreadState::Terminated
                && self.threads[i].exit_code.is_none()
            {
                let removed_idx = i;
                self.threads.remove(removed_idx);
                // Fix ready_queue: remove stale index, shift indices above removed
                self.ready_queue.retain(|&idx| idx != removed_idx);
                for idx in self.ready_queue.iter_mut() {
                    if *idx > removed_idx {
                        *idx -= 1;
                    }
                }
                // Fix current index
                if let Some(ref mut current) = self.current {
                    if *current == removed_idx {
                        self.current = None;
                    } else if *current > removed_idx {
                        *current -= 1;
                    }
                }
                // Don't increment i — next thread shifted into this slot
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
    crate::serial_println!("[OK] Scheduler initialized");
}

/// Get total scheduler ticks (for CPU load calculation).
pub fn total_sched_ticks() -> u32 {
    TOTAL_SCHED_TICKS.load(Ordering::Relaxed)
}

/// Get idle scheduler ticks (for CPU load calculation).
pub fn idle_sched_ticks() -> u32 {
    IDLE_SCHED_TICKS.load(Ordering::Relaxed)
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

    crate::ipc::event_bus::system_emit(crate::ipc::event_bus::EventData::new(
        crate::ipc::event_bus::EVT_PROCESS_SPAWNED, tid, 0, 0, 0,
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

fn schedule_inner(from_timer: bool) {
    // Extract context switch parameters under the lock, then release before switching
    // Tuple: (old_cpu_ctx, new_cpu_ctx, old_fpu_ptr, new_fpu_ptr)
    let switch_info: Option<(*mut CpuContext, *const CpuContext, *mut u8, *const u8)>;

    // Only increment counters on timer-driven scheduling
    if from_timer {
        TOTAL_SCHED_TICKS.fetch_add(1, Ordering::Relaxed);
    }

    let mut guard = match SCHEDULER.try_lock() {
        Some(s) => s,
        None => return, // Scheduler is busy, skip this tick
    };

    {
        let sched = match guard.as_mut() {
            Some(s) => s,
            None => return, // guard drops normally (restores IF) — fine for early return
        };

        // Reap terminated threads to free kernel stacks and page directories
        sched.reap_terminated();

        // Track CPU ticks for the currently running thread (timer-driven only)
        if from_timer {
            if let Some(current_idx) = sched.current {
                if sched.threads[current_idx].state == ThreadState::Running {
                    sched.threads[current_idx].cpu_ticks += 1;
                }
            } else {
                // No thread running = idle
                IDLE_SCHED_TICKS.fetch_add(1, Ordering::Relaxed);
            }
        }

        // Put current thread back to ready
        if let Some(current_idx) = sched.current {
            let thread = &mut sched.threads[current_idx];
            if thread.state == ThreadState::Running {
                thread.state = ThreadState::Ready;
                sched.ready_queue.push_back(current_idx);
            }
        }

        // Pick next thread
        switch_info = if let Some(next_idx) = sched.pick_next() {
            let prev_idx = sched.current;
            sched.current = Some(next_idx);
            sched.threads[next_idx].state = ThreadState::Running;

            // Update lock-free debug TID
            unsafe { DEBUG_CURRENT_TID = sched.threads[next_idx].tid; }

            // Update TSS RSP0 and SYSCALL per-CPU kernel RSP
            let kstack_top = sched.threads[next_idx].kernel_stack_top();
            crate::arch::x86::tss::set_kernel_stack(kstack_top);
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
                // First thread ever - switch from idle
                let idle_ctx = &mut sched.idle_context as *mut CpuContext;
                let new_ctx = &sched.threads[next_idx].context as *const CpuContext;
                let old_fpu = sched.idle_fpu_state.data.as_mut_ptr();
                let new_fpu = sched.threads[next_idx].fpu_state.data.as_ptr();
                Some((idle_ctx, new_ctx, old_fpu, new_fpu))
            }
        } else {
            // No ready threads — count as idle (timer-driven only)
            if from_timer {
                IDLE_SCHED_TICKS.fetch_add(1, Ordering::Relaxed);
            }

            // If the current thread is no longer runnable
            // (e.g. Terminated or Blocked), switch back to the idle context
            // so that kernel_main can resume (e.g. waitpid polling).
            if let Some(current_idx) = sched.current {
                if sched.threads[current_idx].state != ThreadState::Running {
                    sched.current = None;
                    let old_ctx = &mut sched.threads[current_idx].context as *mut CpuContext;
                    let idle_ctx = &sched.idle_context as *const CpuContext;
                    let old_fpu = sched.threads[current_idx].fpu_state.data.as_mut_ptr();
                    let new_fpu = sched.idle_fpu_state.data.as_ptr();
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
    // The old approach (guard drop → sti, then cli) had a 1-instruction window
    // where a timer could fire and cause a nested schedule() that corrupts state.
    guard.release_no_irq_restore();

    // Context switch with the lock released AND interrupts still disabled.
    // context_switch.asm restores the target thread's RFLAGS (which includes IF).
    if let Some((old_ctx, new_ctx, old_fpu, new_fpu)) = switch_info {
        // Safety check: validate RIP before switching
        let new_rip = unsafe { (*new_ctx).rip };
        let new_rsp = unsafe { (*new_ctx).rsp };
        let new_cr3 = unsafe { (*new_ctx).cr3 };
        if new_rip < 0xFFFF_FFFF_8010_0000 {
            crate::serial_println!(
                "BUG: context_switch to bad RIP={:#018x} RSP={:#018x} CR3={:#018x}",
                new_rip, new_rsp, new_cr3,
            );
            // Recover: re-acquire lock and fix scheduler state
            {
                let mut guard = SCHEDULER.try_lock();
                if let Some(ref mut guard) = guard {
                    if let Some(sched) = guard.as_mut() {
                        // The bad thread is marked Running but we won't switch to it.
                        // Mark it Terminated so it's never picked again.
                        if let Some(current_idx) = sched.current {
                            sched.threads[current_idx].state = ThreadState::Terminated;
                            sched.current = None;
                        }
                    }
                }
            }
            return;
        }
        // Save current FPU/SSE state, load new thread's FPU/SSE state
        unsafe {
            core::arch::asm!("fxsave [{}]", in(reg) old_fpu, options(nostack, preserves_flags));
            core::arch::asm!("fxrstor [{}]", in(reg) new_fpu, options(nostack, preserves_flags));
        }
        unsafe { crate::task::context::context_switch(old_ctx, new_ctx); }
    }
}

/// Get the current thread's TID.
pub fn current_tid() -> u32 {
    let guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current {
            return sched.threads[idx].tid;
        }
    }
    0
}

/// Check if the current thread is a user process (has its own page directory).
/// Returns true even when temporarily executing kernel code (syscall, trampoline).
pub fn is_current_thread_user() -> bool {
    let guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current {
            return sched.threads[idx].is_user;
        }
    }
    false
}

/// Get the current thread's name (for diagnostic messages).
pub fn current_thread_name() -> [u8; 32] {
    let guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current {
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
    let guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current {
            return sched.threads[idx].page_directory;
        }
    }
    None
}

/// Get the current thread's program break address.
pub fn current_thread_brk() -> u32 {
    let guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current {
            return sched.threads[idx].brk;
        }
    }
    0
}

/// Set the current thread's program break address.
pub fn set_current_thread_brk(brk: u32) {
    let mut guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_mut() {
        if let Some(idx) = sched.current {
            sched.threads[idx].brk = brk;
        }
    }
}

/// Terminate the current thread with an exit code.
/// Wakes any thread waiting via waitpid.
pub fn exit_current(code: u32) {
    let tid;
    {
        let mut guard = SCHEDULER.lock();
        let sched = guard.as_mut().expect("Scheduler not initialized");

        tid = sched.current.map(|idx| sched.threads[idx].tid).unwrap_or(0);

        if let Some(current_idx) = sched.current {
            sched.threads[current_idx].state = ThreadState::Terminated;
            sched.threads[current_idx].exit_code = Some(code);

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

        // Check if killing current thread
        is_current = sched.current == Some(target_idx);

        // Mark as terminated
        sched.threads[target_idx].state = ThreadState::Terminated;
        sched.threads[target_idx].exit_code = Some(u32::MAX - 1); // killed

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
            sched.current = None;
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
        if let Some(current_idx) = sched.current {
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
    // Timer IRQ will call schedule(), which skips us (Blocked) and runs others.
    // When the target exits, exit_current() wakes us (sets Ready + pushes to queue).
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
    let guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current {
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
    let guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current {
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

/// Enter the scheduler loop (called from kernel_main, becomes idle thread)
pub fn run() -> ! {
    unsafe { core::arch::asm!("sti"); }
    loop {
        unsafe { core::arch::asm!("hlt"); }
    }
}
