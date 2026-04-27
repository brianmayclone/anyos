//! Tests for the kernel heap allocator — Vec, Box, String, BTreeMap.
//!
//! These tests exercise the global allocator indirectly through standard
//! `alloc` collection types and verify that dynamic memory behaves correctly
//! after `memory::heap::init()`.

use crate::kunit::{TestCase, TestContext, TestSuite};
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

pub static SUITE: TestSuite = TestSuite {
    name: "alloc",
    cases: &[
        TestCase {
            name: "vec_push_and_index",
            run: test_vec_push_and_index,
        },
        TestCase {
            name: "vec_pop",
            run: test_vec_pop,
        },
        TestCase {
            name: "vec_grow_large",
            run: test_vec_grow_large,
        },
        TestCase {
            name: "vec_dedup",
            run: test_vec_dedup,
        },
        TestCase {
            name: "box_alloc_and_deref",
            run: test_box_alloc_and_deref,
        },
        TestCase {
            name: "box_large_value",
            run: test_box_large_value,
        },
        TestCase {
            name: "string_push",
            run: test_string_push,
        },
        TestCase {
            name: "string_to_string",
            run: test_string_to_string,
        },
        TestCase {
            name: "btreemap_insert_get",
            run: test_btreemap_insert_get,
        },
        TestCase {
            name: "btreemap_remove",
            run: test_btreemap_remove,
        },
        TestCase {
            name: "btreemap_ordering",
            run: test_btreemap_ordering,
        },
        TestCase {
            name: "nested_alloc",
            run: test_nested_alloc,
        },
        TestCase {
            name: "alloc_zero_sized",
            run: test_alloc_zero_sized,
        },
    ],
};

// ── Vec ──────────────────────────────────────────────────────────────────────

fn test_vec_push_and_index(ctx: &mut TestContext) {
    let mut v: Vec<u32> = Vec::new();
    ctx.expect_eq(v.len(), 0, "empty vec len");
    ctx.expect_true(v.is_empty(), "empty vec is_empty");

    v.push(10);
    v.push(20);
    v.push(30);

    ctx.expect_eq(v.len(), 3, "len after 3 pushes");
    ctx.expect_eq(v[0], 10, "v[0]");
    ctx.expect_eq(v[1], 20, "v[1]");
    ctx.expect_eq(v[2], 30, "v[2]");
}

fn test_vec_pop(ctx: &mut TestContext) {
    let mut v: Vec<i32> = Vec::new();
    v.push(1);
    v.push(2);
    v.push(3);

    ctx.expect_eq(v.pop(), Some(3), "pop last");
    ctx.expect_eq(v.pop(), Some(2), "pop second");
    ctx.expect_eq(v.len(), 1, "len after two pops");
    ctx.expect_eq(v.pop(), Some(1), "pop first");
    ctx.expect_eq(v.pop(), None, "pop empty");
}

fn test_vec_grow_large(ctx: &mut TestContext) {
    // Allocates enough elements to trigger multiple internal reallocs.
    let mut v: Vec<u64> = Vec::new();
    for i in 0..1024u64 {
        v.push(i * 7);
    }
    ctx.expect_eq(v.len(), 1024, "len after 1024 pushes");
    ctx.expect_eq(v[0], 0, "v[0]");
    ctx.expect_eq(v[511], 511 * 7, "v[511]");
    ctx.expect_eq(v[1023], 1023 * 7, "v[1023]");
}

fn test_vec_dedup(ctx: &mut TestContext) {
    let mut v = alloc::vec![1u32, 1, 2, 3, 3, 3, 4];
    v.dedup();
    ctx.expect_eq(v.len(), 4, "len after dedup");
    ctx.expect_eq(v[0], 1, "dedup[0]");
    ctx.expect_eq(v[3], 4, "dedup[3]");
}

// ── Box ──────────────────────────────────────────────────────────────────────

fn test_box_alloc_and_deref(ctx: &mut TestContext) {
    let b: Box<u64> = Box::new(0xDEAD_BEEF_CAFE_BABEu64);
    ctx.expect_eq(*b, 0xDEAD_BEEF_CAFE_BABEu64, "box deref value");
}

fn test_box_large_value(ctx: &mut TestContext) {
    // 4 KiB value on the heap — exercises non-trivial allocation.
    let arr: Box<[u8; 4096]> = Box::new([0xABu8; 4096]);
    ctx.expect_eq(arr[0], 0xAB, "large box first byte");
    ctx.expect_eq(arr[4095], 0xAB, "large box last byte");
    ctx.expect_eq(arr.len(), 4096, "large box len");
}

// ── String ───────────────────────────────────────────────────────────────────

fn test_string_push(ctx: &mut TestContext) {
    let mut s = String::new();
    ctx.expect_eq(s.len(), 0, "empty string len");

    s.push_str("hello");
    s.push(' ');
    s.push_str("world");

    ctx.expect_eq(s.len(), 11, "string len after pushes");
    ctx.expect_eq(s.as_str(), "hello world", "string content");
}

fn test_string_to_string(ctx: &mut TestContext) {
    let n: u32 = 42;
    // Use alloc's format! equivalent via ToString is not available, but
    // we can verify basic string operations.
    let s = "anyOS".to_string();
    ctx.expect_eq(s.as_str(), "anyOS", "to_string");
    ctx.expect_eq(s.len(), 5, "to_string len");
    ctx.expect_true(s.contains("any"), "string contains");
    ctx.expect_true(s.starts_with("any"), "starts_with");
    ctx.expect_true(s.ends_with("OS"), "ends_with");
    let _ = n; // suppress unused warning
}

// ── BTreeMap ─────────────────────────────────────────────────────────────────

fn test_btreemap_insert_get(ctx: &mut TestContext) {
    let mut map: BTreeMap<u32, u32> = BTreeMap::new();
    ctx.expect_eq(map.len(), 0, "empty map len");

    map.insert(1, 100);
    map.insert(2, 200);
    map.insert(3, 300);

    ctx.expect_eq(map.len(), 3, "map len after 3 inserts");
    ctx.expect_eq(map.get(&1).copied(), Some(100), "get key 1");
    ctx.expect_eq(map.get(&2).copied(), Some(200), "get key 2");
    ctx.expect_eq(map.get(&3).copied(), Some(300), "get key 3");
    ctx.expect_eq(map.get(&99).copied(), None, "get missing key");
}

fn test_btreemap_remove(ctx: &mut TestContext) {
    let mut map: BTreeMap<&str, i32> = BTreeMap::new();
    map.insert("alpha", 1);
    map.insert("beta", 2);
    map.insert("gamma", 3);

    let removed = map.remove("beta");
    ctx.expect_eq(removed, Some(2), "remove returns old value");
    ctx.expect_eq(map.len(), 2, "len after remove");
    ctx.expect_false(map.contains_key("beta"), "removed key absent");
    ctx.expect_true(map.contains_key("alpha"), "other key present");
}

fn test_btreemap_ordering(ctx: &mut TestContext) {
    let mut map: BTreeMap<u32, u32> = BTreeMap::new();
    for i in [5u32, 1, 4, 2, 3] {
        map.insert(i, i * 10);
    }
    // BTreeMap iterates in key order.
    let keys: Vec<u32> = map.keys().copied().collect();
    ctx.expect_eq(
        keys,
        alloc::vec![1u32, 2, 3, 4, 5],
        "btreemap sorted iteration",
    );
}

// ── Edge cases ───────────────────────────────────────────────────────────────

fn test_nested_alloc(ctx: &mut TestContext) {
    // Vec<Vec<u8>> — nested heap allocations.
    let mut outer: Vec<Vec<u8>> = Vec::new();
    for i in 0u8..4 {
        let mut inner: Vec<u8> = Vec::new();
        for j in 0u8..8 {
            inner.push(i * 8 + j);
        }
        outer.push(inner);
    }
    ctx.expect_eq(outer.len(), 4, "outer vec len");
    ctx.expect_eq(outer[0].len(), 8, "inner vec len");
    ctx.expect_eq(outer[0][0], 0u8, "nested[0][0]");
    ctx.expect_eq(outer[3][7], 31u8, "nested[3][7]");
}

fn test_alloc_zero_sized(ctx: &mut TestContext) {
    // Zero-sized types must not trigger allocator calls.
    let v: Vec<()> = alloc::vec![(), (), ()];
    ctx.expect_eq(v.len(), 3, "zst vec len");
}
