//! Benchmark workloads for anyBench.
//!
//! Each sub-module contains a single benchmark function that runs for a fixed
//! duration and returns a raw operation count.

// CPU workloads
mod prime_sieve;
mod mandelbrot;
mod memory_copy;
mod matrix_multiply;
mod crypto_hash;
mod sort;

// GPU workloads
mod gpu_fill_rect;
mod gpu_pixels;
mod gpu_lines;
mod gpu_circles;
mod gpu_blending;

pub use prime_sieve::bench_prime_sieve;
pub use mandelbrot::bench_mandelbrot;
pub use memory_copy::bench_memory_copy;
pub use matrix_multiply::bench_matrix_multiply;
pub use crypto_hash::bench_crypto_hash;
pub use sort::bench_sort;

pub use gpu_fill_rect::bench_gpu_fill_rect;
pub use gpu_pixels::bench_gpu_pixels;
pub use gpu_lines::bench_gpu_lines;
pub use gpu_circles::bench_gpu_circles;
pub use gpu_blending::bench_gpu_blending;

use libanyui_client as anyui;

/// Duration for each CPU benchmark in milliseconds.
/// Long enough for CPU frequency scaling / P-states to settle.
pub const CPU_TEST_MS: u32 = 3000;

/// Duration for each GPU benchmark in milliseconds.
/// 5 s is long enough to smooth out compositor and interrupt jitter.
pub const GPU_TEST_MS: u32 = 5000;

pub const NUM_CPU_TESTS: usize = 6;
pub const NUM_GPU_TESTS: usize = 5;

/// Baseline raw scores (calibrated for ~1000 pts on a single-core 2 GHz QEMU VM, 3 s runs).
pub const CPU_BASELINES: [u64; NUM_CPU_TESTS] = [
    30_000_000,    // primes found (sieve iterations * ~9592)
    10_000_000,    // mandelbrot escape iterations
    500_000_000,   // bytes copied (volatile)
    500_000_000,   // matrix multiply ops (N^3 * 2 per rep)
    500_000,       // SHA-256-like hash iterations
    10_000_000,    // elements sorted
];

/// Baseline raw scores for GPU tests (calibrated for ~1000 pts, 5 s runs).
pub const GPU_BASELINES: [u64; NUM_GPU_TESTS] = [
    75_000,      // rectangles filled
    1_250_000,   // pixels set
    150_000,     // lines drawn
    37_500,      // circles drawn
    50_000,      // blended rects
];

pub const CPU_TEST_NAMES: [&str; NUM_CPU_TESTS] = [
    "Integer Math",
    "Floating-Point",
    "Memory Bandwidth",
    "Matrix Math",
    "Crypto Hash",
    "Sorting",
];

pub const GPU_TEST_NAMES: [&str; NUM_GPU_TESTS] = [
    "Fill Rate",
    "Pixel Throughput",
    "Line Drawing",
    "Circle Rendering",
    "Alpha Blending",
];

/// Dispatches a CPU benchmark by 1-based ID. Returns the raw score.
pub fn run_cpu_bench(bench_id: u32) -> u64 {
    match bench_id {
        1 => bench_prime_sieve(),
        2 => bench_mandelbrot(),
        3 => bench_memory_copy(),
        4 => bench_matrix_multiply(),
        5 => bench_crypto_hash(),
        6 => bench_sort(),
        _ => 0,
    }
}

/// Dispatches a GPU benchmark by 0-based index. Returns the raw score.
pub fn run_gpu_test(index: usize, canvas: &anyui::Canvas, offscreen: bool) -> u64 {
    match index {
        0 => bench_gpu_fill_rect(canvas, offscreen),
        1 => bench_gpu_pixels(canvas, offscreen),
        2 => bench_gpu_lines(canvas, offscreen),
        3 => bench_gpu_circles(canvas, offscreen),
        4 => bench_gpu_blending(canvas, offscreen),
        _ => 0,
    }
}

// ────────────────────────────────────────────────────────────────────────
//  Shared GPU helpers (used by multiple workloads)
// ────────────────────────────────────────────────────────────────────────

/// Fast alpha blend: src over dst (ARGB8888).
#[inline]
pub(crate) fn alpha_blend(src: u32, dst: u32) -> u32 {
    let sa = (src >> 24) & 0xFF;
    if sa == 0xFF { return src; }
    if sa == 0 { return dst; }
    let inv_a = 255 - sa;
    let sr = (src >> 16) & 0xFF;
    let sg = (src >> 8) & 0xFF;
    let sb = src & 0xFF;
    let dr = (dst >> 16) & 0xFF;
    let dg = (dst >> 8) & 0xFF;
    let db = dst & 0xFF;
    let r = (sr * sa + dr * inv_a) / 255;
    let g = (sg * sa + dg * inv_a) / 255;
    let b = (sb * sa + db * inv_a) / 255;
    0xFF000000 | (r << 16) | (g << 8) | b
}

/// Bresenham line drawing into an offscreen buffer.
pub(crate) fn draw_line_bresenham(
    buf: &mut [u32], stride: usize, w: i32, h: i32,
    mut x0: i32, mut y0: i32, x1: i32, y1: i32, color: u32,
) {
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx: i32 = if x0 < x1 { 1 } else { -1 };
    let sy: i32 = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;

    loop {
        if x0 >= 0 && x0 < w && y0 >= 0 && y0 < h {
            buf[y0 as usize * stride + x0 as usize] = color;
        }
        if x0 == x1 && y0 == y1 { break; }
        let e2 = 2 * err;
        if e2 >= dy { err += dy; x0 += sx; }
        if e2 <= dx { err += dx; y0 += sy; }
    }
}

/// Filled circle rasterization into an offscreen buffer.
pub(crate) fn draw_filled_circle(
    buf: &mut [u32], stride: usize, w: i32, h: i32,
    cx: i32, cy: i32, r: i32, color: u32,
) {
    for dy in -r..=r {
        let y = cy + dy;
        if y < 0 || y >= h { continue; }
        let half_w = isqrt((r * r - dy * dy) as u32) as i32;
        let x_start = (cx - half_w).max(0);
        let x_end = (cx + half_w).min(w - 1);
        for x in x_start..=x_end {
            buf[y as usize * stride + x as usize] = color;
        }
    }
}

/// Integer square root (Newton's method).
pub(crate) fn isqrt(n: u32) -> u32 {
    if n == 0 { return 0; }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}
