//! Mach-style preemptive scheduler with per-CPU multi-level priority queues.
//!
//! 128 priority levels (0–127, higher = more important) with O(1) bitmap-indexed
//! thread selection. Each CPU maintains its own set of priority queues. Idle CPUs
//! steal work from the busiest CPU. Lazy FPU/SSE switching via CR0.TS avoids
//! saving/restoring 512 bytes of FXSAVE state on every context switch — only
//! threads that actually use FPU/SSE pay the cost.

use crate::memory::address::PhysAddr;
use crate::sync::spinlock::Spinlock;
use crate::task::context::CpuContext;
use crate::task::thread::{Thread, ThreadState};
use crate::arch::x86::smp::MAX_CPUS;
use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

/// Number of discrete priority levels (Mach-style, like macOS).
const NUM_PRIORITIES: usize = 128;
const MAX_PRIORITY: u8 = (NUM_PRIORITIES - 1) as u8; // 127

/// Clamp priority to valid range [0, 127]. Prints a debug warning if clamped.
#[inline]
fn clamp_priority(priority: u8, context: &str) -> u8 {
    if priority > MAX_PRIORITY {
        crate::serial_println!(
            "  WARN: priority {} > {} clamped to {} ({})",
            priority, MAX_PRIORITY, MAX_PRIORITY, context
        );
        MAX_PRIORITY
    } else {
        priority
    }
}

// =============================================================================
// Global state
// =============================================================================

static SCHEDULER: Spinlock<Option<Scheduler>> = Spinlock::new(None);

/// Deferred page directory destruction queue.
/// When kill_thread() kills a thread running on another CPU, the PD can't be
/// destroyed immediately (the other CPU might still be mid-syscall accessing
/// user pages). Instead, the PD is queued here and destroyed on CPU 0's next
/// timer tick.
struct DeferredPdQueue {
    entries: [Option<PhysAddr>; 16],
}
impl DeferredPdQueue {
    const fn new() -> Self { Self { entries: [None; 16] } }
    fn push(&mut self, pd: PhysAddr) {
        for slot in self.entries.iter_mut() {
            if slot.is_none() { *slot = Some(pd); return; }
        }
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

// =============================================================================
// Per-CPU atomics (lock-free, read by ISR/panic handlers)
// =============================================================================

/// TID of the thread currently running on each CPU.
static PER_CPU_CURRENT_TID: [AtomicU32; MAX_CPUS] = {
    const INIT: AtomicU32 = AtomicU32::new(0);
    [INIT; MAX_CPUS]
};

/// True when the current thread on this CPU is a user process.
static PER_CPU_IS_USER: [AtomicBool; MAX_CPUS] = {
    const INIT: AtomicBool = AtomicBool::new(false);
    [INIT; MAX_CPUS]
};

/// True when a non-idle thread is running on this CPU.
static PER_CPU_HAS_THREAD: [AtomicBool; MAX_CPUS] = {
    const INIT: AtomicBool = AtomicBool::new(false);
    [INIT; MAX_CPUS]
};

/// Per-CPU thread name cache for lock-free crash diagnostics.
static mut PER_CPU_THREAD_NAME: [[u8; 32]; MAX_CPUS] = [[0u8; 32]; MAX_CPUS];

/// Per-CPU kernel stack bounds for real-time overflow detection.
static PER_CPU_STACK_BOTTOM: [AtomicU64; MAX_CPUS] = {
    const INIT: AtomicU64 = AtomicU64::new(0);
    [INIT; MAX_CPUS]
};
static PER_CPU_STACK_TOP: [AtomicU64; MAX_CPUS] = {
    const INIT: AtomicU64 = AtomicU64::new(0);
    [INIT; MAX_CPUS]
};

/// Per-CPU idle thread stack top (set once during init, never changes).
/// Used by recovery paths as a safe fallback stack.
static PER_CPU_IDLE_STACK_TOP: [AtomicU64; MAX_CPUS] = {
    const INIT: AtomicU64 = AtomicU64::new(0);
    [INIT; MAX_CPUS]
};

// --- Lazy FPU per-CPU state ---

/// TID whose FPU/SSE state is currently loaded in this CPU's XMM registers.
/// 0 = no owner (default state after boot or after fxsave).
static PER_CPU_FPU_OWNER: [AtomicU32; MAX_CPUS] = {
    const INIT: AtomicU32 = AtomicU32::new(0);
    [INIT; MAX_CPUS]
};

/// Raw pointer to the current thread's FxState data buffer.
/// Set by schedule_inner, read by the #NM handler (lock-free).
static PER_CPU_FPU_PTR: [AtomicU64; MAX_CPUS] = {
    const INIT: AtomicU64 = AtomicU64::new(0);
    [INIT; MAX_CPUS]
};

// --- Tick counters ---

static TOTAL_SCHED_TICKS: AtomicU32 = AtomicU32::new(0);
static IDLE_SCHED_TICKS: AtomicU32 = AtomicU32::new(0);
static ROUND_ROBIN_COUNTER: AtomicU32 = AtomicU32::new(0);

static PER_CPU_TOTAL: [AtomicU32; MAX_CPUS] = {
    const INIT: AtomicU32 = AtomicU32::new(0);
    [INIT; MAX_CPUS]
};
static PER_CPU_IDLE: [AtomicU32; MAX_CPUS] = {
    const INIT: AtomicU32 = AtomicU32::new(0);
    [INIT; MAX_CPUS]
};
/// Busy ticks accumulated while the scheduler lock was contended.
static PER_CPU_CONTENDED_BUSY: [AtomicU32; MAX_CPUS] = {
    const INIT: AtomicU32 = AtomicU32::new(0);
    [INIT; MAX_CPUS]
};

/// True when this CPU is inside schedule_inner (either timer or voluntary path).
/// Checked by the timer handler to prevent re-entrant schedule_tick() calls.
/// Without this, a timer firing during the voluntary schedule's try_lock loop
/// nests schedule_inner → context_switch, which can corrupt saved contexts
/// and cause deadlocks when the restored thread re-enters the try_lock loop.
static PER_CPU_IN_SCHEDULER: [AtomicBool; MAX_CPUS] = {
    const INIT: AtomicBool = AtomicBool::new(false);
    [INIT; MAX_CPUS]
};

// =============================================================================
// Per-CPU name cache helpers
// =============================================================================

#[inline]
fn update_per_cpu_name(cpu_id: usize, name: &[u8; 32]) {
    unsafe {
        let dst = core::ptr::addr_of_mut!(PER_CPU_THREAD_NAME[cpu_id]);
        core::ptr::write_volatile(dst, *name);
    }
}

#[inline]
fn clear_per_cpu_name(cpu_id: usize) {
    unsafe {
        let dst = core::ptr::addr_of_mut!(PER_CPU_THREAD_NAME[cpu_id]);
        core::ptr::write_volatile(dst, [0u8; 32]);
    }
}

/// Read CPU ID from the LAPIC (always accurate, even after migration).
fn get_cpu_id() -> usize {
    let c = crate::arch::x86::smp::current_cpu_id() as usize;
    if c < MAX_CPUS { c } else { 0 }
}

// =============================================================================
// RunQueue — bitmap-indexed multi-level FIFO priority queue
// =============================================================================

/// Per-CPU multi-level priority queue with O(1) highest-priority lookup.
///
/// 128 priority levels (0 = lowest / idle, 127 = highest / real-time).
/// A 2×u64 bitmap tracks which levels have queued threads. Finding the
/// highest non-empty level is a single `leading_zeros` operation.
struct RunQueue {
    /// One FIFO queue per priority level.
    levels: Vec<VecDeque<u32>>,
    /// Bitmap: bit `p` set ⟺ `levels[p]` is non-empty.
    /// `bits[0]` covers priorities 0–63, `bits[1]` covers 64–127.
    bits: [u64; 2],
}

impl RunQueue {
    fn new() -> Self {
        let mut levels = Vec::with_capacity(NUM_PRIORITIES);
        for _ in 0..NUM_PRIORITIES {
            levels.push(VecDeque::new());
        }
        RunQueue { levels, bits: [0; 2] }
    }

    /// Enqueue a TID at the given priority level (back of FIFO).
    fn enqueue(&mut self, tid: u32, priority: u8) {
        let p = (priority as usize).min(NUM_PRIORITIES - 1);
        // Prevent duplicates (safety net — should not happen in normal operation)
        if !self.levels[p].contains(&tid) {
            self.levels[p].push_back(tid);
            self.bits[p / 64] |= 1u64 << (p % 64);
        }
    }

    /// Dequeue the highest-priority thread (front of its FIFO). O(1) via bitmap.
    fn dequeue_highest(&mut self) -> Option<u32> {
        let p = self.highest_priority()?;
        let tid = self.levels[p].pop_front()?;
        if self.levels[p].is_empty() {
            self.bits[p / 64] &= !(1u64 << (p % 64));
        }
        Some(tid)
    }

    /// Dequeue the lowest-priority thread (used for work stealing).
    fn dequeue_lowest(&mut self) -> Option<u32> {
        let p = self.lowest_priority()?;
        let tid = self.levels[p].pop_front()?;
        if self.levels[p].is_empty() {
            self.bits[p / 64] &= !(1u64 << (p % 64));
        }
        Some(tid)
    }

    /// Remove a specific TID from all priority levels.
    fn remove(&mut self, tid: u32) {
        for p in 0..NUM_PRIORITIES {
            if let Some(pos) = self.levels[p].iter().position(|&t| t == tid) {
                self.levels[p].remove(pos);
                if self.levels[p].is_empty() {
                    self.bits[p / 64] &= !(1u64 << (p % 64));
                }
                return;
            }
        }
    }

    /// Total number of queued threads across all priority levels.
    fn total_count(&self) -> usize {
        self.levels.iter().map(|q| q.len()).sum()
    }

    fn is_empty(&self) -> bool {
        self.bits[0] == 0 && self.bits[1] == 0
    }

    /// Highest priority level that has queued threads.
    fn highest_priority(&self) -> Option<usize> {
        if self.bits[1] != 0 {
            Some(127 - self.bits[1].leading_zeros() as usize)
        } else if self.bits[0] != 0 {
            Some(63 - self.bits[0].leading_zeros() as usize)
        } else {
            None
        }
    }

    /// Lowest priority level that has queued threads.
    fn lowest_priority(&self) -> Option<usize> {
        if self.bits[0] != 0 {
            Some(self.bits[0].trailing_zeros() as usize)
        } else if self.bits[1] != 0 {
            Some(64 + self.bits[1].trailing_zeros() as usize)
        } else {
            None
        }
    }
}

// =============================================================================
// Scheduler core
// =============================================================================

/// Per-CPU scheduling state.
struct PerCpuState {
    /// TID of the thread currently executing on this CPU, or None if idle.
    current_tid: Option<u32>,
    /// Multi-level priority queue of ready threads assigned to this CPU.
    run_queue: RunQueue,
}

/// Mach-style scheduler with per-CPU multi-level priority queues.
pub struct Scheduler {
    /// All threads known to the scheduler (any state).
    /// Heap-allocated via Box for pointer stability across Vec resizes.
    threads: Vec<Box<Thread>>,
    /// Per-CPU state: current thread + priority queue.
    per_cpu: Vec<PerCpuState>,
    /// Per-CPU idle thread TIDs. Always valid — never reaped.
    idle_tid: [u32; MAX_CPUS],
}

/// Idle thread entry point. Halts until the next interrupt.
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
                run_queue: RunQueue::new(),
            });
        }

        let mut sched = Scheduler {
            threads: Vec::with_capacity(128),
            per_cpu,
            idle_tid: [0; MAX_CPUS],
        };

        // Create per-CPU idle threads (priority 0 = lowest).
        for cpu in 0..MAX_CPUS {
            let name: &str = match cpu {
                0 => "idle/0",   1 => "idle/1",   2 => "idle/2",   3 => "idle/3",
                4 => "idle/4",   5 => "idle/5",   6 => "idle/6",   7 => "idle/7",
                8 => "idle/8",   9 => "idle/9",  10 => "idle/10", 11 => "idle/11",
               12 => "idle/12", 13 => "idle/13", 14 => "idle/14", 15 => "idle/15",
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

    /// Number of online CPUs (at least 1).
    #[inline]
    fn num_cpus(&self) -> usize {
        let n = crate::arch::x86::smp::cpu_count() as usize;
        if n == 0 { 1 } else { n }
    }

    /// Pick the CPU with the shortest ready queue for load balancing.
    fn least_loaded_cpu(&self) -> usize {
        let n = self.num_cpus();
        let mut best_cpu = 0;
        let mut best_len = usize::MAX;
        let mut tie_count = 0u32;
        let rr = ROUND_ROBIN_COUNTER.fetch_add(1, Ordering::Relaxed);
        for cpu in 0..n {
            let len = self.per_cpu[cpu].run_queue.total_count();
            if len < best_len {
                best_len = len;
                best_cpu = cpu;
                tie_count = 1;
            } else if len == best_len {
                tie_count += 1;
                if rr % tie_count == 0 { best_cpu = cpu; }
            }
        }
        best_cpu
    }

    /// Add a thread to the scheduler and enqueue on the least-loaded CPU.
    fn add_thread(&mut self, mut thread: Thread) -> u32 {
        let tid = thread.tid;
        let cpu = self.least_loaded_cpu();
        let pri = thread.priority;
        thread.last_cpu = cpu;
        self.threads.push(Box::new(thread));
        self.per_cpu[cpu].run_queue.enqueue(tid, pri);
        tid
    }

    /// Add a thread in Blocked state without putting it in any ready queue.
    fn add_thread_blocked(&mut self, mut thread: Thread) -> u32 {
        let tid = thread.tid;
        thread.state = ThreadState::Blocked;
        self.threads.push(Box::new(thread));
        tid
    }

    /// Remove a TID from ALL per-CPU ready queues.
    fn remove_from_all_queues(&mut self, tid: u32) {
        for cpu in 0..MAX_CPUS {
            self.per_cpu[cpu].run_queue.remove(tid);
        }
    }

    /// Reap terminated threads whose exit code has been consumed or auto-reaped.
    fn reap_terminated(&mut self) {
        let current_tick = crate::arch::x86::pit::get_ticks();
        let mut i = 0;
        while i < self.threads.len() {
            if self.threads[i].is_idle {
                i += 1;
                continue;
            }
            if self.threads[i].state == ThreadState::Terminated {
                // Don't reap while context_switch.asm is still saving registers
                if self.threads[i].context.save_complete == 0 {
                    i += 1;
                    continue;
                }
                // Grace period: 50ms for any in-flight context_switch
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
                    self.remove_from_all_queues(tid);
                    for cpu in 0..MAX_CPUS {
                        if self.per_cpu[cpu].current_tid == Some(tid) {
                            self.per_cpu[cpu].current_tid = Some(self.idle_tid[cpu]);
                        }
                    }
                    self.threads.swap_remove(i);
                    // Don't increment — check swapped-in element
                } else {
                    i += 1;
                }
            } else {
                i += 1;
            }
        }
    }

    /// Pick the next thread for this CPU: local queue first, then work stealing.
    /// Returns None if no eligible thread is available.
    fn pick_next(&mut self, cpu_id: usize) -> Option<u32> {
        // 1. Try local queue
        if let Some(tid) = self.pick_eligible(cpu_id) {
            return Some(tid);
        }
        // 2. Work stealing: find the busiest CPU and steal from it
        let n = self.num_cpus();
        let mut max_count = 0;
        let mut victim = cpu_id;
        for c in 0..n {
            if c != cpu_id {
                let count = self.per_cpu[c].run_queue.total_count();
                if count > max_count {
                    max_count = count;
                    victim = c;
                }
            }
        }
        if max_count > 0 {
            self.pick_eligible(victim)
        } else {
            None
        }
    }

    /// Dequeue the highest-priority eligible thread from a CPU's queue.
    /// Skips threads with save_complete==0 (context still being saved by ASM).
    fn pick_eligible(&mut self, queue_cpu: usize) -> Option<u32> {
        let mut attempts = 0;
        loop {
            attempts += 1;
            if attempts > 8 { return None; }
            let tid = match self.per_cpu[queue_cpu].run_queue.dequeue_highest() {
                Some(t) => t,
                None => return None,
            };
            if let Some(idx) = self.find_idx(tid) {
                if self.threads[idx].state == ThreadState::Ready
                    && self.threads[idx].context.save_complete != 0
                {
                    return Some(tid);
                }
                // Not eligible yet — re-enqueue and give up for this tick
                let pri = self.threads[idx].priority;
                self.per_cpu[queue_cpu].run_queue.enqueue(tid, pri);
                return None;
            }
            // Thread reaped — discard stale TID and try next
        }
    }

    /// Wake a blocked thread, enqueuing on the least-loaded CPU.
    fn wake_thread_inner(&mut self, tid: u32) {
        if let Some(idx) = self.find_idx(tid) {
            if self.threads[idx].state == ThreadState::Blocked {
                self.threads[idx].state = ThreadState::Ready;
                let target_cpu = self.least_loaded_cpu();
                self.per_cpu[target_cpu].run_queue.enqueue(tid, self.threads[idx].priority);
            }
        }
    }
}

// =============================================================================
// Public API — Init
// =============================================================================

/// Initialize the global scheduler.
pub fn init() {
    let mut sched = SCHEDULER.lock();
    *sched = Some(Scheduler::new());
    if let Some(s) = sched.as_mut() {
        let idle_tid = s.idle_tid[0];
        s.per_cpu[0].current_tid = Some(idle_tid);
        if let Some(idx) = s.find_idx(idle_tid) {
            s.threads[idx].state = ThreadState::Running;
        }
    }
    crate::serial_println!(
        "[OK] Mach scheduler initialized ({} priority levels, {} CPUs max, lazy FPU)",
        NUM_PRIORITIES, MAX_CPUS,
    );
}

// =============================================================================
// Public API — Tick counters
// =============================================================================

pub fn total_sched_ticks() -> u32 { TOTAL_SCHED_TICKS.load(Ordering::Relaxed) }
pub fn idle_sched_ticks() -> u32 { IDLE_SCHED_TICKS.load(Ordering::Relaxed) }

pub fn per_cpu_total_ticks(cpu: usize) -> u32 {
    if cpu < MAX_CPUS { PER_CPU_TOTAL[cpu].load(Ordering::Relaxed) } else { 0 }
}

pub fn per_cpu_idle_ticks(cpu: usize) -> u32 {
    if cpu < MAX_CPUS { PER_CPU_IDLE[cpu].load(Ordering::Relaxed) } else { 0 }
}

// =============================================================================
// Public API — Spawn
// =============================================================================

/// Create a new kernel thread and add it to the ready queue.
pub fn spawn(entry: extern "C" fn(), priority: u8, name: &str) -> u32 {
    let priority = clamp_priority(priority, name);
    let tid = {
        let thread = Thread::new(entry, priority, name);
        let mut sched = SCHEDULER.lock();
        let sched = sched.as_mut().expect("Scheduler not initialized");
        sched.add_thread(thread)
    };
    // Debug output OUTSIDE the lock — serial I/O takes ~3ms at 115200 baud
    // and holding the lock that long starves CPU 0's reap_terminated.
    #[cfg(feature = "debug_verbose")]
    crate::serial_println!("  Spawned thread '{}' (TID={})", name, tid);
    emit_spawn_event(tid, name);
    tid
}

/// Spawn a kernel thread in Blocked state (not added to any ready queue).
/// The thread will NOT run until [`wake_thread`] is called.
pub fn spawn_blocked(entry: extern "C" fn(), priority: u8, name: &str) -> u32 {
    let priority = clamp_priority(priority, name);
    let tid = {
        let thread = Thread::new(entry, priority, name);
        let mut sched = SCHEDULER.lock();
        let sched = sched.as_mut().expect("Scheduler not initialized");
        sched.add_thread_blocked(thread)
    };
    // Debug output OUTSIDE the lock (serial I/O is slow).
    #[cfg(feature = "debug_verbose")]
    crate::serial_println!("  Spawned thread '{}' (TID={}, blocked)", name, tid);
    emit_spawn_event(tid, name);
    tid
}

/// Helper: emit EVT_PROCESS_SPAWNED with the thread name packed into u32 words.
fn emit_spawn_event(tid: u32, name: &str) {
    let name_bytes = name.as_bytes();
    let mut p2: u32 = 0;
    let mut p3: u32 = 0;
    let mut p4: u32 = 0;
    for i in 0..name_bytes.len().min(12) {
        let word = match i / 4 { 0 => &mut p2, 1 => &mut p3, _ => &mut p4 };
        *word |= (name_bytes[i] as u32) << ((i % 4) * 8);
    }
    crate::ipc::event_bus::system_emit(crate::ipc::event_bus::EventData::new(
        crate::ipc::event_bus::EVT_PROCESS_SPAWNED, tid, p2, p3, p4,
    ));
}

/// Create a new thread within the same address space as the currently running thread.
pub fn create_thread_in_current_process(entry_rip: u64, user_rsp: u64, name: &str, priority: u8) -> u32 {
    let (pd, arch_mode, brk, parent_pri, parent_cwd, parent_caps, parent_uid, parent_gid) = {
        let guard = SCHEDULER.lock();
        let cpu_id = get_cpu_id();
        let sched = match guard.as_ref() { Some(s) => s, None => return 0 };
        let current_tid = match sched.per_cpu[cpu_id].current_tid { Some(t) => t, None => return 0 };
        let idx = match sched.find_idx(current_tid) { Some(i) => i, None => return 0 };
        let thread = &sched.threads[idx];
        let pd = match thread.page_directory { Some(pd) => pd, None => return 0 };
        (pd, thread.arch_mode, thread.brk, thread.priority, thread.cwd, thread.capabilities, thread.uid, thread.gid)
    };

    let effective_pri = if priority == 0 { parent_pri } else { priority };
    let effective_pri = clamp_priority(effective_pri, name);
    let tid = spawn_blocked(crate::task::loader::thread_create_trampoline, effective_pri, name);

    {
        let mut guard = SCHEDULER.lock();
        let sched = guard.as_mut().expect("Scheduler not initialized");
        if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
            thread.page_directory = Some(pd);
            thread.context.cr3 = pd.as_u64();
            thread.context.checksum = thread.context.compute_checksum();
            thread.is_user = true;
            thread.brk = brk;
            thread.arch_mode = arch_mode;
            thread.pd_shared = true;
            thread.cwd = parent_cwd;
            thread.capabilities = parent_caps;
            thread.uid = parent_uid;
            thread.gid = parent_gid;
        }
    }

    crate::task::loader::store_pending_thread(tid, entry_rip, user_rsp);
    wake_thread(tid);
    tid
}

// =============================================================================
// Scheduling
// =============================================================================

/// Called from the timer interrupt for preemptive scheduling.
/// Returns false (and does nothing) if this CPU is already inside schedule_inner,
/// preventing re-entrant scheduling that causes context corruption and deadlocks.
pub fn schedule_tick() -> bool {
    let cpu_id = crate::arch::x86::smp::current_cpu_id() as usize;
    if cpu_id < MAX_CPUS && PER_CPU_IN_SCHEDULER[cpu_id].load(Ordering::Relaxed) {
        // Already in scheduler — just count the tick for timekeeping accuracy
        TOTAL_SCHED_TICKS.fetch_add(1, Ordering::Relaxed);
        PER_CPU_TOTAL[cpu_id].fetch_add(1, Ordering::Relaxed);
        if !PER_CPU_HAS_THREAD[cpu_id].load(Ordering::Relaxed) {
            IDLE_SCHED_TICKS.fetch_add(1, Ordering::Relaxed);
            PER_CPU_IDLE[cpu_id].fetch_add(1, Ordering::Relaxed);
        } else {
            PER_CPU_CONTENDED_BUSY[cpu_id].fetch_add(1, Ordering::Relaxed);
        }
        return false;
    }
    schedule_inner(true);
    true
}

/// Voluntary yield: reschedule without incrementing CPU accounting counters.
pub fn schedule() { schedule_inner(false); }

fn schedule_inner(from_timer: bool) {
    // Read CPU ID and set in-scheduler flag atomically w.r.t. timer interrupts.
    // For from_timer=true, IF is already 0 (inside IRQ handler) — no race.
    // For from_timer=false (voluntary), we MUST briefly disable interrupts to
    // prevent a timer from firing between get_cpu_id() and the flag store.
    // Without this, the timer can preempt, migrate the thread to CPU B, and
    // the thread resumes setting PER_CPU_IN_SCHEDULER[A] (stale) — permanently
    // blocking scheduling on CPU A.
    let saved_flags: u64;
    if !from_timer {
        unsafe { core::arch::asm!("pushfq; pop {0}; cli", out(reg) saved_flags, options(nomem, nostack)); }
    } else {
        saved_flags = 0; // unused — IF already 0 in timer path
    }

    let cpu_id_early = get_cpu_id();

    // Mark this CPU as inside the scheduler to prevent timer nesting.
    if cpu_id_early < MAX_CPUS {
        PER_CPU_IN_SCHEDULER[cpu_id_early].store(true, Ordering::Relaxed);
    }

    // Restore interrupts for voluntary path (needed for spin loop below)
    if !from_timer {
        unsafe { core::arch::asm!("push {0}; popfq", in(reg) saved_flags, options(nomem, nostack)); }
    }

    // CPU 0 processes deferred PD destruction (before acquiring scheduler lock)
    if from_timer && cpu_id_early == 0 {
        let pds = DEFERRED_PD_DESTROY.lock().drain();
        for pd in pds.iter().flatten() {
            crate::memory::virtual_mem::destroy_user_page_directory(*pd);
        }
    }

    // Tick counters (timer path only)
    if from_timer {
        TOTAL_SCHED_TICKS.fetch_add(1, Ordering::Relaxed);
        PER_CPU_TOTAL[cpu_id_early].fetch_add(1, Ordering::Relaxed);
    }

    // Lock acquisition: try_lock for timer (non-blocking), spin for voluntary
    let mut guard = if from_timer {
        match SCHEDULER.try_lock() {
            Some(s) => s,
            None => {
                if !PER_CPU_HAS_THREAD[cpu_id_early].load(Ordering::Relaxed) {
                    IDLE_SCHED_TICKS.fetch_add(1, Ordering::Relaxed);
                    PER_CPU_IDLE[cpu_id_early].fetch_add(1, Ordering::Relaxed);
                } else {
                    PER_CPU_CONTENDED_BUSY[cpu_id_early].fetch_add(1, Ordering::Relaxed);
                }
                PER_CPU_IN_SCHEDULER[cpu_id_early].store(false, Ordering::Relaxed);
                return;
            }
        }
    } else {
        loop {
            match SCHEDULER.try_lock() {
                Some(s) => break s,
                None => core::hint::spin_loop(),
            }
        }
    };

    // Re-read CPU ID under lock (interrupts disabled — can't migrate)
    let cpu_id = get_cpu_id();

    // Extract context switch parameters under the lock
    let mut switch_info: Option<(*mut CpuContext, *const CpuContext, *mut u8, *const u8, u32, u32)>;
    let mut corrupt_diag: Option<(&'static str, u32, *const CpuContext)> = None;

    {
        let sched = match guard.as_mut() {
            Some(s) => s,
            None => {
                PER_CPU_IN_SCHEDULER[cpu_id as usize].store(false, Ordering::Relaxed);
                return;
            }
        };

        // Reap terminated threads (any CPU can do this, not just CPU 0).
        // Previously restricted to CPU 0, but under high lock contention
        // (e.g., stress test with serial debug output) CPU 0 may not get
        // the lock often enough, causing terminated threads to accumulate.
        if from_timer {
            sched.reap_terminated();
        }

        // CPU 0: periodic canary check on all non-Running threads.
        // Rate-limited to every 100 ticks (~100ms) to avoid holding the lock
        // too long with serial output. Only reports first corrupt thread found
        // to keep the critical section short.
        if from_timer && cpu_id == 0 {
            static CANARY_CHECK_CTR: AtomicU32 = AtomicU32::new(0);
            let ctr = CANARY_CHECK_CTR.fetch_add(1, Ordering::Relaxed);
            if ctr % 100 == 0 {
                use crate::task::context::CANARY_MAGIC;
                for i in 0..sched.threads.len() {
                    let t = &sched.threads[i];
                    if t.context.save_complete == 1
                        && t.state != ThreadState::Running
                        && !t.is_idle
                    {
                        if t.context.canary != CANARY_MAGIC {
                            crate::serial_println!(
                                "!CANARY DEAD: TID={} '{}' canary={:#018x} ctx={:#x}",
                                t.tid, t.name_str(),
                                t.context.canary, &t.context as *const _ as u64,
                            );
                            break; // Only report first — keep critical section short
                        } else if t.context.checksum != t.context.compute_checksum() {
                            crate::serial_println!(
                                "!CHECKSUM FAIL: TID={} '{}' chk={:#018x} expect={:#018x}",
                                t.tid, t.name_str(),
                                t.context.checksum, t.context.compute_checksum(),
                            );
                            break;
                        }
                    }
                }
            }
        }

        // Wake expired sleepers
        if from_timer {
            let current_tick = crate::arch::x86::pit::get_ticks();
            for i in 0..sched.threads.len() {
                if sched.threads[i].state == ThreadState::Blocked {
                    if let Some(wake_tick) = sched.threads[i].wake_at_tick {
                        if current_tick.wrapping_sub(wake_tick) < 0x8000_0000 {
                            let tid = sched.threads[i].tid;
                            let pri = sched.threads[i].priority;
                            let target_cpu = sched.least_loaded_cpu();
                            sched.threads[i].state = ThreadState::Ready;
                            sched.threads[i].wake_at_tick = None;
                            sched.per_cpu[target_cpu].run_queue.enqueue(tid, pri);
                        }
                    }
                }
            }
        }

        // Drain contended-busy ticks
        let missed = PER_CPU_CONTENDED_BUSY[cpu_id].swap(0, Ordering::Relaxed);
        if missed > 0 {
            if let Some(current_tid) = sched.per_cpu[cpu_id].current_tid {
                if current_tid != sched.idle_tid[cpu_id] {
                    if let Some(idx) = sched.find_idx(current_tid) {
                        sched.threads[idx].cpu_ticks += missed;
                    }
                }
            }
        }

        // CPU tick accounting
        if from_timer {
            if let Some(current_tid) = sched.per_cpu[cpu_id].current_tid {
                if current_tid == sched.idle_tid[cpu_id] {
                    IDLE_SCHED_TICKS.fetch_add(1, Ordering::Relaxed);
                    PER_CPU_IDLE[cpu_id].fetch_add(1, Ordering::Relaxed);
                } else if let Some(idx) = sched.find_idx(current_tid) {
                    if sched.threads[idx].state == ThreadState::Running {
                        sched.threads[idx].cpu_ticks += 1;
                    }
                }
            }
        }

        // --- Put current thread back into its priority queue ---
        let outgoing_tid = sched.per_cpu[cpu_id].current_tid;
        if let Some(current_tid) = outgoing_tid {
            if current_tid != sched.idle_tid[cpu_id] {
                if let Some(idx) = sched.find_idx(current_tid) {
                    // ALWAYS mark context as unsaved for non-idle outgoing threads.
                    // Defense-in-depth: even if waitpid/sleep_until already set this
                    // to 0, this catches any path that missed it.
                    sched.threads[idx].context.save_complete = 0;
                    if sched.threads[idx].state == ThreadState::Running {
                        sched.threads[idx].state = ThreadState::Ready;
                        sched.threads[idx].last_cpu = cpu_id;
                        let pri = sched.threads[idx].priority;
                        sched.per_cpu[cpu_id].run_queue.enqueue(current_tid, pri);
                    }
                }
            }
        }

        // --- Pick next thread (O(1) via bitmap) ---
        switch_info = if let Some(next_tid) = sched.pick_next(cpu_id) {
            if let Some(next_idx) = sched.find_idx(next_tid) {
                let kstack_top = sched.threads[next_idx].kernel_stack_top();
                let kstack_bottom = sched.threads[next_idx].kernel_stack_bottom();

                // Validate candidate before committing
                let kstack_valid = kstack_top >= 0xFFFF_FFFF_8000_0000;
                if !kstack_valid {
                    crate::serial_println!(
                        "BUG: thread '{}' (TID={}) invalid kstack_top={:#x} — killing",
                        sched.threads[next_idx].name_str(), next_tid, kstack_top,
                    );
                    sched.threads[next_idx].state = ThreadState::Terminated;
                    sched.threads[next_idx].exit_code = Some(139);
                    sched.threads[next_idx].terminated_at_tick =
                        Some(crate::arch::x86::pit::get_ticks());
                    // Restore outgoing as current
                    sched.per_cpu[cpu_id].current_tid = outgoing_tid;
                    if let Some(ot) = outgoing_tid {
                        if let Some(oi) = sched.find_idx(ot) {
                            sched.threads[oi].context.save_complete = 1;
                        }
                    }
                    None
                } else {
                    // Commit: update per-CPU state
                    sched.per_cpu[cpu_id].current_tid = Some(next_tid);
                    sched.threads[next_idx].state = ThreadState::Running;
                    sched.threads[next_idx].last_cpu = cpu_id;

                    PER_CPU_HAS_THREAD[cpu_id].store(true, Ordering::Relaxed);
                    PER_CPU_CURRENT_TID[cpu_id].store(next_tid, Ordering::Relaxed);
                    PER_CPU_IS_USER[cpu_id].store(sched.threads[next_idx].is_user, Ordering::Relaxed);
                    update_per_cpu_name(cpu_id, &sched.threads[next_idx].name);

                    // Update TSS.RSP0 and SYSCALL kernel RSP
                    crate::arch::x86::tss::set_kernel_stack_for_cpu(cpu_id, kstack_top);
                    crate::arch::x86::syscall_msr::set_kernel_rsp(cpu_id, kstack_top);
                    PER_CPU_STACK_BOTTOM[cpu_id].store(kstack_bottom, Ordering::Relaxed);
                    PER_CPU_STACK_TOP[cpu_id].store(kstack_top, Ordering::Relaxed);

                    // Store FPU pointer for lazy #NM handler
                    PER_CPU_FPU_PTR[cpu_id].store(
                        sched.threads[next_idx].fpu_state.data.as_ptr() as u64,
                        Ordering::Relaxed,
                    );

                    if outgoing_tid == Some(next_tid) {
                        // Same thread — no switch needed
                        sched.threads[next_idx].context.save_complete = 1;
                        None
                    } else if let Some(prev_tid) = outgoing_tid {
                        if let Some(prev_idx) = sched.find_idx(prev_tid) {
                            let old_ctx = &mut sched.threads[prev_idx].context as *mut CpuContext;
                            let new_ctx = &sched.threads[next_idx].context as *const CpuContext;
                            let old_fpu = sched.threads[prev_idx].fpu_state.data.as_mut_ptr();
                            let new_fpu = sched.threads[next_idx].fpu_state.data.as_ptr();
                            Some((old_ctx, new_ctx, old_fpu, new_fpu, prev_tid, next_tid))
                        } else {
                            // Previous thread reaped — switch from idle
                            let idle_i = sched.find_idx(sched.idle_tid[cpu_id]).unwrap();
                            let old_ctx = &mut sched.threads[idle_i].context as *mut CpuContext;
                            let new_ctx = &sched.threads[next_idx].context as *const CpuContext;
                            let old_fpu = sched.threads[idle_i].fpu_state.data.as_mut_ptr();
                            let new_fpu = sched.threads[next_idx].fpu_state.data.as_ptr();
                            Some((old_ctx, new_ctx, old_fpu, new_fpu, sched.idle_tid[cpu_id], next_tid))
                        }
                    } else {
                        // No previous thread — switch from idle
                        let idle_i = sched.find_idx(sched.idle_tid[cpu_id]).unwrap();
                        let old_ctx = &mut sched.threads[idle_i].context as *mut CpuContext;
                        let new_ctx = &sched.threads[next_idx].context as *const CpuContext;
                        let old_fpu = sched.threads[idle_i].fpu_state.data.as_mut_ptr();
                        let new_fpu = sched.threads[next_idx].fpu_state.data.as_ptr();
                        Some((old_ctx, new_ctx, old_fpu, new_fpu, sched.idle_tid[cpu_id], next_tid))
                    }
                }
            } else {
                // TID reaped between pick_next and here
                sched.per_cpu[cpu_id].current_tid = outgoing_tid;
                if let Some(ot) = outgoing_tid {
                    if let Some(oi) = sched.find_idx(ot) {
                        sched.threads[oi].context.save_complete = 1;
                    }
                }
                None
            }
        } else {
            // No ready threads — this CPU is idle.
            // NOTE: Do NOT increment IDLE_SCHED_TICKS here — the per-tick
            // accounting at lines 812-824 above already counted this tick
            // as idle (if current==idle) or busy (if current was Running).
            // Double-counting idle ticks causes idle > total, making the
            // Activity Monitor show 0% CPU load permanently.

            // If current thread is no longer runnable, switch to idle
            if let Some(current_tid) = sched.per_cpu[cpu_id].current_tid {
                if let Some(idx) = sched.find_idx(current_tid) {
                    if sched.threads[idx].state != ThreadState::Running {
                        sched.threads[idx].context.save_complete = 0;
                        let idle_tid = sched.idle_tid[cpu_id];
                        let idle_i = sched.find_idx(idle_tid).unwrap();
                        sched.per_cpu[cpu_id].current_tid = Some(idle_tid);
                        sched.threads[idle_i].state = ThreadState::Running;
                        PER_CPU_HAS_THREAD[cpu_id].store(false, Ordering::Relaxed);
                        PER_CPU_IS_USER[cpu_id].store(false, Ordering::Relaxed);
                        PER_CPU_CURRENT_TID[cpu_id].store(idle_tid, Ordering::Relaxed);
                        update_per_cpu_name(cpu_id, &sched.threads[idle_i].name);
                        let idle_kstack_top = sched.threads[idle_i].kernel_stack_top();
                        crate::arch::x86::tss::set_kernel_stack_for_cpu(cpu_id, idle_kstack_top);
                        crate::arch::x86::syscall_msr::set_kernel_rsp(cpu_id, idle_kstack_top);
                        PER_CPU_STACK_BOTTOM[cpu_id].store(sched.threads[idle_i].kernel_stack_bottom(), Ordering::Relaxed);
                        PER_CPU_STACK_TOP[cpu_id].store(idle_kstack_top, Ordering::Relaxed);
                        PER_CPU_FPU_PTR[cpu_id].store(
                            sched.threads[idle_i].fpu_state.data.as_ptr() as u64,
                            Ordering::Relaxed,
                        );
                        let old_ctx = &mut sched.threads[idx].context as *mut CpuContext;
                        let idle_ctx = &sched.threads[idle_i].context as *const CpuContext;
                        let old_fpu = sched.threads[idx].fpu_state.data.as_mut_ptr();
                        let new_fpu = sched.threads[idle_i].fpu_state.data.as_ptr();
                        Some((old_ctx, idle_ctx, old_fpu, new_fpu, current_tid, idle_tid))
                    } else {
                        if !sched.threads[idx].is_idle {
                            sched.threads[idx].context.save_complete = 1;
                        }
                        None
                    }
                } else {
                    // Current thread reaped — set to idle
                    let idle_tid = sched.idle_tid[cpu_id];
                    let idle_i = sched.find_idx(idle_tid).unwrap();
                    sched.per_cpu[cpu_id].current_tid = Some(idle_tid);
                    PER_CPU_HAS_THREAD[cpu_id].store(false, Ordering::Relaxed);
                    PER_CPU_IS_USER[cpu_id].store(false, Ordering::Relaxed);
                    PER_CPU_CURRENT_TID[cpu_id].store(idle_tid, Ordering::Relaxed);
                    update_per_cpu_name(cpu_id, &sched.threads[idle_i].name);
                    let idle_kstack_top = sched.threads[idle_i].kernel_stack_top();
                    crate::arch::x86::tss::set_kernel_stack_for_cpu(cpu_id, idle_kstack_top);
                    crate::arch::x86::syscall_msr::set_kernel_rsp(cpu_id, idle_kstack_top);
                    PER_CPU_STACK_BOTTOM[cpu_id].store(sched.threads[idle_i].kernel_stack_bottom(), Ordering::Relaxed);
                    PER_CPU_STACK_TOP[cpu_id].store(idle_kstack_top, Ordering::Relaxed);
                    None
                }
            } else {
                let idle_tid = sched.idle_tid[cpu_id];
                let idle_i = sched.find_idx(idle_tid).unwrap();
                sched.per_cpu[cpu_id].current_tid = Some(idle_tid);
                PER_CPU_HAS_THREAD[cpu_id].store(false, Ordering::Relaxed);
                let idle_kstack_top = sched.threads[idle_i].kernel_stack_top();
                crate::arch::x86::tss::set_kernel_stack_for_cpu(cpu_id, idle_kstack_top);
                crate::arch::x86::syscall_msr::set_kernel_rsp(cpu_id, idle_kstack_top);
                None
            }
        };

        // ---------------------------------------------------------------
        // Validate incoming context WHILE STILL HOLDING THE LOCK.
        // If corrupt, undo all scheduling state changes and kill the
        // corrupt thread. This prevents the broken recovery scenario where
        // the outgoing thread ends up in the run queue with save_complete=1
        // but its context was never re-saved — leading to two CPUs on the
        // same stack.
        // ---------------------------------------------------------------
        if let Some((_, new_ctx, _, _, out_tid, next_tid)) = switch_info {
            use crate::task::context::CANARY_MAGIC;
            let is_corrupt = unsafe {
                let ctx = &*new_ctx;
                ctx.canary != CANARY_MAGIC
                    || ctx.checksum != ctx.compute_checksum()
                    || ctx.rip < 0xFFFF_FFFF_8010_0000
                    || ctx.rip >= 0xFFFF_FFFF_8200_0000
                    || ctx.rsp < 0xFFFF_FFFF_8010_0000
            };
            if is_corrupt {
                let reason = unsafe {
                    let ctx = &*new_ctx;
                    if ctx.canary != CANARY_MAGIC { "CANARY_DEAD" }
                    else if ctx.checksum != ctx.compute_checksum() { "CHECKSUM_FAIL" }
                    else { "RANGE_BAD" }
                };
                corrupt_diag = Some((reason, next_tid, new_ctx));

                // Kill the corrupt incoming thread
                if let Some(next_idx) = sched.find_idx(next_tid) {
                    sched.threads[next_idx].state = ThreadState::Terminated;
                    sched.threads[next_idx].exit_code = Some(139);
                    sched.threads[next_idx].terminated_at_tick =
                        Some(crate::arch::x86::pit::get_ticks());
                }

                // Determine which thread to restore as current on this CPU.
                // The outgoing thread's state depends on how it became outgoing:
                //   - Was Running → set to Ready + enqueued + save_complete=0
                //   - Was Blocked → save_complete=0, NOT enqueued
                //   - Was idle → nothing was changed
                let restore_tid = if let Some(out_idx) = sched.find_idx(out_tid) {
                    if sched.threads[out_idx].state == ThreadState::Ready {
                        // Was Running, promoted to Ready for enqueue — restore to Running.
                        // It's still in the run queue, but safe: pick_eligible checks
                        // state==Ready, so it will be dequeued and re-enqueued without
                        // being selected.
                        sched.threads[out_idx].state = ThreadState::Running;
                    }
                    sched.threads[out_idx].context.save_complete = 1;
                    if sched.threads[out_idx].state == ThreadState::Running {
                        out_tid
                    } else {
                        // Blocked or other non-running state — go to idle
                        sched.idle_tid[cpu_id]
                    }
                } else {
                    // Outgoing was reaped — go to idle
                    sched.idle_tid[cpu_id]
                };

                // Restore per-CPU state for the chosen thread
                sched.per_cpu[cpu_id].current_tid = Some(restore_tid);
                if let Some(ri) = sched.find_idx(restore_tid) {
                    if restore_tid != out_tid {
                        sched.threads[ri].state = ThreadState::Running;
                    }
                    PER_CPU_HAS_THREAD[cpu_id].store(
                        !sched.threads[ri].is_idle, Ordering::Relaxed);
                    PER_CPU_CURRENT_TID[cpu_id].store(restore_tid, Ordering::Relaxed);
                    PER_CPU_IS_USER[cpu_id].store(
                        sched.threads[ri].is_user, Ordering::Relaxed);
                    update_per_cpu_name(cpu_id, &sched.threads[ri].name);
                    let kstack_top = sched.threads[ri].kernel_stack_top();
                    crate::arch::x86::tss::set_kernel_stack_for_cpu(cpu_id, kstack_top);
                    crate::arch::x86::syscall_msr::set_kernel_rsp(cpu_id, kstack_top);
                    PER_CPU_STACK_BOTTOM[cpu_id].store(
                        sched.threads[ri].kernel_stack_bottom(), Ordering::Relaxed);
                    PER_CPU_STACK_TOP[cpu_id].store(kstack_top, Ordering::Relaxed);
                }

                switch_info = None;
            }
        }

    } // sched borrow ends here

    // Release lock WITHOUT restoring IF — keeps interrupts disabled through context_switch
    guard.release_no_irq_restore();

    // Verbose diagnostics AFTER lock released (serial I/O is slow, ~3ms/char at 115200)
    if let Some((reason, next_tid, ctx_ptr)) = corrupt_diag {
        unsafe {
            let ctx = &*ctx_ptr;
            crate::serial_println!(
                "!{}: TID={} ctx={:#x} canary={:#018x} chk={:#018x} expect={:#018x}",
                reason, next_tid, ctx_ptr as u64,
                ctx.canary, ctx.checksum, ctx.compute_checksum(),
            );
            let p = ctx_ptr as *const u64;
            let names = [
                "rax", "rbx", "rcx", "rdx", "rsi", "rdi", "rbp",
                "r8 ", "r9 ", "r10", "r11", "r12", "r13", "r14", "r15",
                "rsp", "rip", "rfl", "cr3", "sav", "can", "chk",
            ];
            for i in 0..22 {
                crate::serial_println!("  [{}] {} = {:#018x}", i * 8, names[i], *p.add(i));
            }
        }
    }

    // Context switch with lock released, interrupts still disabled
    if let Some((old_ctx, new_ctx, old_fpu, _new_fpu, outgoing_tid, _next_tid)) = switch_info {
        // --- Lazy FPU: save outgoing thread's state if this CPU owns it ---
        let fpu_owner = PER_CPU_FPU_OWNER[cpu_id].load(Ordering::Relaxed);
        if fpu_owner != 0 && fpu_owner == outgoing_tid {
            unsafe {
                core::arch::asm!("fxsave [{}]", in(reg) old_fpu, options(nostack, preserves_flags));
            }
            PER_CPU_FPU_OWNER[cpu_id].store(0, Ordering::Relaxed);
        }

        // Set CR0.TS — next FPU/SSE instruction triggers #NM for lazy restore
        unsafe {
            let cr0: u64;
            core::arch::asm!("mov {}, cr0", out(reg) cr0, options(nostack, nomem, preserves_flags));
            core::arch::asm!("mov cr0, {}", in(reg) cr0 | 8, options(nostack, nomem, preserves_flags));
        }

        // Clear in-scheduler flag BEFORE context_switch. Interrupts are disabled
        // (release_no_irq_restore kept IF=0), so no timer can fire between the
        // clear and the switch. This is CRITICAL because if the new thread is
        // starting for the first time (RIP = entry point, not inside schedule_inner),
        // it will never reach the post-switch cleanup code below.
        PER_CPU_IN_SCHEDULER[cpu_id].store(false, Ordering::Relaxed);

        unsafe { crate::task::context::context_switch(old_ctx, new_ctx); }
    }

    // Also clear after context_switch for the no-switch path (switch_info was None).
    // After context_switch we may be on a different CPU, so re-read cpu_id.
    let cpu_id_exit = get_cpu_id();
    if cpu_id_exit < MAX_CPUS {
        PER_CPU_IN_SCHEDULER[cpu_id_exit].store(false, Ordering::Relaxed);
    }

    // Re-enable interrupts (CRITICAL for voluntary schedule — without this, IF stays 0)
    unsafe { core::arch::asm!("sti"); }
}

// =============================================================================
// Lazy FPU — #NM handler
// =============================================================================

/// Handle Device Not Available exception (#NM, ISR 7).
/// Called when a thread executes an FPU/SSE instruction with CR0.TS set.
/// Loads the thread's FPU state and clears TS so the instruction can retry.
pub fn handle_device_not_available() {
    let cpu_id = get_cpu_id();
    let current_tid = PER_CPU_CURRENT_TID[cpu_id].load(Ordering::Relaxed);
    let fpu_owner = PER_CPU_FPU_OWNER[cpu_id].load(Ordering::Relaxed);

    // If this thread's state is already loaded, just clear TS
    if fpu_owner == current_tid && current_tid != 0 {
        unsafe { core::arch::asm!("clts", options(nostack, preserves_flags)); }
        return;
    }

    // Clear TS first — FXRSTOR also traps on CR0.TS=1
    unsafe { core::arch::asm!("clts", options(nostack, preserves_flags)); }

    // Load this thread's FPU/SSE state
    let fpu_ptr = PER_CPU_FPU_PTR[cpu_id].load(Ordering::Relaxed);
    if fpu_ptr != 0 {
        unsafe {
            core::arch::asm!("fxrstor [{}]", in(reg) fpu_ptr, options(nostack, preserves_flags));
        }
        PER_CPU_FPU_OWNER[cpu_id].store(current_tid, Ordering::Relaxed);
    }
}

// =============================================================================
// Current thread accessors
// =============================================================================

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
        if let Some(tid) = sched.per_cpu[cpu_id].current_tid {
            if let Some(idx) = sched.find_idx(tid) {
                return sched.threads[idx].is_user;
            }
        }
    }
    false
}

/// Get the current thread's name.
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

/// Lock-free read of the current TID on this CPU.
pub fn debug_current_tid() -> u32 {
    let cpu_id = crate::arch::x86::smp::current_cpu_id() as usize;
    if cpu_id < MAX_CPUS { PER_CPU_CURRENT_TID[cpu_id].load(Ordering::Relaxed) } else { 0 }
}

/// Lock-free check: is the current thread a user process?
pub fn debug_is_current_user() -> bool {
    let cpu_id = crate::arch::x86::smp::current_cpu_id() as usize;
    PER_CPU_IS_USER[cpu_id].load(Ordering::Relaxed)
}

/// Lock-free read of the cached thread name for the current CPU.
pub fn debug_current_thread_name() -> [u8; 32] {
    let cpu_id = crate::arch::x86::smp::current_cpu_id() as usize;
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
    let cpu_id = crate::arch::x86::smp::current_cpu_id() as usize;
    let tid = match sched.per_cpu[cpu_id].current_tid { Some(t) => t, None => return };
    let idx = match sched.find_idx(tid) { Some(i) => i, None => return };
    if !sched.threads[idx].check_stack_canary() {
        crate::serial_println!(
            "STACK OVERFLOW after syscall {} in '{}' (TID={}) — killing",
            syscall_num, sched.threads[idx].name_str(), tid,
        );
        sched.threads[idx].state = ThreadState::Terminated;
        sched.threads[idx].exit_code = Some(139);
        sched.threads[idx].terminated_at_tick = Some(crate::arch::x86::pit::get_ticks());
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

// =============================================================================
// Thread configuration
// =============================================================================

/// Configure a thread as a user process.
pub fn set_thread_user_info(tid: u32, pd: PhysAddr, brk: u32) {
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.page_directory = Some(pd);
        thread.context.cr3 = pd.as_u64();
        thread.context.checksum = thread.context.compute_checksum();
        thread.is_user = true;
        thread.brk = brk;
        // Reserve fd 0/1/2 as Tty so pipe()/open() start at fd 3
        thread.fd_table.alloc_at(0, FdKind::Tty);
        thread.fd_table.alloc_at(1, FdKind::Tty);
        thread.fd_table.alloc_at(2, FdKind::Tty);
    }
}

/// Set the architecture mode for a thread.
pub fn set_thread_arch_mode(tid: u32, mode: crate::task::thread::ArchMode) {
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.arch_mode = mode;
    }
}

/// Get the current thread's page directory.
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

/// Get the page directory for a thread by TID.
pub fn thread_page_directory(tid: u32) -> Option<PhysAddr> {
    let guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.find_idx(tid) {
            return sched.threads[idx].page_directory;
        }
    }
    None
}

/// Check if the current thread has a shared page directory.
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

/// Check if any OTHER live thread shares the same page directory.
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

/// Atomically get all info needed for sys_exit.
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
                } else { false };
                let can_destroy = pd.is_some() && !pd_shared && !has_siblings;
                return (tid, pd, can_destroy);
            }
        }
    }
    (0, None, false)
}

/// Get the current thread's program break.
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

/// Set the current thread's program break, syncing across sibling threads.
pub fn set_current_thread_brk(brk: u32) {
    let mut guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_mut() {
        if let Some(tid) = sched.per_cpu[cpu_id].current_tid {
            if let Some(idx) = sched.find_idx(tid) {
                sched.threads[idx].brk = brk;
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

/// Return the current thread's mmap bump pointer.
pub fn current_thread_mmap_next() -> u32 {
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        if let Some(tid) = sched.per_cpu[cpu_id].current_tid {
            if let Some(idx) = sched.find_idx(tid) {
                return sched.threads[idx].mmap_next;
            }
        }
    }
    0
}

/// Set the current thread's mmap bump pointer, syncing across sibling threads.
pub fn set_current_thread_mmap_next(val: u32) {
    let mut guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_mut() {
        if let Some(tid) = sched.per_cpu[cpu_id].current_tid {
            if let Some(idx) = sched.find_idx(tid) {
                sched.threads[idx].mmap_next = val;
                if let Some(pd) = sched.threads[idx].page_directory {
                    let current_tid = sched.threads[idx].tid;
                    for thread in sched.threads.iter_mut() {
                        if thread.tid != current_tid && thread.page_directory == Some(pd) {
                            thread.mmap_next = val;
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
    let mut pd_to_destroy: Option<PhysAddr> = None;
    let mut parent_tid_for_sigchld: u32 = 0;
    let mut guard = SCHEDULER.lock();
    {
        let cpu_id = get_cpu_id();
        let sched = guard.as_mut().expect("Scheduler not initialized");
        tid = sched.per_cpu[cpu_id].current_tid.unwrap_or(0);
        if let Some(current_tid) = sched.per_cpu[cpu_id].current_tid {
            if let Some(idx) = sched.find_idx(current_tid) {
                // Capture parent_tid for SIGCHLD before marking terminated
                parent_tid_for_sigchld = sched.threads[idx].parent_tid;

                sched.threads[idx].state = ThreadState::Terminated;
                sched.threads[idx].exit_code = Some(code);
                sched.threads[idx].terminated_at_tick = Some(crate::arch::x86::pit::get_ticks());
                // Save PD for destruction AFTER lock release (not just None — that leaks frames)
                if let Some(pd) = sched.threads[idx].page_directory {
                    if !sched.threads[idx].pd_shared {
                        let has_live_siblings = sched.threads.iter().any(|t| {
                            t.tid != current_tid && t.page_directory == Some(pd)
                                && t.state != ThreadState::Terminated
                        });
                        if !has_live_siblings {
                            pd_to_destroy = Some(pd);
                        }
                    }
                }
                sched.threads[idx].page_directory = None;
                if let Some(waiter_tid) = sched.threads[idx].waiting_tid {
                    sched.wake_thread_inner(waiter_tid);
                }

                // Send SIGCHLD to parent (while still under the lock)
                if parent_tid_for_sigchld != 0 {
                    if let Some(parent_idx) = sched.find_idx(parent_tid_for_sigchld) {
                        sched.threads[parent_idx].signals.send(crate::ipc::signal::SIGCHLD);
                    }
                }
            }
        }
    }
    // Release lock but keep IF=0 — no timer can fire between here and schedule().
    // This closes the race where a timer could context-switch away a Terminated
    // thread before PD destruction and system_emit run.
    guard.release_no_irq_restore();
    // Destroy page directory (switch to kernel CR3 first)
    if let Some(pd) = pd_to_destroy {
        let kernel_cr3 = crate::memory::virtual_mem::kernel_cr3();
        unsafe { core::arch::asm!("mov cr3, {}", in(reg) kernel_cr3); }
        crate::memory::virtual_mem::destroy_user_page_directory(pd);
    }
    crate::ipc::event_bus::system_emit(crate::ipc::event_bus::EventData::new(
        crate::ipc::event_bus::EVT_PROCESS_EXITED, tid, code, 0, 0,
    ));
    // schedule_inner will sti after context_switch
    schedule();
    loop { unsafe { core::arch::asm!("hlt"); } }
}

/// Try to terminate the current thread (non-blocking lock acquisition).
pub fn try_exit_current(code: u32) -> bool {
    let tid;
    let mut pd_to_destroy: Option<PhysAddr> = None;
    let mut guard = match SCHEDULER.try_lock() {
        Some(g) => g,
        None => return false,
    };
    {
        let cpu_id = get_cpu_id();
        let sched = match guard.as_mut() { Some(s) => s, None => return false };
        if let Some(current_tid) = sched.per_cpu[cpu_id].current_tid {
            tid = current_tid;
            if let Some(idx) = sched.find_idx(current_tid) {
                // Send SIGCHLD to parent
                let parent_tid = sched.threads[idx].parent_tid;
                if parent_tid != 0 {
                    if let Some(parent_idx) = sched.find_idx(parent_tid) {
                        sched.threads[parent_idx].signals.send(crate::ipc::signal::SIGCHLD);
                    }
                }

                sched.threads[idx].state = ThreadState::Terminated;
                sched.threads[idx].exit_code = Some(code);
                sched.threads[idx].terminated_at_tick = Some(crate::arch::x86::pit::get_ticks());
                // Save PD for destruction (same as exit_current)
                if let Some(pd) = sched.threads[idx].page_directory {
                    if !sched.threads[idx].pd_shared {
                        let has_live_siblings = sched.threads.iter().any(|t| {
                            t.tid != current_tid && t.page_directory == Some(pd)
                                && t.state != ThreadState::Terminated
                        });
                        if !has_live_siblings {
                            pd_to_destroy = Some(pd);
                        }
                    }
                }
                sched.threads[idx].page_directory = None;
                if let Some(waiter_tid) = sched.threads[idx].waiting_tid {
                    sched.wake_thread_inner(waiter_tid);
                }
            } else { return false; }
        } else { return false; }
    }
    // Release lock but keep IF=0 (same pattern as exit_current)
    guard.release_no_irq_restore();
    if let Some(pd) = pd_to_destroy {
        let kernel_cr3 = crate::memory::virtual_mem::kernel_cr3();
        unsafe { core::arch::asm!("mov cr3, {}", in(reg) kernel_cr3); }
        crate::memory::virtual_mem::destroy_user_page_directory(pd);
    }
    crate::ipc::event_bus::system_emit(crate::ipc::event_bus::EventData::new(
        crate::ipc::event_bus::EVT_PROCESS_EXITED, tid, code, 0, 0,
    ));
    schedule();
    loop { unsafe { core::arch::asm!("hlt"); } }
}

/// Saved by interrupts.asm before the recovery SWAPGS overwrites RSP.
#[no_mangle]
pub static mut BAD_RSP_SAVED: u64 = 0;

/// Recovery function called from interrupts.asm when an ISR fires with corrupt RSP.
/// Kills the faulting thread, repairs TSS.RSP0, and enters the idle loop.
/// This function never returns.
#[no_mangle]
pub extern "C" fn bad_rsp_recovery() -> ! {
    let cpu_id = crate::arch::x86::smp::current_cpu_id() as usize;
    let tid = PER_CPU_CURRENT_TID[cpu_id].load(Ordering::Relaxed);
    crate::serial_println!("!RSP RECOVERY on CPU {} — killing TID={}, entering idle", cpu_id, tid);

    let bad_rsp = unsafe { BAD_RSP_SAVED };
    let tss_rsp0 = crate::arch::x86::tss::get_kernel_stack_for_cpu(cpu_id);
    crate::serial_println!(
        "  bad_rsp={:#018x} TSS.RSP0={:#018x}", bad_rsp, tss_rsp0,
    );

    crate::arch::x86::apic::eoi();

    let mut idle_stack_top: u64 = 0;
    {
        if let Some(mut guard) = SCHEDULER.try_lock() {
            if let Some(ref mut sched) = *guard {
                if let Some(current_tid) = sched.per_cpu[cpu_id].current_tid {
                    if let Some(idx) = sched.find_idx(current_tid) {
                        if sched.threads[idx].critical {
                            crate::serial_println!(
                                "  CRITICAL thread '{}' (TID={}) spared",
                                sched.threads[idx].name_str(), current_tid,
                            );
                            sched.threads[idx].state = ThreadState::Ready;
                            sched.threads[idx].context.save_complete = 1;
                            let pri = sched.threads[idx].priority;
                            sched.per_cpu[cpu_id].run_queue.enqueue(current_tid, pri);
                        } else if !sched.threads[idx].is_idle {
                            sched.threads[idx].state = ThreadState::Terminated;
                            sched.threads[idx].exit_code = Some(139);
                            sched.threads[idx].terminated_at_tick = Some(crate::arch::x86::pit::get_ticks());
                            if let Some(waiter_tid) = sched.threads[idx].waiting_tid {
                                sched.wake_thread_inner(waiter_tid);
                            }
                        }
                    }
                    sched.per_cpu[cpu_id].current_tid = None;
                }
                let idle_tid = sched.idle_tid[cpu_id];
                if let Some(idx) = sched.find_idx(idle_tid) {
                    let kstack_top = sched.threads[idx].kernel_stack_top();
                    crate::arch::x86::tss::set_kernel_stack_for_cpu(cpu_id, kstack_top);
                    crate::arch::x86::syscall_msr::set_kernel_rsp(cpu_id, kstack_top);
                    idle_stack_top = kstack_top;
                }
            }
        } else {
            let idle_st = PER_CPU_IDLE_STACK_TOP[cpu_id].load(Ordering::Relaxed);
            if idle_st >= 0xFFFF_FFFF_8000_0000 {
                crate::arch::x86::tss::set_kernel_stack_for_cpu(cpu_id, idle_st);
                crate::arch::x86::syscall_msr::set_kernel_rsp(cpu_id, idle_st);
                idle_stack_top = idle_st;
            }
        }
    }

    PER_CPU_HAS_THREAD[cpu_id].store(false, Ordering::Relaxed);
    PER_CPU_IS_USER[cpu_id].store(false, Ordering::Relaxed);
    PER_CPU_CURRENT_TID[cpu_id].store(0, Ordering::Relaxed);
    clear_per_cpu_name(cpu_id);

    unsafe {
        let kcr3 = crate::memory::virtual_mem::kernel_cr3();
        core::arch::asm!("mov cr3, {}", in(reg) kcr3, options(nostack));
    }

    if idle_stack_top >= 0xFFFF_FFFF_8000_0000 {
        unsafe {
            core::arch::asm!(
                "mov rsp, {0}", "sti", "2: hlt", "jmp 2b",
                in(reg) idle_stack_top, options(noreturn)
            );
        }
    } else {
        unsafe { core::arch::asm!("sti"); }
        loop { unsafe { core::arch::asm!("hlt"); } }
    }
}

/// Fallback recovery when try_exit_current fails. Kills thread and enters idle.
pub fn fault_kill_and_idle(signal: u32) -> ! {
    let cpu_id = crate::arch::x86::smp::current_cpu_id() as usize;
    let tid = PER_CPU_CURRENT_TID[cpu_id].load(Ordering::Relaxed);
    crate::serial_println!("  FALLBACK: manual kill TID={} signal={} on CPU {}", tid, signal, cpu_id);

    let cpu = cpu_id as u32;
    if is_scheduler_locked_by_cpu(cpu) {
        unsafe { force_unlock_scheduler(); }
    }

    let mut idle_stack_top: u64 = 0;
    {
        if let Some(mut guard) = SCHEDULER.try_lock() {
            if let Some(ref mut sched) = *guard {
                if let Some(idx) = sched.find_idx(tid) {
                    sched.threads[idx].state = ThreadState::Terminated;
                    sched.threads[idx].exit_code = Some(signal);
                    sched.threads[idx].terminated_at_tick = Some(crate::arch::x86::pit::get_ticks());
                    if let Some(waiter_tid) = sched.threads[idx].waiting_tid {
                        sched.wake_thread_inner(waiter_tid);
                    }
                }
                sched.per_cpu[cpu_id].current_tid = None;
                let idle_tid = sched.idle_tid[cpu_id];
                if let Some(idx) = sched.find_idx(idle_tid) {
                    let kstack_top = sched.threads[idx].kernel_stack_top();
                    crate::arch::x86::tss::set_kernel_stack_for_cpu(cpu_id, kstack_top);
                    crate::arch::x86::syscall_msr::set_kernel_rsp(cpu_id, kstack_top);
                    idle_stack_top = kstack_top;
                    PER_CPU_STACK_BOTTOM[cpu_id].store(sched.threads[idx].kernel_stack_bottom(), Ordering::Relaxed);
                    PER_CPU_STACK_TOP[cpu_id].store(kstack_top, Ordering::Relaxed);
                }
            }
        } else {
            let idle_st = PER_CPU_IDLE_STACK_TOP[cpu_id].load(Ordering::Relaxed);
            if idle_st >= 0xFFFF_FFFF_8000_0000 {
                crate::arch::x86::tss::set_kernel_stack_for_cpu(cpu_id, idle_st);
                crate::arch::x86::syscall_msr::set_kernel_rsp(cpu_id, idle_st);
                idle_stack_top = idle_st;
            }
        }
    }

    PER_CPU_HAS_THREAD[cpu_id].store(false, Ordering::Relaxed);
    PER_CPU_IS_USER[cpu_id].store(false, Ordering::Relaxed);
    PER_CPU_CURRENT_TID[cpu_id].store(0, Ordering::Relaxed);
    clear_per_cpu_name(cpu_id);

    if tid != 0 {
        crate::ipc::event_bus::system_emit(crate::ipc::event_bus::EventData::new(
            crate::ipc::event_bus::EVT_PROCESS_EXITED, tid, signal, 0, 0,
        ));
    }

    unsafe {
        let kcr3 = crate::memory::virtual_mem::kernel_cr3();
        core::arch::asm!("mov cr3, {}", in(reg) kcr3, options(nostack));
    }

    if idle_stack_top >= 0xFFFF_FFFF_8000_0000 {
        unsafe {
            core::arch::asm!(
                "mov rsp, {0}", "sti", "2: hlt", "jmp 2b",
                in(reg) idle_stack_top, options(noreturn)
            );
        }
    } else {
        unsafe { core::arch::asm!("sti"); }
        loop { unsafe { core::arch::asm!("hlt"); } }
    }
}

/// Kill a thread by TID. Returns 0 on success, u32::MAX on error.
pub fn kill_thread(tid: u32) -> u32 {
    if tid == 0 { return u32::MAX; }

    let mut pd_to_destroy: Option<PhysAddr> = None;
    let is_current;
    let running_on_other_cpu;

    let mut guard = SCHEDULER.lock();
    {
        let cpu_id = get_cpu_id();
        let sched = guard.as_mut().expect("Scheduler not initialized");

        let target_idx = match sched.find_idx(tid) {
            Some(idx) => idx,
            None => return u32::MAX, // guard drops normally (restores IF)
        };
        if sched.threads[target_idx].is_idle { return u32::MAX; }

        is_current = sched.per_cpu[cpu_id].current_tid == Some(tid);
        running_on_other_cpu = !is_current && sched.per_cpu.iter().enumerate().any(|(i, cpu)| {
            i != cpu_id && cpu.current_tid == Some(tid)
        });

        sched.threads[target_idx].state = ThreadState::Terminated;
        sched.threads[target_idx].exit_code = Some(u32::MAX - 1);
        sched.threads[target_idx].terminated_at_tick = Some(crate::arch::x86::pit::get_ticks());
        sched.remove_from_all_queues(tid);

        if let Some(pd) = sched.threads[target_idx].page_directory {
            if sched.threads[target_idx].pd_shared {
                sched.threads[target_idx].page_directory = None;
            } else {
                let has_live_siblings = sched.threads.iter().any(|t| {
                    t.tid != tid && t.page_directory == Some(pd) && t.state != ThreadState::Terminated
                });
                if has_live_siblings {
                    sched.threads[target_idx].page_directory = None;
                } else {
                    pd_to_destroy = Some(pd);
                    sched.threads[target_idx].page_directory = None;
                }
            }
        }

        if let Some(waiter_tid) = sched.threads[target_idx].waiting_tid {
            sched.wake_thread_inner(waiter_tid);
        }
        // NOTE: Do NOT set current_tid = idle_tid here! We're still on the
        // killed thread's kernel stack. schedule_inner will handle the
        // Terminated→idle transition properly.
    }

    // For is_current: release lock but keep IF=0 to prevent timer from
    // context-switching before schedule(). For !is_current: normal drop.
    if is_current {
        guard.release_no_irq_restore();
    } else {
        drop(guard);
    }

    if let Some(pd) = pd_to_destroy {
        if running_on_other_cpu {
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
        // IF still 0 — schedule_inner will sti after context_switch.
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
            if let Some(idx) = sched.find_idx(current_tid) {
                // CRITICAL: Mark context as unsaved BEFORE setting Blocked.
                // Without this, another CPU can wake this thread (→ Ready)
                // and load its stale saved context while we're still
                // physically executing on its stack → two CPUs on same stack → crash.
                sched.threads[idx].context.save_complete = 0;
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
        if let Some(idx) = sched.find_idx(current_tid) {
            sched.threads[idx].context.save_complete = 0;
            sched.threads[idx].state = ThreadState::Blocked;
        }
    }
    loop {
        unsafe { core::arch::asm!("sti; hlt"); }
        {
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
        let mut guard = SCHEDULER.lock();
        let cpu_id = get_cpu_id();
        let sched = guard.as_mut().expect("Scheduler not initialized");
        if let Some(current_tid) = sched.per_cpu[cpu_id].current_tid {
            if let Some(idx) = sched.find_idx(current_tid) {
                // CRITICAL: Mark context as unsaved before Blocked (same race as waitpid).
                sched.threads[idx].context.save_complete = 0;
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
                // CRITICAL: Mark context as unsaved before Blocked (same race as waitpid).
                sched.threads[idx].context.save_complete = 0;
                sched.threads[idx].state = ThreadState::Blocked;
            }
        }
    }
    schedule();
}

// =============================================================================
// Thread args / stdout / stdin
// =============================================================================

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

/// Set the current working directory for a thread.
pub fn set_thread_cwd(tid: u32, cwd: &str) {
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        let bytes = cwd.as_bytes();
        let len = bytes.len().min(255);
        thread.cwd[..len].copy_from_slice(&bytes[..len]);
        thread.cwd[len] = 0;
    }
}

/// Get the current working directory for the running thread.
pub fn current_thread_cwd(buf: &mut [u8]) -> usize {
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        if let Some(tid) = sched.per_cpu[cpu_id].current_tid {
            if let Some(idx) = sched.find_idx(tid) {
                let cwd = &sched.threads[idx].cwd;
                let len = cwd.iter().position(|&b| b == 0).unwrap_or(256);
                let copy_len = len.min(buf.len());
                buf[..copy_len].copy_from_slice(&cwd[..copy_len]);
                return copy_len;
            }
        }
    }
    0
}

pub fn set_thread_stdout_pipe(tid: u32, pipe_id: u32) {
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.stdout_pipe = pipe_id;
    }
}

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

pub fn set_thread_stdin_pipe(tid: u32, pipe_id: u32) {
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.stdin_pipe = pipe_id;
    }
}

pub fn current_thread_stdin_pipe() -> u32 {
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        if let Some(tid) = sched.per_cpu[cpu_id].current_tid {
            if let Some(idx) = sched.find_idx(tid) {
                return sched.threads[idx].stdin_pipe;
            }
        }
    }
    0
}

// =============================================================================
// Priority / wake / critical
// =============================================================================

/// Set the priority of a thread by TID (clamped to 0–127).
pub fn set_thread_priority(tid: u32, priority: u8) {
    let priority = clamp_priority(priority, "set_thread_priority");
    let mut guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_mut() {
        if let Some(idx) = sched.find_idx(tid) {
            sched.threads[idx].priority = priority;
        }
    }
}

/// Wake a blocked thread by TID.
pub fn wake_thread(tid: u32) {
    let mut guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_mut() {
        sched.wake_thread_inner(tid);
    }
}

/// Mark a thread as critical (will not be killed by RSP recovery).
pub fn set_thread_critical(tid: u32) {
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.critical = true;
        crate::serial_println!("  Thread '{}' (TID={}) marked as critical", thread.name_str(), tid);
    }
}

/// Get the capability bitmask for the currently running thread.
pub fn current_thread_capabilities() -> crate::task::capabilities::CapSet {
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    let sched = guard.as_ref().expect("Scheduler not initialized");
    if let Some(tid) = sched.per_cpu[cpu_id].current_tid {
        if let Some(thread) = sched.threads.iter().find(|t| t.tid == tid) {
            return thread.capabilities;
        }
    }
    0
}

/// Set the capability bitmask for a thread (called by loader after spawn).
pub fn set_thread_capabilities(tid: u32, caps: crate::task::capabilities::CapSet) {
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.capabilities = caps;
    }
}

/// Get the user ID of the currently running thread.
pub fn current_thread_uid() -> u16 {
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    let sched = guard.as_ref().expect("Scheduler not initialized");
    if let Some(tid) = sched.per_cpu[cpu_id].current_tid {
        if let Some(thread) = sched.threads.iter().find(|t| t.tid == tid) {
            return thread.uid;
        }
    }
    0
}

/// Get the group ID of the currently running thread.
pub fn current_thread_gid() -> u16 {
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    let sched = guard.as_ref().expect("Scheduler not initialized");
    if let Some(tid) = sched.per_cpu[cpu_id].current_tid {
        if let Some(thread) = sched.threads.iter().find(|t| t.tid == tid) {
            return thread.gid;
        }
    }
    0
}

/// Store pending permission info on the current thread.
/// Data is a UTF-8 byte slice: "app_id\x1Fapp_name\x1Fcaps_hex\x1Fbundle_path".
pub fn set_current_perm_pending(data: &[u8]) {
    let mut guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_mut() {
        if let Some(tid) = sched.per_cpu[cpu_id].current_tid {
            if let Some(idx) = sched.find_idx(tid) {
                let len = data.len().min(512);
                sched.threads[idx].perm_pending[..len].copy_from_slice(&data[..len]);
                sched.threads[idx].perm_pending_len = len as u16;
            }
        }
    }
}

/// Read pending permission info from the current thread into `buf`.
/// Returns the number of bytes copied (0 if none).
pub fn current_perm_pending(buf: &mut [u8]) -> usize {
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        if let Some(tid) = sched.per_cpu[cpu_id].current_tid {
            if let Some(idx) = sched.find_idx(tid) {
                let len = sched.threads[idx].perm_pending_len as usize;
                if len > 0 {
                    let copy = len.min(buf.len());
                    buf[..copy].copy_from_slice(&sched.threads[idx].perm_pending[..copy]);
                    return copy;
                }
            }
        }
    }
    0
}

/// Set the user and group IDs for a specific thread.
pub fn set_thread_identity(tid: u32, uid: u16, gid: u16) {
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.uid = uid;
        thread.gid = gid;
    }
}

/// Set uid/gid on ALL threads that share the same page_directory as the given thread.
/// Used by SYS_AUTHENTICATE to propagate identity to all threads in a process.
pub fn set_process_identity(tid: u32, uid: u16, gid: u16) {
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    // Find the page directory of the target thread
    let pd = {
        let thread = match sched.threads.iter().find(|t| t.tid == tid) {
            Some(t) => t,
            None => return,
        };
        thread.page_directory
    };
    // Update all threads sharing the same PD (same process)
    for thread in sched.threads.iter_mut() {
        if thread.page_directory == pd {
            thread.uid = uid;
            thread.gid = gid;
        }
    }
}

// =========================================================================
// fork() helpers — snapshot parent state, copy fields to child
// =========================================================================

/// Snapshot of a thread's state needed for fork().
/// All fields captured under a single scheduler lock to prevent TOCTOU.
pub struct ForkSnapshot {
    pub pd: PhysAddr,
    pub brk: u32,
    pub arch_mode: crate::task::thread::ArchMode,
    pub args: [u8; 256],
    pub cwd: [u8; 256],
    pub capabilities: crate::task::capabilities::CapSet,
    pub uid: u16,
    pub gid: u16,
    pub stdout_pipe: u32,
    pub stdin_pipe: u32,
    pub fpu_data: [u8; 512],
    pub mmap_next: u32,
    pub user_pages: u32,
    pub priority: u8,
    pub name: [u8; 32],
    pub fd_table: crate::fs::fd_table::FdTable,
    pub signals: crate::ipc::signal::SignalState,
    pub parent_tid: u32,
}

/// Capture all fork-relevant fields from the current thread in a single lock.
pub fn current_thread_fork_snapshot() -> Option<ForkSnapshot> {
    let guard = SCHEDULER.lock();
    let sched = guard.as_ref()?;
    let cpu = get_cpu_id();
    let tid = sched.per_cpu[cpu].current_tid?;
    let thread = sched.threads.iter().find(|t| t.tid == tid)?;
    let pd = thread.page_directory?;
    Some(ForkSnapshot {
        pd,
        brk: thread.brk,
        arch_mode: thread.arch_mode,
        args: thread.args,
        cwd: thread.cwd,
        capabilities: thread.capabilities,
        uid: thread.uid,
        gid: thread.gid,
        stdout_pipe: thread.stdout_pipe,
        stdin_pipe: thread.stdin_pipe,
        fpu_data: thread.fpu_state.data,
        mmap_next: thread.mmap_next,
        user_pages: thread.user_pages,
        priority: thread.priority,
        name: thread.name,
        fd_table: thread.fd_table.clone(),
        signals: thread.signals.clone(),
        parent_tid: thread.tid,
    })
}

/// Set the FPU state on a thread (for fork child).
pub fn set_thread_fpu_state(tid: u32, data: &[u8; 512]) {
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.fpu_state.data = *data;
    }
}

/// Set mmap_next on a thread (for fork child).
pub fn set_thread_mmap_next(tid: u32, val: u32) {
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.mmap_next = val;
    }
}

/// Set user_pages count on a thread (for fork child).
pub fn set_thread_user_pages(tid: u32, val: u32) {
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.user_pages = val;
    }
}

/// Update thread state for exec(): new PD, reset brk/mmap/fpu, change arch mode.
pub fn exec_update_thread(
    tid: u32,
    new_pd: PhysAddr,
    brk: u32,
    arch_mode: crate::task::thread::ArchMode,
    user_pages: u32,
) {
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.page_directory = Some(new_pd);
        thread.context.cr3 = new_pd.as_u64();
        thread.brk = brk;
        thread.mmap_next = 0x2000_0000;
        thread.fpu_state = crate::task::thread::FxState::new_default();
        thread.user_pages = user_pages;
        thread.arch_mode = arch_mode;
        thread.context.checksum = thread.context.compute_checksum();
    }
}

/// Get the DR1 watch address (kept for compat — returns 0, no watchpoint in Mach scheduler).
pub fn get_dr1_watch_addr() -> u64 { 0 }

// =============================================================================
// Thread info / diagnostics
// =============================================================================

/// Snapshot of a thread's state for the `ps` / sysinfo syscall.
pub struct ThreadInfo {
    pub tid: u32,
    pub priority: u8,
    pub state: &'static str,
    pub name: alloc::string::String,
    pub cpu_ticks: u32,
    pub arch_mode: u8,
    pub io_read_bytes: u64,
    pub io_write_bytes: u64,
    pub user_pages: u32,
    pub uid: u16,
}

/// List all live threads (lock-free heap allocation pattern).
pub fn list_threads() -> Vec<ThreadInfo> {
    const MAX_SNAP: usize = 64;
    struct ThreadSnap {
        tid: u32, priority: u8, state: u8, arch_mode: u8,
        cpu_ticks: u32, io_read_bytes: u64, io_write_bytes: u64,
        user_pages: u32, name: [u8; 32], name_len: u8, uid: u16,
    }
    let mut buf = [const {
        ThreadSnap { tid: 0, priority: 0, state: 0, arch_mode: 0, cpu_ticks: 0,
            io_read_bytes: 0, io_write_bytes: 0, user_pages: 0, name: [0; 32], name_len: 0, uid: 0 }
    }; MAX_SNAP];
    let mut count = 0;

    {
        let guard = SCHEDULER.lock();
        if let Some(sched) = guard.as_ref() {
            let online_cpus = crate::arch::x86::smp::cpu_count() as usize;
            for thread in &sched.threads {
                if thread.state == ThreadState::Terminated { continue; }
                if thread.is_idle && !sched.idle_tid[..online_cpus].contains(&thread.tid) { continue; }
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
                    tid: thread.tid, priority: thread.priority, state: state_num,
                    arch_mode: thread.arch_mode as u8, cpu_ticks: thread.cpu_ticks,
                    io_read_bytes: thread.io_read_bytes, io_write_bytes: thread.io_write_bytes,
                    user_pages: thread.user_pages, name: name_buf, name_len: len as u8,
                    uid: thread.uid,
                };
                count += 1;
            }
        }
    }

    let mut result = Vec::with_capacity(count);
    for i in 0..count {
        let snap = &buf[i];
        result.push(ThreadInfo {
            tid: snap.tid,
            priority: snap.priority,
            state: match snap.state { 0 => "ready", 1 => "running", _ => "blocked" },
            name: alloc::string::String::from(
                core::str::from_utf8(&snap.name[..snap.name_len as usize]).unwrap_or("?")
            ),
            cpu_ticks: snap.cpu_ticks,
            arch_mode: snap.arch_mode,
            io_read_bytes: snap.io_read_bytes,
            io_write_bytes: snap.io_write_bytes,
            user_pages: snap.user_pages,
            uid: snap.uid,
        });
    }
    result
}

// =============================================================================
// Lock management
// =============================================================================

pub fn is_scheduler_locked_by_cpu(cpu: u32) -> bool { SCHEDULER.is_held_by_cpu(cpu) }
pub fn is_scheduler_locked() -> bool { SCHEDULER.is_locked() }

/// # Safety
/// Must only be called when `is_scheduler_locked_by_cpu(cpu)` returns true.
pub unsafe fn force_unlock_scheduler() {
    SCHEDULER.force_unlock();
    crate::serial_println!("  RECOVERED: force-released scheduler lock");
}

// =============================================================================
// I/O accounting
// =============================================================================

pub fn record_io_read(bytes: u64) {
    let mut guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_mut() {
        if let Some(tid) = sched.per_cpu[cpu_id].current_tid {
            if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
                thread.io_read_bytes += bytes;
            }
        }
    }
}

pub fn record_io_write(bytes: u64) {
    let mut guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_mut() {
        if let Some(tid) = sched.per_cpu[cpu_id].current_tid {
            if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
                thread.io_write_bytes += bytes;
            }
        }
    }
}

pub fn adjust_thread_user_pages(tid: u32, delta: i32) {
    let mut guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_mut() {
        if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
            if delta >= 0 {
                thread.user_pages = thread.user_pages.saturating_add(delta as u32);
            } else {
                thread.user_pages = thread.user_pages.saturating_sub((-delta) as u32);
            }
        }
    }
}

pub fn adjust_current_user_pages(delta: i32) {
    let mut guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_mut() {
        if let Some(tid) = sched.per_cpu[cpu_id].current_tid {
            if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
                if delta >= 0 {
                    thread.user_pages = thread.user_pages.saturating_add(delta as u32);
                } else {
                    thread.user_pages = thread.user_pages.saturating_sub((-delta) as u32);
                }
            }
        }
    }
}

// =============================================================================
// Entry points
// =============================================================================

/// Enter the scheduler loop (called from kernel_main, becomes BSP idle thread).
pub fn run() -> ! {
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
                PER_CPU_HAS_THREAD[0].store(false, Ordering::Relaxed);
                PER_CPU_STACK_BOTTOM[0].store(kstack_bottom, Ordering::Relaxed);
                PER_CPU_STACK_TOP[0].store(kstack_top, Ordering::Relaxed);
                PER_CPU_IDLE_STACK_TOP[0].store(kstack_top, Ordering::Relaxed);
                update_per_cpu_name(0, &sched.threads[idx].name);
            }
        }
    }
    {
        let guard = SCHEDULER.lock();
        if let Some(sched) = guard.as_ref() {
            if let Some(t) = sched.threads.first() {
                t.print_layout_diagnostics();
            }
        }
    }
    unsafe { core::arch::asm!("sti"); }
    loop { unsafe { core::arch::asm!("hlt"); } }
}

/// Register the idle thread for an AP.
pub fn register_ap_idle(cpu_id: usize) {
    crate::debug_println!("  [Sched] register_ap_idle: cpu={} acquiring scheduler lock", cpu_id);
    let mut guard = SCHEDULER.lock();
    crate::debug_println!("  [Sched] register_ap_idle: cpu={} lock acquired", cpu_id);
    if let Some(sched) = guard.as_mut() {
        let idle_tid = sched.idle_tid[cpu_id];
        crate::debug_println!("  [Sched] register_ap_idle: cpu={} idle_tid={}", cpu_id, idle_tid);
        sched.per_cpu[cpu_id].current_tid = Some(idle_tid);
        if let Some(idx) = sched.find_idx(idle_tid) {
            sched.threads[idx].state = ThreadState::Running;
            let kstack_top = sched.threads[idx].kernel_stack_top();
            let kstack_bottom = sched.threads[idx].kernel_stack_bottom();
            crate::debug_println!("  [Sched] register_ap_idle: cpu={} kstack=[{:#x}..{:#x}]",
                cpu_id, kstack_bottom, kstack_top);
            crate::arch::x86::tss::set_kernel_stack_for_cpu(cpu_id, kstack_top);
            crate::arch::x86::syscall_msr::set_kernel_rsp(cpu_id, kstack_top);
            PER_CPU_CURRENT_TID[cpu_id].store(idle_tid, Ordering::Relaxed);
            PER_CPU_HAS_THREAD[cpu_id].store(false, Ordering::Relaxed);
            PER_CPU_STACK_BOTTOM[cpu_id].store(kstack_bottom, Ordering::Relaxed);
            PER_CPU_STACK_TOP[cpu_id].store(kstack_top, Ordering::Relaxed);
            PER_CPU_IDLE_STACK_TOP[cpu_id].store(kstack_top, Ordering::Relaxed);
            update_per_cpu_name(cpu_id, &sched.threads[idx].name);
        } else {
            crate::debug_println!("  [Sched] register_ap_idle: cpu={} ERROR: idle_tid={} not found!", cpu_id, idle_tid);
        }
    } else {
        crate::debug_println!("  [Sched] register_ap_idle: cpu={} ERROR: scheduler not initialized!", cpu_id);
    }
    crate::serial_println!("  SMP: CPU{} idle thread registered", cpu_id);
    crate::debug_println!("  [Sched] register_ap_idle: cpu={} done, releasing lock", cpu_id);
}

// =============================================================================
// Per-process FD table helpers
// =============================================================================

use crate::fs::fd_table::{FdEntry, FdKind, FdTable, MAX_FDS};

/// Allocate an FD in the current thread's FD table.
pub fn current_fd_alloc(kind: FdKind) -> Option<u32> {
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut()?;
    let cpu = get_cpu_id();
    let tid = sched.per_cpu[cpu].current_tid?;
    let thread = sched.threads.iter_mut().find(|t| t.tid == tid)?;
    thread.fd_table.alloc(kind)
}

/// Close an FD in the current thread's FD table.
/// Returns the old FdKind for cleanup (decref, etc.), or None if invalid.
pub fn current_fd_close(fd: u32) -> Option<FdKind> {
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut()?;
    let cpu = get_cpu_id();
    let tid = sched.per_cpu[cpu].current_tid?;
    let thread = sched.threads.iter_mut().find(|t| t.tid == tid)?;
    thread.fd_table.close(fd)
}

/// Look up an FD in the current thread's FD table.
pub fn current_fd_get(fd: u32) -> Option<FdEntry> {
    let guard = SCHEDULER.lock();
    let sched = guard.as_ref()?;
    let cpu = get_cpu_id();
    let tid = sched.per_cpu[cpu].current_tid?;
    let thread = sched.threads.iter().find(|t| t.tid == tid)?;
    thread.fd_table.get(fd).copied()
}

/// Duplicate old_fd to new_fd in the current thread's FD table.
/// Caller must handle closing new_fd first and incrementing refcounts.
pub fn current_fd_dup2(old_fd: u32, new_fd: u32) -> bool {
    let mut guard = SCHEDULER.lock();
    let sched = match guard.as_mut() { Some(s) => s, None => return false };
    let cpu = get_cpu_id();
    let tid = match sched.per_cpu[cpu].current_tid { Some(t) => t, None => return false };
    let thread = match sched.threads.iter_mut().find(|t| t.tid == tid) { Some(t) => t, None => return false };
    thread.fd_table.dup2(old_fd, new_fd)
}

/// Allocate the lowest FD >= min_fd in the current thread's FD table.
pub fn current_fd_alloc_above(min_fd: u32, kind: FdKind) -> Option<u32> {
    let mut guard = SCHEDULER.lock();
    let sched = match guard.as_mut() { Some(s) => s, None => return None };
    let cpu = get_cpu_id();
    let tid = match sched.per_cpu[cpu].current_tid { Some(t) => t, None => return None };
    let thread = match sched.threads.iter_mut().find(|t| t.tid == tid) { Some(t) => t, None => return None };
    thread.fd_table.alloc_above(min_fd, kind)
}

/// Allocate an FD at a specific slot in the current thread's FD table.
pub fn current_fd_alloc_at(fd: u32, kind: FdKind) -> bool {
    let mut guard = SCHEDULER.lock();
    let sched = match guard.as_mut() { Some(s) => s, None => return false };
    let cpu = get_cpu_id();
    let tid = match sched.per_cpu[cpu].current_tid { Some(t) => t, None => return false };
    let thread = match sched.threads.iter_mut().find(|t| t.tid == tid) { Some(t) => t, None => return false };
    thread.fd_table.alloc_at(fd, kind)
}

/// Set or clear the CLOEXEC flag on an FD in the current thread's FD table.
pub fn current_fd_set_cloexec(fd: u32, cloexec: bool) {
    let mut guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_mut() {
        let cpu = get_cpu_id();
        if let Some(tid) = sched.per_cpu[cpu].current_tid {
            if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
                thread.fd_table.set_cloexec(fd, cloexec);
            }
        }
    }
}

/// Set the FD table on a thread (for fork child setup).
pub fn set_thread_fd_table(tid: u32, table: FdTable) {
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.fd_table = table;
    }
}

/// Close all FDs in the current thread's FD table. Returns old FdKinds for cleanup.
pub fn current_fd_close_all() -> [FdKind; MAX_FDS] {
    let mut out = [FdKind::None; MAX_FDS];
    let mut guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_mut() {
        let cpu = get_cpu_id();
        if let Some(tid) = sched.per_cpu[cpu].current_tid {
            if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
                thread.fd_table.close_all(&mut out);
            }
        }
    }
    out
}

/// Close all CLOEXEC FDs in the current thread's FD table. Returns old FdKinds.
pub fn current_fd_close_cloexec() -> [FdKind; MAX_FDS] {
    let mut out = [FdKind::None; MAX_FDS];
    let mut guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_mut() {
        let cpu = get_cpu_id();
        if let Some(tid) = sched.per_cpu[cpu].current_tid {
            if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
                thread.fd_table.close_cloexec(&mut out);
            }
        }
    }
    out
}

/// Close all FDs for a specific thread (by TID). Returns old FdKinds for cleanup.
/// Used during sys_exit before destroying the page directory.
pub fn close_all_fds_for_thread(tid: u32) -> [FdKind; MAX_FDS] {
    let mut out = [FdKind::None; MAX_FDS];
    let mut guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_mut() {
        if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
            thread.fd_table.close_all(&mut out);
        }
    }
    out
}

// =========================================================================
// Signal helpers
// =========================================================================

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
        if let Some(tid) = sched.per_cpu[cpu].current_tid {
            if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
                return thread.signals.dequeue();
            }
        }
    }
    None
}

/// Get the handler address for a signal on the current thread.
pub fn current_signal_handler(sig: u32) -> u64 {
    let guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_ref() {
        let cpu = get_cpu_id();
        if let Some(tid) = sched.per_cpu[cpu].current_tid {
            if let Some(thread) = sched.threads.iter().find(|t| t.tid == tid) {
                return thread.signals.get_handler(sig);
            }
        }
    }
    crate::ipc::signal::SIG_DFL
}

/// Set a signal handler on the current thread. Returns the old handler.
pub fn current_signal_set_handler(sig: u32, handler: u64) -> u64 {
    let mut guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_mut() {
        let cpu = get_cpu_id();
        if let Some(tid) = sched.per_cpu[cpu].current_tid {
            if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
                return thread.signals.set_handler(sig, handler);
            }
        }
    }
    crate::ipc::signal::SIG_DFL
}

/// Get the current thread's blocked signal mask.
pub fn current_signal_get_blocked() -> u32 {
    let guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_ref() {
        let cpu = get_cpu_id();
        if let Some(tid) = sched.per_cpu[cpu].current_tid {
            if let Some(thread) = sched.threads.iter().find(|t| t.tid == tid) {
                return thread.signals.blocked;
            }
        }
    }
    0
}

/// Set the current thread's blocked signal mask. Returns the old mask.
pub fn current_signal_set_blocked(new_mask: u32) -> u32 {
    let mut guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_mut() {
        let cpu = get_cpu_id();
        if let Some(tid) = sched.per_cpu[cpu].current_tid {
            if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
                let old = thread.signals.blocked;
                thread.signals.blocked = new_mask;
                return old;
            }
        }
    }
    0
}

/// Check if the current thread has any pending, unblocked signals.
pub fn current_has_pending_signal() -> bool {
    let guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_ref() {
        let cpu = get_cpu_id();
        if let Some(tid) = sched.per_cpu[cpu].current_tid {
            if let Some(thread) = sched.threads.iter().find(|t| t.tid == tid) {
                return thread.signals.has_pending();
            }
        }
    }
    false
}

/// Set parent_tid on a thread (for fork/spawn child).
pub fn set_thread_parent_tid(tid: u32, parent: u32) {
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
        if let Some(tid) = sched.per_cpu[cpu].current_tid {
            if let Some(thread) = sched.threads.iter().find(|t| t.tid == tid) {
                return thread.parent_tid;
            }
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
