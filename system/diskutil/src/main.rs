//! Disk Utility — GUI for viewing and managing disk partitions.
//!
//! Built with libanyui (anyOS native UI framework).

#![no_std]
#![no_main]

use libanyui_client as ui;
use ui::{Widget, ColumnDef, ALIGN_LEFT, ALIGN_RIGHT};
use anyos_std::{format, vec, String, Vec};
use anyos_std::sys;

anyos_std::entry!(main);

// ── Layout ──────────────────────────────────────────────────────────────────

const WIN_W: u32 = 560;
const WIN_H: u32 = 440;
const HEADER_H: u32 = 36;
const BAR_H: u32 = 48;
const LEGEND_H: u32 = 22;
const BTN_BAR_H: u32 = 36;

// ── Colors ──────────────────────────────────────────────────────────────────

const COL_EXFAT: u32 = 0xFF5856D6;   // Purple
const COL_FAT32: u32 = 0xFF34C759;   // Green
const COL_FAT16: u32 = 0xFFFF9500;   // Orange
const COL_NTFS: u32 = 0xFFAF52DE;    // Violet
const COL_LINUX: u32 = 0xFFFF3B30;   // Red
const COL_ESP: u32 = 0xFFFFCC00;     // Yellow
const COL_FREE: u32 = 0xFF555555;    // Gray

// ── Data ────────────────────────────────────────────────────────────────────

fn read_u64_le(buf: &[u8], off: usize) -> u64 {
    u64::from_le_bytes([
        buf[off], buf[off + 1], buf[off + 2], buf[off + 3],
        buf[off + 4], buf[off + 5], buf[off + 6], buf[off + 7],
    ])
}

fn type_name(t: u8) -> &'static str {
    match t {
        0x00 => "Empty",
        0x01 => "FAT12",
        0x04 | 0x06 | 0x0E => "FAT16",
        0x0B | 0x0C => "FAT32",
        0x07 => "NTFS/exFAT",
        0x82 => "Linux swap",
        0x83 => "Linux",
        0xEE => "GPT",
        0xEF => "EFI System",
        _ => "Unknown",
    }
}

fn type_color(t: u8) -> u32 {
    match t {
        0x07 => COL_EXFAT,
        0x0B | 0x0C => COL_FAT32,
        0x04 | 0x06 | 0x0E => COL_FAT16,
        0x83 | 0x82 => COL_LINUX,
        0xEF => COL_ESP,
        0x00 => COL_FREE,
        _ => 0xFF8E8E93,
    }
}

fn format_size(sectors: u64) -> String {
    let bytes = sectors * 512;
    if bytes >= 1024 * 1024 * 1024 {
        let gib = bytes / (1024 * 1024 * 1024);
        let frac = (bytes % (1024 * 1024 * 1024)) / (1024 * 1024 * 10);
        format!("{}.{} GiB", gib, frac)
    } else if bytes >= 1024 * 1024 {
        format!("{} MiB", bytes / (1024 * 1024))
    } else if bytes >= 1024 {
        format!("{} KiB", bytes / 1024)
    } else {
        format!("{} B", bytes)
    }
}

struct DiskInfo {
    disk_id: u8,
    total_sectors: u64,
}

struct PartInfo {
    index: u8,
    ptype: u8,
    bootable: bool,
    start_lba: u64,
    size_sectors: u64,
}

fn load_disks() -> Vec<DiskInfo> {
    let mut buf = [0u8; 32 * 16];
    let count = sys::disk_list(&mut buf);
    let mut disks = Vec::new();
    for i in 0..count as usize {
        let off = i * 32;
        let disk_id = buf[off + 1];
        let part = buf[off + 2];
        let size_sectors = read_u64_le(&buf, off + 16);
        if part == 0xFF {
            disks.push(DiskInfo { disk_id, total_sectors: size_sectors });
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
            start_lba: read_u64_le(&buf, off + 8),
            size_sectors: read_u64_le(&buf, off + 16),
        });
    }
    parts
}

// ── Draw the partition bar on a Canvas ──────────────────────────────────────

fn draw_partition_bar(canvas: &ui::Canvas, bar_w: u32, bar_h: u32, disks: &[DiskInfo], disk_idx: usize, parts: &[PartInfo]) {
    let tc = ui::theme::colors();
    canvas.clear(tc.toolbar_bg);

    if disks.is_empty() || parts.is_empty() {
        return;
    }

    let total = disks[disk_idx].total_sectors.max(1);
    let margin = 2u32;
    let inner_w = bar_w.saturating_sub(margin * 2) as u64;
    let inner_h = bar_h.saturating_sub(margin * 2);

    for part in parts {
        let px = margin as i32 + (part.start_lba * inner_w / total) as i32;
        let pw = ((part.size_sectors * inner_w / total) as u32).max(4);
        let color = type_color(part.ptype);
        canvas.fill_rect(px, margin as i32, pw, inner_h, color);
    }
}

// ── Legend row helper ────────────────────────────────────────────────────────

fn add_legend_item(row: &ui::FlowPanel, name: &str, color: u32) {
    let dot = ui::View::new();
    dot.set_size(10, 10);
    dot.set_color(color);
    dot.set_margin(0, 3, 4, 0);
    row.add(&dot);

    let lbl = ui::Label::new(name);
    lbl.set_size(60, 14);
    lbl.set_font_size(11);
    let tc = ui::theme::colors();
    lbl.set_color(tc.text_secondary);
    lbl.set_margin(0, 1, 10, 0);
    row.add(&lbl);
}

// ── Main ────────────────────────────────────────────────────────────────────

fn main() {
    if !ui::init() {
        anyos_std::println!("Failed to load libanyui.so");
        return;
    }

    let win = ui::Window::new("Disk Utility", -1, -1, WIN_W, WIN_H);

    // ── Header (DOCK_TOP) ──

    let header = ui::View::new();
    header.set_dock(ui::DOCK_TOP);
    header.set_size(WIN_W, HEADER_H);
    header.set_padding(12, 4, 12, 4);
    win.add(&header);

    let title = ui::Label::new("Disk Utility");
    let tc = ui::theme::colors();

    title.set_position(12, 8);
    title.set_font_size(16);
    title.set_color(tc.text);
    header.add(&title);

    let disk_label = ui::Label::new("Loading...");
    disk_label.set_position(150, 10);
    disk_label.set_size(240, 18);
    disk_label.set_font_size(13);
    disk_label.set_color(tc.text_secondary);
    header.add(&disk_label);

    let btn_prev = ui::Button::new("<");
    btn_prev.set_position(400, 4);
    btn_prev.set_size(28, 28);
    header.add(&btn_prev);

    let btn_next = ui::Button::new(">");
    btn_next.set_position(432, 4);
    btn_next.set_size(28, 28);
    header.add(&btn_next);

    // ── Partition bar (DOCK_TOP) ──

    let bar_wrap = ui::View::new();
    bar_wrap.set_dock(ui::DOCK_TOP);
    bar_wrap.set_size(WIN_W, BAR_H + 12);
    bar_wrap.set_padding(12, 4, 12, 8);
    win.add(&bar_wrap);

    let canvas = ui::Canvas::new(WIN_W - 24, BAR_H);
    canvas.set_position(12, 4);
    canvas.set_dock(ui::DOCK_FILL);
    bar_wrap.add(&canvas);

    // ── Legend (DOCK_TOP) ──

    let legend_wrap = ui::View::new();
    legend_wrap.set_dock(ui::DOCK_TOP);
    legend_wrap.set_size(WIN_W, LEGEND_H);
    legend_wrap.set_padding(12, 0, 12, 4);
    win.add(&legend_wrap);

    let legend = ui::FlowPanel::new();
    legend.set_position(12, 0);
    legend.set_size(WIN_W - 24, 18);
    legend_wrap.add(&legend);

    add_legend_item(&legend, "exFAT/NTFS", COL_EXFAT);
    add_legend_item(&legend, "FAT32", COL_FAT32);
    add_legend_item(&legend, "FAT16", COL_FAT16);
    add_legend_item(&legend, "Linux", COL_LINUX);
    add_legend_item(&legend, "EFI", COL_ESP);
    add_legend_item(&legend, "Free", COL_FREE);

    // ── Divider (DOCK_TOP) ──

    let div = ui::Divider::new();
    div.set_dock(ui::DOCK_TOP);
    div.set_size(WIN_W, 1);
    win.add(&div);

    // ── Bottom button bar (DOCK_BOTTOM) ──

    let btn_bar = ui::View::new();
    btn_bar.set_dock(ui::DOCK_BOTTOM);
    btn_bar.set_size(WIN_W, BTN_BAR_H);
    btn_bar.set_padding(12, 4, 12, 4);
    win.add(&btn_bar);

    let info_label = ui::Label::new("");
    info_label.set_position(12, 10);
    info_label.set_size(300, 16);
    info_label.set_font_size(11);
    info_label.set_color(tc.text_secondary);
    btn_bar.add(&info_label);

    let btn_rescan = ui::Button::new("Rescan");
    btn_rescan.set_position(356, 4);
    btn_rescan.set_size(80, 28);
    btn_bar.add(&btn_rescan);

    let btn_delete = ui::Button::new("Delete");
    btn_delete.set_position(444, 4);
    btn_delete.set_size(80, 28);
    btn_delete.set_color(tc.destructive);
    btn_bar.add(&btn_delete);

    // ── DataGrid (DOCK_FILL — takes remaining space) ──

    let grid = ui::DataGrid::new(WIN_W, 200);
    grid.set_dock(ui::DOCK_FILL);
    grid.set_row_height(24);
    grid.set_header_height(28);
    grid.set_columns(&[
        ColumnDef::new("Partition").width(80).align(ALIGN_LEFT),
        ColumnDef::new("Type").width(90).align(ALIGN_LEFT),
        ColumnDef::new("Boot").width(40).align(ALIGN_LEFT),
        ColumnDef::new("Start LBA").width(100).align(ALIGN_RIGHT),
        ColumnDef::new("Size").width(90).align(ALIGN_RIGHT),
        ColumnDef::new("End LBA").width(100).align(ALIGN_RIGHT),
    ]);
    win.add(&grid);

    // ── Load initial data ──

    let disks = load_disks();
    let parts = if !disks.is_empty() {
        load_partitions(disks[0].disk_id as u32)
    } else {
        Vec::new()
    };

    let bar_w = WIN_W - 24;
    update_ui(&disk_label, &info_label, &canvas, &grid, bar_w, BAR_H, &disks, 0, &parts);

    // ── State stored in static for callbacks ──
    // Safe: anyOS user programs are single-threaded.
    static mut DISK_IDX: usize = 0;
    static mut DISKS: Option<Vec<DiskInfo>> = None;
    static mut PARTS: Option<Vec<PartInfo>> = None;

    unsafe {
        DISKS = Some(disks);
        PARTS = Some(parts);
    }

    // ── Callbacks ──

    btn_prev.on_click(move |_| {
        unsafe {
            let disks = DISKS.as_ref().unwrap();
            if disks.len() > 1 {
                DISK_IDX = if DISK_IDX == 0 { disks.len() - 1 } else { DISK_IDX - 1 };
                let disk_id = disks[DISK_IDX].disk_id;
                PARTS = Some(load_partitions(disk_id as u32));
                update_ui(&disk_label, &info_label, &canvas, &grid, bar_w, BAR_H, disks, DISK_IDX, PARTS.as_ref().unwrap());
            }
        }
    });

    btn_next.on_click(move |_| {
        unsafe {
            let disks = DISKS.as_ref().unwrap();
            if disks.len() > 1 {
                DISK_IDX = (DISK_IDX + 1) % disks.len();
                let disk_id = disks[DISK_IDX].disk_id;
                PARTS = Some(load_partitions(disk_id as u32));
                update_ui(&disk_label, &info_label, &canvas, &grid, bar_w, BAR_H, disks, DISK_IDX, PARTS.as_ref().unwrap());
            }
        }
    });

    btn_rescan.on_click(move |_| {
        unsafe {
            let new_disks = load_disks();
            if DISK_IDX >= new_disks.len() {
                DISK_IDX = 0;
            }
            if !new_disks.is_empty() {
                let disk_id = new_disks[DISK_IDX].disk_id;
                sys::partition_rescan(disk_id as u32);
                PARTS = Some(load_partitions(disk_id as u32));
            } else {
                PARTS = Some(Vec::new());
            }
            DISKS = Some(new_disks);
            let disks = DISKS.as_ref().unwrap();
            update_ui(&disk_label, &info_label, &canvas, &grid, bar_w, BAR_H, disks, DISK_IDX, PARTS.as_ref().unwrap());
        }
    });

    btn_delete.on_click(move |_| {
        unsafe {
            let disks = DISKS.as_ref().unwrap();
            let parts = PARTS.as_ref().unwrap();
            if disks.is_empty() || parts.is_empty() {
                return;
            }
            // Delete the selected partition, or the last one if none selected
            let disk_id = disks[DISK_IDX].disk_id;
            let sel = grid.selected_row();
            let part_idx = if sel != u32::MAX && (sel as usize) < parts.len() {
                parts[sel as usize].index
            } else {
                parts[parts.len() - 1].index
            };
            sys::partition_delete(disk_id as u32, part_idx as u32);
            sys::partition_rescan(disk_id as u32);
            PARTS = Some(load_partitions(disk_id as u32));
            update_ui(&disk_label, &info_label, &canvas, &grid, bar_w, BAR_H, disks, DISK_IDX, PARTS.as_ref().unwrap());
        }
    });

    ui::run();
}

/// Update all dynamic UI elements.
fn update_ui(
    disk_label: &ui::Label,
    info_label: &ui::Label,
    canvas: &ui::Canvas,
    grid: &ui::DataGrid,
    bar_w: u32,
    bar_h: u32,
    disks: &[DiskInfo],
    disk_idx: usize,
    parts: &[PartInfo],
) {
    // Disk label
    if !disks.is_empty() {
        let disk = &disks[disk_idx];
        let size = format_size(disk.total_sectors);
        let text = format!("hd{} - {} ({}/{} disks)", disk.disk_id, size, disk_idx + 1, disks.len());
        disk_label.set_text(&text);
    } else {
        disk_label.set_text("No disks found");
    }

    // Partition bar
    draw_partition_bar(canvas, bar_w, bar_h, disks, disk_idx, parts);

    // DataGrid — set_data replaces all rows (no accumulation)
    let disk_id = if !disks.is_empty() { disks[disk_idx].disk_id } else { 0 };
    let mut row_strings: Vec<[String; 6]> = Vec::new();
    for part in parts {
        let name = format!("hd{}p{}", disk_id, part.index + 1);
        let tname = String::from(type_name(part.ptype));
        let boot = String::from(if part.bootable { "*" } else { "" });
        let start = format!("{}", part.start_lba);
        let size = format_size(part.size_sectors);
        let end = format!("{}", part.start_lba + part.size_sectors - 1);
        row_strings.push([name, tname, boot, start, size, end]);
    }

    let row_refs: Vec<Vec<&str>> = row_strings.iter().map(|row| {
        row.iter().map(|s| s.as_str()).collect()
    }).collect();

    grid.set_data(&row_refs);

    // Per-cell colors for the type column (col 1)
    if !parts.is_empty() {
        let col_count = 6usize;
        let mut colors = vec![0u32; parts.len() * col_count];
        for (i, part) in parts.iter().enumerate() {
            colors[i * col_count + 1] = type_color(part.ptype);
        }
        grid.set_cell_colors(&colors);
    }

    // Info label
    if !disks.is_empty() {
        let disk = &disks[disk_idx];
        let total_str = format_size(disk.total_sectors);
        let used: u64 = parts.iter().map(|p| p.size_sectors).sum();
        let used_str = format_size(used);
        let info = format!("Total: {} | Used: {} | {} partition(s)", total_str, used_str, parts.len());
        info_label.set_text(&info);
    } else {
        info_label.set_text("No disks detected");
    }
}
