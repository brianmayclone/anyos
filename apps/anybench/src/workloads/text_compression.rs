//! CPU Benchmark 9 — Text Compression.
//!
//! Uses a compact LZ-style match finder over synthetic text for
//! [`CPU_TEST_MS`] milliseconds. Returns input bytes processed.

use super::CPU_TEST_MS;
use alloc::vec;

/// LZ-style compression benchmark with short-window match search.
pub fn bench_text_compression() -> u64 {
    const SIZE: usize = 64 * 1024;
    const WINDOW: usize = 96;
    const MAX_MATCH: usize = 18;

    let mut data = vec![0u8; SIZE];
    let pattern = b"{\"name\":\"anyOS\",\"kind\":\"benchmark\",\"values\":[13,21,34,55]},";
    for i in 0..SIZE {
        data[i] = pattern[i % pattern.len()] ^ ((i >> 7) as u8 & 0x0F);
    }

    let mut processed = 0u64;
    let mut checksum = 0u32;
    let start = anyos_std::sys::uptime_ms();
    while anyos_std::sys::uptime_ms().wrapping_sub(start) < CPU_TEST_MS {
        let mut i = 0usize;
        while i < SIZE {
            let win_start = i.saturating_sub(WINDOW);
            let mut best_len = 0usize;
            let mut best_dist = 0usize;

            let mut pos = win_start;
            while pos < i {
                let mut len = 0usize;
                while len < MAX_MATCH && i + len < SIZE && data[pos + len] == data[i + len] {
                    len += 1;
                    if pos + len >= i {
                        break;
                    }
                }
                if len > best_len {
                    best_len = len;
                    best_dist = i - pos;
                }
                pos += 1;
            }

            if best_len >= 4 {
                checksum = checksum
                    .wrapping_mul(33)
                    .wrapping_add((best_len as u32) << 8)
                    .wrapping_add(best_dist as u32);
                i += best_len;
            } else {
                checksum = checksum.wrapping_mul(33).wrapping_add(data[i] as u32);
                i += 1;
            }
        }
        processed += SIZE as u64;
        data[(checksum as usize) & (SIZE - 1)] ^= checksum as u8;
    }

    core::hint::black_box((checksum, &data));
    processed
}
