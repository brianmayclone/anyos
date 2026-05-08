//! CPU Benchmark 8 — Image Filters.
//!
//! Applies chained 3x3 convolution filters to a synthetic grayscale image for
//! [`CPU_TEST_MS`] milliseconds. Returns processed output pixels.

use super::CPU_TEST_MS;
use alloc::vec;

/// 3x3 image convolution benchmark.
pub fn bench_image_filter() -> u64 {
    const W: usize = 192;
    const H: usize = 192;
    const KERNELS: [[i32; 9]; 3] = [
        [1, 2, 1, 2, 4, 2, 1, 2, 1],
        [0, -1, 0, -1, 5, -1, 0, -1, 0],
        [-1, -1, -1, -1, 8, -1, -1, -1, -1],
    ];
    const DIVS: [i32; 3] = [16, 1, 1];

    let mut src = vec![0u16; W * H];
    let mut dst = vec![0u16; W * H];
    for y in 0..H {
        for x in 0..W {
            let v = ((x * 13) ^ (y * 29) ^ ((x * y) >> 2)) & 0xFF;
            src[y * W + x] = v as u16;
        }
    }

    let mut pixels = 0u64;
    let mut pass = 0usize;
    let start = anyos_std::sys::uptime_ms();
    while anyos_std::sys::uptime_ms().wrapping_sub(start) < CPU_TEST_MS {
        let kernel = &KERNELS[pass % KERNELS.len()];
        let div = DIVS[pass % DIVS.len()];
        for y in 1..H - 1 {
            for x in 1..W - 1 {
                let base = y * W + x;
                let mut acc = 0i32;
                acc += src[base - W - 1] as i32 * kernel[0];
                acc += src[base - W] as i32 * kernel[1];
                acc += src[base - W + 1] as i32 * kernel[2];
                acc += src[base - 1] as i32 * kernel[3];
                acc += src[base] as i32 * kernel[4];
                acc += src[base + 1] as i32 * kernel[5];
                acc += src[base + W - 1] as i32 * kernel[6];
                acc += src[base + W] as i32 * kernel[7];
                acc += src[base + W + 1] as i32 * kernel[8];
                let v = if div == 1 { acc } else { acc / div };
                dst[base] = v.max(0).min(255) as u16;
            }
        }
        core::mem::swap(&mut src, &mut dst);
        pixels += ((W - 2) * (H - 2)) as u64;
        pass += 1;
    }

    core::hint::black_box(&src);
    pixels
}
