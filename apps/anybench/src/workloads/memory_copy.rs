//! CPU Benchmark 3 — Memory Bandwidth.
//!
//! Sequential volatile copy of a 256 KB buffer, repeated for [`CPU_TEST_MS`]
//! milliseconds. Returns total bytes copied.

use alloc::vec;
use super::CPU_TEST_MS;

/// Sequential volatile buffer copy benchmark.
pub fn bench_memory_copy() -> u64 {
    const BUF_SIZE: usize = 256 * 1024; // 256 KB
    let src = vec![0xAAu8; BUF_SIZE];
    let mut dst = vec![0u8; BUF_SIZE];
    let mut total_bytes: u64 = 0;
    let start = anyos_std::sys::uptime_ms();
    while anyos_std::sys::uptime_ms().wrapping_sub(start) < CPU_TEST_MS {
        for i in 0..BUF_SIZE {
            unsafe {
                let s = core::ptr::read_volatile(src.as_ptr().add(i));
                core::ptr::write_volatile(dst.as_mut_ptr().add(i), s);
            }
        }
        total_bytes += BUF_SIZE as u64;
    }
    core::hint::black_box(&dst);
    total_bytes
}
