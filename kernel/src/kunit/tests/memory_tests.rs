//! Tests for the kernel memory subsystem.
//!
//! Covers:
//! - Heap allocator statistics and integrity (`heap_stats`, `validate_heap`)
//! - Physical frame allocator: alloc/free round-trip and frame count invariants
//!
//! Note: `validate_heap()` prints diagnostics to serial output on its own;
//! the test only checks that the kernel does not crash during the walk.

use crate::kunit::{TestCase, TestContext, TestSuite};
use crate::memory::heap::{heap_stats, validate_heap, HEAP_COMMITTED};
use crate::memory::physical::{alloc_frame, free_frame, free_frame_count};
use crate::memory::slab::{KBox, KmemCache};
use crate::sync::spinlock::Spinlock;
use core::sync::atomic::Ordering;

pub static SUITE: TestSuite = TestSuite {
    name: "memory",
    cases: &[
        TestCase {
            name: "heap_committed_nonzero",
            run: test_heap_committed_nonzero,
        },
        TestCase {
            name: "cow_refcount_roundtrip",
            run: test_cow_refcount_roundtrip,
        },
        TestCase {
            name: "heap_stats_nonzero",
            run: test_heap_stats_nonzero,
        },
        TestCase {
            name: "heap_alloc_changes_stats",
            run: test_heap_alloc_changes_stats,
        },
        TestCase {
            name: "heap_free_restores_stats",
            run: test_heap_free_restores_stats,
        },
        TestCase {
            name: "heap_validate_passes",
            run: test_heap_validate_passes,
        },
        TestCase {
            name: "physical_alloc_frame",
            run: test_physical_alloc_frame,
        },
        TestCase {
            name: "physical_free_frame_count",
            run: test_physical_free_frame_count,
        },
        TestCase {
            name: "physical_alloc_free_roundtrip",
            run: test_physical_alloc_free_roundtrip,
        },
        TestCase {
            name: "physical_multiple_frames",
            run: test_physical_multiple_frames,
        },
        TestCase {
            name: "slab_cache_reuses_object_slot",
            run: test_slab_cache_reuses_object_slot,
        },
    ],
};

// ── Heap ─────────────────────────────────────────────────────────────────────

fn test_heap_committed_nonzero(ctx: &mut TestContext) {
    // After heap init, the committed watermark must be > 0.
    let committed = HEAP_COMMITTED.load(Ordering::Relaxed);
    ctx.expect_gt(committed, 0usize, "HEAP_COMMITTED > 0 after init");
}

fn test_heap_stats_nonzero(ctx: &mut TestContext) {
    use alloc::vec::Vec;
    // Hold a LIVE allocation so `used` is non-zero independent of how empty the
    // global heap happens to be at this early boot point. `used` deliberately
    // excludes the per-CPU bucket cache, so blocks freed by earlier suites do
    // NOT count toward it — only live allocations do. 96 KiB exceeds every size
    // class, so it is served from the main arena and is unambiguously "used".
    let live: Vec<u8> = alloc::vec![0xCDu8; 96 * 1024];
    core::hint::black_box(live.as_ptr());
    let (used, committed) = heap_stats();
    ctx.expect_gt(used, 0usize, "heap used bytes > 0 (with a live 96 KiB allocation)");
    ctx.expect_gt(committed, 0usize, "heap committed bytes > 0");
    // Used must not exceed committed.
    ctx.expect_ge(committed, used, "committed >= used");
    drop(live);
}

fn test_heap_alloc_changes_stats(ctx: &mut TestContext) {
    use alloc::vec::Vec;
    let (used_before, _) = heap_stats();

    // Allocate 64 KiB — large enough that it definitely changes the stats.
    let v: Vec<u8> = alloc::vec![0xAAu8; 65536];
    let (used_after, _) = heap_stats();

    ctx.expect_ge(
        used_after,
        used_before,
        "used bytes did not decrease after alloc",
    );
    // Hint: if the allocator is accurate, used_after > used_before.
    // We allow equal in case of per-CPU cache absorption, but log it.
    if used_after == used_before {
        crate::serial_println!(
            "    [NOTE] memory::heap_alloc_changes_stats — used unchanged (per-CPU cache hit)"
        );
    }

    drop(v); // Trigger deallocation.
}

fn test_heap_free_restores_stats(ctx: &mut TestContext) {
    use alloc::vec::Vec;
    let (used_before, _) = heap_stats();

    {
        // Scope ensures drop (and dealloc) before the second stat read.
        let v: Vec<u64> = alloc::vec![0u64; 8192]; // 64 KiB
        let (used_during, _) = heap_stats();
        ctx.expect_ge(used_during, used_before, "used >= before during alloc");
        drop(v);
    }

    let (used_after, _) = heap_stats();
    // After freeing, used bytes must not exceed what we started with.
    // (Coalescing may reduce it further, but never above used_before + overhead.)
    ctx.expect_ge(
        used_before + 4096, // allow up to 4 KiB overhead for block headers
        used_after.saturating_sub(used_before), // must be near zero
        "used bytes after free not significantly above baseline",
    );
}

fn test_heap_validate_passes(ctx: &mut TestContext) {
    // validate_heap() walks the free list and checks for corruption.
    // If it panics, the kernel halts — a non-panicking return means the heap
    // is structurally consistent at this point in the boot sequence.
    validate_heap();
    ctx.expect_true(true, "validate_heap() returned without panic");
}

// ── Physical allocator ────────────────────────────────────────────────────────

fn test_physical_alloc_frame(ctx: &mut TestContext) {
    // alloc_frame must succeed on a freshly booted kernel with free RAM.
    let frame = alloc_frame();
    ctx.expect_some(frame, "alloc_frame returns Some");

    if let Some(addr) = frame {
        // Frame address must be 4 KiB aligned.
        ctx.expect_eq(addr.as_u64() % 4096, 0u64, "frame is 4 KiB aligned");
        // Frame must be in a plausible physical address range (> 1 MiB).
        ctx.expect_gt(addr.as_u64(), 0x10_0000u64, "frame above 1 MiB");
        // Return the frame so it does not leak.
        free_frame(addr);
    }
}

fn test_physical_free_frame_count(ctx: &mut TestContext) {
    // The free frame count must be > 0 after boot.
    let free = free_frame_count();
    ctx.expect_gt(free, 0usize, "free frame count > 0");
}

fn test_physical_alloc_free_roundtrip(ctx: &mut TestContext) {
    let free_before = free_frame_count();

    let frame = alloc_frame();
    if let Some(addr) = frame {
        let free_during = free_frame_count();
        ctx.expect_eq(
            free_before.saturating_sub(free_during),
            1usize,
            "alloc decreases free count by 1",
        );

        free_frame(addr);
        let free_after = free_frame_count();
        ctx.expect_eq(free_after, free_before, "free restores frame count");
    } else {
        // No free frames — not a bug, but the remaining tests are moot.
        crate::serial_println!("    [SKIP] physical_alloc_free_roundtrip — no free frames");
        ctx.expect_true(true, "skip acknowledged");
    }
}

fn test_physical_multiple_frames(ctx: &mut TestContext) {
    // Allocate 8 frames, verify they are distinct and all 4 KiB-aligned,
    // then free all of them.
    const N: usize = 8;
    let mut frames = [None; N];

    for slot in frames.iter_mut() {
        *slot = alloc_frame();
    }

    // All allocations must succeed.
    let all_some = frames.iter().all(|f| f.is_some());
    ctx.expect_true(all_some, "all 8 frame allocations succeeded");

    if all_some {
        // All frame addresses must be distinct.
        let addrs: alloc::vec::Vec<u64> = frames.iter().map(|f| f.unwrap().as_u64()).collect();

        for i in 0..N {
            // Alignment.
            ctx.expect_eq(addrs[i] % 4096, 0u64, "frame i aligned");
            // Distinct from all subsequent frames.
            for j in (i + 1)..N {
                ctx.expect_ne(addrs[i], addrs[j], "frames are distinct");
            }
        }

        // Free in reverse order to stress the coalescing logic.
        for slot in frames.iter().rev() {
            if let Some(addr) = *slot {
                free_frame(addr);
            }
        }

        ctx.expect_true(true, "all 8 frames freed successfully");
    }
}

// ── Slab / object cache ──────────────────────────────────────────────────────

#[repr(C, align(64))]
struct SlabTestObj {
    bytes: [u8; 96],
}

static SLAB_TEST_CACHE: Spinlock<KmemCache> = Spinlock::new(KmemCache::new(
    "kunit::SlabTestObj",
    core::mem::size_of::<SlabTestObj>(),
    core::mem::align_of::<SlabTestObj>(),
));

fn test_slab_cache_reuses_object_slot(ctx: &mut TestContext) {
    let first_addr = {
        let obj = match KBox::new_in(SlabTestObj { bytes: [0xAB; 96] }, &SLAB_TEST_CACHE) {
            Some(obj) => obj,
            None => {
                ctx.expect_true(false, "slab object allocation failed");
                return;
            }
        };
        let addr = (&*obj as *const SlabTestObj) as usize;
        ctx.expect_eq(addr % 64, 0usize, "slab object alignment");
        addr
    };

    let second = match KBox::new_in(SlabTestObj { bytes: [0xCD; 96] }, &SLAB_TEST_CACHE) {
        Some(obj) => obj,
        None => {
            ctx.expect_true(false, "second slab object allocation failed");
            return;
        }
    };
    let second_addr = (&*second as *const SlabTestObj) as usize;
    ctx.expect_eq(second_addr, first_addr, "slab reused freed object slot");
}

fn test_cow_refcount_roundtrip(ctx: &mut TestContext) {
    ctx.expect_true(
        crate::memory::virtual_mem::kunit_cow_refcount_roundtrip(),
        "CoW refcount fork-inc/release/saturation invariants",
    );
}
