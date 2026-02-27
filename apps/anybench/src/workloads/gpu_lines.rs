//! GPU Benchmark 3 — Line Drawing.
//!
//! Draws random lines for [`GPU_TEST_MS`] milliseconds using Bresenham's
//! algorithm (offscreen) or the Canvas API (onscreen). Returns total lines drawn.

use alloc::vec;
use libanyui_client as anyui;
use super::{GPU_TEST_MS, draw_line_bresenham};

/// Line rendering throughput benchmark.
pub fn bench_gpu_lines(canvas: &anyui::Canvas, offscreen: bool) -> u64 {
    let w = canvas.get_stride();
    let h = canvas.get_height();
    if w == 0 || h == 0 { return 0; }

    if offscreen {
        let buf_size = (w * h) as usize;
        let mut buf = vec![0u32; buf_size];
        let stride = w as usize;
        let mut count: u64 = 0;
        let start = anyos_std::sys::uptime_ms();
        let mut seed: u32 = 13;
        while anyos_std::sys::uptime_ms().wrapping_sub(start) < GPU_TEST_MS {
            for _ in 0..50 {
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let x0 = ((seed >> 16) % w) as i32;
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let y0 = ((seed >> 16) % h) as i32;
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let x1 = ((seed >> 16) % w) as i32;
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let y1 = ((seed >> 16) % h) as i32;
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let color = seed | 0xFF000000;
                draw_line_bresenham(&mut buf, stride, w as i32, h as i32, x0, y0, x1, y1, color);
                count += 1;
            }
        }
        core::hint::black_box(&buf);
        count
    } else {
        let mut count: u64 = 0;
        let start = anyos_std::sys::uptime_ms();
        let mut seed: u32 = 13;
        while anyos_std::sys::uptime_ms().wrapping_sub(start) < GPU_TEST_MS {
            for _ in 0..50 {
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let x0 = ((seed >> 16) % w) as i32;
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let y0 = ((seed >> 16) % h) as i32;
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let x1 = ((seed >> 16) % w) as i32;
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let y1 = ((seed >> 16) % h) as i32;
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let color = seed | 0xFF000000;
                canvas.draw_line(x0, y0, x1, y1, color);
                count += 1;
            }
        }
        count
    }
}
