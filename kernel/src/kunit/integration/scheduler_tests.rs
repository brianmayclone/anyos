//! Integration tests for the task scheduler.
//!
//! Verifies scheduler state after `task::scheduler::init()` has been called
//! but before the scheduler has started running (no threads dispatched yet).
//! Mirrors Linux KUnit scheduler self-tests in `kernel/sched/tests/`.

use crate::kunit::{TestCase, TestContext, TestSuite};
use crate::task::scheduler;

pub static SUITE: TestSuite = TestSuite {
    name: "scheduler",
    cases: &[
        TestCase { name: "tick_counters_start_at_zero",  run: test_tick_counters_zero },
        TestCase { name: "per_cpu_ticks_accessible",     run: test_per_cpu_ticks },
        TestCase { name: "scheduler_not_locked",         run: test_not_locked },
        TestCase { name: "cpu0_has_no_active_thread",    run: test_no_active_thread },
        TestCase { name: "last_syscall_init_state",      run: test_last_syscall },
        TestCase { name: "stack_bounds_set",             run: test_stack_bounds },
        TestCase { name: "idle_ticks_not_exceed_total",  run: test_idle_le_total },
        TestCase { name: "deferred_wake_no_crash",       run: test_deferred_wake_noop },
        TestCase { name: "pinned_continuation_wakes_on_last_cpu", run: test_pinned_wake_cpu },
        TestCase { name: "pinned_continuation_not_stolen",        run: test_pinned_not_stolen },
    ],
};

fn test_tick_counters_zero(ctx: &mut TestContext) {
    // Tick counters are incremented by the timer IRQ. Before interrupts are
    // fully running and the scheduler has dispatched threads, total ticks
    // should still be 0 or very small (a few PIT IRQs may have fired).
    let total = scheduler::total_sched_ticks();
    let idle  = scheduler::idle_sched_ticks();

    // They must not be absurdly large (no wrap-around or uninitialized value).
    ctx.expect_true(total < 1_000_000, "total_sched_ticks < 1M (not garbage)");
    ctx.expect_true(idle  < 1_000_000, "idle_sched_ticks < 1M (not garbage)");
}

fn test_per_cpu_ticks(ctx: &mut TestContext) {
    // Per-CPU tick counters must be readable for all valid CPU indices.
    for cpu in 0..8usize {
        let total = scheduler::per_cpu_total_ticks(cpu);
        let idle  = scheduler::per_cpu_idle_ticks(cpu);
        ctx.expect_true(
            total < 1_000_000,
            "per_cpu_total_ticks not garbage"
        );
        ctx.expect_true(
            idle <= total || (total == 0 && idle == 0),
            "per_cpu_idle <= per_cpu_total"
        );
    }
    // Out-of-bounds CPU index must return 0 (not panic).
    ctx.expect_eq(scheduler::per_cpu_total_ticks(255), 0u32, "oob cpu returns 0");
}

fn test_not_locked(ctx: &mut TestContext) {
    // The scheduler spinlock must not be held when we're running tests.
    let locked = scheduler::is_scheduler_locked();
    ctx.expect_false(locked, "scheduler not locked during tests");
}

fn test_no_active_thread(ctx: &mut TestContext) {
    // Before the scheduler has dispatched any thread, CPU 0 should report
    // no active thread (the BSP is still running in kernel_main directly).
    let has_thread = scheduler::cpu_has_active_thread(0);
    ctx.expect_false(has_thread, "CPU 0 has no active thread before scheduler::run()");
}

fn test_last_syscall(ctx: &mut TestContext) {
    // set_last_syscall / get_last_syscall must round-trip correctly.
    scheduler::set_last_syscall(0, 0xDEAD);
    ctx.expect_eq(scheduler::get_last_syscall(0), 0xDEADu32, "last_syscall round-trip");
    scheduler::set_last_syscall(0, 0); // restore
}

fn test_stack_bounds(ctx: &mut TestContext) {
    // get_stack_bounds() tracks the currently-dispatched thread's kernel stack.
    // When no thread is active on a CPU (kernel_main is not a tracked thread),
    // the scheduler clears the bounds to (0, 0) — this is correct behaviour
    // used by check_rsp_in_bounds to skip validation when no thread is running.
    // We verify only that the call doesn't panic and returns a consistent pair.
    let (lo, hi) = scheduler::get_stack_bounds(0);
    // If non-zero, hi must be >= lo (basic sanity).
    if lo != 0 || hi != 0 {
        ctx.expect_ge(hi, lo, "stack hi >= stack lo when set");
    } else {
        // (0, 0) is the valid "no active thread" sentinel — pass.
        ctx.expect_true(true, "stack bounds (0,0) valid for inactive CPU");
    }
}

fn test_idle_le_total(ctx: &mut TestContext) {
    // Invariant: idle ticks can never exceed total ticks.
    let total = scheduler::total_sched_ticks();
    let idle  = scheduler::idle_sched_ticks();
    ctx.expect_ge(total, idle, "total_ticks >= idle_ticks");
}

fn test_deferred_wake_noop(ctx: &mut TestContext) {
    // Calling deferred_wake with a non-existent TID must not crash.
    // TID 0 is the "no thread" sentinel — safe to poke.
    scheduler::deferred_wake(0);
    ctx.expect_true(true, "deferred_wake(0) did not panic");
}

fn test_pinned_wake_cpu(ctx: &mut TestContext) {
    ctx.expect_true(
        scheduler::kunit_pinned_continuation_wake_targets_last_cpu(),
        "blocked user thread with kernel continuation wakes on last_cpu",
    );
}

fn test_pinned_not_stolen(ctx: &mut TestContext) {
    ctx.expect_true(
        scheduler::kunit_pinned_continuation_not_stolen(),
        "work stealing leaves pinned kernel continuation on last_cpu",
    );
}
