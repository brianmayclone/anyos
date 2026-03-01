//! Mach-style preemptive scheduler with per-CPU multi-level priority queues.
//!
//! 128 priority levels (0–127, higher = more important) with O(1) bitmap-indexed
//! thread selection. Each CPU maintains its own set of priority queues. Idle CPUs
//! steal work from the busiest CPU. Lazy FPU/SSE/AVX switching via CR0.TS avoids
//! saving/restoring XSAVE state (832 bytes with AVX) on every context switch —
//! only threads that actually use FPU/SSE/AVX pay the cost.

use crate::memory::address::PhysAddr;
use crate::memory::virtual_mem;
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
/// Deferred page-directory destruction queue.
///
/// `kill_thread` **must not** call `destroy_user_page_directory` while holding
/// the scheduler lock — page-table walks and hundreds of `free_frame` calls can
/// take milliseconds, causing SPIN TIMEOUT on other CPUs waiting for the lock.
///
/// Instead, every dying PD is pushed here and drained on CPU 0's next timer
/// tick, **before** the scheduler lock is acquired (see `schedule_inner`).
///
/// ## tid semantics
/// * `tid != 0`: thread was still running on another CPU at kill time;
///   `cleanup_process(tid)` must run with the dying CR3 before destroy.
/// * `tid == 0`: `cleanup_process` already ran in `kill_thread`; just destroy.
struct DeferredPdQueue {
    entries: [Option<(PhysAddr, u32)>; 64],
}
impl DeferredPdQueue {
    const fn new() -> Self { Self { entries: [None; 64] } }
    fn push(&mut self, pd: PhysAddr, tid: u32) {
        for slot in self.entries.iter_mut() {
            if slot.is_none() { *slot = Some((pd, tid)); return; }
        }
        // Queue full (64 pending PDs) — drain one slot synchronously.
        // This is a last-resort fallback for pathological fork storms.
        crate::serial_println!("WARNING: deferred PD queue full, destroying one synchronously");
        if let Some(Some((old_pd, old_tid))) = self.entries.iter_mut().find(|s| s.is_some()).map(|s| s.take()) {
            if old_tid != 0 {
                unsafe {
                    let rflags: u64;
                    core::arch::asm!("pushfq; pop {}", out(reg) rflags, options(nomem));
                    core::arch::asm!("cli");
                    let saved_cr3 = crate::memory::virtual_mem::current_cr3();
                    core::arch::asm!("mov cr3, {}", in(reg) old_pd.as_u64());
                    crate::ipc::shared_memory::cleanup_process(old_tid);
                    core::arch::asm!("mov cr3, {}", in(reg) saved_cr3);
                    core::arch::asm!("push {}; popfq", in(reg) rflags, options(nomem));
                }
            }
            crate::memory::virtual_mem::destroy_user_page_directory(old_pd);
        }
        // Now there is a free slot — insert the new entry.
        for slot in self.entries.iter_mut() {
            if slot.is_none() { *slot = Some((pd, tid)); return; }
        }
    }
    fn drain(&mut self) -> [Option<(PhysAddr, u32)>; 64] {
        let result = self.entries;
        self.entries = [None; 64];
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
    /// Cached total count — avoids O(128) sum on every `total_count()` call.
    count: usize,
}

impl RunQueue {
    fn new() -> Self {
        let mut levels = Vec::with_capacity(NUM_PRIORITIES);
        for _ in 0..NUM_PRIORITIES {
            levels.push(VecDeque::new());
        }
        RunQueue { levels, bits: [0; 2], count: 0 }
    }

    /// Enqueue a TID at the given priority level (back of FIFO).
    /// Caller must ensure no duplicates (the scheduler guarantees this via
    /// state transitions: only Ready threads are enqueued, and they transition
    /// to Running immediately on pick).
    fn enqueue(&mut self, tid: u32, priority: u8) {
        let p = (priority as usize).min(NUM_PRIORITIES - 1);
        self.levels[p].push_back(tid);
        self.bits[p / 64] |= 1u64 << (p % 64);
        self.count += 1;
    }

    /// Dequeue the highest-priority thread (front of its FIFO). O(1) via bitmap.
    fn dequeue_highest(&mut self) -> Option<u32> {
        let p = self.highest_priority()?;
        let tid = self.levels[p].pop_front()?;
        if self.levels[p].is_empty() {
            self.bits[p / 64] &= !(1u64 << (p % 64));
        }
        self.count -= 1;
        Some(tid)
    }

    /// Dequeue the lowest-priority thread (used for work stealing).
    fn dequeue_lowest(&mut self) -> Option<u32> {
        let p = self.lowest_priority()?;
        let tid = self.levels[p].pop_front()?;
        if self.levels[p].is_empty() {
            self.bits[p / 64] &= !(1u64 << (p % 64));
        }
        self.count -= 1;
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
                self.count -= 1;
                return;
            }
        }
    }

    /// Total number of queued threads across all priority levels. O(1).
    #[inline]
    fn total_count(&self) -> usize {
        self.count
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
/// better VMware scheduling), falling back to STI;HLT otherwise.
extern "C" fn idle_thread_entry() {
    let use_mwait = crate::arch::x86::cpuid::HAS_MWAIT
        .load(core::sync::atomic::Ordering::Relaxed);

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
// Public API — Spawn
// =============================================================================

/// Create a new kernel thread and add it to the ready queue.
pub fn spawn(entry: extern "C" fn(), priority: u8, name: &str) -> u32 {
    let priority = clamp_priority(priority, name);
    let tid = {
        // Box the thread BEFORE acquiring SCHEDULER — prevents ALLOCATOR
        // contention (from concurrent clone_pd) from holding SCHEDULER for
        // 100-400 ms and causing SPIN TIMEOUT on other CPUs.
        let thread = Box::new(Thread::new(entry, priority, name));
        crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_SPAWN);
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
        // Box the thread BEFORE acquiring SCHEDULER — same reasoning as spawn().
        let thread = Box::new(Thread::new(entry, priority, name));
        crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_SPAWN_BLOCKED);
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
    let (pd, arch_mode, brk, parent_pri, parent_cwd, parent_caps, parent_uid, parent_gid, parent_pcid, parent_mmap_next) = {
        crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_CREATE_THREAD);
        let guard = SCHEDULER.lock();
        let cpu_id = get_cpu_id();
        let sched = match guard.as_ref() { Some(s) => s, None => return 0 };
        let current_tid = match sched.per_cpu[cpu_id].current_tid { Some(t) => t, None => return 0 };
        let idx = match sched.find_idx(current_tid) { Some(i) => i, None => return 0 };
        let thread = &sched.threads[idx];
        let pd = match thread.page_directory { Some(pd) => pd, None => return 0 };
        (pd, thread.arch_mode, thread.brk, thread.priority, thread.cwd, thread.capabilities, thread.uid, thread.gid, thread.pcid, thread.mmap_next)
    };

    let effective_pri = if priority == 0 { parent_pri } else { priority };
    let effective_pri = clamp_priority(effective_pri, name);
    let tid = spawn_blocked(crate::task::loader::thread_create_trampoline, effective_pri, name);

    {
        crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_CREATE_THREAD);
        let mut guard = SCHEDULER.lock();
        let sched = guard.as_mut().expect("Scheduler not initialized");
        if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
            thread.page_directory = Some(pd);
            thread.pcid = parent_pcid; // Same address space = same PCID
            thread.context.cr3 = pd.as_u64() | parent_pcid as u64;
            thread.context.checksum = thread.context.compute_checksum();
            thread.is_user = true;
            thread.brk = brk;
            thread.arch_mode = arch_mode;
            thread.pd_shared = true;
            thread.cwd = parent_cwd;
            thread.capabilities = parent_caps;
            thread.uid = parent_uid;
            thread.gid = parent_gid;
            thread.mmap_next = parent_mmap_next;
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
                unsafe {
                    let rflags: u64;
                    core::arch::asm!("pushfq; pop {}", out(reg) rflags, options(nomem));
                    core::arch::asm!("cli");
                    let old_cr3 = crate::memory::virtual_mem::current_cr3();
                    core::arch::asm!("mov cr3, {}", in(reg) pd.as_u64());
                    crate::ipc::shared_memory::cleanup_process(tid);
                    core::arch::asm!("mov cr3, {}", in(reg) old_cr3);
                    core::arch::asm!("push {}; popfq", in(reg) rflags, options(nomem));
                }
            }
            // tid == 0: cleanup_process already ran in kill_thread — just destroy.
            crate::memory::virtual_mem::destroy_user_page_directory(pd);
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
            let current_tick = crate::arch::x86::pit::get_ticks();
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
                        crate::arch::x86::tss::set_kernel_stack_for_cpu(cpu_id, idle_kstack_top);
                        crate::arch::x86::syscall_msr::set_kernel_rsp(cpu_id, idle_kstack_top);
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
                    crate::arch::x86::tss::set_kernel_stack_for_cpu(cpu_id, idle_kstack_top);
                    crate::arch::x86::syscall_msr::set_kernel_rsp(cpu_id, idle_kstack_top);
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
            unsafe {
                if crate::arch::x86::cpuid::HAS_XSAVE.load(Ordering::Relaxed) {
                    // XSAVE with mask -1 saves all XCR0-enabled components (x87+SSE+AVX)
                    core::arch::asm!(
                        "xsave [{}]",
                        in(reg) old_fpu,
                        in("eax") 0xFFFF_FFFFu32,
                        in("edx") 0xFFFF_FFFFu32,
                        options(nostack, preserves_flags),
                    );
                } else {
                    core::arch::asm!("fxsave [{}]", in(reg) old_fpu, options(nostack, preserves_flags));
                }
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

    // Load this thread's FPU/SSE/AVX state
    let fpu_ptr = PER_CPU_FPU_PTR[cpu_id].load(Ordering::Relaxed);
    if fpu_ptr != 0 {
        unsafe {
            if crate::arch::x86::cpuid::HAS_XSAVE.load(Ordering::Relaxed) {
                // XRSTOR with mask -1 restores all XCR0-enabled components
                core::arch::asm!(
                    "xrstor [{}]",
                    in(reg) fpu_ptr,
                    in("eax") 0xFFFF_FFFFu32,
                    in("edx") 0xFFFF_FFFFu32,
                    options(nostack, preserves_flags),
                );
            } else {
                core::arch::asm!("fxrstor [{}]", in(reg) fpu_ptr, options(nostack, preserves_flags));
            }
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
    let idx = match sched.current_idx(cpu_id) { Some(i) => i, None => return };
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
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_GET_THREAD_INFO);
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.page_directory = Some(pd);
        thread.pcid = crate::memory::virtual_mem::allocate_pcid();
        thread.context.cr3 = pd.as_u64() | thread.pcid as u64;
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
        if let Some(idx) = sched.current_idx(cpu_id) {
            return sched.threads[idx].page_directory;
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
        if let Some(idx) = sched.current_idx(cpu_id) {
            return sched.threads[idx].pd_shared;
        }
    }
    false
}

/// Check if any OTHER live thread shares the same page directory.
pub fn has_live_pd_siblings() -> bool {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_HAS_LIVE_PD_SIBS);
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current_idx(cpu_id) {
            let tid = sched.threads[idx].tid;
            if let Some(pd) = sched.threads[idx].page_directory {
                return sched.threads.iter().any(|t| {
                    t.tid != tid
                        && t.page_directory == Some(pd)
                        && t.state != ThreadState::Terminated
                });
            }
        }
    }
    false
}

/// Atomically get all info needed for sys_exit.
pub fn current_exit_info() -> (u32, Option<PhysAddr>, bool) {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_CURRENT_EXIT_INFO);
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current_idx(cpu_id) {
            let tid = sched.threads[idx].tid;
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
    (0, None, false)
}

/// Get the current thread's program break.
pub fn current_thread_brk() -> u32 {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_SET_THREAD_BRK);
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current_idx(cpu_id) {
            return sched.threads[idx].brk;
        }
    }
    0
}

/// Set the current thread's program break, syncing across sibling threads.
pub fn set_current_thread_brk(brk: u32) {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_SET_THREAD_BRK);
    let mut guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_mut() {
        if let Some(idx) = sched.current_idx(cpu_id) {
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

/// Return the current thread's mmap bump pointer.
pub fn current_thread_mmap_next() -> u32 {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_SET_THREAD_MMAP);
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current_idx(cpu_id) {
            return sched.threads[idx].mmap_next;
        }
    }
    0
}

/// Set the current thread's mmap bump pointer, syncing across sibling threads.
pub fn set_current_thread_mmap_next(val: u32) {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_SET_THREAD_MMAP);
    let mut guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_mut() {
        if let Some(idx) = sched.current_idx(cpu_id) {
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

// =============================================================================
// Thread lifecycle
// =============================================================================

/// Terminate the current thread with an exit code. Wakes any waitpid waiter.
pub fn exit_current(code: u32) {
    let my_cpu = get_cpu_id();
    let tid;
    let mut pd_to_destroy: Option<PhysAddr> = None;
    let parent_tid_for_sigchld: u32;
    crate::sched_diag::set(my_cpu, crate::sched_diag::PHASE_EXIT_CURRENT);
    let mut guard = SCHEDULER.lock();
    {
        let cpu_id = get_cpu_id();
        let sched = guard.as_mut().expect("Scheduler not initialized");
        tid = sched.per_cpu[cpu_id].current_tid.unwrap_or(0);
        if let Some(current_tid) = sched.per_cpu[cpu_id].current_tid {
            if let Some(idx) = sched.current_idx(cpu_id) {
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
    // Defer page directory destruction to CPU 0's timer tick (drains DEFERRED_PD_DESTROY
    // before acquiring the SCHEDULER lock). This avoids competing for the ALLOCATOR lock
    // (2600+ free_frame calls) with other exiting CPUs while SCHEDULER lock is contested.
    // tid=0: cleanup_process already ran in sys_exit before exit_current was called.
    if let Some(pd) = pd_to_destroy {
        let kernel_cr3 = crate::memory::virtual_mem::kernel_cr3();
        unsafe { core::arch::asm!("mov cr3, {}", in(reg) kernel_cr3); }
        DEFERRED_PD_DESTROY.lock().push(pd, 0);
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
    let my_cpu = get_cpu_id();
    let tid;
    let mut pd_to_destroy: Option<PhysAddr> = None;
    crate::sched_diag::set(my_cpu, crate::sched_diag::PHASE_TRY_EXIT_CURRENT);
    let mut guard = match SCHEDULER.try_lock() {
        Some(g) => g,
        None => return false,
    };
    {
        let cpu_id = get_cpu_id();
        let sched = match guard.as_mut() { Some(s) => s, None => return false };
        if let Some(current_tid) = sched.per_cpu[cpu_id].current_tid {
            tid = current_tid;
            if let Some(idx) = sched.current_idx(cpu_id) {
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
    // Defer PD destruction — same reasoning as exit_current.
    // tid=0: cleanup_process already ran before try_exit_current was called.
    if let Some(pd) = pd_to_destroy {
        let kernel_cr3 = crate::memory::virtual_mem::kernel_cr3();
        unsafe { core::arch::asm!("mov cr3, {}", in(reg) kernel_cr3); }
        DEFERRED_PD_DESTROY.lock().push(pd, 0);
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
                    sched.per_cpu[cpu_id].current_idx = None;
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
    if crate::memory::physical::is_allocator_locked_by_cpu(cpu) {
        unsafe { crate::memory::physical::force_unlock_allocator(); }
        crate::serial_println!("  RECOVERED: force-released physical allocator lock");
    }
    if crate::task::dll::is_dll_locked_by_cpu(cpu) {
        unsafe { crate::task::dll::force_unlock_dlls(); }
        crate::serial_println!("  RECOVERED: force-released LOADED_DLLS lock");
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
                sched.per_cpu[cpu_id].current_idx = None;
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

    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_KILL_THREAD);
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

    // Resource cleanup for killed thread (FDs, shared memory, TCP, env).
    // Must happen AFTER scheduler lock is released (avoids lock-ordering deadlock).
    {
        use crate::fs::fd_table::FdKind;
        let closed = close_all_fds_for_thread(tid);
        for kind in closed.iter() {
            match kind {
                FdKind::File { global_id } => {
                    crate::fs::vfs::decref(*global_id);
                }
                FdKind::PipeRead { pipe_id } => {
                    crate::ipc::anon_pipe::decref_read(*pipe_id);
                }
                FdKind::PipeWrite { pipe_id } => {
                    crate::ipc::anon_pipe::decref_write(*pipe_id);
                }
                FdKind::Tty | FdKind::None => {}
            }
        }
    }
    // cleanup_process requires the dying process's page directory to be the
    // active CR3, so that unmap_page() removes SHM PTEs from the correct
    // address space. Failing to do this leaves SHM frames mapped in the dying
    // process's page tables; destroy_user_page_directory then frees those
    // physical frames even though the compositor still has them mapped — causing
    // the compositor to crash when those frames are reused (e.g. as page tables).
    if let Some(pd) = pd_to_destroy {
        if is_current {
            // Current thread: our CR3 is already the dying process's CR3.
            crate::ipc::shared_memory::cleanup_process(tid);
        } else if !running_on_other_cpu {
            // Killing a thread that is not running right now. Temporarily switch
            // to the dying process's CR3 (same pattern as destroy_user_page_directory).
            unsafe {
                let rflags: u64;
                core::arch::asm!("pushfq; pop {}", out(reg) rflags, options(nomem));
                core::arch::asm!("cli");
                let old_cr3 = crate::memory::virtual_mem::current_cr3();
                core::arch::asm!("mov cr3, {}", in(reg) pd.as_u64());
                crate::ipc::shared_memory::cleanup_process(tid);
                core::arch::asm!("mov cr3, {}", in(reg) old_cr3);
                core::arch::asm!("push {}; popfq", in(reg) rflags, options(nomem));
            }
        }
        // running_on_other_cpu: skip cleanup_process here — the thread is still
        // running on another CPU. The deferred drain will call cleanup_process
        // with the correct CR3 after that CPU has context-switched away.
    }
    crate::net::tcp::cleanup_for_thread(tid);
    if let Some(pd) = pd_to_destroy {
        crate::task::env::cleanup(pd.as_u64());
    }

    if let Some(pd) = pd_to_destroy {
        if running_on_other_cpu {
            // Thread is still executing on another CPU. cleanup_process cannot
            // run yet (we'd unmap pages from the wrong address space). Defer
            // everything — the deferred drain will do cleanup + destroy after
            // that CPU has context-switched away. tid != 0 signals this.
            DEFERRED_PD_DESTROY.lock().push(pd, tid);
        } else {
            // cleanup_process already ran above (correct CR3 was used).
            // Switch off the dying PD immediately for the is_current case so
            // the kernel is no longer running on a soon-to-be-freed CR3.
            if is_current {
                let kernel_cr3 = crate::memory::virtual_mem::kernel_cr3();
                unsafe { core::arch::asm!("mov cr3, {}", in(reg) kernel_cr3); }
            }
            // Defer destroy_user_page_directory so it runs OUTSIDE the
            // scheduler lock (which we still hold here). The deferred drain
            // runs before the next scheduler lock acquisition. tid=0 tells the
            // drain that cleanup_process already ran — no need to repeat it.
            DEFERRED_PD_DESTROY.lock().push(pd, 0);
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
        crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_WAITPID);
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
            if let Some(idx) = sched.current_idx(cpu_id) {
                // CRITICAL: Mark context as unsaved BEFORE setting Blocked.
                // Without this, another CPU can wake this thread (→ Ready)
                // and load its stale saved context while we're still
                // physically executing on its stack → two CPUs on same stack → crash.
                sched.threads[idx].context.save_complete = 0;
                sched.threads[idx].state = ThreadState::Blocked;
            }
        }
    }
    // Yield immediately instead of waiting up to 1ms for timer preemption.
    schedule();
    loop {
        unsafe { core::arch::asm!("sti; hlt"); }
        {
            crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_WAITPID);
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
        crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_WAITPID_ANY);
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
        if let Some(idx) = sched.current_idx(get_cpu_id()) {
            sched.threads[idx].context.save_complete = 0;
            sched.threads[idx].state = ThreadState::Blocked;
        }
    }
    // Yield immediately instead of waiting up to 1ms for timer preemption.
    schedule();
    loop {
        unsafe { core::arch::asm!("sti; hlt"); }
        {
            crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_WAITPID_ANY);
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
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_TRY_WAITPID_ANY);
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
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_TRY_WAITPID);
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
        crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_SLEEP_UNTIL);
        let mut guard = SCHEDULER.lock();
        let cpu_id = get_cpu_id();
        let sched = guard.as_mut().expect("Scheduler not initialized");
        if let Some(idx) = sched.current_idx(cpu_id) {
            // CRITICAL: Mark context as unsaved before Blocked (same race as waitpid).
            sched.threads[idx].context.save_complete = 0;
            sched.threads[idx].wake_at_tick = Some(wake_at);
            sched.threads[idx].state = ThreadState::Blocked;
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
        let sched = guard.as_mut().expect("Scheduler not initialized");
        if let Some(idx) = sched.current_idx(cpu_id) {
            // CRITICAL: Mark context as unsaved before Blocked (same race as waitpid).
            sched.threads[idx].context.save_complete = 0;
            sched.threads[idx].state = ThreadState::Blocked;
        }
    }
    schedule();
}

// =============================================================================
// Thread args / stdout / stdin
// =============================================================================

pub fn set_thread_args(tid: u32, args: &str) {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_SET_THREAD_ARGS);
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
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_GET_THREAD_INFO);
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current_idx(cpu_id) {
            let args = &sched.threads[idx].args;
            let len = args.iter().position(|&b| b == 0).unwrap_or(256);
            let copy_len = len.min(buf.len());
            buf[..copy_len].copy_from_slice(&args[..copy_len]);
            return copy_len;
        }
    }
    0
}

/// Set the current working directory for a thread.
pub fn set_thread_cwd(tid: u32, cwd: &str) {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_SET_THREAD_CWD);
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        let bytes = cwd.as_bytes();
        let len = bytes.len().min(511);
        thread.cwd[..len].copy_from_slice(&bytes[..len]);
        thread.cwd[len] = 0;
    }
}

/// Get the current working directory for the running thread.
pub fn current_thread_cwd(buf: &mut [u8]) -> usize {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_GET_THREAD_INFO);
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current_idx(cpu_id) {
            let cwd = &sched.threads[idx].cwd;
            let len = cwd.iter().position(|&b| b == 0).unwrap_or(512);
            let copy_len = len.min(buf.len());
            buf[..copy_len].copy_from_slice(&cwd[..copy_len]);
            return copy_len;
        }
    }
    0
}

pub fn set_thread_stdout_pipe(tid: u32, pipe_id: u32) {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_SET_THREAD_PIPE);
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.stdout_pipe = pipe_id;
    }
}

pub fn current_thread_stdout_pipe() -> u32 {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_GET_THREAD_INFO);
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current_idx(cpu_id) {
            return sched.threads[idx].stdout_pipe;
        }
    }
    0
}

pub fn set_thread_stdin_pipe(tid: u32, pipe_id: u32) {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_SET_THREAD_PIPE);
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.stdin_pipe = pipe_id;
    }
}

pub fn current_thread_stdin_pipe() -> u32 {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_GET_THREAD_INFO);
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current_idx(cpu_id) {
            return sched.threads[idx].stdin_pipe;
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
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_SET_THREAD_PRIORITY);
    let mut guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_mut() {
        if let Some(idx) = sched.find_idx(tid) {
            sched.threads[idx].priority = priority;
        }
    }
}

/// Wake a blocked thread by TID.
pub fn wake_thread(tid: u32) {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_WAKE_THREAD);
    let mut guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_mut() {
        sched.wake_thread_inner(tid);
    }
}

/// Try to wake a blocked thread by TID (non-blocking).
///
/// Uses `try_lock()` to avoid spinning on the SCHEDULER lock. Returns `true`
/// if the thread was woken, `false` if the lock was contended (caller should
/// retry later or use the deferred-wake mechanism).
///
/// Safe to call from IRQ context — never blocks.
pub fn try_wake_thread(tid: u32) -> bool {
    if let Some(mut guard) = SCHEDULER.try_lock() {
        if let Some(sched) = guard.as_mut() {
            sched.wake_thread_inner(tid);
        }
        true
    } else {
        false
    }
}

/// Mark a thread as critical (will not be killed by RSP recovery).
pub fn set_thread_critical(tid: u32) {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_GET_THREAD_INFO);
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.critical = true;
        crate::serial_println!("  Thread '{}' (TID={}) marked as critical", thread.name_str(), tid);
    }
}

/// Get the capability bitmask for the currently running thread.
pub fn current_thread_capabilities() -> crate::task::capabilities::CapSet {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_GET_THREAD_INFO);
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    let sched = guard.as_ref().expect("Scheduler not initialized");
    if let Some(idx) = sched.current_idx(cpu_id) {
        return sched.threads[idx].capabilities;
    }
    0
}

/// Set the capability bitmask for a thread (called by loader after spawn).
pub fn set_thread_capabilities(tid: u32, caps: crate::task::capabilities::CapSet) {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_GET_THREAD_INFO);
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.capabilities = caps;
    }
}

/// Get the user ID of the currently running thread.
pub fn current_thread_uid() -> u16 {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_GET_THREAD_INFO);
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    let sched = guard.as_ref().expect("Scheduler not initialized");
    if let Some(idx) = sched.current_idx(cpu_id) {
        return sched.threads[idx].uid;
    }
    0
}

/// Get the group ID of the currently running thread.
pub fn current_thread_gid() -> u16 {
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    let sched = guard.as_ref().expect("Scheduler not initialized");
    if let Some(idx) = sched.current_idx(cpu_id) {
        return sched.threads[idx].gid;
    }
    0
}

// =============================================================================
// Pending permission info — static array (NOT in Thread struct).
// =============================================================================
// Follows the same pattern as PENDING_PROGRAMS in loader.rs to avoid enlarging
// the Thread struct (which changes heap layout and can trigger latent bugs).

const MAX_PENDING_PERM: usize = 16;

struct PendingPermSlot {
    tid: u32,
    data: [u8; 512],
    len: u16,
    used: bool,
}

impl PendingPermSlot {
    const fn empty() -> Self {
        PendingPermSlot { tid: 0, data: [0u8; 512], len: 0, used: false }
    }
}

static PENDING_PERM_INFO: Spinlock<[PendingPermSlot; MAX_PENDING_PERM]> = Spinlock::new([
    PendingPermSlot::empty(), PendingPermSlot::empty(),
    PendingPermSlot::empty(), PendingPermSlot::empty(),
    PendingPermSlot::empty(), PendingPermSlot::empty(),
    PendingPermSlot::empty(), PendingPermSlot::empty(),
    PendingPermSlot::empty(), PendingPermSlot::empty(),
    PendingPermSlot::empty(), PendingPermSlot::empty(),
    PendingPermSlot::empty(), PendingPermSlot::empty(),
    PendingPermSlot::empty(), PendingPermSlot::empty(),
]);

/// Store pending permission info for the current thread.
/// Data is a UTF-8 byte slice: "app_id\x1Fapp_name\x1Fcaps_hex\x1Fbundle_path".
pub fn set_current_perm_pending(data: &[u8]) {
    let tid = current_tid();
    if tid == 0 { return; }
    let mut slots = PENDING_PERM_INFO.lock();
    // Overwrite existing slot for this TID, or allocate a new one
    let idx = slots.iter().position(|s| s.used && s.tid == tid)
        .or_else(|| slots.iter().position(|s| !s.used));
    if let Some(i) = idx {
        let len = data.len().min(512);
        slots[i].data[..len].copy_from_slice(&data[..len]);
        slots[i].len = len as u16;
        slots[i].tid = tid;
        slots[i].used = true;
    }
}

/// Read pending permission info for the current thread into `buf`.
/// Consumes (clears) the slot after reading.
/// Returns the number of bytes copied (0 if none).
pub fn current_perm_pending(buf: &mut [u8]) -> usize {
    let tid = current_tid();
    if tid == 0 { return 0; }
    let mut slots = PENDING_PERM_INFO.lock();
    if let Some(slot) = slots.iter_mut().find(|s| s.used && s.tid == tid) {
        let len = slot.len as usize;
        if len > 0 {
            let copy = len.min(buf.len());
            buf[..copy].copy_from_slice(&slot.data[..copy]);
            // Consume the slot so it can be reused
            slot.used = false;
            return copy;
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
    pub cwd: [u8; 512],
    pub capabilities: crate::task::capabilities::CapSet,
    pub uid: u16,
    pub gid: u16,
    pub stdout_pipe: u32,
    pub stdin_pipe: u32,
    pub fpu_data: [u8; crate::task::thread::FPU_STATE_SIZE],
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
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_GET_THREAD_INFO);
    let guard = SCHEDULER.lock();
    let sched = guard.as_ref()?;
    let cpu = get_cpu_id();
    let idx = sched.current_idx(cpu)?;
    let thread = &sched.threads[idx];
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
pub fn set_thread_fpu_state(tid: u32, data: &[u8; crate::task::thread::FPU_STATE_SIZE]) {
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
        thread.pcid = crate::memory::virtual_mem::allocate_pcid();
        thread.context.cr3 = new_pd.as_u64() | thread.pcid as u64;
        thread.brk = brk;
        // ASLR: randomize the mmap base within [0x20000000, 0x20000000 + 16 MiB)
        let mmap_rand = crate::task::loader::random_page_offset(
            crate::task::loader::ASLR_MMAP_MAX_PAGES,
        );
        thread.mmap_next = 0x7000_0000u32.wrapping_add(mmap_rand * 4096);
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
        if let Some(idx) = sched.current_idx(cpu_id) {
            sched.threads[idx].io_read_bytes += bytes;
        }
    }
}

pub fn record_io_write(bytes: u64) {
    let mut guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_mut() {
        if let Some(idx) = sched.current_idx(cpu_id) {
            sched.threads[idx].io_write_bytes += bytes;
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
        if let Some(idx) = sched.current_idx(cpu_id) {
            if delta >= 0 {
                sched.threads[idx].user_pages = sched.threads[idx].user_pages.saturating_add(delta as u32);
            } else {
                sched.threads[idx].user_pages = sched.threads[idx].user_pages.saturating_sub((-delta) as u32);
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
            sched.per_cpu[cpu_id].current_idx = Some(idx);
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
    let idx = sched.current_idx(cpu)?;
    sched.threads[idx].fd_table.alloc(kind)
}

/// Close an FD in the current thread's FD table.
/// Returns the old FdKind for cleanup (decref, etc.), or None if invalid.
pub fn current_fd_close(fd: u32) -> Option<FdKind> {
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut()?;
    let cpu = get_cpu_id();
    let idx = sched.current_idx(cpu)?;
    sched.threads[idx].fd_table.close(fd)
}

/// Look up an FD in the current thread's FD table.
pub fn current_fd_get(fd: u32) -> Option<FdEntry> {
    let guard = SCHEDULER.lock();
    let sched = guard.as_ref()?;
    let cpu = get_cpu_id();
    let idx = sched.current_idx(cpu)?;
    sched.threads[idx].fd_table.get(fd).copied()
}

/// Duplicate old_fd to new_fd in the current thread's FD table.
/// Caller must handle closing new_fd first and incrementing refcounts.
pub fn current_fd_dup2(old_fd: u32, new_fd: u32) -> bool {
    let mut guard = SCHEDULER.lock();
    let sched = match guard.as_mut() { Some(s) => s, None => return false };
    let cpu = get_cpu_id();
    let idx = match sched.current_idx(cpu) { Some(i) => i, None => return false };
    sched.threads[idx].fd_table.dup2(old_fd, new_fd)
}

/// Allocate the lowest FD >= min_fd in the current thread's FD table.
pub fn current_fd_alloc_above(min_fd: u32, kind: FdKind) -> Option<u32> {
    let mut guard = SCHEDULER.lock();
    let sched = match guard.as_mut() { Some(s) => s, None => return None };
    let cpu = get_cpu_id();
    let idx = match sched.current_idx(cpu) { Some(i) => i, None => return None };
    sched.threads[idx].fd_table.alloc_above(min_fd, kind)
}

/// Allocate an FD at a specific slot in the current thread's FD table.
pub fn current_fd_alloc_at(fd: u32, kind: FdKind) -> bool {
    let mut guard = SCHEDULER.lock();
    let sched = match guard.as_mut() { Some(s) => s, None => return false };
    let cpu = get_cpu_id();
    let idx = match sched.current_idx(cpu) { Some(i) => i, None => return false };
    sched.threads[idx].fd_table.alloc_at(fd, kind)
}

/// Set or clear the CLOEXEC flag on an FD in the current thread's FD table.
pub fn current_fd_set_cloexec(fd: u32, cloexec: bool) {
    let mut guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_mut() {
        let cpu = get_cpu_id();
        if let Some(idx) = sched.current_idx(cpu) {
            sched.threads[idx].fd_table.set_cloexec(fd, cloexec);
        }
    }
}

/// Set or clear O_NONBLOCK on an FD in the current thread's FD table.
pub fn current_fd_set_nonblock(fd: u32, nonblock: bool) {
    let mut guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_mut() {
        let cpu = get_cpu_id();
        if let Some(idx) = sched.current_idx(cpu) {
            sched.threads[idx].fd_table.set_nonblock(fd, nonblock);
        }
    }
}

/// Set the FD table on a thread (for fork child setup).
pub fn set_thread_fd_table(tid: u32, table: FdTable) {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_GET_THREAD_INFO);
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
        if let Some(idx) = sched.current_idx(cpu) {
            sched.threads[idx].fd_table.close_all(&mut out);
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
        if let Some(idx) = sched.current_idx(cpu) {
            sched.threads[idx].fd_table.close_cloexec(&mut out);
        }
    }
    out
}

/// Close all FDs for a specific thread (by TID). Returns old FdKinds for cleanup.
/// Used during sys_exit before destroying the page directory.
pub fn close_all_fds_for_thread(tid: u32) -> [FdKind; MAX_FDS] {
    let mut out = [FdKind::None; MAX_FDS];
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_GET_THREAD_INFO);
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

/// Collect TIDs of all live non-idle threads (for system shutdown).
///
/// Returns a Vec of TIDs for threads that are not idle and not yet terminated.
pub fn all_live_tids() -> alloc::vec::Vec<u32> {
    let guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_ref() {
        sched.threads.iter()
            .filter(|t| !t.is_idle && t.state != ThreadState::Terminated)
            .map(|t| t.tid)
            .collect()
    } else {
        alloc::vec::Vec::new()
    }
}

// =============================================================================
// Debug / Trace API (anyTrace)
// =============================================================================

/// Debug event types — must match userspace constants.
pub const DEBUG_EVENT_BREAKPOINT: u32 = 1;
pub const DEBUG_EVENT_SINGLE_STEP: u32 = 2;
pub const DEBUG_EVENT_EXIT: u32 = 3;

/// Attach `debugger_tid` to `target_tid`. The target is suspended (Blocked).
///
/// Rejects: self-attach, kernel/idle threads, already-attached threads.
/// Returns 0 on success, u32::MAX on error.
pub fn debug_attach(debugger_tid: u32, target_tid: u32) -> u32 {
    let mut guard = SCHEDULER.lock();
    let sched = match guard.as_mut() {
        Some(s) => s,
        None => return u32::MAX,
    };

    let idx = match sched.threads.iter().position(|t| t.tid == target_tid) {
        Some(i) => i,
        None => return u32::MAX,
    };

    let thread = &sched.threads[idx];

    // Reject kernel threads, idle threads, terminated threads, already-attached
    if !thread.is_user || thread.is_idle || thread.state == ThreadState::Terminated {
        return u32::MAX;
    }
    if thread.debug_attached_by != 0 {
        return u32::MAX; // Already attached by another debugger
    }

    // Set debug attachment
    sched.threads[idx].debug_attached_by = debugger_tid;
    sched.threads[idx].debug_suspended = true;

    // If thread is Ready, remove from run queue and set Blocked
    if sched.threads[idx].state == ThreadState::Ready {
        // Remove from the run queue on its affinity CPU
        let cpu = sched.threads[idx].affinity_cpu;
        let n = sched.num_cpus();
        let target_cpu = if cpu < n { cpu } else { 0 };
        sched.per_cpu[target_cpu].run_queue.remove(target_tid);
        sched.threads[idx].state = ThreadState::Blocked;
    } else if sched.threads[idx].state == ThreadState::Running {
        // Thread is currently running on some CPU — mark for suspend.
        // It will be blocked on next schedule tick.
        sched.threads[idx].state = ThreadState::Blocked;
        sched.threads[idx].context.save_complete = 0;
    }
    // If already Blocked (e.g., sleeping), just keep it blocked

    0
}

/// Detach `debugger_tid` from `target_tid`. Removes all breakpoints,
/// clears TF, and resumes the thread.
///
/// Returns 0 on success, u32::MAX on error.
pub fn debug_detach(debugger_tid: u32, target_tid: u32) -> u32 {
    // First, collect breakpoint info under lock, then do CR3-switch outside lock
    let (bp_count, breakpoints, target_cr3);
    {
        let mut guard = SCHEDULER.lock();
        let sched = match guard.as_mut() {
            Some(s) => s,
            None => return u32::MAX,
        };

        let idx = match sched.threads.iter().position(|t| t.tid == target_tid) {
            Some(i) => i,
            None => return u32::MAX,
        };

        if sched.threads[idx].debug_attached_by != debugger_tid {
            return u32::MAX;
        }

        // Save breakpoint info for removal
        bp_count = sched.threads[idx].debug_sw_bp_count;
        breakpoints = sched.threads[idx].debug_sw_breakpoints;
        target_cr3 = sched.threads[idx].context.cr3;

        // Clear TF from rflags
        sched.threads[idx].context.rflags &= !0x100;
        // Recompute checksum after modifying rflags
        sched.threads[idx].context.checksum = sched.threads[idx].context.compute_checksum();

        // Clear all debug state
        sched.threads[idx].debug_attached_by = 0;
        sched.threads[idx].debug_suspended = false;
        sched.threads[idx].debug_single_step = false;
        sched.threads[idx].debug_event = None;
        sched.threads[idx].debug_sw_bp_count = 0;
        sched.threads[idx].debug_sw_breakpoints = [(0, 0); 16];

        // Resume thread if it's blocked
        if sched.threads[idx].state == ThreadState::Blocked {
            sched.threads[idx].state = ThreadState::Ready;
            let cpu = sched.threads[idx].affinity_cpu;
            let n = sched.num_cpus();
            let target_cpu = if cpu < n { cpu } else { 0 };
            sched.per_cpu[target_cpu].run_queue.enqueue(target_tid, sched.threads[idx].priority);
        }
    }

    // Restore original bytes at breakpoint locations (outside scheduler lock)
    if bp_count > 0 && target_cr3 != 0 {
        restore_breakpoint_bytes(target_cr3, &breakpoints[..bp_count as usize]);
    }

    0
}

/// Suspend a debug-attached thread.
///
/// Returns 0 on success, u32::MAX on error.
pub fn debug_suspend(debugger_tid: u32, target_tid: u32) -> u32 {
    let mut guard = SCHEDULER.lock();
    let sched = match guard.as_mut() {
        Some(s) => s,
        None => return u32::MAX,
    };

    let idx = match sched.threads.iter().position(|t| t.tid == target_tid) {
        Some(i) => i,
        None => return u32::MAX,
    };

    if sched.threads[idx].debug_attached_by != debugger_tid {
        return u32::MAX;
    }

    sched.threads[idx].debug_suspended = true;

    if sched.threads[idx].state == ThreadState::Ready {
        let cpu = sched.threads[idx].affinity_cpu;
        let n = sched.num_cpus();
        let target_cpu = if cpu < n { cpu } else { 0 };
        sched.per_cpu[target_cpu].run_queue.remove(target_tid);
        sched.threads[idx].state = ThreadState::Blocked;
    } else if sched.threads[idx].state == ThreadState::Running {
        sched.threads[idx].state = ThreadState::Blocked;
        sched.threads[idx].context.save_complete = 0;
    }

    0
}

/// Resume a suspended debug-attached thread.
/// If single_step is pending, sets RFLAGS.TF before resuming.
///
/// Returns 0 on success, u32::MAX on error.
pub fn debug_resume(debugger_tid: u32, target_tid: u32) -> u32 {
    let mut guard = SCHEDULER.lock();
    let sched = match guard.as_mut() {
        Some(s) => s,
        None => return u32::MAX,
    };

    let idx = match sched.threads.iter().position(|t| t.tid == target_tid) {
        Some(i) => i,
        None => return u32::MAX,
    };

    if sched.threads[idx].debug_attached_by != debugger_tid {
        return u32::MAX;
    }
    if !sched.threads[idx].debug_suspended {
        return u32::MAX; // Not suspended
    }

    // If single-step is pending, set TF in RFLAGS
    if sched.threads[idx].debug_single_step {
        sched.threads[idx].context.rflags |= 0x100; // TF bit
        sched.threads[idx].context.checksum = sched.threads[idx].context.compute_checksum();
    }

    sched.threads[idx].debug_suspended = false;

    // Wake thread if it's blocked due to debug suspension
    if sched.threads[idx].state == ThreadState::Blocked {
        sched.threads[idx].state = ThreadState::Ready;
        let cpu = sched.threads[idx].affinity_cpu;
        let n = sched.num_cpus();
        let target_cpu = if cpu < n { cpu } else { 0 };
        sched.per_cpu[target_cpu].run_queue.enqueue(target_tid, sched.threads[idx].priority);
    }

    0
}

/// Read the target thread's CpuContext into a user buffer.
///
/// Returns number of bytes copied, or u32::MAX on error.
pub fn debug_get_regs(debugger_tid: u32, target_tid: u32, buf_ptr: u64, size: u32) -> u32 {
    let guard = SCHEDULER.lock();
    let sched = match guard.as_ref() {
        Some(s) => s,
        None => return u32::MAX,
    };

    let idx = match sched.threads.iter().position(|t| t.tid == target_tid) {
        Some(i) => i,
        None => return u32::MAX,
    };

    if sched.threads[idx].debug_attached_by != debugger_tid {
        return u32::MAX;
    }
    if !sched.threads[idx].debug_suspended {
        return u32::MAX; // Must be suspended to read registers
    }

    // Copy first 160 bytes of CpuContext (20 u64 fields: 16 GPRs + RSP + RIP + RFLAGS + CR3)
    let ctx = &sched.threads[idx].context;
    let ctx_ptr = ctx as *const CpuContext as *const u8;
    let copy_len = (size as usize).min(160); // 20 * 8 = 160 bytes

    unsafe {
        let dst = buf_ptr as *mut u8;
        core::ptr::copy_nonoverlapping(ctx_ptr, dst, copy_len);
    }

    copy_len as u32
}

/// Overwrite the target thread's CpuContext from a user buffer.
///
/// Validates that RIP is in user-space and masks dangerous RFLAGS bits.
/// Returns 0 on success, u32::MAX on error.
pub fn debug_set_regs(debugger_tid: u32, target_tid: u32, buf_ptr: u64, size: u32) -> u32 {
    let mut guard = SCHEDULER.lock();
    let sched = match guard.as_mut() {
        Some(s) => s,
        None => return u32::MAX,
    };

    let idx = match sched.threads.iter().position(|t| t.tid == target_tid) {
        Some(i) => i,
        None => return u32::MAX,
    };

    if sched.threads[idx].debug_attached_by != debugger_tid {
        return u32::MAX;
    }
    if !sched.threads[idx].debug_suspended {
        return u32::MAX;
    }

    let copy_len = (size as usize).min(160);

    // Read new values from user buffer into a temporary context
    let mut new_ctx = sched.threads[idx].context;
    unsafe {
        let src = buf_ptr as *const u8;
        let dst = &mut new_ctx as *mut CpuContext as *mut u8;
        core::ptr::copy_nonoverlapping(src, dst, copy_len);
    }

    // Validate: RIP must be in user-space (below kernel half)
    if new_ctx.rip >= 0x0000_8000_0000_0000 {
        return u32::MAX;
    }
    // RSP must be in user-space
    if new_ctx.rsp >= 0x0000_8000_0000_0000 {
        return u32::MAX;
    }

    // Mask dangerous RFLAGS bits — preserve only safe user flags
    // Keep: CF(0), PF(2), AF(4), ZF(6), SF(7), TF(8, for single-step), DF(10), OF(11), IF(9)
    // IOPL must stay 0 (ring-3), VM must stay 0, VIF/VIP must stay 0
    const SAFE_FLAGS: u64 = 0xCD5; // CF|PF|AF|ZF|SF|DF|OF
    const IF_FLAG: u64 = 0x200;
    new_ctx.rflags = (new_ctx.rflags & SAFE_FLAGS) | IF_FLAG; // Always keep IF=1 for user

    // Preserve CR3 — debugger cannot change address space
    new_ctx.cr3 = sched.threads[idx].context.cr3;

    // Recompute integrity fields
    new_ctx.canary = crate::task::context::CANARY_MAGIC;
    new_ctx.save_complete = 1;
    new_ctx.checksum = new_ctx.compute_checksum();

    sched.threads[idx].context = new_ctx;

    0
}

/// Read memory from the target thread's address space using CR3-switch.
///
/// Returns number of bytes read, or u32::MAX on error.
pub fn debug_read_mem(debugger_tid: u32, target_tid: u32, addr: u64, buf_ptr: u64, size: u32) -> u32 {
    let target_cr3;
    {
        let guard = SCHEDULER.lock();
        let sched = match guard.as_ref() {
            Some(s) => s,
            None => return u32::MAX,
        };

        let idx = match sched.threads.iter().position(|t| t.tid == target_tid) {
            Some(i) => i,
            None => return u32::MAX,
        };

        if sched.threads[idx].debug_attached_by != debugger_tid {
            return u32::MAX;
        }

        target_cr3 = sched.threads[idx].context.cr3;
    }

    if target_cr3 == 0 {
        return u32::MAX;
    }

    // Perform CR3-switch read (outside scheduler lock to avoid contention)
    let bytes_read = cr3_switch_read(target_cr3, addr, buf_ptr, size);
    bytes_read
}

/// Write memory into the target thread's address space using CR3-switch.
///
/// Returns number of bytes written, or u32::MAX on error.
pub fn debug_write_mem(debugger_tid: u32, target_tid: u32, addr: u64, buf_ptr: u64, size: u32) -> u32 {
    let target_cr3;
    {
        let guard = SCHEDULER.lock();
        let sched = match guard.as_ref() {
            Some(s) => s,
            None => return u32::MAX,
        };

        let idx = match sched.threads.iter().position(|t| t.tid == target_tid) {
            Some(i) => i,
            None => return u32::MAX,
        };

        if sched.threads[idx].debug_attached_by != debugger_tid {
            return u32::MAX;
        }

        target_cr3 = sched.threads[idx].context.cr3;
    }

    if target_cr3 == 0 {
        return u32::MAX;
    }

    let bytes_written = cr3_switch_write(target_cr3, addr, buf_ptr, size);
    bytes_written
}

/// Set a software breakpoint (INT3) at `addr` in the target's address space.
///
/// Returns 0 on success, u32::MAX on error.
pub fn debug_set_breakpoint(debugger_tid: u32, target_tid: u32, addr: u64) -> u32 {
    let target_cr3;
    {
        let mut guard = SCHEDULER.lock();
        let sched = match guard.as_mut() {
            Some(s) => s,
            None => return u32::MAX,
        };

        let idx = match sched.threads.iter().position(|t| t.tid == target_tid) {
            Some(i) => i,
            None => return u32::MAX,
        };

        if sched.threads[idx].debug_attached_by != debugger_tid {
            return u32::MAX;
        }
        if !sched.threads[idx].debug_suspended {
            return u32::MAX;
        }

        // Check if breakpoint already exists
        for i in 0..sched.threads[idx].debug_sw_bp_count as usize {
            if sched.threads[idx].debug_sw_breakpoints[i].0 == addr {
                return 0; // Already set
            }
        }

        // Check capacity
        if sched.threads[idx].debug_sw_bp_count >= 16 {
            return u32::MAX;
        }

        target_cr3 = sched.threads[idx].context.cr3;
    }

    if target_cr3 == 0 {
        return u32::MAX;
    }

    // Read original byte via CR3-switch
    let mut original_byte: u8 = 0;
    let read = cr3_switch_read(target_cr3, addr, &mut original_byte as *mut u8 as u64, 1);
    if read != 1 {
        return u32::MAX;
    }

    // Write INT3 (0xCC) via CR3-switch
    let int3: u8 = 0xCC;
    let written = cr3_switch_write(target_cr3, addr, &int3 as *const u8 as u64, 1);
    if written != 1 {
        return u32::MAX;
    }

    // Record breakpoint in thread struct
    {
        let mut guard = SCHEDULER.lock();
        let sched = match guard.as_mut() {
            Some(s) => s,
            None => return u32::MAX,
        };
        let idx = match sched.threads.iter().position(|t| t.tid == target_tid) {
            Some(i) => i,
            None => return u32::MAX,
        };
        let bp_count = sched.threads[idx].debug_sw_bp_count as usize;
        sched.threads[idx].debug_sw_breakpoints[bp_count] = (addr, original_byte);
        sched.threads[idx].debug_sw_bp_count += 1;
    }

    0
}

/// Clear a software breakpoint, restoring the original byte.
///
/// Returns 0 on success, u32::MAX on error.
pub fn debug_clr_breakpoint(debugger_tid: u32, target_tid: u32, addr: u64) -> u32 {
    let target_cr3;
    let original_byte;
    {
        let mut guard = SCHEDULER.lock();
        let sched = match guard.as_mut() {
            Some(s) => s,
            None => return u32::MAX,
        };

        let idx = match sched.threads.iter().position(|t| t.tid == target_tid) {
            Some(i) => i,
            None => return u32::MAX,
        };

        if sched.threads[idx].debug_attached_by != debugger_tid {
            return u32::MAX;
        }

        // Find the breakpoint
        let bp_count = sched.threads[idx].debug_sw_bp_count as usize;
        let bp_pos = (0..bp_count)
            .find(|&i| sched.threads[idx].debug_sw_breakpoints[i].0 == addr);

        let bp_pos = match bp_pos {
            Some(p) => p,
            None => return u32::MAX, // Breakpoint not found
        };

        original_byte = sched.threads[idx].debug_sw_breakpoints[bp_pos].1;
        target_cr3 = sched.threads[idx].context.cr3;

        // Remove from array by shifting
        for i in bp_pos..bp_count - 1 {
            sched.threads[idx].debug_sw_breakpoints[i] =
                sched.threads[idx].debug_sw_breakpoints[i + 1];
        }
        sched.threads[idx].debug_sw_breakpoints[bp_count - 1] = (0, 0);
        sched.threads[idx].debug_sw_bp_count -= 1;
    }

    // Restore original byte
    if target_cr3 != 0 {
        cr3_switch_write(target_cr3, addr, &original_byte as *const u8 as u64, 1);
    }

    0
}

/// Mark the target for single-step execution.
///
/// Returns 0 on success, u32::MAX on error.
pub fn debug_single_step(debugger_tid: u32, target_tid: u32) -> u32 {
    let mut guard = SCHEDULER.lock();
    let sched = match guard.as_mut() {
        Some(s) => s,
        None => return u32::MAX,
    };

    let idx = match sched.threads.iter().position(|t| t.tid == target_tid) {
        Some(i) => i,
        None => return u32::MAX,
    };

    if sched.threads[idx].debug_attached_by != debugger_tid {
        return u32::MAX;
    }
    if !sched.threads[idx].debug_suspended {
        return u32::MAX;
    }

    sched.threads[idx].debug_single_step = true;

    // Set TF in saved RFLAGS so it takes effect on resume
    sched.threads[idx].context.rflags |= 0x100;
    sched.threads[idx].context.checksum = sched.threads[idx].context.compute_checksum();

    // Resume the thread so it executes one instruction
    sched.threads[idx].debug_suspended = false;
    if sched.threads[idx].state == ThreadState::Blocked {
        sched.threads[idx].state = ThreadState::Ready;
        let cpu = sched.threads[idx].affinity_cpu;
        let n = sched.num_cpus();
        let target_cpu = if cpu < n { cpu } else { 0 };
        sched.per_cpu[target_cpu].run_queue.enqueue(target_tid, sched.threads[idx].priority);
    }

    0
}

/// Walk the target's page tables and return memory regions.
///
/// Each region is 24 bytes: (start: u64, end: u64, flags: u64).
/// Returns number of regions written.
pub fn debug_get_mem_map(debugger_tid: u32, target_tid: u32, buf_ptr: u64, size: u32) -> u32 {
    let target_cr3;
    {
        let guard = SCHEDULER.lock();
        let sched = match guard.as_ref() {
            Some(s) => s,
            None => return u32::MAX,
        };

        let idx = match sched.threads.iter().position(|t| t.tid == target_tid) {
            Some(i) => i,
            None => return u32::MAX,
        };

        if sched.threads[idx].debug_attached_by != debugger_tid {
            return u32::MAX;
        }

        target_cr3 = sched.threads[idx].context.cr3;
    }

    if target_cr3 == 0 {
        return u32::MAX;
    }

    // Walk page tables under CR3-switch
    let max_regions = (size as usize) / 24;
    let regions = cr3_switch_walk_pages(target_cr3, max_regions);

    // Copy results to user buffer
    let count = regions.len().min(max_regions);
    for i in 0..count {
        let (start, end, flags) = regions[i];
        let offset = (i * 24) as u64;
        unsafe {
            let dst = (buf_ptr + offset) as *mut u64;
            dst.write(start);
            dst.add(1).write(end);
            dst.add(2).write(flags);
        }
    }

    count as u32
}

/// Poll for a pending debug event on the target thread.
///
/// Returns event type (1=BP, 2=step, 3=exit), 0 if no event, u32::MAX on error.
pub fn debug_wait_event(debugger_tid: u32, target_tid: u32, buf_ptr: u64, size: u32) -> u32 {
    let mut guard = SCHEDULER.lock();
    let sched = match guard.as_mut() {
        Some(s) => s,
        None => return u32::MAX,
    };

    let idx = match sched.threads.iter().position(|t| t.tid == target_tid) {
        Some(i) => i,
        None => return u32::MAX,
    };

    if sched.threads[idx].debug_attached_by != debugger_tid {
        return u32::MAX;
    }

    let event = sched.threads[idx].debug_event.take();
    match event {
        Some((event_type, addr)) => {
            // Write event data to user buffer if provided
            if buf_ptr != 0 && size >= 12 {
                unsafe {
                    let dst = buf_ptr as *mut u32;
                    dst.write(event_type);
                    let addr_dst = (buf_ptr + 4) as *mut u64;
                    addr_dst.write(addr);
                }
            }
            event_type
        }
        None => 0,
    }
}

/// Get extended thread information.
///
/// Layout (128 bytes):
///   0: parent_tid (u32)
///   4: state (u32)
///   8: priority (u32)
///  12: cpu_ticks (u32)
///  16: last_cpu (u32)
///  20: user_pages (u32)
///  24: brk (u32)
///  28: mmap_next (u32)
///  32: rip (u64)
///  40: rsp (u64)
///  48: cr3 (u64)
///  56: io_read_bytes (u64)
///  64: io_write_bytes (u64)
///  72: capabilities (u32)
///  76: uid (u16)
///  78: gid (u16)
///  80: debug_attached_by (u32)
///  84: name (32 bytes)
/// 116: arch_mode (u32)
/// 120: reserved (8 bytes)
///
/// Returns number of bytes written.
pub fn thread_info_ex(target_tid: u32, buf_ptr: u64, size: u32) -> u32 {
    let guard = SCHEDULER.lock();
    let sched = match guard.as_ref() {
        Some(s) => s,
        None => return u32::MAX,
    };

    let thread = match sched.threads.iter().find(|t| t.tid == target_tid) {
        Some(t) => t,
        None => return u32::MAX,
    };

    let write_len = (size as usize).min(128);
    let mut buf = [0u8; 128];

    // Pack fields into buffer
    let state_u32: u32 = match thread.state {
        ThreadState::Ready => 0,
        ThreadState::Running => 1,
        ThreadState::Blocked => 2,
        ThreadState::Terminated => 3,
    };

    // Helper to write u32 LE at offset
    fn put_u32(buf: &mut [u8], off: usize, val: u32) {
        if off + 4 <= buf.len() {
            buf[off..off + 4].copy_from_slice(&val.to_le_bytes());
        }
    }
    fn put_u64(buf: &mut [u8], off: usize, val: u64) {
        if off + 8 <= buf.len() {
            buf[off..off + 8].copy_from_slice(&val.to_le_bytes());
        }
    }
    fn put_u16(buf: &mut [u8], off: usize, val: u16) {
        if off + 2 <= buf.len() {
            buf[off..off + 2].copy_from_slice(&val.to_le_bytes());
        }
    }

    put_u32(&mut buf, 0, thread.parent_tid);
    put_u32(&mut buf, 4, state_u32);
    put_u32(&mut buf, 8, thread.priority as u32);
    put_u32(&mut buf, 12, thread.cpu_ticks);
    put_u32(&mut buf, 16, thread.last_cpu as u32);
    put_u32(&mut buf, 20, thread.user_pages);
    put_u32(&mut buf, 24, thread.brk);
    put_u32(&mut buf, 28, thread.mmap_next);
    put_u64(&mut buf, 32, thread.context.rip);
    put_u64(&mut buf, 40, thread.context.rsp);
    put_u64(&mut buf, 48, thread.context.cr3);
    put_u64(&mut buf, 56, thread.io_read_bytes);
    put_u64(&mut buf, 64, thread.io_write_bytes);
    put_u32(&mut buf, 72, thread.capabilities);
    put_u16(&mut buf, 76, thread.uid);
    put_u16(&mut buf, 78, thread.gid);
    put_u32(&mut buf, 80, thread.debug_attached_by);
    // Copy name (32 bytes at offset 84)
    let name_end = 84 + 32;
    if name_end <= buf.len() {
        buf[84..name_end].copy_from_slice(&thread.name);
    }
    let arch_mode_u32: u32 = match thread.arch_mode {
        crate::task::thread::ArchMode::Native64 => 0,
        crate::task::thread::ArchMode::Compat32 => 1,
    };
    put_u32(&mut buf, 116, arch_mode_u32);

    // Copy to user buffer
    unsafe {
        let dst = buf_ptr as *mut u8;
        core::ptr::copy_nonoverlapping(buf.as_ptr(), dst, write_len);
    }

    write_len as u32
}

/// Called from ISR 1 (#DB) or ISR 3 (#BP) when a debug-attached thread
/// hits a breakpoint or completes a single-step.
///
/// This is called from interrupt context — uses lock-free per-CPU TID lookup
/// and deferred wake for the debugger.
pub fn debug_auto_suspend(tid: u32, event_type: u32, addr: u64) {
    // Must use try_lock since we're in interrupt context
    if let Some(mut guard) = SCHEDULER.try_lock() {
        if let Some(sched) = guard.as_mut() {
            if let Some(idx) = sched.threads.iter().position(|t| t.tid == tid) {
                if sched.threads[idx].debug_attached_by != 0 {
                    sched.threads[idx].debug_suspended = true;
                    sched.threads[idx].debug_event = Some((event_type, addr));
                    sched.threads[idx].debug_single_step = false;

                    // Clear TF so the thread doesn't immediately step again
                    sched.threads[idx].context.rflags &= !0x100;
                    sched.threads[idx].context.checksum =
                        sched.threads[idx].context.compute_checksum();

                    // Block the thread — it will be suspended until debugger resumes it
                    if sched.threads[idx].state == ThreadState::Running {
                        sched.threads[idx].context.save_complete = 0;
                        sched.threads[idx].state = ThreadState::Blocked;
                    }
                }
            }
        }
    }
    // If lock failed, the thread will continue running (no debug event recorded).
    // This is acceptable — the debugger will retry.
}

/// Check if the current thread on this CPU is debug-attached.
/// Lock-free: reads per-CPU TID atomically.
pub fn is_debug_attached_current() -> bool {
    let cpu_id = crate::arch::x86::smp::current_cpu_id() as usize;
    let tid = if cpu_id < MAX_CPUS {
        PER_CPU_CURRENT_TID[cpu_id].load(Ordering::Relaxed)
    } else {
        return false;
    };
    if tid == 0 {
        return false;
    }

    // Must use try_lock since this may be called from ISR context
    if let Some(guard) = SCHEDULER.try_lock() {
        if let Some(sched) = guard.as_ref() {
            if let Some(thread) = sched.threads.iter().find(|t| t.tid == tid) {
                return thread.debug_attached_by != 0;
            }
        }
    }
    false
}

// ---- CR3-switch helpers for cross-process memory access ----

/// Read `size` bytes from another process's address space.
/// Uses the cli → CR3 switch → copy → restore pattern.
///
/// Returns number of bytes actually read.
fn cr3_switch_read(target_cr3: u64, src_addr: u64, dst_addr: u64, size: u32) -> u32 {
    if size == 0 {
        return 0;
    }
    let len = size.min(4096) as usize;

    // Use kernel stack buffer as intermediate: after switching to the target's
    // CR3 the debugger's user-space pages are no longer mapped, so we cannot
    // write directly into dst_addr.  Kernel stack addresses (higher-half) are
    // accessible regardless of which CR3 is active.
    let mut tmp = [0u8; 4096];
    let mut read = 0usize;

    unsafe {
        let rflags: u64;
        core::arch::asm!("pushfq; pop {}", out(reg) rflags, options(nomem));
        core::arch::asm!("cli", options(nomem, nostack));

        let old_cr3 = virtual_mem::current_cr3();
        core::arch::asm!("mov cr3, {}", in(reg) target_cr3);

        // Read page-by-page, checking mappings via recursive page tables
        // to avoid #PF on unmapped addresses.
        let src = src_addr as *const u8;
        while read < len {
            let cur_addr = src_addr + read as u64;
            if !is_page_present_recursive(cur_addr) {
                break; // Stop at first unmapped page
            }
            // Read until end of this 4K page or end of requested range
            let page_end = (cur_addr & !0xFFF) + 0x1000;
            let chunk_end = core::cmp::min(page_end as usize, src_addr as usize + len);
            let chunk_len = chunk_end - cur_addr as usize;
            for _ in 0..chunk_len {
                tmp[read] = src.add(read).read_volatile();
                read += 1;
            }
        }

        core::arch::asm!("mov cr3, {}", in(reg) old_cr3);
        core::arch::asm!("push {}; popfq", in(reg) rflags, options(nomem));
    }

    // Copy from kernel buffer to debugger's user buffer (now back in
    // debugger's address space).
    if read > 0 {
        unsafe {
            let dst = dst_addr as *mut u8;
            for i in 0..read {
                dst.add(i).write(tmp[i]);
            }
        }
    }

    read as u32
}

/// Check if a virtual address has a present page mapping using the recursive
/// page table structure.  Must be called with the target CR3 already active
/// and interrupts disabled.
unsafe fn is_page_present_recursive(vaddr: u64) -> bool {
    use crate::memory::address::VirtAddr;
    let v = VirtAddr::new(vaddr);
    let ri = 510u64; // RECURSIVE_INDEX
    let pml4i = v.pml4_index() as u64;
    let pdpti = v.pdpt_index() as u64;
    let pdi = v.pd_index() as u64;

    // PML4 — recursive_pml4_base = ri<<39 | ri<<30 | ri<<21 | ri<<12
    let pml4_ptr = 0xFFFF_FF7F_BFDF_E000u64 as *const u64;
    let pml4e = pml4_ptr.add(v.pml4_index()).read_volatile();
    if pml4e & 1 == 0 { return false; }

    // PDPT — recursive_pdpt_base = ri<<39 | ri<<30 | ri<<21 | pml4i<<12
    let pdpt_ptr = sign_extend_addr(ri << 39 | ri << 30 | ri << 21 | pml4i << 12) as *const u64;
    let pdpte = pdpt_ptr.add(v.pdpt_index()).read_volatile();
    if pdpte & 1 == 0 { return false; }

    // PD — recursive_pd_base = ri<<39 | ri<<30 | pml4i<<21 | pdpti<<12
    let pd_ptr = sign_extend_addr(ri << 39 | ri << 30 | pml4i << 21 | pdpti << 12) as *const u64;
    let pde = pd_ptr.add(v.pd_index()).read_volatile();
    if pde & 1 == 0 { return false; }

    // Check for 2 MiB huge page (PS bit)
    if pde & (1 << 7) != 0 { return true; }

    // PT — recursive_pt_base = ri<<39 | pml4i<<30 | pdpti<<21 | pdi<<12
    let pt_ptr = sign_extend_addr(ri << 39 | pml4i << 30 | pdpti << 21 | pdi << 12) as *const u64;
    let pte = pt_ptr.add(v.pt_index()).read_volatile();
    pte & 1 != 0
}

/// Sign-extend a 48-bit virtual address to canonical 64-bit form.
fn sign_extend_addr(addr: u64) -> u64 {
    if addr & (1u64 << 47) != 0 {
        addr | 0xFFFF_0000_0000_0000
    } else {
        addr & 0x0000_FFFF_FFFF_FFFF
    }
}

/// Write `size` bytes into another process's address space.
///
/// Uses a kernel stack buffer as intermediate because the source buffer
/// (in the debugger's address space) is not mapped under the target's CR3.
fn cr3_switch_write(target_cr3: u64, dst_addr: u64, src_addr: u64, size: u32) -> u32 {
    if size == 0 {
        return 0;
    }
    let len = size.min(4096) as usize;

    // Copy source data into kernel stack buffer first (while still in
    // debugger's address space).
    let mut tmp = [0u8; 4096];
    unsafe {
        let src = src_addr as *const u8;
        for i in 0..len {
            tmp[i] = src.add(i).read();
        }
    }

    let mut written = 0usize;
    unsafe {
        let rflags: u64;
        core::arch::asm!("pushfq; pop {}", out(reg) rflags, options(nomem));
        core::arch::asm!("cli", options(nomem, nostack));

        let old_cr3 = virtual_mem::current_cr3();
        core::arch::asm!("mov cr3, {}", in(reg) target_cr3);

        let dst = dst_addr as *mut u8;
        while written < len {
            let cur_addr = dst_addr + written as u64;
            if !is_page_present_recursive(cur_addr) {
                break;
            }
            let page_end = (cur_addr & !0xFFF) + 0x1000;
            let chunk_end = core::cmp::min(page_end as usize, dst_addr as usize + len);
            let chunk_len = chunk_end - cur_addr as usize;
            for _ in 0..chunk_len {
                dst.add(written).write_volatile(tmp[written]);
                written += 1;
            }
        }

        core::arch::asm!("mov cr3, {}", in(reg) old_cr3);
        core::arch::asm!("push {}; popfq", in(reg) rflags, options(nomem));
    }

    written as u32
}

/// Restore original bytes at breakpoint locations via CR3-switch.
fn restore_breakpoint_bytes(target_cr3: u64, breakpoints: &[(u64, u8)]) {
    for &(addr, original_byte) in breakpoints {
        if addr != 0 {
            cr3_switch_write(target_cr3, addr, &original_byte as *const u8 as u64, 1);
        }
    }
}

/// Walk page tables under CR3-switch to enumerate mapped memory regions.
///
/// Returns a Vec of (start_addr, end_addr, flags) tuples.
fn cr3_switch_walk_pages(target_cr3: u64, max_regions: usize) -> Vec<(u64, u64, u64)> {
    let mut regions: Vec<(u64, u64, u64)> = Vec::new();

    if max_regions == 0 {
        return regions;
    }

    unsafe {
        let rflags: u64;
        core::arch::asm!("pushfq; pop {}", out(reg) rflags, options(nomem));
        core::arch::asm!("cli", options(nomem, nostack));

        let old_cr3 = virtual_mem::current_cr3();
        core::arch::asm!("mov cr3, {}", in(reg) target_cr3);

        // Walk PML4 entries 0..255 (user-space half only)
        let pml4 = 0xFFFF_FF7F_BFDF_E000u64 as *const u64; // RECURSIVE_PML4_BASE

        let mut cur_start: u64 = 0;
        let mut cur_flags: u64 = 0;
        let mut cur_end: u64 = 0;
        let mut in_region = false;

        'outer: for pml4i in 0..256usize {
            let pml4e = pml4.add(pml4i).read_volatile();
            if pml4e & 1 == 0 { // PAGE_PRESENT
                if in_region {
                    regions.push((cur_start, cur_end, cur_flags));
                    in_region = false;
                    if regions.len() >= max_regions { break 'outer; }
                }
                continue;
            }

            // PDPT entries
            let pdpt_base = 0xFFFF_FF7F_BFC0_0000u64 + (pml4i as u64) * 0x1000;
            let pdpt = pdpt_base as *const u64;

            for pdpti in 0..512usize {
                let pdpte = pdpt.add(pdpti).read_volatile();
                if pdpte & 1 == 0 {
                    if in_region {
                        regions.push((cur_start, cur_end, cur_flags));
                        in_region = false;
                        if regions.len() >= max_regions { break 'outer; }
                    }
                    continue;
                }

                // PD entries
                let pd_base = 0xFFFF_FF7F_8000_0000u64
                    + (pml4i as u64) * 0x20_0000
                    + (pdpti as u64) * 0x1000;
                let pd = pd_base as *const u64;

                for pdi in 0..512usize {
                    let pde = pd.add(pdi).read_volatile();
                    if pde & 1 == 0 {
                        if in_region {
                            regions.push((cur_start, cur_end, cur_flags));
                            in_region = false;
                            if regions.len() >= max_regions { break 'outer; }
                        }
                        continue;
                    }

                    // Check for 2 MiB huge page (PS bit)
                    if pde & 0x80 != 0 {
                        let page_start = ((pml4i as u64) << 39)
                            | ((pdpti as u64) << 30)
                            | ((pdi as u64) << 21);
                        let page_end = page_start + 0x20_0000; // 2 MiB
                        let page_flags = pde & 0x8000_0000_0000_001F; // P|RW|US|PWT|PCD + NX

                        if in_region && page_flags == cur_flags && page_start == cur_end {
                            cur_end = page_end;
                        } else {
                            if in_region {
                                regions.push((cur_start, cur_end, cur_flags));
                                if regions.len() >= max_regions { break 'outer; }
                            }
                            cur_start = page_start;
                            cur_end = page_end;
                            cur_flags = page_flags;
                            in_region = true;
                        }
                        continue;
                    }

                    // PT entries
                    let pt_base = 0xFFFF_FF00_0000_0000u64
                        + (pml4i as u64) * 0x4000_0000
                        + (pdpti as u64) * 0x20_0000
                        + (pdi as u64) * 0x1000;
                    let pt = pt_base as *const u64;

                    for pti in 0..512usize {
                        let pte = pt.add(pti).read_volatile();
                        if pte & 1 == 0 {
                            if in_region {
                                regions.push((cur_start, cur_end, cur_flags));
                                in_region = false;
                                if regions.len() >= max_regions { break 'outer; }
                            }
                            continue;
                        }

                        let page_start = ((pml4i as u64) << 39)
                            | ((pdpti as u64) << 30)
                            | ((pdi as u64) << 21)
                            | ((pti as u64) << 12);
                        let page_end = page_start + 0x1000; // 4 KiB
                        let page_flags = pte & 0x8000_0000_0000_001F;

                        if in_region && page_flags == cur_flags && page_start == cur_end {
                            cur_end = page_end;
                        } else {
                            if in_region {
                                regions.push((cur_start, cur_end, cur_flags));
                                if regions.len() >= max_regions { break 'outer; }
                            }
                            cur_start = page_start;
                            cur_end = page_end;
                            cur_flags = page_flags;
                            in_region = true;
                        }
                    }
                }
            }
        }

        if in_region && regions.len() < max_regions {
            regions.push((cur_start, cur_end, cur_flags));
        }

        core::arch::asm!("mov cr3, {}", in(reg) old_cr3);
        core::arch::asm!("push {}; popfq", in(reg) rflags, options(nomem));
    }

    regions
}
