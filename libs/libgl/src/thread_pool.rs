//! Tile-based parallel rasterization thread pool.
//!
//! Splits the framebuffer into horizontal bands and distributes rasterization
//! across worker threads. Fully transparent to client applications — the pool
//! is initialized automatically on first use and all synchronization is internal.
//!
//! Workers use `sleep_us(10)` when idle to avoid wasting CPU cycles.

use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use crate::syscall;
use crate::rasterizer::ClipVertex;
use crate::rasterizer::raster::ResolvedTexture;

/// Maximum number of worker threads (hard cap).
const MAX_WORKERS: usize = 7;

/// Maximum triangles per batch (screen-space, after clipping).
const MAX_TRIS: usize = 16384;

/// Stack size per worker thread (32 KiB).
const WORKER_STACK_SIZE: usize = 32 * 1024;

// ── Shared work data (written by main thread, read by workers) ───────────

/// A screen-space triangle ready for rasterization.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct ScreenTri {
    pub v0: ClipVertex,
    pub v1: ClipVertex,
    pub v2: ClipVertex,
    pub s0: [f32; 3],
    pub s1: [f32; 3],
    pub s2: [f32; 3],
}

/// Per-batch parameters shared between main thread and all workers.
#[repr(C)]
struct BatchParams {
    // Framebuffer raw pointers (non-overlapping y-bands → no data race)
    color_ptr: *mut u32,
    depth_ptr: *mut f32,
    fb_w: u32,
    fb_h: u32,

    // Triangle list
    tris: *const ScreenTri,
    tri_count: u32,

    // Rasterization parameters
    depth_test: bool,
    depth_func: u32,
    depth_mask: bool,
    blend_enabled: bool,
    blend_src: u32,
    blend_dst: u32,

    // Fast path texture (null if using general path)
    fast_tex_data: *const u32,
    fast_tex_w: u32,
    fast_tex_h: u32,
    fast_tex_len: usize,
    fast_mat_r: f32,
    fast_mat_g: f32,
    fast_mat_b: f32,
    use_fast_path: bool,

    // General path shader data
    fs_ir_ptr: *const crate::compiler::ir::Program,
    uniforms_ptr: *const [f32; 4],
    uniforms_len: usize,
    num_varyings: usize,
    fs_jit: Option<crate::compiler::backend_jit::JitFn>,
}

unsafe impl Send for BatchParams {}
unsafe impl Sync for BatchParams {}

/// Worker state machine.
const WORKER_IDLE: u32 = 0;
const WORKER_WORK: u32 = 1;
const WORKER_DONE: u32 = 2;
const WORKER_EXIT: u32 = 3;

/// Per-worker control block.
#[repr(C)]
struct WorkerCtl {
    state: AtomicU32,
    band_min_y: u32,
    band_max_y: u32,
}

/// Global thread pool state.
static mut POOL: Option<ThreadPool> = None;

struct ThreadPool {
    num_workers: usize,
    workers: [WorkerCtl; MAX_WORKERS],
    batch: BatchParams,
    tri_buf: [ScreenTri; MAX_TRIS],
    tri_count: usize,
}

// Worker thread IDs (to keep threads alive)
static mut WORKER_TIDS: [u32; MAX_WORKERS] = [0; MAX_WORKERS];
// Worker stack addresses (to keep allocations alive)
static mut WORKER_STACKS: [u64; MAX_WORKERS] = [0; MAX_WORKERS];

/// Initialize the thread pool with `n` workers. Called once.
fn init_pool(n: usize, fb_h: u32) {
    let n = n.min(MAX_WORKERS);
    if n == 0 { return; }

    unsafe {
        let pool = POOL.get_or_insert_with(|| ThreadPool {
            num_workers: n,
            workers: core::array::from_fn(|_| WorkerCtl {
                state: AtomicU32::new(WORKER_IDLE),
                band_min_y: 0,
                band_max_y: 0,
            }),
            batch: core::mem::zeroed(),
            tri_buf: core::mem::zeroed(),
            tri_count: 0,
        });
        pool.num_workers = n;

        // Divide framebuffer into bands
        let band_h = fb_h / n as u32;
        for i in 0..n {
            pool.workers[i].band_min_y = i as u32 * band_h;
            pool.workers[i].band_max_y = if i == n - 1 { fb_h } else { (i as u32 + 1) * band_h };
            pool.workers[i].state.store(WORKER_IDLE, Ordering::Relaxed);
        }

        // Spawn worker threads
        static WORKER_ENTRIES: [fn(); MAX_WORKERS] = [
            worker_entry_0,
            worker_entry_1,
            worker_entry_2,
            worker_entry_3,
            worker_entry_4,
            worker_entry_5,
            worker_entry_6,
        ];

        for i in 0..n {
            let stack_addr = syscall::mmap(WORKER_STACK_SIZE as u32);
            if stack_addr == u64::MAX || stack_addr == 0 {
                crate::serial_println!("[libgl] thread_pool: mmap failed for worker {}", i);
                pool.num_workers = i;
                return;
            }
            WORKER_STACKS[i] = stack_addr;
            // x86_64 ABI: RSP = stack_top - 8 at function entry
            let stack_top = (stack_addr as usize) + WORKER_STACK_SIZE - 8;
            let tid = syscall::thread_create(WORKER_ENTRIES[i], stack_top, "gl_worker");
            if tid == 0 {
                crate::serial_println!("[libgl] thread_pool: thread_create failed for worker {}", i);
                syscall::munmap(stack_addr, WORKER_STACK_SIZE as u32);
                WORKER_STACKS[i] = 0;
                pool.num_workers = i;
                return;
            }
            WORKER_TIDS[i] = tid;
        }

        crate::serial_println!("[libgl] thread_pool: spawned {} workers, fb_h={}", n, fb_h);
    }
}

// Worker entry points (one per slot, routes to generic worker_main)
fn worker_entry_0() { worker_main(0); }
fn worker_entry_1() { worker_main(1); }
fn worker_entry_2() { worker_main(2); }
fn worker_entry_3() { worker_main(3); }
fn worker_entry_4() { worker_main(4); }
fn worker_entry_5() { worker_main(5); }
fn worker_entry_6() { worker_main(6); }

/// Worker main loop. Waits for work, rasterizes its band, signals done.
fn worker_main(id: usize) {
    loop {
        // Wait for work with increasing sleep intervals to avoid burning CPU
        let pool = unsafe { POOL.as_ref().unwrap() };
        let ctl = &pool.workers[id];

        let mut idle_spins: u32 = 0;
        loop {
            let s = ctl.state.load(Ordering::Acquire);
            if s == WORKER_WORK { break; }
            if s == WORKER_EXIT { return; }
            // Exponential backoff: 100µs → 500µs → 1ms (cap)
            let sleep = if idle_spins < 10 { 100 } else if idle_spins < 50 { 500 } else { 1000 };
            syscall::sleep_us(sleep);
            idle_spins = idle_spins.saturating_add(1);
        }

        // Do work
        let pool = unsafe { POOL.as_ref().unwrap() };
        let batch = &pool.batch;
        let min_y = ctl.band_min_y as i32;
        let max_y = ctl.band_max_y as i32 - 1; // inclusive

        if batch.tri_count > 0 && !batch.color_ptr.is_null() {
            let tris = unsafe { core::slice::from_raw_parts(batch.tris, batch.tri_count as usize) };

            for tri in tris {
                if batch.use_fast_path {
                    rasterize_tri_fast_band(batch, tri, min_y, max_y);
                } else {
                    rasterize_tri_band(batch, tri, min_y, max_y, id);
                }
            }
        }

        // Signal done
        ctl.state.store(WORKER_DONE, Ordering::Release);
    }
}

/// Check if thread pool is available and has workers.
pub fn pool_active() -> bool {
    unsafe { POOL.as_ref().map_or(false, |p| p.num_workers > 0) }
}

/// Determine optimal worker count based on available CPU cores.
/// Returns `max(cpu_count - 1, 1)` clamped to MAX_WORKERS.
/// Even on single-core: 1 worker allows main thread to do vertex processing
/// while worker does rasterization (overlapping with next frame's vertex work).
fn optimal_worker_count() -> usize {
    let cpus = syscall::cpu_count() as usize;
    if cpus <= 1 {
        // Single core: still use 1 worker for pipelining
        1
    } else {
        (cpus - 1).min(MAX_WORKERS)
    }
}

/// Ensure pool is initialized for the given framebuffer height.
pub fn ensure_pool(fb_h: u32) {
    unsafe {
        if POOL.is_none() {
            let n = optimal_worker_count();
            crate::serial_println!("[libgl] thread_pool: {} CPUs detected, spawning {} workers", syscall::cpu_count(), n);
            if n > 0 {
                init_pool(n, fb_h);
            }
        } else {
            // Update bands if framebuffer height changed
            let pool = POOL.as_mut().unwrap();
            let n = pool.num_workers;
            if n > 0 && pool.workers[n - 1].band_max_y != fb_h {
                let band_h = fb_h / n as u32;
                for i in 0..n {
                    pool.workers[i].band_min_y = i as u32 * band_h;
                    pool.workers[i].band_max_y = if i == n - 1 { fb_h } else { (i as u32 + 1) * band_h };
                }
            }
        }
    }
}

/// Shut down all worker threads and free resources.
pub fn shutdown_pool() {
    unsafe {
        if let Some(pool) = POOL.as_mut() {
            let n = pool.num_workers;
            // Signal all workers to exit
            for i in 0..n {
                pool.workers[i].state.store(WORKER_EXIT, Ordering::Release);
            }
            // Wait briefly for workers to see the exit signal
            syscall::sleep_us(2000);
            // Free stacks
            for i in 0..n {
                if WORKER_STACKS[i] != 0 {
                    syscall::munmap(WORKER_STACKS[i], WORKER_STACK_SIZE as u32);
                    WORKER_STACKS[i] = 0;
                }
                WORKER_TIDS[i] = 0;
            }
            pool.num_workers = 0;
            crate::serial_println!("[libgl] thread_pool: shut down {} workers", n);
        }
        POOL = None;
    }
}

/// Get mutable ref to the triangle buffer for filling.
pub fn tri_buffer() -> &'static mut [ScreenTri; MAX_TRIS] {
    unsafe { &mut POOL.as_mut().unwrap().tri_buf }
}

/// Get current triangle count.
pub fn tri_count() -> usize {
    unsafe { POOL.as_ref().unwrap().tri_count }
}

/// Set triangle count.
pub fn set_tri_count(n: usize) {
    unsafe { POOL.as_mut().unwrap().tri_count = n.min(MAX_TRIS); }
}

/// Maximum triangles the buffer can hold.
pub fn max_tris() -> usize {
    MAX_TRIS
}

/// Submit a batch of triangles for parallel rasterization.
/// The main thread also participates by rasterizing a band.
pub fn submit_batch(
    ctx: &mut crate::state::GlContext,
    fast: Option<(&ResolvedTexture, f32, f32, f32)>,
    fs_ir: *const crate::compiler::ir::Program,
    uniforms: &[[f32; 4]],
    num_varyings: usize,
    fs_jit: Option<crate::compiler::backend_jit::JitFn>,
) {
    let pool = unsafe { POOL.as_mut().unwrap() };
    let n = pool.num_workers;
    if n == 0 || pool.tri_count == 0 { return; }

    let fb_w = ctx.default_fb.width;
    let fb_h = ctx.default_fb.height;

    // Fill batch params
    pool.batch.color_ptr = ctx.default_fb.color.as_mut_ptr();
    pool.batch.depth_ptr = ctx.default_fb.depth.as_mut_ptr();
    pool.batch.fb_w = fb_w;
    pool.batch.fb_h = fb_h;
    pool.batch.tris = pool.tri_buf.as_ptr();
    pool.batch.tri_count = pool.tri_count as u32;
    pool.batch.depth_test = ctx.depth_test;
    pool.batch.depth_func = ctx.depth_func;
    pool.batch.depth_mask = ctx.depth_mask;
    pool.batch.blend_enabled = ctx.blend;
    pool.batch.blend_src = ctx.blend_src_rgb;
    pool.batch.blend_dst = ctx.blend_dst_rgb;
    pool.batch.num_varyings = num_varyings;
    pool.batch.fs_jit = fs_jit;
    pool.batch.fs_ir_ptr = fs_ir;
    pool.batch.uniforms_ptr = uniforms.as_ptr();
    pool.batch.uniforms_len = uniforms.len();

    if let Some((tex, mr, mg, mb)) = fast {
        pool.batch.use_fast_path = true;
        pool.batch.fast_tex_data = tex.data;
        pool.batch.fast_tex_w = tex.width;
        pool.batch.fast_tex_h = tex.height;
        pool.batch.fast_tex_len = tex.len;
        pool.batch.fast_mat_r = mr;
        pool.batch.fast_mat_g = mg;
        pool.batch.fast_mat_b = mb;
    } else {
        pool.batch.use_fast_path = false;
    }

    // Signal all workers to start
    for i in 0..n {
        pool.workers[i].state.store(WORKER_WORK, Ordering::Release);
    }

    // Main thread also rasterizes a band (the last band, after all workers)
    // Actually, main thread does no band — all n bands are covered by n workers.
    // Main thread just waits for completion.

    // Wait for all workers to finish
    for i in 0..n {
        let mut wait_spins: u32 = 0;
        loop {
            let s = pool.workers[i].state.load(Ordering::Acquire);
            if s == WORKER_DONE { break; }
            // First few checks are tight spins (work should be fast),
            // then back off to avoid burning CPU
            if wait_spins < 20 {
                core::hint::spin_loop();
            } else {
                syscall::sleep_us(50);
            }
            wait_spins += 1;
        }
        pool.workers[i].state.store(WORKER_IDLE, Ordering::Release);
    }

    pool.tri_count = 0;
}

// ── Band-restricted rasterization ──────────────────────────────────────────

use crate::rasterizer::raster::{self, edge_fn, min3, max3, fast_rcp};
use crate::rasterizer::fragment;
use crate::rasterizer::MAX_VARYINGS;
use crate::simd::Vec4;

/// Rasterize one triangle in fast path, restricted to [band_min_y, band_max_y].
fn rasterize_tri_fast_band(batch: &BatchParams, tri: &ScreenTri, band_min_y: i32, band_max_y: i32) {
    let s0 = &tri.s0;
    let s1 = &tri.s1;
    let s2 = &tri.s2;
    let fb_w = batch.fb_w as i32;
    let fb_h = batch.fb_h as i32;

    // Bounding box clipped to band
    let min_x = min3(s0[0], s1[0], s2[0]).max(0.0) as i32;
    let max_x = (crate::rasterizer::math::ceil(max3(s0[0], s1[0], s2[0])) as i32).min(fb_w - 1);
    let min_y = (min3(s0[1], s1[1], s2[1]).max(0.0) as i32).max(band_min_y);
    let max_y = (crate::rasterizer::math::ceil(max3(s0[1], s1[1], s2[1])) as i32).min(fb_h - 1).min(band_max_y);
    if min_x > max_x || min_y > max_y { return; }

    let area = edge_fn(s0, s1, s2);
    if area.abs() < 1e-6 { return; }
    let inv_area = 1.0 / area.abs();

    let w0_clip = tri.v0.position[3];
    let w1_clip = tri.v1.position[3];
    let w2_clip = tri.v2.position[3];
    if w0_clip.abs() < 1e-6 || w1_clip.abs() < 1e-6 || w2_clip.abs() < 1e-6 { return; }

    let inv_w0c = 1.0 / w0_clip;
    let inv_w1c = 1.0 / w1_clip;
    let inv_w2c = 1.0 / w2_clip;

    let v0_lit = [tri.v0.varyings[0][0] * inv_w0c, tri.v0.varyings[0][1] * inv_w0c, tri.v0.varyings[0][2] * inv_w0c];
    let v1_lit = [tri.v1.varyings[0][0] * inv_w1c, tri.v1.varyings[0][1] * inv_w1c, tri.v1.varyings[0][2] * inv_w1c];
    let v2_lit = [tri.v2.varyings[0][0] * inv_w2c, tri.v2.varyings[0][1] * inv_w2c, tri.v2.varyings[0][2] * inv_w2c];

    let v0_uv = [tri.v0.varyings[1][0] * inv_w0c, tri.v0.varyings[1][1] * inv_w0c];
    let v1_uv = [tri.v1.varyings[1][0] * inv_w1c, tri.v1.varyings[1][1] * inv_w1c];
    let v2_uv = [tri.v2.varyings[1][0] * inv_w2c, tri.v2.varyings[1][1] * inv_w2c];

    let z0 = s0[2]; let z1 = s1[2]; let z2 = s2[2];
    let fb_width = batch.fb_w;

    let tex_data = batch.fast_tex_data;
    let tex_w = batch.fast_tex_w;
    let tex_h = batch.fast_tex_h;
    let tex_w_f = tex_w as f32;
    let tex_h_f = tex_h as f32;
    let tex_w_max = (tex_w - 1) as i32;
    let tex_h_max = (tex_h - 1) as i32;
    let mat_r = batch.fast_mat_r;
    let mat_g = batch.fast_mat_g;
    let mat_b = batch.fast_mat_b;

    let mut a12 = s1[1] - s2[1];
    let mut b12 = s2[0] - s1[0];
    let mut a20 = s2[1] - s0[1];
    let mut b20 = s0[0] - s2[0];
    let mut a01 = s0[1] - s1[1];
    let mut b01 = s1[0] - s0[0];

    let p0x = min_x as f32 + 0.5;
    let p0y = min_y as f32 + 0.5;
    let mut w0_row = (s2[0] - s1[0]) * (p0y - s1[1]) - (s2[1] - s1[1]) * (p0x - s1[0]);
    let mut w1_row = (s0[0] - s2[0]) * (p0y - s2[1]) - (s0[1] - s2[1]) * (p0x - s2[0]);
    let mut w2_row = (s1[0] - s0[0]) * (p0y - s0[1]) - (s1[1] - s0[1]) * (p0x - s0[0]);

    if area < 0.0 {
        w0_row = -w0_row; w1_row = -w1_row; w2_row = -w2_row;
        a12 = -a12; b12 = -b12;
        a20 = -a20; b20 = -b20;
        a01 = -a01; b01 = -b01;
    }

    let depth_test = batch.depth_test;
    let depth_func = batch.depth_func;
    let depth_mask = batch.depth_mask;

    for py in min_y..=max_y {
        let mut span_left = min_x;
        let mut span_right = max_x;
        let mut empty = false;

        macro_rules! edge_clip {
            ($w:expr, $a:expr) => {
                if !empty {
                    let w_val: f32 = $w;
                    let a_val: f32 = $a;
                    if a_val > 1e-8 {
                        if w_val < 0.0 {
                            let x = min_x + crate::rasterizer::math::ceil((-w_val) / a_val) as i32;
                            if x > span_left { span_left = x; }
                        }
                    } else if a_val < -1e-8 {
                        if w_val < 0.0 { empty = true; }
                        else {
                            let x = min_x + (w_val / (-a_val)) as i32;
                            if x < span_right { span_right = x; }
                        }
                    } else if w_val < -1e-8 { empty = true; }
                }
            };
        }

        edge_clip!(w0_row, a12);
        edge_clip!(w1_row, a20);
        edge_clip!(w2_row, a01);

        if !empty && span_left <= span_right {
            let dx = (span_left - min_x) as f32;
            let mut w0 = w0_row + a12 * dx;
            let mut w1 = w1_row + a20 * dx;
            let mut w2 = w2_row + a01 * dx;
            let row_base = py as u32 * fb_width;

            for px in span_left..=span_right {
                if w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0 {
                    let bary0 = w0 * inv_area;
                    let bary1 = w1 * inv_area;
                    let bary2 = w2 * inv_area;

                    let depth = bary0 * z0 + bary1 * z1 + bary2 * z2;
                    let fb_idx = (row_base + px as u32) as usize;

                    if depth_test {
                        let cur = unsafe { *batch.depth_ptr.add(fb_idx) };
                        if !fragment::depth_test(depth, cur, depth_func) {
                            w0 += a12; w1 += a20; w2 += a01;
                            continue;
                        }
                    }

                    let inv_w = bary0 * inv_w0c + bary1 * inv_w1c + bary2 * inv_w2c;
                    let corr = fast_rcp(inv_w);

                    let lit_r = (bary0 * v0_lit[0] + bary1 * v1_lit[0] + bary2 * v2_lit[0]) * corr;
                    let lit_g = (bary0 * v0_lit[1] + bary1 * v1_lit[1] + bary2 * v2_lit[1]) * corr;
                    let lit_b = (bary0 * v0_lit[2] + bary1 * v1_lit[2] + bary2 * v2_lit[2]) * corr;

                    let u_raw = (bary0 * v0_uv[0] + bary1 * v1_uv[0] + bary2 * v2_uv[0]) * corr;
                    let v_raw = (bary0 * v0_uv[1] + bary1 * v1_uv[1] + bary2 * v2_uv[1]) * corr;

                    let u_f = u_raw - (u_raw as i32) as f32;
                    let u_w = if u_f < 0.0 { u_f + 1.0 } else { u_f };
                    let v_f = v_raw - (v_raw as i32) as f32;
                    let v_w = if v_f < 0.0 { v_f + 1.0 } else { v_f };

                    let tx = ((u_w * tex_w_f) as i32).min(tex_w_max).max(0) as u32;
                    let ty = ((v_w * tex_h_f) as i32).min(tex_h_max).max(0) as u32;
                    let texel = unsafe { *tex_data.add((ty * tex_w + tx) as usize) };

                    let tex_r = ((texel >> 16) & 0xFF) as f32;
                    let tex_g = ((texel >> 8) & 0xFF) as f32;
                    let tex_b = (texel & 0xFF) as f32;

                    let r = (lit_r * tex_r * mat_r).min(255.0).max(0.0) as u32;
                    let g = (lit_g * tex_g * mat_g).min(255.0).max(0.0) as u32;
                    let b = (lit_b * tex_b * mat_b).min(255.0).max(0.0) as u32;

                    let color = 0xFF000000 | (r << 16) | (g << 8) | b;

                    unsafe {
                        if depth_mask {
                            *batch.depth_ptr.add(fb_idx) = depth;
                        }
                        *batch.color_ptr.add(fb_idx) = color;
                    }
                }
                w0 += a12; w1 += a20; w2 += a01;
            }
        }
        w0_row += b12; w1_row += b20; w2_row += b01;
    }
}

/// Rasterize one triangle in general path, restricted to [band_min_y, band_max_y].
fn rasterize_tri_band(batch: &BatchParams, tri: &ScreenTri, band_min_y: i32, band_max_y: i32, worker_id: usize) {
    let s0 = &tri.s0;
    let s1 = &tri.s1;
    let s2 = &tri.s2;
    let fb_w = batch.fb_w as i32;
    let fb_h = batch.fb_h as i32;

    let min_x = min3(s0[0], s1[0], s2[0]).max(0.0) as i32;
    let max_x = (crate::rasterizer::math::ceil(max3(s0[0], s1[0], s2[0])) as i32).min(fb_w - 1);
    let min_y = (min3(s0[1], s1[1], s2[1]).max(0.0) as i32).max(band_min_y);
    let max_y = (crate::rasterizer::math::ceil(max3(s0[1], s1[1], s2[1])) as i32).min(fb_h - 1).min(band_max_y);
    if min_x > max_x || min_y > max_y { return; }

    let area = edge_fn(s0, s1, s2);
    if area.abs() < 1e-6 { return; }
    let inv_area = 1.0 / area.abs();

    let v0 = &tri.v0;
    let v1 = &tri.v1;
    let v2 = &tri.v2;

    let w0_clip = v0.position[3];
    let w1_clip = v1.position[3];
    let w2_clip = v2.position[3];
    if w0_clip.abs() < 1e-6 || w1_clip.abs() < 1e-6 || w2_clip.abs() < 1e-6 { return; }

    let inv_w0c = 1.0 / w0_clip;
    let inv_w1c = 1.0 / w1_clip;
    let inv_w2c = 1.0 / w2_clip;

    let nv = batch.num_varyings.min(MAX_VARYINGS);
    let mut v0_persp = [[0.0f32; 4]; MAX_VARYINGS];
    let mut v1_persp = [[0.0f32; 4]; MAX_VARYINGS];
    let mut v2_persp = [[0.0f32; 4]; MAX_VARYINGS];
    for vi in 0..nv {
        let iw0 = Vec4::splat(inv_w0c);
        let iw1 = Vec4::splat(inv_w1c);
        let iw2 = Vec4::splat(inv_w2c);
        Vec4::load(&v0.varyings[vi]).mul(iw0).store(&mut v0_persp[vi]);
        Vec4::load(&v1.varyings[vi]).mul(iw1).store(&mut v1_persp[vi]);
        Vec4::load(&v2.varyings[vi]).mul(iw2).store(&mut v2_persp[vi]);
    }

    let z0 = s0[2]; let z1 = s1[2]; let z2 = s2[2];
    let fb_width = batch.fb_w;
    let tex_sample_addr = raster::real_tex_sample as usize;

    let mut a12 = s1[1] - s2[1];
    let mut b12 = s2[0] - s1[0];
    let mut a20 = s2[1] - s0[1];
    let mut b20 = s0[0] - s2[0];
    let mut a01 = s0[1] - s1[1];
    let mut b01 = s1[0] - s0[0];

    let p0x = min_x as f32 + 0.5;
    let p0y = min_y as f32 + 0.5;
    let mut w0_row = (s2[0] - s1[0]) * (p0y - s1[1]) - (s2[1] - s1[1]) * (p0x - s1[0]);
    let mut w1_row = (s0[0] - s2[0]) * (p0y - s2[1]) - (s0[1] - s2[1]) * (p0x - s2[0]);
    let mut w2_row = (s1[0] - s0[0]) * (p0y - s0[1]) - (s1[1] - s0[1]) * (p0x - s0[0]);

    if area < 0.0 {
        w0_row = -w0_row; w1_row = -w1_row; w2_row = -w2_row;
        a12 = -a12; b12 = -b12;
        a20 = -a20; b20 = -b20;
        a01 = -a01; b01 = -b01;
    }

    let depth_test_enabled = batch.depth_test;
    let depth_func = batch.depth_func;
    let depth_mask = batch.depth_mask;
    let blend_enabled = batch.blend_enabled;
    let blend_src = batch.blend_src;
    let blend_dst = batch.blend_dst;

    let uniforms = unsafe { core::slice::from_raw_parts(batch.uniforms_ptr, batch.uniforms_len) };
    let fs_ir = unsafe { &*batch.fs_ir_ptr };
    let fs_jit = batch.fs_jit;

    // Each worker needs its own ShaderExec (stack-allocated)
    let mut fs_exec = crate::compiler::backend_sw::ShaderExec::new(fs_ir.num_regs, nv);
    let mut varying_buf = [[0.0f32; 4]; MAX_VARYINGS];

    for py in min_y..=max_y {
        let mut span_left = min_x;
        let mut span_right = max_x;
        let mut empty = false;

        macro_rules! edge_clip {
            ($w:expr, $a:expr) => {
                if !empty {
                    let w_val: f32 = $w;
                    let a_val: f32 = $a;
                    if a_val > 1e-8 {
                        if w_val < 0.0 {
                            let x = min_x + crate::rasterizer::math::ceil((-w_val) / a_val) as i32;
                            if x > span_left { span_left = x; }
                        }
                    } else if a_val < -1e-8 {
                        if w_val < 0.0 { empty = true; }
                        else {
                            let x = min_x + (w_val / (-a_val)) as i32;
                            if x < span_right { span_right = x; }
                        }
                    } else if w_val < -1e-8 { empty = true; }
                }
            };
        }

        edge_clip!(w0_row, a12);
        edge_clip!(w1_row, a20);
        edge_clip!(w2_row, a01);

        if !empty && span_left <= span_right {
            let dx = (span_left - min_x) as f32;
            let mut w0 = w0_row + a12 * dx;
            let mut w1 = w1_row + a20 * dx;
            let mut w2 = w2_row + a01 * dx;
            let row_base = py as u32 * fb_width;

            for px in span_left..=span_right {
                if w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0 {
                    let bary0 = w0 * inv_area;
                    let bary1 = w1 * inv_area;
                    let bary2 = w2 * inv_area;

                    let depth = bary0 * z0 + bary1 * z1 + bary2 * z2;
                    let fb_idx = (row_base + px as u32) as usize;

                    if depth_test_enabled {
                        let cur = unsafe { *batch.depth_ptr.add(fb_idx) };
                        if !fragment::depth_test(depth, cur, depth_func) {
                            w0 += a12; w1 += a20; w2 += a01;
                            continue;
                        }
                    }

                    let inv_w = bary0 * inv_w0c + bary1 * inv_w1c + bary2 * inv_w2c;
                    if inv_w.abs() < 1e-10 {
                        w0 += a12; w1 += a20; w2 += a01;
                        continue;
                    }
                    let corr = fast_rcp(inv_w);

                    let b0v = Vec4::splat(bary0);
                    let b1v = Vec4::splat(bary1);
                    let b2v = Vec4::splat(bary2);
                    let corr_v = Vec4::splat(corr);

                    for vi in 0..nv {
                        b0v.mul(Vec4::load(&v0_persp[vi]))
                            .add(b1v.mul(Vec4::load(&v1_persp[vi])))
                            .add(b2v.mul(Vec4::load(&v2_persp[vi])))
                            .mul(corr_v)
                            .store(&mut varying_buf[vi]);
                    }

                    fs_exec.frag_color = [0.0, 0.0, 0.0, 1.0];
                    fs_exec.discarded = false;
                    if let Some(jit) = fs_jit {
                        let mut jit_ctx = crate::compiler::backend_jit::JitContext {
                            regs: fs_exec.regs.as_mut_ptr() as *mut f32,
                            uniforms: uniforms.as_ptr() as *const f32,
                            attributes: core::ptr::null(),
                            varyings_in: varying_buf.as_ptr() as *const f32,
                            varyings_out: core::ptr::null_mut(),
                            position: core::ptr::null_mut(),
                            frag_color: fs_exec.frag_color.as_mut_ptr(),
                            point_size: core::ptr::null_mut(),
                            tex_sample: tex_sample_addr,
                            discarded: 0,
                        };
                        unsafe { jit(&mut jit_ctx); }
                        if jit_ctx.discarded != 0 {
                            w0 += a12; w1 += a20; w2 += a01;
                            continue;
                        }
                    } else {
                        fs_exec.execute(fs_ir, &[], uniforms, Some(&varying_buf[..nv]), raster::real_tex_sample);
                        if fs_exec.discarded {
                            w0 += a12; w1 += a20; w2 += a01;
                            continue;
                        }
                    }

                    let fc = fs_exec.frag_color;
                    let r = (fc[0].clamp(0.0, 1.0) * 255.0) as u32;
                    let g = (fc[1].clamp(0.0, 1.0) * 255.0) as u32;
                    let b = (fc[2].clamp(0.0, 1.0) * 255.0) as u32;
                    let a = (fc[3].clamp(0.0, 1.0) * 255.0) as u32;
                    let color = (a << 24) | (r << 16) | (g << 8) | b;

                    let final_color = if blend_enabled {
                        let dst = unsafe { *batch.color_ptr.add(fb_idx) };
                        fragment::blend(color, dst, blend_src, blend_dst)
                    } else {
                        color
                    };

                    unsafe {
                        if depth_mask {
                            *batch.depth_ptr.add(fb_idx) = depth;
                        }
                        *batch.color_ptr.add(fb_idx) = final_color;
                    }
                }
                w0 += a12; w1 += a20; w2 += a01;
            }
        }
        w0_row += b12; w1_row += b20; w2_row += b01;
    }
}
