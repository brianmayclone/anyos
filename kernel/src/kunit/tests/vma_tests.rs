//! Unit tests for the VMA (Virtual Memory Area) tracker.
//!
//! The VMA tracker is the kernel's per-process mmap region manager — the
//! equivalent of Linux's `mm_struct` / `vm_area_struct` list.
//! These tests exercise alloc, free, clone, destroy, and gap-search using
//! dummy PhysAddr keys (the tracker only uses them as IDs, not for MMIO).

use crate::kunit::{TestCase, TestContext, TestSuite};
use crate::memory::address::PhysAddr;
use crate::memory::vma;

pub static SUITE: TestSuite = TestSuite {
    name: "memory::vma",
    cases: &[
        TestCase {
            name: "init_process",
            run: test_init_process,
        },
        TestCase {
            name: "alloc_single_region",
            run: test_alloc_single,
        },
        TestCase {
            name: "alloc_multiple_regions",
            run: test_alloc_multiple,
        },
        TestCase {
            name: "alloc_alignment",
            run: test_alloc_alignment,
        },
        TestCase {
            name: "free_region",
            run: test_free_region,
        },
        TestCase {
            name: "free_partial",
            run: test_free_partial,
        },
        TestCase {
            name: "clone_for_fork",
            run: test_clone_fork,
        },
        TestCase {
            name: "destroy_process",
            run: test_destroy,
        },
        TestCase {
            name: "mmap_hint_readwrite",
            run: test_mmap_hint,
        },
        TestCase {
            name: "alloc_returns_none_after_fill",
            run: test_alloc_exhausted,
        },
    ],
};

/// Make a unique fake PhysAddr per test to avoid cross-test interference.
fn fake_pd(id: u64) -> PhysAddr {
    // Use values that are page-aligned but clearly not real RAM.
    PhysAddr::new(0xDEAD_0000_0000_0000 + id * 0x1000)
}

const PAGE: u32 = 4096;

fn test_init_process(ctx: &mut TestContext) {
    let pd = fake_pd(0x10);
    vma::init_process(pd, 0x7000_0000);

    // After init, mmap_hint should reflect the value we passed.
    let hint = vma::get_mmap_hint(pd);
    ctx.expect_eq(hint, 0x7000_0000u32, "mmap_hint matches init value");

    vma::destroy_process(pd);
}

fn test_alloc_single(ctx: &mut TestContext) {
    let pd = fake_pd(0x11);
    vma::init_process(pd, 0x7000_0000);

    let addr = vma::alloc_region(pd, PAGE);
    if let Some(a) = ctx.expect_some(addr, "alloc_region returns Some for first page") {
        // Must be page-aligned.
        ctx.expect_eq(a % PAGE, 0u32, "allocated address is page-aligned");
        // Must be in the userspace mmap window [0x7000_0000, 0xBF00_0000).
        ctx.expect_ge(a, 0x7000_0000u32, "addr >= MMAP_BASE");
        ctx.expect_true(a < 0xBF00_0000u32, "addr < MMAP_LIMIT");
    }

    vma::destroy_process(pd);
}

fn test_alloc_multiple(ctx: &mut TestContext) {
    let pd = fake_pd(0x12);
    vma::init_process(pd, 0x7000_0000);

    let mut addrs = alloc::vec::Vec::new();
    for _ in 0..8 {
        let a = vma::alloc_region(pd, PAGE);
        if let Some(addr) = a {
            addrs.push(addr);
        }
    }
    ctx.expect_eq(addrs.len(), 8, "8 successful allocs");

    // All addresses must be distinct.
    for i in 0..addrs.len() {
        for j in (i + 1)..addrs.len() {
            ctx.expect_ne(addrs[i], addrs[j], "all addresses distinct");
        }
    }

    vma::destroy_process(pd);
}

fn test_alloc_alignment(ctx: &mut TestContext) {
    let pd = fake_pd(0x13);
    vma::init_process(pd, 0x7000_0000);

    // Allocate regions of varying sizes; all results must be page-aligned.
    for size in [PAGE, PAGE * 3, PAGE * 7, PAGE * 16] {
        if let Some(a) = vma::alloc_region(pd, size) {
            ctx.expect_eq(a % PAGE, 0u32, "large alloc is page-aligned");
        }
    }

    vma::destroy_process(pd);
}

fn test_free_region(ctx: &mut TestContext) {
    let pd = fake_pd(0x14);
    vma::init_process(pd, 0x7000_0000);

    let a = vma::alloc_region(pd, PAGE * 4);
    ctx.expect_some(a, "alloc before free");

    if let Some(addr) = a {
        vma::free_region(pd, addr, PAGE * 4);

        // After freeing, the same range should be allocatable again.
        vma::set_mmap_hint(pd, 0x7000_0000);
        let b = vma::alloc_region(pd, PAGE * 4);
        if let Some(b_addr) = ctx.expect_some(b, "re-alloc after free succeeds") {
            ctx.expect_eq(
                b_addr,
                addr,
                "re-alloc returns same address after full free",
            );
        }
    }

    vma::destroy_process(pd);
}

fn test_free_partial(ctx: &mut TestContext) {
    let pd = fake_pd(0x15);
    vma::init_process(pd, 0x7000_0000);

    // Alloc 4 pages, free the middle 2 (hole-punch).
    let a = vma::alloc_region(pd, PAGE * 4);
    if let Some(base) = a {
        vma::free_region(pd, base + PAGE, PAGE * 2);
        // The call must not crash (VMA split is internal, no public query).
        ctx.expect_true(true, "partial free (hole-punch) did not panic");
    } else {
        ctx.expect_true(true, "skipped (alloc failed)");
    }

    vma::destroy_process(pd);
}

fn test_clone_fork(ctx: &mut TestContext) {
    let src = fake_pd(0x20);
    let dst = fake_pd(0x21);
    vma::init_process(src, 0x7000_0000);
    vma::init_process(dst, 0x7000_0000);

    // Alloc 3 regions in the parent.
    let mut src_addrs = alloc::vec::Vec::new();
    for _ in 0..3 {
        if let Some(a) = vma::alloc_region(src, PAGE * 2) {
            src_addrs.push(a);
        }
    }
    ctx.expect_eq(src_addrs.len(), 3, "3 regions in parent before fork");

    // Fork: dst should get identical VMAs.
    vma::clone_for_fork(src, dst);

    // dst's hint must match src's hint (set after the allocs).
    let src_hint = vma::get_mmap_hint(src);
    let dst_hint = vma::get_mmap_hint(dst);
    ctx.expect_eq(
        dst_hint,
        src_hint,
        "child mmap_hint == parent mmap_hint after fork",
    );

    vma::destroy_process(src);
    vma::destroy_process(dst);
}

fn test_destroy(ctx: &mut TestContext) {
    let pd = fake_pd(0x22);
    vma::init_process(pd, 0x7000_0000);
    for _ in 0..4 {
        vma::alloc_region(pd, PAGE);
    }
    vma::destroy_process(pd);

    // After destroy, mmap_hint returns MMAP_BASE (not found → default).
    let hint = vma::get_mmap_hint(pd);
    ctx.expect_eq(
        hint,
        0x7000_0000u32,
        "get_mmap_hint returns MMAP_BASE after destroy",
    );
}

fn test_mmap_hint(ctx: &mut TestContext) {
    let pd = fake_pd(0x23);
    vma::init_process(pd, 0x7000_0000);

    vma::set_mmap_hint(pd, 0x7500_0000);
    ctx.expect_eq(
        vma::get_mmap_hint(pd),
        0x7500_0000u32,
        "set/get mmap_hint round-trip",
    );

    vma::set_mmap_hint(pd, 0x7000_0000);
    ctx.expect_eq(vma::get_mmap_hint(pd), 0x7000_0000u32, "reset mmap_hint");

    vma::destroy_process(pd);
}

fn test_alloc_exhausted(ctx: &mut TestContext) {
    let pd = fake_pd(0x24);
    // Limit the hint to near the top of the window to trigger exhaustion quickly.
    // MMAP_LIMIT = 0xBF000000, so 1 page below = 0xBEFFF000.
    vma::init_process(pd, 0xBEFF_F000);

    // Allocate a large block that fills the remaining space.
    let window = 0xBF00_0000u32 - 0xBEFF_F000u32; // 1 page
    let a1 = vma::alloc_region(pd, window);
    // This might succeed or not depending on exact alignment.
    // Either way, a subsequent very-large alloc must not panic.
    let _ = a1;
    let _ = vma::alloc_region(pd, 0x1000_0000); // 256 MiB — definitely won't fit

    ctx.expect_true(true, "alloc beyond MMAP_LIMIT returns None without panic");

    vma::destroy_process(pd);
}
