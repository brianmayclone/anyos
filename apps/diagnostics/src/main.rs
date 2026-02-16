#![no_std]
#![no_main]

use anyos_std::fs;
use anyos_std::process;
use anyos_std::sys;
use anyos_std::ui::window;
use anyos_std::Vec;

anyos_std::entry!(main);

use uisys_client::*;

// ── Layout ──────────────────────────────────────────────────────────────────

const WIN_W: u32 = 480;
const WIN_H: u32 = 440;
const PAD: i32 = 10;
const ROW_H: i32 = 18;

// ── Timing helper ───────────────────────────────────────────────────────────

fn ticks_now() -> u32 {
    sys::uptime()
}

fn ticks_to_us(ticks: u32, hz: u32) -> u32 {
    if hz == 0 { return 0; }
    // ticks * 1_000_000 / hz — avoid overflow with u64
    ((ticks as u64) * 1_000_000 / (hz as u64)) as u32
}

fn ticks_to_sec(ticks: u32, hz: u32) -> u32 {
    if hz == 0 { return 0; }
    ticks / hz
}

// ── Serial logging (uses anyos_std::println! which goes to serial) ──────────

macro_rules! log {
    ($($arg:tt)*) => {
        anyos_std::println!($($arg)*);
    };
}

// ── Benchmark results ───────────────────────────────────────────────────────

const MAX_RESULTS: usize = 24;

struct BenchResult {
    name: [u8; 40],
    name_len: usize,
    value_us: u32,
    iterations: u32,
}

struct BenchState {
    results: [Option<BenchResult>; MAX_RESULTS],
    count: usize,
    running: bool,
    done: bool,
    current_test: usize,
    total_tests: usize,
}

impl BenchState {
    fn new() -> Self {
        const NONE: Option<BenchResult> = None;
        Self {
            results: [NONE; MAX_RESULTS],
            count: 0,
            running: false,
            done: false,
            current_test: 0,
            total_tests: 10,
        }
    }

    fn add(&mut self, name: &str, value_us: u32, iterations: u32) {
        if self.count >= MAX_RESULTS { return; }
        let mut n = [0u8; 40];
        let len = name.len().min(40);
        n[..len].copy_from_slice(&name.as_bytes()[..len]);
        self.results[self.count] = Some(BenchResult {
            name: n,
            name_len: len,
            value_us,
            iterations,
        });
        self.count += 1;
    }
}

// ── Stress test state ───────────────────────────────────────────────────────

struct StressState {
    running: bool,
    iterations: u32,
    total_syscalls: u64,
    errors: u32,
    start_tick: u32,
    last_render_tick: u32,
}

impl StressState {
    fn new() -> Self {
        Self {
            running: false,
            iterations: 0,
            total_syscalls: 0,
            errors: 0,
            start_tick: 0,
            last_render_tick: 0,
        }
    }
}

// ── Benchmark tests ─────────────────────────────────────────────────────────

fn bench_syscall_overhead(state: &mut BenchState, hz: u32) {
    log!("=== Test: Syscall Overhead (uptime) ===");
    let iters = 1000u32;
    let t0 = ticks_now();
    for _ in 0..iters {
        let _ = sys::uptime();
    }
    let t1 = ticks_now();
    let us = ticks_to_us(t1.wrapping_sub(t0), hz);
    let per = us / iters;
    log!("  {} iterations in {} us ({} us/call)", iters, us, per);
    state.add("Syscall (uptime)", per, iters);
}

fn bench_file_stat(state: &mut BenchState, hz: u32) {
    log!("=== Test: File stat() ===");
    let iters = 100u32;
    let mut stat_buf = [0u32; 2];
    let t0 = ticks_now();
    for _ in 0..iters {
        let _ = fs::stat("/Applications/Diagnostics.app", &mut stat_buf);
    }
    let t1 = ticks_now();
    let us = ticks_to_us(t1.wrapping_sub(t0), hz);
    let per = if iters > 0 { us / iters } else { 0 };
    log!("  {} iterations in {} us ({} us/call)", iters, us, per);
    state.add("File stat()", per, iters);
}

fn bench_file_open_close(state: &mut BenchState, hz: u32) {
    log!("=== Test: File open()+close() ===");
    let iters = 100u32;
    let t0 = ticks_now();
    for _ in 0..iters {
        let fd = fs::open("/System/fonts/sfpro.ttf", 0);
        if fd != u32::MAX {
            fs::close(fd);
        }
    }
    let t1 = ticks_now();
    let us = ticks_to_us(t1.wrapping_sub(t0), hz);
    let per = if iters > 0 { us / iters } else { 0 };
    log!("  {} iterations in {} us ({} us/call)", iters, us, per);
    state.add("File open+close", per, iters);
}

fn bench_file_read_small(state: &mut BenchState, hz: u32) {
    log!("=== Test: File read 4 KiB ===");
    let iters = 50u32;
    let mut buf = [0u8; 4096];
    let t0 = ticks_now();
    for _ in 0..iters {
        let fd = fs::open("/System/fonts/sfpro.ttf", 0);
        if fd != u32::MAX {
            fs::read(fd, &mut buf);
            fs::close(fd);
        }
    }
    let t1 = ticks_now();
    let us = ticks_to_us(t1.wrapping_sub(t0), hz);
    let per = if iters > 0 { us / iters } else { 0 };
    log!("  {} iterations in {} us ({} us/call)", iters, us, per);
    state.add("Read 4K file", per, iters);
}

fn bench_file_read_large(state: &mut BenchState, hz: u32) {
    log!("=== Test: File read large (sfpro.ttf ~166 KiB) ===");
    let iters = 10u32;
    let mut buf = [0u8; 8192];
    let mut total_bytes = 0u64;
    let t0 = ticks_now();
    for _ in 0..iters {
        let fd = fs::open("/System/fonts/sfpro.ttf", 0);
        if fd != u32::MAX {
            loop {
                let n = fs::read(fd, &mut buf);
                if n == 0 || n == u32::MAX { break; }
                total_bytes += n as u64;
            }
            fs::close(fd);
        }
    }
    let t1 = ticks_now();
    let us = ticks_to_us(t1.wrapping_sub(t0), hz);
    let per = if iters > 0 { us / iters } else { 0 };
    let kb = total_bytes / 1024;
    log!("  {} iterations, {} KiB total, {} us ({} us/read)", iters, kb, us, per);
    state.add("Read ~166K file", per, iters);
}

fn bench_file_read_testdata(state: &mut BenchState, hz: u32) {
    log!("=== Test: File read testdata (64 KiB) ===");
    let iters = 20u32;
    let mut buf = [0u8; 8192];
    let mut total_bytes = 0u64;
    let t0 = ticks_now();
    for _ in 0..iters {
        let fd = fs::open("/System/testdata.bin", 0);
        if fd != u32::MAX {
            loop {
                let n = fs::read(fd, &mut buf);
                if n == 0 || n == u32::MAX { break; }
                total_bytes += n as u64;
            }
            fs::close(fd);
        } else {
            log!("  WARN: /System/testdata.bin not found, skipping");
            state.add("Read 64K testdata", 0, 0);
            return;
        }
    }
    let t1 = ticks_now();
    let us = ticks_to_us(t1.wrapping_sub(t0), hz);
    let per = if iters > 0 { us / iters } else { 0 };
    let kb = total_bytes / 1024;
    log!("  {} iterations, {} KiB total, {} us ({} us/read)", iters, kb, us, per);
    state.add("Read 64K testdata", per, iters);
}

fn bench_heap_alloc(state: &mut BenchState, hz: u32) {
    log!("=== Test: Heap alloc+drop 4 KiB Vec ===");
    let iters = 500u32;
    let t0 = ticks_now();
    for _ in 0..iters {
        let v: Vec<u8> = Vec::with_capacity(4096);
        core::hint::black_box(&v);
        drop(v);
    }
    let t1 = ticks_now();
    let us = ticks_to_us(t1.wrapping_sub(t0), hz);
    let per = if iters > 0 { us / iters } else { 0 };
    log!("  {} iterations in {} us ({} us/call)", iters, us, per);
    state.add("Heap alloc 4K", per, iters);
}

fn bench_window_create_destroy(state: &mut BenchState, hz: u32) {
    log!("=== Test: Window create+destroy ===");
    let iters = 5u32;
    let mut total_create = 0u32;
    let mut total_destroy = 0u32;
    for i in 0..iters {
        let tc0 = ticks_now();
        let wid = window::create("BenchWin", 50, 50, 200, 150);
        let tc1 = ticks_now();
        if wid == u32::MAX {
            log!("  iter {}: create FAILED", i);
            continue;
        }
        let td0 = ticks_now();
        window::destroy(wid);
        let td1 = ticks_now();

        let c_us = ticks_to_us(tc1.wrapping_sub(tc0), hz);
        let d_us = ticks_to_us(td1.wrapping_sub(td0), hz);
        total_create += c_us;
        total_destroy += d_us;
        log!("  iter {}: create={} us, destroy={} us", i, c_us, d_us);

        // Small delay between iterations to let compositor process
        process::sleep(50);
    }
    let avg_c = if iters > 0 { total_create / iters } else { 0 };
    let avg_d = if iters > 0 { total_destroy / iters } else { 0 };
    log!("  avg create={} us, avg destroy={} us", avg_c, avg_d);
    state.add("Window create", avg_c, iters);
    state.add("Window destroy", avg_d, iters);
}

fn bench_window_render(state: &mut BenchState, hz: u32) {
    log!("=== Test: Window fill+present ===");
    let wid = window::create("RenderBench", 50, 50, 320, 240);
    if wid == u32::MAX {
        log!("  FAILED to create window");
        state.add("Window render", 0, 0);
        return;
    }
    let iters = 20u32;
    let t0 = ticks_now();
    for _ in 0..iters {
        window::fill_rect(wid, 0, 0, 320, 240, 0xFF2D2D2D);
        label(wid, 10, 10, "Benchmark", 0xFFFFFFFF, FontSize::Normal, TextAlign::Left);
        card(wid, 10, 30, 300, 60);
        progress(wid, 20, 50, 280, 8, 75);
        window::present(wid);
    }
    let t1 = ticks_now();
    window::destroy(wid);

    let us = ticks_to_us(t1.wrapping_sub(t0), hz);
    let per = if iters > 0 { us / iters } else { 0 };
    log!("  {} frames in {} us ({} us/frame)", iters, us, per);
    state.add("Window render", per, iters);
}

fn bench_sleep_accuracy(state: &mut BenchState, hz: u32) {
    log!("=== Test: Sleep accuracy (100ms target) ===");
    let t0 = ticks_now();
    process::sleep(100);
    let t1 = ticks_now();
    let us = ticks_to_us(t1.wrapping_sub(t0), hz);
    log!("  sleep(100) actual={} us (target=100000 us)", us);
    state.add("Sleep(100ms) actual", us, 1);
}

fn bench_process_spawn(state: &mut BenchState, hz: u32) {
    log!("=== Test: Process spawn (/bin/ls) ===");
    let iters = 3u32;
    let mut total = 0u32;
    for i in 0..iters {
        let t0 = ticks_now();
        let tid = process::spawn("/System/bin/ls", "/");
        let t1 = ticks_now();
        let us = ticks_to_us(t1.wrapping_sub(t0), hz);
        total += us;
        log!("  iter {}: spawn={} us (tid={})", i, us, tid);
        // Wait for it to finish
        if tid != u32::MAX && tid != 0 {
            process::sleep(200);
        }
    }
    let avg = if iters > 0 { total / iters } else { 0 };
    log!("  avg spawn={} us", avg);
    state.add("Spawn /bin/ls", avg, iters);
}

// ── Stress test (one iteration) ─────────────────────────────────────────────

/// Run one iteration of the stress test: a mix of syscalls that exercises
/// different kernel paths, uisys DLL function pointers, and compositor IPC.
/// Returns (syscall_count, error_count).
fn stress_iteration(hz: u32, win_id: u32) -> (u32, u32) {
    let mut calls: u32 = 0;
    let mut errors: u32 = 0;

    // 1. Syscall burst: 200x uptime (fast SYSCALL/SYSRET path)
    for _ in 0..200 {
        let _ = sys::uptime();
        calls += 1;
    }

    // 2. File operations: open + read + close (exercises VFS + storage syscalls)
    for _ in 0..10 {
        let fd = fs::open("/System/fonts/sfpro.ttf", 0);
        calls += 1;
        if fd != u32::MAX {
            let mut buf = [0u8; 512];
            let n = fs::read(fd, &mut buf);
            calls += 1;
            if n == u32::MAX { errors += 1; }
            fs::close(fd);
            calls += 1;
        } else {
            errors += 1;
        }
    }

    // 3. File stat (exercises directory traversal)
    for _ in 0..20 {
        let mut stat_buf = [0u32; 2];
        let r = fs::stat("/Applications/Diagnostics.app", &mut stat_buf);
        calls += 1;
        if r == u32::MAX { errors += 1; }
    }

    // 4. Heap alloc + free (exercises sbrk syscall under the hood)
    for _ in 0..50 {
        let v: Vec<u8> = Vec::with_capacity(1024);
        core::hint::black_box(&v);
        drop(v);
        calls += 1;
    }

    // 5. Sleep (exercises scheduler voluntary yield + timer wakeup)
    process::sleep(1);
    calls += 1;

    // 6. Tick Hz query (another fast syscall)
    for _ in 0..50 {
        let _ = sys::tick_hz();
        calls += 1;
    }

    // 7. getpid (fast syscall, exercises current_tid)
    for _ in 0..50 {
        let _ = process::getpid();
        calls += 1;
    }

    // 8. uisys DLL components — exercises DLL shared page function pointers +
    //    kernel drawing syscalls (SYS_WIN_FILL_RECT, SYS_WIN_DRAW_TEXT, etc.)
    //    Draws into the lower region of the window (overwritten by next render).
    {
        let y0: i32 = 300;

        // window::fill_rect — direct compositor syscall
        window::fill_rect(win_id, 0, y0 as i16, WIN_W as u16, 140, colors::WINDOW_BG());
        calls += 1;

        // label — DLL label_render → SYS_WIN_DRAW_TEXT_EX
        label(win_id, 10, y0, "Stress", colors::TEXT(), FontSize::Small, TextAlign::Left);
        calls += 1;
        label(win_id, 60, y0, "Testing", colors::TEXT_SECONDARY(), FontSize::Normal, TextAlign::Left);
        calls += 1;

        // label_measure — DLL label_measure → SYS_FONT_MEASURE
        let _ = label_measure("Measure test string", FontSize::Normal);
        calls += 1;

        // label_ellipsis — DLL label_render_ellipsis
        label_ellipsis(win_id, 10, y0 + 18, "A long label truncated with ellipsis", colors::TEXT(), FontSize::Small, 200);
        calls += 1;

        // button — DLL button_render (3 styles)
        button(win_id, 10, y0 + 36, 80, 28, "OK", ButtonStyle::Primary, ButtonState::Normal);
        calls += 1;
        button(win_id, 100, y0 + 36, 80, 28, "Cancel", ButtonStyle::Default, ButtonState::Hover);
        calls += 1;
        button(win_id, 190, y0 + 36, 80, 28, "Delete", ButtonStyle::Destructive, ButtonState::Normal);
        calls += 1;

        // button_measure + button_hit_test — DLL-only (no kernel syscall)
        let _ = button_measure("Measure");
        calls += 1;
        let _ = button_hit_test(10, y0 + 36, 80, 28, 50, y0 + 50);
        calls += 1;

        // toggle — DLL toggle_render
        toggle(win_id, 280, y0 + 36, true);
        calls += 1;
        toggle(win_id, 340, y0 + 36, false);
        calls += 1;

        // checkbox — DLL checkbox_render
        checkbox(win_id, 10, y0 + 70, CheckboxState::Checked, "Opt A");
        calls += 1;
        checkbox(win_id, 100, y0 + 70, CheckboxState::Unchecked, "Opt B");
        calls += 1;

        // radio — DLL radio_render
        radio(win_id, 200, y0 + 70, true, "Yes");
        calls += 1;
        radio(win_id, 270, y0 + 70, false, "No");
        calls += 1;

        // slider — DLL slider_render
        slider(win_id, 10, y0 + 94, 200, 0, 100, 42, 20);
        calls += 1;

        // progress — DLL progress_render
        progress(win_id, 220, y0 + 94, 150, 8, 66);
        calls += 1;

        // badge — DLL badge_render
        badge(win_id, 400, y0, 99);
        calls += 1;
        badge_dot(win_id, 440, y0);
        calls += 1;

        // stepper — DLL stepper_render
        stepper(win_id, 380, y0 + 36, 7, 0, 10);
        calls += 1;

        // divider — DLL divider_render_h / _v
        divider_h(win_id, 10, y0 + 116, 460);
        calls += 1;
        divider_v(win_id, 240, y0, 116);
        calls += 1;

        // status_indicator — DLL status_render
        status_indicator(win_id, 10, y0 + 120, StatusKind::Online, "OK");
        calls += 1;
        status_indicator(win_id, 80, y0 + 120, StatusKind::Error, "Fail");
        calls += 1;

        // colorwell — DLL colorwell_render
        colorwell(win_id, 170, y0 + 120, 16, 0xFF007AFF);
        calls += 1;

        // card — DLL card_render
        card(win_id, 200, y0 + 118, 80, 24);
        calls += 1;

        // groupbox — DLL groupbox_render
        groupbox(win_id, 300, y0 + 70, 160, 50, "Group");
        calls += 1;

        // tooltip — DLL tooltip_render
        tooltip(win_id, 300, y0 + 118, "Tooltip text");
        calls += 1;

        // iconbutton — DLL iconbutton_render (circle + square)
        iconbutton(win_id, 420, y0 + 70, 24, 0, 0xFF007AFF);
        calls += 1;
        iconbutton(win_id, 450, y0 + 70, 24, 1, 0xFFFF3B30);
        calls += 1;

        // fill_rounded_rect_aa — DLL → kernel AA fill syscall
        fill_rounded_rect_aa(win_id, 10, y0 + 136, 120, 20, 6, 0xFF3C3C3C);
        calls += 1;

        // draw_text_with_font — DLL → kernel font render syscall
        draw_text_with_font(win_id, 150, y0 + 136, 0xFFFFFFFF, 13, 0, "Font test");
        calls += 1;

        // font_measure — DLL → kernel font measure syscall
        let _ = font_measure(0, 13, "Measure me");
        calls += 1;

        // gpu_has_accel — DLL → kernel GPU query syscall
        let _ = gpu_has_accel();
        calls += 1;

        // window::get_size — compositor query
        let _ = window::get_size(win_id);
        calls += 1;
    }

    (calls, errors)
}

// ── Rendering ───────────────────────────────────────────────────────────────

/// UI mode
#[derive(PartialEq, Clone, Copy)]
enum Mode {
    Idle,
    Bench,
    BenchDone,
    Stress,
    StressDone,
}

fn render_main(
    win_id: u32,
    mode: Mode,
    bench: &BenchState,
    stress: &StressState,
    hz: u32,
    btn_start: &UiToolbarButton,
    btn_stress: &UiToolbarButton,
) {
    window::fill_rect(win_id, 0, 0, WIN_W as u16, WIN_H as u16, colors::WINDOW_BG());

    // Title bar
    card(win_id, 0, 0, WIN_W, 36);
    label(win_id, PAD, 8, "System Diagnostics", 0xFF00C8FF, FontSize::Normal, TextAlign::Left);

    match mode {
        Mode::Idle => {
            btn_start.render(win_id, "Start");
            btn_stress.render(win_id, "Stress Test");

            label(win_id, PAD, 50, "Press Start to run benchmarks,", colors::TEXT(), FontSize::Normal, TextAlign::Left);
            label(win_id, PAD, 70, "or Stress Test for a continuous loop.", colors::TEXT(), FontSize::Normal, TextAlign::Left);
            label(win_id, PAD, 96, "Stress Test runs syscalls in a loop until", colors::TEXT_SECONDARY(), FontSize::Small, TextAlign::Left);
            label(win_id, PAD, 112, "you press Stop. Results go to serial.", colors::TEXT_SECONDARY(), FontSize::Small, TextAlign::Left);
        }
        Mode::Bench => {
            let pct = if bench.total_tests > 0 {
                (bench.current_test as u32 * 100 / bench.total_tests as u32).min(100)
            } else { 0 };
            let mut pbuf = [0u8; 40];
            let ps = fmt_running(&mut pbuf, bench.current_test, bench.total_tests);
            label(win_id, PAD, 50, ps, colors::TEXT(), FontSize::Normal, TextAlign::Left);
            progress(win_id, PAD, 72, WIN_W - PAD as u32 * 2, 10, pct);
        }
        Mode::BenchDone => {
            btn_start.render(win_id, "Re-run");
            btn_stress.render(win_id, "Stress Test");
            render_bench_results(win_id, bench);
        }
        Mode::Stress => {
            btn_start.render(win_id, "Stop");
            render_stress_status(win_id, stress, hz);
        }
        Mode::StressDone => {
            btn_start.render(win_id, "Re-run");
            btn_stress.render(win_id, "Stress Test");
            render_stress_results(win_id, stress, hz);
        }
    }

    window::present(win_id);
}

fn render_bench_results(win_id: u32, state: &BenchState) {
    let mut y: i32 = 42;

    window::fill_rect(win_id, 0, y as i16, WIN_W as u16, ROW_H as u16, 0xFF4A4A4A);
    label(win_id, PAD, y + 2, "Test", colors::TEXT(), FontSize::Small, TextAlign::Left);
    label(win_id, 280, y + 2, "Time", colors::TEXT(), FontSize::Small, TextAlign::Left);
    label(win_id, 370, y + 2, "Iters", colors::TEXT(), FontSize::Small, TextAlign::Left);
    y += ROW_H;

    for i in 0..state.count {
        if y + ROW_H > WIN_H as i32 - 10 { break; }
        if let Some(ref r) = state.results[i] {
            if i % 2 == 1 {
                window::fill_rect(win_id, 0, y as i16, WIN_W as u16, ROW_H as u16, 0xFF333333);
            }
            let name = unsafe { core::str::from_utf8_unchecked(&r.name[..r.name_len]) };
            label(win_id, PAD, y + 2, name, colors::TEXT(), FontSize::Small, TextAlign::Left);

            let mut vbuf = [0u8; 24];
            let vs = fmt_us(&mut vbuf, r.value_us);
            label(win_id, 280, y + 2, vs, 0xFF00FF80, FontSize::Small, TextAlign::Left);

            let mut ibuf = [0u8; 12];
            let is = fmt_u32(&mut ibuf, r.iterations);
            label(win_id, 370, y + 2, is, colors::TEXT_SECONDARY(), FontSize::Small, TextAlign::Left);

            y += ROW_H;
        }
    }
}

fn render_stress_status(win_id: u32, stress: &StressState, hz: u32) {
    let elapsed = ticks_to_sec(ticks_now().wrapping_sub(stress.start_tick), hz);

    let mut y: i32 = 50;

    // Status line
    let status_color = if stress.errors == 0 { 0xFF00FF80 } else { 0xFFFF4040 };
    let status_text = if stress.errors == 0 { "RUNNING — all OK" } else { "RUNNING — ERRORS" };
    label(win_id, PAD, y, status_text, status_color, FontSize::Normal, TextAlign::Left);
    y += 28;

    // Progress animation (indeterminate)
    progress_indeterminate(win_id, PAD, y, WIN_W - PAD as u32 * 2, 6);
    y += 20;

    // Stats card
    card(win_id, PAD, y, WIN_W as u32 - PAD as u32 * 2, 120);
    y += 10;

    let x_label = PAD + 10;
    let x_value = 200;

    // Iterations
    label(win_id, x_label, y, "Iterations:", colors::TEXT(), FontSize::Normal, TextAlign::Left);
    let mut buf = [0u8; 12];
    let s = fmt_u32(&mut buf, stress.iterations);
    label(win_id, x_value, y, s, 0xFF00C8FF, FontSize::Normal, TextAlign::Left);
    y += 24;

    // Total syscalls
    label(win_id, x_label, y, "Total syscalls:", colors::TEXT(), FontSize::Normal, TextAlign::Left);
    let mut buf2 = [0u8; 12];
    let s = fmt_u32(&mut buf2, stress.total_syscalls as u32);
    label(win_id, x_value, y, s, 0xFF00C8FF, FontSize::Normal, TextAlign::Left);
    y += 24;

    // Elapsed time
    label(win_id, x_label, y, "Elapsed:", colors::TEXT(), FontSize::Normal, TextAlign::Left);
    let mut tbuf = [0u8; 24];
    let ts = fmt_elapsed(&mut tbuf, elapsed);
    label(win_id, x_value, y, ts, colors::TEXT(), FontSize::Normal, TextAlign::Left);
    y += 24;

    // Errors
    label(win_id, x_label, y, "Errors:", colors::TEXT(), FontSize::Normal, TextAlign::Left);
    let mut ebuf = [0u8; 12];
    let es = fmt_u32(&mut ebuf, stress.errors);
    let err_color = if stress.errors == 0 { 0xFF00FF80 } else { 0xFFFF4040 };
    label(win_id, x_value, y, es, err_color, FontSize::Normal, TextAlign::Left);
}

fn render_stress_results(win_id: u32, stress: &StressState, hz: u32) {
    let elapsed = ticks_to_sec(stress.start_tick, hz); // stored as duration at stop

    let mut y: i32 = 50;

    let status_color = if stress.errors == 0 { 0xFF00FF80 } else { 0xFFFF4040 };
    let status_text = if stress.errors == 0 { "PASSED — no errors" } else { "FAILED — errors detected" };
    label(win_id, PAD, y, status_text, status_color, FontSize::Normal, TextAlign::Left);
    y += 28;

    card(win_id, PAD, y, WIN_W as u32 - PAD as u32 * 2, 120);
    y += 10;

    let x_label = PAD + 10;
    let x_value = 200;

    label(win_id, x_label, y, "Iterations:", colors::TEXT(), FontSize::Normal, TextAlign::Left);
    let mut buf = [0u8; 12];
    let s = fmt_u32(&mut buf, stress.iterations);
    label(win_id, x_value, y, s, 0xFF00C8FF, FontSize::Normal, TextAlign::Left);
    y += 24;

    label(win_id, x_label, y, "Total syscalls:", colors::TEXT(), FontSize::Normal, TextAlign::Left);
    let mut buf2 = [0u8; 12];
    let s = fmt_u32(&mut buf2, stress.total_syscalls as u32);
    label(win_id, x_value, y, s, 0xFF00C8FF, FontSize::Normal, TextAlign::Left);
    y += 24;

    label(win_id, x_label, y, "Duration:", colors::TEXT(), FontSize::Normal, TextAlign::Left);
    let mut tbuf = [0u8; 24];
    let ts = fmt_elapsed(&mut tbuf, elapsed);
    label(win_id, x_value, y, ts, colors::TEXT(), FontSize::Normal, TextAlign::Left);
    y += 24;

    label(win_id, x_label, y, "Errors:", colors::TEXT(), FontSize::Normal, TextAlign::Left);
    let mut ebuf = [0u8; 12];
    let es = fmt_u32(&mut ebuf, stress.errors);
    let err_color = if stress.errors == 0 { 0xFF00FF80 } else { 0xFFFF4040 };
    label(win_id, x_value, y, es, err_color, FontSize::Normal, TextAlign::Left);
}

// ── Formatting ──────────────────────────────────────────────────────────────

fn fmt_u32<'a>(buf: &'a mut [u8; 12], val: u32) -> &'a str {
    if val == 0 { buf[0] = b'0'; return unsafe { core::str::from_utf8_unchecked(&buf[..1]) }; }
    let mut v = val; let mut tmp = [0u8; 12]; let mut n = 0;
    while v > 0 { tmp[n] = b'0' + (v % 10) as u8; v /= 10; n += 1; }
    for i in 0..n { buf[i] = tmp[n - 1 - i]; }
    unsafe { core::str::from_utf8_unchecked(&buf[..n]) }
}

fn fmt_us<'a>(buf: &'a mut [u8; 24], us: u32) -> &'a str {
    let mut tmp = [0u8; 12];
    let s = fmt_u32(&mut tmp, us);
    let mut p = 0;
    buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p..p + 3].copy_from_slice(b" us"); p += 3;
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

fn fmt_running<'a>(buf: &'a mut [u8; 40], cur: usize, total: usize) -> &'a str {
    let mut p = 0;
    buf[p..p + 9].copy_from_slice(b"Running ("); p += 9;
    let mut tmp = [0u8; 12];
    let s = fmt_u32(&mut tmp, cur as u32);
    buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p] = b'/'; p += 1;
    let s = fmt_u32(&mut tmp, total as u32);
    buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p..p + 3].copy_from_slice(b").."); p += 3;
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

fn fmt_elapsed<'a>(buf: &'a mut [u8; 24], seconds: u32) -> &'a str {
    let mins = seconds / 60;
    let secs = seconds % 60;
    let mut p = 0;
    let mut tmp = [0u8; 12];
    let s = fmt_u32(&mut tmp, mins);
    buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p] = b'm'; p += 1;
    buf[p] = b' '; p += 1;
    let s = fmt_u32(&mut tmp, secs);
    buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p] = b's'; p += 1;
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

// ── Main ────────────────────────────────────────────────────────────────────

fn main() {
    let win_id = window::create("Diagnostics", 100, 60, WIN_W as u16, WIN_H as u16);
    if win_id == u32::MAX { return; }

    let mut mb = window::MenuBarBuilder::new()
        .menu("File")
            .item(1, "Close", 0)
        .end_menu()
        .menu("Benchmark")
            .item(10, "Run All", 0)
            .item(11, "Stress Test", 0)
        .end_menu();
    let data = mb.build();
    window::set_menu(win_id, data);

    let hz = sys::tick_hz();
    let mut bench = BenchState::new();
    let mut stress = StressState::new();
    let mut mode = Mode::Idle;
    let mut event = [0u32; 5];

    // Buttons
    let btn_start = UiToolbarButton::new(WIN_W as i32 - 110, 6, 100, 24);
    let btn_stress = UiToolbarButton::new(WIN_W as i32 - 220, 6, 100, 24);

    render_main(win_id, mode, &bench, &stress, hz, &btn_start, &btn_stress);

    loop {
        if mode == Mode::Stress {
            // In stress mode: run one iteration, then check for stop events
            let (calls, errs) = stress_iteration(hz, win_id);
            stress.iterations += 1;
            stress.total_syscalls += calls as u64;
            stress.errors += errs;

            // Log every 100 iterations
            if stress.iterations % 100 == 0 {
                let elapsed = ticks_to_sec(ticks_now().wrapping_sub(stress.start_tick), hz);
                log!(
                    "STRESS: iter={} syscalls={} errors={} elapsed={}s",
                    stress.iterations, stress.total_syscalls, stress.errors, elapsed
                );
            }

            // Update display ~4x per second (every 250ms worth of ticks)
            let now = ticks_now();
            let render_interval = hz / 4; // ~250ms
            if now.wrapping_sub(stress.last_render_tick) >= render_interval || stress.iterations <= 1 {
                stress.last_render_tick = now;
                render_main(win_id, mode, &bench, &stress, hz, &btn_start, &btn_stress);
            }

            // Check for stop events (non-blocking)
            while window::get_event(win_id, &mut event) == 1 {
                let ev = UiEvent::from_raw(&event);

                if ev.is_key_down() && ev.key_code() == KEY_ESCAPE {
                    stop_stress(&mut stress, &mut mode, hz);
                    render_main(win_id, mode, &bench, &stress, hz, &btn_start, &btn_stress);
                    break;
                }
                if event[0] == EVENT_WINDOW_CLOSE {
                    window::destroy(win_id);
                    return;
                }
                // Stop button click
                if ev.is_mouse_down() {
                    if btn_start.handle_event(&ev) {
                        stop_stress(&mut stress, &mut mode, hz);
                        render_main(win_id, mode, &bench, &stress, hz, &btn_start, &btn_stress);
                        break;
                    }
                }
                if event[0] == window::EVENT_MENU_ITEM && event[2] == 1 {
                    window::destroy(win_id);
                    return;
                }
            }
            continue;
        }

        // Normal event loop (not in stress mode)
        if window::get_event(win_id, &mut event) == 1 {
            let ev = UiEvent::from_raw(&event);

            if ev.is_key_down() && ev.key_code() == KEY_ESCAPE {
                break;
            }
            if event[0] == EVENT_WINDOW_CLOSE { break; }

            if event[0] == window::EVENT_MENU_ITEM {
                match event[2] {
                    1 => break,
                    10 => {
                        if mode != Mode::Bench {
                            start_bench(win_id, &mut bench, &mut mode, hz, &btn_start, &btn_stress);
                        }
                    }
                    11 => {
                        if mode != Mode::Stress && mode != Mode::Bench {
                            start_stress(win_id, &mut stress, &mut mode, hz, &btn_start, &btn_stress);
                        }
                    }
                    _ => {}
                }
            }

            if event[0] == 0x0050 {
                render_main(win_id, mode, &bench, &stress, hz, &btn_start, &btn_stress);
            }

            if ev.is_mouse_down() {
                match mode {
                    Mode::Idle | Mode::BenchDone | Mode::StressDone => {
                        if btn_start.handle_event(&ev) {
                            start_bench(win_id, &mut bench, &mut mode, hz, &btn_start, &btn_stress);
                        } else if btn_stress.handle_event(&ev) {
                            start_stress(win_id, &mut stress, &mut mode, hz, &btn_start, &btn_stress);
                        }
                    }
                    _ => {}
                }
            }
        }

        process::sleep(16);
    }

    window::destroy(win_id);
}

fn start_bench(
    win_id: u32,
    bench: &mut BenchState,
    mode: &mut Mode,
    hz: u32,
    btn_start: &UiToolbarButton,
    btn_stress: &UiToolbarButton,
) {
    *bench = BenchState::new();
    bench.running = true;
    *mode = Mode::Bench;
    let stress_dummy = StressState::new();
    render_main(win_id, *mode, bench, &stress_dummy, hz, btn_start, btn_stress);
    run_all_benchmarks(win_id, bench, hz, btn_start, btn_stress);
    *mode = Mode::BenchDone;
    render_main(win_id, *mode, bench, &stress_dummy, hz, btn_start, btn_stress);
}

fn start_stress(
    win_id: u32,
    stress: &mut StressState,
    mode: &mut Mode,
    hz: u32,
    btn_start: &UiToolbarButton,
    btn_stress: &UiToolbarButton,
) {
    *stress = StressState::new();
    stress.running = true;
    stress.start_tick = ticks_now();
    stress.last_render_tick = stress.start_tick;
    *mode = Mode::Stress;
    let bench_dummy = BenchState::new();

    log!("");
    log!("╔══════════════════════════════════════════╗");
    log!("║   STRESS TEST STARTED                     ║");
    log!("║   Press Stop or Escape to end             ║");
    log!("╚══════════════════════════════════════════╝");
    log!("");

    render_main(win_id, *mode, &bench_dummy, stress, hz, btn_start, btn_stress);
}

fn stop_stress(stress: &mut StressState, mode: &mut Mode, hz: u32) {
    stress.running = false;
    // Store elapsed duration in start_tick for the results screen
    let elapsed = ticks_to_sec(ticks_now().wrapping_sub(stress.start_tick), hz);
    stress.start_tick = elapsed;
    *mode = Mode::StressDone;

    log!("");
    log!("╔══════════════════════════════════════════╗");
    log!("║   STRESS TEST COMPLETE                    ║");
    log!("╚══════════════════════════════════════════╝");
    log!("  Iterations:    {}", stress.iterations);
    log!("  Total syscalls: {}", stress.total_syscalls);
    log!("  Errors:        {}", stress.errors);
    log!("  Duration:      {}s", elapsed);
    log!("");
}

fn run_all_benchmarks(
    win_id: u32,
    state: &mut BenchState,
    hz: u32,
    btn_start: &UiToolbarButton,
    btn_stress: &UiToolbarButton,
) {
    log!("");
    log!("╔══════════════════════════════════════════╗");
    log!("║   anyOS System Diagnostics v1.1          ║");
    log!("║   Tick rate: {} Hz                        ║", hz);
    log!("╚══════════════════════════════════════════╝");
    log!("");

    let tests: [fn(&mut BenchState, u32); 12] = [
        bench_syscall_overhead,
        bench_file_stat,
        bench_file_open_close,
        bench_file_read_small,
        bench_file_read_large,
        bench_file_read_testdata,
        bench_heap_alloc,
        bench_window_create_destroy,
        bench_window_render,
        bench_sleep_accuracy,
        bench_process_spawn,
        |_, _| {},  // sentinel
    ];
    let num_tests = 11;
    state.total_tests = num_tests;
    let stress_dummy = StressState::new();

    for i in 0..num_tests {
        state.current_test = i + 1;
        render_main(win_id, Mode::Bench, state, &stress_dummy, hz, btn_start, btn_stress);
        process::sleep(30);
        tests[i](state, hz);
    }

    state.running = false;
    state.done = true;

    log!("");
    log!("╔══════════════════════════════════════════╗");
    log!("║   RESULTS SUMMARY                        ║");
    log!("╚══════════════════════════════════════════╝");
    for i in 0..state.count {
        if let Some(ref r) = state.results[i] {
            let name = unsafe { core::str::from_utf8_unchecked(&r.name[..r.name_len]) };
            log!("  {:<24} {:>8} us  ({} iters)", name, r.value_us, r.iterations);
        }
    }
    log!("");
    log!("=== Diagnostics complete ===");
}
