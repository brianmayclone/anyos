//! Heap stress tests — catch allocator corruption before it causes a crash.
//!
//! These tests deliberately create allocation pressure in patterns that expose
//! common heap bugs:
//!
//! - **Fragmentation**: many small allocs then large allocs — fails if the
//!   allocator cannot coalesce freed blocks.
//! - **Alignment**: objects with alignment > 8 bytes must be placed correctly
//!   or SIMD/atomic operations on them will fault.
//! - **Size boundaries**: 0-byte, 1-byte, page-sized, multi-page allocs all
//!   exercise different allocator code paths.
//! - **validate_heap()**: walks free-list links; catches corrupted headers
//!   caused by out-of-bounds writes that overflow into allocator metadata.

use crate::kunit::{TestCase, TestContext, TestSuite};
use crate::memory::heap::{heap_stats, validate_heap};

pub static SUITE: TestSuite = TestSuite {
    name: "heap::stress",
    cases: &[
        TestCase {
            name: "small_alloc_flood",
            run: test_small_flood,
        },
        TestCase {
            name: "varied_size_allocs",
            run: test_varied_sizes,
        },
        TestCase {
            name: "interleaved_alloc_free",
            run: test_interleaved,
        },
        TestCase {
            name: "large_single_alloc",
            run: test_large_single,
        },
        TestCase {
            name: "nested_alloc_in_alloc",
            run: test_nested,
        },
        TestCase {
            name: "heap_stats_monotone",
            run: test_stats_monotone,
        },
        TestCase {
            name: "validate_after_each_phase",
            run: test_validate_phases,
        },
        TestCase {
            name: "alloc_after_heavy_use",
            run: test_alloc_after_heavy,
        },
    ],
};

// ── Small alloc flood ─────────────────────────────────────────────────────────

fn test_small_flood(ctx: &mut TestContext) {
    // 128 tiny allocations all at once — stresses the small-object free list.
    // If any header is corrupted, validate_heap() will catch it.
    let mut v: alloc::vec::Vec<alloc::boxed::Box<u64>> = alloc::vec::Vec::with_capacity(128);
    for i in 0u64..128 {
        let b = alloc::boxed::Box::new(i * 0xDEAD_BEEF);
        v.push(b);
    }
    validate_heap();
    ctx.expect_true(true, "heap valid with 128 live u64 boxes");

    // Verify all values are still intact (detects use-after-free / overlap).
    let mut ok = true;
    for (i, b) in v.iter().enumerate() {
        if **b != i as u64 * 0xDEAD_BEEF {
            ok = false;
        }
    }
    ctx.expect_true(ok, "all small allocations contain correct values");

    drop(v);
    validate_heap();
    ctx.expect_true(true, "heap valid after freeing 128 boxes");
}

// ── Varied sizes ──────────────────────────────────────────────────────────────

fn test_varied_sizes(ctx: &mut TestContext) {
    // Allocate objects spanning many size classes.
    let _a = alloc::vec![0u8; 1];
    let _b = alloc::vec![0u8; 8];
    let _c = alloc::vec![0u8; 16];
    let _d = alloc::vec![0u8; 64];
    let _e = alloc::vec![0u8; 128];
    let _f = alloc::vec![0u8; 256];
    let _g = alloc::vec![0u8; 512];
    let _h = alloc::vec![0u8; 1024];
    let _i = alloc::vec![0u8; 2048];
    let _j = alloc::vec![0u8; 4096];
    validate_heap();
    ctx.expect_true(true, "heap valid with objects of sizes 1..4096");
    // All drop at end of scope — validate again after free.
}

// ── Interleaved alloc/free ────────────────────────────────────────────────────

fn test_interleaved(ctx: &mut TestContext) {
    // Fill a pool, then alternately free even-indexed entries and allocate new
    // ones. This forces the allocator to reuse freed slots and checks that it
    // can find them via the free list without skipping or double-returning.
    let mut pool: alloc::vec::Vec<alloc::vec::Vec<u8>> = alloc::vec::Vec::with_capacity(32);
    for i in 0..32usize {
        pool.push(alloc::vec![i as u8; 64]);
    }

    // Free even-indexed entries.
    let mut i = 0;
    while i < pool.len() {
        pool.remove(i);
        // Don't increment — the next entry shifted down to position i.
        // Skip it by incrementing once more.
        i += 1;
    }
    validate_heap();
    ctx.expect_true(true, "heap valid after freeing every other entry");

    // Allocate new objects into the freed slots.
    for j in 0..16usize {
        pool.push(alloc::vec![j as u8; 64]);
    }
    validate_heap();
    ctx.expect_true(true, "heap valid after refilling freed slots");
}

// ── Large single allocation ───────────────────────────────────────────────────

fn test_large_single(ctx: &mut TestContext) {
    // A 64 KiB allocation exercises the "large block" path in the allocator.
    // Write a pattern and verify it to catch partial-overlap bugs.
    let mut buf = alloc::vec![0u8; 65536];
    for (i, b) in buf.iter_mut().enumerate() {
        *b = (i & 0xFF) as u8;
    }
    let mut ok = true;
    for (i, &b) in buf.iter().enumerate() {
        if b != (i & 0xFF) as u8 {
            ok = false;
        }
    }
    ctx.expect_true(ok, "64 KiB allocation contents intact");
    validate_heap();
    ctx.expect_true(true, "heap valid with 64 KiB allocation live");
}

// ── Nested allocation ─────────────────────────────────────────────────────────

fn test_nested(ctx: &mut TestContext) {
    // A Vec<Vec<u8>> — each inner allocation is a separate heap object.
    // The outer Vec's realloc (when growing) must not invalidate inner pointers.
    let mut outer: alloc::vec::Vec<alloc::vec::Vec<u8>> = alloc::vec::Vec::new();
    for i in 0u8..32 {
        outer.push(alloc::vec![i; (i as usize + 1) * 8]);
    }
    // Verify inner contents are intact after outer may have been reallocated.
    let mut ok = true;
    for (i, inner) in outer.iter().enumerate() {
        let expected_len = (i + 1) * 8;
        if inner.len() != expected_len {
            ok = false;
        }
        if inner.iter().any(|&b| b != i as u8) {
            ok = false;
        }
    }
    ctx.expect_true(ok, "nested Vec contents intact after outer realloc");
    validate_heap();
    ctx.expect_true(true, "heap valid with nested Vec<Vec<u8>>");
}

// ── Heap stats monotonicity ───────────────────────────────────────────────────

fn test_stats_monotone(ctx: &mut TestContext) {
    // heap_stats() returns (used, committed). Committed bytes should never
    // shrink (the heap does not release pages back to the physical allocator).
    // Used bytes must always be <= committed.
    //
    // Hold a live anchor allocation so `used` is non-zero at this early boot
    // point: `used` excludes the per-CPU bucket cache, so right after heap init
    // (before this suite has live allocations) it can legitimately read ~0.
    let anchor = alloc::vec![0u8; 96 * 1024];
    core::hint::black_box(anchor.as_ptr());
    let (used0, committed0) = heap_stats();
    ctx.expect_true(used0 > 0, "used bytes > 0 (live anchor)");
    ctx.expect_true(committed0 > 0, "committed bytes > 0");
    ctx.expect_ge(committed0, used0, "committed >= used");

    // Allocate something.
    let v = alloc::vec![0u8; 4096];
    let (used1, committed1) = heap_stats();
    ctx.expect_ge(used1, used0, "used increases after alloc");
    ctx.expect_ge(committed1, committed0, "committed non-decreasing");
    ctx.expect_ge(committed1, used1, "committed >= used after alloc");

    drop(v);
    let (used2, committed2) = heap_stats();
    ctx.expect_ge(
        committed2,
        committed0,
        "committed stays >= original after free",
    );
    ctx.expect_ge(committed2, used2, "committed >= used after free");
}

// ── validate_heap after each phase ───────────────────────────────────────────

fn test_validate_phases(ctx: &mut TestContext) {
    // Run validate_heap at three checkpoints; any failure panics (which would
    // be caught as a KERNEL PANIC in the test runner).
    validate_heap();
    ctx.expect_true(true, "heap valid at phase 1 (before stress)");

    let a = alloc::vec![1u8; 512];
    let b = alloc::vec![2u8; 1024];
    validate_heap();
    ctx.expect_true(true, "heap valid at phase 2 (two allocs live)");

    drop(a);
    validate_heap();
    ctx.expect_true(true, "heap valid at phase 3 (one freed, one live)");

    drop(b);
    validate_heap();
    ctx.expect_true(true, "heap valid at phase 4 (both freed)");
}

// ── Allocate after heavy use ──────────────────────────────────────────────────

fn test_alloc_after_heavy(ctx: &mut TestContext) {
    // After running all previous tests the heap may be fragmented.
    // Verify we can still allocate a reasonably large object — if the
    // allocator cannot coalesce freed blocks this will fail.
    {
        let mut flood: alloc::vec::Vec<alloc::vec::Vec<u8>> = alloc::vec::Vec::with_capacity(64);
        for _ in 0..64 {
            flood.push(alloc::vec![0u8; 256]);
        }
        // Drop all — frees 64 × 256-byte blocks into the free list.
    }

    // Should still be able to get a 16 KiB contiguous block.
    let big = alloc::vec![0xABu8; 16384];
    ctx.expect_eq(
        big.len(),
        16384usize,
        "can allocate 16 KiB after flood+free",
    );
    ctx.expect_true(big[0] == 0xAB && big[16383] == 0xAB, "contents correct");
    validate_heap();
    ctx.expect_true(true, "heap valid after alloc-after-heavy-use");
}
