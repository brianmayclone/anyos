#![no_std]
#![no_main]

mod state;
mod helpers;
mod worker;

use anyos_std::{format, String, Vec};
use anyos_std::process;
use core::sync::atomic::Ordering;

use libanyui_client as ui;
use ui::{Widget, DOCK_TOP, DOCK_BOTTOM, DOCK_FILL};

use state::*;
use helpers::*;

anyos_std::entry!(main);

// ── Disk card UI ───────────────────────────────────────────────────────────

/// Build the list of installable targets from raw disk entries.
/// Returns only whole disks (if unpartitioned) or individual partitions
/// that are large enough.
fn build_target_list(all: &[DiskEntry]) -> Vec<usize> {
    const MIN_SECTORS: u64 = 256 * 1024 * 1024 / 512;
    let mut targets = Vec::new();
    for (i, d) in all.iter().enumerate() {
        if d.size_sectors < MIN_SECTORS { continue; }
        // Show whole disk only if it has no partitions
        if d.partition_index.is_none() {
            let has_parts = all.iter().any(|e| e.disk_id == d.disk_id && e.partition_index.is_some());
            if has_parts { continue; } // skip whole-disk entry, show partitions instead
        }
        targets.push(i);
    }
    targets
}

fn populate_disk_cards(disks: &[DiskEntry]) {
    let a = app();
    let container = ui::Control::from_id(a.disk_container_id);

    for &card_id in &a.disk_card_ids {
        ui::Control::from_id(card_id).set_visible(false);
    }
    a.disk_card_ids.clear();

    let targets = build_target_list(disks);
    let any_suitable = !targets.is_empty();

    // Auto-select largest target
    let mut best_idx: Option<usize> = None;
    let mut best_size: u64 = 0;
    for &ti in &targets {
        if disks[ti].size_sectors > best_size {
            best_size = disks[ti].size_sectors;
            best_idx = Some(ti);
        }
    }
    a.selected_disk = best_idx;

    let card_w: u32 = 160;
    let card_h: u32 = 200;
    let gap: i32 = 16;
    let max_per_row = 3;

    let total_cards = targets.len().min(max_per_row);
    let total_w = total_cards as i32 * card_w as i32 + (total_cards as i32 - 1).max(0) * gap;
    let container_w = 560i32;
    let x_offset = (container_w - total_w) / 2;

    for (vi, &di) in targets.iter().enumerate() {
        let disk = &disks[di];
        let col = (vi % max_per_row) as i32;
        let row = (vi / max_per_row) as i32;
        let x = x_offset + col * (card_w as i32 + gap);
        let y = row * (card_h as i32 + gap);

        let is_selected = a.selected_disk == Some(di);
        let card_color = if is_selected { 0xFF1A3A5C } else { 0xFF2A2A2E };

        let card = ui::Card::new();
        card.set_position(x, y);
        card.set_size(card_w, card_h);
        card.set_color(card_color);

        if let Some(ico) = ui::Icon::load("/System/media/icons/devices/generic.ico", 48) {
            let iv = ico.into_image_view(48, 48);
            iv.set_position((card_w as i32 - 48) / 2, 10);
            card.add(&iv);
        }

        // Title: "Disk 0" or "Disk 0, Part 1"
        let name = if let Some(pi) = disk.partition_index {
            format!("Disk {} Part {}", disk.disk_id, pi + 1)
        } else {
            format!("Disk {}", disk.disk_id)
        };
        let name_label = ui::Label::new(&name);
        name_label.set_position(0, 68);
        name_label.set_size(card_w, 20);
        name_label.set_font_size(14);
        name_label.set_color(0x00000000);
        name_label.set_text_color(0xFFFFFFFF);
        name_label.set_text_align(ui::TEXT_ALIGN_CENTER);
        card.add(&name_label);

        // Label (model string) — truncate to fit card width
        if !disk.label.is_empty() {
            let display_label = if disk.label.len() > 20 {
                format!("{}...", &disk.label[..18])
            } else {
                disk.label.clone()
            };
            let model_label = ui::Label::new(&display_label);
            model_label.set_position(0, 90);
            model_label.set_size(card_w, 16);
            model_label.set_font_size(10);
            model_label.set_color(0x00000000);
            model_label.set_text_color(0xFF888888);
            model_label.set_text_align(ui::TEXT_ALIGN_CENTER);
            card.add(&model_label);
        }

        // Size
        let size_str = format_size(disk.size_sectors);
        let size_label = ui::Label::new(&size_str);
        size_label.set_position(0, 108);
        size_label.set_size(card_w, 18);
        size_label.set_font_size(12);
        size_label.set_color(0x00000000);
        size_label.set_text_color(0xFFAAAAAA);
        size_label.set_text_align(ui::TEXT_ALIGN_CENTER);
        card.add(&size_label);

        // Status line
        let status_str = if disk.partition_index.is_none() {
            if disk.partition_count == 0 {
                String::from("Unpartitioned")
            } else {
                format!("{} partition{}", disk.partition_count,
                    if disk.partition_count == 1 { "" } else { "s" })
            }
        } else {
            String::from("Partition")
        };
        let status_label = ui::Label::new(&status_str);
        status_label.set_position(0, 128);
        status_label.set_size(card_w, 16);
        status_label.set_font_size(11);
        status_label.set_color(0x00000000);
        status_label.set_text_color(0xFF777777);
        status_label.set_text_align(ui::TEXT_ALIGN_CENTER);
        card.add(&status_label);

        // Selection indicator
        if is_selected {
            let dot = ui::View::new();
            dot.set_position((card_w as i32 - 8) / 2, card_h as i32 - 16);
            dot.set_size(8, 8);
            dot.set_color(ACCENT);
            card.add(&dot);
        }

        // Click area
        let click_area = ui::Label::new("");
        click_area.set_position(0, 0);
        click_area.set_size(card_w, card_h);
        click_area.set_color(0x00000000);
        click_area.on_click_raw(disk_card_click, di as u64);
        card.add(&click_area);

        a.disk_card_ids.push(card.id());
        container.add_child(card.id());
    }

    if disks.is_empty() || !any_suitable {
        let msg = if disks.is_empty() {
            "No disks detected."
        } else {
            "No suitable disk found. anyOS requires at least 256 MB."
        };
        let warn_label = ui::Label::new(msg);
        warn_label.set_position(0, 80);
        warn_label.set_size(560, 20);
        warn_label.set_font_size(13);
        warn_label.set_color(0x00000000);
        warn_label.set_text_color(if disks.is_empty() { 0xFF888888 } else { 0xFFFF6B6B });
        warn_label.set_text_align(ui::TEXT_ALIGN_CENTER);
        container.add_child(warn_label.id());
        a.disk_card_ids.push(warn_label.id());
    }

    let btn = ui::Control::from_id(a.btn_next_id);
    if any_suitable && a.selected_disk.is_some() {
        btn.set_enabled(true);
        btn.set_color(ACCENT);
    } else {
        btn.set_enabled(false);
        btn.set_color(0xFF555555);
    }
}

extern "C" fn disk_card_click(_control_id: u32, _event_type: u32, userdata: u64) {
    let idx = userdata as usize;
    app().selected_disk = Some(idx);
    let disks_copy = detect_disks();
    populate_disk_cards(&disks_copy);
    app().disks = disks_copy;
}

// ── Page navigation ────────────────────────────────────────────────────────

fn show_page(step: u32) {
    let a = app();
    a.current_step = step;
    a.page0.set_visible(step == 0);
    a.page1.set_visible(step == 1);
    a.page2.set_visible(step == 2);
    a.page3.set_visible(step == 3);
    a.page4.set_visible(step == 4);

    let show_back = step >= 1 && step <= 3;
    let show_next = step <= 3;
    unsafe {
        ui::marshal_set_visible(a.btn_back_id, show_back);
        ui::marshal_set_visible(a.btn_next_id, show_next);
    }

    let next_ctrl = ui::Control::from_id(a.btn_next_id);
    match step {
        0 => {
            next_ctrl.set_text("Continue");
            next_ctrl.set_enabled(true);
            next_ctrl.set_color(ACCENT);
        }
        1 => {
            next_ctrl.set_text("Agree");
            next_ctrl.set_enabled(true);
            next_ctrl.set_color(ACCENT);
        }
        2 => {
            next_ctrl.set_text("Continue");
            let has_selection = a.selected_disk.is_some();
            next_ctrl.set_enabled(has_selection);
            next_ctrl.set_color(if has_selection { ACCENT } else { 0xFF555555 });
        }
        3 => {
            next_ctrl.set_text("Install");
            // Disable until password is validated
            next_ctrl.set_enabled(false);
            next_ctrl.set_color(0xFF555555);
            // Clear fields when entering
            a.pw_field1.set_text("");
            a.pw_field2.set_text("");
            a.pw_error_label.set_text("");
        }
        _ => {}
    }
}

/// Validate the password fields and update the Install button + error label.
fn validate_password() {
    let a = app();
    let mut buf1 = [0u8; 128];
    let mut buf2 = [0u8; 128];
    let len1 = a.pw_field1.get_text(&mut buf1) as usize;
    let len2 = a.pw_field2.get_text(&mut buf2) as usize;
    let pw1 = core::str::from_utf8(&buf1[..len1]).unwrap_or("");
    let pw2 = core::str::from_utf8(&buf2[..len2]).unwrap_or("");

    let next_ctrl = ui::Control::from_id(a.btn_next_id);

    if pw1.is_empty() {
        a.pw_error_label.set_text("");
        next_ctrl.set_enabled(false);
        next_ctrl.set_color(0xFF555555);
        return;
    }

    if pw1.len() < 4 {
        a.pw_error_label.set_text("Password must be at least 4 characters.");
        a.pw_error_label.set_text_color(0xFFFF6B6B);
        next_ctrl.set_enabled(false);
        next_ctrl.set_color(0xFF555555);
        return;
    }

    if pw2.is_empty() {
        a.pw_error_label.set_text("");
        next_ctrl.set_enabled(false);
        next_ctrl.set_color(0xFF555555);
        return;
    }

    if pw1 != pw2 {
        a.pw_error_label.set_text("Passwords do not match.");
        a.pw_error_label.set_text_color(0xFFFF6B6B);
        next_ctrl.set_enabled(false);
        next_ctrl.set_color(0xFF555555);
        return;
    }

    a.pw_error_label.set_text("Passwords match.");
    a.pw_error_label.set_text_color(0xFF34C759);
    next_ctrl.set_enabled(true);
    next_ctrl.set_color(ACCENT);
}

// ── Install flow ───────────────────────────────────────────────────────────

fn start_install() {
    show_page(4);

    WORKER_ACTIVE.store(true, Ordering::Release);
    WORKER_DONE.store(false, Ordering::Release);
    WORKER_ERROR.store(false, Ordering::Release);
    WORKER_PROGRESS.store(0, Ordering::Release);

    if let Ok(h) = process::Thread::spawn_with_stack(worker::install_worker, 256 * 1024, "installer") {
        core::mem::forget(h);
    }
    app().timer_id = ui::set_timer(200, || { poll_worker(); });
}

fn poll_worker() {
    let a = app();

    let seq = COPY_FILE_SEQ.load(Ordering::Acquire);
    if seq != a.last_copy_seq {
        a.last_copy_seq = seq;
        let len = COPY_FILE_LEN.load(Ordering::Acquire) as usize;
        if len > 0 {
            let path = unsafe {
                core::str::from_utf8(&COPY_FILE_BUF[..len]).unwrap_or("")
            };
            let display = path.strip_prefix("/mnt/target").unwrap_or(path);
            a.details_text.push_str(display);
            a.details_text.push('\n');
            a.details_log.set_text(&a.details_text);
        }
    }

    if WORKER_DONE.load(Ordering::Acquire) {
        ui::kill_timer(a.timer_id);
        let is_error = WORKER_ERROR.load(Ordering::Acquire);

        ui::Control::from_id(a.progress_card_id).set_visible(false);
        a.progress_bar.set_visible(false);
        a.phase_label.set_visible(false);
        a.status_label.set_visible(false);
        a.btn_details.set_visible(false);
        a.details_card.set_visible(false);

        if is_error {
            a.btn_reboot.set_text("Close");
            a.btn_reboot.set_visible(true);
            ui::Control::from_id(a.complete_label_id).set_text("Installation Failed");
            ui::Control::from_id(a.complete_label_id).set_text_color(0xFFFF6B6B);
            ui::Control::from_id(a.complete_label_id).set_visible(true);
            a.phase_label.set_visible(true);
            a.status_label.set_visible(true);
            ui::Control::from_id(a.complete_sub_id).set_visible(true);
        } else {
            ui::Control::from_id(a.complete_label_id).set_visible(true);
            ui::Control::from_id(a.complete_sub_id).set_visible(true);
            a.btn_reboot.set_visible(true);
        }
    }
}

// ── UI construction ────────────────────────────────────────────────────────

fn main() {
    if !ui::init() { return; }

    let tc = ui::theme::colors();
    let win = ui::Window::new("Install anyOS", -1, -1, WIN_W, WIN_H);
    let cx = (WIN_W / 2) as i32;

    // ── Bottom bar ─────────────────────────────────────────────────────────
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

    // ═══════════════════════════════════════════════════════════════════════
    // Page 0: Welcome
    // ═══════════════════════════════════════════════════════════════════════

    let page0 = ui::View::new();
    page0.set_dock(DOCK_FILL);
    page0.set_color(tc.window_bg);
    win.add(&page0);

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

    // ═══════════════════════════════════════════════════════════════════════
    // Page 1: License Agreement
    // ═══════════════════════════════════════════════════════════════════════

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

    // ═══════════════════════════════════════════════════════════════════════
    // Page 2: Select Disk
    // ═══════════════════════════════════════════════════════════════════════

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

    // ═══════════════════════════════════════════════════════════════════════
    // Page 3: Set Root Password
    // ═══════════════════════════════════════════════════════════════════════

    let page3 = ui::View::new();
    page3.set_dock(DOCK_FILL);
    page3.set_color(tc.window_bg);
    page3.set_visible(false);
    win.add(&page3);

    let pw_title = ui::Label::new("Create a Root Password");
    pw_title.set_position(0, 30);
    pw_title.set_size(WIN_W, 34);
    pw_title.set_font_size(24);
    pw_title.set_color(tc.window_bg);
    pw_title.set_text_color(0xFFFFFFFF);
    pw_title.set_text_align(ui::TEXT_ALIGN_CENTER);
    page3.add(&pw_title);

    let pw_desc = ui::Label::new(
        "You will need this password to log in after the installation."
    );
    pw_desc.set_position(0, 68);
    pw_desc.set_size(WIN_W, 20);
    pw_desc.set_font_size(13);
    pw_desc.set_color(tc.window_bg);
    pw_desc.set_text_color(0xFF999999);
    pw_desc.set_text_align(ui::TEXT_ALIGN_CENTER);
    page3.add(&pw_desc);

    let field_w: u32 = 360;
    let field_x = cx - (field_w as i32 / 2);

    let pw_label1 = ui::Label::new("Password");
    pw_label1.set_position(field_x, 130);
    pw_label1.set_size(field_w, 18);
    pw_label1.set_font_size(13);
    pw_label1.set_color(tc.window_bg);
    pw_label1.set_text_color(0xFFCCCCCC);
    page3.add(&pw_label1);

    let pw_field1 = ui::TextField::new();
    pw_field1.set_position(field_x, 152);
    pw_field1.set_size(field_w, 32);
    pw_field1.set_font_size(14);
    pw_field1.set_password_mode(true);
    pw_field1.set_placeholder("Enter password");
    pw_field1.set_max_length(64);
    page3.add(&pw_field1);

    let pw_label2 = ui::Label::new("Confirm Password");
    pw_label2.set_position(field_x, 204);
    pw_label2.set_size(field_w, 18);
    pw_label2.set_font_size(13);
    pw_label2.set_color(tc.window_bg);
    pw_label2.set_text_color(0xFFCCCCCC);
    page3.add(&pw_label2);

    let pw_field2 = ui::TextField::new();
    pw_field2.set_position(field_x, 226);
    pw_field2.set_size(field_w, 32);
    pw_field2.set_font_size(14);
    pw_field2.set_password_mode(true);
    pw_field2.set_placeholder("Confirm password");
    pw_field2.set_max_length(64);
    page3.add(&pw_field2);

    let pw_error_label = ui::Label::new("");
    pw_error_label.set_position(0, 274);
    pw_error_label.set_size(WIN_W, 18);
    pw_error_label.set_font_size(12);
    pw_error_label.set_color(tc.window_bg);
    pw_error_label.set_text_color(0xFFFF6B6B);
    pw_error_label.set_text_align(ui::TEXT_ALIGN_CENTER);
    page3.add(&pw_error_label);

    let pw_hint = ui::Label::new("The root account has full system access. Choose a strong password.");
    pw_hint.set_position(0, 320);
    pw_hint.set_size(WIN_W, 16);
    pw_hint.set_font_size(11);
    pw_hint.set_color(tc.window_bg);
    pw_hint.set_text_color(0xFF777777);
    pw_hint.set_text_align(ui::TEXT_ALIGN_CENTER);
    page3.add(&pw_hint);

    // ═══════════════════════════════════════════════════════════════════════
    // Page 4: Installation Progress + Completion
    // ═══════════════════════════════════════════════════════════════════════

    let page4 = ui::View::new();
    page4.set_dock(DOCK_FILL);
    page4.set_color(tc.window_bg);
    page4.set_visible(false);
    win.add(&page4);

    let p4_title = ui::Label::new("Installing anyOS...");
    p4_title.set_position(0, 30);
    p4_title.set_size(WIN_W, 34);
    p4_title.set_font_size(24);
    p4_title.set_color(tc.window_bg);
    p4_title.set_text_color(0xFFFFFFFF);
    p4_title.set_text_align(ui::TEXT_ALIGN_CENTER);
    page4.add(&p4_title);

    let phase_label = ui::Label::new("Preparing...");
    phase_label.set_position(0, 78);
    phase_label.set_size(WIN_W, 22);
    phase_label.set_font_size(14);
    phase_label.set_color(tc.window_bg);
    phase_label.set_text_color(0xFFCCCCCC);
    phase_label.set_text_align(ui::TEXT_ALIGN_CENTER);
    page4.add(&phase_label);

    let bar_w = 560u32;
    let progress_card = ui::Card::new();
    progress_card.set_position(cx - (bar_w as i32 / 2) - 10, 116);
    progress_card.set_size(bar_w + 20, 44);
    page4.add(&progress_card);

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
    page4.add(&status_label);

    let btn_details = ui::Button::new("Show Details");
    btn_details.set_position(cx - 60, 204);
    btn_details.set_size(120, 26);
    page4.add(&btn_details);

    let content_w = bar_w + 20;
    let details_card = ui::Card::new();
    details_card.set_position(cx - (content_w as i32 / 2), 240);
    details_card.set_size(content_w, 130);
    details_card.set_visible(false);
    page4.add(&details_card);

    let details_log = ui::TextArea::new();
    details_log.set_position(4, 4);
    details_log.set_size(content_w - 8, 122);
    details_log.set_font_size(11);
    details_card.add(&details_log);

    let complete_label = ui::Label::new("Installation Complete");
    complete_label.set_position(0, 200);
    complete_label.set_size(WIN_W, 36);
    complete_label.set_font_size(22);
    complete_label.set_color(tc.window_bg);
    complete_label.set_text_color(0xFF34C759);
    complete_label.set_text_align(ui::TEXT_ALIGN_CENTER);
    complete_label.set_visible(false);
    page4.add(&complete_label);

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
    page4.add(&complete_sub);

    let btn_reboot = ui::Button::new("Restart");
    btn_reboot.set_position(cx - 65, 310);
    btn_reboot.set_size(130, 36);
    btn_reboot.set_color(ACCENT);
    btn_reboot.set_text_color(0xFFFFFFFF);
    btn_reboot.set_visible(false);
    page4.add(&btn_reboot);

    let progress_card_id = Widget::id(&progress_card);
    let complete_label_id = Widget::id(&complete_label);
    let complete_sub_id = Widget::id(&complete_sub);

    unsafe {
        PROGRESS_BAR_ID = Widget::id(&progress_bar);
        STATUS_LABEL_ID = Widget::id(&status_label);
        PHASE_LABEL_ID = Widget::id(&phase_label);
        BTN_REBOOT_ID = Widget::id(&btn_reboot);
    }

    // ── Initial data ───────────────────────────────────────────────────────
    let disks = detect_disks();
    let _ = &disk_grid;

    set_app(InstallerApp {
        win,
        page0, page1, page2, page3, page4,
        progress_card_id,
        complete_label_id,
        complete_sub_id,
        disk_grid,
        disk_container_id: disk_container.id(),
        disk_card_ids: Vec::new(),
        pw_field1,
        pw_field2,
        pw_error_label,
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

    {
        let disks_copy = detect_disks();
        populate_disk_cards(&disks_copy);
        app().disks = disks_copy;
    }

    show_page(0);

    // ── UEFI boot check ────────────────────────────────────────────────────
    if is_uefi_boot() {
        ui::MessageBox::show(ui::MessageBoxType::Warning,
            "This system was booted in UEFI mode.\n\n\
             anyOS currently only supports BIOS/MBR installation.\n\
             Please reboot in Legacy/CSM mode to install anyOS.",
            None);
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Event Handlers
    // ═══════════════════════════════════════════════════════════════════════

    app().disk_grid.on_selection_changed(|e| {
        app().selected_disk = Some(e.index as usize);
    });

    // Password field validation on every keystroke
    app().pw_field1.on_text_changed(|_| { validate_password(); });
    app().pw_field2.on_text_changed(|_| { validate_password(); });

    btn_next.on_click(move |_| {
        let a = app();
        match a.current_step {
            0 => {
                // Welcome → License
                show_page(1);
            }
            1 => {
                // License → Disk Selection
                a.license_accepted = true;
                show_page(2);
            }
            2 => {
                // Disk Selection → Password Setup
                let idx = match a.selected_disk {
                    Some(i) if i < a.disks.len() => i,
                    _ => {
                        ui::MessageBox::show(ui::MessageBoxType::Warning,
                            "Please select a target.", None);
                        return;
                    }
                };

                let disk = &a.disks[idx];

                // Check for existing OS on the target disk
                if let Some(os_name) = detect_existing_os(disk.device_id) {
                    let warn_msg = format!(
                        "The selected disk appears to contain {}.\n\
                         Installing anyOS will erase all data on this disk.\n\n\
                         Do you want to continue?",
                        os_name
                    );
                    ui::MessageBox::show(ui::MessageBoxType::Warning, &warn_msg, Some("Continue"));
                }

                show_page(3);
            }
            3 => {
                // Password Setup → Confirm + Install
                let mut buf1 = [0u8; 128];
                let len1 = a.pw_field1.get_text(&mut buf1) as usize;
                let pw = core::str::from_utf8(&buf1[..len1]).unwrap_or("");

                if pw.is_empty() || pw.len() < 4 {
                    a.pw_error_label.set_text("Password must be at least 4 characters.");
                    a.pw_error_label.set_text_color(0xFFFF6B6B);
                    return;
                }

                // Store password for worker thread
                store_root_password(pw);

                let idx = a.selected_disk.unwrap();
                let disk = &a.disks[idx];
                let is_partition = disk.partition_index.is_some();

                // Build confirmation message
                let target_name = if let Some(pi) = disk.partition_index {
                    format!("Disk {} Partition {} ({})",
                        disk.disk_id, pi + 1, format_size(disk.size_sectors))
                } else {
                    format!("Disk {} ({})",
                        disk.disk_id, format_size(disk.size_sectors))
                };
                let msg = format!(
                    "All data on {} will be permanently erased \
                     and anyOS will be installed.",
                    target_name
                );
                ui::MessageBox::show(ui::MessageBoxType::Warning, &msg, Some("Install"));

                INSTALL_DISK_ID.store(disk.device_id as u32, Ordering::Release);
                if is_partition {
                    // Install to existing partition (format only, no partitioning)
                    INSTALL_MODE.store(1, Ordering::Release);
                } else {
                    // Whole disk — create partition table + format
                    INSTALL_MODE.store(0, Ordering::Release);
                }
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
            3 => show_page(2),
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
