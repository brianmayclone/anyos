#![no_std]
#![no_main]

use anyos_std::process;
use anyos_std::sys;
use anyos_std::fs;
use anyos_std::ipc;
use anyos_std::println;
use anyos_std::Box;

use libanyui_client as ui;
use ui::Widget;

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

anyos_std::entry!(main);

mod assets;

const DIALOG_W: u32 = 340;
const DIALOG_H: u32 = 280;
const FIELD_W: u32 = 280;
const PAD: i32 = 30;

// ── Shared state between worker thread and UI ───────────────────────────────

/// Immutable status snapshot published from the worker thread to the UI thread.
///
/// We intentionally publish whole snapshots instead of mutating a shared
/// `static mut` buffer. The previous approach was UB across threads and was
/// especially brittle on ARM64.
struct StatusMessage {
    len: usize,
    bytes: [u8; 128],
}

static STATUS_PTR: AtomicUsize = AtomicUsize::new(0);

/// Progress: 0..100
static PROGRESS: AtomicU32 = AtomicU32::new(0);

/// Worker thread done flag.
static WORKER_DONE: AtomicBool = AtomicBool::new(false);

fn set_status(msg: &str) {
    let len = msg.len().min(128);
    let mut status = Box::new(StatusMessage {
        len,
        bytes: [0u8; 128],
    });
    status.bytes[..len].copy_from_slice(&msg.as_bytes()[..len]);
    let ptr = Box::into_raw(status) as usize;
    let _old = STATUS_PTR.swap(ptr, Ordering::AcqRel);
}

fn get_status(buf: &mut [u8; 128]) -> usize {
    let ptr = STATUS_PTR.load(Ordering::Acquire) as *const StatusMessage;
    if ptr.is_null() {
        return 0;
    }
    let status = unsafe { &*ptr };
    buf[..status.len].copy_from_slice(&status.bytes[..status.len]);
    status.len
}

// ── CPU Benchmark ───────────────────────────────────────────────────────────

fn benchmark_cpu(duration_ticks: u32) -> u32 {
    let start = sys::uptime();
    let mut iterations: u32 = 0;
    let mut acc: u32 = 0x12345678;

    loop {
        for _ in 0..64 {
            for _ in 0..1000 {
                acc = acc.wrapping_mul(1103515245).wrapping_add(12345);
                acc ^= acc >> 16;
                acc = acc.wrapping_add(acc << 5);
            }
            iterations += 1;
        }
        if sys::uptime().wrapping_sub(start) >= duration_ticks {
            break;
        }
    }

    if acc == 0 { iterations += 1; }
    iterations
}

fn benchmark_memory(duration_ticks: u32) -> u32 {
    let start = sys::uptime();
    let mut iterations: u32 = 0;
    let mut buf = [0u32; 4096]; // 16 KiB

    loop {
        for _ in 0..32 {
            for i in 0..buf.len() {
                buf[i] = (i as u32).wrapping_mul(0xDEADBEEF);
            }
            let mut sum: u32 = 0;
            for i in 0..buf.len() {
                sum = sum.wrapping_add(buf[i]);
            }
            if sum == 0 { buf[0] = 1; }
            iterations += 1;
        }
        if sys::uptime().wrapping_sub(start) >= duration_ticks {
            break;
        }
    }

    iterations
}

// ── Init Config Parser ──────────────────────────────────────────────────────

fn run_init_conf(total_steps: &mut u32, current_step: &mut u32) {
    let fd = fs::open("/System/etc/init/init.conf", 0);
    if fd == u32::MAX {
        println!("init: /System/etc/init/init.conf not found, skipping");
        return;
    }

    let mut buf = [0u8; 1024];
    let n = fs::read(fd, &mut buf) as usize;
    fs::close(fd);

    if n == 0 {
        return;
    }

    // First pass: count non-comment, non-empty lines for progress
    let data = &buf[..n];
    let mut line_count: u32 = 0;
    {
        let mut ls = 0;
        for i in 0..=n {
            let at_end = i == n;
            let is_nl = !at_end && data[i] == b'\n';
            if is_nl || at_end {
                let le = if !at_end && i > 0 && data[i.saturating_sub(1)] == b'\r' { i - 1 } else { i };
                let line = &data[ls..le];
                ls = i + 1;
                let trimmed = trim_bytes(line);
                if !trimmed.is_empty() && trimmed[0] != b'#' {
                    line_count += 1;
                }
            }
        }
    }

    // Benchmarks are 2 steps, services are line_count steps
    *total_steps = 2 + line_count;

    // Second pass: execute
    let mut line_start = 0;
    for i in 0..=n {
        let at_end = i == n;
        let is_newline = !at_end && data[i] == b'\n';

        if is_newline || at_end {
            let line_end = if !at_end && i > 0 && data[i.saturating_sub(1)] == b'\r' {
                i - 1
            } else {
                i
            };
            let line = &data[line_start..line_end];
            line_start = i + 1;

            let trimmed = trim_bytes(line);
            if trimmed.is_empty() || trimmed[0] == b'#' {
                continue;
            }

            if let Ok(entry) = core::str::from_utf8(trimmed) {
                let (cmd, background) = if entry.ends_with('&') {
                    (entry[..entry.len() - 1].trim_end(), true)
                } else {
                    (entry, false)
                };
                let path = match cmd.find(' ') {
                    Some(idx) => &cmd[..idx],
                    None => cmd,
                };

                // Extract a readable service name from the path
                let svc_name = path.rsplit('/').next().unwrap_or(path);
                set_status("Starting Services ...");
                *current_step += 1;
                let pct = (*current_step * 100 / *total_steps).min(100);
                PROGRESS.store(pct, Ordering::Release);

                println!("init: spawning '{}'{}", cmd, if background { " [bg]" } else { "" });
                let tid = process::spawn(path, cmd);
                if tid == u32::MAX {
                    println!("init: FAILED to spawn '{}'", path);
                } else if !background {
                    let code = process::waitpid(tid);
                    println!("init: '{}' exited (code={})", path, code);
                }
            }
        }
    }
}

fn trim_bytes(b: &[u8]) -> &[u8] {
    let mut start = 0;
    while start < b.len() && (b[start] == b' ' || b[start] == b'\t') {
        start += 1;
    }
    let mut end = b.len();
    while end > start && (b[end - 1] == b' ' || b[end - 1] == b'\t') {
        end -= 1;
    }
    &b[start..end]
}

// ── Formatting ──────────────────────────────────────────────────────────────

fn fmt_u32(buf: &mut [u8], val: u32) -> usize {
    let mut tmp = [0u8; 12];
    let s = anyos_std::fmt::fmt_u32(&mut tmp, val);
    let n = s.len();
    buf[..n].copy_from_slice(s.as_bytes());
    n
}

// ── Worker thread ───────────────────────────────────────────────────────────

fn worker_entry() {
    let hz = sys::tick_hz();

    // Phase 1: CPU benchmark
    set_status("Initializing Hardware (1/2)...");
    PROGRESS.store(5, Ordering::Release);
    let cpu_score = benchmark_cpu(hz * 2);

    // Phase 2: Memory benchmark
    set_status("Initializing Hardware (2/2)...");
    PROGRESS.store(15, Ordering::Release);
    let mem_score = benchmark_memory(hz);

    // Report results
    let mut line = [0u8; 80];
    let mut p: usize;

    p = 0;
    let s = b"  CPU score : ";
    line[p..p + s.len()].copy_from_slice(s); p += s.len();
    p += fmt_u32(&mut line[p..], cpu_score);
    let s = b" Kops/2s";
    line[p..p + s.len()].copy_from_slice(s); p += s.len();
    if let Ok(s) = core::str::from_utf8(&line[..p]) { println!("{}", s); }

    p = 0;
    let s = b"  Mem score : ";
    line[p..p + s.len()].copy_from_slice(s); p += s.len();
    p += fmt_u32(&mut line[p..], mem_score);
    let s = b" passes/1s (16K buf)";
    line[p..p + s.len()].copy_from_slice(s); p += s.len();
    if let Ok(s) = core::str::from_utf8(&line[..p]) { println!("{}", s); }

    let pipe_id = ipc::pipe_create("sys:startup_info");
    if pipe_id > 0 {
        let mut info = [0u8; 128];
        let mut ip = 0;
        let s = b"cpu_score=";
        info[ip..ip + s.len()].copy_from_slice(s); ip += s.len();
        ip += fmt_u32(&mut info[ip..], cpu_score);
        info[ip] = b'\n'; ip += 1;
        let s = b"mem_score=";
        info[ip..ip + s.len()].copy_from_slice(s); ip += s.len();
        ip += fmt_u32(&mut info[ip..], mem_score);
        info[ip] = b'\n'; ip += 1;
        ipc::pipe_write(pipe_id, &info[..ip]);
    }

    sys::boot_ready();

    // Phase 3: Run init.conf
    set_status("Starting services...");
    PROGRESS.store(25, Ordering::Release);

    let mut total_steps: u32 = 5;
    let mut current_step: u32 = 2; // benchmarks already done
    run_init_conf(&mut total_steps, &mut current_step);

    // Done
    set_status("Ready");
    PROGRESS.store(100, Ordering::Release);
    WORKER_DONE.store(true, Ordering::Release);
}

// ── Main (GUI) ──────────────────────────────────────────────────────────────

fn main() {
    // Try GUI mode; fall back to headless if anyui is unavailable
    if !ui::init() {
        println!("init: no GUI available, running headless");
        worker_entry();
        return;
    }

    let (sw, sh) = ui::screen_size();
    let wx = ((sw as i32 - DIALOG_W as i32) / 2).max(0);
    let wy = ((sh as i32 - DIALOG_H as i32) / 2).max(0);

    let flags = ui::WIN_FLAG_BORDERLESS
        | ui::WIN_FLAG_SHADOW
        | ui::WIN_FLAG_NOT_RESIZABLE
        | ui::WIN_FLAG_NO_CLOSE
        | ui::WIN_FLAG_NO_MINIMIZE
        | ui::WIN_FLAG_NO_MAXIMIZE;
    let win = ui::Window::new_with_flags("System", wx, wy, DIALOG_W, DIALOG_H, flags);
    win.set_color(0xFFF0F0F0);

    // ── Logo ──
    let mut y_cursor: i32 = 30;
    if let Some((pixels, dw, dh)) = assets::load_and_scale_logo(48) {
        let logo = ui::ImageView::new(dw, dh);
        logo.set_pixels(&pixels, dw, dh);
        logo.set_position(((DIALOG_W as i32 - dw as i32) / 2).max(0), y_cursor);
        win.add(&logo);
        y_cursor += dh as i32 + 40;
    }

    // ── Welcome label ──
    let welcome = ui::Label::new("Welcome to anyOS");
    welcome.set_font_size(20);
    welcome.set_position(PAD, y_cursor);
    welcome.set_size(FIELD_W, 30);
    welcome.set_text_align(ui::TEXT_ALIGN_CENTER);
    win.add(&welcome);
    y_cursor += 46;

    // ── Spinner ──
    let spinner_size: u32 = 28;
    let spinner = ui::Spinner::new();
    spinner.set_size(spinner_size, spinner_size);
    spinner.set_position(((DIALOG_W as i32 - spinner_size as i32) / 2).max(0), y_cursor);
    win.add(&spinner);
    let _spinner_timer = spinner.start();
    y_cursor += spinner_size as i32 + 14;

    // ── Status label (shows current service) ──
    let status_label = ui::Label::new("Initializing...");
    status_label.set_font_size(11);
    status_label.set_position(PAD, y_cursor);
    status_label.set_size(FIELD_W, 18);
    status_label.set_text_align(ui::TEXT_ALIGN_CENTER);
    status_label.set_text_color(0xFF888888);
    win.add(&status_label);

    // ── Spawn worker thread ──
    let stack_size: usize = 128 * 1024;
    let stack_vec = alloc::vec![0u8; stack_size];
    let stack_base = stack_vec.as_ptr() as usize;
    core::mem::forget(stack_vec);
    #[cfg(target_arch = "x86_64")]
    let stack_top = ((stack_base + stack_size) & !0xF) - 8;
    #[cfg(target_arch = "aarch64")]
    let stack_top = (stack_base + stack_size) & !0xF;
    unsafe { *(stack_top as *mut usize) = process::thread_exit_stub_addr(); }
    process::thread_create_with_priority(worker_entry, stack_top, "init/worker", 100);

    // ── Timer: update status label from worker state ──
    let status_id = status_label.id();

    ui::set_timer(150, move || {
        let mut buf = [0u8; 128];
        let len = get_status(&mut buf);
        if let Ok(msg) = core::str::from_utf8(&buf[..len]) {
            ui::Control::from_id(status_id).set_text(msg);
        }

        if WORKER_DONE.load(Ordering::Acquire) {
            ui::quit();
        }
    });

    ui::run();
}
