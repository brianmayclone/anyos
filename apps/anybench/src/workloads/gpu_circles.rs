//! GPU Benchmark 4 — Circle Rendering.
//!
//! Draws filled circles at random positions for [`GPU_TEST_MS`] milliseconds.
//! Returns total circles drawn.

use alloc::vec;
use libanyui_client as anyui;
use super::{GPU_TEST_MS, draw_filled_circle};

/// Filled circle rendering throughput benchmark.
pub fn bench_gpu_circles(canvas: &anyui::Canvas, offscreen: bool) -> u64 {
    let w = canvas.get_stride();
    let h = canvas.get_height();
    if w == 0 || h == 0 { return 0; }

    if offscreen {
        let buf_size = (w * h) as usize;
        let mut buf = vec![0u32; buf_size];
        let stride = w as usize;
        let mut count: u64 = 0;
        let start = anyos_std::sys::uptime_ms();
        let mut seed: u32 = 42;
        while anyos_std::sys::uptime_ms().wrapping_sub(start) < GPU_TEST_MS {
            for _ in 0..20 {
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let cx = ((seed >> 16) % w) as i32;
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let cy = ((seed >> 16) % h) as i32;
                let r = (((seed >> 8) & 0x1F) + 5) as i32;
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let color = seed | 0xFF000000;
                draw_filled_circle(&mut buf, stride, w as i32, h as i32, cx, cy, r, color);
                count += 1;
            }
        }
        core::hint::black_box(&buf);
        count
    } else {
        let mut count: u64 = 0;
        let start = anyos_std::sys::uptime_ms();
        let mut seed: u32 = 42;
        while anyos_std::sys::uptime_ms().wrapping_sub(start) < GPU_TEST_MS {
            for _ in 0..20 {
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let cx = ((seed >> 16) % w) as i32;
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let cy = ((seed >> 16) % h) as i32;
                let r = (((seed >> 8) & 0x1F) + 5) as i32;
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let color = seed | 0xFF000000;
                canvas.fill_circle(cx, cy, r, color);
                count += 1;
            }
        }
        count
    }
}
