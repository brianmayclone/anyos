//! Installation worker thread: partitioning, formatting, bootloader, file copy.

use anyos_std::{format, String, Vec};
use anyos_std::{fs, sys};
use core::sync::atomic::Ordering;
use libanyui_client as ui;

use crate::state::*;
use crate::helpers::fix_case;

// ── Progress helpers (callable from worker thread via marshal) ──────────────

pub fn signal_copy_file(path: &str) {
    let len = path.len().min(255);
    unsafe { COPY_FILE_BUF[..len].copy_from_slice(&path.as_bytes()[..len]); }
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
    set_status(&format!(
        "Copying {}... ({}:{:02}, {}.{} MB/s)",
        label, secs / 60, secs % 60, kbs / 1024, (kbs % 1024) * 10 / 1024
    ));
}

// ── Main worker entry point ────────────────────────────────────────────────

/// Find the whole-disk device ID for a given disk_id.
fn find_whole_disk_dev(target_disk_id: u8) -> Option<u32> {
    let mut buf = [0u8; 64 * 32];
    let count = sys::disk_list(&mut buf);
    for i in 0..count as usize {
        let off = i * 64;
        if buf[off + 1] == target_disk_id && buf[off + 2] == 0xFF {
            return Some(buf[off] as u32);
        }
    }
    None
}

pub fn install_worker() {
    let dev_id = INSTALL_DISK_ID.load(Ordering::Acquire);
    let mode = INSTALL_MODE.load(Ordering::Acquire);

    let mut buf = [0u8; 64 * 32];
    let count = sys::disk_list(&mut buf);
    let mut disk_id: u8 = 0;
    let mut total_sectors: u64 = 0;
    for i in 0..count as usize {
        let off = i * 64;
        if buf[off] == dev_id as u8 {
            disk_id = buf[off + 1];
            total_sectors = u64::from_le_bytes([
                buf[off + 12], buf[off + 13], buf[off + 14], buf[off + 15],
                buf[off + 16], buf[off + 17], buf[off + 18], buf[off + 19],
            ]);
            break;
        }
    }

    if total_sectors == 0 {
        set_phase(99, "Error: Disk not found");
        set_status(&format!("Block device {} could not be read.", dev_id));
        WORKER_ERROR.store(true, Ordering::Release);
        WORKER_DONE.store(true, Ordering::Release);
        return;
    }

    anyos_std::println!("installer: dev_id={} disk_id={} total_sectors={}", dev_id, disk_id, total_sectors);

    if mode == 0 {
        // Whole-disk install: bootloader → partition table → format
        let fs_sectors = (total_sectors - PARTITION_START as u64) as u32;

        set_phase(1, "Installing bootloader...");
        set_progress(5);
        // Find the whole-disk device for this disk_id (we need the raw disk for MBR)
        let whole_disk_dev = find_whole_disk_dev(disk_id);
        let bl_dev = whole_disk_dev.unwrap_or(dev_id);
        if !install_bootloader(bl_dev) {
            set_phase(99, "Error: Bootloader not found");
            WORKER_ERROR.store(true, Ordering::Release);
            WORKER_DONE.store(true, Ordering::Release);
            return;
        }

        set_phase(2, "Creating partition table...");
        set_progress(10);
        create_partition(bl_dev, disk_id as u32, total_sectors);

        set_phase(3, "Formatting filesystem...");
        set_progress(15);
        format_exfat(bl_dev, PARTITION_START, fs_sectors);
    } else {
        // Partition install: format only (no bootloader, no partition table)
        set_phase(3, "Formatting partition...");
        set_progress(15);
        // Format the partition at offset 0 (relative to partition start)
        format_exfat(dev_id, 0, total_sectors as u32);
    }

    fs::sync();

    set_phase(4, "Mounting target filesystem...");
    set_progress(20);

    let part_id: u8;
    if mode == 0 {
        // Whole-disk: rescan to find the new partition
        anyos_std::println!("installer: partition_rescan(disk_id={})", disk_id);
        sys::partition_rescan(disk_id as u32);
        anyos_std::process::sleep(1000);

        let mut part_dev_id: Option<u8> = None;
        let mut buf2 = [0u8; 64 * 32];
        let count2 = sys::disk_list(&mut buf2);
        anyos_std::println!("installer: disk_list after rescan: {} devices", count2);
        for i in 0..count2 as usize {
            let off = i * 64;
            let d_id = buf2[off];
            let d_disk = buf2[off + 1];
            let d_part = buf2[off + 2];
            let d_size = u64::from_le_bytes([
                buf2[off + 12], buf2[off + 13], buf2[off + 14], buf2[off + 15],
                buf2[off + 16], buf2[off + 17], buf2[off + 18], buf2[off + 19],
            ]);
            anyos_std::println!("  dev={} disk={} part={:#04x} size={}", d_id, d_disk, d_part, d_size);
            if d_disk == disk_id && d_part != 0xFF {
                part_dev_id = Some(d_id);
            }
        }

        part_id = match part_dev_id {
            Some(id) => id,
            None => {
                set_phase(99, "Error: No partition found after format");
                set_status(&format!("Disk {} has no partition after rescan.", disk_id));
                WORKER_ERROR.store(true, Ordering::Release);
                WORKER_DONE.store(true, Ordering::Release);
                return;
            }
        };
    } else {
        // Partition install: we already have the device ID
        part_id = dev_id as u8;
    }

    let dev_str = format!("{}", part_id);
    if fs::mount("/mnt/target", &dev_str, FS_TYPE_EXFAT) != 0 {
        set_phase(99, "Error: Could not mount target filesystem");
        set_status(&format!("Mount failed for dev {}. exFAT format may be invalid.", dev_str));
        WORKER_ERROR.store(true, Ordering::Release);
        WORKER_DONE.store(true, Ordering::Release);
        return;
    }

    // Set root password on the live system BEFORE copying files.
    // This updates /System/users/passwd on disk, which then gets copied to the target.
    let pw = read_root_password();
    if !pw.is_empty() {
        set_phase(5, "Setting root password...");
        set_progress(22);
        let result = anyos_std::users::chpasswd("root", "", pw);
        if result != 0 {
            anyos_std::println!("installer: warning: chpasswd returned {}", result);
        }
        // Give the kernel time to flush the passwd file
        anyos_std::process::sleep(200);
    }

    set_phase(6, "Copying system files...");
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

    set_phase(7, "Finishing...");
    set_progress(98);

    // Write a clean boot.cfg WITHOUT the setup entry
    let clean_boot_cfg = concat!(
        "# anyOS Boot Configuration\n",
        "timeout=5\n",
        "default=0\n",
        "\n",
        "[anyOS]\n",
        "kernel=0\n",
        "description=anyOS with default settings\n",
        "\n",
        "[anyOS (Verbose)]\n",
        "kernel=0\n",
        "params=verbose\n",
        "description=anyOS with verbose kernel logging\n",
        "\n",
        "[anyOS (Textmode)]\n",
        "kernel=0\n",
        "params=nogui\n",
        "description=anyOS without compositor (text console login)\n",
        "\n",
        "[anyOS (Custom)]\n",
        "kernel=0\n",
        "params=custom\n",
        "description=anyOS with custom boot parameters\n",
    );
    let _ = fs::write_bytes("/mnt/target/boot/boot.cfg", clean_boot_cfg.as_bytes());

    fs::umount("/mnt/target");

    let total_bytes = COPY_BYTES_TOTAL.load(Ordering::Relaxed);
    let total_mb = total_bytes / (1024 * 1024);
    let avg_kbs = if copy_ms > 0 { (total_bytes as u64 * 1000) / (copy_ms as u64 * 1024) } else { 0 };

    set_phase(8, "Installation complete!");
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

// ── ExFAT formatting ───────────────────────────────────────────────────────

fn write_le16(buf: &mut [u8], off: usize, val: u16) {
    buf[off] = val as u8; buf[off + 1] = (val >> 8) as u8;
}

fn write_le32(buf: &mut [u8], off: usize, val: u32) {
    buf[off] = val as u8; buf[off + 1] = (val >> 8) as u8;
    buf[off + 2] = (val >> 16) as u8; buf[off + 3] = (val >> 24) as u8;
}

fn write_le64(buf: &mut [u8], off: usize, val: u64) {
    for i in 0..8 { buf[off + i] = (val >> (i * 8)) as u8; }
}

fn disk_write_sector(dev: u32, lba: u32, data: &[u8; 512]) {
    sys::disk_write(dev, lba as u64, 1, data);
}

fn write_cluster(dev: u32, fs_start: u32, heap_off: u32, cluster: u32, spc: u32, data: &[u8]) {
    let lba = fs_start + heap_off + (cluster - 2) * spc;
    let sectors = (data.len() as u32 + SECTOR_SIZE - 1) / SECTOR_SIZE;
    for s in 0..sectors.min(spc) {
        let off = (s * SECTOR_SIZE) as usize;
        let mut sector = [0u8; 512];
        let end = (off + 512).min(data.len());
        sector[..end - off].copy_from_slice(&data[off..end]);
        sys::disk_write(dev, (lba + s) as u64, 1, &sector);
    }
}

fn format_exfat(dev_id: u32, fs_start: u32, fs_sectors: u32) {
    let (spc, spc_shift): (u32, u8) = if fs_sectors > 512 * 1024 {
        (16, 4)
    } else {
        (SPC, SPC_SHIFT)
    };
    let cluster_size = spc * SECTOR_SIZE;

    let est_clusters = (fs_sectors - FAT_OFFSET) / spc;
    let fat_length = ((est_clusters + 2) * 4 + SECTOR_SIZE - 1) / SECTOR_SIZE;
    let cluster_heap_offset = FAT_OFFSET + fat_length;
    let cluster_count = (fs_sectors - cluster_heap_offset) / spc;
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
    vbr[108] = 9; vbr[109] = spc_shift; vbr[110] = 1; vbr[111] = 0x80; vbr[112] = 0xFF;
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
    {
        let clear_end = (1u32 + 32).min(fat_length);
        let zero_batch = [0u8; 32 * 512];
        if clear_end > 1 {
            let n = clear_end - 1;
            sys::disk_write(dev, (fat_abs + 1) as u64, n, &zero_batch[..(n as usize * 512)]);
        }
    }

    // Upcase table (cluster 3)
    let csz = cluster_size as usize;
    let mut upcase = anyos_std::vec![0u8; csz];
    for i in 0u16..128 {
        let u = if i >= 0x61 && i <= 0x7A { i - 0x20 } else { i };
        write_le16(&mut upcase, i as usize * 2, u);
    }
    let upcase_len: u32 = 256;
    let mut uc: u32 = 0;
    for i in 0..upcase_len as usize { uc = uc.rotate_right(1).wrapping_add(upcase[i] as u32); }
    write_cluster(dev, fs_start, cluster_heap_offset, 3, spc, &upcase);

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
    write_cluster(dev, fs_start, cluster_heap_offset, 4, spc, &root);

    // Bitmap (cluster 2)
    let mut bitmap = anyos_std::vec![0u8; csz];
    bitmap[0] = 0x07;
    write_cluster(dev, fs_start, cluster_heap_offset, 2, spc, &bitmap);

    // Zero padding
    let zero = [0u8; 512];
    for s in 24..FAT_OFFSET { disk_write_sector(dev, fs_start + s, &zero); }
}

// ── Bootloader ─────────────────────────────────────────────────────────────

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

// ── Partition table ────────────────────────────────────────────────────────

fn create_partition(dev_id: u32, disk_id: u32, total_sectors: u64) {
    let part_size = total_sectors - PARTITION_START as u64;

    let mut mbr = [0u8; 512];
    sys::disk_read(dev_id, 0, 1, &mut mbr);

    mbr[510] = 0x55;
    mbr[511] = 0xAA;

    let off = 446;
    mbr[off] = 0x80;
    mbr[off + 1] = 0xFE;
    mbr[off + 2] = 0xFF;
    mbr[off + 3] = 0xFF;
    mbr[off + 4] = 0x07;
    mbr[off + 5] = 0xFE;
    mbr[off + 6] = 0xFF;
    mbr[off + 7] = 0xFF;
    mbr[off + 8..off + 12].copy_from_slice(&(PARTITION_START).to_le_bytes());
    mbr[off + 12..off + 16].copy_from_slice(&(part_size as u32).to_le_bytes());

    for i in 1..4 {
        let o = 446 + i * 16;
        for b in &mut mbr[o..o + 16] { *b = 0; }
    }

    sys::disk_write(dev_id, 0, 1, &mbr);
}

// ── Recursive file copy ────────────────────────────────────────────────────

pub fn copy_recursive(src: &str, dst: &str, depth: u32) -> u32 {
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
