//! Disk Utility — GParted-style partition manager for anyOS.

#![no_std]
#![no_main]

use libanyui_client as ui;
use ui::{Widget, ColumnDef, ALIGN_LEFT, ALIGN_RIGHT, DOCK_TOP, DOCK_BOTTOM, DOCK_LEFT, DOCK_FILL};
use anyos_std::{format, i18n, vec, String, Vec};
use anyos_std::sys;

anyos_std::entry!(main);

const WIN_W: u32 = 900;
const WIN_H: u32 = 600;
const SIDEBAR_W: u32 = 200;
const BAR_H: u32 = 80;

const COL_EXFAT: u32 = 0xFF5856D6;
const COL_FAT32: u32 = 0xFF34C759;
const COL_FAT16: u32 = 0xFFFF9500;
const COL_LINUX: u32 = 0xFFFF3B30;
const COL_ESP: u32   = 0xFFFFCC00;
const COL_FREE: u32  = 0xFF6E6E73;
const COL_NTFS: u32  = 0xFF30B0C7;
const COL_SWAP: u32  = 0xFFAF52DE;
const ACCENT: u32    = 0xFF007AFF;

const MAX_SIDEBAR_DISKS: usize = 8;
const MAX_MOUNTS: usize = 32;

// ── Data ────────────────────────────────────────────────────────────────────

fn read_u64_le(buf: &[u8], off: usize) -> u64 {
    u64::from_le_bytes([buf[off],buf[off+1],buf[off+2],buf[off+3],
                        buf[off+4],buf[off+5],buf[off+6],buf[off+7]])
}

fn type_name(t: u8) -> &'static str {
    match t {
        0x01 => "FAT12",
        0x04 | 0x06 | 0x0E => "FAT16",
        0x0B | 0x0C => "FAT32",
        0x07 => "exFAT/NTFS",
        0x82 => "Linux swap",
        0x83 => "Linux (ext)",
        0xEE => "GPT",
        0xEF => "EFI System",
        0x00 => "unallocated",
        _ => "Unknown",
    }
}

fn type_color(t: u8) -> u32 {
    match t {
        0x07 => COL_EXFAT,
        0x0B | 0x0C => COL_FAT32,
        0x04 | 0x06 | 0x0E => COL_FAT16,
        0x83 => COL_LINUX,
        0x82 => COL_SWAP,
        0xEF => COL_ESP,
        0x00 => COL_FREE,
        _ => 0xFF8E8E93,
    }
}

/// Convert disk_id to Linux-style device letter: 0 → 'a', 1 → 'b', …
fn disk_letter(disk_id: u8) -> char {
    if disk_id < 26 { (b'a' + disk_id) as char } else { '?' }
}

fn format_size(sectors: u64) -> String {
    let bytes = sectors * 512;
    if bytes >= 1024 * 1024 * 1024 {
        let g = bytes / (1024 * 1024 * 1024);
        let f = (bytes % (1024 * 1024 * 1024)) * 10 / (1024 * 1024 * 1024);
        format!("{}.{} GB", g, f)
    } else if bytes >= 1024 * 1024 {
        let m = bytes / (1024 * 1024);
        let f = (bytes % (1024 * 1024)) * 10 / (1024 * 1024);
        format!("{}.{} MB", m, f)
    } else if bytes >= 1024 {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{} B", bytes)
    }
}

fn format_size_short(sectors: u64) -> String {
    let bytes = sectors * 512;
    if bytes >= 1024 * 1024 * 1024 {
        let g = bytes / (1024 * 1024 * 1024);
        let f = (bytes % (1024 * 1024 * 1024)) * 10 / (1024 * 1024 * 1024);
        format!("{}.{} G", g, f)
    } else if bytes >= 1024 * 1024 {
        format!("{} M", bytes / (1024 * 1024))
    } else {
        format!("{} K", bytes / 1024)
    }
}

#[derive(Clone)]
struct DiskInfo {
    disk_id: u8,
    total_sectors: u64,
}

#[derive(Clone)]
struct PartInfo {
    index: u8,
    ptype: u8,
    bootable: bool,
    start_lba: u64,
    size_sectors: u64,
}

/// A display row — either a real partition or an unallocated gap.
struct DisplayRow {
    name: String,
    fs_type: String,
    ptype: u8,
    mount_point: String,
    flags: String,
    start_lba: u64,
    end_lba: u64,
    size_sectors: u64,
    is_free: bool,
    part_index: Option<u8>,
}

fn load_disks() -> Vec<DiskInfo> {
    let mut buf = [0u8; 32 * 16];
    let count = sys::disk_list(&mut buf);
    let mut disks = Vec::new();
    for i in 0..count as usize {
        let off = i * 32;
        if buf[off + 2] == 0xFF {
            disks.push(DiskInfo {
                disk_id: buf[off + 1],
                total_sectors: read_u64_le(&buf, off + 12),
            });
        }
    }
    disks
}

fn load_partitions(disk_id: u32) -> Vec<PartInfo> {
    let mut buf = [0u8; 32 * 8];
    let count = sys::disk_partitions(disk_id, &mut buf);
    let mut parts = Vec::new();
    if count == u32::MAX || count == 0 {
        return parts;
    }
    for i in 0..count as usize {
        let off = i * 32;
        parts.push(PartInfo {
            index: buf[off],
            ptype: buf[off + 1],
            bootable: buf[off + 2] != 0,
            start_lba: read_u64_le(&buf, off + 4),
            size_sectors: read_u64_le(&buf, off + 12),
        });
    }
    parts.sort_by_key(|p| p.start_lba);
    parts
}

/// Load current mounts as (mount_path, fs_type_str) pairs.
fn load_mounts() -> Vec<(String, String)> {
    let mut buf = [0u8; 4096];
    let n = anyos_std::fs::list_mounts(&mut buf);
    let mut mounts = Vec::new();
    if n == 0 || n == u32::MAX {
        return mounts;
    }
    let text = core::str::from_utf8(&buf[..n as usize]).unwrap_or("");
    for line in text.split('\n') {
        if line.is_empty() { continue; }
        let mut parts = line.splitn(3, '\t');
        let path = parts.next().unwrap_or("");
        let fs = parts.next().unwrap_or("");
        if !path.is_empty() {
            mounts.push((String::from(path), String::from(fs)));
        }
    }
    mounts
}

/// Build display rows: partitions + unallocated gaps between them.
fn build_display_rows(disk: &DiskInfo, parts: &[PartInfo], mounts: &[(String, String)]) -> Vec<DisplayRow> {
    let mut rows = Vec::new();
    let total = disk.total_sectors;
    // Start cursor at the first partition's start or standard 2048 alignment
    let first_usable: u64 = if !parts.is_empty() {
        parts[0].start_lba.min(2048)
    } else {
        2048
    };
    let mut cursor = first_usable;

    for p in parts {
        // Gap before this partition?
        if p.start_lba > cursor {
            let gap = p.start_lba - cursor;
            rows.push(DisplayRow {
                name: String::from("unallocated"),
                fs_type: String::from("unallocated"),
                ptype: 0x00,
                mount_point: String::new(),
                flags: String::new(),
                start_lba: cursor,
                end_lba: p.start_lba - 1,
                size_sectors: gap,
                is_free: true,
                part_index: None,
            });
        }

        // The partition itself
        let part_name = format!("sd{}{}", disk_letter(disk.disk_id), p.index + 1);
        let dev_path = format!("/dev/sd{}{}", disk_letter(disk.disk_id), p.index + 1);

        // Find mount point
        let mp = mounts.iter()
            .find(|(_, _)| {
                // Simple heuristic: check if any mount contains our device name
                // Real implementation would match device to mount
                false
            })
            .map(|(p, _)| p.clone())
            .unwrap_or_default();

        let mut flags = String::new();
        if p.bootable {
            flags = String::from("boot");
        }

        rows.push(DisplayRow {
            name: part_name,
            fs_type: String::from(type_name(p.ptype)),
            ptype: p.ptype,
            mount_point: mp,
            flags,
            start_lba: p.start_lba,
            end_lba: p.start_lba + p.size_sectors - 1,
            size_sectors: p.size_sectors,
            is_free: false,
            part_index: Some(p.index),
        });

        cursor = p.start_lba + p.size_sectors;
    }

    // Trailing free space
    if cursor < total {
        let gap = total - cursor;
        rows.push(DisplayRow {
            name: String::from("unallocated"),
            fs_type: String::from("unallocated"),
            ptype: 0x00,
            mount_point: String::new(),
            flags: String::new(),
            start_lba: cursor,
            end_lba: total - 1,
            size_sectors: gap,
            is_free: true,
            part_index: None,
        });
    }

    rows
}

fn find_free_space(disk: &DiskInfo, parts: &[PartInfo]) -> Option<(u64, u64)> {
    let start: u64 = 2048;
    let end = disk.total_sectors;
    let mut used: Vec<(u64, u64)> = parts.iter().map(|p| (p.start_lba, p.start_lba + p.size_sectors)).collect();
    used.sort_by_key(|&(s, _)| s);
    let mut c = start;
    for &(s, e) in &used {
        if c < s { return Some((c, s - c)); }
        if e > c { c = e; }
    }
    if c < end { Some((c, end - c)) } else { None }
}

// ── Draw ────────────────────────────────────────────────────────────────────

fn draw_bar(canvas: &ui::Canvas, w: u32, h: u32, disk: &DiskInfo, rows: &[DisplayRow]) {
    let tc = ui::theme::colors();
    canvas.clear(tc.window_bg);
    if rows.is_empty() {
        canvas.fill_rect(4, 4, w - 8, h - 8, COL_FREE);
        canvas.draw_text((w / 2 - 40) as i32, (h / 2 - 6) as i32, 0xFFCCCCCC, 0, 11, "No partitions");
        return;
    }
    let total = disk.total_sectors.max(1);
    let m = 4u32;
    let iw = (w - m * 2) as u64;
    let ih = h - m * 2;
    let gap_px = 2u32; // gap between segments

    for (i, row) in rows.iter().enumerate() {
        let px = m as i32 + (row.start_lba * iw / total) as i32;
        let mut pw = ((row.size_sectors * iw / total) as u32).max(8);
        // Leave gap between segments
        if i + 1 < rows.len() && pw > gap_px * 2 {
            pw -= gap_px;
        }

        let color = type_color(row.ptype);
        canvas.fill_rect(px, m as i32, pw, ih, color);

        // Draw border for better separation
        if !row.is_free {
            let darker = darken_color(color, 30);
            canvas.draw_rect(px, m as i32, pw, ih, darker, 1);
        }

        // Draw label on segment if wide enough
        if pw > 60 {
            let label = if row.is_free {
                format!("unallocated\n{}", format_size_short(row.size_sectors))
            } else {
                format!("{}\n{}", row.name, format_size_short(row.size_sectors))
            };
            // Split into two lines
            let line1_end = label.find('\n').unwrap_or(label.len());
            let line1 = &label[..line1_end];
            let line2 = if line1_end < label.len() { &label[line1_end + 1..] } else { "" };

            let text_color = if row.is_free { 0xFFAAAAAA } else { 0xFFFFFFFF };
            let tx = px + 6;
            let ty = m as i32 + (ih as i32 / 2) - 12;
            canvas.draw_text(tx, ty, text_color, 0, 11, line1);
            if !line2.is_empty() {
                canvas.draw_text(tx, ty + 14, text_color, 2, 10, line2);
            }
        } else if pw > 30 {
            // Just show size
            let text_color = if row.is_free { 0xFFAAAAAA } else { 0xFFFFFFFF };
            canvas.draw_text(px + 4, m as i32 + (ih as i32 / 2) - 5, text_color, 2, 9,
                &format_size_short(row.size_sectors));
        }
    }
}

fn darken_color(c: u32, amount: u32) -> u32 {
    let a = c & 0xFF000000;
    let r = ((c >> 16) & 0xFF).saturating_sub(amount);
    let g = ((c >> 8) & 0xFF).saturating_sub(amount);
    let b = (c & 0xFF).saturating_sub(amount);
    a | (r << 16) | (g << 8) | b
}

fn add_legend(row: &ui::FlowPanel, name: &str, color: u32) {
    let dot = ui::View::new();
    dot.set_size(10, 10);
    dot.set_color(color);
    dot.set_margin(0, 3, 4, 0);
    row.add(&dot);
    let lbl = ui::Label::new(name);
    lbl.set_size(60, 14);
    lbl.set_font_size(10);
    lbl.set_color(ui::theme::colors().text_secondary);
    lbl.set_margin(0, 2, 10, 0);
    row.add(&lbl);
}

fn write_le32_buf(b: &mut [u8], o: usize, v: u32) {
    b[o] = v as u8;
    b[o + 1] = (v >> 8) as u8;
    b[o + 2] = (v >> 16) as u8;
    b[o + 3] = (v >> 24) as u8;
}
fn write_le16_buf(b: &mut [u8], o: usize, v: u16) {
    b[o] = v as u8;
    b[o + 1] = (v >> 8) as u8;
}
fn write_le64_buf(b: &mut [u8], o: usize, v: u64) {
    for i in 0..8 {
        b[o + i] = (v >> (i * 8)) as u8;
    }
}

// ── Main ────────────────────────────────────────────────────────────────────

fn main() {
    if !ui::init() { return; }
    i18n::init();
    let tc = ui::theme::colors();
    let win = ui::Window::new("Disk Utility", -1, -1, WIN_W, WIN_H);

    // ── Menu ──
    let mut mb = ui::MenuBarBuilder::new()
        .menu("File")
            .item(1, "Refresh", 0)
            .separator()
            .item(2, "Quit", 0)
        .end_menu()
        .menu("Partition")
            .item(10, "New...", 0)
            .item(11, "Delete", 0)
            .separator()
            .item(12, "Format as exFAT", 0)
            .item(13, "Format as FAT32", 0)
            .separator()
            .item(14, "Set Boot Flag", 0)
            .item(15, "Clear Boot Flag", 0)
        .end_menu()
        .menu("Device")
            .item(20, "Rescan Disks", 0)
        .end_menu();
    let menu = ui::MenuBar::set(win.id(), mb.build());

    // ── Toolbar ──
    let toolbar = ui::Toolbar::new();
    toolbar.set_dock(DOCK_TOP);
    toolbar.set_size(WIN_W, 46);
    toolbar.set_color(ui::theme::darken(tc.window_bg, 10));
    toolbar.set_padding(8, 6, 8, 6);
    win.add(&toolbar);

    let btn_rescan = toolbar.add_icon_button("");
    btn_rescan.set_size(34, 34);
    btn_rescan.set_system_icon("refresh", ui::IconType::Outline, 0xFFCCCCCC, 20);
    btn_rescan.set_tooltip("Rescan disks");
    toolbar.add_separator();

    let btn_new = toolbar.add_icon_button("");
    btn_new.set_size(34, 34);
    btn_new.set_system_icon("circle-plus", ui::IconType::Outline, 0xFFCCCCCC, 20);
    btn_new.set_tooltip("New partition");

    let btn_delete = toolbar.add_icon_button("");
    btn_delete.set_size(34, 34);
    btn_delete.set_system_icon("trash", ui::IconType::Outline, 0xFFFF6B6B, 20);
    btn_delete.set_tooltip("Delete partition");

    toolbar.add_separator();

    let btn_format = toolbar.add_icon_button("");
    btn_format.set_size(34, 34);
    btn_format.set_system_icon("device-floppy", ui::IconType::Outline, 0xFFCCCCCC, 20);
    btn_format.set_tooltip("Format partition");

    toolbar.add_separator();

    let btn_boot = toolbar.add_icon_button("");
    btn_boot.set_size(34, 34);
    btn_boot.set_system_icon("flag", ui::IconType::Outline, 0xFFCCCCCC, 20);
    btn_boot.set_tooltip("Toggle boot flag");

    toolbar.add_separator();

    let btn_info = toolbar.add_icon_button("");
    btn_info.set_size(34, 34);
    btn_info.set_system_icon("info-circle", ui::IconType::Outline, 0xFFCCCCCC, 20);
    btn_info.set_tooltip("Partition info");

    // ── Sidebar: disk list ──
    let sidebar = ui::View::new();
    sidebar.set_dock(DOCK_LEFT);
    sidebar.set_size(SIDEBAR_W, WIN_H);
    sidebar.set_color(ui::theme::darken(tc.window_bg, 12));
    win.add(&sidebar);

    let side_title = ui::Label::new("Disks");
    side_title.set_position(14, 8);
    side_title.set_size(170, 20);
    side_title.set_font_size(13);
    side_title.set_color(ui::theme::darken(tc.window_bg, 12));
    side_title.set_text_color(0xFF888888);
    sidebar.add(&side_title);

    let side_div = ui::Divider::new();
    side_div.set_position(10, 30);
    side_div.set_size(SIDEBAR_W - 20, 1);
    sidebar.add(&side_div);

    // Disk entries — each has a button (name) + label (size + partition count)
    let mut disk_buttons: [ui::Button; MAX_SIDEBAR_DISKS] =
        core::array::from_fn(|_| ui::Button::new(""));
    let mut disk_info_labels: [ui::Label; MAX_SIDEBAR_DISKS] =
        core::array::from_fn(|_| ui::Label::new(""));

    let sidebar_bg = ui::theme::darken(tc.window_bg, 12);
    let selected_bg = ui::theme::darken(tc.window_bg, 2);

    for i in 0..MAX_SIDEBAR_DISKS {
        let y = 38 + i as i32 * 52;
        let ref btn = disk_buttons[i];
        btn.set_position(6, y);
        btn.set_size(SIDEBAR_W - 12, 46);
        btn.set_color(sidebar_bg);
        btn.set_text_color(0xFFFFFFFF);
        btn.set_font_size(13);
        btn.set_visible(false);
        sidebar.add(btn);

        let ref lbl = disk_info_labels[i];
        lbl.set_position(14, y + 26);
        lbl.set_size(SIDEBAR_W - 24, 16);
        lbl.set_font_size(10);
        lbl.set_color(sidebar_bg);
        lbl.set_text_color(0xFF999999);
        lbl.set_visible(false);
        sidebar.add(lbl);
    }

    // ── Status bar ──
    let status_bar = ui::View::new();
    status_bar.set_dock(DOCK_BOTTOM);
    status_bar.set_size(WIN_W, 26);
    status_bar.set_color(ui::theme::darken(tc.window_bg, 6));
    win.add(&status_bar);
    let info_label = ui::Label::new("");
    info_label.set_position(12, 5);
    info_label.set_size(680, 16);
    info_label.set_font_size(11);
    info_label.set_color(ui::theme::darken(tc.window_bg, 6));
    info_label.set_text_color(0xFF999999);
    status_bar.add(&info_label);

    // ── Main content ──
    let content = ui::View::new();
    content.set_dock(DOCK_FILL);
    content.set_color(tc.window_bg);
    win.add(&content);

    let cw = WIN_W - SIDEBAR_W;

    // Partition bar (visual representation)
    let bar_card = ui::Card::new();
    bar_card.set_position(12, 10);
    bar_card.set_size(cw - 24, BAR_H + 8);
    content.add(&bar_card);
    let canvas = ui::Canvas::new(cw - 32, BAR_H);
    canvas.set_position(4, 4);
    bar_card.add(&canvas);

    // Legend row
    let legend = ui::FlowPanel::new();
    legend.set_position(12, (BAR_H + 24) as i32);
    legend.set_size(cw - 24, 18);
    content.add(&legend);
    add_legend(&legend, "exFAT", COL_EXFAT);
    add_legend(&legend, "NTFS", COL_NTFS);
    add_legend(&legend, "FAT32", COL_FAT32);
    add_legend(&legend, "FAT16", COL_FAT16);
    add_legend(&legend, "Linux", COL_LINUX);
    add_legend(&legend, "swap", COL_SWAP);
    add_legend(&legend, "EFI", COL_ESP);
    add_legend(&legend, "Free", COL_FREE);

    // Partition grid (table)
    let grid_top = BAR_H + 48;
    let grid_card = ui::Card::new();
    grid_card.set_position(12, grid_top as i32);
    grid_card.set_size(cw - 24, WIN_H - 46 - 26 - grid_top - 16);
    content.add(&grid_card);

    let grid_h = WIN_H - 46 - 26 - grid_top - 28;
    let grid = ui::DataGrid::new(cw - 34, grid_h);
    grid.set_position(5, 5);
    grid.set_row_height(24);
    grid.set_header_height(28);
    grid.set_columns(&[
        ColumnDef::new("Partition").width(110).align(ALIGN_LEFT),
        ColumnDef::new("File System").width(100).align(ALIGN_LEFT),
        ColumnDef::new("Mount Point").width(100).align(ALIGN_LEFT),
        ColumnDef::new("Flags").width(60).align(ALIGN_LEFT),
        ColumnDef::new("Size").width(90).align(ALIGN_RIGHT),
        ColumnDef::new("Start LBA").width(90).align(ALIGN_RIGHT),
        ColumnDef::new("End LBA").width(90).align(ALIGN_RIGHT),
    ]);
    grid_card.add(&grid);

    // Context menu for grid rows
    let ctx_menu = ui::ContextMenu::new(
        "New Partition...|Format as exFAT|Format as FAT32|Delete Partition|Toggle Boot Flag|Partition Info"
    );
    grid.set_context_menu(&ctx_menu);

    // ── Create Partition Panel (hidden by default, overlays grid) ──
    let create_panel = ui::View::new();
    create_panel.set_position(12, grid_top as i32);
    create_panel.set_size(cw - 24, WIN_H - 46 - 26 - grid_top - 16);
    create_panel.set_color(tc.window_bg);
    create_panel.set_visible(false);
    content.add(&create_panel);

    let cp_title = ui::Label::new("Create New Partition");
    cp_title.set_position(16, 12);
    cp_title.set_size(300, 24);
    cp_title.set_font_size(15);
    cp_title.set_color(tc.window_bg);
    cp_title.set_text_color(0xFFFFFFFF);
    create_panel.add(&cp_title);

    let cp_free_label = ui::Label::new("Free space: --");
    cp_free_label.set_position(16, 42);
    cp_free_label.set_size(400, 16);
    cp_free_label.set_font_size(12);
    cp_free_label.set_color(tc.window_bg);
    cp_free_label.set_text_color(0xFF999999);
    create_panel.add(&cp_free_label);

    // Size slider
    let cp_size_label = ui::Label::new("Partition size:");
    cp_size_label.set_position(16, 76);
    cp_size_label.set_size(120, 16);
    cp_size_label.set_font_size(12);
    cp_size_label.set_color(tc.window_bg);
    cp_size_label.set_text_color(0xFFCCCCCC);
    create_panel.add(&cp_size_label);

    let cp_slider = ui::Slider::new(100);
    cp_slider.set_position(16, 100);
    cp_slider.set_size(cw - 220, 20);
    create_panel.add(&cp_slider);

    let cp_size_value = ui::Label::new("-- MB");
    cp_size_value.set_position((cw - 190) as i32, 100);
    cp_size_value.set_size(160, 18);
    cp_size_value.set_font_size(13);
    cp_size_value.set_color(tc.window_bg);
    cp_size_value.set_text_color(0xFFFFFFFF);
    create_panel.add(&cp_size_value);

    // Size text input
    let cp_size_input = ui::TextField::new();
    cp_size_input.set_position(16, 136);
    cp_size_input.set_size(140, 28);
    cp_size_input.set_placeholder("Size in MB");
    create_panel.add(&cp_size_input);

    let cp_mb_label = ui::Label::new("MB");
    cp_mb_label.set_position(162, 142);
    cp_mb_label.set_size(30, 16);
    cp_mb_label.set_font_size(12);
    cp_mb_label.set_color(tc.window_bg);
    cp_mb_label.set_text_color(0xFFCCCCCC);
    create_panel.add(&cp_mb_label);

    // Type selector
    let cp_type_label = ui::Label::new("Filesystem type:");
    cp_type_label.set_position(220, 136);
    cp_type_label.set_size(120, 16);
    cp_type_label.set_font_size(12);
    cp_type_label.set_color(tc.window_bg);
    cp_type_label.set_text_color(0xFFCCCCCC);
    create_panel.add(&cp_type_label);

    let cp_type_seg = ui::SegmentedControl::new("exFAT|FAT32|Linux");
    cp_type_seg.set_position(220, 156);
    cp_type_seg.set_size(220, 28);
    create_panel.add(&cp_type_seg);

    // Buttons
    let cp_btn_create = ui::Button::new("Create");
    cp_btn_create.set_position((cw - 24 - 190) as i32, 205);
    cp_btn_create.set_size(86, 32);
    create_panel.add(&cp_btn_create);

    let cp_btn_cancel = ui::Button::new("Cancel");
    cp_btn_cancel.set_position((cw - 24 - 96) as i32, 205);
    cp_btn_cancel.set_size(86, 32);
    create_panel.add(&cp_btn_cancel);

    // ── IDs for cross-closure access ──
    let create_panel_id = Widget::id(&create_panel);
    let grid_card_id = Widget::id(&grid_card);
    let cp_free_label_id = Widget::id(&cp_free_label);
    let cp_size_value_id = Widget::id(&cp_size_value);
    let cp_slider_id = Widget::id(&cp_slider);

    // ── State ──
    static mut DISK_IDX: usize = 0;
    static mut DISKS: Option<Vec<DiskInfo>> = None;
    static mut PARTS: Option<Vec<PartInfo>> = None;
    static mut DISPLAY_ROWS: Option<Vec<DisplayRow>> = None;
    static mut FREE_START: u64 = 0;
    static mut FREE_SECTORS: u64 = 0;

    let disks = load_disks();
    let parts = if !disks.is_empty() { load_partitions(disks[0].disk_id as u32) } else { Vec::new() };
    let mounts = load_mounts();
    let bar_w = cw - 32;

    update_sidebar(&disk_buttons, &disk_info_labels, &disks, &parts, 0, sidebar_bg, selected_bg);
    let drows = if !disks.is_empty() {
        build_display_rows(&disks[0], &parts, &mounts)
    } else {
        Vec::new()
    };
    update_ui(&info_label, &canvas, &grid, bar_w, BAR_H, &disks, 0, &parts, &drows);

    unsafe {
        DISKS = Some(disks);
        PARTS = Some(parts);
        DISPLAY_ROWS = Some(drows);
    }

    // ── Sidebar disk click handlers ──
    for i in 0..MAX_SIDEBAR_DISKS {
        disk_buttons[i].on_click(move |_| {
            unsafe {
                let d = DISKS.as_ref().unwrap();
                if i < d.len() {
                    DISK_IDX = i;
                    let p = load_partitions(d[i].disk_id as u32);
                    let m = load_mounts();
                    let drows = build_display_rows(&d[i], &p, &m);
                    update_sidebar(&disk_buttons, &disk_info_labels, d, &p, i, sidebar_bg, selected_bg);
                    update_ui(&info_label, &canvas, &grid, bar_w, BAR_H, d, i, &p, &drows);
                    DISPLAY_ROWS = Some(drows);
                    PARTS = Some(p);
                }
            }
        });
    }

    // ── Rescan ──
    let rescan = move || {
        unsafe {
            let new = load_disks();
            if DISK_IDX >= new.len() { DISK_IDX = 0; }
            let p = if !new.is_empty() {
                sys::partition_rescan(new[DISK_IDX].disk_id as u32);
                load_partitions(new[DISK_IDX].disk_id as u32)
            } else {
                Vec::new()
            };
            let m = load_mounts();
            let drows = if !new.is_empty() {
                build_display_rows(&new[DISK_IDX], &p, &m)
            } else {
                Vec::new()
            };
            DISKS = Some(new);
            let d = DISKS.as_ref().unwrap();
            update_sidebar(&disk_buttons, &disk_info_labels, d, &p, DISK_IDX, sidebar_bg, selected_bg);
            update_ui(&info_label, &canvas, &grid, bar_w, BAR_H, d, DISK_IDX, &p, &drows);
            DISPLAY_ROWS = Some(drows);
            PARTS = Some(p);
        }
    };
    btn_rescan.on_click(move |_| { rescan(); });

    // ── New partition: show create panel ──
    let show_create_panel = move || {
        unsafe {
            let disks = DISKS.as_ref().unwrap();
            let parts = PARTS.as_ref().unwrap();
            if disks.is_empty() { return; }
            if parts.len() >= 4 {
                ui::MessageBox::show(ui::MessageBoxType::Warning,
                    "Maximum 4 partitions (MBR limit).", None);
                return;
            }
            let disk = &disks[DISK_IDX];
            match find_free_space(disk, parts) {
                Some((start, size)) => {
                    FREE_START = start;
                    FREE_SECTORS = size;
                    let mb = size * 512 / (1024 * 1024);
                    ui::marshal_set_text(cp_free_label_id,
                        &format!("Free space: {} (starting at LBA {})", format_size(size), start));
                    ui::marshal_set_text(cp_size_value_id, &format!("{} MB", mb));
                    ui::marshal_set_state(cp_slider_id, 100);
                    cp_size_input.set_text(&format!("{}", mb));
                    ui::marshal_set_visible(grid_card_id, false);
                    ui::marshal_set_visible(create_panel_id, true);
                }
                None => {
                    ui::MessageBox::show(ui::MessageBoxType::Warning,
                        "No free space available on this disk.", None);
                }
            }
        }
    };
    btn_new.on_click(move |_| { show_create_panel(); });

    // ── Slider changes update size display ──
    cp_slider.on_value_changed(move |e| {
        unsafe {
            let max_mb = FREE_SECTORS * 512 / (1024 * 1024);
            let mb = (max_mb as u64 * e.value as u64 / 100).max(1);
            ui::marshal_set_text(cp_size_value_id, &format!("{} MB", mb));
            cp_size_input.set_text(&format!("{}", mb));
        }
    });

    // ── Create button ──
    cp_btn_create.on_click(move |_| {
        unsafe {
            let disks = DISKS.as_ref().unwrap();
            if disks.is_empty() { return; }
            let disk = &disks[DISK_IDX];

            let mut input_buf = [0u8; 32];
            let input_len = cp_size_input.get_text(&mut input_buf);
            let input_str = core::str::from_utf8(&input_buf[..input_len as usize]).unwrap_or("");
            let mb: u64 = input_str.trim().parse().unwrap_or(0);
            if mb == 0 {
                ui::MessageBox::show(ui::MessageBoxType::Warning,
                    "Enter a valid size in MB.", None);
                return;
            }

            let sectors = (mb * (1024 * 1024) / 512).min(FREE_SECTORS);
            if sectors == 0 { return; }

            let ptype: u8 = match cp_type_seg.get_state() {
                1 => 0x0C, // FAT32
                2 => 0x83, // Linux
                _ => 0x07, // exFAT/NTFS
            };

            // Find first free MBR slot (0-3)
            let parts = PARTS.as_ref().unwrap();
            let mut used_slots = [false; 4];
            for p in parts.iter() {
                if (p.index as usize) < 4 { used_slots[p.index as usize] = true; }
            }
            let slot = match used_slots.iter().position(|&u| !u) {
                Some(s) => s as u8,
                None => {
                    ui::MessageBox::show(ui::MessageBoxType::Warning,
                        "No free MBR partition slots (max 4).", None);
                    return;
                }
            };

            let mut entry = [0u8; 16];
            entry[0] = slot;          // partition index (0-3)
            entry[1] = ptype;         // filesystem type
            entry[2] = 0x01;          // bootable flag (1=yes, kernel checks != 0)
            write_le32_buf(&mut entry, 4, FREE_START as u32);
            write_le32_buf(&mut entry, 8, sectors as u32);
            sys::partition_create(disk.disk_id as u32, &entry);
            sys::partition_rescan(disk.disk_id as u32);

            let p = load_partitions(disk.disk_id as u32);
            let m = load_mounts();
            let drows = build_display_rows(disk, &p, &m);

            ui::marshal_set_visible(create_panel_id, false);
            ui::marshal_set_visible(grid_card_id, true);

            let d = DISKS.as_ref().unwrap();
            update_ui(&info_label, &canvas, &grid, bar_w, BAR_H, d, DISK_IDX, &p, &drows);
            DISPLAY_ROWS = Some(drows);
            PARTS = Some(p);
        }
    });

    // ── Cancel button ──
    cp_btn_cancel.on_click(move |_| {
        ui::marshal_set_visible(create_panel_id, false);
        ui::marshal_set_visible(grid_card_id, true);
    });

    // ── Delete partition ──
    let do_delete = move || {
        unsafe {
            let disks = DISKS.as_ref().unwrap();
            let parts = PARTS.as_ref().unwrap();
            let drows = DISPLAY_ROWS.as_ref().unwrap();
            if disks.is_empty() || parts.is_empty() { return; }
            let disk = &disks[DISK_IDX];
            let sel = grid.selected_row();
            if sel == u32::MAX || (sel as usize) >= drows.len() {
                ui::MessageBox::show(ui::MessageBoxType::Warning,
                    "Select a partition to delete.", None);
                return;
            }
            let row = &drows[sel as usize];
            if row.is_free {
                ui::MessageBox::show(ui::MessageBoxType::Warning,
                    "Cannot delete unallocated space.", None);
                return;
            }
            let pidx = match row.part_index {
                Some(i) => i,
                None => return,
            };

            let msg = format!(
                "Delete partition {} ({})?\n\nAll data will be permanently lost!",
                row.name, format_size(row.size_sectors)
            );
            ui::MessageBox::show(ui::MessageBoxType::Warning, &msg, Some("Delete"));

            sys::partition_delete(disk.disk_id as u32, pidx as u32);
            sys::partition_rescan(disk.disk_id as u32);

            let p = load_partitions(disk.disk_id as u32);
            let m = load_mounts();
            let dr = build_display_rows(disk, &p, &m);
            let d = DISKS.as_ref().unwrap();
            update_ui(&info_label, &canvas, &grid, bar_w, BAR_H, d, DISK_IDX, &p, &dr);
            DISPLAY_ROWS = Some(dr);
            PARTS = Some(p);
        }
    };
    btn_delete.on_click(move |_| { do_delete(); });

    // ── Format partition ──
    let do_format_exfat = move || {
        unsafe {
            let disks = DISKS.as_ref().unwrap();
            let drows = DISPLAY_ROWS.as_ref().unwrap();
            if disks.is_empty() { return; }
            let disk = &disks[DISK_IDX];
            let sel = grid.selected_row();
            if sel == u32::MAX || (sel as usize) >= drows.len() {
                ui::MessageBox::show(ui::MessageBoxType::Warning,
                    "Select a partition to format.", None);
                return;
            }
            let row = &drows[sel as usize];
            if row.is_free {
                ui::MessageBox::show(ui::MessageBoxType::Warning,
                    "Cannot format unallocated space. Create a partition first.", None);
                return;
            }
            let msg = format!(
                "Format {} ({}) as exFAT?\n\nAll data on this partition will be permanently erased!",
                row.name, format_size(row.size_sectors)
            );
            ui::MessageBox::show(ui::MessageBoxType::Warning, &msg, Some("Format as exFAT"));
            format_partition_exfat(disk.disk_id, row.start_lba as u32, row.size_sectors as u32);
            ui::MessageBox::show(ui::MessageBoxType::Info,
                "Partition formatted successfully as exFAT.", None);
        }
    };
    btn_format.on_click(move |_| { do_format_exfat(); });

    // ── Toggle boot flag ──
    let do_toggle_boot = move || {
        unsafe {
            let disks = DISKS.as_ref().unwrap();
            let parts = PARTS.as_ref().unwrap();
            let drows = DISPLAY_ROWS.as_ref().unwrap();
            if disks.is_empty() { return; }
            let disk = &disks[DISK_IDX];
            let sel = grid.selected_row();
            if sel == u32::MAX || (sel as usize) >= drows.len() {
                ui::MessageBox::show(ui::MessageBoxType::Warning,
                    "Select a partition.", None);
                return;
            }
            let row = &drows[sel as usize];
            if row.is_free { return; }
            let pidx = match row.part_index {
                Some(i) => i,
                None => return,
            };

            // Read MBR, toggle boot flag on partition entry, write back
            let mut mbr = [0u8; 512];
            sys::disk_read(disk.disk_id as u32, 0, 1, &mut mbr);
            let entry_off = 0x1BE + (pidx as usize) * 16;
            if entry_off + 16 <= 512 {
                mbr[entry_off] = if mbr[entry_off] == 0x80 { 0x00 } else { 0x80 };
                sys::disk_write(disk.disk_id as u32, 0, 1, &mbr);
            }
            sys::partition_rescan(disk.disk_id as u32);

            let p = load_partitions(disk.disk_id as u32);
            let m = load_mounts();
            let dr = build_display_rows(disk, &p, &m);
            let d = DISKS.as_ref().unwrap();
            update_ui(&info_label, &canvas, &grid, bar_w, BAR_H, d, DISK_IDX, &p, &dr);
            DISPLAY_ROWS = Some(dr);
            PARTS = Some(p);
        }
    };
    btn_boot.on_click(move |_| { do_toggle_boot(); });

    // ── Info button ──
    btn_info.on_click(move |_| {
        unsafe {
            let disks = DISKS.as_ref().unwrap();
            let drows = DISPLAY_ROWS.as_ref().unwrap();
            if disks.is_empty() { return; }
            let disk = &disks[DISK_IDX];
            let sel = grid.selected_row();
            if sel == u32::MAX || (sel as usize) >= drows.len() {
                ui::MessageBox::show(ui::MessageBoxType::Info,
                    &format!("Disk sd{}\nTotal: {}\nSectors: {}",
                        disk_letter(disk.disk_id), format_size(disk.total_sectors), disk.total_sectors),
                    None);
                return;
            }
            let row = &drows[sel as usize];
            let msg = if row.is_free {
                format!("Unallocated Space\n\nStart LBA: {}\nEnd LBA: {}\nSize: {} ({} sectors)",
                    row.start_lba, row.end_lba, format_size(row.size_sectors), row.size_sectors)
            } else {
                format!("{}\n\nFile System: {}\nFlags: {}\nMount: {}\nStart LBA: {}\nEnd LBA: {}\nSize: {} ({} sectors)",
                    row.name, row.fs_type,
                    if row.flags.is_empty() { "none" } else { row.flags.as_str() },
                    if row.mount_point.is_empty() { "not mounted" } else { row.mount_point.as_str() },
                    row.start_lba, row.end_lba, format_size(row.size_sectors), row.size_sectors)
            };
            ui::MessageBox::show(ui::MessageBoxType::Info, &msg, None);
        }
    });

    // ── Context menu handler ──
    ctx_menu.on_item_click(move |e| {
        match e.index {
            0 => show_create_panel(),    // New Partition
            1 => do_format_exfat(),      // Format as exFAT
            2 => {                       // Format as FAT32
                // TODO: Implement FAT32 formatting
                ui::MessageBox::show(ui::MessageBoxType::Info,
                    "FAT32 formatting is not yet implemented.", None);
            }
            3 => do_delete(),            // Delete Partition
            4 => do_toggle_boot(),       // Toggle Boot Flag
            5 => {                       // Partition Info
                unsafe {
                    let disks = DISKS.as_ref().unwrap();
                    let drows = DISPLAY_ROWS.as_ref().unwrap();
                    if disks.is_empty() { return; }
                    let disk = &disks[DISK_IDX];
                    let sel = grid.selected_row();
                    if sel == u32::MAX || (sel as usize) >= drows.len() { return; }
                    let row = &drows[sel as usize];
                    let msg = if row.is_free {
                        format!("Unallocated Space\n\nStart: {}\nEnd: {}\nSize: {}",
                            row.start_lba, row.end_lba, format_size(row.size_sectors))
                    } else {
                        format!("{}\nType: {}\nFlags: {}\nStart: {} End: {}\nSize: {}",
                            row.name, row.fs_type,
                            if row.flags.is_empty() { "none" } else { row.flags.as_str() },
                            row.start_lba, row.end_lba, format_size(row.size_sectors))
                    };
                    ui::MessageBox::show(ui::MessageBoxType::Info, &msg, None);
                }
            }
            _ => {}
        }
    });

    // ── Menu handler ──
    menu.on_item(move |e| {
        match e.item_id {
            1 | 20 => rescan(),
            2 => anyos_std::process::exit(0),
            10 => show_create_panel(),
            11 => do_delete(),
            12 => do_format_exfat(),
            13 => {
                ui::MessageBox::show(ui::MessageBoxType::Info,
                    "FAT32 formatting is not yet implemented.", None);
            }
            14 => do_toggle_boot(),
            15 => do_toggle_boot(), // same handler toggles
            _ => {}
        }
    });

    ui::run();
}

// ── Sidebar update ──────────────────────────────────────────────────────────

fn update_sidebar(
    buttons: &[ui::Button; MAX_SIDEBAR_DISKS],
    info_labels: &[ui::Label; MAX_SIDEBAR_DISKS],
    disks: &[DiskInfo], parts: &[PartInfo],
    selected: usize, normal_bg: u32, sel_bg: u32,
) {
    for i in 0..MAX_SIDEBAR_DISKS {
        if i < disks.len() {
            let d = &disks[i];
            let bg = if i == selected { sel_bg } else { normal_bg };
            buttons[i].set_color(bg);
            buttons[i].set_text(&format!("  sd{}", disk_letter(d.disk_id)));
            buttons[i].set_visible(true);

            // Show size and partition count for selected disk
            let pcount = if i == selected { parts.len() } else { 0 };
            let info_text = if i == selected {
                format!("{} \u{2022} {} partition(s)", format_size(d.total_sectors), pcount)
            } else {
                format_size(d.total_sectors)
            };
            info_labels[i].set_text(&info_text);
            info_labels[i].set_color(bg);
            info_labels[i].set_visible(true);
        } else {
            buttons[i].set_visible(false);
            info_labels[i].set_visible(false);
        }
    }
}

// ── Update main UI ──────────────────────────────────────────────────────────

fn update_ui(
    info: &ui::Label, canvas: &ui::Canvas, grid: &ui::DataGrid,
    bar_w: u32, bar_h: u32, disks: &[DiskInfo], idx: usize,
    parts: &[PartInfo], display_rows: &[DisplayRow],
) {
    if disks.is_empty() {
        canvas.clear(ui::theme::colors().window_bg);
        canvas.draw_text(10, 30, 0xFF999999, 0, 13, "No disks detected");
        grid.set_data(&[]);
        info.set_text("No disks detected");
        return;
    }

    let disk = &disks[idx];
    draw_bar(canvas, bar_w, bar_h, disk, display_rows);

    // Build grid data from display rows (partitions + unallocated)
    let cols = 7usize;
    let mut rows: Vec<[String; 7]> = Vec::new();
    for r in display_rows {
        rows.push([
            r.name.clone(),
            r.fs_type.clone(),
            if r.mount_point.is_empty() { String::new() } else { r.mount_point.clone() },
            r.flags.clone(),
            format_size(r.size_sectors),
            format!("{}", r.start_lba),
            format!("{}", r.end_lba),
        ]);
    }
    let refs: Vec<Vec<&str>> = rows.iter()
        .map(|r| r.iter().map(|s| s.as_str()).collect())
        .collect();
    grid.set_data(&refs);

    // Color coding: type color in column 1, gray text for unallocated rows
    let row_count = display_rows.len();
    let mut text_colors = vec![0u32; row_count * cols];
    let mut bg_colors = vec![0u32; row_count * cols];

    for (i, r) in display_rows.iter().enumerate() {
        // File system type column gets the type color
        text_colors[i * cols + 1] = type_color(r.ptype);

        if r.is_free {
            // Gray out all columns for unallocated rows
            for c in 0..cols {
                text_colors[i * cols + c] = 0xFF6E6E73;
            }
            // Subtle darker background for unallocated
            for c in 0..cols {
                bg_colors[i * cols + c] = 0xFF2A2A2E;
            }
        }
    }
    grid.set_cell_colors(&text_colors);
    grid.set_cell_bg_colors(&bg_colors);

    // Status bar
    let used: u64 = parts.iter().map(|p| p.size_sectors).sum();
    let free = disk.total_sectors.saturating_sub(used);
    info.set_text(&format!(
        "sd{} \u{2022} Total: {} \u{2022} Used: {} \u{2022} Free: {} \u{2022} {} partition(s)",
        disk_letter(disk.disk_id),
        format_size(disk.total_sectors),
        format_size(used),
        format_size(free),
        parts.len()
    ));
}

// ── ExFAT formatting ────────────────────────────────────────────────────────

const SECTOR_SIZE: u32 = 512;
const FAT_OFFSET: u32 = 32;
const SPC: u32 = 8;
const SPC_SHIFT: u8 = 3;

fn disk_write(disk_id: u8, lba: u32, data: &[u8; 512]) {
    sys::disk_write(disk_id as u32, lba as u64, 1, data);
}
fn write_cluster_data(disk_id: u8, fs_start: u32, heap_off: u32, cluster: u32, data: &[u8]) {
    let lba = fs_start + heap_off + (cluster - 2) * SPC;
    let sectors = (data.len() as u32 + SECTOR_SIZE - 1) / SECTOR_SIZE;
    for s in 0..sectors.min(SPC) {
        let off = (s * SECTOR_SIZE) as usize;
        let mut sector = [0u8; 512];
        let end = (off + 512).min(data.len());
        sector[..end - off].copy_from_slice(&data[off..end]);
        disk_write(disk_id, lba + s, &sector);
    }
}

fn format_partition_exfat(disk_id: u8, fs_start: u32, fs_sectors: u32) {
    let est_c = (fs_sectors - FAT_OFFSET) / SPC;
    let fl = ((est_c + 2) * 4 + SECTOR_SIZE - 1) / SECTOR_SIZE;
    let ho = FAT_OFFSET + fl;
    let cc = (fs_sectors - ho) / SPC;
    let fl = ((cc + 2) * 4 + SECTOR_SIZE - 1) / SECTOR_SIZE;
    let ho = FAT_OFFSET + fl;
    let rc: u32 = 4;

    let mut vbr = [0u8; 512];
    vbr[0] = 0xEB; vbr[1] = 0x76; vbr[2] = 0x90;
    vbr[3..11].copy_from_slice(b"EXFAT   ");
    write_le64_buf(&mut vbr, 64, fs_start as u64);
    write_le64_buf(&mut vbr, 72, fs_sectors as u64);
    write_le32_buf(&mut vbr, 80, FAT_OFFSET);
    write_le32_buf(&mut vbr, 84, fl);
    write_le32_buf(&mut vbr, 88, ho);
    write_le32_buf(&mut vbr, 92, cc);
    write_le32_buf(&mut vbr, 96, rc);
    write_le32_buf(&mut vbr, 100, 0x414E594F);
    write_le16_buf(&mut vbr, 104, 0x0100);
    vbr[108] = 9; vbr[109] = SPC_SHIFT; vbr[110] = 1; vbr[111] = 0x80; vbr[112] = 0xFF;
    vbr[510] = 0x55; vbr[511] = 0xAA;

    let mut ext = [0u8; 512];
    ext[510] = 0x55; ext[511] = 0xAA;
    let oem = [0u8; 512];
    let res = [0u8; 512];

    let mut ck: u32 = 0;
    let regs: [&[u8; 512]; 11] = [&vbr, &ext, &ext, &ext, &ext, &ext, &ext, &ext, &ext, &oem, &res];
    for (si, sec) in regs.iter().enumerate() {
        for (bi, &b) in sec.iter().enumerate() {
            let a = si * 512 + bi;
            if a == 106 || a == 107 || a == 112 { continue; }
            ck = ck.rotate_right(1).wrapping_add(b as u32);
        }
    }
    let mut cs = [0u8; 512];
    for i in 0..128 {
        write_le32_buf(&mut cs, i * 4, ck);
    }

    for base in [0u32, 12] {
        disk_write(disk_id, fs_start + base, &vbr);
        for i in 0..8u32 {
            disk_write(disk_id, fs_start + base + 1 + i, &ext);
        }
        disk_write(disk_id, fs_start + base + 9, &oem);
        disk_write(disk_id, fs_start + base + 10, &res);
        disk_write(disk_id, fs_start + base + 11, &cs);
    }

    let fa = fs_start + FAT_OFFSET;
    {
        let mut s = [0u8; 512];
        write_le32_buf(&mut s, 0, 0xFFFFFFF8);
        write_le32_buf(&mut s, 4, 0xFFFFFFFF);
        write_le32_buf(&mut s, 8, 0xFFFFFFFF);
        write_le32_buf(&mut s, 12, 0xFFFFFFFF);
        write_le32_buf(&mut s, 16, 0xFFFFFFFF);
        disk_write(disk_id, fa, &s);
    }
    {
        let n = (1u32 + 32).min(fl);
        let zb = [0u8; 32 * 512];
        if n > 1 {
            sys::disk_write(disk_id as u32, (fa + 1) as u64, n - 1, &zb[..((n - 1) as usize * 512)]);
        }
    }

    let csz = (SPC * SECTOR_SIZE) as usize;
    let mut up = vec![0u8; csz];
    for i in 0u16..128 {
        let u = if i >= 0x61 && i <= 0x7A { i - 0x20 } else { i };
        write_le16_buf(&mut up, i as usize * 2, u);
    }
    let ul: u32 = 256;
    let mut uc: u32 = 0;
    for i in 0..ul as usize {
        uc = uc.rotate_right(1).wrapping_add(up[i] as u32);
    }
    write_cluster_data(disk_id, fs_start, ho, 3, &up);

    let mut root = vec![0u8; csz];
    let bms = (cc + 7) / 8;
    root[0] = 0x81;
    write_le32_buf(&mut root, 20, 2);
    write_le64_buf(&mut root, 24, bms as u64);
    root[32] = 0x82;
    write_le32_buf(&mut root, 36, uc);
    write_le32_buf(&mut root, 52, 3);
    write_le64_buf(&mut root, 56, ul as u64);
    root[64] = 0x83;
    root[65] = 5;
    for (i, &c) in b"anyOS".iter().enumerate() {
        write_le16_buf(&mut root, 66 + i * 2, c as u16);
    }
    write_cluster_data(disk_id, fs_start, ho, 4, &root);

    let mut bm = vec![0u8; csz];
    bm[0] = 0x07;
    write_cluster_data(disk_id, fs_start, ho, 2, &bm);

    let zero = [0u8; 512];
    for s in 24..FAT_OFFSET {
        disk_write(disk_id, fs_start + s, &zero);
    }
}
