#![no_std]
#![no_main]

use anyos_std::{format, String, Vec};
use anyos_std::{fs, sys, process};
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use libanyui_client as ui;
use ui::{ColumnDef, Widget, ALIGN_RIGHT, DOCK_TOP, DOCK_BOTTOM, DOCK_FILL};

anyos_std::entry!(main);

// ── Shared worker state ─────────────────────────────────────────────────────

static WORKER_ACTIVE: AtomicBool = AtomicBool::new(false);
static WORKER_DONE: AtomicBool = AtomicBool::new(false);
static WORKER_ERROR: AtomicBool = AtomicBool::new(false);
static WORKER_PROGRESS: AtomicU32 = AtomicU32::new(0);
static INSTALL_DISK_ID: AtomicU32 = AtomicU32::new(u32::MAX);
static INSTALL_MODE: AtomicU32 = AtomicU32::new(0); // 0=auto, 1=manual

static mut PROGRESS_BAR_ID: u32 = 0;
static mut STATUS_LABEL_ID: u32 = 0;
static mut PHASE_LABEL_ID: u32 = 0;
static mut BTN_REBOOT_ID: u32 = 0;
// Shared buffer for current file being copied (worker → UI thread)
static COPY_FILE_SEQ: AtomicU32 = AtomicU32::new(0);
static COPY_FILE_LEN: AtomicU32 = AtomicU32::new(0);
static mut COPY_FILE_BUF: [u8; 256] = [0u8; 256];
// Total bytes copied (worker increments, UI reads for MB/s display)
static COPY_BYTES_TOTAL: AtomicU32 = AtomicU32::new(0);
static COPY_START_MS: AtomicU32 = AtomicU32::new(0);
static COPY_FILES_SINCE_UPDATE: AtomicU32 = AtomicU32::new(0);
static mut COPY_CURRENT_LABEL: [u8; 32] = [0u8; 32];
static COPY_CURRENT_LABEL_LEN: AtomicU32 = AtomicU32::new(0);

// ── Constants ───────────────────────────────────────────────────────────────

const SECTOR_SIZE: u32 = 512;
const PARTITION_START: u32 = 128;
const FAT_OFFSET: u32 = 32;
const SPC: u32 = 8;
const SPC_SHIFT: u8 = 3;
const CLUSTER_SIZE: u32 = SPC * SECTOR_SIZE;
const FS_TYPE_EXFAT: u32 = 7;

const WIN_W: u32 = 780;
const WIN_H: u32 = 520;

// ── Data ────────────────────────────────────────────────────────────────────

struct DiskEntry {
    device_id: u8,
    disk_id: u8,
    size_sectors: u64,
    partition_count: u32,
}

struct InstallerApp {
    win: ui::Window,
    // Sidebar step indicators
    sidebar: ui::View,
    step_labels: [ui::Label; 4],
    step_dots: [ui::View; 4],
    // Pages
    page0: ui::View,
    page1: ui::View,
    page2: ui::View,
    page3: ui::View,
    // Page 1
    disk_grid: ui::DataGrid,
    // Page 2
    method_info: ui::Label,
    // Page 3
    progress_bar: ui::ProgressBar,
    phase_label: ui::Label,
    status_label: ui::Label,
    btn_reboot: ui::Button,
    btn_details: ui::Button,
    details_card: ui::Card,
    details_log: ui::TextArea,
    details_text: String,
    // State
    disks: Vec<DiskEntry>,
    selected_disk: Option<usize>,
    current_step: u32,
    timer_id: u32,
    details_visible: bool,
    last_copy_seq: u32,
}

static mut APP: Option<InstallerApp> = None;
fn app() -> &'static mut InstallerApp { unsafe { APP.as_mut().unwrap() } }

// ── Helpers ─────────────────────────────────────────────────────────────────

fn detect_disks() -> Vec<DiskEntry> {
    let mut buf = [0u8; 32 * 32];
    let count = sys::disk_list(&mut buf);
    let mut disks = Vec::new();
    for i in 0..count as usize {
        let off = i * 32;
        let device_id = buf[off];
        let disk_id = buf[off + 1];
        let partition = buf[off + 2];
        let size_sectors = u64::from_le_bytes([
            buf[off+12], buf[off+13], buf[off+14], buf[off+15],
            buf[off+16], buf[off+17], buf[off+18], buf[off+19],
        ]);
        if partition == 0xFF {
            let part_count = (0..count as usize)
                .filter(|&j| {
                    let jo = j * 32;
                    buf[jo + 1] == disk_id && buf[jo + 2] != 0xFF
                })
                .count() as u32;
            disks.push(DiskEntry { device_id, disk_id, size_sectors, partition_count: part_count });
        }
    }
    disks.sort_by_key(|d| d.disk_id);
    disks
}

fn format_size(sectors: u64) -> String {
    let bytes = sectors * 512;
    if bytes >= 1024 * 1024 * 1024 {
        let gb = bytes / (1024 * 1024 * 1024);
        let frac = (bytes % (1024 * 1024 * 1024)) * 10 / (1024 * 1024 * 1024);
        format!("{}.{} GB", gb, frac)
    } else if bytes >= 1024 * 1024 {
        format!("{} MB", bytes / (1024 * 1024))
    } else {
        format!("{} KB", bytes / 1024)
    }
}

fn populate_disk_grid(grid: &ui::DataGrid, disks: &[DiskEntry]) {
    let rows: Vec<Vec<&str>> = Vec::new();
    let mut row_strings: Vec<[String; 4]> = Vec::new();
    for disk in disks {
        row_strings.push([
            format!("Disk {}", disk.disk_id),
            format_size(disk.size_sectors),
            format!("{}", disk.partition_count),
            format!("{}", disk.device_id),
        ]);
    }
    let row_refs: Vec<Vec<&str>> = row_strings.iter().map(|r| {
        r.iter().map(|s| s.as_str()).collect()
    }).collect();
    grid.set_row_count(disks.len() as u32);
    grid.set_data(&row_refs);
}

// ── Sidebar step management ─────────────────────────────────────────────────

const ACCENT: u32 = 0xFF007AFF;      // Apple blue
const ACCENT_DIM: u32 = 0xFF3A3A5C;  // Inactive step
const STEP_DONE: u32 = 0xFF34C759;   // Green checkmark color

fn update_sidebar(step: u32) {
    let a = app();
    a.current_step = step;
    for i in 0..4 {
        let active = step >= (i + 1) as u32;
        a.step_dots[i].set_color(if active { ACCENT } else { ACCENT_DIM });
        a.step_labels[i].set_text_color(if active { 0xFFFFFFFF } else { 0xFF888888 });
    }
}

fn show_page(step: u32) {
    let a = app();
    a.page0.set_visible(step == 0);
    a.page1.set_visible(step == 1);
    a.page2.set_visible(step == 2);
    a.page3.set_visible(step == 3);
    update_sidebar(step);
}

// ── UI construction ─────────────────────────────────────────────────────────

fn main() {
    if !ui::init() { return; }

    let tc = ui::theme::colors();
    let win = ui::Window::new("anyOS Installer", -1, -1, WIN_W, WIN_H);

    // ── Sidebar (left, step indicators) ──
    let sidebar = ui::View::new();
    sidebar.set_dock(ui::DOCK_LEFT);
    sidebar.set_size(170, WIN_H);
    sidebar.set_color(ui::theme::darken(tc.window_bg, 15));
    win.add(&sidebar);

    // App title in sidebar
    let logo_label = ui::Label::new("anyOS");
    logo_label.set_position(20, 20);
    logo_label.set_size(130, 28);
    logo_label.set_font_size(20);
    logo_label.set_color(ui::theme::darken(tc.window_bg, 15));
    logo_label.set_text_color(0xFFFFFFFF);
    sidebar.add(&logo_label);

    let sub_label = ui::Label::new("Installer");
    sub_label.set_position(20, 48);
    sub_label.set_size(130, 18);
    sub_label.set_font_size(12);
    sub_label.set_color(ui::theme::darken(tc.window_bg, 15));
    sub_label.set_text_color(0xFF888888);
    sidebar.add(&sub_label);

    // Divider
    let side_div = ui::Divider::new();
    side_div.set_position(15, 78);
    side_div.set_size(140, 1);
    sidebar.add(&side_div);

    // Step indicators (4 steps: Welcome → Select Disk → Method → Install)
    let step_names = ["Welcome", "Select Disk", "Method", "Install"];
    let mut step_labels: [ui::Label; 4] = core::array::from_fn(|_| ui::Label::new(""));
    let mut step_dots: [ui::View; 4] = core::array::from_fn(|_| ui::View::new());

    for (i, &name) in step_names.iter().enumerate() {
        let y = 95 + i as i32 * 38;

        let dot = ui::View::new();
        dot.set_position(20, y + 2);
        dot.set_size(10, 10);
        dot.set_color(ACCENT_DIM);
        sidebar.add(&dot);

        let lbl = ui::Label::new(name);
        lbl.set_position(38, y);
        lbl.set_size(120, 16);
        lbl.set_font_size(13);
        lbl.set_color(ui::theme::darken(tc.window_bg, 15));
        lbl.set_text_color(0xFF888888);
        sidebar.add(&lbl);

        step_dots[i] = dot;
        step_labels[i] = lbl;
    }

    // ── Bottom bar ──
    let bottom = ui::View::new();
    bottom.set_dock(DOCK_BOTTOM);
    bottom.set_size(WIN_W, 50);
    bottom.set_color(ui::theme::darken(tc.window_bg, 8));
    win.add(&bottom);

    let bottom_div = ui::Divider::new();
    bottom_div.set_dock(DOCK_TOP);
    bottom_div.set_size(WIN_W, 1);
    bottom.add(&bottom_div);

    let btn_back = ui::Button::new("Back");
    btn_back.set_position(12, 10);
    btn_back.set_size(80, 30);
    btn_back.set_visible(false);
    let btn_back_id = Widget::id(&btn_back);
    bottom.add(&btn_back);

    let btn_next = ui::Button::new("Continue");
    btn_next.set_position(WIN_W as i32 - 170 - 100, 10);
    btn_next.set_size(90, 30);
    bottom.add(&btn_next);

    let btn_refresh = ui::IconButton::new("");
    btn_refresh.set_position(WIN_W as i32 - 170 - 140, 10);
    btn_refresh.set_size(30, 30);
    btn_refresh.set_system_icon("refresh", ui::IconType::Outline, 0xFFCCCCCC, 18);
    btn_refresh.set_tooltip("Refresh disk list");
    bottom.add(&btn_refresh);

    // ═══════════════════════════════════════════════════════════════
    // Page 0: Welcome
    // ═══════════════════════════════════════════════════════════════

    let page0 = ui::View::new();
    page0.set_dock(DOCK_FILL);
    page0.set_color(tc.window_bg);
    win.add(&page0);

    // Large anyOS branding
    let welcome_title = ui::Label::new("Welcome to anyOS");
    welcome_title.set_position(40, 60);
    welcome_title.set_size(500, 44);
    welcome_title.set_font_size(28);
    welcome_title.set_color(tc.window_bg);
    welcome_title.set_text_color(0xFFFFFFFF);
    page0.add(&welcome_title);

    let welcome_sub = ui::Label::new(
        "This assistant will guide you through the installation of anyOS \
         on your computer. It only takes a few minutes."
    );
    welcome_sub.set_position(40, 116);
    welcome_sub.set_size(500, 44);
    welcome_sub.set_font_size(13);
    welcome_sub.set_color(tc.window_bg);
    welcome_sub.set_text_color(0xFFAAAAAA);
    page0.add(&welcome_sub);

    // Version info
    let ver_label = ui::Label::new("anyOS version 0.4");
    ver_label.set_position(40, 380);
    ver_label.set_size(300, 18);
    ver_label.set_font_size(11);
    ver_label.set_color(tc.window_bg);
    ver_label.set_text_color(0xFF666666);
    page0.add(&ver_label);

    // ═══════════════════════════════════════════════════════════════
    // Page 1: Select Disk
    // ═══════════════════════════════════════════════════════════════

    let page1 = ui::View::new();
    page1.set_dock(DOCK_FILL);
    page1.set_color(tc.window_bg);
    page1.set_visible(false);
    win.add(&page1);

    let p1_title = ui::Label::new("Select a Destination");
    p1_title.set_position(24, 16);
    p1_title.set_size(400, 28);
    p1_title.set_font_size(18);
    p1_title.set_color(tc.window_bg);
    p1_title.set_text_color(0xFFFFFFFF);
    page1.add(&p1_title);

    let p1_desc = ui::Label::new("Choose the disk where anyOS will be installed.");
    p1_desc.set_position(24, 46);
    p1_desc.set_size(420, 18);
    p1_desc.set_font_size(12);
    p1_desc.set_color(tc.window_bg);
    p1_desc.set_text_color(0xFF999999);
    page1.add(&p1_desc);

    // Disk card
    let disk_card = ui::Card::new();
    disk_card.set_position(20, 76);
    disk_card.set_size(WIN_W - 170 - 44, 260);
    page1.add(&disk_card);

    let disk_grid = ui::DataGrid::new(WIN_W - 170 - 54, 220);
    disk_grid.set_position(5, 5);
    disk_grid.set_row_height(28);
    disk_grid.set_header_height(30);
    disk_grid.set_columns(&[
        ColumnDef::new("Disk").width(70),
        ColumnDef::new("Size").width(100).align(ALIGN_RIGHT),
        ColumnDef::new("Partitions").width(80).align(ALIGN_RIGHT),
        ColumnDef::new("ID").width(50).align(ALIGN_RIGHT),
    ]);
    disk_card.add(&disk_grid);

    let p1_warn = ui::Label::new("All data on the selected disk may be erased during installation.");
    p1_warn.set_position(24, 346);
    p1_warn.set_size(420, 16);
    p1_warn.set_font_size(11);
    p1_warn.set_color(tc.window_bg);
    p1_warn.set_text_color(0xFFFF6B6B);
    page1.add(&p1_warn);

    // ═══════════════════════════════════════════════════════════════
    // Page 2: Partition Method
    // ═══════════════════════════════════════════════════════════════

    let page2 = ui::View::new();
    page2.set_dock(DOCK_FILL);
    page2.set_color(tc.window_bg);
    page2.set_visible(false);
    win.add(&page2);

    let p2_title = ui::Label::new("Installation Type");
    p2_title.set_position(24, 16);
    p2_title.set_size(400, 28);
    p2_title.set_font_size(18);
    p2_title.set_color(tc.window_bg);
    p2_title.set_text_color(0xFFFFFFFF);
    page2.add(&p2_title);

    let method_info = ui::Label::new("");
    method_info.set_position(24, 48);
    method_info.set_size(420, 18);
    method_info.set_font_size(12);
    method_info.set_color(tc.window_bg);
    method_info.set_text_color(0xFF999999);
    page2.add(&method_info);

    let card_w = WIN_W - 170 - 44;

    // Option 1: Erase & Install
    let card_auto = ui::Card::new();
    card_auto.set_position(20, 80);
    card_auto.set_size(card_w, 114);
    page2.add(&card_auto);

    let auto_icon = ui::IconButton::new("");
    auto_icon.set_position(14, 14);
    auto_icon.set_size(36, 36);
    auto_icon.set_system_icon("device-floppy", ui::IconType::Outline, ACCENT, 24);
    auto_icon.set_enabled(false);
    card_auto.add(&auto_icon);

    let auto_title = ui::Label::new("Erase disk and install");
    auto_title.set_position(60, 10);
    auto_title.set_size(card_w - 70, 20);
    auto_title.set_font_size(14);
    auto_title.set_color(tc.window_bg);
    auto_title.set_text_color(0xFFFFFFFF);
    card_auto.add(&auto_title);

    let auto_desc = ui::Label::new("Recommended. Erases the entire disk,\ncreates a single partition and installs anyOS.");
    auto_desc.set_position(60, 32);
    auto_desc.set_size(card_w - 70, 40);
    auto_desc.set_font_size(11);
    auto_desc.set_color(tc.window_bg);
    auto_desc.set_text_color(0xFF999999);
    card_auto.add(&auto_desc);

    let btn_auto = ui::Button::new("Erase && Install");
    btn_auto.set_position(60, 76);
    btn_auto.set_size(130, 28);
    card_auto.add(&btn_auto);

    // Option 2: Manual
    let card_manual = ui::Card::new();
    card_manual.set_position(20, 208);
    card_manual.set_size(card_w, 114);
    page2.add(&card_manual);

    let manual_icon = ui::IconButton::new("");
    manual_icon.set_position(14, 14);
    manual_icon.set_size(36, 36);
    manual_icon.set_system_icon("settings", ui::IconType::Outline, 0xFFCCCCCC, 24);
    manual_icon.set_enabled(false);
    card_manual.add(&manual_icon);

    let manual_title = ui::Label::new("Custom partitioning");
    manual_title.set_position(60, 10);
    manual_title.set_size(card_w - 70, 20);
    manual_title.set_font_size(14);
    manual_title.set_color(tc.window_bg);
    manual_title.set_text_color(0xFFFFFFFF);
    card_manual.add(&manual_title);

    let manual_desc = ui::Label::new("Open Disk Utility to create and\nmanage partitions yourself.");
    manual_desc.set_position(60, 32);
    manual_desc.set_size(card_w - 70, 40);
    manual_desc.set_font_size(11);
    manual_desc.set_color(tc.window_bg);
    manual_desc.set_text_color(0xFF999999);
    card_manual.add(&manual_desc);

    let btn_manual = ui::Button::new("Disk Utility...");
    btn_manual.set_position(60, 76);
    btn_manual.set_size(130, 28);
    card_manual.add(&btn_manual);

    // ═══════════════════════════════════════════════════════════════
    // Page 3: Installation Progress
    // ═══════════════════════════════════════════════════════════════

    let page3 = ui::View::new();
    page3.set_dock(DOCK_FILL);
    page3.set_color(tc.window_bg);
    page3.set_visible(false);
    win.add(&page3);

    let p3_title = ui::Label::new("Installing anyOS...");
    p3_title.set_position(24, 16);
    p3_title.set_size(400, 28);
    p3_title.set_font_size(18);
    p3_title.set_color(tc.window_bg);
    p3_title.set_text_color(0xFFFFFFFF);
    page3.add(&p3_title);

    let phase_label = ui::Label::new("Preparing...");
    phase_label.set_position(24, 120);
    phase_label.set_size(400, 22);
    phase_label.set_font_size(14);
    phase_label.set_color(tc.window_bg);
    phase_label.set_text_color(0xFFFFFFFF);
    page3.add(&phase_label);

    let progress_card = ui::Card::new();
    progress_card.set_position(20, 150);
    progress_card.set_size(WIN_W - 170 - 44, 50);
    page3.add(&progress_card);

    let progress_bar = ui::ProgressBar::new(0);
    progress_bar.set_position(10, 15);
    progress_bar.set_size(WIN_W - 170 - 64, 20);
    progress_card.add(&progress_bar);

    let status_label = ui::Label::new("");
    status_label.set_position(24, 215);
    status_label.set_size(400, 18);
    status_label.set_font_size(12);
    status_label.set_color(tc.window_bg);
    status_label.set_text_color(0xFF888888);
    page3.add(&status_label);

    let btn_details = ui::Button::new("Show Details");
    btn_details.set_position(24, 242);
    btn_details.set_size(110, 26);
    page3.add(&btn_details);

    let content_w = WIN_W - 170 - 44;
    let details_card = ui::Card::new();
    details_card.set_position(20, 274);
    details_card.set_size(content_w, 140);
    details_card.set_visible(false);
    page3.add(&details_card);

    let details_log = ui::TextArea::new();
    details_log.set_position(4, 4);
    details_log.set_size(content_w - 8, 132);
    details_log.set_font_size(11);
    details_card.add(&details_log);

    let btn_reboot = ui::Button::new("Restart");
    btn_reboot.set_position((WIN_W - 170) as i32 / 2 - 60, 420);
    btn_reboot.set_size(120, 34);
    btn_reboot.set_visible(false);
    page3.add(&btn_reboot);

    // ── Marshal IDs for worker thread ──
    unsafe {
        PROGRESS_BAR_ID = Widget::id(&progress_bar);
        STATUS_LABEL_ID = Widget::id(&status_label);
        PHASE_LABEL_ID = Widget::id(&phase_label);
        BTN_REBOOT_ID = Widget::id(&btn_reboot);
    }

    // ── Initial data ──
    let disks = detect_disks();
    populate_disk_grid(&disk_grid, &disks);

    unsafe {
        APP = Some(InstallerApp {
            win,
            sidebar,
            step_labels,
            step_dots,
            page0, page1, page2, page3,
            disk_grid,
            method_info,
            progress_bar,
            phase_label,
            status_label,
            btn_reboot,
            btn_details,
            details_card,
            details_log,
            details_text: String::new(),
            disks,
            selected_disk: None,
            current_step: 0,
            timer_id: 0,
            details_visible: false,
            last_copy_seq: 0,
        });
    }

    update_sidebar(0);

    // ═══════════════════════════════════════════════════════════════
    // Event Handlers
    // ═══════════════════════════════════════════════════════════════

    app().disk_grid.on_selection_changed(|e| {
        app().selected_disk = Some(e.index as usize);
    });

    btn_refresh.on_click(|_| {
        let disks = detect_disks();
        populate_disk_grid(&app().disk_grid, &disks);
        app().disks = disks;
        app().selected_disk = None;
    });

    btn_next.on_click(move |_| {
        let a = app();
        match a.current_step {
            0 => {
                // Welcome → Select Disk
                show_page(1);
            }
            1 => {
                // Validate disk selection
                let idx = match a.selected_disk {
                    Some(i) if i < a.disks.len() => i,
                    _ => {
                        ui::MessageBox::show(ui::MessageBoxType::Warning,
                            "Please select a target disk.", None);
                        return;
                    }
                };
                if a.disks[idx].size_sectors < 1024 * 64 {
                    ui::MessageBox::show(ui::MessageBoxType::Warning,
                        "Disk is too small (minimum 32 MB).", None);
                    return;
                }
                let info = format!(
                    "Disk {} -- {} -- {} partition(s)",
                    a.disks[idx].disk_id,
                    format_size(a.disks[idx].size_sectors),
                    a.disks[idx].partition_count
                );
                a.method_info.set_text(&info);
                show_page(2);
                unsafe { ui::marshal_set_visible(btn_back_id, true); }
            }
            _ => {}
        }
    });

    btn_back.on_click(move |_| {
        let a = app();
        match a.current_step {
            1 => {
                show_page(0);
                unsafe { ui::marshal_set_visible(btn_back_id, false); }
            }
            2 => {
                show_page(1);
            }
            _ => {}
        }
    });

    btn_auto.on_click(|_| {
        let a = app();
        let idx = a.selected_disk.unwrap();
        let disk = &a.disks[idx];
        let msg = format!(
            "ALL data on Disk {} ({}) will be permanently erased.",
            disk.disk_id, format_size(disk.size_sectors)
        );
        ui::MessageBox::show(ui::MessageBoxType::Warning, &msg, Some("Erase && Install"));
        INSTALL_DISK_ID.store(disk.device_id as u32, Ordering::Release);
        INSTALL_MODE.store(0, Ordering::Release);
        start_install();
    });

    btn_manual.on_click(|_| {
        process::spawn("/Applications/Disk Utility.app", "");
        ui::MessageBox::show(ui::MessageBoxType::Info,
            "Partition the disk using Disk Utility.\nClick OK when done to continue.",
            Some("OK -- Continue"));
        let a = app();
        let idx = a.selected_disk.unwrap();
        INSTALL_DISK_ID.store(a.disks[idx].device_id as u32, Ordering::Release);
        INSTALL_MODE.store(1, Ordering::Release);
        start_install();
    });

    app().btn_details.on_click(|_| {
        let a = app();
        a.details_visible = !a.details_visible;
        a.details_card.set_visible(a.details_visible);
        a.btn_details.set_text(if a.details_visible { "Hide Details" } else { "Show Details" });
    });

    app().btn_reboot.on_click(|_| { process::reboot(); });
    app().win.on_close(|_| { ui::quit(); });

    ui::run();
}

// ── Install flow ────────────────────────────────────────────────────────────

fn start_install() {
    show_page(3);

    WORKER_ACTIVE.store(true, Ordering::Release);
    WORKER_DONE.store(false, Ordering::Release);
    WORKER_ERROR.store(false, Ordering::Release);
    WORKER_PROGRESS.store(0, Ordering::Release);

    if let Ok(h) = process::Thread::spawn_with_stack(install_worker, 256 * 1024, "installer") {
        core::mem::forget(h);
    }
    app().timer_id = ui::set_timer(200, || { poll_worker(); });
}

fn signal_copy_file(path: &str) {
    let len = path.len().min(255);
    unsafe {
        COPY_FILE_BUF[..len].copy_from_slice(&path.as_bytes()[..len]);
    }
    COPY_FILE_LEN.store(len as u32, Ordering::Release);
    COPY_FILE_SEQ.fetch_add(1, Ordering::Release);
}

fn set_phase(_phase: u32, text: &str) {
    unsafe { ui::marshal_set_text(PHASE_LABEL_ID, text); }
}
fn set_status(text: &str) {
    unsafe { ui::marshal_set_text(STATUS_LABEL_ID, text); }
}
fn set_progress(pct: u32) {
    WORKER_PROGRESS.store(pct, Ordering::Release);
    unsafe { ui::marshal_set_state(PROGRESS_BAR_ID, pct); }
}
fn set_copy_label(label: &str) {
    let len = label.len().min(31);
    unsafe { COPY_CURRENT_LABEL[..len].copy_from_slice(&label.as_bytes()[..len]); }
    COPY_CURRENT_LABEL_LEN.store(len as u32, Ordering::Release);
    COPY_FILES_SINCE_UPDATE.store(0, Ordering::Relaxed);
    update_copy_status();
}
fn update_copy_status() {
    let start_ms = COPY_START_MS.load(Ordering::Relaxed);
    let elapsed_ms = sys::uptime_ms() - start_ms;
    let secs = elapsed_ms / 1000;
    let bytes = COPY_BYTES_TOTAL.load(Ordering::Relaxed);
    let kbs = if elapsed_ms > 0 { (bytes as u64 * 1000) / (elapsed_ms as u64 * 1024) } else { 0 };
    let label_len = COPY_CURRENT_LABEL_LEN.load(Ordering::Acquire) as usize;
    let label = unsafe { core::str::from_utf8(&COPY_CURRENT_LABEL[..label_len]).unwrap_or("") };
    let total_files = COPY_FILES_SINCE_UPDATE.load(Ordering::Relaxed);
    let _ = total_files; // used implicitly via COPY_BYTES_TOTAL
    set_status(&format!(
        "Copying {}... ({}:{:02}, {}.{} MB/s)",
        label, secs / 60, secs % 60, kbs / 1024, (kbs % 1024) * 10 / 1024
    ));
}

fn install_worker() {
    let dev_id = INSTALL_DISK_ID.load(Ordering::Acquire);
    let mode = INSTALL_MODE.load(Ordering::Acquire);

    let mut buf = [0u8; 32 * 32];
    let count = sys::disk_list(&mut buf);
    let mut disk_id: u8 = 0;
    let mut total_sectors: u64 = 0;
    for i in 0..count as usize {
        let off = i * 32;
        if buf[off] == dev_id as u8 {
            disk_id = buf[off + 1];
            total_sectors = u64::from_le_bytes([
                buf[off+12], buf[off+13], buf[off+14], buf[off+15],
                buf[off+16], buf[off+17], buf[off+18], buf[off+19],
            ]);
            break;
        }
    }

    if total_sectors == 0 {
        set_phase(99, "Error: Disk not found");
        WORKER_ERROR.store(true, Ordering::Release);
        WORKER_DONE.store(true, Ordering::Release);
        return;
    }

    // dev_id = block device ID (for sys::disk_write — raw I/O on the whole disk)
    // disk_id = physical disk number (for partition_create, partition_rescan, disk_list lookup)

    if mode == 0 {
        let fs_sectors = (total_sectors - PARTITION_START as u64) as u32;

        // Install bootloader FIRST (writes stage1 to MBR bytes 0-439,
        // preserving the partition table area at 446-509).
        set_phase(1, "Installing bootloader...");
        set_progress(5);
        if !install_bootloader(dev_id) {
            set_phase(99, "Error: Bootloader not found");
            WORKER_ERROR.store(true, Ordering::Release);
            WORKER_DONE.store(true, Ordering::Release);
            return;
        }

        // Create partition table AFTER bootloader (writes PT into MBR 446-509,
        // the bootloader code at 0-439 is already in place).
        set_phase(2, "Creating partition table...");
        set_progress(10);
        create_partition(dev_id, disk_id as u32, total_sectors);

        // Format the partition.
        set_phase(3, "Formatting filesystem...");
        set_progress(15);
        format_exfat(dev_id, PARTITION_START, fs_sectors);
    } else {
        set_phase(2, "Installing bootloader...");
        set_progress(10);
        if !install_bootloader(dev_id) {
            set_phase(99, "Error: Bootloader not found");
            WORKER_ERROR.store(true, Ordering::Release);
            WORKER_DONE.store(true, Ordering::Release);
            return;
        }
    }

    set_phase(4, "Mounting target filesystem...");
    set_progress(20);
    sys::partition_rescan(disk_id as u32);
    process::sleep(500);

    // Find the new partition on this disk
    let mut part_dev_id: Option<u8> = None;
    let count2 = sys::disk_list(&mut buf);
    for i in 0..count2 as usize {
        let off = i * 32;
        if buf[off + 1] == disk_id && buf[off + 2] != 0xFF {
            part_dev_id = Some(buf[off]);
            break;
        }
    }

    let part_id = match part_dev_id {
        Some(id) => id,
        None => {
            set_phase(99, "Error: No partition found after format");
            set_status("Partition table may not have been written correctly.");
            WORKER_ERROR.store(true, Ordering::Release);
            WORKER_DONE.store(true, Ordering::Release);
            return;
        }
    };

    let dev_str = format!("{}", part_id);
    if fs::mount("/mnt/target", &dev_str, FS_TYPE_EXFAT) != 0 {
        set_phase(99, "Error: Could not mount target");
        set_status("Is the partition formatted as exFAT?");
        WORKER_ERROR.store(true, Ordering::Release);
        WORKER_DONE.store(true, Ordering::Release);
        return;
    }

    set_phase(5, "Copying system files...");
    set_progress(25);

    let copy_start_ms = sys::uptime_ms();
    COPY_START_MS.store(copy_start_ms, Ordering::Relaxed);
    COPY_BYTES_TOTAL.store(0, Ordering::Relaxed);
    COPY_FILES_SINCE_UPDATE.store(0, Ordering::Relaxed);

    let dirs: &[(&str, &str, &str)] = &[
        ("/System",       "/mnt/target/System",       "System"),
        ("/Applications", "/mnt/target/Applications", "Applications"),
        ("/Users",        "/mnt/target/Users",        "Users"),
        ("/boot",         "/mnt/target/boot",         "boot"),
        ("/media",        "/mnt/target/media",        "media"),
    ];

    let n = dirs.len() as u32;
    let mut total_files = 0u32;
    for (i, &(src, dst, label)) in dirs.iter().enumerate() {
        set_progress(25 + (i as u32 * 70) / n);
        set_copy_label(label);
        fs::mkdir(dst);
        total_files += copy_recursive(src, dst, 0);
    }

    let mut stat_buf = [0u32; 7];
    if fs::stat("/Libraries", &mut stat_buf) == 0 {
        set_copy_label("Libraries");
        fs::mkdir("/mnt/target/Libraries");
        total_files += copy_recursive("/Libraries", "/mnt/target/Libraries", 0);
    }

    let copy_ms = sys::uptime_ms() - copy_start_ms;
    let copy_secs = copy_ms / 1000;

    set_phase(6, "Finishing...");
    set_progress(98);
    fs::umount("/mnt/target");

    let total_bytes = COPY_BYTES_TOTAL.load(Ordering::Relaxed);
    let total_mb = total_bytes / (1024 * 1024);
    let avg_kbs = if copy_ms > 0 { (total_bytes as u64 * 1000) / (copy_ms as u64 * 1024) } else { 0 };

    set_phase(7, "Installation complete!");
    set_status(&format!(
        "{} files ({} MB) copied in {}:{:02} ({}.{} MB/s)",
        total_files, total_mb,
        copy_secs / 60, copy_secs % 60,
        avg_kbs / 1024, (avg_kbs % 1024) * 10 / 1024
    ));
    set_progress(100);

    unsafe { ui::marshal_set_visible(BTN_REBOOT_ID, true); }
    WORKER_DONE.store(true, Ordering::Release);
    WORKER_ACTIVE.store(false, Ordering::Release);
}

fn poll_worker() {
    let a = app();

    // Check for new file copy notifications from worker thread
    let seq = COPY_FILE_SEQ.load(Ordering::Acquire);
    if seq != a.last_copy_seq {
        a.last_copy_seq = seq;
        let len = COPY_FILE_LEN.load(Ordering::Acquire) as usize;
        if len > 0 {
            let path = unsafe {
                core::str::from_utf8(&COPY_FILE_BUF[..len]).unwrap_or("")
            };
            // Strip "/mnt/target" prefix for cleaner display
            let display = path.strip_prefix("/mnt/target").unwrap_or(path);
            a.details_text.push_str(display);
            a.details_text.push('\n');
            a.details_log.set_text(&a.details_text);
        }
    }

    if WORKER_DONE.load(Ordering::Acquire) {
        ui::kill_timer(a.timer_id);
        if WORKER_ERROR.load(Ordering::Acquire) {
            a.btn_reboot.set_text("Close");
            a.btn_reboot.set_visible(true);
        }
    }
}

// ── ExFAT formatting ────────────────────────────────────────────────────────

fn write_le16(buf: &mut [u8], off: usize, val: u16) {
    buf[off] = val as u8; buf[off+1] = (val >> 8) as u8;
}
fn write_le32(buf: &mut [u8], off: usize, val: u32) {
    buf[off] = val as u8; buf[off+1] = (val >> 8) as u8;
    buf[off+2] = (val >> 16) as u8; buf[off+3] = (val >> 24) as u8;
}
fn write_le64(buf: &mut [u8], off: usize, val: u64) {
    for i in 0..8 { buf[off + i] = (val >> (i * 8)) as u8; }
}

fn disk_write_sector(dev: u32, lba: u32, data: &[u8; 512]) {
    sys::disk_write(dev, lba as u64, 1, data);
}

fn write_cluster(dev: u32, fs_start: u32, heap_off: u32, cluster: u32, data: &[u8]) {
    let lba = fs_start + heap_off + (cluster - 2) * SPC;
    let sectors = (data.len() as u32 + SECTOR_SIZE - 1) / SECTOR_SIZE;
    for s in 0..sectors.min(SPC) {
        let off = (s * SECTOR_SIZE) as usize;
        let mut sector = [0u8; 512];
        let end = (off + 512).min(data.len());
        sector[..end - off].copy_from_slice(&data[off..end]);
        sys::disk_write(dev, (lba + s) as u64, 1, &sector);
    }
}

fn format_exfat(dev_id: u32, fs_start: u32, fs_sectors: u32) {
    let est_clusters = (fs_sectors - FAT_OFFSET) / SPC;
    let fat_length = ((est_clusters + 2) * 4 + SECTOR_SIZE - 1) / SECTOR_SIZE;
    let cluster_heap_offset = FAT_OFFSET + fat_length;
    let cluster_count = (fs_sectors - cluster_heap_offset) / SPC;
    let fat_length = ((cluster_count + 2) * 4 + SECTOR_SIZE - 1) / SECTOR_SIZE;
    let cluster_heap_offset = FAT_OFFSET + fat_length;
    let root_cluster: u32 = 4;
    let dev = dev_id;

    // VBR
    let mut vbr = [0u8; 512];
    vbr[0] = 0xEB; vbr[1] = 0x76; vbr[2] = 0x90;
    vbr[3..11].copy_from_slice(b"EXFAT   ");
    write_le64(&mut vbr, 64, fs_start as u64);
    write_le64(&mut vbr, 72, fs_sectors as u64);
    write_le32(&mut vbr, 80, FAT_OFFSET);
    write_le32(&mut vbr, 84, fat_length);
    write_le32(&mut vbr, 88, cluster_heap_offset);
    write_le32(&mut vbr, 92, cluster_count);
    write_le32(&mut vbr, 96, root_cluster);
    write_le32(&mut vbr, 100, 0x414E594F);
    write_le16(&mut vbr, 104, 0x0100);
    vbr[108] = 9; vbr[109] = SPC_SHIFT; vbr[110] = 1; vbr[111] = 0x80; vbr[112] = 0xFF;
    vbr[510] = 0x55; vbr[511] = 0xAA;

    let mut ext = [0u8; 512]; ext[510] = 0x55; ext[511] = 0xAA;
    let oem = [0u8; 512]; let reserved = [0u8; 512];

    let mut checksum: u32 = 0;
    let regions: [&[u8; 512]; 11] = [&vbr, &ext, &ext, &ext, &ext, &ext, &ext, &ext, &ext, &oem, &reserved];
    for (si, sector) in regions.iter().enumerate() {
        for (bi, &b) in sector.iter().enumerate() {
            let abs = si * 512 + bi;
            if abs == 106 || abs == 107 || abs == 112 { continue; }
            checksum = checksum.rotate_right(1).wrapping_add(b as u32);
        }
    }
    let mut cs_sector = [0u8; 512];
    for i in 0..128 { write_le32(&mut cs_sector, i * 4, checksum); }

    for base in [0u32, 12] {
        disk_write_sector(dev, fs_start + base, &vbr);
        for i in 0..8u32 { disk_write_sector(dev, fs_start + base + 1 + i, &ext); }
        disk_write_sector(dev, fs_start + base + 9, &oem);
        disk_write_sector(dev, fs_start + base + 10, &reserved);
        disk_write_sector(dev, fs_start + base + 11, &cs_sector);
    }

    // FAT
    let fat_abs = fs_start + FAT_OFFSET;
    {
        let mut s = [0u8; 512];
        write_le32(&mut s, 0, 0xFFFFFFF8);
        write_le32(&mut s, 4, 0xFFFFFFFF);
        write_le32(&mut s, 8, 0xFFFFFFFF);
        write_le32(&mut s, 12, 0xFFFFFFFF);
        write_le32(&mut s, 16, 0xFFFFFFFF);
        disk_write_sector(dev, fat_abs, &s);
    }
    // Clear first 32 sectors after entries (bitmap is authoritative for free state)
    {
        let clear_end = (1u32 + 32).min(fat_length);
        let zero_batch = [0u8; 32 * 512];
        if clear_end > 1 {
            let n = clear_end - 1;
            sys::disk_write(dev, (fat_abs + 1) as u64, n, &zero_batch[..(n as usize * 512)]);
        }
    }

    // Upcase table (cluster 3)
    let csz = CLUSTER_SIZE as usize;
    let mut upcase = anyos_std::vec![0u8; csz];
    for i in 0u16..128 {
        let u = if i >= 0x61 && i <= 0x7A { i - 0x20 } else { i };
        write_le16(&mut upcase, i as usize * 2, u);
    }
    let upcase_len: u32 = 256;
    let mut uc: u32 = 0;
    for i in 0..upcase_len as usize { uc = uc.rotate_right(1).wrapping_add(upcase[i] as u32); }
    write_cluster(dev, fs_start, cluster_heap_offset, 3, &upcase);

    // Root dir (cluster 4)
    let mut root = anyos_std::vec![0u8; csz];
    let bitmap_size = (cluster_count + 7) / 8;
    root[0] = 0x81;
    write_le32(&mut root, 20, 2);
    write_le64(&mut root, 24, bitmap_size as u64);
    root[32] = 0x82;
    write_le32(&mut root, 36, uc);
    write_le32(&mut root, 52, 3);
    write_le64(&mut root, 56, upcase_len as u64);
    root[64] = 0x83; root[65] = 5;
    for (i, &ch) in b"anyOS".iter().enumerate() { write_le16(&mut root, 66 + i * 2, ch as u16); }
    write_cluster(dev, fs_start, cluster_heap_offset, 4, &root);

    // Bitmap (cluster 2)
    let mut bitmap = anyos_std::vec![0u8; csz];
    bitmap[0] = 0x07;
    write_cluster(dev, fs_start, cluster_heap_offset, 2, &bitmap);

    // Zero padding
    let zero = [0u8; 512];
    for s in 24..FAT_OFFSET { disk_write_sector(dev, fs_start + s, &zero); }
}

// ── Bootloader ──────────────────────────────────────────────────────────────

fn install_bootloader(dev_id: u32) -> bool {
    let dev = dev_id;
    let mut stage1 = [0u8; 512];
    let s1 = fs::open("/boot/stage1.bin", 0);
    if s1 == u32::MAX { return false; }
    fs::read(s1, &mut stage1); fs::close(s1);

    let s2 = fs::open("/boot/stage2.bin", 0);
    if s2 == u32::MAX { return false; }
    let mut stage2 = anyos_std::vec![0u8; 63 * 512];
    let mut total = 0usize;
    loop {
        let n = fs::read(s2, &mut stage2[total..]);
        if n == 0 || n == u32::MAX { break; }
        total += n as usize;
    }
    fs::close(s2);
    if total == 0 { return false; }

    let mut mbr = [0u8; 512];
    sys::disk_read(dev, 0, 1, &mut mbr);
    mbr[..440].copy_from_slice(&stage1[..440]);
    mbr[510] = 0x55; mbr[511] = 0xAA;
    sys::disk_write(dev, 0, 1, &mbr);

    for s in 0..(total + 511) / 512 {
        let off = s * 512;
        let mut sector = [0u8; 512];
        let end = (off + 512).min(total);
        sector[..end - off].copy_from_slice(&stage2[off..end]);
        sys::disk_write(dev, (1 + s) as u64, 1, &sector);
    }
    true
}

/// Write MBR partition table directly via disk_write (same I/O path as bootloader).
/// This avoids any mismatch between the kernel's direct storage path and the
/// BlockDevice path that disk_write uses.
fn create_partition(dev_id: u32, disk_id: u32, total_sectors: u64) {
    let part_size = total_sectors - PARTITION_START as u64;

    // Read current MBR (which already has stage1 bootloader code + 0x55AA)
    // dev_id = block device ID for raw I/O, disk_id = physical disk number for partition_rescan
    let mut mbr = [0u8; 512];
    sys::disk_read(dev_id, 0, 1, &mut mbr);

    // Ensure MBR signature exists
    mbr[510] = 0x55;
    mbr[511] = 0xAA;

    // Write partition entry 0 at offset 446
    let off = 446;
    mbr[off] = 0x80;     // bootable
    mbr[off+1] = 0xFE;   // CHS start (LBA mode)
    mbr[off+2] = 0xFF;
    mbr[off+3] = 0xFF;
    mbr[off+4] = 0x07;   // type: exFAT/NTFS
    mbr[off+5] = 0xFE;   // CHS end (LBA mode)
    mbr[off+6] = 0xFF;
    mbr[off+7] = 0xFF;
    mbr[off+8..off+12].copy_from_slice(&(PARTITION_START).to_le_bytes());
    mbr[off+12..off+16].copy_from_slice(&(part_size as u32).to_le_bytes());

    // Clear partition entries 1-3
    for i in 1..4 {
        let o = 446 + i * 16;
        for b in &mut mbr[o..o+16] { *b = 0; }
    }

    // Write MBR back via same path as bootloader
    sys::disk_write(dev_id, 0, 1, &mbr);
}

// ── File copy ───────────────────────────────────────────────────────────────

const CASE_MAP: &[(&str, &str)] = &[
    ("system", "System"), ("applications", "Applications"),
    ("users", "Users"), ("libraries", "Libraries"),
    ("info.conf", "Info.conf"), ("icon.ico", "Icon.ico"),
];

fn fix_case(name: &str) -> String {
    for &(lower, proper) in CASE_MAP {
        if name == lower { return String::from(proper); }
    }
    if name.ends_with(".app") { return capitalize_words(name); }
    if name.ends_with(".dlib") { return capitalize_first(name); }
    String::from(name)
}

fn capitalize_first(s: &str) -> String {
    let mut r = String::with_capacity(s.len());
    let mut first = true;
    for ch in s.chars() {
        if first && ch.is_ascii_lowercase() { r.push((ch as u8 - 32) as char); }
        else { r.push(ch); }
        first = false;
    }
    r
}

fn capitalize_words(s: &str) -> String {
    let mut r = String::with_capacity(s.len());
    let mut cap = true;
    for ch in s.chars() {
        if ch == ' ' || ch == '-' || ch == '_' { r.push(ch); cap = true; }
        else if cap && ch.is_ascii_lowercase() { r.push((ch as u8 - 32) as char); cap = false; }
        else { r.push(ch); cap = false; }
    }
    r
}

fn copy_recursive(src: &str, dst: &str, depth: u32) -> u32 {
    if depth > 16 { return 0; }
    let mut buf = [0u8; 256 * 64];
    let count = fs::readdir(src, &mut buf);
    if count == u32::MAX { return 0; }
    let mut copied = 0u32;
    for i in 0..count as usize {
        let off = i * 64;
        let entry_type = buf[off];
        let name_len = buf[off + 1] as usize;
        if name_len == 0 || name_len > 56 { continue; }
        let name = match core::str::from_utf8(&buf[off + 8..off + 8 + name_len]) {
            Ok(s) => s, Err(_) => continue,
        };
        if name == "." || name == ".." { continue; }
        if depth == 0 && (name == "src" || name == "apps") { continue; }
        let fixed = fix_case(name);
        let child_src = format!("{}/{}", src, name);
        let child_dst = format!("{}/{}", dst, fixed);
        if entry_type == 1 {
            fs::mkdir(&child_dst);
            copied += copy_recursive(&child_src, &child_dst, depth + 1);
        } else {
            if copy_file(&child_src, &child_dst) { copied += 1; }
        }
    }
    copied
}

fn copy_file(src: &str, dst: &str) -> bool {
    signal_copy_file(dst);
    let fd = fs::open(src, 0);
    if fd == u32::MAX { return false; }
    let mut data = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = fs::read(fd, &mut buf);
        if n == 0 || n == u32::MAX { break; }
        data.extend_from_slice(&buf[..n as usize]);
    }
    fs::close(fd);
    let len = data.len() as u32;
    let ok = fs::write_bytes(dst, &data).is_ok();
    if ok {
        COPY_BYTES_TOTAL.fetch_add(len, Ordering::Relaxed);
        if COPY_FILES_SINCE_UPDATE.fetch_add(1, Ordering::Relaxed) % 50 == 49 {
            update_copy_status();
        }
    }
    ok
}
