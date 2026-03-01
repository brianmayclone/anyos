//! Mach-style preemptive scheduler with per-CPU multi-level priority queues.
//!
//! 128 priority levels (0–127, higher = more important) with O(1) bitmap-indexed
//! thread selection. Each CPU maintains its own set of priority queues. Idle CPUs
//! steal work from the busiest CPU. Lazy FPU/SSE/AVX switching via CR0.TS avoids
//! saving/restoring XSAVE state (832 bytes with AVX) on every context switch —
//! only threads that actually use FPU/SSE/AVX pay the cost.

// --- Submodules ---
mod run_queue;
mod deferred;
mod fpu;
mod perm;
mod spawn;
mod accessors;
mod priority;
mod thread_config;
mod wait;
mod fork;
mod diagnostics;
mod fd_table;
mod signals;
mod lifecycle;
mod debug_trace;

// Re-export all public API functions from submodules.
pub use fpu::*;
pub use perm::*;
pub use spawn::*;
pub use accessors::*;
pub use priority::*;
pub use thread_config::*;
pub use wait::*;
pub use fork::*;
pub use diagnostics::*;
pub use fd_table::*;
pub use signals::*;
pub use lifecycle::*;
pub use debug_trace::*;

use crate::sync::spinlock::Spinlock;
use crate::task::context::CpuContext;
use crate::task::thread::{Thread, ThreadState};
use crate::arch::hal::MAX_CPUS;
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use run_queue::RunQueue;
use deferred::DEFERRED_PD_DESTROY;

/// Number of discrete priority levels (Mach-style, like macOS).
const NUM_PRIORITIES: usize = 128;
const MAX_PRIORITY: u8 = (NUM_PRIORITIES - 1) as u8; // 127

/// Lowest valid kernel virtual address (architecture-specific higher-half base).
#[cfg(target_arch = "x86_64")]
pub(crate) const KERNEL_ADDR_MIN: u64 = 0xFFFF_FFFF_8000_0000;
#[cfg(target_arch = "aarch64")]
pub(crate) const KERNEL_ADDR_MIN: u64 = 0xFFFF_0000_8000_0000;

/// Valid kernel code PC range (architecture-specific).
#[cfg(target_arch = "x86_64")]
const KERNEL_PC_MIN: u64 = 0xFFFF_FFFF_8010_0000;
#[cfg(target_arch = "x86_64")]
const KERNEL_PC_MAX: u64 = 0xFFFF_FFFF_8200_0000;
#[cfg(target_arch = "aarch64")]
const KERNEL_PC_MIN: u64 = 0xFFFF_0000_8040_0000;
#[cfg(target_arch = "aarch64")]
const KERNEL_PC_MAX: u64 = 0xFFFF_0000_C000_0000;

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

/// TID whose FPU/SSE/AVX state is currently loaded in this CPU's registers.
/// 0 = no owner (default state after boot or after xsave/fxsave).
static PER_CPU_FPU_OWNER: [AtomicU32; MAX_CPUS] = {
    const INIT: AtomicU32 = AtomicU32::new(0);
    [INIT; MAX_CPUS]
};

/// Raw pointer to the current thread's FxState data buffer (64-byte aligned).
/// Set by schedule_inner, read by the #NM handler (lock-free).
static PER_CPU_FPU_PTR: [AtomicU64; MAX_CPUS] = {
    const INIT: AtomicU64 = AtomicU64::new(0);
    [INIT; MAX_CPUS]
};

/// Per-CPU scratch CpuContext used when the outgoing thread has been reaped
/// and we need to context_switch to idle but have no valid old_ctx to save into.
/// The saved state is discarded — we just need a writable CpuContext address.
static mut SCRATCH_CTX: [CpuContext; MAX_CPUS] = {
    #[cfg(target_arch = "x86_64")]
    const INIT: CpuContext = CpuContext {
        rax: 0, rbx: 0, rcx: 0, rdx: 0,
        rsi: 0, rdi: 0, rbp: 0,
        r8: 0, r9: 0, r10: 0, r11: 0,
        r12: 0, r13: 0, r14: 0, r15: 0,
        rsp: 0, rip: 0, rflags: 0, cr3: 0,
        save_complete: 1,
        canary: 0,
        checksum: 0,
    };
    #[cfg(target_arch = "aarch64")]
    const INIT: CpuContext = CpuContext {
        x: [0; 31],
        sp: 0, pc: 0, pstate: 0, ttbr0: 0, tpidr: 0,
        save_complete: 1,
        canary: 0,
        checksum: 0,
    };
    [INIT; MAX_CPUS]
};

/// Per-CPU scratch FPU buffer for the same reaped-outgoing-thread case.
/// Sized for XSAVE (832 bytes), aligned to 64 bytes for XSAVE requirement.
#[repr(C, align(64))]
struct AlignedFpuBuf([u8; crate::task::thread::FPU_STATE_SIZE]);
static mut SCRATCH_FPU: [AlignedFpuBuf; MAX_CPUS] = {
    const INIT: AlignedFpuBuf = AlignedFpuBuf([0u8; crate::task::thread::FPU_STATE_SIZE]);
    [INIT; MAX_CPUS]
};

// --- Deferred wake queue (IRQ-safe, lock-free) ---
// IRQ handlers that need to wake a thread store TIDs here (via atomic swap).
// The timer handler drains these every tick and calls wake_thread_inner under
// the SCHEDULER lock.  This avoids blocking `SCHEDULER.lock()` in IRQ context,
// which can cause RSP corruption when the IRQ handler stalls on a contended lock.

/// Up to 4 deferred-wake TIDs (0 = empty slot).
static DEFERRED_WAKE_TIDS: [AtomicU32; 4] = {
    const INIT: AtomicU32 = AtomicU32::new(0);
    [INIT; 4]
};

/// Enqueue a TID for deferred wake (called from IRQ context, lock-free).
/// Overwrites the oldest slot if all slots are occupied.  Missing a wake
/// is acceptable — the compositor's 16ms timeout provides a safety net.
pub fn deferred_wake(tid: u32) {
    for slot in &DEFERRED_WAKE_TIDS {
        // Try to claim an empty slot (0 → tid).
        if slot.compare_exchange(0, tid, Ordering::Release, Ordering::Relaxed).is_ok() {
            return;
        }
        // If this slot already holds our TID, no-op (avoid duplicate wakes).
        if slot.load(Ordering::Relaxed) == tid {
            return;
        }
    }
    // All slots full — overwrite slot 0 (best-effort; timeout covers misses).
    DEFERRED_WAKE_TIDS[0].store(tid, Ordering::Release);
}

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

/// Read CPU ID (always accurate, even after migration).
fn get_cpu_id() -> usize {
    let c = crate::arch::hal::cpu_id();
    if c < MAX_CPUS { c } else { 0 }
}

// =============================================================================
// Scheduler core
// =============================================================================

/// Per-CPU scheduling state.
struct PerCpuState {
    /// TID of the thread currently executing on this CPU, or None if idle.
    current_tid: Option<u32>,
    /// Cached index into `threads` Vec for the current thread. O(1) lookup
    /// instead of O(n) find_idx on every accessor call. Validated by TID match.
    current_idx: Option<usize>,
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

/// Idle thread entry point. Uses MONITOR/MWAIT when available (faster wake-up,
/// better VMware scheduling), falling back to STI;HLT (x86) or WFI (ARM64).
extern "C" fn idle_thread_entry() {
    #[cfg(target_arch = "x86_64")]
    {
        let use_mwait = crate::arch::hal::has_mwait();

        if use_mwait {
            // MWAIT with "interrupts as break events" (ECX bit 0 = 1) exits on
            // any interrupt, matching HLT semantics. EAX=0x00 requests C1 (lowest
            // latency). The watched address is a dummy — wake is interrupt-driven.
            let watch: u64 = 0;
            loop {
                unsafe {
                    core::arch::asm!(
                        "sti",
                        "mov rax, {addr}",
                        "xor ecx, ecx",
                        "xor edx, edx",
                        "monitor",
                        "xor eax, eax",
                        "mov ecx, 1",
                        "mwait",
                        addr = in(reg) &watch as *const u64 as u64,
                        options(nomem, nostack),
                    );
                }
            }
        } else {
            loop {
                unsafe { core::arch::asm!("sti; hlt"); }
            }
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        loop {
            crate::arch::hal::enable_interrupts();
            crate::arch::hal::halt();
        }
    }
}

impl Scheduler {
    fn new() -> Self {
        let mut per_cpu = Vec::with_capacity(MAX_CPUS);
        for _ in 0..MAX_CPUS {
            per_cpu.push(PerCpuState {
                current_tid: None,
                current_idx: None,
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

    /// Find a thread's index in the threads Vec by TID. O(n) linear scan.
    #[inline]
    fn find_idx(&self, tid: u32) -> Option<usize> {
        self.threads.iter().position(|t| t.tid == tid)
    }

    /// Get index of the current thread on the given CPU.
    /// O(1) via cached index (validated by TID match), O(n) fallback.
    /// Eliminates redundant find_idx calls in the 38+ accessor functions.
    #[inline]
    fn current_idx(&self, cpu_id: usize) -> Option<usize> {
        let tid = self.per_cpu[cpu_id].current_tid?;
        if let Some(cached) = self.per_cpu[cpu_id].current_idx {
            if cached < self.threads.len() && self.threads[cached].tid == tid {
                return Some(cached);
            }
        }
        // Cache miss — fall back to linear scan
        self.find_idx(tid)
    }

    /// Number of online CPUs (at least 1).
    #[inline]
    fn num_cpus(&self) -> usize {
        crate::arch::hal::cpu_count()
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
    /// Sets both `last_cpu` and `affinity_cpu` to the selected CPU.
    ///
    /// Accepts a pre-boxed `Thread` so that the heap allocation happens
    /// **before** the caller acquires the SCHEDULER lock — preventing
    /// ALLOCATOR contention from holding SCHEDULER for 100-400 ms during
    /// fork storms (clone_pd + thousands of free_frame calls contend the
    /// ALLOCATOR lock; Box::new inside SCHEDULER would block there).
    fn add_thread(&mut self, mut thread: Box<Thread>) -> u32 {
        let tid = thread.tid;
        let cpu = self.least_loaded_cpu();
        let pri = thread.priority;
        thread.last_cpu = cpu;
        thread.affinity_cpu = cpu;
        self.threads.push(thread);
        self.per_cpu[cpu].run_queue.enqueue(tid, pri);
        tid
    }

    /// Add a thread in Blocked state without putting it in any ready queue.
    /// Sets `affinity_cpu` to the least-loaded CPU so the first wake goes there.
    ///
    /// See [`add_thread`] — pre-boxing avoids ALLOCATOR contention inside the lock.
    fn add_thread_blocked(&mut self, mut thread: Box<Thread>) -> u32 {
        let tid = thread.tid;
        thread.state = ThreadState::Blocked;
        let cpu = self.least_loaded_cpu();
        thread.last_cpu = cpu;
        thread.affinity_cpu = cpu;
        self.threads.push(thread);
        tid
    }

    /// Remove a TID from ALL per-CPU ready queues.
    fn remove_from_all_queues(&mut self, tid: u32) {
        for cpu in 0..MAX_CPUS {
            self.per_cpu[cpu].run_queue.remove(tid);
        }
    }

    /// Reap terminated threads whose exit code has been consumed or auto-reaped.
    ///
    /// Returns up to 8 reaped `Box<Thread>` objects so the caller can drop
    /// them **outside** the SCHEDULER lock.  Dropping `Box<Thread>` calls
    /// `dealloc` on the ~68 KiB kernel stack; doing that under the lock while
    /// other CPUs contest the ALLOCATOR (e.g. during a fork storm) causes a
    /// 100–400 ms SPIN TIMEOUT.
    fn reap_terminated(&mut self) -> [Option<Box<Thread>>; 8] {
        let mut reaped: [Option<Box<Thread>>; 8] = Default::default();
        let mut reap_count = 0usize;
        let current_tick = crate::arch::hal::timer_current_ticks();
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

                    // SAFETY: Do NOT reap if any CPU still has this thread as
                    // current_tid.  The timer path uses try_lock — if a vCPU was
                    // paused by the hypervisor (VirtualBox NEM, Hyper-V) it may
                    // not have entered schedule_inner yet.  Its interrupt handler
                    // stack frames are still on this thread's kernel stack.
                    // Freeing the stack now would cause IRETQ to load garbage
                    // (RIP/RSP from zeroed or reused heap memory → #UD / #DF).
                    // Skip now; the CPU will switch away on its next tick, then
                    // the NEXT reap pass will find no CPU referencing this TID.
                    let still_current_on_cpu = (0..MAX_CPUS).any(|cpu| {
                        self.per_cpu[cpu].current_tid == Some(tid)
                    });
                    if still_current_on_cpu {
                        i += 1;
                        continue;
                    }

                    self.remove_from_all_queues(tid);
                    // swap_remove RETURNS the Box<Thread> — do NOT let it drop here.
                    // We collect it and drop it after releasing the SCHEDULER lock
                    // (see schedule_inner) so dealloc doesn't contend the ALLOCATOR
                    // while we hold the lock.
                    let thread = self.threads.swap_remove(i);
                    // Maintain current_idx caches: swap_remove moved the
                    // last element into position i.
                    let moved_from = self.threads.len();
                    if i < self.threads.len() {
                        for cpu in 0..MAX_CPUS {
                            if self.per_cpu[cpu].current_idx == Some(moved_from) {
                                self.per_cpu[cpu].current_idx = Some(i);
                            }
                        }
                    }
                    if reap_count < 8 {
                        reaped[reap_count] = Some(thread);
                        reap_count += 1;
                    }
                    // Don't increment — check swapped-in element
                } else {
                    i += 1;
                }
            } else {
                i += 1;
            }
        }
        reaped
    }

    /// Pick the next thread for this CPU: local queue first, then work stealing.
    /// Returns None if no eligible thread is available.
    fn pick_next(&mut self, cpu_id: usize) -> Option<u32> {
        // 1. Try local queue
        if let Some(tid) = self.pick_eligible(cpu_id) {
            return Some(tid);
        }
        // 2. Work stealing: find the busiest CPU and steal only when the
        //    imbalance is large enough to justify cache-line invalidation.
        //    With few threads and many CPUs, overeager stealing causes
        //    constant migration of heavy threads (DOOM, compositor).
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
        // Only steal when the victim has 3+ queued threads — this keeps 2 for
        // the victim (1 running + 1 queued) before we take one.  Prevents
        // thrashing when 2 threads share a CPU via affinity.
        if max_count >= 3 {
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

    /// Wake a blocked thread, enqueuing on its stable `affinity_cpu`.
    fn wake_thread_inner(&mut self, tid: u32) {
        if let Some(idx) = self.find_idx(tid) {
            if self.threads[idx].state == ThreadState::Blocked {
                self.threads[idx].state = ThreadState::Ready;
                let cpu = self.threads[idx].affinity_cpu;
                let n = self.num_cpus();
                let target = if cpu < n { cpu } else { 0 };
                self.per_cpu[target].run_queue.enqueue(tid, self.threads[idx].priority);
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
// Scheduling
// =============================================================================

/// Called from the timer interrupt for preemptive scheduling.
/// Returns false (and does nothing) if this CPU is already inside schedule_inner,
/// preventing re-entrant scheduling that causes context corruption and deadlocks.
pub fn schedule_tick() -> bool {
    #[cfg(target_arch = "aarch64")]
    {
        static TICK_DBG: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
        let n = TICK_DBG.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        if n < 3 {
            crate::serial_println!("  [SCHED] schedule_tick #{}", n);
        }
    }
    let cpu_id = crate::arch::hal::cpu_id();
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
        saved_flags = crate::arch::hal::save_and_disable_interrupts();
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
        crate::arch::hal::restore_interrupt_state(saved_flags);
    }

    // Drain deferred PD destruction queue BEFORE acquiring the scheduler lock.
    // destroy_user_page_directory is slow (page-table walk + hundreds of
    // free_frame calls); running it under the scheduler lock causes SPIN TIMEOUT
    // on other CPUs waiting for the lock.
    //
    // Only CPU 0 drains, once per timer tick, to ensure that threads which were
    // running on another CPU at kill time have had at least one tick to
    // context-switch away before we touch their page tables.
    if from_timer && cpu_id_early == 0 {
        let pds = DEFERRED_PD_DESTROY.lock().drain();
        for entry in pds.iter().flatten() {
            let (pd, tid) = *entry;
            if tid != 0 {
                // Thread was still running on another CPU at kill time.
                // cleanup_process hasn't run yet; do it now with the correct CR3.
                {
                    let rflags = crate::arch::hal::save_and_disable_interrupts();
                    let old_cr3 = crate::arch::hal::current_page_table();
                    crate::arch::hal::switch_page_table(pd.as_u64());
                    crate::ipc::shared_memory::cleanup_process(tid);
                    crate::arch::hal::switch_page_table(old_cr3);
                    crate::arch::hal::restore_interrupt_state(rflags);
                }
            }
            // tid == 0: cleanup_process already ran in kill_thread — just destroy.
            crate::memory::virtual_mem::destroy_user_page_directory(pd);
            crate::memory::vma::destroy_process(pd);
        }
    }

    // Tick counters (timer path only)
    if from_timer {
        TOTAL_SCHED_TICKS.fetch_add(1, Ordering::Relaxed);
        PER_CPU_TOTAL[cpu_id_early].fetch_add(1, Ordering::Relaxed);
    }

    // Lock acquisition: try_lock for timer (non-blocking), spin for voluntary
    crate::sched_diag::set(cpu_id_early, if from_timer {
        crate::sched_diag::PHASE_SCHEDULE_TIMER
    } else {
        crate::sched_diag::PHASE_SCHEDULE_VOLUNTARY
    });
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
    // Reaped Box<Thread> objects deferred for drop AFTER lock release.
    // Dropping Box<Thread> frees the ~68 KiB kernel stack via dealloc; doing
    // that under the SCHEDULER lock while the ALLOCATOR is contended (fork
    // storms with concurrent clone_user_page_directory) causes SPIN TIMEOUT.
    let mut reaped_threads: [Option<Box<Thread>>; 8] = Default::default();

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
            reaped_threads = sched.reap_terminated();
        }

        // Drain deferred wakes (IRQ handlers store TIDs here lock-free).
        // Process under the already-held lock to avoid extra lock/unlock.
        for slot in &DEFERRED_WAKE_TIDS {
            let tid = slot.swap(0, Ordering::Acquire);
            if tid != 0 {
                sched.wake_thread_inner(tid);
            }
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
        let n_cpus = sched.num_cpus();
        if from_timer {
            let current_tick = crate::arch::hal::timer_current_ticks();
            for i in 0..sched.threads.len() {
                if sched.threads[i].state == ThreadState::Blocked {
                    if let Some(wake_tick) = sched.threads[i].wake_at_tick {
                        if current_tick.wrapping_sub(wake_tick) < 0x8000_0000 {
                            let tid = sched.threads[i].tid;
                            let pri = sched.threads[i].priority;
                            let aff = sched.threads[i].affinity_cpu;
                            let target_cpu = if aff < n_cpus { aff } else { 0 };
                            sched.threads[i].state = ThreadState::Ready;
                            sched.threads[i].wake_at_tick = None;
                            sched.per_cpu[target_cpu].run_queue.enqueue(tid, pri);
                        }
                    }
                }
            }
        }

        // --- Periodic affinity rebalancing (CPU 0 only, every ~1 second) ---
        // Counts how many Ready/Running threads have affinity to each CPU.
        // If any CPU is overloaded (3+ more than the lightest), migrate one
        // thread's affinity to the lightest CPU.  This is the ONLY place
        // where affinity_cpu changes after spawn.
        if from_timer && cpu_id == 0 {
            static REBALANCE_CTR: AtomicU32 = AtomicU32::new(0);
            let ctr = REBALANCE_CTR.fetch_add(1, Ordering::Relaxed);
            if ctr % 1000 == 0 {
                let mut aff_count = [0u32; MAX_CPUS];
                for t in sched.threads.iter() {
                    if t.is_idle { continue; }
                    match t.state {
                        ThreadState::Ready | ThreadState::Running => {
                            let c = t.affinity_cpu;
                            if c < n_cpus { aff_count[c] += 1; }
                        }
                        _ => {}
                    }
                }
                // Find busiest and lightest
                let mut busiest_cpu = 0usize;
                let mut busiest_val = 0u32;
                let mut lightest_cpu = 0usize;
                let mut lightest_val = u32::MAX;
                for c in 0..n_cpus {
                    if aff_count[c] > busiest_val {
                        busiest_val = aff_count[c];
                        busiest_cpu = c;
                    }
                    if aff_count[c] < lightest_val {
                        lightest_val = aff_count[c];
                        lightest_cpu = c;
                    }
                }
                // Only rebalance if imbalance >= 3 threads
                if busiest_val >= lightest_val + 3 && busiest_cpu != lightest_cpu {
                    // Migrate the lowest-priority non-idle thread from busiest
                    let mut victim_idx: Option<usize> = None;
                    let mut victim_pri = 0u8;
                    for (i, t) in sched.threads.iter().enumerate() {
                        if t.is_idle || t.critical { continue; }
                        if t.affinity_cpu == busiest_cpu {
                            match t.state {
                                ThreadState::Ready | ThreadState::Running => {
                                    if victim_idx.is_none() || t.priority >= victim_pri {
                                        victim_idx = Some(i);
                                        victim_pri = t.priority;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    if let Some(vi) = victim_idx {
                        sched.threads[vi].affinity_cpu = lightest_cpu;
                    }
                }
            }
        }

        // --- Cache commonly-used indices (eliminates 10+ redundant find_idx calls) ---
        let idle_tid = sched.idle_tid[cpu_id];
        let idle_idx = sched.find_idx(idle_tid).expect("idle thread missing");
        let outgoing_tid = sched.per_cpu[cpu_id].current_tid;
        let outgoing_is_idle = outgoing_tid == Some(idle_tid);
        let outgoing_idx = if outgoing_is_idle {
            Some(idle_idx)
        } else {
            outgoing_tid.and_then(|t| sched.find_idx(t))
        };

        // Drain contended-busy ticks
        let missed = PER_CPU_CONTENDED_BUSY[cpu_id].swap(0, Ordering::Relaxed);
        if missed > 0 && !outgoing_is_idle {
            if let Some(idx) = outgoing_idx {
                sched.threads[idx].cpu_ticks += missed;
            }
        }

        // CPU tick accounting
        if from_timer {
            if outgoing_is_idle {
                IDLE_SCHED_TICKS.fetch_add(1, Ordering::Relaxed);
                PER_CPU_IDLE[cpu_id].fetch_add(1, Ordering::Relaxed);
            } else if let Some(idx) = outgoing_idx {
                if sched.threads[idx].state == ThreadState::Running {
                    sched.threads[idx].cpu_ticks += 1;
                }
            }
        }

        // --- Put current thread back into its priority queue ---
        if !outgoing_is_idle {
            if let Some(idx) = outgoing_idx {
                // ALWAYS mark context as unsaved for non-idle outgoing threads.
                sched.threads[idx].context.save_complete = 0;
                if sched.threads[idx].state == ThreadState::Running {
                    sched.threads[idx].state = ThreadState::Ready;
                    sched.threads[idx].last_cpu = cpu_id;
                    let pri = sched.threads[idx].priority;
                    sched.per_cpu[cpu_id].run_queue.enqueue(
                        outgoing_tid.unwrap(), pri);
                }
            }
        }

        // --- Pick next thread (O(1) via bitmap) ---
        #[cfg(target_arch = "aarch64")]
        {
            static PICK_DBG: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
            let n = PICK_DBG.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            if n < 5 {
                let ready_count = sched.threads.iter()
                    .filter(|t| t.state == ThreadState::Ready).count();
                crate::serial_println!("  [SCHED] pick_next: cpu={} outgoing={:?} ready={} total={}",
                    cpu_id, outgoing_tid, ready_count, sched.threads.len());
            }
        }
        switch_info = if let Some(next_tid) = sched.pick_next(cpu_id) {
            if let Some(next_idx) = sched.find_idx(next_tid) {
                let kstack_top = sched.threads[next_idx].kernel_stack_top();
                let kstack_bottom = sched.threads[next_idx].kernel_stack_bottom();

                // Validate candidate before committing
                let kstack_valid = kstack_top >= KERNEL_ADDR_MIN;
                if !kstack_valid {
                    crate::serial_println!(
                        "BUG: thread '{}' (TID={}) invalid kstack_top={:#x} — killing",
                        sched.threads[next_idx].name_str(), next_tid, kstack_top,
                    );
                    sched.threads[next_idx].state = ThreadState::Terminated;
                    sched.threads[next_idx].exit_code = Some(139);
                    sched.threads[next_idx].terminated_at_tick =
                        Some(crate::arch::hal::timer_current_ticks());
                    // Restore outgoing as current
                    sched.per_cpu[cpu_id].current_tid = outgoing_tid;
                    sched.per_cpu[cpu_id].current_idx = outgoing_idx;
                    if let Some(oi) = outgoing_idx {
                        sched.threads[oi].context.save_complete = 1;
                    }
                    None
                } else {
                    // Commit: update per-CPU state
                    sched.per_cpu[cpu_id].current_tid = Some(next_tid);
                    sched.per_cpu[cpu_id].current_idx = Some(next_idx);
                    sched.threads[next_idx].state = ThreadState::Running;
                    sched.threads[next_idx].last_cpu = cpu_id;

                    PER_CPU_HAS_THREAD[cpu_id].store(true, Ordering::Relaxed);
                    PER_CPU_CURRENT_TID[cpu_id].store(next_tid, Ordering::Relaxed);
                    PER_CPU_IS_USER[cpu_id].store(sched.threads[next_idx].is_user, Ordering::Relaxed);
                    update_per_cpu_name(cpu_id, &sched.threads[next_idx].name);

                    // Update TSS.RSP0 and SYSCALL kernel RSP
                    crate::arch::hal::set_kernel_stack_for_cpu(cpu_id, kstack_top);
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
                    } else if let Some(prev_idx) = outgoing_idx {
                        // Use cached outgoing_idx (same thread, avoids redundant find_idx)
                        let prev_tid = outgoing_tid.unwrap();
                        let old_ctx = &mut sched.threads[prev_idx].context as *mut CpuContext;
                        let new_ctx = &sched.threads[next_idx].context as *const CpuContext;
                        let old_fpu = sched.threads[prev_idx].fpu_state.data.as_mut_ptr();
                        let new_fpu = sched.threads[next_idx].fpu_state.data.as_ptr();
                        Some((old_ctx, new_ctx, old_fpu, new_fpu, prev_tid, next_tid))
                    } else {
                        // Previous thread reaped or no previous — switch from idle
                        let old_ctx = &mut sched.threads[idle_idx].context as *mut CpuContext;
                        let new_ctx = &sched.threads[next_idx].context as *const CpuContext;
                        let old_fpu = sched.threads[idle_idx].fpu_state.data.as_mut_ptr();
                        let new_fpu = sched.threads[next_idx].fpu_state.data.as_ptr();
                        Some((old_ctx, new_ctx, old_fpu, new_fpu, idle_tid, next_tid))
                    }
                }
            } else {
                // TID reaped between pick_next and here
                sched.per_cpu[cpu_id].current_tid = outgoing_tid;
                sched.per_cpu[cpu_id].current_idx = outgoing_idx;
                if let Some(oi) = outgoing_idx {
                    sched.threads[oi].context.save_complete = 1;
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
            if let Some(current_tid) = outgoing_tid {
                if let Some(idx) = outgoing_idx {
                    if sched.threads[idx].state != ThreadState::Running {
                        sched.threads[idx].context.save_complete = 0;
                        sched.per_cpu[cpu_id].current_tid = Some(idle_tid);
                        sched.per_cpu[cpu_id].current_idx = Some(idle_idx);
                        sched.threads[idle_idx].state = ThreadState::Running;
                        PER_CPU_HAS_THREAD[cpu_id].store(false, Ordering::Relaxed);
                        PER_CPU_IS_USER[cpu_id].store(false, Ordering::Relaxed);
                        PER_CPU_CURRENT_TID[cpu_id].store(idle_tid, Ordering::Relaxed);
                        update_per_cpu_name(cpu_id, &sched.threads[idle_idx].name);
                        let idle_kstack_top = sched.threads[idle_idx].kernel_stack_top();
                        crate::arch::hal::set_kernel_stack_for_cpu(cpu_id, idle_kstack_top);
                        PER_CPU_STACK_BOTTOM[cpu_id].store(sched.threads[idle_idx].kernel_stack_bottom(), Ordering::Relaxed);
                        PER_CPU_STACK_TOP[cpu_id].store(idle_kstack_top, Ordering::Relaxed);
                        PER_CPU_FPU_PTR[cpu_id].store(
                            sched.threads[idle_idx].fpu_state.data.as_ptr() as u64,
                            Ordering::Relaxed,
                        );
                        let old_ctx = &mut sched.threads[idx].context as *mut CpuContext;
                        let idle_ctx = &sched.threads[idle_idx].context as *const CpuContext;
                        let old_fpu = sched.threads[idx].fpu_state.data.as_mut_ptr();
                        let new_fpu = sched.threads[idle_idx].fpu_state.data.as_ptr();
                        Some((old_ctx, idle_ctx, old_fpu, new_fpu, current_tid, idle_tid))
                    } else {
                        if !sched.threads[idx].is_idle {
                            sched.threads[idx].context.save_complete = 1;
                        }
                        None
                    }
                } else {
                    // Current thread reaped — MUST context_switch to idle.
                    crate::serial_println!(
                        "!REAPED-CURRENT: CPU{} tid={} → idle, switching via scratch ctx",
                        cpu_id, current_tid,
                    );
                    sched.per_cpu[cpu_id].current_tid = Some(idle_tid);
                    sched.per_cpu[cpu_id].current_idx = Some(idle_idx);
                    PER_CPU_HAS_THREAD[cpu_id].store(false, Ordering::Relaxed);
                    PER_CPU_IS_USER[cpu_id].store(false, Ordering::Relaxed);
                    PER_CPU_CURRENT_TID[cpu_id].store(idle_tid, Ordering::Relaxed);
                    update_per_cpu_name(cpu_id, &sched.threads[idle_idx].name);
                    let idle_kstack_top = sched.threads[idle_idx].kernel_stack_top();
                    crate::arch::hal::set_kernel_stack_for_cpu(cpu_id, idle_kstack_top);
                    PER_CPU_STACK_BOTTOM[cpu_id].store(sched.threads[idle_idx].kernel_stack_bottom(), Ordering::Relaxed);
                    PER_CPU_STACK_TOP[cpu_id].store(idle_kstack_top, Ordering::Relaxed);
                    PER_CPU_FPU_PTR[cpu_id].store(
                        sched.threads[idle_idx].fpu_state.data.as_ptr() as u64,
                        Ordering::Relaxed,
                    );
                    let scratch_ctx = unsafe { &mut SCRATCH_CTX[cpu_id] as *mut CpuContext };
                    let idle_ctx = &sched.threads[idle_idx].context as *const CpuContext;
                    let scratch_fpu = unsafe { SCRATCH_FPU[cpu_id].0.as_mut_ptr() };
                    let new_fpu = sched.threads[idle_idx].fpu_state.data.as_ptr();
                    Some((scratch_ctx, idle_ctx, scratch_fpu, new_fpu, idle_tid, idle_tid))
                }
            } else {
                sched.per_cpu[cpu_id].current_tid = Some(idle_tid);
                sched.per_cpu[cpu_id].current_idx = Some(idle_idx);
                PER_CPU_HAS_THREAD[cpu_id].store(false, Ordering::Relaxed);
                let idle_kstack_top = sched.threads[idle_idx].kernel_stack_top();
                crate::arch::hal::set_kernel_stack_for_cpu(cpu_id, idle_kstack_top);
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
                    || ctx.get_pc() < KERNEL_PC_MIN
                    || ctx.get_pc() >= KERNEL_PC_MAX
                    || ctx.get_sp() < KERNEL_ADDR_MIN
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
                        Some(crate::arch::hal::timer_current_ticks());
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
                    crate::arch::hal::set_kernel_stack_for_cpu(cpu_id, kstack_top);
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
            #[cfg(target_arch = "x86_64")]
            {
                let names = [
                    "rax", "rbx", "rcx", "rdx", "rsi", "rdi", "rbp",
                    "r8 ", "r9 ", "r10", "r11", "r12", "r13", "r14", "r15",
                    "rsp", "rip", "rfl", "cr3", "sav", "can", "chk",
                ];
                for i in 0..22 {
                    crate::serial_println!("  [{}] {} = {:#018x}", i * 8, names[i], *p.add(i));
                }
            }
            #[cfg(target_arch = "aarch64")]
            {
                // Dump x0-x30 + sp + pc + pstate + ttbr0 + tpidr + sav + can + chk
                for i in 0..31 {
                    crate::serial_println!("  [{}] x{} = {:#018x}", i * 8, i, *p.add(i));
                }
                let names = ["sp ", "pc ", "pst", "tt0", "tpi", "sav", "can", "chk"];
                for i in 0..8 {
                    crate::serial_println!("  [{}] {} = {:#018x}", (31 + i) * 8, names[i], *p.add(31 + i));
                }
            }
        }
    }

    // Drop reaped threads NOW — BEFORE context_switch, while interrupts are
    // disabled and the stack frame is pristine.  Previously the implicit drop
    // happened at function return (AFTER context_switch + sti), which is
    // problematic:
    //   1. context_switch suspends this function; the resume may be on a
    //      different CPU tick, and the compiler's drop-glue register state
    //      (base pointer for the array iteration) can be corrupted if a
    //      callee-saved register spill slot on the stack is overwritten.
    //   2. After sti, a timer interrupt can fire mid-drop, re-entering
    //      schedule_inner and pushing a deep stack frame that overlaps
    //      with the reaped_threads array.
    // By dropping here (lock released, IF=0), we avoid both issues and
    // still keep dealloc contention outside the SCHEDULER lock.
    drop(reaped_threads);

    // Context switch with lock released, interrupts still disabled
    if let Some((old_ctx, new_ctx, old_fpu, _new_fpu, outgoing_tid, _next_tid)) = switch_info {
        // --- Lazy FPU: save outgoing thread's state if this CPU owns it ---
        let fpu_owner = PER_CPU_FPU_OWNER[cpu_id].load(Ordering::Relaxed);
        if fpu_owner != 0 && fpu_owner == outgoing_tid {
            crate::arch::hal::fpu_save(old_fpu);
            PER_CPU_FPU_OWNER[cpu_id].store(0, Ordering::Relaxed);
        }

        // Set FPU trap — next FPU/SSE instruction triggers lazy restore
        crate::arch::hal::fpu_set_trap();

        // Clear in-scheduler flag BEFORE context_switch. Interrupts are disabled
        // (release_no_irq_restore kept IF=0), so no timer can fire between the
        // clear and the switch. This is CRITICAL because if the new thread is
        // starting for the first time (RIP = entry point, not inside schedule_inner),
        // it will never reach the post-switch cleanup code below.
        PER_CPU_IN_SCHEDULER[cpu_id].store(false, Ordering::Relaxed);

        #[cfg(target_arch = "aarch64")]
        {
            let next_pc = unsafe { (*new_ctx).get_pc() };
            let next_sp = unsafe { (*new_ctx).get_sp() };
            let next_x30 = unsafe { (*new_ctx).x[30] };
            crate::serial_println!("  [SCHED] ctx_switch: out={} in={} pc={:#x} sp={:#x} x30={:#x}",
                outgoing_tid, _next_tid, next_pc, next_sp, next_x30);
        }
        unsafe { crate::task::context::context_switch(old_ctx, new_ctx); }
    }

    // Also clear after context_switch for the no-switch path (switch_info was None).
    // After context_switch we may be on a different CPU, so re-read cpu_id.
    let cpu_id_exit = get_cpu_id();
    if cpu_id_exit < MAX_CPUS {
        PER_CPU_IN_SCHEDULER[cpu_id_exit].store(false, Ordering::Relaxed);
    }

    // Re-enable interrupts (CRITICAL for voluntary schedule — without this, IF stays 0)
    crate::arch::hal::enable_interrupts();
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
                crate::arch::hal::set_kernel_stack_for_cpu(0, kstack_top);
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
    crate::arch::hal::enable_interrupts();
    #[cfg(target_arch = "aarch64")]
    {
        let daif: u64;
        unsafe { core::arch::asm!("mrs {}, daif", out(reg) daif, options(nomem, nostack)); }
        let ticks = crate::arch::arm64::generic_timer::get_ticks();
        crate::serial_println!("  [IDLE] entering idle loop, DAIF={:#x} ticks={}", daif, ticks);

        // Check if timer is actually armed
        let ctl: u64;
        unsafe { core::arch::asm!("mrs {}, cntp_ctl_el0", out(reg) ctl, options(nomem, nostack)); }
        crate::serial_println!("  [IDLE] CNTP_CTL={:#x} (ENABLE={}, IMASK={}, ISTATUS={})",
            ctl, ctl & 1, (ctl >> 1) & 1, (ctl >> 2) & 1);
    }
    loop { crate::arch::hal::halt(); }
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
            sched.per_cpu[cpu_id].current_idx = Some(idx);
            sched.threads[idx].state = ThreadState::Running;
            let kstack_top = sched.threads[idx].kernel_stack_top();
            let kstack_bottom = sched.threads[idx].kernel_stack_bottom();
            crate::debug_println!("  [Sched] register_ap_idle: cpu={} kstack=[{:#x}..{:#x}]",
                cpu_id, kstack_bottom, kstack_top);
            crate::arch::hal::set_kernel_stack_for_cpu(cpu_id, kstack_top);
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
