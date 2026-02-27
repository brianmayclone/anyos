//! CPU Benchmark 6 — Quicksort.
//!
//! Sorts a 50 000-element pseudo-random array repeatedly for [`CPU_TEST_MS`]
//! milliseconds. Returns cumulative number of elements sorted.

use alloc::vec;
use super::CPU_TEST_MS;

/// Quicksort benchmark on pseudo-random data.
pub fn bench_sort() -> u64 {
    const SIZE: usize = 50_000;
    let mut data = vec![0u32; SIZE];
    let mut total: u64 = 0;
    let mut rep: u32 = 0;
    let start = anyos_std::sys::uptime_ms();
    while anyos_std::sys::uptime_ms().wrapping_sub(start) < CPU_TEST_MS {
        let mut seed: u32 = 42u32.wrapping_add(rep);
        for v in data.iter_mut() {
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
            *v = seed;
        }
        quicksort(&mut data);
        total += SIZE as u64;
        rep = rep.wrapping_add(1);
    }
    core::hint::black_box(&data);
    total
}

/// In-place quicksort (Hoare partition).
fn quicksort(arr: &mut [u32]) {
    if arr.len() <= 1 { return; }
    let pivot = arr[arr.len() / 2];
    let mut lo = 0usize;
    let mut hi = arr.len() - 1;
    while lo <= hi {
        while arr[lo] < pivot { lo += 1; }
        while arr[hi] > pivot {
            if hi == 0 { break; }
            hi -= 1;
        }
        if lo <= hi {
            arr.swap(lo, hi);
            lo += 1;
            if hi == 0 { break; }
            hi -= 1;
        }
    }
    if hi > 0 { quicksort(&mut arr[..=hi]); }
    if lo < arr.len() { quicksort(&mut arr[lo..]); }
}
