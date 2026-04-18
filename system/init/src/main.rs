#![no_std]
#![no_main]

use anyos_std::process;
use anyos_std::fs;
use anyos_std::println;
use anyos_std::format;
use anyos_std::Box;

use libanyui_client as ui;
use libinstall;
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

// ── Service Startup ─────────────────────────────────────────────────────────

fn run_service_manager() {
    let path = "/System/svc";
    let args = "/System/svc start-all";

    println!("init: spawning '{}'", args);
    let tid = process::spawn(path, args);
    if tid == u32::MAX {
        println!("init: FAILED to spawn '{}'", path);
        return;
    }

    let code = process::waitpid(tid);
    println!("init: '{}' exited (code={})", path, code);
}

// ── Worker thread ───────────────────────────────────────────────────────────

fn worker_entry() {
    recover_pending_upgrade_if_needed();

    set_status("Starting services...");
    PROGRESS.store(15, Ordering::Release);
    run_service_manager();

    // Done
    set_status("Ready");
    PROGRESS.store(100, Ordering::Release);
    WORKER_DONE.store(true, Ordering::Release);
}

fn recover_pending_upgrade_if_needed() {
    if !libinstall::has_pending_upgrade("/") {
        return;
    }

    set_status("Recovering interrupted upgrade...");
    PROGRESS.store(2, Ordering::Release);
    println!("init: detected interrupted system upgrade, starting recovery");

    match libinstall::recover_pending_upgrade("/") {
        Ok(restored) => {
            println!("init: recovered interrupted upgrade ({} files restored)", restored);
            let _ = fs::mkdir("/System/Logs");
            let _ = fs::write_bytes(
                "/System/Logs/upgrade-recovery.log",
                format!("recovered interrupted upgrade: {} files restored\n", restored).as_bytes(),
            );
        }
        Err(err) => {
            println!("init: FAILED to recover interrupted upgrade: {}", err);
            set_status("Upgrade recovery failed");
        }
    }
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
    #[cfg(target_arch = "x86_64")]
    unsafe {
        *(stack_top as *mut usize) = process::thread_exit_stub_addr();
    }
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
