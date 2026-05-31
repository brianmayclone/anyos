//! Boundary tests for the user-memory access primitives that every syscall
//! handler relies on (`is_valid_user_ptr`, `is_user_range_accessible`,
//! `copy_user_bytes`, `copy_to_user_bytes`).
//!
//! These lock the rule that NULL, kernel-space, overflowing, zero-length and
//! over-long pointers are rejected **without** dereferencing — the kernel trust
//! boundary. Only the deterministic rejection paths are exercised here: every
//! case short-circuits in `is_valid_user_ptr`/length checks before any page
//! probing, so the suite is safe to run before the scheduler exists (unit
//! phase) and never depends on demand-paging.

use crate::kunit::{TestCase, TestContext, TestSuite};
use crate::syscall::handlers::helpers::{
    copy_to_user_bytes, copy_user_bytes, is_user_range_accessible, is_valid_user_ptr,
};

/// First address at or above the non-canonical / kernel half (user space is
/// strictly below this).
const USER_TOP: u64 = 0x0000_8000_0000_0000;
/// A canonical higher-half (kernel) address.
const KERNEL_HALF: u64 = 0xFFFF_8000_0000_0000;

pub static SUITE: TestSuite = TestSuite {
    name: "syscall::user_access",
    cases: &[
        TestCase {
            name: "is_valid_user_ptr_rejects_bad",
            run: test_valid_user_ptr,
        },
        TestCase {
            name: "range_accessible_rejects_bad",
            run: test_range_accessible,
        },
        TestCase {
            name: "copy_user_bytes_rejects_bad",
            run: test_copy_user_bytes,
        },
        TestCase {
            name: "copy_to_user_bytes_rejects_bad",
            run: test_copy_to_user_bytes,
        },
    ],
};

fn test_valid_user_ptr(ctx: &mut TestContext) {
    ctx.expect_false(is_valid_user_ptr(0, 1), "NULL is rejected");
    ctx.expect_false(is_valid_user_ptr(KERNEL_HALF, 1), "kernel-half address rejected");
    ctx.expect_false(is_valid_user_ptr(USER_TOP, 1), "first non-user address rejected");
    ctx.expect_false(
        is_valid_user_ptr(USER_TOP - 1, 8),
        "range straddling the user/kernel boundary rejected",
    );
    ctx.expect_false(is_valid_user_ptr(u64::MAX, 1), "max address (overflow) rejected");
    ctx.expect_false(is_valid_user_ptr(0x1000, u64::MAX), "length overflow rejected");
    // A small range fully inside user space passes the range check.
    ctx.expect_true(is_valid_user_ptr(0x1000, 0x1000), "small in-range pointer accepted");
}

fn test_range_accessible(ctx: &mut TestContext) {
    // All of these reject in is_valid_user_ptr / the length guard, i.e. before
    // any page table probing — so no scheduler / demand-paging dependency.
    ctx.expect_false(is_user_range_accessible(0x4000, 0), "zero-length range rejected");
    ctx.expect_false(is_user_range_accessible(0, 8), "NULL range rejected");
    ctx.expect_false(is_user_range_accessible(KERNEL_HALF, 8), "kernel-half range rejected");
    ctx.expect_false(
        is_user_range_accessible(USER_TOP - 4, 8),
        "range crossing into kernel half rejected",
    );
    ctx.expect_false(is_user_range_accessible(0x2000, u64::MAX), "length overflow rejected");
}

fn test_copy_user_bytes(ctx: &mut TestContext) {
    ctx.expect_true(copy_user_bytes(0, 8, 8).is_none(), "copy_user_bytes(NULL) -> None");
    ctx.expect_true(
        copy_user_bytes(KERNEL_HALF, 8, 8).is_none(),
        "copy_user_bytes(kernel) -> None",
    );
    ctx.expect_true(copy_user_bytes(0x4000, 0, 8).is_none(), "copy_user_bytes(len=0) -> None");
    ctx.expect_true(
        copy_user_bytes(0x4000, 16, 8).is_none(),
        "copy_user_bytes(len>max) -> None",
    );
}

fn test_copy_to_user_bytes(ctx: &mut TestContext) {
    let data = [0xABu8; 8];
    ctx.expect_false(copy_to_user_bytes(0, &data, 8), "copy_to_user_bytes(NULL) -> false");
    ctx.expect_false(
        copy_to_user_bytes(KERNEL_HALF, &data, 8),
        "copy_to_user_bytes(kernel) -> false",
    );
    ctx.expect_false(
        copy_to_user_bytes(0x4000, &data, 4),
        "copy_to_user_bytes(len>max) -> false",
    );
    let empty: [u8; 0] = [];
    ctx.expect_false(
        copy_to_user_bytes(0x4000, &empty, 8),
        "copy_to_user_bytes(empty) -> false",
    );
}
