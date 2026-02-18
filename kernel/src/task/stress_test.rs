//! Kernel thread lifecycle stress test.
//!
//! Rapidly creates and destroys kernel threads to exercise the full
//! scheduler lifecycle: spawn → schedule → run → exit → reap.
//! Enabled only with the `debug_verbose` Cargo feature.

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
    crate::serial_println!("STRESS: thread lifecycle test started (spawn+exit+reap loop)");
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
            crate::serial_println!(
                "STRESS: iter={} done={} ticks={}/{} ({}% idle)",
                iter, completed, idle, total,
                if total > 0 { idle as u64 * 100 / total as u64 } else { 0 },
            );
        }
    }
}
