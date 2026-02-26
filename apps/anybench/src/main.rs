//! anyBench — Comprehensive system benchmark for anyOS.
//!
//! CPU tests (SingleCore & MultiCore):
//!   1. Integer Math     — Prime sieve (Eratosthenes)
//!   2. Floating-Point   — Mandelbrot set computation
//!   3. Memory Bandwidth — Sequential buffer copy
//!   4. Matrix Math      — Dense matrix multiplication
//!   5. Crypto           — SHA-256–like hash chain
//!   6. Sorting          — Quicksort on random data
//!
//! GPU tests (OnScreen & OffScreen):
//!   1. Fill Rate        — Rectangle fill throughput
//!   2. Pixel Throughput — Individual pixel write speed
//!   3. Line Drawing     — Line rendering throughput
//!   4. Circle Rendering — Filled circle throughput
//!   5. Blending         — Alpha-blended rectangle compositing

#![no_std]
#![no_main]

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};
use libanyui_client as anyui;
use anyui::Widget;

anyos_std::entry!(main);

const TEXT_ALIGN_CENTER: u32 = 1;
const TEXT_ALIGN_RIGHT: u32 = 2;

// ════════════════════════════════════════════════════════════════════════
//  Constants
// ════════════════════════════════════════════════════════════════════════

const NUM_CPU_TESTS: usize = 6;
const NUM_GPU_TESTS: usize = 5;
const MAX_CORES: usize = 64;

// Baseline raw scores (calibrated for ~1000 pts on a single-core 2 GHz QEMU VM)
const CPU_BASELINES: [u64; NUM_CPU_TESTS] = [
    4_000,       // primes found in sieve
    200_000,     // mandelbrot iterations
    50_000_000,  // bytes copied
    800_000,     // matrix multiply ops
    15_000,      // hash iterations
    80_000,      // elements sorted
];

const GPU_BASELINES: [u64; NUM_GPU_TESTS] = [
    30_000,      // rectangles filled
    500_000,     // pixels set
    60_000,      // lines drawn
    15_000,      // circles drawn
    20_000,      // blended rects
];

// Colors
const BG_DARK: u32       = 0xFF1C1C1E;
const CARD_BG: u32       = 0xFF2C2C2E;
const ACCENT: u32        = 0xFF0A84FF;
const ACCENT_GREEN: u32  = 0xFF30D158;
const TEXT_PRIMARY: u32  = 0xFFFFFFFF;
const TEXT_SECONDARY: u32 = 0xFF8E8E93;
const SCORE_GOLD: u32    = 0xFFFFD60A;

// ════════════════════════════════════════════════════════════════════════
//  Shared state for benchmark workers (fork-based)
// ════════════════════════════════════════════════════════════════════════

// Which benchmark to run (1-6 for CPU tests)
static BENCH_ID: AtomicU32 = AtomicU32::new(0);
// Child process TIDs from fork()
static mut CHILD_TIDS: [u32; MAX_CORES] = [0; MAX_CORES];
// Results collected from child exit codes
static mut CHILD_RESULTS: [u64; MAX_CORES] = [0; MAX_CORES];
// How many children were forked so far (may still be forking one per tick)
static mut CHILDREN_FORKED: u32 = 0;
// Total children needed for this test
static mut NUM_CHILDREN: u32 = 0;
// How many have been reaped via waitpid
static mut CHILDREN_REAPED: u32 = 0;

// ════════════════════════════════════════════════════════════════════════
//  Global application state
// ════════════════════════════════════════════════════════════════════════

struct AppState {
    win: anyui::Window,
    num_cpus: u32,

    // Tab navigation
    tabs: anyui::SegmentedControl,

    // Overview panel
    panel_overview: anyui::View,
    lbl_subtitle: anyui::Label,
    progress: anyui::ProgressBar,
    lbl_status: anyui::Label,
    btn_run_all: anyui::Button,
    btn_run_cpu: anyui::Button,
    btn_run_gpu: anyui::Button,

    // CPU panel
    panel_cpu: anyui::View,
    lbl_cpu_single_score: anyui::Label,
    lbl_cpu_multi_score: anyui::Label,
    cpu_single_labels: [anyui::Label; NUM_CPU_TESTS],
    cpu_single_scores: [anyui::Label; NUM_CPU_TESTS],
    cpu_multi_labels: [anyui::Label; NUM_CPU_TESTS],
    cpu_multi_scores: [anyui::Label; NUM_CPU_TESTS],

    // GPU panel
    panel_gpu: anyui::View,
    lbl_gpu_onscreen_score: anyui::Label,
    lbl_gpu_offscreen_score: anyui::Label,
    gpu_on_labels: [anyui::Label; NUM_GPU_TESTS],
    gpu_on_scores: [anyui::Label; NUM_GPU_TESTS],
    gpu_off_labels: [anyui::Label; NUM_GPU_TESTS],
    gpu_off_scores: [anyui::Label; NUM_GPU_TESTS],
    canvas: anyui::Canvas,

    // Benchmark state
    running: bool,
    phase: BenchPhase,
    current_test: usize,
    timer_id: u32,

    // Results storage
    cpu_single_raw: [u64; NUM_CPU_TESTS],
    cpu_multi_raw: [u64; NUM_CPU_TESTS],
    gpu_on_raw: [u64; NUM_GPU_TESTS],
    gpu_off_raw: [u64; NUM_GPU_TESTS],
}

#[derive(Clone, Copy, PartialEq)]
enum BenchPhase {
    Idle,
    CpuSingle,
    CpuMulti,
    GpuOnScreen,
    GpuOffScreen,
}

static mut APP: Option<AppState> = None;
fn app() -> &'static mut AppState {
    unsafe { APP.as_mut().expect("app not init") }
}

// ════════════════════════════════════════════════════════════════════════
//  CPU Benchmark Implementations
// ════════════════════════════════════════════════════════════════════════

/// 1. Prime Sieve — Sieve of Eratosthenes up to N
fn bench_prime_sieve() -> u64 {
    const N: usize = 100_000;
    let mut sieve = vec![true; N];
    sieve[0] = false;
    if N > 1 { sieve[1] = false; }
    let mut i = 2;
    while i * i < N {
        if sieve[i] {
            let mut j = i * i;
            while j < N {
                sieve[j] = false;
                j += i;
            }
        }
        i += 1;
    }
    let count = sieve.iter().filter(|&&v| v).count() as u64;
    // Run multiple iterations to extend the test
    let mut total = count;
    for _ in 1..5 {
        // Re-run sieve
        for v in sieve.iter_mut() { *v = true; }
        sieve[0] = false;
        sieve[1] = false;
        i = 2;
        while i * i < N {
            if sieve[i] {
                let mut j = i * i;
                while j < N {
                    sieve[j] = false;
                    j += i;
                }
            }
            i += 1;
        }
        total += sieve.iter().filter(|&&v| v).count() as u64;
    }
    total
}

/// 2. Mandelbrot — Compute escape iterations for a 256x256 grid
fn bench_mandelbrot() -> u64 {
    const W: usize = 256;
    const H: usize = 256;
    const MAX_ITER: u32 = 100;
    let mut total_iter: u64 = 0;

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
    total_iter
}

/// 3. Memory Bandwidth — Sequential copy of a large buffer
fn bench_memory_copy() -> u64 {
    const BUF_SIZE: usize = 256 * 1024; // 256 KB
    const ITERATIONS: usize = 8;
    let src = vec![0xAAu8; BUF_SIZE];
    let mut dst = vec![0u8; BUF_SIZE];
    let mut total_bytes: u64 = 0;
    for _ in 0..ITERATIONS {
        // Manual copy to prevent optimizer from eliding it
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

/// 4. Matrix Multiply — 64x64 dense integer matrix multiplication
fn bench_matrix_multiply() -> u64 {
    const N: usize = 64;
    const REPS: usize = 3;
    let mut a = vec![0i32; N * N];
    let mut b = vec![0i32; N * N];
    let mut c = vec![0i32; N * N];

    // Fill with pseudo-random data
    let mut seed: u32 = 12345;
    for i in 0..N * N {
        seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
        a[i] = (seed >> 16) as i32 & 0xFF;
        seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
        b[i] = (seed >> 16) as i32 & 0xFF;
    }

    let mut ops: u64 = 0;
    for _ in 0..REPS {
        for i in 0..N {
            for j in 0..N {
                let mut sum = 0i32;
                for k in 0..N {
                    sum = sum.wrapping_add(a[i * N + k].wrapping_mul(b[k * N + j]));
                }
                c[i * N + j] = sum;
            }
        }
        ops += (N * N * N * 2) as u64; // multiply + add per element
    }
    core::hint::black_box(&c);
    ops
}

/// 5. Crypto — SHA-256–like hash chain (simplified Merkle-Damgard)
fn bench_crypto_hash() -> u64 {
    const ITERATIONS: usize = 50_000;
    let mut hash: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
        0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];

    for i in 0..ITERATIONS {
        // Simplified compression: mix each word with the round index
        let mut a = hash[0];
        let mut b = hash[1];
        let mut c = hash[2];
        let mut d = hash[3];
        let mut e = hash[4];
        let mut f = hash[5];
        let mut g = hash[6];
        let mut h = hash[7];

        for r in 0..64u32 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = h.wrapping_add(s1).wrapping_add(ch).wrapping_add(r).wrapping_add(i as u32);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }

        hash[0] = hash[0].wrapping_add(a);
        hash[1] = hash[1].wrapping_add(b);
        hash[2] = hash[2].wrapping_add(c);
        hash[3] = hash[3].wrapping_add(d);
        hash[4] = hash[4].wrapping_add(e);
        hash[5] = hash[5].wrapping_add(f);
        hash[6] = hash[6].wrapping_add(g);
        hash[7] = hash[7].wrapping_add(h);
    }
    core::hint::black_box(&hash);
    ITERATIONS as u64
}

/// 6. Sorting — Quicksort on pseudo-random array
fn bench_sort() -> u64 {
    const SIZE: usize = 50_000;
    const REPS: usize = 3;
    let mut data = vec![0u32; SIZE];
    let mut total: u64 = 0;

    for rep in 0..REPS {
        // Fill with pseudo-random data
        let mut seed: u32 = 42 + rep as u32;
        for v in data.iter_mut() {
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
            *v = seed;
        }
        quicksort(&mut data);
        total += SIZE as u64;
    }
    core::hint::black_box(&data);
    total
}

fn quicksort(arr: &mut [u32]) {
    if arr.len() <= 1 { return; }
    let pivot = arr[arr.len() / 2];
    let mut lo = 0usize;
    let mut hi = arr.len() - 1;
    while lo <= hi {
        while arr[lo] < pivot { lo += 1; }
        while arr[hi] > pivot {
            if hi == 0 { break; }
            hi -= 1;
        }
        if lo <= hi {
            arr.swap(lo, hi);
            lo += 1;
            if hi == 0 { break; }
            hi -= 1;
        }
    }
    if hi > 0 { quicksort(&mut arr[..=hi]); }
    if lo < arr.len() { quicksort(&mut arr[lo..]); }
}

// ════════════════════════════════════════════════════════════════════════
//  Fork-based benchmark worker
// ════════════════════════════════════════════════════════════════════════

/// Run a CPU benchmark by ID. Returns the raw score.
fn run_cpu_bench(bench_id: u32) -> u64 {
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

/// Fork a child process that runs the given benchmark and exits with the result.
/// Returns the child TID in the parent, or 0 on fork failure.
fn fork_bench_worker(bench_id: u32) -> u32 {
    let child = anyos_std::process::fork();
    if child == 0 {
        // Child process: run benchmark, exit with result as exit code
        let result = run_cpu_bench(bench_id);
        // Cap at u32::MAX - 2 to avoid conflicting with STILL_RUNNING / error sentinels
        let code = if result > 0xFFFF_FFFD { 0xFFFF_FFFD } else { result as u32 };
        anyos_std::process::exit(code);
    }
    // Parent: child == child TID (>0), or u32::MAX on error
    if child == u32::MAX { 0 } else { child }
}

// ════════════════════════════════════════════════════════════════════════
//  GPU Benchmark Implementations
// ════════════════════════════════════════════════════════════════════════

/// GPU benchmarks run for a fixed time and count operations
const GPU_TEST_MS: u32 = 2000;

fn bench_gpu_fill_rect(canvas: &anyui::Canvas, offscreen: bool) -> u64 {
    let w = canvas.get_stride();
    let h = canvas.get_height();
    if w == 0 || h == 0 { return 0; }

    if offscreen {
        // Offscreen: write to raw buffer directly
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
        // OnScreen: use Canvas API
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

fn bench_gpu_pixels(canvas: &anyui::Canvas, offscreen: bool) -> u64 {
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

fn bench_gpu_lines(canvas: &anyui::Canvas, offscreen: bool) -> u64 {
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

fn bench_gpu_circles(canvas: &anyui::Canvas, offscreen: bool) -> u64 {
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

fn bench_gpu_blending(canvas: &anyui::Canvas, offscreen: bool) -> u64 {
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
        // OnScreen: use fill_rect with semi-transparent colors
        // Canvas API doesn't do alpha blending, so we simulate with opaque rects at varying brightness
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

/// Fast alpha blend: src over dst (ARGB8888)
#[inline]
fn alpha_blend(src: u32, dst: u32) -> u32 {
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

/// Bresenham line drawing for offscreen buffer
fn draw_line_bresenham(buf: &mut [u32], stride: usize, w: i32, h: i32, mut x0: i32, mut y0: i32, x1: i32, y1: i32, color: u32) {
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

/// Filled circle for offscreen buffer
fn draw_filled_circle(buf: &mut [u32], stride: usize, w: i32, h: i32, cx: i32, cy: i32, r: i32, color: u32) {
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

fn isqrt(n: u32) -> u32 {
    if n == 0 { return 0; }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

// ════════════════════════════════════════════════════════════════════════
//  Scoring
// ════════════════════════════════════════════════════════════════════════

fn compute_score(raw: u64, baseline: u64) -> u32 {
    if baseline == 0 { return 0; }
    ((raw * 1000) / baseline) as u32
}

/// Geometric mean of scores (integer approximation)
fn geometric_mean(scores: &[u32]) -> u32 {
    if scores.is_empty() { return 0; }
    // Use log-sum approach: exp(mean(ln(scores)))
    // Approximate with integer math: product^(1/n)
    // For stability, use successive averaging in log space with fixed-point
    let mut log_sum: u64 = 0;
    let mut count = 0u32;
    for &s in scores {
        if s > 0 {
            log_sum += int_log2_fp(s as u64);
            count += 1;
        }
    }
    if count == 0 { return 0; }
    let avg_log = log_sum / count as u64;
    int_exp2_fp(avg_log)
}

/// Fixed-point log2 (16.16 format): returns log2(x) * 65536
fn int_log2_fp(x: u64) -> u64 {
    if x <= 1 { return 0; }
    let mut val = x;
    let mut result: u64 = 0;
    // Integer part
    while val >= 2 {
        val >>= 1;
        result += 65536;
    }
    // Fractional part (4 iterations of Newton's method)
    let mut frac = x << 16 >> (result >> 16);
    for _ in 0..16 {
        frac = (frac * frac) >> 16;
        if frac >= 2 * 65536 {
            frac >>= 1;
            result += 65536 >> 1;
        }
        // Reduce shift count
        result >>= 0; // no-op, just for loop stability
    }
    // Simplified: just use integer part
    result
}

/// Fixed-point exp2 (16.16 format): returns 2^(x/65536)
fn int_exp2_fp(x: u64) -> u32 {
    let int_part = x >> 16;
    if int_part >= 31 { return u32::MAX; }
    (1u32 << int_part as u32) as u32
}

// ════════════════════════════════════════════════════════════════════════
//  Test names
// ════════════════════════════════════════════════════════════════════════

const CPU_TEST_NAMES: [&str; NUM_CPU_TESTS] = [
    "Integer Math",
    "Floating-Point",
    "Memory Bandwidth",
    "Matrix Math",
    "Crypto Hash",
    "Sorting",
];

const GPU_TEST_NAMES: [&str; NUM_GPU_TESTS] = [
    "Fill Rate",
    "Pixel Throughput",
    "Line Drawing",
    "Circle Rendering",
    "Alpha Blending",
];

// ════════════════════════════════════════════════════════════════════════
//  UI Construction
// ════════════════════════════════════════════════════════════════════════

fn make_label_pair(parent: &anyui::View, name: &str, y: i32) -> (anyui::Label, anyui::Label) {
    let lbl_name = anyui::Label::new(name);
    lbl_name.set_position(16, y);
    lbl_name.set_size(200, 20);
    lbl_name.set_text_color(TEXT_PRIMARY);
    lbl_name.set_font_size(13);
    parent.add(&lbl_name);

    let lbl_score = anyui::Label::new("-");
    lbl_score.set_position(230, y);
    lbl_score.set_size(100, 20);
    lbl_score.set_text_color(TEXT_SECONDARY);
    lbl_score.set_font_size(13);
    lbl_score.set_state(TEXT_ALIGN_CENTER);
    parent.add(&lbl_score);

    (lbl_name, lbl_score)
}

fn build_ui() {
    let win = anyui::Window::new("anyBench", -1, -1, 640, 520);
    win.set_color(BG_DARK);

    // Get CPU count
    let num_cpus = anyos_std::sys::sysinfo(2, &mut [0u8; 4]);
    let num_cpus = if num_cpus == 0 { 1 } else { num_cpus };

    // ── Tab bar ──
    let tabs = anyui::SegmentedControl::new("Overview|CPU|GPU");
    tabs.set_dock(anyui::DOCK_TOP);
    tabs.set_size(640, 36);
    tabs.set_margin(8, 8, 8, 0);
    win.add(&tabs);

    // ════════════════════════════════════════════════════════════════
    //  Overview Panel
    // ════════════════════════════════════════════════════════════════

    let panel_overview = anyui::View::new();
    panel_overview.set_dock(anyui::DOCK_FILL);
    panel_overview.set_color(BG_DARK);
    win.add(&panel_overview);

    let lbl_title = anyui::Label::new("anyBench");
    lbl_title.set_position(0, 20);
    lbl_title.set_size(640, 36);
    lbl_title.set_font_size(28);
    lbl_title.set_font(1); // bold
    lbl_title.set_text_color(TEXT_PRIMARY);
    lbl_title.set_state(TEXT_ALIGN_CENTER);
    panel_overview.add(&lbl_title);

    let lbl_subtitle = anyui::Label::new(&format!("Comprehensive System Benchmark  |  {} CPU Core{} detected",
        num_cpus, if num_cpus != 1 { "s" } else { "" }));
    lbl_subtitle.set_position(0, 60);
    lbl_subtitle.set_size(640, 20);
    lbl_subtitle.set_font_size(13);
    lbl_subtitle.set_text_color(TEXT_SECONDARY);
    lbl_subtitle.set_state(TEXT_ALIGN_CENTER);
    panel_overview.add(&lbl_subtitle);

    // Buttons
    let btn_run_all = anyui::Button::new("Run All Tests");
    btn_run_all.set_position(200, 110);
    btn_run_all.set_size(240, 40);
    btn_run_all.set_color(ACCENT);
    btn_run_all.set_text_color(TEXT_PRIMARY);
    btn_run_all.set_font_size(15);
    panel_overview.add(&btn_run_all);

    let btn_run_cpu = anyui::Button::new("CPU Only");
    btn_run_cpu.set_position(170, 165);
    btn_run_cpu.set_size(140, 34);
    btn_run_cpu.set_font_size(13);
    panel_overview.add(&btn_run_cpu);

    let btn_run_gpu = anyui::Button::new("GPU Only");
    btn_run_gpu.set_position(330, 165);
    btn_run_gpu.set_size(140, 34);
    btn_run_gpu.set_font_size(13);
    panel_overview.add(&btn_run_gpu);

    // Progress
    let progress = anyui::ProgressBar::new(0);
    progress.set_position(40, 225);
    progress.set_size(560, 10);
    panel_overview.add(&progress);

    let lbl_status = anyui::Label::new("Ready. Press 'Run All Tests' to begin.");
    lbl_status.set_position(0, 245);
    lbl_status.set_size(640, 20);
    lbl_status.set_font_size(12);
    lbl_status.set_text_color(TEXT_SECONDARY);
    lbl_status.set_state(TEXT_ALIGN_CENTER);
    panel_overview.add(&lbl_status);

    // Score summary cards
    let card_labels = ["CPU Single-Core", "CPU Multi-Core", "GPU OnScreen", "GPU OffScreen"];
    let card_x = [32, 172, 340, 480];
    // We'll display these scores when available
    let _lbl_summary_titles: Vec<anyui::Label> = (0..4).map(|i| {
        let l = anyui::Label::new(card_labels[i]);
        l.set_position(card_x[i], 285);
        l.set_size(130, 16);
        l.set_font_size(11);
        l.set_text_color(TEXT_SECONDARY);
        l.set_state(TEXT_ALIGN_CENTER);
        panel_overview.add(&l);
        l
    }).collect();

    let lbl_summary_scores: Vec<anyui::Label> = (0..4).map(|i| {
        let l = anyui::Label::new("-");
        l.set_position(card_x[i], 305);
        l.set_size(130, 28);
        l.set_font_size(22);
        l.set_font(1);
        l.set_text_color(SCORE_GOLD);
        l.set_state(TEXT_ALIGN_CENTER);
        panel_overview.add(&l);
        l
    }).collect();

    // Divider
    let div = anyui::Divider::new();
    div.set_position(32, 350);
    div.set_size(576, 1);
    panel_overview.add(&div);

    let lbl_info = anyui::Label::new("anyBench measures CPU and GPU performance with standardized workloads.");
    lbl_info.set_position(0, 370);
    lbl_info.set_size(640, 16);
    lbl_info.set_font_size(11);
    lbl_info.set_text_color(TEXT_SECONDARY);
    lbl_info.set_state(TEXT_ALIGN_CENTER);
    panel_overview.add(&lbl_info);

    let lbl_info2 = anyui::Label::new("Higher scores indicate better performance. Baseline: 1000 pts.");
    lbl_info2.set_position(0, 390);
    lbl_info2.set_size(640, 16);
    lbl_info2.set_font_size(11);
    lbl_info2.set_text_color(TEXT_SECONDARY);
    lbl_info2.set_state(TEXT_ALIGN_CENTER);
    panel_overview.add(&lbl_info2);

    // ════════════════════════════════════════════════════════════════
    //  CPU Panel
    // ════════════════════════════════════════════════════════════════

    let panel_cpu = anyui::View::new();
    panel_cpu.set_dock(anyui::DOCK_FILL);
    panel_cpu.set_color(BG_DARK);
    panel_cpu.set_visible(false);
    win.add(&panel_cpu);

    // Single-Core section
    let lbl_sc_header = anyui::Label::new("Single-Core Performance");
    lbl_sc_header.set_position(16, 12);
    lbl_sc_header.set_size(300, 24);
    lbl_sc_header.set_font_size(16);
    lbl_sc_header.set_font(1);
    lbl_sc_header.set_text_color(TEXT_PRIMARY);
    panel_cpu.add(&lbl_sc_header);

    let lbl_cpu_single_score = anyui::Label::new("Score: -");
    lbl_cpu_single_score.set_position(400, 12);
    lbl_cpu_single_score.set_size(220, 24);
    lbl_cpu_single_score.set_font_size(16);
    lbl_cpu_single_score.set_font(1);
    lbl_cpu_single_score.set_text_color(SCORE_GOLD);
    lbl_cpu_single_score.set_state(TEXT_ALIGN_RIGHT);
    panel_cpu.add(&lbl_cpu_single_score);

    let mut cpu_single_labels = [anyui::Label::new(""); NUM_CPU_TESTS];
    let mut cpu_single_scores = [anyui::Label::new(""); NUM_CPU_TESTS];
    for i in 0..NUM_CPU_TESTS {
        let y = 44 + i as i32 * 28;
        let (ln, ls) = make_label_pair(&panel_cpu, CPU_TEST_NAMES[i], y);
        cpu_single_labels[i] = ln;
        cpu_single_scores[i] = ls;
    }

    // Multi-Core section
    let lbl_mc_header = anyui::Label::new("Multi-Core Performance");
    lbl_mc_header.set_position(16, 226);
    lbl_mc_header.set_size(300, 24);
    lbl_mc_header.set_font_size(16);
    lbl_mc_header.set_font(1);
    lbl_mc_header.set_text_color(TEXT_PRIMARY);
    panel_cpu.add(&lbl_mc_header);

    let lbl_mc_cores = anyui::Label::new(&format!("({} Cores)", num_cpus));
    lbl_mc_cores.set_position(220, 226);
    lbl_mc_cores.set_size(100, 24);
    lbl_mc_cores.set_font_size(13);
    lbl_mc_cores.set_text_color(TEXT_SECONDARY);
    panel_cpu.add(&lbl_mc_cores);

    let lbl_cpu_multi_score = anyui::Label::new("Score: -");
    lbl_cpu_multi_score.set_position(400, 226);
    lbl_cpu_multi_score.set_size(220, 24);
    lbl_cpu_multi_score.set_font_size(16);
    lbl_cpu_multi_score.set_font(1);
    lbl_cpu_multi_score.set_text_color(SCORE_GOLD);
    lbl_cpu_multi_score.set_state(TEXT_ALIGN_RIGHT);
    panel_cpu.add(&lbl_cpu_multi_score);

    let mut cpu_multi_labels = [anyui::Label::new(""); NUM_CPU_TESTS];
    let mut cpu_multi_scores = [anyui::Label::new(""); NUM_CPU_TESTS];
    for i in 0..NUM_CPU_TESTS {
        let y = 258 + i as i32 * 28;
        let (ln, ls) = make_label_pair(&panel_cpu, CPU_TEST_NAMES[i], y);
        cpu_multi_labels[i] = ln;
        cpu_multi_scores[i] = ls;
    }

    // ════════════════════════════════════════════════════════════════
    //  GPU Panel
    // ════════════════════════════════════════════════════════════════

    let panel_gpu = anyui::View::new();
    panel_gpu.set_dock(anyui::DOCK_FILL);
    panel_gpu.set_color(BG_DARK);
    panel_gpu.set_visible(false);
    win.add(&panel_gpu);

    // Canvas for onscreen rendering
    let canvas = anyui::Canvas::new(300, 180);
    canvas.set_position(320, 16);
    canvas.set_size(300, 180);
    canvas.clear(0xFF000000);
    panel_gpu.add(&canvas);

    let lbl_canvas_title = anyui::Label::new("Render Preview");
    lbl_canvas_title.set_position(320, 200);
    lbl_canvas_title.set_size(300, 16);
    lbl_canvas_title.set_font_size(10);
    lbl_canvas_title.set_text_color(TEXT_SECONDARY);
    lbl_canvas_title.set_state(TEXT_ALIGN_CENTER);
    panel_gpu.add(&lbl_canvas_title);

    // OnScreen section
    let lbl_on_header = anyui::Label::new("OnScreen");
    lbl_on_header.set_position(16, 12);
    lbl_on_header.set_size(200, 24);
    lbl_on_header.set_font_size(16);
    lbl_on_header.set_font(1);
    lbl_on_header.set_text_color(TEXT_PRIMARY);
    panel_gpu.add(&lbl_on_header);

    let lbl_gpu_onscreen_score = anyui::Label::new("Score: -");
    lbl_gpu_onscreen_score.set_position(140, 12);
    lbl_gpu_onscreen_score.set_size(160, 24);
    lbl_gpu_onscreen_score.set_font_size(16);
    lbl_gpu_onscreen_score.set_font(1);
    lbl_gpu_onscreen_score.set_text_color(SCORE_GOLD);
    lbl_gpu_onscreen_score.set_state(TEXT_ALIGN_RIGHT);
    panel_gpu.add(&lbl_gpu_onscreen_score);

    let mut gpu_on_labels = [anyui::Label::new(""); NUM_GPU_TESTS];
    let mut gpu_on_scores = [anyui::Label::new(""); NUM_GPU_TESTS];
    for i in 0..NUM_GPU_TESTS {
        let y = 44 + i as i32 * 28;
        let (ln, ls) = make_label_pair(&panel_gpu, GPU_TEST_NAMES[i], y);
        gpu_on_labels[i] = ln;
        gpu_on_scores[i] = ls;
    }

    // OffScreen section
    let lbl_off_header = anyui::Label::new("OffScreen");
    lbl_off_header.set_position(16, 230);
    lbl_off_header.set_size(200, 24);
    lbl_off_header.set_font_size(16);
    lbl_off_header.set_font(1);
    lbl_off_header.set_text_color(TEXT_PRIMARY);
    panel_gpu.add(&lbl_off_header);

    let lbl_gpu_offscreen_score = anyui::Label::new("Score: -");
    lbl_gpu_offscreen_score.set_position(140, 230);
    lbl_gpu_offscreen_score.set_size(160, 24);
    lbl_gpu_offscreen_score.set_font_size(16);
    lbl_gpu_offscreen_score.set_font(1);
    lbl_gpu_offscreen_score.set_text_color(SCORE_GOLD);
    lbl_gpu_offscreen_score.set_state(TEXT_ALIGN_RIGHT);
    panel_gpu.add(&lbl_gpu_offscreen_score);

    let mut gpu_off_labels = [anyui::Label::new(""); NUM_GPU_TESTS];
    let mut gpu_off_scores = [anyui::Label::new(""); NUM_GPU_TESTS];
    for i in 0..NUM_GPU_TESTS {
        let y = 262 + i as i32 * 28;
        let (ln, ls) = make_label_pair(&panel_gpu, GPU_TEST_NAMES[i], y);
        gpu_off_labels[i] = ln;
        gpu_off_scores[i] = ls;
    }

    // ── Tab switching ──
    tabs.connect_panels(&[&panel_overview, &panel_cpu, &panel_gpu]);

    // ── Store reference labels for summary scores ──
    // We'll store them via the panel_overview children, accessed by ID later
    // For simplicity, store summary score label IDs in a static
    unsafe {
        SUMMARY_SCORE_IDS = [
            lbl_summary_scores[0].id(),
            lbl_summary_scores[1].id(),
            lbl_summary_scores[2].id(),
            lbl_summary_scores[3].id(),
        ];
    }

    // ── Create state ──
    unsafe {
        APP = Some(AppState {
            win,
            num_cpus,
            tabs,
            panel_overview,
            lbl_subtitle,
            progress,
            lbl_status,
            btn_run_all,
            btn_run_cpu,
            btn_run_gpu,
            panel_cpu,
            lbl_cpu_single_score,
            lbl_cpu_multi_score,
            cpu_single_labels,
            cpu_single_scores,
            cpu_multi_labels,
            cpu_multi_scores,
            panel_gpu,
            lbl_gpu_onscreen_score,
            lbl_gpu_offscreen_score,
            gpu_on_labels,
            gpu_on_scores,
            gpu_off_labels,
            gpu_off_scores,
            canvas,
            running: false,
            phase: BenchPhase::Idle,
            current_test: 0,
            timer_id: 0,
            cpu_single_raw: [0; NUM_CPU_TESTS],
            cpu_multi_raw: [0; NUM_CPU_TESTS],
            gpu_on_raw: [0; NUM_GPU_TESTS],
            gpu_off_raw: [0; NUM_GPU_TESTS],
        });
    }

    // ── Button callbacks ──
    btn_run_all.on_click(|_| start_benchmark(BenchMode::All));
    btn_run_cpu.on_click(|_| start_benchmark(BenchMode::CpuOnly));
    btn_run_gpu.on_click(|_| start_benchmark(BenchMode::GpuOnly));

    win.on_close(|_| anyui::quit());
}

static mut SUMMARY_SCORE_IDS: [u32; 4] = [0; 4];
static mut BENCH_MODE: BenchMode = BenchMode::All;

#[derive(Clone, Copy, PartialEq)]
enum BenchMode {
    All,
    CpuOnly,
    GpuOnly,
}

// ════════════════════════════════════════════════════════════════════════
//  Benchmark orchestration
// ════════════════════════════════════════════════════════════════════════

fn start_benchmark(mode: BenchMode) {
    let a = app();
    if a.running { return; }

    a.running = true;
    unsafe { BENCH_MODE = mode; }

    // Reset scores
    a.cpu_single_raw = [0; NUM_CPU_TESTS];
    a.cpu_multi_raw = [0; NUM_CPU_TESTS];
    a.gpu_on_raw = [0; NUM_GPU_TESTS];
    a.gpu_off_raw = [0; NUM_GPU_TESTS];

    // Reset UI score labels
    for i in 0..NUM_CPU_TESTS {
        a.cpu_single_scores[i].set_text("-"); // —
        a.cpu_multi_scores[i].set_text("-");
    }
    for i in 0..NUM_GPU_TESTS {
        a.gpu_on_scores[i].set_text("-");
        a.gpu_off_scores[i].set_text("-");
    }
    a.lbl_cpu_single_score.set_text("Score: -");
    a.lbl_cpu_multi_score.set_text("Score: -");
    a.lbl_gpu_onscreen_score.set_text("Score: -");
    a.lbl_gpu_offscreen_score.set_text("Score: -");

    // Disable buttons
    a.btn_run_all.set_enabled(false);
    a.btn_run_cpu.set_enabled(false);
    a.btn_run_gpu.set_enabled(false);

    match mode {
        BenchMode::All | BenchMode::CpuOnly => {
            a.phase = BenchPhase::CpuSingle;
            a.current_test = 0;
            a.progress.set_state(0);
            a.lbl_status.set_text("Starting CPU Single-Core tests...");
        }
        BenchMode::GpuOnly => {
            a.phase = BenchPhase::GpuOnScreen;
            a.current_test = 0;
            a.progress.set_state(0);
            a.lbl_status.set_text("Starting GPU OnScreen tests...");
        }
    }

    // Start timer to drive benchmark steps
    a.timer_id = anyui::set_timer(50, tick_benchmark);
}

fn total_steps() -> u32 {
    let mode = unsafe { BENCH_MODE };
    match mode {
        BenchMode::All => (NUM_CPU_TESTS * 2 + NUM_GPU_TESTS * 2) as u32,
        BenchMode::CpuOnly => (NUM_CPU_TESTS * 2) as u32,
        BenchMode::GpuOnly => (NUM_GPU_TESTS * 2) as u32,
    }
}

fn current_step() -> u32 {
    let a = app();
    let mode = unsafe { BENCH_MODE };
    let base = match a.phase {
        BenchPhase::CpuSingle => 0,
        BenchPhase::CpuMulti => NUM_CPU_TESTS as u32,
        BenchPhase::GpuOnScreen => {
            if mode == BenchMode::GpuOnly { 0 }
            else { (NUM_CPU_TESTS * 2) as u32 }
        }
        BenchPhase::GpuOffScreen => {
            if mode == BenchMode::GpuOnly { NUM_GPU_TESTS as u32 }
            else { (NUM_CPU_TESTS * 2 + NUM_GPU_TESTS) as u32 }
        }
        BenchPhase::Idle => return total_steps(),
    };
    base + a.current_test as u32
}

// State machine for driving benchmarks asynchronously
static BENCH_STATE: AtomicU32 = AtomicU32::new(0); // 0=ready, 1=running, 2=done

fn tick_benchmark() {
    let a = app();
    if !a.running { return; }

    let state = BENCH_STATE.load(Ordering::SeqCst);

    if state == 0 {
        // Start next test
        let progress_pct = if total_steps() > 0 {
            (current_step() * 100 / total_steps()).min(100)
        } else { 0 };
        a.progress.set_state(progress_pct);

        match a.phase {
            BenchPhase::CpuSingle => {
                if a.current_test >= NUM_CPU_TESTS {
                    // Done with single-core, move to multi-core
                    a.phase = BenchPhase::CpuMulti;
                    a.current_test = 0;
                    update_cpu_single_summary();
                    a.lbl_status.set_text("Starting CPU Multi-Core tests...");
                    return;
                }
                let name = CPU_TEST_NAMES[a.current_test];
                a.lbl_status.set_text(&format!("CPU Single-Core: {}...", name));
                a.cpu_single_scores[a.current_test].set_text("...");
                a.cpu_single_scores[a.current_test].set_text_color(ACCENT);

                // Run single-core test in a forked child process
                let bench_id = (a.current_test + 1) as u32;
                BENCH_ID.store(bench_id, Ordering::SeqCst);
                unsafe {
                    NUM_CHILDREN = 1;
                    CHILDREN_FORKED = 1;
                    CHILDREN_REAPED = 0;
                    CHILD_RESULTS[0] = 0;
                    CHILD_TIDS[0] = fork_bench_worker(bench_id);
                }
                BENCH_STATE.store(1, Ordering::SeqCst);
            }
            BenchPhase::CpuMulti => {
                if a.current_test >= NUM_CPU_TESTS {
                    // Done with multi-core
                    update_cpu_multi_summary();
                    let mode = unsafe { BENCH_MODE };
                    if mode == BenchMode::CpuOnly {
                        finish_benchmark();
                        return;
                    }
                    a.phase = BenchPhase::GpuOnScreen;
                    a.current_test = 0;
                    a.lbl_status.set_text("Starting GPU OnScreen tests...");
                    return;
                }
                let name = CPU_TEST_NAMES[a.current_test];
                a.lbl_status.set_text(&format!("CPU Multi-Core ({}T): {}...", a.num_cpus, name));
                a.cpu_multi_scores[a.current_test].set_text("...");
                a.cpu_multi_scores[a.current_test].set_text_color(ACCENT);

                let bench_id = (a.current_test + 1) as u32;
                BENCH_ID.store(bench_id, Ordering::SeqCst);
                let n = a.num_cpus.min(MAX_CORES as u32);
                unsafe {
                    NUM_CHILDREN = n;
                    CHILDREN_FORKED = 1; // fork first child now, rest one per tick
                    CHILDREN_REAPED = 0;
                    CHILD_RESULTS[0] = 0;
                    CHILD_TIDS[0] = fork_bench_worker(bench_id);
                }
                BENCH_STATE.store(1, Ordering::SeqCst);
            }
            BenchPhase::GpuOnScreen => {
                if a.current_test >= NUM_GPU_TESTS {
                    update_gpu_on_summary();
                    a.phase = BenchPhase::GpuOffScreen;
                    a.current_test = 0;
                    a.lbl_status.set_text("Starting GPU OffScreen tests...");
                    return;
                }
                let name = GPU_TEST_NAMES[a.current_test];
                a.lbl_status.set_text(&format!("GPU OnScreen: {}...", name));
                a.gpu_on_scores[a.current_test].set_text("...");
                a.gpu_on_scores[a.current_test].set_text_color(ACCENT);
                BENCH_STATE.store(1, Ordering::SeqCst);

                // Run GPU test directly (on UI thread, uses Canvas)
                a.canvas.clear(0xFF000000);
                let result = run_gpu_test(a.current_test, &a.canvas, false);
                a.gpu_on_raw[a.current_test] = result;
                let score = compute_score(result, GPU_BASELINES[a.current_test]);
                a.gpu_on_scores[a.current_test].set_text(&format!("{}", score));
                a.gpu_on_scores[a.current_test].set_text_color(score_color(score));
                a.current_test += 1;
                BENCH_STATE.store(0, Ordering::SeqCst);
            }
            BenchPhase::GpuOffScreen => {
                if a.current_test >= NUM_GPU_TESTS {
                    update_gpu_off_summary();
                    finish_benchmark();
                    return;
                }
                let name = GPU_TEST_NAMES[a.current_test];
                a.lbl_status.set_text(&format!("GPU OffScreen: {}...", name));
                a.gpu_off_scores[a.current_test].set_text("...");
                a.gpu_off_scores[a.current_test].set_text_color(ACCENT);
                BENCH_STATE.store(1, Ordering::SeqCst);

                let result = run_gpu_test(a.current_test, &a.canvas, true);
                a.gpu_off_raw[a.current_test] = result;
                let score = compute_score(result, GPU_BASELINES[a.current_test]);
                a.gpu_off_scores[a.current_test].set_text(&format!("{}", score));
                a.gpu_off_scores[a.current_test].set_text_color(score_color(score));
                a.current_test += 1;
                BENCH_STATE.store(0, Ordering::SeqCst);
            }
            BenchPhase::Idle => {}
        }
    } else if state == 1 {
        // First: fork remaining children one per tick (avoids kernel lock contention)
        let forked = unsafe { CHILDREN_FORKED };
        let total = unsafe { NUM_CHILDREN };
        if forked < total {
            let i = forked as usize;
            let bench_id = BENCH_ID.load(Ordering::SeqCst);
            unsafe {
                CHILD_RESULTS[i] = 0;
                CHILD_TIDS[i] = fork_bench_worker(bench_id);
                CHILDREN_FORKED += 1;
            }
            return; // wait for next tick before forking more
        }

        // All children forked — check if they are done via try_waitpid
        let n = total as usize;
        for i in 0..n {
            let tid = unsafe { CHILD_TIDS[i] };
            if tid == 0 { continue; } // already reaped
            let status = anyos_std::process::try_waitpid(tid);
            if status != anyos_std::process::STILL_RUNNING && status != u32::MAX {
                // Child exited — collect its result
                unsafe {
                    CHILD_RESULTS[i] = status as u64;
                    CHILD_TIDS[i] = 0; // mark as reaped
                    CHILDREN_REAPED += 1;
                }
            }
        }

        if unsafe { CHILDREN_REAPED } >= unsafe { NUM_CHILDREN } {
            // All children done — sum up results
            let mut total: u64 = 0;
            for i in 0..n {
                total += unsafe { CHILD_RESULTS[i] };
            }

            match a.phase {
                BenchPhase::CpuSingle => {
                    a.cpu_single_raw[a.current_test] = total;
                    let score = compute_score(total, CPU_BASELINES[a.current_test]);
                    a.cpu_single_scores[a.current_test].set_text(&format!("{}", score));
                    a.cpu_single_scores[a.current_test].set_text_color(score_color(score));
                    a.current_test += 1;
                }
                BenchPhase::CpuMulti => {
                    a.cpu_multi_raw[a.current_test] = total;
                    let score = compute_score(total, CPU_BASELINES[a.current_test]);
                    a.cpu_multi_scores[a.current_test].set_text(&format!("{}", score));
                    a.cpu_multi_scores[a.current_test].set_text_color(score_color(score));
                    a.current_test += 1;
                }
                _ => {}
            }
            BENCH_STATE.store(0, Ordering::SeqCst);
        }
    }
}

fn run_gpu_test(index: usize, canvas: &anyui::Canvas, offscreen: bool) -> u64 {
    match index {
        0 => bench_gpu_fill_rect(canvas, offscreen),
        1 => bench_gpu_pixels(canvas, offscreen),
        2 => bench_gpu_lines(canvas, offscreen),
        3 => bench_gpu_circles(canvas, offscreen),
        4 => bench_gpu_blending(canvas, offscreen),
        _ => 0,
    }
}

fn score_color(score: u32) -> u32 {
    if score >= 1200 { ACCENT_GREEN }
    else if score >= 800 { SCORE_GOLD }
    else if score >= 400 { 0xFFFF9F0A } // orange
    else { 0xFFFF453A } // red
}

fn update_cpu_single_summary() {
    let a = app();
    let scores: Vec<u32> = (0..NUM_CPU_TESTS)
        .map(|i| compute_score(a.cpu_single_raw[i], CPU_BASELINES[i]))
        .collect();
    let overall = geometric_mean(&scores);
    a.lbl_cpu_single_score.set_text(&format!("Score: {}", overall));
    unsafe {
        let id = SUMMARY_SCORE_IDS[0];
        anyui::Control::from_id(id).set_text(&format!("{}", overall));
    }
}

fn update_cpu_multi_summary() {
    let a = app();
    let scores: Vec<u32> = (0..NUM_CPU_TESTS)
        .map(|i| compute_score(a.cpu_multi_raw[i], CPU_BASELINES[i]))
        .collect();
    let overall = geometric_mean(&scores);
    a.lbl_cpu_multi_score.set_text(&format!("Score: {}", overall));
    unsafe {
        let id = SUMMARY_SCORE_IDS[1];
        anyui::Control::from_id(id).set_text(&format!("{}", overall));
    }
}

fn update_gpu_on_summary() {
    let a = app();
    let scores: Vec<u32> = (0..NUM_GPU_TESTS)
        .map(|i| compute_score(a.gpu_on_raw[i], GPU_BASELINES[i]))
        .collect();
    let overall = geometric_mean(&scores);
    a.lbl_gpu_onscreen_score.set_text(&format!("Score: {}", overall));
    unsafe {
        let id = SUMMARY_SCORE_IDS[2];
        anyui::Control::from_id(id).set_text(&format!("{}", overall));
    }
}

fn update_gpu_off_summary() {
    let a = app();
    let scores: Vec<u32> = (0..NUM_GPU_TESTS)
        .map(|i| compute_score(a.gpu_off_raw[i], GPU_BASELINES[i]))
        .collect();
    let overall = geometric_mean(&scores);
    a.lbl_gpu_offscreen_score.set_text(&format!("Score: {}", overall));
    unsafe {
        let id = SUMMARY_SCORE_IDS[3];
        anyui::Control::from_id(id).set_text(&format!("{}", overall));
    }
}

fn finish_benchmark() {
    let a = app();
    a.running = false;
    a.phase = BenchPhase::Idle;
    a.progress.set_state(100);
    a.lbl_status.set_text("Benchmark complete!");
    a.lbl_status.set_text_color(ACCENT_GREEN);

    // Re-enable buttons
    a.btn_run_all.set_enabled(true);
    a.btn_run_cpu.set_enabled(true);
    a.btn_run_gpu.set_enabled(true);

    // Kill timer
    if a.timer_id != 0 {
        anyui::kill_timer(a.timer_id);
        a.timer_id = 0;
    }
}

// ════════════════════════════════════════════════════════════════════════
//  Entry point
// ════════════════════════════════════════════════════════════════════════

fn main() {
    if !anyui::init() {
        anyos_std::println!("anybench: failed to load libanyui.so");
        return;
    }

    build_ui();
    anyui::run();
}
