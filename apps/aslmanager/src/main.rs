//! ASL Manager - one-click Debian setup for Anyos Subsystem for Linux.

#![no_std]
#![no_main]

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::fs;
use anyos_std::ipc;
use anyos_std::process;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use libanyui_client as anyui;
use libanyui_client::Widget;

anyos_std::entry!(main);

const WIN_W: u32 = 820;
const WIN_H: u32 = 520;
const SIDEBAR_W: u32 = 230;

const DISTRO_NAME: &str = "debian";
const IMAGE_REF: &str = "debian-stable-amd64-netboot";
const OWNER: &str = "root";
const ASL_ROOT: &str = "/System/var/asl";
const DISTRO_ROOT: &str = "/System/var/asl/distros/debian";
const BOOT_DIR: &str = "/System/var/asl/distros/debian/boot";
const KERNEL_PATH: &str = "/System/var/asl/distros/debian/boot/vmlinuz";
const INITRD_PATH: &str = "/System/var/asl/distros/debian/boot/initrd.img";
const KERNEL_TMP: &str = "/System/var/asl/distros/debian/boot/vmlinuz.part";
const INITRD_TMP: &str = "/System/var/asl/distros/debian/boot/initrd.img.part";
const DEBIAN_KERNEL_URL: &str =
    "https://deb.debian.org/debian/dists/stable/main/installer-amd64/current/images/netboot/debian-installer/amd64/linux";
const DEBIAN_INITRD_URL: &str =
    "https://deb.debian.org/debian/dists/stable/main/installer-amd64/current/images/netboot/debian-installer/amd64/initrd.gz";

static WORKER_ACTIVE: AtomicBool = AtomicBool::new(false);
static WORKER_DONE: AtomicBool = AtomicBool::new(false);
static WORKER_ERROR: AtomicBool = AtomicBool::new(false);
static LOG_SEQ: AtomicU32 = AtomicU32::new(0);
static LOG_LINE_LEN: AtomicU32 = AtomicU32::new(0);
static mut LOG_LINE_BUF: [u8; 512] = [0u8; 512];
static mut PHASE_LABEL_ID: u32 = 0;
static mut STATUS_LABEL_ID: u32 = 0;
static mut PROGRESS_BAR_ID: u32 = 0;
static mut TERMINAL_BUTTON_ID: u32 = 0;

struct AppState {
    install_btn: anyui::Button,
    terminal_btn: anyui::Button,
    status_label: anyui::Label,
    phase_label: anyui::Label,
    progress_bar: anyui::ProgressBar,
    log_area: anyui::TextArea,
    log_text: String,
    last_log_seq: u32,
    worker: Option<process::Thread>,
}

anyos_std::global_app_state!(AppState);

struct AsldResponse {
    ok: bool,
    lines: Vec<String>,
    message: String,
}

extern "C" fn download_progress_cb(received: u32, total: u32, userdata: u64) {
    if total == 0 {
        return;
    }
    let base = (userdata >> 32) as u32;
    let span = userdata as u32;
    let pct = base + (((received as u64 * span as u64) / total as u64) as u32);
    unsafe {
        anyui::marshal_set_state(PROGRESS_BAR_ID, pct.min(100));
    }
}

fn main() {
    if !anyui::init() {
        anyos_std::println!("[ASL Manager] failed to init libanyui");
        return;
    }
    if !libhttp_client::init() {
        anyos_std::println!("[ASL Manager] failed to init libhttp");
        return;
    }

    let tc = anyui::theme::colors();
    let win = anyui::Window::new("ASL Manager", -1, -1, WIN_W, WIN_H);

    let toolbar = anyui::Toolbar::new();
    toolbar.set_dock(anyui::DOCK_TOP);
    win.add(&toolbar);

    let app_icon = toolbar.add_icon_button("");
    app_icon.set_system_icon("server", anyui::IconType::Outline, 0xFFD9463E, 22);
    app_icon.set_enabled(false);

    let title = toolbar.add_label("ASL Manager");
    title.set_font(1);
    title.set_font_size(14);
    title.set_text_color(tc.text);

    toolbar.add_separator();

    let refresh_btn = toolbar.add_icon_button("Refresh");
    refresh_btn.set_system_icon("refresh", anyui::IconType::Outline, tc.text, 20);

    let root = anyui::View::new();
    root.set_dock(anyui::DOCK_FILL);
    root.set_color(tc.window_bg);
    win.add(&root);

    let sidebar = anyui::View::new();
    sidebar.set_position(0, 0);
    sidebar.set_size(SIDEBAR_W, WIN_H - 40);
    sidebar.set_color(anyui::theme::darken(tc.window_bg, 5));
    root.add(&sidebar);

    let side_title = anyui::Label::new("Distributions");
    side_title.set_position(18, 18);
    side_title.set_size(SIDEBAR_W - 36, 22);
    side_title.set_font_size(13);
    side_title.set_text_color(tc.text_secondary);
    side_title.set_color(anyui::theme::darken(tc.window_bg, 5));
    sidebar.add(&side_title);

    let debian_row = anyui::View::new();
    debian_row.set_position(12, 54);
    debian_row.set_size(SIDEBAR_W - 24, 62);
    debian_row.set_color(anyui::theme::darken(tc.window_bg, 1));
    sidebar.add(&debian_row);

    let debian_name = anyui::Label::new("Debian");
    debian_name.set_position(14, 9);
    debian_name.set_size(SIDEBAR_W - 52, 22);
    debian_name.set_font_size(15);
    debian_name.set_font(1);
    debian_name.set_text_color(tc.text);
    debian_name.set_color(anyui::theme::darken(tc.window_bg, 1));
    debian_row.add(&debian_name);

    let debian_sub = anyui::Label::new("stable amd64, headless");
    debian_sub.set_position(14, 33);
    debian_sub.set_size(SIDEBAR_W - 52, 18);
    debian_sub.set_font_size(11);
    debian_sub.set_text_color(tc.text_secondary);
    debian_sub.set_color(anyui::theme::darken(tc.window_bg, 1));
    debian_row.add(&debian_sub);

    let content_x = SIDEBAR_W as i32;
    let content_w = WIN_W - SIDEBAR_W;

    let heading = anyui::Label::new("Debian for ASL");
    heading.set_position(content_x + 24, 24);
    heading.set_size(content_w - 48, 30);
    heading.set_font(1);
    heading.set_font_size(21);
    heading.set_text_color(tc.text);
    heading.set_color(tc.window_bg);
    root.add(&heading);

    let summary = anyui::Label::new(
        "Downloads Debian netboot artifacts, registers an ASL distro, and starts it immediately.",
    );
    summary.set_position(content_x + 24, 58);
    summary.set_size(content_w - 48, 36);
    summary.set_font_size(12);
    summary.set_text_color(tc.text_secondary);
    summary.set_color(tc.window_bg);
    root.add(&summary);

    let install_btn = anyui::Button::new("Install & Start");
    install_btn.set_position(content_x + 24, 112);
    install_btn.set_size(142, 34);
    install_btn.set_color(tc.accent);
    install_btn.set_text_color(0xFFFFFFFF);
    root.add(&install_btn);

    let terminal_btn = anyui::Button::new("Open Terminal");
    terminal_btn.set_position(content_x + 178, 112);
    terminal_btn.set_size(132, 34);
    terminal_btn.set_enabled(false);
    root.add(&terminal_btn);

    let status_label = anyui::Label::new("Ready.");
    status_label.set_position(content_x + 24, 166);
    status_label.set_size(content_w - 48, 20);
    status_label.set_font_size(12);
    status_label.set_text_color(tc.text);
    status_label.set_color(tc.window_bg);
    root.add(&status_label);

    let phase_label = anyui::Label::new("No installation running.");
    phase_label.set_position(content_x + 24, 192);
    phase_label.set_size(content_w - 48, 20);
    phase_label.set_font_size(12);
    phase_label.set_text_color(tc.text_secondary);
    phase_label.set_color(tc.window_bg);
    root.add(&phase_label);

    let progress_bar = anyui::ProgressBar::new(0);
    progress_bar.set_position(content_x + 24, 224);
    progress_bar.set_size(content_w - 48, 18);
    root.add(&progress_bar);

    let details_title = anyui::Label::new("Install log");
    details_title.set_position(content_x + 24, 270);
    details_title.set_size(content_w - 48, 20);
    details_title.set_font(1);
    details_title.set_font_size(13);
    details_title.set_text_color(tc.text);
    details_title.set_color(tc.window_bg);
    root.add(&details_title);

    let log_area = anyui::TextArea::new();
    log_area.set_position(content_x + 24, 296);
    log_area.set_size(content_w - 48, 150);
    log_area.set_read_only(true);
    log_area.set_font(4);
    log_area.set_font_size(12);
    log_area.set_text("ASL Manager ready.\n");
    root.add(&log_area);

    unsafe {
        PHASE_LABEL_ID = Widget::id(&phase_label);
        STATUS_LABEL_ID = Widget::id(&status_label);
        PROGRESS_BAR_ID = Widget::id(&progress_bar);
        TERMINAL_BUTTON_ID = Widget::id(&terminal_btn);
    }

    unsafe {
        APP = Some(AppState {
            install_btn,
            terminal_btn,
            status_label,
            phase_label,
            progress_bar,
            log_area,
            log_text: String::from("ASL Manager ready.\n"),
            last_log_seq: 0,
            worker: None,
        });
    }

    refresh_status();

    app().install_btn.on_click(|_| {
        start_install();
    });

    app().terminal_btn.on_click(|_| {
        open_terminal();
    });

    refresh_btn.on_click(|_| {
        refresh_status();
    });

    let _timer_id = anyui::set_timer(150, || on_timer());

    anyui::run();
}

fn refresh_status() {
    let installed = file_exists(KERNEL_PATH) && file_exists(INITRD_PATH);
    let status = request_asld(&format!("STATUS {}", DISTRO_NAME));
    let state = match status {
        Ok(resp) if resp.ok => {
            String::from(field_value(&resp.lines, "state").unwrap_or("registered"))
        }
        _ if installed => String::from("artifacts ready"),
        _ => String::from("not installed"),
    };
    let text = format!("Debian: {}", state);
    app().status_label.set_text(&text);
    app().terminal_btn.set_enabled(installed);
}

fn start_install() {
    if WORKER_ACTIVE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    WORKER_DONE.store(false, Ordering::Release);
    WORKER_ERROR.store(false, Ordering::Release);
    app().install_btn.set_enabled(false);
    app().terminal_btn.set_enabled(false);
    app().progress_bar.set_state(0);
    app().phase_label.set_text("Starting Debian setup...");
    log_line_ui("Starting Debian one-click setup.");

    match process::Thread::spawn(install_worker, "asl-debian-install") {
        Ok(thread) => {
            app().worker = Some(thread);
        }
        Err(_) => {
            WORKER_ACTIVE.store(false, Ordering::Release);
            WORKER_DONE.store(true, Ordering::Release);
            WORKER_ERROR.store(true, Ordering::Release);
            app().install_btn.set_enabled(true);
            app()
                .status_label
                .set_text("Could not start installer thread.");
            log_line_ui("ERROR: failed to spawn installer thread.");
        }
    }
}

fn on_timer() {
    let seq = LOG_SEQ.load(Ordering::Acquire);
    if seq != app().last_log_seq {
        app().last_log_seq = seq;
        let len = LOG_LINE_LEN.load(Ordering::Acquire) as usize;
        let line = unsafe { core::str::from_utf8(&LOG_LINE_BUF[..len]).unwrap_or("") };
        app().log_text.push_str(line);
        app().log_text.push('\n');
        app().log_area.set_text(&app().log_text);
        app().log_area.set_cursor(app().log_text.len() as u32);
    }

    if WORKER_DONE.swap(false, Ordering::AcqRel) {
        if let Some(thread) = app().worker.take() {
            let _ = thread.join();
        }
        WORKER_ACTIVE.store(false, Ordering::Release);
        app().install_btn.set_enabled(true);
        app().terminal_btn.set_enabled(true);
        if WORKER_ERROR.load(Ordering::Acquire) {
            app().status_label.set_text("Debian setup failed.");
        } else {
            app()
                .status_label
                .set_text("Debian is installed and starting.");
            app().progress_bar.set_state(100);
        }
    }
}

fn install_worker() {
    set_status("Preparing ASL storage...");
    set_phase("Creating ASL distro directories");
    set_progress(4);

    if !ensure_dirs() {
        finish_error("could not create ASL directory tree");
        return;
    }

    if !ensure_artifact(
        KERNEL_PATH,
        KERNEL_TMP,
        DEBIAN_KERNEL_URL,
        "Linux kernel",
        10,
        25,
    ) {
        finish_error("kernel download failed");
        return;
    }
    if !ensure_artifact(
        INITRD_PATH,
        INITRD_TMP,
        DEBIAN_INITRD_URL,
        "Debian initrd",
        40,
        35,
    ) {
        finish_error("initrd download failed");
        return;
    }

    set_progress(78);
    set_phase("Registering Debian with asld");
    match request_asld(&format!("STATUS {}", DISTRO_NAME)) {
        Ok(resp) if resp.ok => log_line("ASL distro already exists; reusing configuration."),
        _ => {
            let cmd = format!("CREATE {}\t{}\t{}\t-", DISTRO_NAME, IMAGE_REF, OWNER);
            match request_asld(&cmd) {
                Ok(resp) if resp.ok => log_line("ASL distro created."),
                Ok(resp) => {
                    finish_error(&format!("asld create failed: {}", resp.message));
                    return;
                }
                Err(err) => {
                    finish_error(err);
                    return;
                }
            }
        }
    }

    set_progress(88);
    set_phase("Starting Debian VM");
    match request_asld(&format!("START {}", DISTRO_NAME)) {
        Ok(resp) if resp.ok => {
            log_line("Debian start request accepted by asld.");
        }
        Ok(resp) if resp.message.contains("already running") => {
            log_line("Debian is already running.");
        }
        Ok(resp) => {
            finish_error(&format!("asld start failed: {}", resp.message));
            return;
        }
        Err(err) => {
            finish_error(err);
            return;
        }
    }

    set_progress(100);
    set_phase("Debian is starting");
    set_status("Debian is starting. Open Terminal to attach.");
    log_line("Done. Debian has been installed and started.");
    WORKER_ERROR.store(false, Ordering::Release);
    WORKER_DONE.store(true, Ordering::Release);
}

fn ensure_artifact(path: &str, tmp: &str, url: &str, label: &str, base: u32, span: u32) -> bool {
    if file_exists(path) {
        log_line(&format!("{} already present.", label));
        return true;
    }
    let _ = fs::unlink(tmp);
    set_phase(&format!("Downloading {}", label));
    log_line(&format!("Downloading {} from Debian mirror.", label));
    let userdata = ((base as u64) << 32) | span as u64;
    if !libhttp_client::download_progress(url, tmp, download_progress_cb, userdata) {
        let _ = fs::unlink(tmp);
        return false;
    }
    let _ = fs::unlink(path);
    if fs::rename(tmp, path) != 0 {
        let _ = fs::unlink(tmp);
        return false;
    }
    log_line(&format!("{} ready.", label));
    true
}

fn ensure_dirs() -> bool {
    for dir in [
        "/System/var",
        ASL_ROOT,
        "/System/var/asl/distros",
        DISTRO_ROOT,
        BOOT_DIR,
    ] {
        if fs::mkdir(dir) == u32::MAX && !dir_exists(dir) {
            return false;
        }
    }
    true
}

fn request_asld(command: &str) -> Result<AsldResponse, &'static str> {
    const RESPONSE_TIMEOUT_TICKS: u32 = 200;
    const RESPONSE_SLEEP_MS: u32 = 20;

    let pid = process::getpid();
    let reply_name = format!("asld-{}", pid);
    let old_reply = ipc::pipe_open(&reply_name);
    if old_reply != 0 {
        let _ = ipc::pipe_close(old_reply);
    }

    let reply_pipe = ipc::pipe_create(&reply_name);
    if reply_pipe == 0 || reply_pipe == u32::MAX {
        return Err("failed to create asld reply pipe");
    }

    let request_pipe = ipc::pipe_open("asld");
    if request_pipe == 0 || request_pipe == u32::MAX {
        let _ = ipc::pipe_close(reply_pipe);
        return Err("asld is not running");
    }

    let request = format!("{}\t{}\n", pid, command);
    if ipc::pipe_write(request_pipe, request.as_bytes()) == u32::MAX {
        let _ = ipc::pipe_close(reply_pipe);
        return Err("failed to write asld request");
    }

    let mut raw = String::new();
    let mut buf = [0u8; 1024];
    for _ in 0..RESPONSE_TIMEOUT_TICKS {
        let n = ipc::pipe_read(reply_pipe, &mut buf);
        if n == u32::MAX {
            let _ = ipc::pipe_close(reply_pipe);
            return Err("failed to read asld response");
        }
        if n > 0 {
            let chunk = match core::str::from_utf8(&buf[..n as usize]) {
                Ok(text) => text,
                Err(_) => {
                    let _ = ipc::pipe_close(reply_pipe);
                    return Err("asld response was not valid UTF-8");
                }
            };
            raw.push_str(chunk);
            if raw.ends_with("\n\n") {
                let _ = ipc::pipe_close(reply_pipe);
                return Ok(parse_asld_response(&raw));
            }
        }
        process::sleep(RESPONSE_SLEEP_MS);
    }

    let _ = ipc::pipe_close(reply_pipe);
    Err("timed out waiting for asld")
}

fn parse_asld_response(raw: &str) -> AsldResponse {
    let trimmed = raw.trim_matches('\n');
    let mut lines = trimmed.split('\n');
    let header = lines.next().unwrap_or("");
    let mut header_parts = header.split('\t');
    match header_parts.next() {
        Some("OK") => AsldResponse {
            ok: true,
            lines: lines
                .filter(|line| !line.is_empty())
                .map(String::from)
                .collect(),
            message: String::new(),
        },
        Some("ERR") => {
            let code = header_parts.next().unwrap_or("unknown");
            let msg = join_tab_fields(&mut header_parts);
            AsldResponse {
                ok: false,
                lines: Vec::new(),
                message: if msg.is_empty() {
                    String::from(code)
                } else {
                    format!("{} {}", code, msg)
                },
            }
        }
        _ => AsldResponse {
            ok: false,
            lines: Vec::new(),
            message: String::from("invalid asld response"),
        },
    }
}

fn join_tab_fields<'a>(parts: &mut core::str::Split<'a, char>) -> String {
    let mut out = String::new();
    for part in parts {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(part);
    }
    out
}

fn field_value<'a>(lines: &'a [String], key: &str) -> Option<&'a str> {
    for line in lines {
        let mut parts = line.split('\t');
        if parts.next() == Some(key) {
            return parts.next();
        }
    }
    None
}

fn file_exists(path: &str) -> bool {
    let mut stat_buf = [0u32; 7];
    fs::stat(path, &mut stat_buf) == 0 && stat_buf[1] > 0
}

fn dir_exists(path: &str) -> bool {
    let mut stat_buf = [0u32; 7];
    fs::stat(path, &mut stat_buf) == 0
}

fn set_phase(text: &str) {
    unsafe {
        anyui::marshal_set_text(PHASE_LABEL_ID, text);
    }
    log_line(text);
}

fn set_status(text: &str) {
    unsafe {
        anyui::marshal_set_text(STATUS_LABEL_ID, text);
    }
}

fn set_progress(pct: u32) {
    unsafe {
        anyui::marshal_set_state(PROGRESS_BAR_ID, pct.min(100));
    }
}

fn finish_error(message: &str) {
    set_phase("Setup failed");
    set_status(message);
    log_line(&format!("ERROR: {}", message));
    WORKER_ERROR.store(true, Ordering::Release);
    WORKER_DONE.store(true, Ordering::Release);
}

fn log_line(line: &str) {
    let len = line.len().min(511);
    unsafe {
        LOG_LINE_BUF[..len].copy_from_slice(&line.as_bytes()[..len]);
    }
    LOG_LINE_LEN.store(len as u32, Ordering::Release);
    LOG_SEQ.fetch_add(1, Ordering::Release);
}

fn log_line_ui(line: &str) {
    app().log_text.push_str(line);
    app().log_text.push('\n');
    app().log_area.set_text(&app().log_text);
}

fn open_terminal() {
    unsafe {
        anyui::marshal_set_visible(TERMINAL_BUTTON_ID, true);
    }
    let tid = process::launch_app("/Applications/Terminal.app/Terminal", "");
    if tid == u32::MAX {
        app()
            .status_label
            .set_text("Could not open Terminal. Run aslctl shell debian.");
    } else {
        app()
            .status_label
            .set_text("Terminal opened. Run: aslctl shell debian --fallback-console");
    }
}
