//! GPU Benchmark 1 — Fill Rate.
//!
//! Draws randomly positioned opaque rectangles as fast as possible for
//! [`GPU_TEST_MS`] milliseconds. Returns total rectangles filled.

use alloc::vec;
use libanyui_client as anyui;
use super::GPU_TEST_MS;

/// Rectangle fill throughput benchmark (onscreen via Canvas API, offscreen via raw buffer).
pub fn bench_gpu_fill_rect(canvas: &anyui::Canvas, offscreen: bool) -> u64 {
    let w = canvas.get_stride();
    let h = canvas.get_height();
    if w == 0 || h == 0 { return 0; }

    if offscreen {
        let buf_size = (w * h) as usize;
        let mut buf = vec![0u32; buf_size];
        let stride = w as usize;
        let mut count: u64 = 0;
        let start = anyos_std::sys::uptime_ms();
        let mut seed: u32 = 1;
        while anyos_std::sys::uptime_ms().wrapping_sub(start) < GPU_TEST_MS {
            for _ in 0..100 {
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let rx = ((seed >> 16) % w) as usize;
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let ry = ((seed >> 16) % h) as usize;
                let rw = (((seed >> 8) & 0x3F) + 4) as usize;
                let rh = (((seed >> 4) & 0x3F) + 4) as usize;
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let color = seed | 0xFF000000;
                let x_end = (rx + rw).min(w as usize);
                let y_end = (ry + rh).min(h as usize);
                for y in ry..y_end {
                    for x in rx..x_end {
                        buf[y * stride + x] = color;
                    }
                }
                count += 1;
            }
        }
        core::hint::black_box(&buf);
        count
    } else {
        let mut count: u64 = 0;
        let start = anyos_std::sys::uptime_ms();
        let mut seed: u32 = 1;
        while anyos_std::sys::uptime_ms().wrapping_sub(start) < GPU_TEST_MS {
            for _ in 0..50 {
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let rx = (seed >> 16) % w;
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let ry = (seed >> 16) % h;
                let rw = ((seed >> 8) & 0x3F) + 4;
                let rh = ((seed >> 4) & 0x3F) + 4;
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let color = seed | 0xFF000000;
                canvas.fill_rect(rx as i32, ry as i32, rw, rh, color);
                count += 1;
            }
        }
        count
    }
}
