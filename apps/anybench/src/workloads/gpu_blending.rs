//! GPU Benchmark 5 — Alpha Blending.
//!
//! Composites semi-transparent rectangles for [`GPU_TEST_MS`] milliseconds.
//! Offscreen uses per-pixel alpha blending; onscreen uses opaque rects
//! (Canvas API has no alpha support). Returns total rectangles composited.

use alloc::vec;
use libanyui_client as anyui;
use super::{GPU_TEST_MS, alpha_blend};

/// Alpha-blended rectangle compositing benchmark.
pub fn bench_gpu_blending(canvas: &anyui::Canvas, offscreen: bool) -> u64 {
    let w = canvas.get_stride();
    let h = canvas.get_height();
    if w == 0 || h == 0 { return 0; }

    if offscreen {
        let buf_size = (w * h) as usize;
        let mut buf = vec![0u32; buf_size];
        let stride = w as usize;
        let mut count: u64 = 0;
        let start = anyos_std::sys::uptime_ms();
        let mut seed: u32 = 99;
        while anyos_std::sys::uptime_ms().wrapping_sub(start) < GPU_TEST_MS {
            for _ in 0..50 {
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let rx = ((seed >> 16) % w) as usize;
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let ry = ((seed >> 16) % h) as usize;
                let rw = (((seed >> 8) & 0x3F) + 8) as usize;
                let rh = (((seed >> 4) & 0x3F) + 8) as usize;
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let alpha = ((seed >> 16) & 0xBF) + 0x20; // 32..223
                let color = (alpha << 24) | (seed & 0x00FFFFFF);
                let x_end = (rx + rw).min(w as usize);
                let y_end = (ry + rh).min(h as usize);
                for y in ry..y_end {
                    for x in rx..x_end {
                        let dst = buf[y * stride + x];
                        buf[y * stride + x] = alpha_blend(color, dst);
                    }
                }
                count += 1;
            }
        }
        core::hint::black_box(&buf);
        count
    } else {
        // OnScreen: Canvas API doesn't support alpha blending, use opaque rects
        let mut count: u64 = 0;
        let start = anyos_std::sys::uptime_ms();
        let mut seed: u32 = 99;
        while anyos_std::sys::uptime_ms().wrapping_sub(start) < GPU_TEST_MS {
            for _ in 0..50 {
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let rx = ((seed >> 16) % w) as i32;
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let ry = ((seed >> 16) % h) as i32;
                let rw = ((seed >> 8) & 0x3F) + 8;
                let rh = ((seed >> 4) & 0x3F) + 8;
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let color = seed | 0xFF000000;
                canvas.fill_rect(rx, ry, rw, rh, color);
                count += 1;
            }
        }
        count
    }
}
