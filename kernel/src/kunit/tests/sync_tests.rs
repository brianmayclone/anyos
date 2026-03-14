//! Tests for kernel synchronisation primitives (Spinlock).
//!
//! Note: Mutex requires the scheduler to be running (it yields on contention)
//! so Mutex tests would need a multi-threaded environment.  Only Spinlock is
//! tested here since it works in any boot context.

use crate::kunit::{TestCase, TestContext, TestSuite};
use crate::sync::spinlock::Spinlock;

pub static SUITE: TestSuite = TestSuite {
    name: "sync",
    cases: &[
        TestCase { name: "spinlock_basic_lock_unlock",  run: test_basic_lock_unlock },
        TestCase { name: "spinlock_data_access",        run: test_data_access },
        TestCase { name: "spinlock_mutation",           run: test_mutation },
        TestCase { name: "spinlock_bool_value",         run: test_bool_value },
        TestCase { name: "spinlock_option_value",       run: test_option_value },
        TestCase { name: "spinlock_counter_increment",  run: test_counter_increment },
        TestCase { name: "spinlock_independent_locks",  run: test_independent_locks },
        TestCase { name: "spinlock_array_data",         run: test_array_data },
    ],
};

fn test_basic_lock_unlock(ctx: &mut TestContext) {
    let lock: Spinlock<u32> = Spinlock::new(0);
    {
        let guard = lock.lock();
        ctx.expect_eq(*guard, 0u32, "initial value through guard");
        // Guard drops here, releasing the lock.
    }
    // Lock must be acquirable again after release.
    let guard2 = lock.lock();
    ctx.expect_eq(*guard2, 0u32, "value after re-acquire");
}

fn test_data_access(ctx: &mut TestContext) {
    let lock: Spinlock<u64> = Spinlock::new(0xCAFEBABE_DEADBEEFu64);
    let guard = lock.lock();
    ctx.expect_eq(*guard, 0xCAFEBABE_DEADBEEFu64, "read stored value");
}

fn test_mutation(ctx: &mut TestContext) {
    let lock: Spinlock<u32> = Spinlock::new(0);
    {
        let mut guard = lock.lock();
        *guard = 42;
    }
    let guard = lock.lock();
    ctx.expect_eq(*guard, 42u32, "mutation persists after re-acquire");
}

fn test_bool_value(ctx: &mut TestContext) {
    let lock: Spinlock<bool> = Spinlock::new(false);
    {
        let mut g = lock.lock();
        ctx.expect_false(*g, "initial false");
        *g = true;
    }
    let g = lock.lock();
    ctx.expect_true(*g, "set to true");
}

fn test_option_value(ctx: &mut TestContext) {
    let lock: Spinlock<Option<u32>> = Spinlock::new(None);
    {
        let mut g = lock.lock();
        ctx.expect_none(*g, "initial None");
        *g = Some(99);
    }
    let g = lock.lock();
    ctx.expect_eq(*g, Some(99u32), "set to Some(99)");
}

fn test_counter_increment(ctx: &mut TestContext) {
    let counter: Spinlock<u64> = Spinlock::new(0);
    for _ in 0..100 {
        let mut g = counter.lock();
        *g += 1;
    }
    let g = counter.lock();
    ctx.expect_eq(*g, 100u64, "counter after 100 increments");
}

fn test_independent_locks(ctx: &mut TestContext) {
    // Two separate locks must be acquirable and modifiable independently.
    let lock_a: Spinlock<u32> = Spinlock::new(1);
    let lock_b: Spinlock<u32> = Spinlock::new(2);

    {
        let mut a = lock_a.lock();
        *a = 10;
    }
    {
        let mut b = lock_b.lock();
        *b = 20;
    }

    ctx.expect_eq(*lock_a.lock(), 10u32, "lock_a independent");
    ctx.expect_eq(*lock_b.lock(), 20u32, "lock_b independent");
}

fn test_array_data(ctx: &mut TestContext) {
    let lock: Spinlock<[u8; 8]> = Spinlock::new([0u8; 8]);
    {
        let mut g = lock.lock();
        for (i, b) in g.iter_mut().enumerate() {
            *b = i as u8 * 3;
        }
    }
    let g = lock.lock();
    ctx.expect_eq(g[0], 0u8, "array[0]");
    ctx.expect_eq(g[1], 3u8, "array[1]");
    ctx.expect_eq(g[7], 21u8, "array[7]");
}
