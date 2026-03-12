//! Tile-based parallel rasterization thread pool.
//!
//! Frame-level batching: draw calls collect triangles + render state into
//! sub-batches during the frame. At `gl_swap_buffers`, all sub-batches are
//! submitted to workers in one go — ONE wake-up per frame, not per draw call.
//!
//! Fully transparent to client applications — the pool is initialized
//! automatically at `gl_init` and workers are cleaned up at `gl_deinit`
//! (or when the process exits, if the kernel supports it).

use core::sync::atomic::{AtomicU32, Ordering};
use crate::syscall;
use crate::rasterizer::ClipVertex;
use crate::rasterizer::raster::ResolvedTexture;

/// Maximum number of worker threads (hard cap).
const MAX_WORKERS: usize = 7;

/// Maximum triangles per frame (screen-space, after clipping).
const MAX_TRIS: usize = 16384;

/// Maximum sub-batches per frame (one per draw call).
const MAX_SUB_BATCHES: usize = 64;

/// Stack size per worker thread (64 KiB — needs room for ShaderExec).
const WORKER_STACK_SIZE: usize = 64 * 1024;

// ── Shared work data ─────────────────────────────────────────────────────

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

/// Per-draw-call render state + triangle range.
#[repr(C)]
struct SubBatch {
    tri_start: u32,
    tri_end: u32,

    // Rasterization parameters
    depth_test: bool,
    depth_func: u32,
    depth_mask: bool,
    blend_enabled: bool,
    blend_src: u32,
    blend_dst: u32,

    // Fast path texture
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

unsafe impl Send for SubBatch {}
unsafe impl Sync for SubBatch {}

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

    // Framebuffer pointers (set once at flush, read by workers)
    color_ptr: *mut u32,
    depth_ptr: *mut f32,
    fb_w: u32,
    fb_h: u32,

    // Triangle buffer (filled during draw calls)
    tri_buf: [ScreenTri; MAX_TRIS],
    tri_count: usize,

    // Sub-batch list (filled during draw calls, processed at flush)
    sub_batches: [SubBatch; MAX_SUB_BATCHES],
    sub_batch_count: usize,
}

// Worker thread IDs and stacks
static mut WORKER_TIDS: [u32; MAX_WORKERS] = [0; MAX_WORKERS];
static mut WORKER_STACKS: [u64; MAX_WORKERS] = [0; MAX_WORKERS];

/// Initialize the thread pool with `n` workers.
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
            color_ptr: core::ptr::null_mut(),
            depth_ptr: core::ptr::null_mut(),
            fb_w: 0,
            fb_h: 0,
            tri_buf: core::mem::zeroed(),
            tri_count: 0,
            sub_batches: core::mem::zeroed(),
            sub_batch_count: 0,
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
            worker_entry_0, worker_entry_1, worker_entry_2, worker_entry_3,
            worker_entry_4, worker_entry_5, worker_entry_6,
        ];

        for i in 0..n {
            let stack_addr = syscall::mmap(WORKER_STACK_SIZE as u32);
            if stack_addr == u64::MAX || stack_addr == 0 {
                crate::serial_println!("[libgl] thread_pool: mmap failed for worker {}", i);
                pool.num_workers = i;
                return;
            }
            WORKER_STACKS[i] = stack_addr;
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

// Worker entry points
fn worker_entry_0() { worker_main(0); }
fn worker_entry_1() { worker_main(1); }
fn worker_entry_2() { worker_main(2); }
fn worker_entry_3() { worker_main(3); }
fn worker_entry_4() { worker_main(4); }
fn worker_entry_5() { worker_main(5); }
fn worker_entry_6() { worker_main(6); }

/// Worker main loop. Sleeps until work arrives, processes ALL sub-batches
/// for its y-band, then signals done.
fn worker_main(id: usize) {
    loop {
        let pool = unsafe { POOL.as_ref().unwrap() };
        let ctl = &pool.workers[id];

        // Wait for work — tight spin first, then sleep
        let mut idle_spins: u32 = 0;
        loop {
            let s = ctl.state.load(Ordering::Acquire);
            if s == WORKER_WORK { break; }
            if s == WORKER_EXIT { return; }
            if idle_spins < 50 {
                core::hint::spin_loop();
            } else {
                syscall::sleep_us(100);
            }
            idle_spins = idle_spins.saturating_add(1);
        }

        // Process all sub-batches for our y-band
        let pool = unsafe { POOL.as_ref().unwrap() };
        let min_y = ctl.band_min_y as i32;
        let max_y = ctl.band_max_y as i32 - 1; // inclusive

        for sb_idx in 0..pool.sub_batch_count {
            let sb = &pool.sub_batches[sb_idx];
            let start = sb.tri_start as usize;
            let end = sb.tri_end as usize;
            if start >= end { continue; }

            let tris = &pool.tri_buf[start..end];
            for tri in tris {
                if sb.use_fast_path {
                    rasterize_tri_fast_band(pool, sb, tri, min_y, max_y);
                } else {
                    rasterize_tri_band(pool, sb, tri, min_y, max_y, id);
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
fn optimal_worker_count() -> usize {
    let raw = syscall::cpu_count();
    let cpus = if raw == 0 || raw > 64 { 8 } else { raw as usize };
    crate::serial_println!("[libgl] thread_pool: raw cpu_count={}, effective={}", raw, cpus);
    if cpus <= 1 { 1 } else { (cpus - 1).min(MAX_WORKERS) }
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
            for i in 0..n {
                pool.workers[i].state.store(WORKER_EXIT, Ordering::Release);
            }
            syscall::sleep_us(2000);
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

// ── Frame-level batching API ─────────────────────────────────────────────

/// Append triangles from one draw call into the frame buffer.
/// Returns the number of triangles actually added.
pub fn append_tris(tris: &[ScreenTri]) -> usize {
    let pool = unsafe { match POOL.as_mut() { Some(p) => p, None => return 0 } };
    let avail = MAX_TRIS - pool.tri_count;
    let n = tris.len().min(avail);
    if n == 0 { return 0; }
    pool.tri_buf[pool.tri_count..pool.tri_count + n].copy_from_slice(&tris[..n]);
    pool.tri_count += n;
    n
}

/// Begin a sub-batch for the current draw call's render state.
/// Call this BEFORE `append_tris`. Returns the tri_start index.
pub fn begin_sub_batch() -> Option<u32> {
    let pool = unsafe { match POOL.as_mut() { Some(p) => p, None => return None } };
    if pool.sub_batch_count >= MAX_SUB_BATCHES { return None; }
    Some(pool.tri_count as u32)
}

/// Finalize a sub-batch after appending triangles.
pub fn end_sub_batch(
    tri_start: u32,
    depth_test: bool,
    depth_func: u32,
    depth_mask: bool,
    blend_enabled: bool,
    blend_src: u32,
    blend_dst: u32,
    fast: Option<(&ResolvedTexture, f32, f32, f32)>,
    fs_ir: *const crate::compiler::ir::Program,
    uniforms: &[[f32; 4]],
    num_varyings: usize,
    fs_jit: Option<crate::compiler::backend_jit::JitFn>,
) {
    let pool = unsafe { match POOL.as_mut() { Some(p) => p, None => return } };
    let tri_end = pool.tri_count as u32;
    if tri_end <= tri_start { return; } // no triangles added
    if pool.sub_batch_count >= MAX_SUB_BATCHES { return; }

    let idx = pool.sub_batch_count;
    let sb = &mut pool.sub_batches[idx];
    sb.tri_start = tri_start;
    sb.tri_end = tri_end;
    sb.depth_test = depth_test;
    sb.depth_func = depth_func;
    sb.depth_mask = depth_mask;
    sb.blend_enabled = blend_enabled;
    sb.blend_src = blend_src;
    sb.blend_dst = blend_dst;
    sb.fs_ir_ptr = fs_ir;
    sb.uniforms_ptr = uniforms.as_ptr();
    sb.uniforms_len = uniforms.len();
    sb.num_varyings = num_varyings;
    sb.fs_jit = fs_jit;

    if let Some((tex, mr, mg, mb)) = fast {
        sb.use_fast_path = true;
        sb.fast_tex_data = tex.data;
        sb.fast_tex_w = tex.width;
        sb.fast_tex_h = tex.height;
        sb.fast_tex_len = tex.len;
        sb.fast_mat_r = mr;
        sb.fast_mat_g = mg;
        sb.fast_mat_b = mb;
    } else {
        sb.use_fast_path = false;
    }

    pool.sub_batch_count = idx + 1;
}

/// Flush all accumulated sub-batches: wake workers, wait for completion, reset.
/// Called from `gl_swap_buffers`.
pub fn flush_frame(ctx: &mut crate::state::GlContext) {
    let pool = unsafe { match POOL.as_mut() { Some(p) => p, None => return } };
    let n = pool.num_workers;
    if n == 0 || pool.sub_batch_count == 0 {
        pool.tri_count = 0;
        pool.sub_batch_count = 0;
        return;
    }

    // Set framebuffer pointers for this frame
    pool.color_ptr = ctx.default_fb.color.as_mut_ptr();
    pool.depth_ptr = ctx.default_fb.depth.as_mut_ptr();
    pool.fb_w = ctx.default_fb.width;
    pool.fb_h = ctx.default_fb.height;

    static mut FLUSH_DBG: u32 = 0;
    if unsafe { FLUSH_DBG } < 5 {
        unsafe { FLUSH_DBG += 1; }
        crate::serial_println!("[libgl] flush_frame: {} sub-batches, {} tris, {} workers",
            pool.sub_batch_count, pool.tri_count, n);
    }

    // Signal all workers to start
    for i in 0..n {
        pool.workers[i].state.store(WORKER_WORK, Ordering::Release);
    }

    // Wait for all workers to finish
    for i in 0..n {
        let mut wait_spins: u32 = 0;
        loop {
            let s = pool.workers[i].state.load(Ordering::Acquire);
            if s == WORKER_DONE { break; }
            if wait_spins < 100 {
                core::hint::spin_loop();
            } else {
                syscall::sleep_us(10);
            }
            wait_spins += 1;
        }
        pool.workers[i].state.store(WORKER_IDLE, Ordering::Release);
    }

    // Reset for next frame
    pool.tri_count = 0;
    pool.sub_batch_count = 0;
}

/// Get mutable ref to the triangle buffer for direct filling.
pub fn tri_buffer_slice(start: usize, count: usize) -> Option<&'static mut [ScreenTri]> {
    let pool = unsafe { POOL.as_mut()? };
    let end = start + count;
    if end > MAX_TRIS { return None; }
    Some(&mut pool.tri_buf[start..end])
}

/// Current triangle count.
pub fn tri_count() -> usize {
    unsafe { POOL.as_ref().map_or(0, |p| p.tri_count) }
}

/// Add to triangle count (after direct buffer fill).
pub fn advance_tri_count(n: usize) {
    unsafe { if let Some(p) = POOL.as_mut() { p.tri_count = (p.tri_count + n).min(MAX_TRIS); } }
}

/// Maximum triangles the buffer can hold.
pub fn max_tris() -> usize { MAX_TRIS }

// ── Band-restricted rasterization ──────────────────────────────────────────

use crate::rasterizer::raster::{self, edge_fn, min3, max3, fast_rcp};
use crate::rasterizer::fragment;
use crate::rasterizer::MAX_VARYINGS;
use crate::simd::Vec4;

/// Rasterize one triangle in fast path, restricted to [band_min_y, band_max_y].
fn rasterize_tri_fast_band(pool: &ThreadPool, sb: &SubBatch, tri: &ScreenTri, band_min_y: i32, band_max_y: i32) {
    let s0 = &tri.s0;
    let s1 = &tri.s1;
    let s2 = &tri.s2;
    let fb_w = pool.fb_w as i32;
    let fb_h = pool.fb_h as i32;

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
    let fb_width = pool.fb_w;

    let tex_data = sb.fast_tex_data;
    let tex_w = sb.fast_tex_w;
    let tex_h = sb.fast_tex_h;
    let tex_w_f = tex_w as f32;
    let tex_h_f = tex_h as f32;
    let tex_w_max = (tex_w - 1) as i32;
    let tex_h_max = (tex_h - 1) as i32;
    let mat_r = sb.fast_mat_r;
    let mat_g = sb.fast_mat_g;
    let mat_b = sb.fast_mat_b;

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

    let depth_test = sb.depth_test;
    let depth_func = sb.depth_func;
    let depth_mask = sb.depth_mask;

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
                        let cur = unsafe { *pool.depth_ptr.add(fb_idx) };
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
                            *pool.depth_ptr.add(fb_idx) = depth;
                        }
                        *pool.color_ptr.add(fb_idx) = color;
                    }
                }
                w0 += a12; w1 += a20; w2 += a01;
            }
        }
        w0_row += b12; w1_row += b20; w2_row += b01;
    }
}

/// Rasterize one triangle in general path, restricted to [band_min_y, band_max_y].
fn rasterize_tri_band(pool: &ThreadPool, sb: &SubBatch, tri: &ScreenTri, band_min_y: i32, band_max_y: i32, _worker_id: usize) {
    let s0 = &tri.s0;
    let s1 = &tri.s1;
    let s2 = &tri.s2;
    let fb_w = pool.fb_w as i32;
    let fb_h = pool.fb_h as i32;

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

    let nv = sb.num_varyings.min(MAX_VARYINGS);
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
    let fb_width = pool.fb_w;
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

    let depth_test_enabled = sb.depth_test;
    let depth_func = sb.depth_func;
    let depth_mask = sb.depth_mask;
    let blend_enabled = sb.blend_enabled;
    let blend_src = sb.blend_src;
    let blend_dst = sb.blend_dst;

    let uniforms = unsafe { core::slice::from_raw_parts(sb.uniforms_ptr, sb.uniforms_len) };
    let fs_ir = unsafe { &*sb.fs_ir_ptr };
    let fs_jit = sb.fs_jit;

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
                        let cur = unsafe { *pool.depth_ptr.add(fb_idx) };
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
                        let dst = unsafe { *pool.color_ptr.add(fb_idx) };
                        fragment::blend(color, dst, blend_src, blend_dst)
                    } else {
                        color
                    };

                    unsafe {
                        if depth_mask {
                            *pool.depth_ptr.add(fb_idx) = depth;
                        }
                        *pool.color_ptr.add(fb_idx) = final_color;
                    }
                }
                w0 += a12; w1 += a20; w2 += a01;
            }
        }
        w0_row += b12; w1_row += b20; w2_row += b01;
    }
}
