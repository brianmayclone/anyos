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
const WIN_H: u32 = 400;
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
        let _ = fs::stat("/system/diagnostics", &mut stat_buf);
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
        let fd = fs::open("/system/fonts/sfpro.ttf", 0);
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
        let fd = fs::open("/system/fonts/sfpro.ttf", 0);
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
        let fd = fs::open("/system/fonts/sfpro.ttf", 0);
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
        let fd = fs::open("/system/testdata.bin", 0);
        if fd != u32::MAX {
            loop {
                let n = fs::read(fd, &mut buf);
                if n == 0 || n == u32::MAX { break; }
                total_bytes += n as u64;
            }
            fs::close(fd);
        } else {
            log!("  WARN: /system/testdata.bin not found, skipping");
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
        let tid = process::spawn("/bin/ls", "/");
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

// ── Rendering ───────────────────────────────────────────────────────────────

fn render(win_id: u32, state: &BenchState, btn: &UiToolbarButton) {
    window::fill_rect(win_id, 0, 0, WIN_W as u16, WIN_H as u16, colors::WINDOW_BG);

    // Title bar with Start/Re-run button
    card(win_id, 0, 0, WIN_W, 36);
    label(win_id, PAD, 8, "System Diagnostics", 0xFF00C8FF, FontSize::Normal, TextAlign::Left);

    if !state.running {
        let btn_label = if state.done { "Re-run" } else { "Start" };
        btn.render(win_id, btn_label);
    }

    if !state.running && !state.done {
        // Show start prompt
        label(win_id, PAD, 50, "Press Start to run benchmarks.", colors::TEXT, FontSize::Normal, TextAlign::Left);
        label(win_id, PAD, 72, "Results will be logged to serial console.", colors::TEXT_SECONDARY, FontSize::Normal, TextAlign::Left);
    } else if state.running {
        // Show progress
        let pct = if state.total_tests > 0 {
            (state.current_test as u32 * 100 / state.total_tests as u32).min(100)
        } else { 0 };

        let mut pbuf = [0u8; 40];
        let ps = fmt_running(&mut pbuf, state.current_test, state.total_tests);
        label(win_id, PAD, 50, ps, colors::TEXT, FontSize::Normal, TextAlign::Left);
        progress(win_id, PAD, 72, WIN_W - PAD as u32 * 2, 10, pct);
    } else {
        // Show results
        let mut y: i32 = 42;

        // Header row
        window::fill_rect(win_id, 0, y as i16, WIN_W as u16, ROW_H as u16, 0xFF4A4A4A);
        label(win_id, PAD, y + 2, "Test", colors::TEXT, FontSize::Small, TextAlign::Left);
        label(win_id, 280, y + 2, "Time", colors::TEXT, FontSize::Small, TextAlign::Left);
        label(win_id, 370, y + 2, "Iters", colors::TEXT, FontSize::Small, TextAlign::Left);
        y += ROW_H;

        for i in 0..state.count {
            if y + ROW_H > WIN_H as i32 - 10 { break; }
            if let Some(ref r) = state.results[i] {
                if i % 2 == 1 {
                    window::fill_rect(win_id, 0, y as i16, WIN_W as u16, ROW_H as u16, 0xFF333333);
                }
                let name = unsafe { core::str::from_utf8_unchecked(&r.name[..r.name_len]) };
                label(win_id, PAD, y + 2, name, colors::TEXT, FontSize::Small, TextAlign::Left);

                let mut vbuf = [0u8; 24];
                let vs = fmt_us(&mut vbuf, r.value_us);
                label(win_id, 280, y + 2, vs, 0xFF00FF80, FontSize::Small, TextAlign::Left);

                let mut ibuf = [0u8; 12];
                let is = fmt_u32(&mut ibuf, r.iterations);
                label(win_id, 370, y + 2, is, colors::TEXT_SECONDARY, FontSize::Small, TextAlign::Left);

                y += ROW_H;
            }
        }
    }

    window::present(win_id);
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
        .end_menu();
    let data = mb.build();
    window::set_menu(win_id, data);

    let hz = sys::tick_hz();
    let mut state = BenchState::new();
    let mut event = [0u32; 5];

    // Start button area
    let btn = UiToolbarButton::new(WIN_W as i32 - 110, 6, 100, 24);

    render(win_id, &state, &btn);

    loop {
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
                        if !state.running {
                            state = BenchState::new();
                            state.running = true;
                            render(win_id, &state, &btn);
                            run_all_benchmarks(win_id, &mut state, hz, &btn);
                        }
                    }
                    _ => {}
                }
            }

            // Start button click
            if ev.is_mouse_down() && !state.running {
                if btn.handle_event(&ev) {
                    state = BenchState::new();
                    state.running = true;
                    render(win_id, &state, &btn);
                    run_all_benchmarks(win_id, &mut state, hz, &btn);
                }
            }
        }

        process::sleep(16);
    }

    window::destroy(win_id);
}

fn run_all_benchmarks(win_id: u32, state: &mut BenchState, hz: u32, btn: &UiToolbarButton) {
    log!("");
    log!("╔══════════════════════════════════════════╗");
    log!("║   anyOS System Diagnostics v1.0          ║");
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
    // Number of real tests (exclude sentinel)
    let num_tests = 11;
    state.total_tests = num_tests;

    for i in 0..num_tests {
        state.current_test = i + 1;
        render(win_id, state, btn);
        // Small delay so the progress bar is visible
        process::sleep(30);
        tests[i](state, hz);
    }

    // Done
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

    render(win_id, state, btn);
}
