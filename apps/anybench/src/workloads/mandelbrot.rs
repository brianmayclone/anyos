//! CPU Benchmark 2 — Mandelbrot set computation.
//!
//! Computes escape iterations for a 256×256 grid of the Mandelbrot set,
//! repeated for [`CPU_TEST_MS`] milliseconds. Returns cumulative iterations.

use super::CPU_TEST_MS;

/// Mandelbrot escape-time benchmark over a 256×256 grid.
pub fn bench_mandelbrot() -> u64 {
    const W: usize = 256;
    const H: usize = 256;
    const MAX_ITER: u32 = 100;
    let mut total_iter: u64 = 0;
    let start = anyos_std::sys::uptime_ms();
    while anyos_std::sys::uptime_ms().wrapping_sub(start) < CPU_TEST_MS {
        for py in 0..H {
            for px in 0..W {
                let x0 = (px as f64 / W as f64) * 3.5 - 2.5;
                let y0 = (py as f64 / H as f64) * 2.0 - 1.0;
                let mut x = 0.0f64;
                let mut y = 0.0f64;
                let mut iter: u32 = 0;
                while x * x + y * y <= 4.0 && iter < MAX_ITER {
                    let xt = x * x - y * y + x0;
                    y = 2.0 * x * y + y0;
                    x = xt;
                    iter += 1;
                }
                total_iter += iter as u64;
            }
        }
    }
    total_iter
}
