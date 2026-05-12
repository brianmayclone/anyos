use anyos_std::{format, String};
use core::sync::atomic::Ordering;
use libanyui_client as ui;
use ui::{DOCK_BOTTOM, DOCK_FILL, DOCK_TOP};

use crate::inspect;
use crate::state::*;
use crate::worker;

pub fn build() {
    let tc = ui::theme::colors();
    let win = ui::Window::new("LXE Manager", -1, -1, WIN_W, WIN_H);
    ui::set_blur_behind(&win, 16);

    let bottom = ui::View::new();
    bottom.set_dock(DOCK_BOTTOM);
    bottom.set_size(WIN_W, 58);
    bottom.set_color(ui::theme::darken(tc.window_bg, 5));
    win.add(&bottom);

    let divider = ui::Divider::new();
    divider.set_dock(DOCK_TOP);
    divider.set_size(WIN_W, 1);
    bottom.add(&divider);

    let btn_back = ui::Button::new("Back");
    btn_back.set_position(20, 14);
    btn_back.set_size(86, 30);
    btn_back.set_visible(false);
    bottom.add(&btn_back);

    let btn_secondary = ui::Button::new("Delete Rootfs");
    btn_secondary.set_position(WIN_W as i32 - 250, 14);
    btn_secondary.set_size(112, 30);
    btn_secondary.set_visible(false);
    bottom.add(&btn_secondary);

    let btn_primary = ui::Button::new("Install");
    btn_primary.set_position(WIN_W as i32 - 124, 14);
    btn_primary.set_size(104, 30);
    btn_primary.set_color(ACCENT);
    btn_primary.set_text_color(0xFFFFFFFF);
    bottom.add(&btn_primary);

    let welcome = build_welcome_page(&win, tc.window_bg);
    let manage_widgets = build_manage_page(&win, tc.window_bg);
    let progress_widgets = build_progress_page(&win, tc.window_bg);
    let done_widgets = build_done_page(&win, tc.window_bg);

    set_app(App {
        welcome,
        manage: manage_widgets.0,
        progress: progress_widgets.0,
        done: done_widgets.0,
        btn_back,
        btn_primary,
        btn_secondary,
        info_title: manage_widgets.1,
        info_subtitle: manage_widgets.2,
        info_root: manage_widgets.3,
        info_files: manage_widgets.4,
        info_packages: manage_widgets.5,
        info_daemon: manage_widgets.6,
        progress_bar: progress_widgets.1,
        progress_title: progress_widgets.2,
        progress_detail: progress_widgets.3,
        progress_eta: progress_widgets.4,
        done_title: done_widgets.1,
        done_detail: done_widgets.2,
        mode: ScreenMode::Welcome,
        timer_id: 0,
        active_operation: Operation::Install,
    });

    app().btn_primary.on_click(|_| primary_action());
    app().btn_secondary.on_click(|_| delete_action());
    app().btn_back.on_click(|_| {
        refresh();
    });
    refresh();
}

fn build_welcome_page(win: &ui::Window, bg: u32) -> ui::View {
    let page = ui::View::new();
    page.set_dock(DOCK_FILL);
    page.set_color(bg);
    win.add(&page);

    let mark = ui::Label::new("LXE");
    mark.set_position(0, 68);
    mark.set_size(WIN_W, 42);
    mark.set_font_size(30);
    mark.set_font(1);
    mark.set_text_color(0xFFFFFFFF);
    mark.set_text_align(ui::TEXT_ALIGN_CENTER);
    page.add(&mark);

    let title = ui::Label::new("Welcome to Linux Experience Extension");
    title.set_position(0, 126);
    title.set_size(WIN_W, 34);
    title.set_font_size(24);
    title.set_text_color(0xFFFFFFFF);
    title.set_text_align(ui::TEXT_ALIGN_CENTER);
    page.add(&title);

    let sub = ui::Label::new(
        "This assistant installs a shared Linux rootfs used by every lxe run session.",
    );
    sub.set_position(130, 176);
    sub.set_size(560, 42);
    sub.set_font_size(14);
    sub.set_text_color(0xFFB8B8BE);
    sub.set_text_align(ui::TEXT_ALIGN_CENTER);
    page.add(&sub);

    add_step(
        &page,
        206,
        "1",
        "Prepare",
        "Create the LXE layout under /System/var/lxe.",
    );
    add_step(
        &page,
        270,
        "2",
        "Download",
        "Fetch Debian package metadata and missing bootstrap packages.",
    );
    add_step(
        &page,
        334,
        "3",
        "Install",
        "Unpack binaries, libraries and Linux runtime metadata.",
    );
    add_step(
        &page,
        398,
        "4",
        "Sync",
        "Ask lxed to synchronize the shared rootfs context.",
    );

    page
}

fn build_manage_page(
    win: &ui::Window,
    bg: u32,
) -> (
    ui::View,
    ui::Label,
    ui::Label,
    ui::Label,
    ui::Label,
    ui::Label,
    ui::Label,
) {
    let page = ui::View::new();
    page.set_dock(DOCK_FILL);
    page.set_color(bg);
    page.set_visible(false);
    win.add(&page);

    let title = ui::Label::new("");
    title.set_position(44, 42);
    title.set_size(620, 34);
    title.set_font_size(24);
    title.set_font(1);
    title.set_text_color(0xFFFFFFFF);
    page.add(&title);

    let subtitle = ui::Label::new("");
    subtitle.set_position(44, 82);
    subtitle.set_size(700, 38);
    subtitle.set_font_size(13);
    subtitle.set_text_color(0xFFA7A7AD);
    page.add(&subtitle);

    let root = info_card(&page, 44, 148, "Location");
    let files = info_card(&page, 424, 148, "Payload");
    let packages = info_card(&page, 44, 266, "Bootstrap");
    let daemon = info_card(&page, 424, 266, "Runtime");

    (page, title, subtitle, root, files, packages, daemon)
}

fn build_progress_page(
    win: &ui::Window,
    bg: u32,
) -> (ui::View, ui::ProgressBar, ui::Label, ui::Label, ui::Label) {
    let page = ui::View::new();
    page.set_dock(DOCK_FILL);
    page.set_color(bg);
    page.set_visible(false);
    win.add(&page);

    let title = ui::Label::new("Installing LXE");
    title.set_position(0, 94);
    title.set_size(WIN_W, 34);
    title.set_font_size(24);
    title.set_font(1);
    title.set_text_color(0xFFFFFFFF);
    title.set_text_align(ui::TEXT_ALIGN_CENTER);
    page.add(&title);

    let detail = ui::Label::new("Preparing...");
    detail.set_position(110, 142);
    detail.set_size(600, 32);
    detail.set_font_size(13);
    detail.set_text_color(0xFFB8B8BE);
    detail.set_text_align(ui::TEXT_ALIGN_CENTER);
    page.add(&detail);

    let progress = ui::ProgressBar::new(0);
    progress.set_position(160, 210);
    progress.set_size(500, 16);
    page.add(&progress);

    let eta = ui::Label::new("Estimating time remaining...");
    eta.set_position(0, 244);
    eta.set_size(WIN_W, 22);
    eta.set_font_size(12);
    eta.set_text_color(0xFF8E8E93);
    eta.set_text_align(ui::TEXT_ALIGN_CENTER);
    page.add(&eta);

    (page, progress, title, detail, eta)
}

fn build_done_page(win: &ui::Window, bg: u32) -> (ui::View, ui::Label, ui::Label) {
    let page = ui::View::new();
    page.set_dock(DOCK_FILL);
    page.set_color(bg);
    page.set_visible(false);
    win.add(&page);

    let title = ui::Label::new("LXE is ready");
    title.set_position(0, 138);
    title.set_size(WIN_W, 38);
    title.set_font_size(26);
    title.set_font(1);
    title.set_text_color(0xFFFFFFFF);
    title.set_text_align(ui::TEXT_ALIGN_CENTER);
    page.add(&title);

    let detail = ui::Label::new("Click Done to close LXE Manager.");
    detail.set_position(120, 190);
    detail.set_size(580, 42);
    detail.set_font_size(14);
    detail.set_text_color(0xFFB8B8BE);
    detail.set_text_align(ui::TEXT_ALIGN_CENTER);
    page.add(&detail);

    (page, title, detail)
}

fn add_step(parent: &ui::View, y: i32, number: &str, title: &str, detail: &str) {
    let dot = ui::Label::new(number);
    dot.set_position(190, y);
    dot.set_size(30, 30);
    dot.set_color(ACCENT);
    dot.set_text_color(0xFFFFFFFF);
    dot.set_text_align(ui::TEXT_ALIGN_CENTER);
    dot.set_font_size(13);
    parent.add(&dot);

    let head = ui::Label::new(title);
    head.set_position(240, y - 1);
    head.set_size(160, 20);
    head.set_font_size(14);
    head.set_font(1);
    head.set_text_color(0xFFFFFFFF);
    parent.add(&head);

    let body = ui::Label::new(detail);
    body.set_position(240, y + 19);
    body.set_size(420, 24);
    body.set_font_size(12);
    body.set_text_color(0xFF98989F);
    parent.add(&body);
}

fn info_card(parent: &ui::View, x: i32, y: i32, label: &str) -> ui::Label {
    let card = ui::Card::new();
    card.set_position(x, y);
    card.set_size(352, 86);
    card.set_color(0xFF242429);
    card.set_style(ui::STYLE_RADIUS, 8);
    parent.add(&card);

    let caption = ui::Label::new(label);
    caption.set_position(18, 13);
    caption.set_size(300, 18);
    caption.set_font_size(11);
    caption.set_text_color(0xFF8E8E93);
    card.add(&caption);

    let value = ui::Label::new("");
    value.set_position(18, 38);
    value.set_size(310, 34);
    value.set_font_size(13);
    value.set_text_color(0xFFFFFFFF);
    card.add(&value);
    value
}

fn refresh() {
    let snap = inspect::load_snapshot();
    app().info_title.set_text(&snap.title);
    app().info_subtitle.set_text(&snap.subtitle);
    app().info_root.set_text(&snap.root_line);
    app().info_files.set_text(&snap.file_line);
    app().info_packages.set_text(&snap.package_line);
    app().info_daemon.set_text(&snap.daemon_line);

    if snap.has_payload {
        app().active_operation = Operation::Repair;
        show(ScreenMode::Manage);
        app()
            .btn_primary
            .set_text(if snap.complete { "Repair" } else { "Repair" });
        app().btn_secondary.set_visible(true);
    } else {
        app().active_operation = Operation::Install;
        show(ScreenMode::Welcome);
        app().btn_primary.set_text("Install");
        app().btn_secondary.set_visible(false);
    }
    app().btn_primary.set_color(ACCENT);
    app().btn_primary.set_text_color(0xFFFFFFFF);
}

fn primary_action() {
    match app().mode {
        ScreenMode::Welcome => start_operation(Operation::Install),
        ScreenMode::Manage => start_operation(Operation::Repair),
        ScreenMode::Done => ui::quit(),
        ScreenMode::Progress => {}
    }
}

fn delete_action() {
    ui::MessageBox::show(
        ui::MessageBoxType::Warning,
        "The LXE rootfs will be removed. Cached package files stay untouched.",
        Some("Delete Rootfs"),
    );
    start_operation(Operation::Delete);
}

fn start_operation(op: Operation) {
    app().active_operation = op;
    app().progress_bar.set_state(0);
    app().progress_title.set_text(match op {
        Operation::Install => "Installing LXE",
        Operation::Repair => "Repairing LXE",
        Operation::Delete => "Deleting LXE Rootfs",
    });
    app().progress_detail.set_text("Preparing...");
    app().progress_eta.set_text("Estimating time remaining...");
    show(ScreenMode::Progress);
    worker::spawn(op);
    app().timer_id = ui::set_timer(250, || poll_worker());
}

fn poll_worker() {
    let seq = PROGRESS_SEQ.load(Ordering::Acquire);
    if seq != 0 {
        let percent = PROGRESS_PERCENT.load(Ordering::Acquire);
        app().progress_bar.set_state(percent);
        let phase = read_static_text(0);
        let detail = read_static_text(1);
        let step = PROGRESS_STEP.load(Ordering::Acquire);
        let steps = PROGRESS_STEPS.load(Ordering::Acquire);
        app()
            .progress_title
            .set_text(&format!("{} - {}%", phase, percent));
        app()
            .progress_detail
            .set_text(&format!("Step {}/{}: {}", step, steps, detail));
        app().progress_eta.set_text(&eta_line());
    }

    if WORKER_DONE.load(Ordering::Acquire) {
        let timer = app().timer_id;
        if timer != 0 {
            ui::kill_timer(timer);
            app().timer_id = 0;
        }
        if WORKER_ERROR.load(Ordering::Acquire) {
            let err = read_static_text(2);
            show_done("LXE operation failed", &err, true);
        } else {
            let op = app().active_operation;
            let detail = match op {
                Operation::Install => {
                    "The Linux rootfs is installed and lxed has synchronized the shared context."
                }
                Operation::Repair => {
                    "The existing LXE installation has been repaired and synchronized."
                }
                Operation::Delete => {
                    "The LXE rootfs has been removed. Reopen LXE Manager to install again."
                }
            };
            show_done("LXE operation complete", detail, false);
        }
    }
}

fn eta_line() -> String {
    let done = PROGRESS_BYTES_DONE.load(Ordering::Acquire);
    let total = PROGRESS_BYTES_TOTAL.load(Ordering::Acquire);
    let eta = PROGRESS_ETA.load(Ordering::Acquire);
    if total > 0 {
        format!(
            "{} of {} downloaded - about {} remaining",
            bytes(done as u64),
            bytes(total as u64),
            time_left(eta)
        )
    } else if eta > 0 {
        format!("About {} remaining", time_left(eta))
    } else {
        String::from("Estimating time remaining from install steps...")
    }
}

fn show_done(title: &str, detail: &str, error: bool) {
    app().done_title.set_text(title);
    app()
        .done_title
        .set_text_color(if error { RED } else { GREEN });
    app().done_detail.set_text(detail);
    app().btn_primary.set_text("Done");
    app().btn_secondary.set_visible(false);
    app().btn_back.set_visible(false);
    show(ScreenMode::Done);
}

fn show(mode: ScreenMode) {
    app().mode = mode;
    app().welcome.set_visible(mode == ScreenMode::Welcome);
    app().manage.set_visible(mode == ScreenMode::Manage);
    app().progress.set_visible(mode == ScreenMode::Progress);
    app().done.set_visible(mode == ScreenMode::Done);
    app().btn_back.set_visible(false);
    app().btn_primary.set_visible(mode != ScreenMode::Progress);
    app().btn_secondary.set_visible(mode == ScreenMode::Manage);
}

fn read_static_text(which: u32) -> String {
    match which {
        0 => {
            let len = PROGRESS_PHASE_LEN.load(Ordering::Acquire) as usize;
            unsafe {
                core::str::from_utf8(&PROGRESS_PHASE_BUF[..len])
                    .unwrap_or("")
                    .into()
            }
        }
        1 => {
            let len = PROGRESS_DETAIL_LEN.load(Ordering::Acquire) as usize;
            unsafe {
                core::str::from_utf8(&PROGRESS_DETAIL_BUF[..len])
                    .unwrap_or("")
                    .into()
            }
        }
        _ => {
            let len = ERROR_LEN.load(Ordering::Acquire) as usize;
            unsafe { core::str::from_utf8(&ERROR_BUF[..len]).unwrap_or("").into() }
        }
    }
}

fn bytes(value: u64) -> String {
    if value >= 1024 * 1024 {
        format!("{} MiB", value / (1024 * 1024))
    } else if value >= 1024 {
        format!("{} KiB", value / 1024)
    } else {
        format!("{} B", value)
    }
}

fn time_left(seconds: u32) -> String {
    if seconds >= 60 {
        format!("{}:{:02} min", seconds / 60, seconds % 60)
    } else {
        format!("{} sec", seconds)
    }
}
