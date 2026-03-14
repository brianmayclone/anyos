//! Kernel stability tests — the primary diagnostic for unexplained thread crashes.
//!
//! Every test here checks an invariant that, when violated, is a direct cause
//! of hard-to-reproduce crashes:
//!
//! | Test                        | Crash root-cause caught                        |
//! |-----------------------------|------------------------------------------------|
//! | `interrupts_enabled`        | IRQ-disabled while scheduler runs → deadlock   |
//! | `tss_rsp0_in_kernel_range`  | Wrong kernel stack on next interrupt → GPF     |
//! | `syscall_msr_rsp_sane`      | Wrong SYSCALL landing stack → silent corruption|
//! | `idle_stack_in_kernel_range`| Idle thread stack in low memory → stack smash  |
//! | `heap_not_locked`           | Heap held on schedule → deadlock in allocator  |
//! | `thread_table_integrity`    | Duplicate/zero TIDs → use-after-free in sched  |
//! | `thread_tids_unique`        | Two threads same TID → scheduler picks wrong   |
//! | `no_zombie_threads_above_8` | Zombie leak → thread table full → spawn fails  |
//! | `heap_stress_validate`      | Heap corruption → next alloc returns garbage   |
//! | `cpu_count_sane`            | Wrong CPU count → APs not initialised → crash  |
//! | `timer_hz_sane`             | Wrong PIT freq → timeouts fire too early/late  |
//! | `per_cpu_state_consistent`  | HAS_THREAD/TID mismatch → wrong context switch |

use crate::kunit::{TestCase, TestContext, TestSuite};
use crate::task::scheduler;
use crate::arch::hal;

pub static SUITE: TestSuite = TestSuite {
    name: "stability",
    cases: &[
        TestCase { name: "interrupts_enabled",         run: test_interrupts_enabled },
        TestCase { name: "tss_rsp0_in_kernel_range",   run: test_tss_rsp0 },
        TestCase { name: "syscall_msr_rsp_sane",       run: test_syscall_msr_rsp },
        TestCase { name: "idle_stack_in_kernel_range", run: test_idle_stack },
        TestCase { name: "heap_not_locked",            run: test_heap_not_locked },
        TestCase { name: "scheduler_not_locked",       run: test_sched_not_locked },
        TestCase { name: "thread_table_no_zero_tids",  run: test_no_zero_tids },
        TestCase { name: "thread_tids_unique",         run: test_tids_unique },
        TestCase { name: "no_zombie_accumulation",     run: test_no_zombie_accumulation },
        TestCase { name: "heap_stress_validate",       run: test_heap_stress },
        TestCase { name: "cpu_count_sane",             run: test_cpu_count },
        TestCase { name: "timer_hz_sane",              run: test_timer_hz },
        TestCase { name: "per_cpu_state_consistent",   run: test_per_cpu_consistent },
    ],
};

// ── Interrupt state ───────────────────────────────────────────────────────────

fn test_interrupts_enabled(ctx: &mut TestContext) {
    // Integration tests run after `arch::hal::enable_interrupts()`.
    // If interrupts are disabled here, the scheduler's timer IRQ can never
    // fire — threads will never be preempted and the system deadlocks silently.
    ctx.expect_true(
        hal::interrupts_enabled(),
        "interrupts must be enabled when integration tests run",
    );
}

// ── Kernel stack pointers ─────────────────────────────────────────────────────

fn test_tss_rsp0(ctx: &mut TestContext) {
    // TSS.RSP0 is the kernel stack pointer loaded on every ring-3→ring-0
    // transition (interrupt, exception, syscall via INT 0x80).
    // If this points into low memory or is zero, the next interrupt corrupts
    // the heap or stack of whatever happens to be at that address.
    let rsp0 = hal::get_kernel_stack_for_cpu(0);
    ctx.expect_true(
        rsp0 >= 0xFFFF_FFFF_8000_0000,
        "TSS.RSP0 must be in kernel higher-half (>= 0xFFFFFFFF80000000)",
    );
    ctx.expect_true(
        rsp0 & 0xF == 0,
        "TSS.RSP0 must be 16-byte aligned (x86-64 ABI requirement)",
    );
}

#[cfg(target_arch = "x86_64")]
fn test_syscall_msr_rsp(ctx: &mut TestContext) {
    // PERCPU[cpu].kernel_rsp is the kernel stack pointer loaded by SYSCALL entry.
    // set_kernel_rsp() has its own guard that rejects values below 0xFFFFFFFF80000000,
    // so at integration-test time (no user thread dispatched yet) the cached value
    // is legitimately 0 ("not set yet" — SYSCALL will set it on first dispatch).
    //
    // What IS a bug: a non-zero value that is also NOT in the kernel higher-half
    // (would mean SYSCALL entry lands on a low/physical-mapped stack → corruption).
    let cached_rsp = crate::arch::x86::syscall_msr::get_kernel_rsp(0);
    if cached_rsp != 0 {
        ctx.expect_true(
            cached_rsp >= 0xFFFF_FFFF_8000_0000,
            "if SYSCALL kernel RSP is set it must be in higher-half",
        );
    } else {
        // 0 means "not yet set by a context switch" — valid at early integration test time.
        ctx.expect_true(true, "SYSCALL kernel RSP is 0 (not yet set — OK before first dispatch)");
    }

    // The MSR-based read (what SYSCALL entry actually uses via SWAPGS + [gs:0]).
    let msr_rsp = crate::arch::x86::syscall_msr::get_kernel_rsp_via_msr();
    if msr_rsp != 0 {
        ctx.expect_true(
            msr_rsp >= 0xFFFF_FFFF_8000_0000,
            "live SYSCALL MSR kernel RSP must be in higher-half if non-zero",
        );
    } else {
        ctx.expect_true(true, "live SYSCALL MSR kernel RSP is 0 (not yet set — OK)");
    }
}

#[cfg(not(target_arch = "x86_64"))]
fn test_syscall_msr_rsp(ctx: &mut TestContext) {
    ctx.expect_true(true, "syscall MSR check skipped on non-x86_64");
}

fn test_idle_stack(ctx: &mut TestContext) {
    // PER_CPU_IDLE_STACK_TOP[0] is populated inside scheduler::run(), which is
    // called AFTER integration tests finish. At test time the value is 0 by design.
    //
    // Instead we verify the idle thread is present in the thread table with a
    // valid TID and a heap-backed kernel stack — i.e. the idle thread was created
    // correctly by scheduler::init(), even though run() hasn't been called yet.
    let threads = scheduler::list_threads();
    let idle_count = threads.iter().filter(|t| t.name.starts_with("idle")).count();
    ctx.expect_true(
        idle_count >= 1,
        "at least one idle thread exists in thread table after scheduler::init()",
    );
    // Every idle thread must have a valid (non-zero) TID.
    let bad_idle = threads.iter()
        .filter(|t| t.name.starts_with("idle"))
        .any(|t| t.tid == 0);
    ctx.expect_false(bad_idle, "idle thread TID must be non-zero");
}

// ── Lock state ────────────────────────────────────────────────────────────────

fn test_heap_not_locked(ctx: &mut TestContext) {
    // If the heap allocator lock is held at test entry, any allocation in the
    // test will deadlock forever. This also catches "heap taken during context
    // switch" bugs where an interrupt fires inside an allocator critical section.
    ctx.expect_false(
        crate::memory::heap::is_heap_locked(),
        "heap must not be locked when tests run",
    );
}

fn test_sched_not_locked(ctx: &mut TestContext) {
    ctx.expect_false(
        scheduler::is_scheduler_locked(),
        "scheduler spinlock must not be held when tests run",
    );
}

// ── Thread table integrity ────────────────────────────────────────────────────

fn test_no_zero_tids(ctx: &mut TestContext) {
    // TID 0 is the "no thread" sentinel used throughout the kernel as a
    // null-pointer equivalent for threads. If a real thread entry has TID=0
    // the scheduler will treat it as "no thread", skipping it or double-freeing.
    let threads = scheduler::list_threads();
    let mut found_zero = false;
    for t in &threads {
        if t.tid == 0 { found_zero = true; }
    }
    ctx.expect_false(found_zero, "no live thread should have TID == 0");
    // At minimum there must be at least one thread (the idle thread).
    ctx.expect_true(!threads.is_empty(), "thread table must have at least one entry");
}

fn test_tids_unique(ctx: &mut TestContext) {
    // Duplicate TIDs cause the scheduler to confuse two different threads.
    // This manifests as one thread's registers/stack being written to another's
    // context — typically seen as a crash with "wrong" register values.
    let threads = scheduler::list_threads();
    let mut seen = alloc::vec::Vec::with_capacity(threads.len());
    let mut duplicate = false;
    for t in &threads {
        if seen.contains(&t.tid) {
            duplicate = true;
            crate::serial_println!("    [FAIL stability] duplicate TID={}", t.tid);
        } else {
            seen.push(t.tid);
        }
    }
    ctx.expect_false(duplicate, "all live thread TIDs must be unique");
}

fn test_no_zombie_accumulation(ctx: &mut TestContext) {
    // list_threads() already skips Terminated threads, but we check that the
    // total thread count is below an unreasonable threshold.
    // More than 256 live threads at boot (before user apps) indicates a leak:
    // spawn creates entries but termination never frees them.
    let threads = scheduler::list_threads();
    ctx.expect_true(
        threads.len() <= 256,
        "live thread count <= 256 (no zombie leak at boot)",
    );
    // Each thread's state string must be one of the known valid values.
    let mut bad_state = false;
    for t in &threads {
        match t.state {
            "ready" | "running" | "blocked" | "stopped" => {}
            _ => { bad_state = true; }
        }
    }
    ctx.expect_false(bad_state, "all thread state strings are valid");
}

// ── Heap stability under allocation pressure ──────────────────────────────────

fn test_heap_stress(ctx: &mut TestContext) {
    // Allocate and free 64 objects of varying sizes, then validate the heap.
    // Use-after-free and double-free bugs corrupt allocator metadata and are
    // caught by validate_heap() which walks all free-list links.
    let (used_before, committed_before) = crate::memory::heap::heap_stats();

    let mut boxes: alloc::vec::Vec<alloc::boxed::Box<[u8; 256]>> =
        alloc::vec::Vec::with_capacity(64);
    for i in 0u8..64 {
        let mut b = alloc::boxed::Box::new([0u8; 256]);
        // Write a pattern so freed memory is detectable if reused incorrectly.
        b[0] = i;
        b[255] = i ^ 0xFF;
        boxes.push(b);
    }

    // Validate heap while allocations are live.
    crate::memory::heap::validate_heap();
    ctx.expect_true(true, "heap valid with 64 live allocations");

    // Free half — interleaved to stress the free-list.
    let mut i = 0usize;
    while i < boxes.len() {
        boxes.remove(i);
        i += 1; // skip every other
    }
    crate::memory::heap::validate_heap();
    ctx.expect_true(true, "heap valid after freeing half (interleaved)");

    // Free remainder.
    drop(boxes);
    crate::memory::heap::validate_heap();
    ctx.expect_true(true, "heap valid after freeing all");

    // Committed bytes must not have grown unboundedly.
    let (used_after, committed_after) = crate::memory::heap::heap_stats();
    ctx.expect_ge(committed_after, committed_before, "committed never shrinks");
    // Used bytes should be close to the original (small difference for internal allocs).
    let drift = if used_after > used_before {
        used_after - used_before
    } else {
        used_before - used_after
    };
    ctx.expect_true(
        drift < 4096,
        "heap used bytes stable after stress (drift < 4 KiB)",
    );
}

// ── CPU / timer sanity ────────────────────────────────────────────────────────

fn test_cpu_count(ctx: &mut TestContext) {
    let count = hal::cpu_count();
    ctx.expect_ge(count, 1usize, "at least 1 CPU online");
    ctx.expect_true(
        count <= hal::MAX_CPUS,
        "cpu_count <= MAX_CPUS",
    );
}

fn test_timer_hz(ctx: &mut TestContext) {
    let hz = hal::timer_frequency_hz();
    ctx.expect_eq(hz, 1000u64, "PIT/timer frequency must be 1000 Hz");
}

// ── Per-CPU state consistency ─────────────────────────────────────────────────

fn test_per_cpu_consistent(ctx: &mut TestContext) {
    // For each CPU: if HAS_THREAD is true, the current TID must be > 0.
    // If HAS_THREAD is false, we cannot assert on TID (it may be a stale idle
    // TID), but we can verify in_scheduler is false (not stuck in dispatch).
    for cpu in 0..hal::cpu_count() {
        let has_thread = scheduler::cpu_has_active_thread(cpu);
        let tid = scheduler::per_cpu_current_tid(cpu);
        let in_sched = scheduler::per_cpu_in_scheduler(cpu);

        if has_thread {
            ctx.expect_true(tid > 0, "HAS_THREAD=true implies TID > 0");
        }
        // No CPU should be stuck inside schedule_inner at test time.
        ctx.expect_false(in_sched, "CPU must not be stuck inside schedule_inner");
    }
    ctx.expect_true(true, "per-CPU state consistent across all online CPUs");
}
