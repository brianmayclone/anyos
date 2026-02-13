//! Preemptive round-robin scheduler with per-CPU ready queues.
//!
//! Each CPU has its own ready queue (TID-based) and tracks its current thread by TID.
//! This eliminates cross-CPU index manipulation — the root cause of thread identity
//! corruption in the previous shared-queue design. Work stealing ensures idle CPUs
//! pick up threads from overloaded ones.

use crate::memory::address::PhysAddr;
use crate::sync::spinlock::Spinlock;
use crate::task::context::CpuContext;
use crate::task::thread::{FxState, Thread, ThreadState};
use crate::arch::x86::smp::MAX_CPUS;
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

static SCHEDULER: Spinlock<Option<Scheduler>> = Spinlock::new(None);

/// Deferred page directory destruction queue.
/// When kill_thread() kills a thread running on another CPU, the PD can't be
/// destroyed immediately (the other CPU might still be mid-syscall accessing
/// user pages). Instead, the PD is queued here and destroyed on CPU 0's next
/// timer tick — by which time the killed thread has been descheduled.
struct DeferredPdQueue {
    entries: [Option<PhysAddr>; 16],
}
impl DeferredPdQueue {
    const fn new() -> Self {
        Self { entries: [None; 16] }
    }
    fn push(&mut self, pd: PhysAddr) {
        for slot in self.entries.iter_mut() {
            if slot.is_none() {
                *slot = Some(pd);
                return;
            }
        }
        // Queue full — destroy synchronously (last resort, risky)
        crate::serial_println!("WARNING: deferred PD queue full, destroying synchronously");
        crate::memory::virtual_mem::destroy_user_page_directory(pd);
    }
    fn drain(&mut self) -> [Option<PhysAddr>; 16] {
        let result = self.entries;
        self.entries = [None; 16];
        result
    }
}
static DEFERRED_PD_DESTROY: Spinlock<DeferredPdQueue> = Spinlock::new(DeferredPdQueue::new());

/// Per-CPU lock-free TID: tracks the currently running thread TID on each CPU.
/// Updated by schedule() before context switch. Read by exception handlers
/// without acquiring any locks (safe for use in fault handlers).
static PER_CPU_CURRENT_TID: [AtomicU32; MAX_CPUS] = {
    const INIT: AtomicU32 = AtomicU32::new(0);
    [INIT; MAX_CPUS]
};

/// Per-CPU lock-free flag: true when the currently running thread is a user process.
/// Updated in schedule_inner. Read by exception handlers (try_kill_faulting_thread)
/// without locks, preventing deadlock if a fault fires while the scheduler lock is held.
static PER_CPU_IS_USER: [AtomicBool; MAX_CPUS] = {
    const INIT: AtomicBool = AtomicBool::new(false);
    [INIT; MAX_CPUS]
};

/// Round-robin counter for tie-breaking in least_loaded_cpu().
static ROUND_ROBIN_COUNTER: AtomicU32 = AtomicU32::new(0);

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

/// Per-CPU kernel stack bounds for real-time overflow detection.
/// Updated in schedule_inner when switching threads. Read by the timer ISR
/// (lock-free) to detect stack overflow BEFORE it corrupts adjacent memory.
static PER_CPU_STACK_BOTTOM: [AtomicU64; MAX_CPUS] = {
    const INIT: AtomicU64 = AtomicU64::new(0);
    [INIT; MAX_CPUS]
};
static PER_CPU_STACK_TOP: [AtomicU64; MAX_CPUS] = {
    const INIT: AtomicU64 = AtomicU64::new(0);
    [INIT; MAX_CPUS]
};

/// Per-CPU scheduling state: current thread (TID) and local ready queue (TIDs).
struct PerCpuState {
    /// TID of the thread currently executing on this CPU, or None if idle.
    current_tid: Option<u32>,
    /// TIDs of threads ready to run on this CPU.
    ready_queue: Vec<u32>,
}

/// Core scheduler state with per-CPU ready queues and TID-based tracking.
///
/// All thread references in queues and current-thread tracking use TIDs (stable,
/// never change) instead of Vec indices (shift on remove). This eliminates the
/// entire class of cross-CPU index corruption bugs.
pub struct Scheduler {
    /// All threads known to the scheduler (running, ready, blocked, terminated).
    /// Each thread is heap-allocated in a `Box` for pointer stability — schedule_inner
    /// extracts raw pointers to CpuContext/FxState under the lock and uses them
    /// after releasing it (for context_switch).
    threads: Vec<Box<Thread>>,
    /// Per-CPU state: current thread TID + local ready queue.
    per_cpu: Vec<PerCpuState>,
    /// Per-CPU idle thread TIDs. Each CPU always has an idle thread as fallback
    /// so that current_tid is never None and TSS.RSP0 always points to a valid stack.
    idle_tid: [u32; MAX_CPUS],
}

/// Idle thread entry point (per-CPU). Just halts until the next interrupt.
extern "C" fn idle_thread_entry() {
    loop {
        unsafe { core::arch::asm!("sti; hlt"); }
    }
}

impl Scheduler {
    fn new() -> Self {
        let mut per_cpu = Vec::with_capacity(MAX_CPUS);
        for _ in 0..MAX_CPUS {
            per_cpu.push(PerCpuState {
                current_tid: None,
                ready_queue: Vec::new(),
            });
        }
        let mut sched = Scheduler {
            threads: Vec::with_capacity(128),
            per_cpu,
            idle_tid: [0; MAX_CPUS],
        };

        // Create per-CPU idle threads. These are real threads with their own
        // kernel stacks, ensuring TSS.RSP0 always points to valid memory.
        // All MAX_CPUS slots are pre-created so schedule_inner() can always
        // find idle_tid[cpu]. Non-online idle threads are filtered in list_threads().
        for cpu in 0..MAX_CPUS {
            let name: &str = match cpu {
                0 => "idle/0", 1 => "idle/1", 2 => "idle/2", 3 => "idle/3",
                4 => "idle/4", 5 => "idle/5", 6 => "idle/6", 7 => "idle/7",
                _ => "idle/?",
            };
            let mut thread = Thread::new(idle_thread_entry, 0, name);
            thread.is_idle = true;
            let tid = thread.tid;
            sched.idle_tid[cpu] = tid;
            sched.threads.push(Box::new(thread));
        }

        sched
    }

    /// Find a thread's index in the threads Vec by TID.
    #[inline]
    fn find_idx(&self, tid: u32) -> Option<usize> {
        self.threads.iter().position(|t| t.tid == tid)
    }

    /// Get the number of active CPUs (at least 1).
    #[inline]
    fn num_cpus(&self) -> usize {
        let n = crate::arch::x86::smp::cpu_count() as usize;
        if n == 0 { 1 } else { n }
    }

    /// Pick the CPU with the shortest ready queue for load balancing.
    /// On ties, round-robin using a monotonic counter to avoid always picking CPU 0.
    fn least_loaded_cpu(&self) -> usize {
        let n = self.num_cpus();
        let mut best_cpu = 0;
        let mut best_len = usize::MAX;
        let mut tie_count = 0u32;
        let rr = ROUND_ROBIN_COUNTER.fetch_add(1, Ordering::Relaxed);
        for cpu in 0..n {
            let len = self.per_cpu[cpu].ready_queue.len();
            if len < best_len {
                best_len = len;
                best_cpu = cpu;
                tie_count = 1;
            } else if len == best_len {
                tie_count += 1;
                // Deterministic round-robin among tied CPUs
                if rr % tie_count == 0 {
                    best_cpu = cpu;
                }
            }
        }
        best_cpu
    }

    /// Enqueue a TID on a specific CPU's ready queue (if not already present).
    fn enqueue_on_cpu(&mut self, tid: u32, cpu_id: usize) {
        let queue = &mut self.per_cpu[cpu_id].ready_queue;
        if !queue.contains(&tid) {
            queue.push(tid);
        }
    }

    /// Remove a TID from ALL per-CPU ready queues (defensive cleanup).
    fn remove_from_all_queues(&mut self, tid: u32) {
        for cpu in 0..MAX_CPUS {
            self.per_cpu[cpu].ready_queue.retain(|&t| t != tid);
        }
    }

    /// Add a thread to the scheduler and enqueue it on the least-loaded CPU.
    fn add_thread(&mut self, mut thread: Thread) -> u32 {
        let tid = thread.tid;
        let cpu = self.least_loaded_cpu();
        thread.last_cpu = cpu;
        self.threads.push(Box::new(thread));
        self.per_cpu[cpu].ready_queue.push(tid);
        tid
    }

    /// Add a thread in Blocked state without putting it in any ready queue.
    /// Used by `spawn_blocked()` to prevent SMP races.
    fn add_thread_blocked(&mut self, mut thread: Thread) -> u32 {
        let tid = thread.tid;
        thread.state = ThreadState::Blocked;
        self.threads.push(Box::new(thread));
        tid
    }

    /// Remove terminated threads whose exit code has been consumed or auto-reaped.
    /// With TID-based queues, no index adjustment is needed — just remove TIDs.
    fn reap_terminated(&mut self) {
        let current_tick = crate::arch::x86::pit::get_ticks();
        let mut i = 0;
        while i < self.threads.len() {
            // Never reap idle threads
            if self.threads[i].is_idle {
                i += 1;
                continue;
            }
            if self.threads[i].state == ThreadState::Terminated {
                // Safety: don't reap if context_switch.asm is still saving to
                // this thread's CpuContext (save_complete == 0 means ASM hasn't
                // finished writing all registers yet).
                if self.threads[i].context.save_complete == 0 {
                    i += 1;
                    continue;
                }
                // Grace period: another CPU may still hold raw pointers to this
                // thread's CpuContext/FxState from a context_switch in progress.
                // 50 ticks (50ms) gives ample time for any in-flight context_switch.
                let min_elapsed = self.threads[i].terminated_at_tick
                    .map(|t| current_tick.wrapping_sub(t) > 50)
                    .unwrap_or(false);
                if !min_elapsed {
                    i += 1;
                    continue;
                }

                let consumed = self.threads[i].exit_code.is_none();
                let auto_reap = self.threads[i].waiting_tid.is_none()
                    && self.threads[i].terminated_at_tick
                        .map(|t| current_tick.wrapping_sub(t) > 200)
                        .unwrap_or(false);

                if consumed || auto_reap {
                    let tid = self.threads[i].tid;
                    // Remove TID from all per-CPU ready queues (defensive)
                    self.remove_from_all_queues(tid);
                    // Clear per-CPU current_tid if it points to this thread (defensive)
                    for cpu in 0..MAX_CPUS {
                        if self.per_cpu[cpu].current_tid == Some(tid) {
                            self.per_cpu[cpu].current_tid = Some(self.idle_tid[cpu]);
                        }
                    }
                    // swap_remove is O(1) — safe because all refs are TID-based
                    self.threads.swap_remove(i);
                    // Don't increment i — check the swapped-in element
                } else {
                    i += 1;
                }
            } else {
                i += 1;
            }
        }
    }

    /// Pick the highest-priority ready thread from this CPU's queue.
    /// If the local queue is empty, steal from the busiest CPU (work stealing).
    fn pick_next(&mut self, cpu_id: usize) -> Option<u32> {
        // Try local queue first
        if let Some(tid) = self.pick_from_queue(cpu_id) {
            return Some(tid);
        }
        // Work stealing: find the CPU with the longest ready queue
        let n = self.num_cpus();
        let mut max_len = 0;
        let mut max_cpu = cpu_id;
        for c in 0..n {
            if c != cpu_id {
                let len = self.per_cpu[c].ready_queue.len();
                if len > max_len {
                    max_len = len;
                    max_cpu = c;
                }
            }
        }
        if max_len > 0 {
            self.pick_from_queue(max_cpu)
        } else {
            None
        }
    }

    /// Pick the highest-priority ready thread from a specific CPU's queue.
    fn pick_from_queue(&mut self, cpu_id: usize) -> Option<u32> {
        let mut best_tid: Option<u32> = None;
        let mut best_pri: u8 = 0;
        let mut best_pos: usize = 0;

        for (pos, &tid) in self.per_cpu[cpu_id].ready_queue.iter().enumerate() {
            if let Some(idx) = self.find_idx(tid) {
                let thread = &self.threads[idx];
                // Skip threads whose context is still being saved (save_complete == 0)
                if thread.state == ThreadState::Ready && thread.context.save_complete != 0 {
                    if best_tid.is_none() || thread.priority > best_pri {
                        best_tid = Some(tid);
                        best_pri = thread.priority;
                        best_pos = pos;
                    }
                }
            }
        }

        if best_tid.is_some() {
            self.per_cpu[cpu_id].ready_queue.swap_remove(best_pos);
        }
        best_tid
    }

    /// Internal: wake a blocked thread, enqueuing on the least-loaded CPU.
    fn wake_thread_inner(&mut self, tid: u32) {
        if let Some(idx) = self.find_idx(tid) {
            if self.threads[idx].state == ThreadState::Blocked {
                self.threads[idx].state = ThreadState::Ready;
                let target_cpu = self.least_loaded_cpu();
                self.enqueue_on_cpu(tid, target_cpu);
            }
        }
    }
}

// =============================================================================
// Public API
// =============================================================================

/// Initialize the global scheduler.
pub fn init() {
    let mut sched = SCHEDULER.lock();
    *sched = Some(Scheduler::new());
    // Set BSP's current thread to its idle thread — ensures current_tid is never None
    if let Some(s) = sched.as_mut() {
        let idle_tid = s.idle_tid[0];
        s.per_cpu[0].current_tid = Some(idle_tid);
        if let Some(idx) = s.find_idx(idle_tid) {
            s.threads[idx].state = ThreadState::Running;
        }
    }
    crate::serial_println!("[OK] Scheduler initialized (per-CPU queues, {} CPUs max)", MAX_CPUS);
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

/// Spawn a kernel thread in Blocked state (not added to any ready queue).
///
/// The thread will NOT run on any CPU until [`wake_thread`] is called with the
/// returned TID. This prevents SMP races where an AP picks up the thread before
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

/// Create a new thread within the same address space as the currently running thread.
///
/// The new thread shares the caller's page directory (same CR3) and starts executing
/// at `entry_rip` with its stack at `user_rsp`. Returns the TID of the new thread,
/// or 0 on error.
///
/// `priority`: 0 = inherit from parent, 1-255 = explicit priority.
pub fn create_thread_in_current_process(entry_rip: u64, user_rsp: u64, name: &str, priority: u8) -> u32 {
    // Read caller's state under the lock
    let (pd, arch_mode, brk, parent_pri) = {
        let guard = SCHEDULER.lock();
        let cpu_id = get_cpu_id();
        let sched = match guard.as_ref() {
            Some(s) => s,
            None => return 0,
        };
        let current_tid = match sched.per_cpu[cpu_id].current_tid {
            Some(t) => t,
            None => return 0,
        };
        let idx = match sched.find_idx(current_tid) {
            Some(i) => i,
            None => return 0,
        };
        let thread = &sched.threads[idx];
        let pd = match thread.page_directory {
            Some(pd) => pd,
            None => return 0, // Must be a user process
        };
        (pd, thread.arch_mode, thread.brk, thread.priority)
    };

    // Priority: 0 = inherit from parent, otherwise use explicit value
    let effective_pri = if priority == 0 { parent_pri } else { priority };

    // Spawn a new kernel thread in Blocked state (uses the loader's trampoline).
    let tid = spawn_blocked(crate::task::loader::thread_create_trampoline, effective_pri, name);

    // Configure the new thread: share the page directory, mark as user, set pd_shared.
    {
        let mut guard = SCHEDULER.lock();
        let sched = guard.as_mut().expect("Scheduler not initialized");
        if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
            thread.page_directory = Some(pd);
            thread.context.cr3 = pd.as_u64();
            thread.is_user = true;
            thread.brk = brk;
            thread.arch_mode = arch_mode;
            thread.pd_shared = true;
        }
    }

    // Store entry point and user stack in the pending programs table.
    crate::task::loader::store_pending_thread(tid, entry_rip, user_rsp);

    // All setup complete — make the thread schedulable.
    wake_thread(tid);

    tid
}

/// Called from the timer interrupt (PIT or LAPIC) to perform preemptive scheduling.
pub fn schedule_tick() {
    schedule_inner(true);
}

/// Voluntary yield: reschedule without incrementing CPU accounting counters.
pub fn schedule() {
    schedule_inner(false);
}

/// Get the current CPU index, always reading from the LAPIC.
fn get_cpu_id() -> usize {
    let c = crate::arch::x86::smp::current_cpu_id() as usize;
    if c < MAX_CPUS { c } else { 0 }
}

fn schedule_inner(from_timer: bool) {
    // Read cpu_id early for pre-lock counters. For the timer path (from_timer=true),
    // interrupts are already disabled (IRQ handler), so this is always correct.
    // For the voluntary path (from_timer=false), this might be stale if preempted,
    // but we only use it for the early-return counter path (which is from_timer only).
    let cpu_id_early = get_cpu_id();

    // CPU 0 processes deferred PD destruction from remote kill_thread() calls.
    // Done BEFORE acquiring the scheduler lock — destroy_user_page_directory
    // may temporarily switch CR3 and must not be called under the scheduler lock.
    if from_timer && cpu_id_early == 0 {
        let pds = DEFERRED_PD_DESTROY.lock().drain();
        for pd in pds.iter().flatten() {
            crate::memory::virtual_mem::destroy_user_page_directory(*pd);
        }
    }

    // Extract context switch parameters under the lock, then release before switching
    let switch_info: Option<(*mut CpuContext, *const CpuContext, *mut u8, *const u8)>;
    // Saved for TSS.RSP0 recovery in post-lock error paths
    let mut prev_tid: Option<u32> = None;
    let mut prev_kstack_top: u64 = 0;

    // Only increment counters on timer-driven scheduling
    if from_timer {
        TOTAL_SCHED_TICKS.fetch_add(1, Ordering::Relaxed);
        PER_CPU_TOTAL[cpu_id_early].fetch_add(1, Ordering::Relaxed);
    }

    // For timer-driven scheduling (from_timer=true), use try_lock to avoid
    // blocking the IRQ handler — if the lock is contended, skip this tick.
    // For voluntary scheduling (from_timer=false, e.g. sleep_until), the
    // current thread is already marked Blocked and MUST be descheduled.
    // Using try_lock here would return immediately on contention, leaving
    // a Blocked thread running — causing sleep(5) to return instantly on SMP.
    // Blocking lock() is safe: voluntary callers aren't in interrupt context,
    // and lock() disables IF before spinning, preventing deadlock.
    let mut guard = if from_timer {
        match SCHEDULER.try_lock() {
            Some(s) => s,
            None => {
                // Lock contended — still count idle if this CPU has no running thread
                if !PER_CPU_HAS_THREAD[cpu_id_early].load(Ordering::Relaxed) {
                    IDLE_SCHED_TICKS.fetch_add(1, Ordering::Relaxed);
                    PER_CPU_IDLE[cpu_id_early].fetch_add(1, Ordering::Relaxed);
                }
                return;
            }
        }
    } else {
        SCHEDULER.lock()
    };

    // CRITICAL: Re-read CPU ID now that interrupts are disabled (lock acquired).
    // For voluntary schedule() calls via INT 0x80 (IF=1), a timer IRQ could have
    // preempted us between get_cpu_id() above and try_lock(), migrating this thread
    // to a different CPU. Using the stale cpu_id would corrupt another CPU's
    // per-CPU state (current_tid, ready_queue, TSS.RSP0) — causing the exact
    // "SCHED FIX" TID/RSP mismatch symptoms.
    let cpu_id = get_cpu_id();

    {
        let sched = match guard.as_mut() {
            Some(s) => s,
            None => return, // guard drops normally (restores IF)
        };

        // CPU 0 reaps terminated threads
        if cpu_id == 0 {
            sched.reap_terminated();
        }

        // All CPUs wake sleepers whose timer has expired, distributing to least-loaded CPU.
        // Previously only CPU 0 did this, which funneled all woken threads back to CPU 0.
        if from_timer {
            let current_tick = crate::arch::x86::pit::get_ticks();
            for i in 0..sched.threads.len() {
                if sched.threads[i].state == ThreadState::Blocked {
                    if let Some(wake_tick) = sched.threads[i].wake_at_tick {
                        if current_tick.wrapping_sub(wake_tick) < 0x8000_0000 {
                            let tid = sched.threads[i].tid;
                            let target_cpu = sched.least_loaded_cpu();
                            sched.threads[i].state = ThreadState::Ready;
                            sched.threads[i].wake_at_tick = None;
                            sched.enqueue_on_cpu(tid, target_cpu);
                        }
                    }
                }
            }
        }

        // Track CPU ticks for the currently running thread on THIS CPU
        if from_timer {
            if let Some(current_tid) = sched.per_cpu[cpu_id].current_tid {
                if current_tid == sched.idle_tid[cpu_id] {
                    // Idle thread running = CPU is idle
                    IDLE_SCHED_TICKS.fetch_add(1, Ordering::Relaxed);
                    PER_CPU_IDLE[cpu_id].fetch_add(1, Ordering::Relaxed);
                } else if let Some(idx) = sched.find_idx(current_tid) {
                    if sched.threads[idx].state == ThreadState::Running {
                        sched.threads[idx].cpu_ticks += 1;
                    }
                }
            }
        }

        // RSP validation: verify our actual RSP matches the expected thread.
        // With per-CPU TID-based queues this should never fire, but serves as
        // a safety net against context corruption.
        let current_rsp: u64;
        unsafe { core::arch::asm!("mov {}, rsp", out(reg) current_rsp); }

        if let Some(current_tid) = sched.per_cpu[cpu_id].current_tid {
            if let Some(idx) = sched.find_idx(current_tid) {
                let bottom = sched.threads[idx].kernel_stack_bottom();
                let top = sched.threads[idx].kernel_stack_top();
                if current_rsp < bottom || current_rsp > top {
                    // Mismatch! Find the actual thread by RSP scan.
                    let mut actual_tid = None;
                    for t in sched.threads.iter() {
                        let b = t.kernel_stack_bottom();
                        let tt = t.kernel_stack_top();
                        if current_rsp >= b && current_rsp <= tt {
                            actual_tid = Some(t.tid);
                            break;
                        }
                    }
                    if let Some(at) = actual_tid {
                        crate::serial_println!(
                            "SCHED FIX: CPU{} current_tid={} but RSP {:#x} in TID={} — correcting",
                            cpu_id, current_tid, current_rsp, at,
                        );
                        if let Some(ai) = sched.find_idx(at) {
                            if sched.threads[ai].state == ThreadState::Terminated {
                                // Don't resurrect a dead thread — this happens when
                                // bad_rsp_recovery() killed a thread but RSP is still
                                // on its stack (before the RSP switch to idle stack).
                                // Fall back to idle instead.
                                crate::serial_println!(
                                    "SCHED FIX: TID={} is Terminated — using idle instead",
                                    at,
                                );
                                sched.per_cpu[cpu_id].current_tid = Some(sched.idle_tid[cpu_id]);
                            } else {
                                sched.per_cpu[cpu_id].current_tid = Some(at);
                                if sched.threads[ai].state != ThreadState::Running {
                                    sched.threads[ai].state = ThreadState::Running;
                                }
                                sched.threads[ai].last_cpu = cpu_id;
                            }
                        } else {
                            // Thread was reaped — RSP is on freed memory, use idle
                            sched.per_cpu[cpu_id].current_tid = Some(sched.idle_tid[cpu_id]);
                        }
                    } else {
                        // RSP not in any thread — we're on idle/boot stack
                        sched.per_cpu[cpu_id].current_tid = Some(sched.idle_tid[cpu_id]);
                    }
                }
            } else {
                // current_tid not found (thread was reaped?) — fall back to idle
                sched.per_cpu[cpu_id].current_tid = Some(sched.idle_tid[cpu_id]);
            }
        }

        // Put current thread back to Ready on THIS CPU's queue.
        // Mark save_complete = 0 so other CPUs won't pick it until
        // context_switch.asm finishes saving its registers.
        if let Some(current_tid) = sched.per_cpu[cpu_id].current_tid {
            // Idle threads are never enqueued in ready queues
            if current_tid != sched.idle_tid[cpu_id] {
                if let Some(idx) = sched.find_idx(current_tid) {
                    if sched.threads[idx].state == ThreadState::Running {
                        sched.threads[idx].context.save_complete = 0;
                        sched.threads[idx].state = ThreadState::Ready;
                        sched.threads[idx].last_cpu = cpu_id;
                        sched.enqueue_on_cpu(current_tid, cpu_id);
                    }
                }
            }
        }

        prev_tid = sched.per_cpu[cpu_id].current_tid;

        // Save previous thread's kernel stack top for TSS.RSP0 recovery.
        // If anything goes wrong during scheduling, we can restore TSS.RSP0
        // to this value to prevent cascading corruption.
        prev_kstack_top = if let Some(pt) = prev_tid {
            if let Some(pi) = sched.find_idx(pt) {
                sched.threads[pi].kernel_stack_top()
            } else { 0 }
        } else { 0 };

        // Pick next thread for THIS CPU (local queue first, then work stealing)
        switch_info = if let Some(next_tid) = sched.pick_next(cpu_id) {
            if let Some(next_idx) = sched.find_idx(next_tid) {
                // === VALIDATE candidate BEFORE committing per-CPU state ===
                // This prevents TSS.RSP0 corruption: if we set TSS.RSP0 to
                // the next thread's stack and then discover its context is
                // corrupt, the previous thread would be left with wrong
                // TSS.RSP0 → cascading corruption on next Ring 3→0 transition.
                let kstack_top = sched.threads[next_idx].kernel_stack_top();
                let kstack_bottom = sched.threads[next_idx].kernel_stack_bottom();
                let next_rsp = sched.threads[next_idx].context.rsp;
                let next_rip = sched.threads[next_idx].context.rip;

                let rsp_valid = next_rsp >= kstack_bottom && next_rsp <= kstack_top;
                let rip_valid = next_rip >= 0xFFFF_FFFF_8010_0000
                             && next_rip < 0xFFFF_FFFF_C000_0000;

                if !rsp_valid || !rip_valid {
                    // Kill the corrupt thread WITHOUT modifying TSS.RSP0
                    crate::serial_println!(
                        "BUG: thread '{}' (TID={}) corrupt context RSP={:#018x} RIP={:#018x} (stack=[{:#018x}..{:#018x}]) — killing (CPU{})",
                        sched.threads[next_idx].name_str(), next_tid,
                        next_rsp, next_rip, kstack_bottom, kstack_top, cpu_id,
                    );
                    sched.threads[next_idx].state = ThreadState::Terminated;
                    sched.threads[next_idx].exit_code = Some(139);
                    sched.threads[next_idx].terminated_at_tick =
                        Some(crate::arch::x86::pit::get_ticks());
                    // Restore prev thread — TSS.RSP0 was NOT changed
                    sched.per_cpu[cpu_id].current_tid = prev_tid;
                    PER_CPU_HAS_THREAD[cpu_id].store(prev_tid.is_some(), Ordering::Relaxed);
                    if let Some(pt) = prev_tid {
                        if let Some(pi) = sched.find_idx(pt) {
                            sched.threads[pi].context.save_complete = 1;
                            PER_CPU_CURRENT_TID[cpu_id].store(pt, Ordering::Relaxed);
                            PER_CPU_IS_USER[cpu_id].store(sched.threads[pi].is_user, Ordering::Relaxed);
                        }
                    }
                    None
                } else {
                    // === Candidate validated — now commit per-CPU state ===
                    sched.per_cpu[cpu_id].current_tid = Some(next_tid);
                    sched.threads[next_idx].state = ThreadState::Running;
                    sched.threads[next_idx].last_cpu = cpu_id;
                    PER_CPU_HAS_THREAD[cpu_id].store(true, Ordering::Relaxed);
                    PER_CPU_CURRENT_TID[cpu_id].store(next_tid, Ordering::Relaxed);
                    PER_CPU_IS_USER[cpu_id].store(sched.threads[next_idx].is_user, Ordering::Relaxed);

                    // Update this CPU's TSS RSP0 and SYSCALL per-CPU kernel RSP
                    crate::arch::x86::tss::set_kernel_stack_for_cpu(cpu_id, kstack_top);
                    crate::arch::x86::syscall_msr::set_kernel_rsp(cpu_id, kstack_top);
                    PER_CPU_STACK_BOTTOM[cpu_id].store(kstack_bottom, Ordering::Relaxed);
                    PER_CPU_STACK_TOP[cpu_id].store(kstack_top, Ordering::Relaxed);

                    // Check stack canary for the outgoing thread
                    if let Some(pt) = prev_tid {
                        if let Some(pi) = sched.find_idx(pt) {
                            if !sched.threads[pi].check_stack_canary() {
                                crate::serial_println!(
                                    "STACK OVERFLOW: thread '{}' (TID={}) canary destroyed! CPU{} — killing",
                                    sched.threads[pi].name_str(), pt, cpu_id,
                                );
                                sched.threads[pi].state = ThreadState::Terminated;
                                sched.threads[pi].exit_code = Some(139);
                                sched.threads[pi].terminated_at_tick =
                                    Some(crate::arch::x86::pit::get_ticks());
                                // Remove from queue (we just enqueued it)
                                sched.per_cpu[cpu_id].ready_queue.retain(|&t| t != pt);
                            }
                        }
                    }

                    if let Some(pt) = prev_tid {
                        if pt != next_tid {
                            // Different thread — context switch from prev to next
                            if let Some(prev_idx) = sched.find_idx(pt) {
                                let old_ctx = &mut sched.threads[prev_idx].context as *mut CpuContext;
                                let new_ctx = &sched.threads[next_idx].context as *const CpuContext;
                                let old_fpu = sched.threads[prev_idx].fpu_state.data.as_mut_ptr();
                                let new_fpu = sched.threads[next_idx].fpu_state.data.as_ptr();
                                Some((old_ctx, new_ctx, old_fpu, new_fpu))
                            } else {
                                // prev thread gone (reaped) — switch from idle thread
                                let idle_i = sched.find_idx(sched.idle_tid[cpu_id]).unwrap();
                                let idle_ctx = &mut sched.threads[idle_i].context as *mut CpuContext;
                                let new_ctx = &sched.threads[next_idx].context as *const CpuContext;
                                let old_fpu = sched.threads[idle_i].fpu_state.data.as_mut_ptr();
                                let new_fpu = sched.threads[next_idx].fpu_state.data.as_ptr();
                                Some((idle_ctx, new_ctx, old_fpu, new_fpu))
                            }
                        } else {
                            // Same thread — no switch needed, restore save_complete
                            sched.threads[next_idx].context.save_complete = 1;
                            None
                        }
                    } else {
                        // No previous thread — switch from idle thread
                        let idle_i = sched.find_idx(sched.idle_tid[cpu_id]).unwrap();
                        let idle_ctx = &mut sched.threads[idle_i].context as *mut CpuContext;
                        let new_ctx = &sched.threads[next_idx].context as *const CpuContext;
                        let old_fpu = sched.threads[idle_i].fpu_state.data.as_mut_ptr();
                        let new_fpu = sched.threads[next_idx].fpu_state.data.as_ptr();
                        Some((idle_ctx, new_ctx, old_fpu, new_fpu))
                    }
                }
            } else {
                // find_idx returned None — TID was reaped between pick_next and here.
                // TSS.RSP0 was NOT modified, so no corruption risk.
                if let Some(pt) = prev_tid {
                    if let Some(pi) = sched.find_idx(pt) {
                        sched.threads[pi].context.save_complete = 1;
                    }
                }
                sched.per_cpu[cpu_id].current_tid = prev_tid;
                None
            }
        } else {
            // No ready threads — this CPU is idle
            if from_timer {
                IDLE_SCHED_TICKS.fetch_add(1, Ordering::Relaxed);
                PER_CPU_IDLE[cpu_id].fetch_add(1, Ordering::Relaxed);
            }

            // If the current thread is no longer runnable, switch to idle thread
            if let Some(current_tid) = sched.per_cpu[cpu_id].current_tid {
                if let Some(idx) = sched.find_idx(current_tid) {
                    if sched.threads[idx].state != ThreadState::Running {
                        // Mark save pending — thread could be woken while being saved
                        sched.threads[idx].context.save_complete = 0;
                        // Switch to this CPU's idle thread
                        let idle_tid = sched.idle_tid[cpu_id];
                        let idle_i = sched.find_idx(idle_tid).unwrap();
                        sched.per_cpu[cpu_id].current_tid = Some(idle_tid);
                        sched.threads[idle_i].state = ThreadState::Running;
                        PER_CPU_HAS_THREAD[cpu_id].store(true, Ordering::Relaxed);
                        PER_CPU_IS_USER[cpu_id].store(false, Ordering::Relaxed);
                        PER_CPU_CURRENT_TID[cpu_id].store(idle_tid, Ordering::Relaxed);
                        let idle_kstack_top = sched.threads[idle_i].kernel_stack_top();
                        crate::arch::x86::tss::set_kernel_stack_for_cpu(cpu_id, idle_kstack_top);
                        crate::arch::x86::syscall_msr::set_kernel_rsp(cpu_id, idle_kstack_top);
                        PER_CPU_STACK_BOTTOM[cpu_id].store(sched.threads[idle_i].kernel_stack_bottom(), Ordering::Relaxed);
                        PER_CPU_STACK_TOP[cpu_id].store(idle_kstack_top, Ordering::Relaxed);
                        let old_ctx = &mut sched.threads[idx].context as *mut CpuContext;
                        let idle_ctx = &sched.threads[idle_i].context as *const CpuContext;
                        let old_fpu = sched.threads[idx].fpu_state.data.as_mut_ptr();
                        let new_fpu = sched.threads[idle_i].fpu_state.data.as_ptr();
                        Some((old_ctx, idle_ctx, old_fpu, new_fpu))
                    } else {
                        // Current thread still Running (idle or real), no switch needed
                        if !sched.threads[idx].is_idle {
                            sched.threads[idx].context.save_complete = 1;
                        }
                        None
                    }
                } else {
                    // Thread not found — fall back to idle thread
                    let idle_tid = sched.idle_tid[cpu_id];
                    let idle_i = sched.find_idx(idle_tid).unwrap();
                    sched.per_cpu[cpu_id].current_tid = Some(idle_tid);
                    PER_CPU_HAS_THREAD[cpu_id].store(true, Ordering::Relaxed);
                    PER_CPU_IS_USER[cpu_id].store(false, Ordering::Relaxed);
                    PER_CPU_CURRENT_TID[cpu_id].store(idle_tid, Ordering::Relaxed);
                    // CRITICAL: Set TSS.RSP0 to idle thread's stack — without this,
                    // TSS.RSP0 points to the reaped thread's freed stack.
                    let idle_kstack_top = sched.threads[idle_i].kernel_stack_top();
                    crate::arch::x86::tss::set_kernel_stack_for_cpu(cpu_id, idle_kstack_top);
                    crate::arch::x86::syscall_msr::set_kernel_rsp(cpu_id, idle_kstack_top);
                    PER_CPU_STACK_BOTTOM[cpu_id].store(sched.threads[idle_i].kernel_stack_bottom(), Ordering::Relaxed);
                    PER_CPU_STACK_TOP[cpu_id].store(idle_kstack_top, Ordering::Relaxed);
                    None
                }
            } else {
                // No current thread — set to idle (shouldn't happen)
                let idle_tid = sched.idle_tid[cpu_id];
                let idle_i = sched.find_idx(idle_tid).unwrap();
                sched.per_cpu[cpu_id].current_tid = Some(idle_tid);
                // Set TSS.RSP0 to idle thread's stack
                let idle_kstack_top = sched.threads[idle_i].kernel_stack_top();
                crate::arch::x86::tss::set_kernel_stack_for_cpu(cpu_id, idle_kstack_top);
                crate::arch::x86::syscall_msr::set_kernel_rsp(cpu_id, idle_kstack_top);
                None
            }
        };
        // sched borrow ends here
    }

    // CRITICAL: Release the lock WITHOUT restoring IF. This keeps interrupts
    // disabled from lock acquisition all the way through context_switch.
    guard.release_no_irq_restore();

    // Context switch with the lock released AND interrupts still disabled.
    if let Some((old_ctx, new_ctx, old_fpu, new_fpu)) = switch_info {
        // Post-lock safety check: validate RIP hasn't been corrupted since
        // the in-lock check (e.g., by heap corruption or wild pointer).
        let new_rip = unsafe { (*new_ctx).rip };
        let new_rsp = unsafe { (*new_ctx).rsp };
        let new_cr3 = unsafe { (*new_ctx).cr3 };
        if new_rip < 0xFFFF_FFFF_8010_0000 || new_rip >= 0xFFFF_FFFF_C000_0000 {
            crate::serial_println!(
                "BUG: context_switch to bad RIP={:#018x} RSP={:#018x} CR3={:#018x} CPU{}",
                new_rip, new_rsp, new_cr3, cpu_id,
            );
            unsafe { (*old_ctx).save_complete = 1; }
            // Recover: kill the bad thread AND restore TSS.RSP0
            {
                if let Some(mut guard) = SCHEDULER.try_lock() {
                    if let Some(sched) = guard.as_mut() {
                        if let Some(tid) = sched.per_cpu[cpu_id].current_tid {
                            if let Some(idx) = sched.find_idx(tid) {
                                sched.threads[idx].state = ThreadState::Terminated;
                                sched.threads[idx].terminated_at_tick =
                                    Some(crate::arch::x86::pit::get_ticks());
                            }
                        }
                        // CRITICAL: Restore TSS.RSP0 to prev thread's stack.
                        // Without this, the prev thread runs with TSS.RSP0
                        // pointing to the killed thread's stack — cascading
                        // corruption on next Ring 3→0 transition.
                        sched.per_cpu[cpu_id].current_tid = prev_tid;
                        if prev_kstack_top >= 0xFFFF_FFFF_8000_0000 {
                            crate::arch::x86::tss::set_kernel_stack_for_cpu(cpu_id, prev_kstack_top);
                            crate::arch::x86::syscall_msr::set_kernel_rsp(cpu_id, prev_kstack_top);
                        }
                        if let Some(pt) = prev_tid {
                            PER_CPU_CURRENT_TID[cpu_id].store(pt, Ordering::Relaxed);
                        }
                    }
                }
            }
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

    // Re-enable interrupts. For voluntary schedule() calls this is CRITICAL —
    // without it, IF stays 0 permanently and sleep() loops forever.
    unsafe { core::arch::asm!("sti"); }
}

// =============================================================================
// Current thread accessors (TID-based lookup)
// =============================================================================

/// Get the current thread's TID (on the calling CPU).
pub fn current_tid() -> u32 {
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id(); // after lock — interrupts disabled, can't migrate
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
        if let Some(tid) = sched.per_cpu[cpu_id].current_tid {
            if let Some(idx) = sched.find_idx(tid) {
                return sched.threads[idx].is_user;
            }
        }
    }
    false
}

/// Get the current thread's name (for diagnostic messages).
pub fn current_thread_name() -> [u8; 32] {
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        if let Some(tid) = sched.per_cpu[cpu_id].current_tid {
            if let Some(idx) = sched.find_idx(tid) {
                return sched.threads[idx].name;
            }
        }
    }
    [0u8; 32]
}

/// Lock-free read of the last-scheduled thread TID on this CPU.
pub fn debug_current_tid() -> u32 {
    let cpu_id = crate::arch::x86::smp::current_cpu_id() as usize;
    if cpu_id < MAX_CPUS {
        PER_CPU_CURRENT_TID[cpu_id].load(Ordering::Relaxed)
    } else {
        0
    }
}

/// Lock-free check: is the current thread on this CPU a user process?
pub fn debug_is_current_user() -> bool {
    let cpu_id = crate::arch::x86::smp::current_cpu_id() as usize;
    PER_CPU_IS_USER[cpu_id].load(Ordering::Relaxed)
}

/// Lock-free check: does this CPU have an active thread running?
pub fn cpu_has_active_thread(cpu_id: usize) -> bool {
    if cpu_id < MAX_CPUS {
        PER_CPU_HAS_THREAD[cpu_id].load(Ordering::Relaxed)
    } else {
        false
    }
}

/// Check the current thread's stack canary after a syscall completes.
pub fn check_current_stack_canary(syscall_num: u32) {
    let mut guard = SCHEDULER.lock();
    let sched = match guard.as_mut() {
        Some(s) => s,
        None => return,
    };
    let cpu_id = crate::arch::x86::smp::current_cpu_id() as usize;
    let tid = match sched.per_cpu[cpu_id].current_tid {
        Some(t) => t,
        None => return,
    };
    let idx = match sched.find_idx(tid) {
        Some(i) => i,
        None => return,
    };
    if !sched.threads[idx].check_stack_canary() {
        let name = sched.threads[idx].name_str();
        crate::serial_println!(
            "STACK OVERFLOW detected after syscall {} in thread '{}' (TID={}) — killing",
            syscall_num, name, tid,
        );
        sched.threads[idx].state = ThreadState::Terminated;
        sched.threads[idx].exit_code = Some(139);
        sched.threads[idx].terminated_at_tick = Some(crate::arch::x86::pit::get_ticks());
    }
}

/// Lock-free check: is the given RSP within this CPU's current thread's kernel stack?
pub fn check_rsp_in_bounds(cpu_id: usize, rsp: u64) -> bool {
    let bottom = PER_CPU_STACK_BOTTOM[cpu_id].load(Ordering::Relaxed);
    let top = PER_CPU_STACK_TOP[cpu_id].load(Ordering::Relaxed);
    if bottom == 0 || top == 0 {
        return true; // Not initialized yet
    }
    rsp >= bottom && rsp <= top
}

/// Get the per-CPU stack bounds (lock-free, for diagnostics).
pub fn get_stack_bounds(cpu_id: usize) -> (u64, u64) {
    let bottom = PER_CPU_STACK_BOTTOM[cpu_id].load(Ordering::Relaxed);
    let top = PER_CPU_STACK_TOP[cpu_id].load(Ordering::Relaxed);
    (bottom, top)
}

// =============================================================================
// Thread configuration
// =============================================================================

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
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        if let Some(tid) = sched.per_cpu[cpu_id].current_tid {
            if let Some(idx) = sched.find_idx(tid) {
                return sched.threads[idx].page_directory;
            }
        }
    }
    None
}

/// Check if the current thread has a shared page directory (intra-process child).
pub fn current_thread_pd_shared() -> bool {
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        if let Some(tid) = sched.per_cpu[cpu_id].current_tid {
            if let Some(idx) = sched.find_idx(tid) {
                return sched.threads[idx].pd_shared;
            }
        }
    }
    false
}

/// Check if any OTHER live thread shares the same page directory as the current thread.
/// Used by sys_exit to decide whether it's safe to destroy the PD.
pub fn has_live_pd_siblings() -> bool {
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        if let Some(tid) = sched.per_cpu[cpu_id].current_tid {
            if let Some(idx) = sched.find_idx(tid) {
                if let Some(pd) = sched.threads[idx].page_directory {
                    return sched.threads.iter().any(|t| {
                        t.tid != tid
                            && t.page_directory == Some(pd)
                            && t.state != ThreadState::Terminated
                    });
                }
            }
        }
    }
    false
}

/// Atomically get all info needed for sys_exit in a single lock acquisition.
/// Returns (tid, pd, can_destroy_pd) — eliminates TOCTOU races from
/// calling current_tid / current_thread_page_directory / pd_shared / has_live_pd_siblings
/// as separate locked operations with timer-preemptible gaps.
pub fn current_exit_info() -> (u32, Option<PhysAddr>, bool) {
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        if let Some(tid) = sched.per_cpu[cpu_id].current_tid {
            if let Some(idx) = sched.find_idx(tid) {
                let pd = sched.threads[idx].page_directory;
                let pd_shared = sched.threads[idx].pd_shared;
                let has_siblings = if let Some(pd_addr) = pd {
                    pd_shared || sched.threads.iter().any(|t| {
                        t.tid != tid
                            && t.page_directory == Some(pd_addr)
                            && t.state != ThreadState::Terminated
                    })
                } else {
                    false
                };
                let can_destroy = pd.is_some() && !pd_shared && !has_siblings;
                return (tid, pd, can_destroy);
            }
        }
    }
    (0, None, false)
}

/// Get the current thread's program break address.
pub fn current_thread_brk() -> u32 {
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        if let Some(tid) = sched.per_cpu[cpu_id].current_tid {
            if let Some(idx) = sched.find_idx(tid) {
                return sched.threads[idx].brk;
            }
        }
    }
    0
}

/// Set the current thread's program break address, and sync across sibling threads.
pub fn set_current_thread_brk(brk: u32) {
    let mut guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_mut() {
        if let Some(tid) = sched.per_cpu[cpu_id].current_tid {
            if let Some(idx) = sched.find_idx(tid) {
                sched.threads[idx].brk = brk;
                // Sync brk across all sibling threads sharing the same PD
                if let Some(pd) = sched.threads[idx].page_directory {
                    let current_tid = sched.threads[idx].tid;
                    for thread in sched.threads.iter_mut() {
                        if thread.tid != current_tid && thread.page_directory == Some(pd) {
                            thread.brk = brk;
                        }
                    }
                }
            }
        }
    }
}

// =============================================================================
// Thread lifecycle
// =============================================================================

/// Terminate the current thread with an exit code. Wakes any waitpid waiter.
pub fn exit_current(code: u32) {
    let tid;
    {
        let mut guard = SCHEDULER.lock();
        let cpu_id = get_cpu_id();
        let sched = guard.as_mut().expect("Scheduler not initialized");

        tid = sched.per_cpu[cpu_id].current_tid.unwrap_or(0);

        if let Some(current_tid) = sched.per_cpu[cpu_id].current_tid {
            if let Some(idx) = sched.find_idx(current_tid) {
                sched.threads[idx].state = ThreadState::Terminated;
                sched.threads[idx].exit_code = Some(code);
                sched.threads[idx].terminated_at_tick =
                    Some(crate::arch::x86::pit::get_ticks());
                sched.threads[idx].page_directory = None;

                // Wake any thread waiting on us
                if let Some(waiter_tid) = sched.threads[idx].waiting_tid {
                    sched.wake_thread_inner(waiter_tid);
                }
            }
        }
    }

    crate::ipc::event_bus::system_emit(crate::ipc::event_bus::EventData::new(
        crate::ipc::event_bus::EVT_PROCESS_EXITED, tid, code, 0, 0,
    ));

    schedule();
    loop { unsafe { core::arch::asm!("hlt"); } }
}

/// Try to terminate the current thread using `try_lock` (non-blocking).
/// Returns `false` if the scheduler lock could not be acquired.
pub fn try_exit_current(code: u32) -> bool {
    let tid;
    {
        let mut guard = match SCHEDULER.try_lock() {
            Some(g) => g,
            None => return false,
        };
        let cpu_id = get_cpu_id(); // after lock — interrupts disabled
        let sched = match guard.as_mut() {
            Some(s) => s,
            None => return false,
        };

        if let Some(current_tid) = sched.per_cpu[cpu_id].current_tid {
            tid = current_tid;
            if let Some(idx) = sched.find_idx(current_tid) {
                sched.threads[idx].state = ThreadState::Terminated;
                sched.threads[idx].exit_code = Some(code);
                sched.threads[idx].terminated_at_tick =
                    Some(crate::arch::x86::pit::get_ticks());
                sched.threads[idx].page_directory = None;

                if let Some(waiter_tid) = sched.threads[idx].waiting_tid {
                    sched.wake_thread_inner(waiter_tid);
                }
            } else {
                return false;
            }
        } else {
            return false;
        }
    }

    // Notify compositor (and other listeners) so windows get cleaned up
    crate::ipc::event_bus::system_emit(crate::ipc::event_bus::EventData::new(
        crate::ipc::event_bus::EVT_PROCESS_EXITED, tid, code, 0, 0,
    ));

    schedule();
    loop { unsafe { core::arch::asm!("hlt"); } }
}

/// Recovery function called from interrupts.asm when an ISR/IRQ fires with a
/// corrupt RSP (TSS.RSP0 was bad on Ring 3→0 transition). By the time we get
/// here, the ASM stub has already switched RSP to the per-CPU kernel_rsp via
/// SWAPGS. We kill the faulting thread, repair TSS.RSP0, send LAPIC EOI, and
/// enter the idle loop. This function never returns.
///
/// IMPORTANT: We must NOT call schedule() or try_exit_current() here because
/// those call context_switch, and our stack frame is the PERCPU kernel_rsp
/// (not a normal call chain). Instead, we mark the thread as Terminated,
/// repair per-CPU state, and enter the idle loop. The next timer tick will
/// call schedule() normally.
#[no_mangle]
pub extern "C" fn bad_rsp_recovery() -> ! {
    let cpu_id = crate::arch::x86::smp::current_cpu_id() as usize;
    crate::serial_println!(
        "!RSP RECOVERY on CPU {} — killing current thread, entering idle",
        cpu_id
    );

    // Send EOI to LAPIC unconditionally — we may have entered from an IRQ,
    // and without EOI the LAPIC won't deliver further interrupts to this CPU.
    crate::arch::x86::apic::eoi();

    // Capture the idle thread's kernel stack top so we can switch RSP to it
    // after releasing the lock. Without this, the idle loop runs on the dead
    // thread's stack — causing SCHED FIX to resurrect the terminated thread.
    let mut idle_stack_top: u64 = 0;

    // Try to mark the current thread as terminated and repair TSS.RSP0
    // in a single lock acquisition. If the lock is contended, we still
    // enter idle (the scheduler will clean up on the next tick).
    {
        if let Some(mut guard) = SCHEDULER.try_lock() {
            if let Some(ref mut sched) = *guard {
                // Kill the current thread
                if let Some(current_tid) = sched.per_cpu[cpu_id].current_tid {
                    if let Some(idx) = sched.find_idx(current_tid) {
                        if !sched.threads[idx].is_idle {
                            sched.threads[idx].state = ThreadState::Terminated;
                            sched.threads[idx].exit_code = Some(139); // SIGSEGV
                            sched.threads[idx].terminated_at_tick =
                                Some(crate::arch::x86::pit::get_ticks());
                            // Wake any waiter
                            if let Some(waiter_tid) = sched.threads[idx].waiting_tid {
                                sched.wake_thread_inner(waiter_tid);
                            }
                            crate::serial_println!(
                                "  Killed tid={} (corrupt RSP)", current_tid
                            );
                        }
                    }
                    // Clear current so scheduler picks fresh next tick
                    sched.per_cpu[cpu_id].current_tid = None;
                }

                // Repair TSS.RSP0 to idle thread's kernel stack
                let idle_tid = sched.idle_tid[cpu_id];
                if let Some(idx) = sched.find_idx(idle_tid) {
                    let kstack_top = sched.threads[idx].kernel_stack_top();
                    crate::arch::x86::tss::set_kernel_stack_for_cpu(cpu_id, kstack_top);
                    crate::arch::x86::syscall_msr::set_kernel_rsp(cpu_id, kstack_top);
                    idle_stack_top = kstack_top;
                    crate::serial_println!(
                        "  RSP0 repaired to idle stack 0x{:x} for CPU {}",
                        kstack_top, cpu_id
                    );
                }
            }
        } else {
            crate::serial_println!("  WARNING: scheduler locked, deferring cleanup");
            // Fall back to per-CPU atomic stack top (lock-free)
            let stack_top = PER_CPU_STACK_TOP[cpu_id].load(Ordering::Relaxed);
            if stack_top >= 0xFFFF_FFFF_8000_0000 {
                crate::arch::x86::tss::set_kernel_stack_for_cpu(cpu_id, stack_top);
                crate::arch::x86::syscall_msr::set_kernel_rsp(cpu_id, stack_top);
                idle_stack_top = stack_top;
            }
        }
    }

    // Update lock-free per-CPU state
    PER_CPU_HAS_THREAD[cpu_id].store(false, Ordering::Relaxed);
    PER_CPU_IS_USER[cpu_id].store(false, Ordering::Relaxed);
    PER_CPU_CURRENT_TID[cpu_id].store(0, Ordering::Relaxed);

    // Switch to kernel CR3 so we're not running on a user page directory
    // that might get destroyed.
    unsafe {
        let kcr3 = crate::memory::virtual_mem::kernel_cr3();
        core::arch::asm!("mov cr3, {}", in(reg) kcr3, options(nostack));
    }

    // CRITICAL: Switch RSP to the idle thread's stack before entering the
    // idle loop. Without this, the loop runs on the killed thread's stack.
    // When reap_terminated() frees that stack, we'd be executing on freed
    // memory. Worse, SCHED FIX detects RSP in the dead thread's range and
    // resurrects it — causing RIP=garbage crashes.
    crate::serial_println!("  CPU {} entering idle loop", cpu_id);
    if idle_stack_top >= 0xFFFF_FFFF_8000_0000 {
        unsafe {
            core::arch::asm!(
                "mov rsp, {0}",
                "sti",
                "2: hlt",
                "jmp 2b",
                in(reg) idle_stack_top,
                options(noreturn)
            );
        }
    } else {
        // Last resort: couldn't determine idle stack. Stay on current stack
        // and hope scheduler fixes it on next tick.
        unsafe { core::arch::asm!("sti"); }
        loop {
            unsafe { core::arch::asm!("hlt"); }
        }
    }
}

/// Kill a thread by TID. Returns 0 on success, u32::MAX on error.
pub fn kill_thread(tid: u32) -> u32 {
    if tid == 0 {
        return u32::MAX;
    }

    let mut pd_to_destroy: Option<PhysAddr> = None;
    let mut is_current = false;
    let mut running_on_other_cpu = false;

    {
        let mut guard = SCHEDULER.lock();
        let cpu_id = get_cpu_id();
        let sched = guard.as_mut().expect("Scheduler not initialized");

        let target_idx = match sched.find_idx(tid) {
            Some(idx) => idx,
            None => return u32::MAX,
        };

        // Protect idle threads (never kill them)
        if sched.threads[target_idx].is_idle {
            return u32::MAX;
        }

        is_current = sched.per_cpu[cpu_id].current_tid == Some(tid);

        // Check if the thread is currently executing on another CPU
        running_on_other_cpu = !is_current && sched.per_cpu.iter().enumerate().any(|(i, cpu)| {
            i != cpu_id && cpu.current_tid == Some(tid)
        });

        sched.threads[target_idx].state = ThreadState::Terminated;
        sched.threads[target_idx].exit_code = Some(u32::MAX - 1);
        sched.threads[target_idx].terminated_at_tick =
            Some(crate::arch::x86::pit::get_ticks());

        // Remove from all ready queues
        sched.remove_from_all_queues(tid);

        // Only destroy PD if this thread owns it (not shared with siblings).
        // Shared PD threads (pd_shared=true) must NOT destroy the PD — siblings
        // are still using it. Also check that no OTHER thread shares this PD.
        if let Some(pd) = sched.threads[target_idx].page_directory {
            if sched.threads[target_idx].pd_shared {
                // Child thread: don't destroy the shared PD
                sched.threads[target_idx].page_directory = None;
            } else {
                // Owner thread: only destroy if no siblings are still alive
                let has_live_siblings = sched.threads.iter().any(|t| {
                    t.tid != tid
                        && t.page_directory == Some(pd)
                        && t.state != ThreadState::Terminated
                });
                if has_live_siblings {
                    // Siblings still running — don't destroy PD yet
                    sched.threads[target_idx].page_directory = None;
                } else {
                    pd_to_destroy = Some(pd);
                    sched.threads[target_idx].page_directory = None;
                }
            }
        }

        // Wake any thread waiting on us
        if let Some(waiter_tid) = sched.threads[target_idx].waiting_tid {
            sched.wake_thread_inner(waiter_tid);
        }

        if is_current {
            sched.per_cpu[cpu_id].current_tid = Some(sched.idle_tid[cpu_id]);
        }
    }

    if let Some(pd) = pd_to_destroy {
        if running_on_other_cpu {
            // CRITICAL: Thread is still executing on another CPU — the other CPU
            // might be mid-syscall reading/writing user pages. Destroying the PD
            // now would corrupt the other CPU's page table walks.
            // Defer destruction to CPU 0's next timer tick (by which time the
            // killed thread will have been descheduled).
            DEFERRED_PD_DESTROY.lock().push(pd);
        } else {
            if is_current {
                let kernel_cr3 = crate::memory::virtual_mem::kernel_cr3();
                unsafe { core::arch::asm!("mov cr3, {}", in(reg) kernel_cr3); }
            }
            crate::memory::virtual_mem::destroy_user_page_directory(pd);
        }
    }

    crate::ipc::event_bus::system_emit(crate::ipc::event_bus::EventData::new(
        crate::ipc::event_bus::EVT_PROCESS_EXITED, tid, u32::MAX - 1, 0, 0,
    ));

    if is_current {
        schedule();
        loop { unsafe { core::arch::asm!("hlt"); } }
    }

    0
}

// =============================================================================
// Waiting / sleeping
// =============================================================================

/// Wait for a thread to terminate and return its exit code.
pub fn waitpid(tid: u32) -> u32 {
    {
        let mut guard = SCHEDULER.lock();
        let cpu_id = get_cpu_id();
        let sched = guard.as_mut().expect("Scheduler not initialized");

        // Check if already terminated
        if let Some(target) = sched.threads.iter_mut().find(|t| t.tid == tid) {
            if target.state == ThreadState::Terminated {
                let code = target.exit_code.unwrap_or(0);
                target.exit_code = None;
                return code;
            }
        } else {
            return u32::MAX;
        }

        // Block the current thread and register as waiter
        if let Some(current_tid) = sched.per_cpu[cpu_id].current_tid {
            if let Some(target) = sched.threads.iter_mut().find(|t| t.tid == tid) {
                target.waiting_tid = Some(current_tid);
            }
            if let Some(idx) = sched.find_idx(current_tid) {
                sched.threads[idx].state = ThreadState::Blocked;
            }
        }
    }

    loop {
        unsafe { core::arch::asm!("sti; hlt"); }

        {
            let mut guard = SCHEDULER.lock();
            if let Some(sched) = guard.as_mut() {
                if let Some(target) = sched.threads.iter_mut().find(|t| t.tid == tid) {
                    if target.state == ThreadState::Terminated {
                        let code = target.exit_code.unwrap_or(0);
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

/// Block the current thread until the given PIT tick count is reached.
pub fn sleep_until(wake_at: u32) {
    {
        let mut guard = SCHEDULER.lock();
        let cpu_id = get_cpu_id();
        let sched = guard.as_mut().expect("Scheduler not initialized");
        if let Some(current_tid) = sched.per_cpu[cpu_id].current_tid {
            if let Some(idx) = sched.find_idx(current_tid) {
                sched.threads[idx].wake_at_tick = Some(wake_at);
                sched.threads[idx].state = ThreadState::Blocked;
            }
        }
    }
    schedule();
}

/// Block the current thread unconditionally (no wake condition).
pub fn block_current_thread() {
    {
        let mut guard = SCHEDULER.lock();
        let cpu_id = get_cpu_id();
        let sched = guard.as_mut().expect("Scheduler not initialized");
        if let Some(current_tid) = sched.per_cpu[cpu_id].current_tid {
            if let Some(idx) = sched.find_idx(current_tid) {
                sched.threads[idx].state = ThreadState::Blocked;
            }
        }
    }
    schedule();
}

// =============================================================================
// Thread args / stdout pipe
// =============================================================================

/// Set command-line arguments for a thread.
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
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        if let Some(tid) = sched.per_cpu[cpu_id].current_tid {
            if let Some(idx) = sched.find_idx(tid) {
                let args = &sched.threads[idx].args;
                let len = args.iter().position(|&b| b == 0).unwrap_or(256);
                let copy_len = len.min(buf.len());
                buf[..copy_len].copy_from_slice(&args[..copy_len]);
                return copy_len;
            }
        }
    }
    0
}

/// Set a thread's stdout pipe ID.
pub fn set_thread_stdout_pipe(tid: u32, pipe_id: u32) {
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.stdout_pipe = pipe_id;
    }
}

/// Get the current thread's stdout pipe ID.
pub fn current_thread_stdout_pipe() -> u32 {
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        if let Some(tid) = sched.per_cpu[cpu_id].current_tid {
            if let Some(idx) = sched.find_idx(tid) {
                return sched.threads[idx].stdout_pipe;
            }
        }
    }
    0
}

// =============================================================================
// Priority / wake / thread info
// =============================================================================

/// Set the priority of a thread by TID.
pub fn set_thread_priority(tid: u32, priority: u8) {
    let mut guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_mut() {
        if let Some(idx) = sched.find_idx(tid) {
            sched.threads[idx].priority = priority;
        }
    }
}

/// Wake a blocked thread by TID, moving it to its last CPU's ready queue.
pub fn wake_thread(tid: u32) {
    let mut guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_mut() {
        sched.wake_thread_inner(tid);
    }
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

/// List all live threads (for `ps` command).
///
/// Collects lightweight data under the scheduler lock (no heap allocation),
/// then builds the Vec outside the lock. This prevents holding the scheduler
/// lock while the heap allocator's lock is acquired — eliminating a lock chain
/// (SCHEDULER → HEAP) that can cause all-CPU deadlock if the heap is contended
/// or its free list is corrupted.
pub fn list_threads() -> Vec<ThreadInfo> {
    // Phase 1: collect raw data under the lock — stack-only, no heap allocs.
    const MAX_SNAP: usize = 64;
    struct ThreadSnap {
        tid: u32,
        priority: u8,
        state: u8,       // 0=ready, 1=running, 2=blocked
        arch_mode: u8,
        cpu_ticks: u32,
        name: [u8; 32],
        name_len: u8,
    }
    let mut buf = [const {
        ThreadSnap { tid: 0, priority: 0, state: 0, arch_mode: 0, cpu_ticks: 0, name: [0; 32], name_len: 0 }
    }; MAX_SNAP];
    let mut count = 0;

    {
        let guard = SCHEDULER.lock();
        if let Some(sched) = guard.as_ref() {
            let online_cpus = crate::arch::x86::smp::cpu_count() as usize;
            for thread in &sched.threads {
                if thread.state == ThreadState::Terminated { continue; }
                // Skip idle threads for CPUs that aren't online
                if thread.is_idle && !sched.idle_tid[..online_cpus].contains(&thread.tid) {
                    continue;
                }
                if count >= MAX_SNAP { break; }
                let state_num = match thread.state {
                    ThreadState::Ready => 0u8,
                    ThreadState::Running => 1,
                    ThreadState::Blocked => 2,
                    ThreadState::Terminated => unreachable!(),
                };
                let name_str = thread.name_str();
                let len = name_str.len().min(32);
                let mut name_buf = [0u8; 32];
                name_buf[..len].copy_from_slice(&name_str.as_bytes()[..len]);
                buf[count] = ThreadSnap {
                    tid: thread.tid,
                    priority: thread.priority,
                    state: state_num,
                    arch_mode: thread.arch_mode as u8,
                    cpu_ticks: thread.cpu_ticks,
                    name: name_buf,
                    name_len: len as u8,
                };
                count += 1;
            }
        }
    } // SCHEDULER lock released here — before any heap allocation

    // Phase 2: build Vec from stack buffer (heap allocations happen here, lock-free).
    let mut result = Vec::with_capacity(count);
    for i in 0..count {
        let snap = &buf[i];
        let state_str = match snap.state {
            0 => "ready",
            1 => "running",
            _ => "blocked",
        };
        let name = alloc::string::String::from(
            core::str::from_utf8(&snap.name[..snap.name_len as usize]).unwrap_or("?")
        );
        result.push(ThreadInfo {
            tid: snap.tid,
            priority: snap.priority,
            state: state_str,
            name,
            cpu_ticks: snap.cpu_ticks,
            arch_mode: snap.arch_mode,
        });
    }
    result
}

/// Check if the scheduler lock is held by the given CPU.
/// Used by the fault handler to detect deadlock before force-recovery.
pub fn is_scheduler_locked_by_cpu(cpu: u32) -> bool {
    SCHEDULER.is_held_by_cpu(cpu)
}

/// Check if the scheduler lock is currently held (by any CPU).
/// Lock-free diagnostic — safe to call from IRQ context.
pub fn is_scheduler_locked() -> bool {
    SCHEDULER.is_locked()
}

/// Force-release the scheduler lock held by a faulting thread.
///
/// Called from the ISR fault handler when a user thread faults while this CPU
/// holds the scheduler lock. Without this, the lock remains permanently held
/// and ALL CPUs deadlock on the next schedule() call.
///
/// After force-releasing, the caller should proceed to kill the faulting thread
/// via `try_exit_current()`, which will re-acquire the lock cleanly.
///
/// # Safety
/// Must only be called when `is_scheduler_locked_by_cpu(cpu)` returns true.
pub unsafe fn force_unlock_scheduler() {
    SCHEDULER.force_unlock();
    crate::serial_println!("  RECOVERED: force-released scheduler lock");
}

/// Enter the scheduler loop (called from kernel_main, becomes BSP idle thread).
pub fn run() -> ! {
    // Set TSS.RSP0 and PERCPU to idle thread's kernel stack
    {
        let guard = SCHEDULER.lock();
        if let Some(sched) = guard.as_ref() {
            let idle_tid = sched.idle_tid[0];
            if let Some(idx) = sched.find_idx(idle_tid) {
                let kstack_top = sched.threads[idx].kernel_stack_top();
                let kstack_bottom = sched.threads[idx].kernel_stack_bottom();
                crate::arch::x86::tss::set_kernel_stack_for_cpu(0, kstack_top);
                crate::arch::x86::syscall_msr::set_kernel_rsp(0, kstack_top);
                PER_CPU_CURRENT_TID[0].store(idle_tid, Ordering::Relaxed);
                PER_CPU_HAS_THREAD[0].store(true, Ordering::Relaxed);
                PER_CPU_STACK_BOTTOM[0].store(kstack_bottom, Ordering::Relaxed);
                PER_CPU_STACK_TOP[0].store(kstack_top, Ordering::Relaxed);
            }
        }
    }
    // Enable hardware watchpoint on TSS.RSP0 for CPU 0 — catches any write
    // (including wild pointers) that corrupts RSP0. #DB handler logs the
    // faulting RIP when the new value is bad.
    crate::arch::x86::tss::enable_rsp0_watchpoint(0);
    unsafe { core::arch::asm!("sti"); }
    loop {
        unsafe { core::arch::asm!("hlt"); }
    }
}

/// Register the idle thread for an AP. Called from ap_entry() before enabling interrupts.
pub fn register_ap_idle(cpu_id: usize) {
    let mut guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_mut() {
        let idle_tid = sched.idle_tid[cpu_id];
        sched.per_cpu[cpu_id].current_tid = Some(idle_tid);
        if let Some(idx) = sched.find_idx(idle_tid) {
            sched.threads[idx].state = ThreadState::Running;
            let kstack_top = sched.threads[idx].kernel_stack_top();
            let kstack_bottom = sched.threads[idx].kernel_stack_bottom();
            crate::arch::x86::tss::set_kernel_stack_for_cpu(cpu_id, kstack_top);
            crate::arch::x86::syscall_msr::set_kernel_rsp(cpu_id, kstack_top);
            PER_CPU_CURRENT_TID[cpu_id].store(idle_tid, Ordering::Relaxed);
            PER_CPU_HAS_THREAD[cpu_id].store(true, Ordering::Relaxed);
            PER_CPU_STACK_BOTTOM[cpu_id].store(kstack_bottom, Ordering::Relaxed);
            PER_CPU_STACK_TOP[cpu_id].store(kstack_top, Ordering::Relaxed);
        }
    }
    crate::serial_println!("  SMP: CPU{} idle thread registered", cpu_id);
    // Enable hardware watchpoint on TSS.RSP0 for this AP
    crate::arch::x86::tss::enable_rsp0_watchpoint(cpu_id);
}
