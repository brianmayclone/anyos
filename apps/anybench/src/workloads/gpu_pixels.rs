//! GPU Benchmark 2 — Pixel Throughput.
//!
//! Writes individual pixels at random positions for [`GPU_TEST_MS`]
//! milliseconds. Returns total pixels written.

use alloc::vec;
use libanyui_client as anyui;
use super::GPU_TEST_MS;

/// Individual pixel write throughput benchmark.
pub fn bench_gpu_pixels(canvas: &anyui::Canvas, offscreen: bool) -> u64 {
    let w = canvas.get_stride();
    let h = canvas.get_height();
    if w == 0 || h == 0 { return 0; }

    if offscreen {
        let buf_size = (w * h) as usize;
        let mut buf = vec![0u32; buf_size];
        let stride = w as usize;
        let mut count: u64 = 0;
        let start = anyos_std::sys::uptime_ms();
        let mut seed: u32 = 7;
        while anyos_std::sys::uptime_ms().wrapping_sub(start) < GPU_TEST_MS {
            for _ in 0..500 {
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let x = ((seed >> 16) % w) as usize;
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let y = ((seed >> 16) % h) as usize;
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let color = seed | 0xFF000000;
                buf[y * stride + x] = color;
                count += 1;
            }
        }
        core::hint::black_box(&buf);
        count
    } else {
        let mut count: u64 = 0;
        let start = anyos_std::sys::uptime_ms();
        let mut seed: u32 = 7;
        while anyos_std::sys::uptime_ms().wrapping_sub(start) < GPU_TEST_MS {
            for _ in 0..200 {
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let x = (seed >> 16) % w;
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let y = (seed >> 16) % h;
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let color = seed | 0xFF000000;
                canvas.set_pixel(x as i32, y as i32, color);
                count += 1;
            }
        }
        count
    }
}
