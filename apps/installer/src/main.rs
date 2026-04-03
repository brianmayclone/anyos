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
    // Pages: 0=Welcome, 1=License, 2=Select Disk, 3=Installing/Done
    page0: ui::View,    // Welcome
    page1: ui::View,    // License Agreement
    page2: ui::View,    // Select Disk (cards)
    page3: ui::View,    // Installing → Complete
    // Page 2 (disk selection)
    disk_grid: ui::DataGrid,
    disk_container_id: u32,
    disk_card_ids: Vec<u32>,
    // Page 3 (progress + completion)
    progress_bar: ui::ProgressBar,
    phase_label: ui::Label,
    status_label: ui::Label,
    btn_reboot: ui::Button,
    btn_details: ui::Button,
    details_card: ui::Card,
    details_log: ui::TextArea,
    details_text: String,
    progress_card_id: u32,
    complete_label_id: u32,
    complete_sub_id: u32,
    // Bottom bar buttons
    btn_back_id: u32,
    btn_next_id: u32,
    // State
    disks: Vec<DiskEntry>,
    selected_disk: Option<usize>,
    current_step: u32,
    timer_id: u32,
    details_visible: bool,
    last_copy_seq: u32,
    license_accepted: bool,
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

fn populate_disk_grid(_grid: &ui::DataGrid, _disks: &[DiskEntry]) {
    // Legacy — now using populate_disk_cards() instead
}

/// Build macOS-style disk selection cards. Each disk is a large card with
/// a disk icon, name, size, and partition info. Selected card has blue border.
fn populate_disk_cards(disks: &[DiskEntry]) {
    let a = app();
    let container = ui::Control::from_id(a.disk_container_id);

    // Remove old cards (if refreshing)
    for &card_id in &a.disk_card_ids {
        ui::Control::from_id(card_id).set_visible(false);
    }
    a.disk_card_ids.clear();

    let card_w: u32 = 160;
    let card_h: u32 = 190;
    let gap: i32 = 16;
    let max_per_row = 3;

    // Find best default: largest disk that's >= 1 GB
    let mut best_idx: Option<usize> = None;
    let mut best_size: u64 = 0;
    for (i, d) in disks.iter().enumerate() {
        let bytes = d.size_sectors * 512;
        if bytes >= 1024 * 1024 * 1024 && bytes > best_size {
            best_size = bytes;
            best_idx = Some(i);
        }
    }
    // Fallback: largest disk of any size
    if best_idx.is_none() && !disks.is_empty() {
        best_idx = Some(0);
        for (i, d) in disks.iter().enumerate() {
            if d.size_sectors > disks[best_idx.unwrap()].size_sectors {
                best_idx = Some(i);
            }
        }
    }
    if let Some(idx) = best_idx {
        a.selected_disk = Some(idx);
    }

    // Center cards: compute total width and offset
    let total_cards = disks.len().min(max_per_row);
    let total_w = total_cards as i32 * card_w as i32 + (total_cards as i32 - 1).max(0) * gap;
    let container_w = 560i32;
    let x_offset = (container_w - total_w) / 2;

    for (i, disk) in disks.iter().enumerate() {
        let col = (i % max_per_row) as i32;
        let row = (i / max_per_row) as i32;
        let x = x_offset + col * (card_w as i32 + gap);
        let y = row * (card_h as i32 + gap);

        let is_selected = a.selected_disk == Some(i);
        let card_color = if is_selected { 0xFF1A3A5C } else { 0xFF2A2A2E };

        let card = ui::Card::new();
        card.set_position(x, y);
        card.set_size(card_w, card_h);
        card.set_color(card_color);

        // Disk icon from /System/media/icons/devices/generic.ico
        if let Some(ico) = ui::Icon::load("/System/media/icons/devices/generic.ico", 48) {
            let iv = ico.into_image_view(48, 48);
            iv.set_position((card_w as i32 - 48) / 2, 14);
            card.add(&iv);
        }

        // Disk name — centered
        let name = format!("Disk {}", disk.disk_id);
        let name_label = ui::Label::new(&name);
        name_label.set_position(0, 76);
        name_label.set_size(card_w, 20);
        name_label.set_font_size(14);
        name_label.set_color(0x00000000);
        name_label.set_text_color(0xFFFFFFFF);
        name_label.set_text_align(ui::TEXT_ALIGN_CENTER);
        card.add(&name_label);

        // Size — centered
        let size_str = format_size(disk.size_sectors);
        let size_label = ui::Label::new(&size_str);
        size_label.set_position(0, 98);
        size_label.set_size(card_w, 18);
        size_label.set_font_size(12);
        size_label.set_color(0x00000000);
        size_label.set_text_color(0xFFAAAAAA);
        size_label.set_text_align(ui::TEXT_ALIGN_CENTER);
        card.add(&size_label);

        // Partition count — centered
        let part_str = if disk.partition_count == 0 {
            String::from("Unpartitioned")
        } else {
            format!("{} partition{}", disk.partition_count,
                if disk.partition_count == 1 { "" } else { "s" })
        };
        let part_label = ui::Label::new(&part_str);
        part_label.set_position(0, 118);
        part_label.set_size(card_w, 16);
        part_label.set_font_size(11);
        part_label.set_color(0x00000000);
        part_label.set_text_color(0xFF777777);
        part_label.set_text_align(ui::TEXT_ALIGN_CENTER);
        card.add(&part_label);

        // Selection indicator (blue dot at bottom-center)
        if is_selected {
            let dot = ui::View::new();
            dot.set_position((card_w as i32 - 8) / 2, card_h as i32 - 16);
            dot.set_size(8, 8);
            dot.set_color(ACCENT);
            card.add(&dot);
        }

        // Clickable overlay (Card doesn't support on_click, so use a transparent label)
        let disk_idx = i;
        let click_area = ui::Label::new("");
        click_area.set_position(0, 0);
        click_area.set_size(card_w, card_h);
        click_area.set_color(0x00000000);
        click_area.on_click_raw(disk_card_click, disk_idx as u64);
        card.add(&click_area);

        a.disk_card_ids.push(card.id());
        container.add_child(card.id());
    }

    if disks.is_empty() {
        let empty_label = ui::Label::new("No disks detected. Connect a disk and click Refresh.");
        empty_label.set_position(0, 60);
        empty_label.set_size(450, 20);
        empty_label.set_font_size(13);
        empty_label.set_color(0x00000000);
        empty_label.set_text_color(0xFF888888);
        container.add_child(empty_label.id());
        a.disk_card_ids.push(empty_label.id());
    }
}

// ── Sidebar step management ─────────────────────────────────────────────────

const ACCENT: u32 = 0xFF007AFF;      // Apple blue
const ACCENT_DIM: u32 = 0xFF3A3A5C;  // Inactive step
const STEP_DONE: u32 = 0xFF34C759;   // Green checkmark color

extern "C" fn disk_card_click(_control_id: u32, _event_type: u32, userdata: u64) {
    let idx = userdata as usize;
    app().selected_disk = Some(idx);
    let disks_copy = detect_disks();
    populate_disk_cards(&disks_copy);
    app().disks = disks_copy;
}

fn show_page(step: u32) {
    let a = app();
    a.current_step = step;
    a.page0.set_visible(step == 0);
    a.page1.set_visible(step == 1);
    a.page2.set_visible(step == 2);
    a.page3.set_visible(step == 3);

    // Bottom buttons
    let show_back = step >= 1 && step <= 2;
    let show_next = step <= 2;
    unsafe {
        ui::marshal_set_visible(a.btn_back_id, show_back);
        ui::marshal_set_visible(a.btn_next_id, show_next);
    }
    // Change button text based on page
    let next_ctrl = ui::Control::from_id(a.btn_next_id);
    match step {
        0 => next_ctrl.set_text("Continue"),
        1 => next_ctrl.set_text("Agree"),
        2 => next_ctrl.set_text("Install"),
        _ => {}
    }
}

// ── UI construction ─────────────────────────────────────────────────────────

fn main() {
    if !ui::init() { return; }

    let tc = ui::theme::colors();
    let win = ui::Window::new("Install anyOS", -1, -1, WIN_W, WIN_H);

    // ── Bottom bar (macOS style: "Go Back" left, "Continue" right, blue) ──
    let bottom = ui::View::new();
    bottom.set_dock(DOCK_BOTTOM);
    bottom.set_size(WIN_W, 56);
    bottom.set_color(ui::theme::darken(tc.window_bg, 5));
    win.add(&bottom);

    let bottom_div = ui::Divider::new();
    bottom_div.set_dock(DOCK_TOP);
    bottom_div.set_size(WIN_W, 1);
    bottom.add(&bottom_div);

    let btn_back = ui::Button::new("Go Back");
    btn_back.set_position(20, 13);
    btn_back.set_size(90, 30);
    btn_back.set_visible(false);
    let btn_back_id = Widget::id(&btn_back);
    bottom.add(&btn_back);

    let btn_next = ui::Button::new("Continue");
    btn_next.set_position(WIN_W as i32 - 120, 13);
    btn_next.set_size(100, 30);
    btn_next.set_color(ACCENT);
    btn_next.set_text_color(0xFFFFFFFF);
    let btn_next_id = Widget::id(&btn_next);
    bottom.add(&btn_next);

    // ═══════════════════════════════════════════════════════════════
    // Page 0: Welcome
    // ═══════════════════════════════════════════════════════════════

    let page0 = ui::View::new();
    page0.set_dock(DOCK_FILL);
    page0.set_color(tc.window_bg);
    win.add(&page0);

    // Centered welcome layout (macOS style)
    let cx = (WIN_W / 2) as i32; // center x

    let welcome_title = ui::Label::new("Welcome to anyOS");
    welcome_title.set_position(0, 100);
    welcome_title.set_size(WIN_W, 44);
    welcome_title.set_font_size(30);
    welcome_title.set_color(tc.window_bg);
    welcome_title.set_text_color(0xFFFFFFFF);
    welcome_title.set_text_align(ui::TEXT_ALIGN_CENTER);
    page0.add(&welcome_title);

    let welcome_sub = ui::Label::new(
        "This assistant will guide you through the steps needed to install anyOS on your computer."
    );
    welcome_sub.set_position(0, 160);
    welcome_sub.set_size(WIN_W, 40);
    welcome_sub.set_font_size(14);
    welcome_sub.set_color(tc.window_bg);
    welcome_sub.set_text_color(0xFFAAAAAA);
    welcome_sub.set_text_align(ui::TEXT_ALIGN_CENTER);
    page0.add(&welcome_sub);

    let welcome_hint = ui::Label::new("Click Continue to get started.");
    welcome_hint.set_position(0, 216);
    welcome_hint.set_size(WIN_W, 20);
    welcome_hint.set_font_size(13);
    welcome_hint.set_color(tc.window_bg);
    welcome_hint.set_text_color(0xFF888888);
    welcome_hint.set_text_align(ui::TEXT_ALIGN_CENTER);
    page0.add(&welcome_hint);

    let ver_label = ui::Label::new("anyOS version 0.4");
    ver_label.set_position(0, 380);
    ver_label.set_size(WIN_W, 18);
    ver_label.set_font_size(11);
    ver_label.set_color(tc.window_bg);
    ver_label.set_text_color(0xFF555555);
    ver_label.set_text_align(ui::TEXT_ALIGN_CENTER);
    page0.add(&ver_label);

    // ═══════════════════════════════════════════════════════════════
    // Page 1: Select Disk (macOS Ventura style — large disk cards)
    // ═══════════════════════════════════════════════════════════════

    // ═══════════════════════════════════════════════════════════════
    // Page 1: License Agreement
    // ═══════════════════════════════════════════════════════════════

    let page1 = ui::View::new();
    page1.set_dock(DOCK_FILL);
    page1.set_color(tc.window_bg);
    page1.set_visible(false);
    win.add(&page1);

    let lic_title = ui::Label::new("Software License Agreement");
    lic_title.set_position(0, 30);
    lic_title.set_size(WIN_W, 34);
    lic_title.set_font_size(24);
    lic_title.set_color(tc.window_bg);
    lic_title.set_text_color(0xFFFFFFFF);
    lic_title.set_text_align(ui::TEXT_ALIGN_CENTER);
    page1.add(&lic_title);

    let lic_desc = ui::Label::new("Please read and agree to the terms before continuing.");
    lic_desc.set_position(0, 68);
    lic_desc.set_size(WIN_W, 20);
    lic_desc.set_font_size(13);
    lic_desc.set_color(tc.window_bg);
    lic_desc.set_text_color(0xFF999999);
    lic_desc.set_text_align(ui::TEXT_ALIGN_CENTER);
    page1.add(&lic_desc);

    // License text card
    let lic_card = ui::Card::new();
    lic_card.set_position(cx - 290, 106);
    lic_card.set_size(580, 230);
    page1.add(&lic_card);

    let lic_text = ui::TextArea::new();
    lic_text.set_position(8, 8);
    lic_text.set_size(564, 214);
    lic_text.set_font_size(12);
    lic_text.set_text(concat!(
        "anyOS Software License Agreement\n",
        "MIT License\n\n",
        "Copyright (c) 2024-2026 Christian Moeller, Mike Stratmann\n\n",
        "Permission is hereby granted, free of charge, to any person \n",
        "obtaining a copy of this software and associated documentation \n",
        "files (the \"Software\"), to deal in the Software without \n",
        "restriction, including without limitation the rights to use, \n",
        "copy, modify, merge, publish, distribute, sublicense, and/or \n",
        "sell copies of the Software, and to permit persons to whom the \n",
        "Software is furnished to do so, subject to the following \n",
        "conditions:\n\n",
        "The above copyright notice and this permission notice shall be \n",
        "included in all copies or substantial portions of the Software.\n\n",
        "THE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, \n",
        "EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES \n",
        "OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND \n",
        "NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT \n",
        "HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, \n",
        "WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING \n",
        "FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR \n",
        "OTHER DEALINGS IN THE SOFTWARE.\n\n",
        "By clicking \"Agree\", you accept these terms and conditions.",
    ));
    lic_card.add(&lic_text);

    // "I agree" label at bottom
    let lic_agree_hint = ui::Label::new(
        "Click \"Agree\" to accept and continue with the installation."
    );
    lic_agree_hint.set_position(0, 350);
    lic_agree_hint.set_size(WIN_W, 18);
    lic_agree_hint.set_font_size(12);
    lic_agree_hint.set_color(tc.window_bg);
    lic_agree_hint.set_text_color(0xFF888888);
    lic_agree_hint.set_text_align(ui::TEXT_ALIGN_CENTER);
    page1.add(&lic_agree_hint);

    // (method_info removed — no Install Type page)

    // ═══════════════════════════════════════════════════════════════
    // Page 2: Select Disk (macOS-style disk cards)
    // ═══════════════════════════════════════════════════════════════

    let page2 = ui::View::new();
    page2.set_dock(DOCK_FILL);
    page2.set_color(tc.window_bg);
    page2.set_visible(false);
    win.add(&page2);

    let p2_title = ui::Label::new("Select a Destination");
    p2_title.set_position(0, 30);
    p2_title.set_size(WIN_W, 34);
    p2_title.set_font_size(24);
    p2_title.set_color(tc.window_bg);
    p2_title.set_text_color(0xFFFFFFFF);
    p2_title.set_text_align(ui::TEXT_ALIGN_CENTER);
    page2.add(&p2_title);

    let p2_desc = ui::Label::new("Select the disk where you would like to install anyOS.");
    p2_desc.set_position(0, 68);
    p2_desc.set_size(WIN_W, 20);
    p2_desc.set_font_size(13);
    p2_desc.set_color(tc.window_bg);
    p2_desc.set_text_color(0xFF999999);
    p2_desc.set_text_align(ui::TEXT_ALIGN_CENTER);
    page2.add(&p2_desc);

    let disk_container = ui::View::new();
    disk_container.set_position(cx - 280, 110);
    disk_container.set_size(560, 260);
    disk_container.set_color(tc.window_bg);
    page2.add(&disk_container);

    let disk_grid = ui::DataGrid::new(1, 1);
    disk_grid.set_visible(false);
    page2.add(&disk_grid);

    let p2_warn = ui::Label::new(
        "The selected disk will be erased and anyOS will be installed."
    );
    p2_warn.set_position(0, 390);
    p2_warn.set_size(WIN_W, 16);
    p2_warn.set_font_size(11);
    p2_warn.set_color(tc.window_bg);
    p2_warn.set_text_color(0xFF777777);
    p2_warn.set_text_align(ui::TEXT_ALIGN_CENTER);
    page2.add(&p2_warn);

    // ═══════════════════════════════════════════════════════════════
    // Page 3: Installation Progress + Completion
    // ═══════════════════════════════════════════════════════════════

    let page3 = ui::View::new();
    page3.set_dock(DOCK_FILL);
    page3.set_color(tc.window_bg);
    page3.set_visible(false);
    win.add(&page3);

    let p3_title = ui::Label::new("Installing anyOS...");
    p3_title.set_position(0, 30);
    p3_title.set_size(WIN_W, 34);
    p3_title.set_font_size(24);
    p3_title.set_color(tc.window_bg);
    p3_title.set_text_color(0xFFFFFFFF);
    p3_title.set_text_align(ui::TEXT_ALIGN_CENTER);
    page3.add(&p3_title);

    let phase_label = ui::Label::new("Preparing...");
    phase_label.set_position(0, 78);
    phase_label.set_size(WIN_W, 22);
    phase_label.set_font_size(14);
    phase_label.set_color(tc.window_bg);
    phase_label.set_text_color(0xFFCCCCCC);
    phase_label.set_text_align(ui::TEXT_ALIGN_CENTER);
    page3.add(&phase_label);

    // Progress bar (full width, centered)
    let bar_w = 560u32;
    let progress_card = ui::Card::new();
    progress_card.set_position(cx - (bar_w as i32 / 2) - 10, 116);
    progress_card.set_size(bar_w + 20, 44);
    page3.add(&progress_card);

    let progress_bar = ui::ProgressBar::new(0);
    progress_bar.set_position(10, 12);
    progress_bar.set_size(bar_w, 20);
    progress_card.add(&progress_bar);

    let status_label = ui::Label::new("");
    status_label.set_position(0, 175);
    status_label.set_size(WIN_W, 18);
    status_label.set_font_size(12);
    status_label.set_color(tc.window_bg);
    status_label.set_text_color(0xFF888888);
    status_label.set_text_align(ui::TEXT_ALIGN_CENTER);
    page3.add(&status_label);

    // Optional details toggle
    let btn_details = ui::Button::new("Show Details");
    btn_details.set_position(cx - 60, 204);
    btn_details.set_size(120, 26);
    page3.add(&btn_details);

    let content_w = bar_w + 20;
    let details_card = ui::Card::new();
    details_card.set_position(cx - (content_w as i32 / 2), 240);
    details_card.set_size(content_w, 130);
    details_card.set_visible(false);
    page3.add(&details_card);

    let details_log = ui::TextArea::new();
    details_log.set_position(4, 4);
    details_log.set_size(content_w - 8, 122);
    details_log.set_font_size(11);
    details_card.add(&details_log);

    // Completion: hidden until done — replaces progress UI
    let complete_label = ui::Label::new("Installation Complete");
    complete_label.set_position(0, 200);
    complete_label.set_size(WIN_W, 36);
    complete_label.set_font_size(22);
    complete_label.set_color(tc.window_bg);
    complete_label.set_text_color(0xFF34C759); // Green
    complete_label.set_text_align(ui::TEXT_ALIGN_CENTER);
    complete_label.set_visible(false);
    page3.add(&complete_label);

    let complete_sub = ui::Label::new(
        "anyOS has been installed successfully. Please restart your computer to complete the setup."
    );
    complete_sub.set_position(0, 244);
    complete_sub.set_size(WIN_W, 44);
    complete_sub.set_font_size(14);
    complete_sub.set_color(tc.window_bg);
    complete_sub.set_text_color(0xFFAAAAAA);
    complete_sub.set_text_align(ui::TEXT_ALIGN_CENTER);
    complete_sub.set_visible(false);
    page3.add(&complete_sub);

    let btn_reboot = ui::Button::new("Restart");
    btn_reboot.set_position(cx - 65, 310);
    btn_reboot.set_size(130, 36);
    btn_reboot.set_color(ACCENT);
    btn_reboot.set_text_color(0xFFFFFFFF);
    btn_reboot.set_visible(false);
    page3.add(&btn_reboot);

    let progress_card_id = Widget::id(&progress_card);
    let complete_label_id = Widget::id(&complete_label);
    let complete_sub_id = Widget::id(&complete_sub);

    // ── Marshal IDs for worker thread ──
    unsafe {
        PROGRESS_BAR_ID = Widget::id(&progress_bar);
        STATUS_LABEL_ID = Widget::id(&status_label);
        PHASE_LABEL_ID = Widget::id(&phase_label);
        BTN_REBOOT_ID = Widget::id(&btn_reboot);
    }

    // ── Initial data ──
    let disks = detect_disks();
    // Populate macOS-style disk cards (initial load after APP is set up below)
    // populate_disk_cards will be called after APP = Some(...)
    let _ = &disk_grid; // keep disk_grid referenced

    unsafe {
        APP = Some(InstallerApp {
            win,
            page0, page1, page2, page3,
            progress_card_id,
            complete_label_id,
            complete_sub_id,
            disk_grid,
            disk_container_id: disk_container.id(),
            disk_card_ids: Vec::new(),
            progress_bar,
            phase_label,
            status_label,
            btn_reboot,
            btn_details,
            details_card,
            details_log,
            details_text: String::new(),
            btn_back_id,
            btn_next_id,
            disks,
            selected_disk: None,
            current_step: 0,
            timer_id: 0,
            details_visible: false,
            last_copy_seq: 0,
            license_accepted: false,
        });
    }

    // Populate disk cards now that APP is initialized
    {
        let disks_copy = detect_disks();
        populate_disk_cards(&disks_copy);
        app().disks = disks_copy;
    }

    show_page(0);

    // ═══════════════════════════════════════════════════════════════
    // Event Handlers
    // ═══════════════════════════════════════════════════════════════

    app().disk_grid.on_selection_changed(|e| {
        app().selected_disk = Some(e.index as usize);
    });

    // Disk refresh happens automatically when navigating to page 1

    btn_next.on_click(move |_| {
        let a = app();
        match a.current_step {
            0 => {
                // Welcome → License
                show_page(1);
            }
            1 => {
                // License → Accept + go to Disk Selection
                a.license_accepted = true;
                show_page(2);
            }
            2 => {
                // Disk Selection → Confirm + Install
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
                // Confirmation dialog
                let msg = format!(
                    "All data on Disk {} ({}) will be permanently erased \
                     and anyOS will be installed.",
                    a.disks[idx].disk_id, format_size(a.disks[idx].size_sectors)
                );
                ui::MessageBox::show(ui::MessageBoxType::Warning, &msg, Some("Install"));
                INSTALL_DISK_ID.store(a.disks[idx].device_id as u32, Ordering::Release);
                INSTALL_MODE.store(0, Ordering::Release);
                start_install();
            }
            _ => {}
        }
    });

    btn_back.on_click(move |_| {
        let a = app();
        match a.current_step {
            1 => show_page(0),
            2 => show_page(1),
            _ => {}
        }
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
        let is_error = WORKER_ERROR.load(Ordering::Acquire);

        // Hide all progress UI
        ui::Control::from_id(a.progress_card_id).set_visible(false);
        a.progress_bar.set_visible(false);
        a.phase_label.set_visible(false);
        a.status_label.set_visible(false);
        a.btn_details.set_visible(false);
        a.details_card.set_visible(false);

        if is_error {
            // Show error state
            a.btn_reboot.set_text("Close");
            a.btn_reboot.set_visible(true);
            ui::Control::from_id(a.complete_label_id).set_text("Installation Failed");
            ui::Control::from_id(a.complete_label_id).set_text_color(0xFFFF6B6B); // Red
            ui::Control::from_id(a.complete_label_id).set_visible(true);
            ui::Control::from_id(a.complete_sub_id).set_text(
                "An error occurred during installation.\nPlease try again."
            );
            ui::Control::from_id(a.complete_sub_id).set_visible(true);
        } else {
            // Show success state
            ui::Control::from_id(a.complete_label_id).set_visible(true);
            ui::Control::from_id(a.complete_sub_id).set_visible(true);
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
