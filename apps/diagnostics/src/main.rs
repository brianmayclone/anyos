#![no_std]
#![no_main]

use anyos_std::fs;
use anyos_std::process;
use anyos_std::sys;
use anyos_std::ui::window;
use anyos_std::Vec;
use libanyui_client as anyui;
use anyui::Widget;

anyos_std::entry!(main);

const WIN_W: u32 = 480;
const WIN_H: u32 = 440;

// ── Timing helper ───────────────────────────────────────────────────────────

fn ticks_now() -> u32 { sys::uptime() }

fn ticks_to_us(ticks: u32, hz: u32) -> u32 {
    if hz == 0 { return 0; }
    ((ticks as u64) * 1_000_000 / (hz as u64)) as u32
}

fn ticks_to_sec(ticks: u32, hz: u32) -> u32 {
    if hz == 0 { return 0; }
    ticks / hz
}

macro_rules! log {
    ($($arg:tt)*) => { anyos_std::println!($($arg)*); };
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
    current_test: usize,
    total_tests: usize,
}

impl BenchState {
    fn new() -> Self {
        const NONE: Option<BenchResult> = None;
        Self { results: [NONE; MAX_RESULTS], count: 0, current_test: 0, total_tests: 11 }
    }

    fn add(&mut self, name: &str, value_us: u32, iterations: u32) {
        if self.count >= MAX_RESULTS { return; }
        let mut n = [0u8; 40];
        let len = name.len().min(40);
        n[..len].copy_from_slice(&name.as_bytes()[..len]);
        self.results[self.count] = Some(BenchResult { name: n, name_len: len, value_us, iterations });
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
    elapsed_sec: u32,
}

impl StressState {
    fn new() -> Self {
        Self { running: false, iterations: 0, total_syscalls: 0, errors: 0, start_tick: 0, elapsed_sec: 0 }
    }
}

// ── UI mode ─────────────────────────────────────────────────────────────────

#[derive(PartialEq, Clone, Copy)]
enum Mode { Idle, Bench, BenchDone, Stress, StressDone }

// ── App state ───────────────────────────────────────────────────────────────

struct AppState {
    mode: Mode,
    bench: BenchState,
    stress: StressState,
    hz: u32,
    btn_start: anyui::Button,
    btn_stress: anyui::Button,
    lbl_line1: anyui::Label,
    lbl_line2: anyui::Label,
    lbl_stat1: anyui::Label,
    lbl_stat2: anyui::Label,
    lbl_stat3: anyui::Label,
    lbl_stat4: anyui::Label,
    progress: anyui::ProgressBar,
    grid: anyui::DataGrid,
}

static mut APP: Option<AppState> = None;
fn app() -> &'static mut AppState { unsafe { APP.as_mut().unwrap() } }

// ── Benchmark tests ─────────────────────────────────────────────────────────

fn bench_syscall_overhead(state: &mut BenchState, hz: u32) {
    log!("=== Test: Syscall Overhead (uptime) ===");
    let iters = 1000u32;
    let t0 = ticks_now();
    for _ in 0..iters { let _ = sys::uptime(); }
    let t1 = ticks_now();
    let us = ticks_to_us(t1.wrapping_sub(t0), hz);
    let per = us / iters;
    log!("  {} iterations in {} us ({} us/call)", iters, us, per);
    state.add("Syscall (uptime)", per, iters);
}

fn bench_file_stat(state: &mut BenchState, hz: u32) {
    log!("=== Test: File stat() ===");
    let iters = 100u32;
    let mut stat_buf = [0u32; 7];
    let t0 = ticks_now();
    for _ in 0..iters { let _ = fs::stat("/Applications/Diagnostics.app", &mut stat_buf); }
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
        if fd != u32::MAX { fs::close(fd); }
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
        if fd != u32::MAX { fs::read(fd, &mut buf); fs::close(fd); }
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
    log!("  {} iterations, {} KiB total, {} us ({} us/read)", iters, total_bytes / 1024, us, per);
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
    log!("  {} iterations, {} KiB total, {} us ({} us/read)", iters, total_bytes / 1024, us, per);
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
        if wid == u32::MAX { log!("  iter {}: create FAILED", i); continue; }
        let td0 = ticks_now();
        window::destroy(wid);
        let td1 = ticks_now();
        total_create += ticks_to_us(tc1.wrapping_sub(tc0), hz);
        total_destroy += ticks_to_us(td1.wrapping_sub(td0), hz);
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
        state.add("Window render", 0, 0);
        return;
    }
    let iters = 20u32;
    let t0 = ticks_now();
    for _ in 0..iters {
        window::fill_rect(wid, 0, 0, 320, 240, 0xFF2D2D2D);
        window::draw_text(wid, 10, 10, 0xFFFFFFFF, "Benchmark");
        window::fill_rect(wid, 10, 30, 300, 60, 0xFF3C3C3C);
        window::fill_rect(wid, 20, 50, 280, 8, 0xFF555555);
        window::fill_rect(wid, 20, 50, 210, 8, 0xFF007AFF);
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
        if tid != u32::MAX && tid != 0 { process::sleep(200); }
    }
    let avg = if iters > 0 { total / iters } else { 0 };
    log!("  avg spawn={} us", avg);
    state.add("Spawn /bin/ls", avg, iters);
}

// ── Stress iteration ────────────────────────────────────────────────────────

fn stress_iteration(_hz: u32) -> (u32, u32) {
    let mut calls: u32 = 0;
    let mut errors: u32 = 0;

    // 1. Syscall burst: 200x uptime
    for _ in 0..200 { let _ = sys::uptime(); calls += 1; }

    // 2. File operations: open + read + close
    for _ in 0..10 {
        let fd = fs::open("/System/fonts/sfpro.ttf", 0);
        calls += 1;
        if fd != u32::MAX {
            let mut buf = [0u8; 512];
            let n = fs::read(fd, &mut buf); calls += 1;
            if n == u32::MAX { errors += 1; }
            fs::close(fd); calls += 1;
        } else { errors += 1; }
    }

    // 3. File stat
    for _ in 0..20 {
        let mut stat_buf = [0u32; 7];
        let r = fs::stat("/Applications/Diagnostics.app", &mut stat_buf);
        calls += 1;
        if r == u32::MAX { errors += 1; }
    }

    // 4. Heap alloc + free
    for _ in 0..50 {
        let v: Vec<u8> = Vec::with_capacity(1024);
        core::hint::black_box(&v);
        drop(v);
        calls += 1;
    }

    // 5. Sleep (scheduler yield)
    process::sleep(1); calls += 1;

    // 6. Tick Hz query
    for _ in 0..50 { let _ = sys::tick_hz(); calls += 1; }

    // 7. getpid
    for _ in 0..50 { let _ = process::getpid(); calls += 1; }

    // 8. Window IPC: create + fill + present + destroy
    {
        let wid = window::create("StressWin", 0, 0, 64, 64);
        calls += 1;
        if wid != u32::MAX {
            window::fill_rect(wid, 0, 0, 64, 64, 0xFF2D2D2D); calls += 1;
            window::draw_text(wid, 2, 2, 0xFFFFFFFF, "S"); calls += 1;
            window::present(wid); calls += 1;
            window::destroy(wid); calls += 1;
        }
    }

    (calls, errors)
}

// ── Benchmark runner (one test per call) ────────────────────────────────────

fn run_bench_step(state: &mut BenchState, hz: u32) {
    let tests: [fn(&mut BenchState, u32); 11] = [
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
    ];

    if state.current_test < state.total_tests {
        tests[state.current_test](state, hz);
        state.current_test += 1;
    }
}

// ── DataGrid population ─────────────────────────────────────────────────────

fn populate_grid(grid: &anyui::DataGrid, bench: &BenchState) {
    grid.set_row_count(bench.count as u32);

    let mut data = Vec::new();
    let mut text_colors = Vec::new();
    for i in 0..bench.count {
        if let Some(ref r) = bench.results[i] {
            if i > 0 { data.push(0x1E); }
            data.extend_from_slice(&r.name[..r.name_len]);
            data.push(0x1F);
            let mut vbuf = [0u8; 12];
            let vs = fmt_u32_str(&mut vbuf, r.value_us);
            data.extend_from_slice(vs.as_bytes());
            data.extend_from_slice(b" us");
            data.push(0x1F);
            let mut ibuf = [0u8; 12];
            let is = fmt_u32_str(&mut ibuf, r.iterations);
            data.extend_from_slice(is.as_bytes());

            text_colors.push(0xFFE6E6E6);
            text_colors.push(0xFF00FF80);
            text_colors.push(0xFF999999);
        }
    }
    grid.set_data_raw(&data);
    grid.set_cell_colors(&text_colors);
}

// ── UI update ───────────────────────────────────────────────────────────────

fn update_ui() {
    let a = app();
    match a.mode {
        Mode::Idle => {
            a.lbl_line1.set_text("Press Start to run benchmarks,");
            a.lbl_line1.set_text_color(0xFFE6E6E6);
            a.lbl_line2.set_text("or Stress Test for a continuous loop.");
            a.lbl_stat1.set_text("Stress Test runs syscalls in a loop");
            a.lbl_stat2.set_text("until you press Stop.");
            a.lbl_stat3.set_text("");
            a.lbl_stat4.set_text("");
            a.progress.set_state(0);
            a.progress.set_size(0, 0);
            a.grid.set_size(0, 0);
            a.btn_start.set_text("Start");
            a.btn_stress.set_text("Stress Test");
            a.btn_stress.set_size(90, 28);
        }
        Mode::Bench => {
            let cur = a.bench.current_test;
            let total = a.bench.total_tests;
            let pct = if total > 0 { (cur as u32 * 100 / total as u32).min(100) } else { 0 };
            let mut buf = [0u8; 40];
            let s = fmt_running(&mut buf, cur, total);
            a.lbl_line1.set_text(s);
            a.lbl_line1.set_text_color(0xFFE6E6E6);
            a.lbl_line2.set_text("");
            a.lbl_stat1.set_text("");
            a.lbl_stat2.set_text("");
            a.lbl_stat3.set_text("");
            a.lbl_stat4.set_text("");
            a.progress.set_size(WIN_W.saturating_sub(20), 10);
            a.progress.set_state(pct);
            a.grid.set_size(0, 0);
            a.btn_start.set_text("...");
            a.btn_stress.set_size(0, 0);
        }
        Mode::BenchDone => {
            a.lbl_line1.set_text("");
            a.lbl_line2.set_text("");
            a.lbl_stat1.set_text("");
            a.lbl_stat2.set_text("");
            a.lbl_stat3.set_text("");
            a.lbl_stat4.set_text("");
            a.progress.set_size(0, 0);
            a.grid.set_size(WIN_W.saturating_sub(20), 300);
            populate_grid(&a.grid, &a.bench);
            a.btn_start.set_text("Re-run");
            a.btn_stress.set_text("Stress Test");
            a.btn_stress.set_size(90, 28);
        }
        Mode::Stress => {
            let elapsed = ticks_to_sec(ticks_now().wrapping_sub(a.stress.start_tick), a.hz);
            let status_color = if a.stress.errors == 0 { 0xFF00FF80 } else { 0xFFFF4040 };
            let status_text = if a.stress.errors == 0 { "RUNNING - all OK" } else { "RUNNING - ERRORS" };
            a.lbl_line1.set_text(status_text);
            a.lbl_line1.set_text_color(status_color);
            a.lbl_line2.set_text("");

            let mut buf1 = [0u8; 32];
            a.lbl_stat1.set_text(fmt_kv(&mut buf1, "Iterations: ", a.stress.iterations));

            let mut buf2 = [0u8; 32];
            a.lbl_stat2.set_text(fmt_kv(&mut buf2, "Syscalls:   ", a.stress.total_syscalls as u32));

            let mut buf3 = [0u8; 32];
            a.lbl_stat3.set_text(fmt_elapsed_label(&mut buf3, "Elapsed:    ", elapsed));

            let mut buf4 = [0u8; 32];
            a.lbl_stat4.set_text(fmt_kv(&mut buf4, "Errors:     ", a.stress.errors));
            a.lbl_stat4.set_text_color(if a.stress.errors == 0 { 0xFF00FF80 } else { 0xFFFF4040 });

            a.progress.set_size(WIN_W.saturating_sub(20), 6);
            a.progress.set_state((a.stress.iterations % 100) as u32);
            a.grid.set_size(0, 0);
            a.btn_start.set_text("Stop");
            a.btn_stress.set_size(0, 0);
        }
        Mode::StressDone => {
            let status_color = if a.stress.errors == 0 { 0xFF00FF80 } else { 0xFFFF4040 };
            let status_text = if a.stress.errors == 0 { "PASSED - no errors" } else { "FAILED - errors detected" };
            a.lbl_line1.set_text(status_text);
            a.lbl_line1.set_text_color(status_color);
            a.lbl_line2.set_text("");

            let mut buf1 = [0u8; 32];
            a.lbl_stat1.set_text(fmt_kv(&mut buf1, "Iterations: ", a.stress.iterations));

            let mut buf2 = [0u8; 32];
            a.lbl_stat2.set_text(fmt_kv(&mut buf2, "Syscalls:   ", a.stress.total_syscalls as u32));

            let mut buf3 = [0u8; 32];
            a.lbl_stat3.set_text(fmt_elapsed_label(&mut buf3, "Duration:   ", a.stress.elapsed_sec));

            let mut buf4 = [0u8; 32];
            a.lbl_stat4.set_text(fmt_kv(&mut buf4, "Errors:     ", a.stress.errors));
            a.lbl_stat4.set_text_color(if a.stress.errors == 0 { 0xFF00FF80 } else { 0xFFFF4040 });

            a.progress.set_size(0, 0);
            a.grid.set_size(0, 0);
            a.btn_start.set_text("Re-run");
            a.btn_stress.set_text("Stress Test");
            a.btn_stress.set_size(90, 28);
        }
    }
}

// ── Main ────────────────────────────────────────────────────────────────────

fn main() {
    if !anyui::init() { return; }

    let win = anyui::Window::new("Diagnostics", -1, -1, WIN_W, WIN_H);

    // Toolbar (DOCK_TOP)
    let toolbar = anyui::Toolbar::new();
    toolbar.set_dock(anyui::DOCK_TOP);
    toolbar.set_size(WIN_W, 36);
    toolbar.set_color(0xFF252526);
    toolbar.set_padding(4, 4, 4, 4);

    let title_lbl = toolbar.add_label("System Diagnostics");
    title_lbl.set_text_color(0xFF00C8FF);
    title_lbl.set_size(200, 28);

    let btn_stress = toolbar.add_button("Stress Test");
    btn_stress.set_size(90, 28);

    let btn_start = toolbar.add_button("Start");
    btn_start.set_size(80, 28);

    win.add(&toolbar);

    // Content area (DOCK_FILL)
    let content = anyui::View::new();
    content.set_dock(anyui::DOCK_FILL);
    content.set_color(0xFF1E1E1E);

    let lbl_line1 = anyui::Label::new("Press Start to run benchmarks,");
    lbl_line1.set_position(10, 10);
    lbl_line1.set_size(460, 20);
    lbl_line1.set_text_color(0xFFE6E6E6);
    content.add(&lbl_line1);

    let lbl_line2 = anyui::Label::new("or Stress Test for a continuous loop.");
    lbl_line2.set_position(10, 30);
    lbl_line2.set_size(460, 20);
    lbl_line2.set_text_color(0xFFE6E6E6);
    content.add(&lbl_line2);

    let progress = anyui::ProgressBar::new(0);
    progress.set_position(10, 55);
    progress.set_size(0, 0);
    content.add(&progress);

    let lbl_stat1 = anyui::Label::new("Stress Test runs syscalls in a loop");
    lbl_stat1.set_position(10, 70);
    lbl_stat1.set_size(460, 20);
    lbl_stat1.set_text_color(0xFF999999);
    content.add(&lbl_stat1);

    let lbl_stat2 = anyui::Label::new("until you press Stop.");
    lbl_stat2.set_position(10, 94);
    lbl_stat2.set_size(460, 20);
    lbl_stat2.set_text_color(0xFF999999);
    content.add(&lbl_stat2);

    let lbl_stat3 = anyui::Label::new("");
    lbl_stat3.set_position(10, 118);
    lbl_stat3.set_size(460, 20);
    lbl_stat3.set_text_color(0xFFE6E6E6);
    content.add(&lbl_stat3);

    let lbl_stat4 = anyui::Label::new("");
    lbl_stat4.set_position(10, 142);
    lbl_stat4.set_size(460, 20);
    lbl_stat4.set_text_color(0xFF00FF80);
    content.add(&lbl_stat4);

    let grid = anyui::DataGrid::new(0, 0);
    grid.set_position(10, 10);
    grid.set_columns(&[
        anyui::ColumnDef::new("Test").width(200),
        anyui::ColumnDef::new("Time").width(100).align(anyui::ALIGN_RIGHT),
        anyui::ColumnDef::new("Iters").width(80).align(anyui::ALIGN_RIGHT).numeric(),
    ]);
    content.add(&grid);

    win.add(&content);

    let hz = sys::tick_hz();

    unsafe {
        APP = Some(AppState {
            mode: Mode::Idle,
            bench: BenchState::new(),
            stress: StressState::new(),
            hz,
            btn_start,
            btn_stress,
            lbl_line1,
            lbl_line2,
            lbl_stat1,
            lbl_stat2,
            lbl_stat3,
            lbl_stat4,
            progress,
            grid,
        });
    }

    app().btn_start.on_click(|_| {
        let a = app();
        match a.mode {
            Mode::Idle | Mode::BenchDone | Mode::StressDone => {
                a.bench = BenchState::new();
                a.mode = Mode::Bench;
                log!("");
                log!("======================================");
                log!("  anyOS System Diagnostics v2.0");
                log!("  Tick rate: {} Hz", a.hz);
                log!("======================================");
                log!("");
                update_ui();
            }
            Mode::Stress => {
                let elapsed = ticks_to_sec(ticks_now().wrapping_sub(a.stress.start_tick), a.hz);
                a.stress.running = false;
                a.stress.elapsed_sec = elapsed;
                a.mode = Mode::StressDone;
                log!("  STRESS TEST COMPLETE: {} iters, {} errors, {}s",
                    a.stress.iterations, a.stress.errors, elapsed);
                update_ui();
            }
            _ => {}
        }
    });

    app().btn_stress.on_click(|_| {
        let a = app();
        if a.mode == Mode::Idle || a.mode == Mode::BenchDone || a.mode == Mode::StressDone {
            a.stress = StressState::new();
            a.stress.running = true;
            a.stress.start_tick = ticks_now();
            a.mode = Mode::Stress;
            log!("  STRESS TEST STARTED");
            update_ui();
        }
    });

    win.on_key_down(|ke| {
        if ke.keycode == anyui::KEY_ESCAPE {
            if app().mode == Mode::Stress {
                let a = app();
                let elapsed = ticks_to_sec(ticks_now().wrapping_sub(a.stress.start_tick), a.hz);
                a.stress.running = false;
                a.stress.elapsed_sec = elapsed;
                a.mode = Mode::StressDone;
                update_ui();
            } else {
                anyui::quit();
            }
        }
    });

    win.on_close(|_| anyui::quit());

    anyui::set_timer(30, || {
        let a = app();
        match a.mode {
            Mode::Bench => {
                if a.bench.current_test < a.bench.total_tests {
                    let hz = a.hz;
                    run_bench_step(&mut a.bench, hz);
                    update_ui();
                } else {
                    a.mode = Mode::BenchDone;
                    log!("");
                    log!("  RESULTS SUMMARY");
                    for i in 0..a.bench.count {
                        if let Some(ref r) = a.bench.results[i] {
                            let name = unsafe { core::str::from_utf8_unchecked(&r.name[..r.name_len]) };
                            log!("  {:<24} {:>8} us  ({} iters)", name, r.value_us, r.iterations);
                        }
                    }
                    log!("");
                    update_ui();
                }
            }
            Mode::Stress if a.stress.running => {
                let hz = a.hz;
                let (calls, errs) = stress_iteration(hz);
                a.stress.iterations += 1;
                a.stress.total_syscalls += calls as u64;
                a.stress.errors += errs;

                if a.stress.iterations % 100 == 0 {
                    let elapsed = ticks_to_sec(ticks_now().wrapping_sub(a.stress.start_tick), hz);
                    log!("STRESS: iter={} syscalls={} errors={} elapsed={}s",
                        a.stress.iterations, a.stress.total_syscalls, a.stress.errors, elapsed);
                }

                if a.stress.iterations % 8 == 0 || a.stress.iterations <= 1 {
                    update_ui();
                }
            }
            _ => {}
        }
    });

    anyui::run();
}

// ── Formatting ──────────────────────────────────────────────────────────────

fn fmt_u32_str<'a>(buf: &'a mut [u8; 12], val: u32) -> &'a str {
    if val == 0 { buf[0] = b'0'; return unsafe { core::str::from_utf8_unchecked(&buf[..1]) }; }
    let mut v = val; let mut tmp = [0u8; 12]; let mut n = 0;
    while v > 0 { tmp[n] = b'0' + (v % 10) as u8; v /= 10; n += 1; }
    for i in 0..n { buf[i] = tmp[n - 1 - i]; }
    unsafe { core::str::from_utf8_unchecked(&buf[..n]) }
}

fn fmt_running<'a>(buf: &'a mut [u8; 40], cur: usize, total: usize) -> &'a str {
    let mut p = 0;
    let prefix = b"Running (";
    buf[p..p + prefix.len()].copy_from_slice(prefix); p += prefix.len();
    let mut tmp = [0u8; 12];
    let s = fmt_u32_str(&mut tmp, cur as u32);
    buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p] = b'/'; p += 1;
    let s = fmt_u32_str(&mut tmp, total as u32);
    buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    let suffix = b")...";
    buf[p..p + suffix.len()].copy_from_slice(suffix); p += suffix.len();
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

fn fmt_kv<'a>(buf: &'a mut [u8; 32], label: &str, val: u32) -> &'a str {
    let mut p = 0;
    for &b in label.as_bytes() { if p < 31 { buf[p] = b; p += 1; } }
    let mut tmp = [0u8; 12];
    let s = fmt_u32_str(&mut tmp, val);
    for &b in s.as_bytes() { if p < 31 { buf[p] = b; p += 1; } }
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

fn fmt_elapsed_label<'a>(buf: &'a mut [u8; 32], label: &str, seconds: u32) -> &'a str {
    let mut p = 0;
    for &b in label.as_bytes() { if p < 31 { buf[p] = b; p += 1; } }
    let mut tmp = [0u8; 12];
    let s = fmt_u32_str(&mut tmp, seconds / 60);
    for &b in s.as_bytes() { if p < 31 { buf[p] = b; p += 1; } }
    if p < 31 { buf[p] = b'm'; p += 1; }
    if p < 31 { buf[p] = b' '; p += 1; }
    let s = fmt_u32_str(&mut tmp, seconds % 60);
    for &b in s.as_bytes() { if p < 31 { buf[p] = b; p += 1; } }
    if p < 31 { buf[p] = b's'; p += 1; }
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}
