//! Scheduler stress tests.
//!
//! Two modes:
//! - `stress_master`: the original single-threaded spawn→exit→reap loop
//!   (lifecycle smoke test), enabled only with the `debug_verbose` feature.
//! - `smp_stress_master`: a CONCURRENT multi-CPU stress test, opt-in via the
//!   `schedstress` boot parameter. It is the safety net for the Phase 4b
//!   per-CPU run-queue lock split: it hammers the scheduler with parallel
//!   spawn / block / wake / exit / migration across all CPUs and verifies the
//!   invariants a lock change could break (every worker completes, the thread
//!   table returns to baseline = no leaked/lost threads). Run it before and
//!   after any scheduler-locking change to catch SMP races and missed wakes.

use core::sync::atomic::{AtomicU32, Ordering};

/// Number of completed worker threads (for progress reporting).
static WORKERS_COMPLETED: AtomicU32 = AtomicU32::new(0);

/// Worker thread: does minimal work, then exits.
/// The kernel_thread_exit trampoline (set up in Thread::new) catches the return,
/// but we call exit_current explicitly for clarity.
extern "C" fn stress_worker() {
    WORKERS_COMPLETED.fetch_add(1, Ordering::Relaxed);
    crate::task::scheduler::exit_current(0);
}

/// Master thread: spawns workers in a tight loop, waits for each to complete.
/// Prints progress every 100 iterations along with scheduler state.
pub extern "C" fn stress_master() {
    crate::serial_verbose_println!("STRESS: thread lifecycle test started (spawn+exit+reap loop)");
    let mut iter: u32 = 0;
    loop {
        // Spawn a worker
        let tid = crate::task::scheduler::spawn(stress_worker, 50, "stress_w");

        // Wait for it to terminate
        crate::task::scheduler::waitpid(tid);

        iter += 1;
        if iter % 100 == 0 {
            let completed = WORKERS_COMPLETED.load(Ordering::Relaxed);
            let total = crate::task::scheduler::total_sched_ticks();
            let idle = crate::task::scheduler::idle_sched_ticks();
            crate::serial_verbose_println!(
                "STRESS: iter={} done={} ticks={}/{} ({}% idle)",
                iter,
                completed,
                idle,
                total,
                if total > 0 {
                    idle as u64 * 100 / total as u64
                } else {
                    0
                },
            );
        }
    }
}

// ── Concurrent SMP stress test (Phase 4b safety net) ─────────────────────────

const SMP_ROUNDS: u32 = 40;
const SMP_BATCH: u32 = 48;

static SMP_SPAWNED: AtomicU32 = AtomicU32::new(0);
static SMP_COMPLETED: AtomicU32 = AtomicU32::new(0);

/// Concurrent worker: forces the block→wake→migrate→exit transitions a
/// run-queue lock change is most likely to break. A short timed sleep makes
/// the thread block and later be woken by the timer/sleeper path (re-enqueued,
/// possibly on a different CPU), instead of just running straight to exit.
extern "C" fn smp_stress_worker() {
    let hz = crate::arch::hal::timer_frequency_hz().max(1) as u32;
    // ~2 ms: long enough to actually block and be re-scheduled, short enough
    // that a full round stays brief.
    let ticks = (2 * hz / 1000).max(1);
    let now = crate::arch::hal::timer_current_ticks();
    crate::task::scheduler::sleep_until(now.wrapping_add(ticks));
    SMP_COMPLETED.fetch_add(1, Ordering::Relaxed);
    crate::task::scheduler::exit_current(0);
}

/// Count threads whose name marks them as stress workers — used to confirm the
/// scheduler reaped them all (no leaked/stuck threads) after each round.
fn live_stress_workers() -> usize {
    crate::task::scheduler::list_threads()
        .iter()
        .filter(|t| t.name.starts_with("smp_stress"))
        .count()
}

/// Master for the concurrent SMP scheduler stress test. Each round spawns
/// `SMP_BATCH` workers *before* joining any, so up to `SMP_BATCH` threads run
/// and block/wake concurrently across every CPU. After joining, it verifies no
/// worker leaked. At the end it reports PASS/FAIL with the spawn/complete tally
/// and the final thread-table size relative to the pre-test baseline.
pub extern "C" fn smp_stress_master() {
    let baseline = crate::task::scheduler::list_threads().len();
    crate::serial_println!(
        "SCHEDSTRESS: concurrent SMP stress starting (rounds={} batch={} baseline_threads={})",
        SMP_ROUNDS,
        SMP_BATCH,
        baseline
    );

    let mut tids: alloc::vec::Vec<u32> = alloc::vec::Vec::with_capacity(SMP_BATCH as usize);
    let mut worst_leftover = 0usize;

    for round in 0..SMP_ROUNDS {
        tids.clear();
        for _ in 0..SMP_BATCH {
            let tid = crate::task::scheduler::spawn(smp_stress_worker, 60, "smp_stress_w");
            if tid != 0 {
                SMP_SPAWNED.fetch_add(1, Ordering::Relaxed);
                tids.push(tid);
            }
        }
        // Join all workers of this round. They are already running/blocking on
        // other CPUs concurrently; waitpid blocks us until each terminates.
        for &tid in &tids {
            crate::task::scheduler::waitpid(tid);
        }

        // Give the deferred reaper a moment, then check no worker is stuck.
        let hz = crate::arch::hal::timer_frequency_hz().max(1) as u32;
        let now = crate::arch::hal::timer_current_ticks();
        crate::task::scheduler::sleep_until(now.wrapping_add((hz / 100).max(1)));
        let leftover = live_stress_workers();
        if leftover > worst_leftover {
            worst_leftover = leftover;
        }

        if round % 5 == 0 || round == SMP_ROUNDS - 1 {
            crate::serial_println!(
                "SCHEDSTRESS: round {}/{} spawned={} completed={} live_workers={}",
                round + 1,
                SMP_ROUNDS,
                SMP_SPAWNED.load(Ordering::Relaxed),
                SMP_COMPLETED.load(Ordering::Relaxed),
                leftover
            );
        }
    }

    // Final drain so the reaper recycles the last round.
    let hz = crate::arch::hal::timer_frequency_hz().max(1) as u32;
    let now = crate::arch::hal::timer_current_ticks();
    crate::task::scheduler::sleep_until(now.wrapping_add(hz / 2));

    let spawned = SMP_SPAWNED.load(Ordering::Relaxed);
    let completed = SMP_COMPLETED.load(Ordering::Relaxed);
    let final_threads = crate::task::scheduler::list_threads().len();
    let leftover = live_stress_workers();

    // Invariants (scheduler correctness under concurrency):
    //  - every spawned worker ran its body to completion (no lost wake / no
    //    thread stuck Blocked forever),
    //  - no stress worker is left in the table at the end, and none was ever
    //    seen stuck after a round (all reaped) => no stress-worker leak.
    //
    // NOTE: we deliberately do NOT compare final_threads to `baseline`. The
    // test runs while userspace is still starting (compositor/login/dock spawn
    // their own threads), so the table legitimately grows by a handful during
    // the run — that is system startup, not a scheduler leak. `worst_leftover`
    // is the precise, startup-immune leak signal because it counts only
    // stress-named workers. final_threads/baseline are logged for visibility.
    let all_completed = completed == spawned && spawned > 0;
    let no_worker_leak = leftover == 0 && worst_leftover == 0;

    if all_completed && no_worker_leak {
        crate::serial_println!(
            "SCHEDSTRESS: PASS spawned={} completed={} worst_leftover={} (threads {} -> {} incl. concurrent userspace startup)",
            spawned,
            completed,
            worst_leftover,
            baseline,
            final_threads
        );
    } else {
        crate::serial_println!(
            "SCHEDSTRESS: FAIL spawned={} completed={} all_completed={} no_worker_leak={} leftover={} worst_leftover={} threads {} -> {}",
            spawned,
            completed,
            all_completed,
            no_worker_leak,
            leftover,
            worst_leftover,
            baseline,
            final_threads
        );
    }

    crate::task::scheduler::exit_current(0);
}
