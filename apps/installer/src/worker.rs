//! Installation worker thread: executes steps from install.ini script.

use anyos_std::format;
use anyos_std::{fs, sys, String};
use core::sync::atomic::Ordering;
use libanyui_client as ui;
use libinstall::{self, ApplyStats, PreflightCheck};

use crate::state::*;
use crate::script::{self, StepAction};
use crate::helpers::fix_case;

struct CopyStats {
    files: u32,
    dirs: u32,
    errors: u32,
    bytes: u64,
}

impl CopyStats {
    const fn new() -> Self {
        Self { files: 0, dirs: 0, errors: 0, bytes: 0 }
    }

    fn add(&mut self, other: Self) {
        self.files += other.files;
        self.dirs += other.dirs;
        self.errors += other.errors;
        self.bytes += other.bytes;
    }
}

// ── Log + progress helpers (worker thread → UI via marshal) ────────────────

// Current step label for log prefix (set before each step)
static mut STEP_PREFIX: [u8; 64] = [0u8; 64];
static mut STEP_PREFIX_LEN: usize = 0;

fn set_step_prefix(label: &str) {
    let len = label.len().min(63);
    unsafe {
        STEP_PREFIX[..len].copy_from_slice(&label.as_bytes()[..len]);
        STEP_PREFIX_LEN = len;
    }
}

fn step_prefix() -> &'static str {
    unsafe { core::str::from_utf8(&STEP_PREFIX[..STEP_PREFIX_LEN]).unwrap_or("") }
}

/// Append a line to the install log (written to file + shared buffer for UI).
/// Lines within a step are automatically prefixed with the step label.
fn log(line: &str) {
    let prefixed = format!("[{}] {}", step_prefix(), line);
    let bytes = prefixed.as_bytes();
    let len = bytes.len().min(511);
    unsafe { LOG_LINE_BUF[..len].copy_from_slice(&bytes[..len]); }
    LOG_LINE_LEN.store(len as u32, Ordering::Release);
    LOG_SEQ.fetch_add(1, Ordering::Release);
    append_log_file(&prefixed);
}

/// Log a line without step prefix (for top-level messages).
fn log_raw(line: &str) {
    let len = line.len().min(511);
    unsafe { LOG_LINE_BUF[..len].copy_from_slice(&line.as_bytes()[..len]); }
    LOG_LINE_LEN.store(len as u32, Ordering::Release);
    LOG_SEQ.fetch_add(1, Ordering::Release);
    append_log_file(line);
}

fn set_phase(text: &str) {
    unsafe { ui::marshal_set_text(PHASE_LABEL_ID, text); }
}

fn set_progress(pct: u32) {
    WORKER_PROGRESS.store(pct, Ordering::Release);
    unsafe { ui::marshal_set_state(PROGRESS_BAR_ID, pct); }
}

fn update_elapsed() {
    let start = INSTALL_START_MS.load(Ordering::Relaxed);
    let elapsed = sys::uptime_ms().wrapping_sub(start);
    let secs = elapsed / 1000;
    let text = format!("Elapsed: {}:{:02}", secs / 60, secs % 60);
    unsafe { ui::marshal_set_text(STATUS_LABEL_ID, &text); }
}

fn fail(msg: &str) {
    log_raw(&format!("FATAL: {}", msg));
    set_phase(msg);
    close_log_file();
    WORKER_ACTIVE.store(false, Ordering::Release);
    WORKER_ERROR.store(true, Ordering::Release);
    WORKER_DONE.store(true, Ordering::Release);
}

// Install log file handle (opened after mount, closed at finish)
static mut LOG_FD: u32 = u32::MAX;

fn open_log_file() {
    let fd = fs::open(
        "/mnt/target/install.log",
        fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC | fs::O_SYNC,
    );
    unsafe { LOG_FD = fd; }
}

fn append_log_file(line: &str) {
    let fd = unsafe { LOG_FD };
    if fd != u32::MAX {
        let mut buf = [0u8; 520];
        let len = line.len().min(512);
        buf[..len].copy_from_slice(&line.as_bytes()[..len]);
        buf[len] = b'\n';
        fs::write(fd, &buf[..len + 1]);
    }
}

fn close_log_file() {
    let fd = unsafe { LOG_FD };
    if fd != u32::MAX {
        let _ = fs::fsync(fd as i32);
        fs::close(fd);
        unsafe { LOG_FD = u32::MAX; }
    }
}

// ── Main worker entry point ────────────────────────────────────────────────

pub fn install_worker() {
    INSTALL_START_MS.store(sys::uptime_ms(), Ordering::Relaxed);

    let dev_id = INSTALL_DISK_ID.load(Ordering::Acquire);
    let mode = INSTALL_MODE.load(Ordering::Acquire);
    let operation = INSTALL_OPERATION.load(Ordering::Acquire);

    // Resolve disk info
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
        fail(&format!("Disk not found (device {})", dev_id));
        return;
    }

    log_raw(&format!("Target: dev={} disk={} sectors={} mode={} operation={}",
        dev_id, disk_id, total_sectors, mode, operation));

    if operation == 1 {
        if let Err(msg) = run_upgrade(dev_id, disk_id, mode) {
            fail(&msg);
            return;
        }
        set_progress(100);
        set_phase("Upgrade complete!");
        update_elapsed();
        unsafe { ui::marshal_set_visible(BTN_REBOOT_ID, true); }
        WORKER_DONE.store(true, Ordering::Release);
        WORKER_ACTIVE.store(false, Ordering::Release);
        return;
    }

    // Load install script
    let steps = script::load_steps();
    let total_steps = steps.len() as u32;
    log_raw(&format!("Install script: {} steps", total_steps));

    for (i, step) in steps.iter().enumerate() {
        let pct = ((i as u32) * 100) / total_steps;
        set_progress(pct);
        set_phase(&step.label);
        set_step_prefix(&step.label);
        log_raw(&format!("--- [{}/{}] {} ---", i + 1, total_steps, step.label));
        update_elapsed();

        match &step.action {
            StepAction::Bootloader => {
                if mode == 1 { log("  Skipped (partition install)"); continue; }
                let bl_dev = find_whole_disk_dev(disk_id).unwrap_or(dev_id);
                if !install_bootloader(bl_dev) {
                    fail("Bootloader not found (stage1.bin / stage2.bin)");
                    return;
                }
                log("  Bootloader written to MBR");
            }
            StepAction::Partition => {
                if mode == 1 { log("  Skipped (partition install)"); continue; }
                let bl_dev = find_whole_disk_dev(disk_id).unwrap_or(dev_id);
                create_partition(bl_dev, disk_id as u32, total_sectors);
                log("  Partition table created");
            }
            StepAction::Format => {
                if mode == 0 {
                    let bl_dev = find_whole_disk_dev(disk_id).unwrap_or(dev_id);
                    let fs_sectors = (total_sectors - PARTITION_START as u64) as u32;
                    format_exfat(bl_dev, PARTITION_START, fs_sectors);
                } else {
                    format_exfat(dev_id, 0, total_sectors as u32);
                }
                fs::sync();
                log("  Filesystem formatted (exFAT)");

                // Mount the target
                let part_id = mount_target(mode, dev_id, disk_id);
                if part_id.is_none() {
                    fail("Could not mount target filesystem");
                    return;
                }
                log(&format!("  Mounted /mnt/target (dev={})", part_id.unwrap()));

                // Open log file on the target partition
                open_log_file();
                log_raw("--- anyOS Installation Log ---");
            }
            StepAction::Password => {
                let pw = read_root_password();
                log("  Starting password configuration");
                if !pw.is_empty() {
                    let result = anyos_std::users::chpasswd("root", "", pw);
                    if result != 0 {
                        log(&format!("  Warning: chpasswd returned {}", result));
                    } else {
                        log("  Root password set");
                    }
                    anyos_std::process::sleep(200);
                } else {
                    log("  Skipped (no password set)");
                }
            }
            StepAction::Users => {
                let dirs = [
                    "/mnt/target/Users",
                    "/mnt/target/Users/Shared",
                    "/mnt/target/Users/Shared/www",
                    "/mnt/target/Users/settings",
                ];
                for dir in dirs {
                    let rc = fs::mkdir(dir);
                    if rc == 0 {
                        log(&format!("  Created {}", dir));
                    } else {
                        log(&format!("  {}", dir));
                    }
                }
            }
            StepAction::Extract { source, target } => {
                log(&format!("  Preparing target {}", target));
                fs::mkdir(target);
                if !libzip_client::init() {
                    log("  Warning: libzip not available, falling back to copy");
                    // Fallback: try copying from equivalent non-packed path
                    let fallback_src = source.replace("/install/", "/").replace(".tar.gz", "");
                    let mut stat = [0u32; 7];
                    if fs::stat(&fallback_src, &mut stat) == 0 {
                        let stats = copy_recursive(&fallback_src, target, 0);
                        log(&format!(
                            "  Copied {} files from {} ({} dirs, {} bytes, {} errors)",
                            stats.files, fallback_src, stats.dirs, stats.bytes, stats.errors
                        ));
                    } else {
                        log(&format!("  Warning: {} not found, skipped", fallback_src));
                    }
                    continue;
                }
                match libzip_client::TarReader::open(source) {
                    Some(reader) => {
                        let count = reader.entry_count();
                        log(&format!("  Extracting {} ({} entries)", source, count));
                        for j in 0..count {
                            let name = reader.entry_name(j);
                            let dest_path = format!("{}/{}", target, name);
                            if reader.entry_is_dir(j) {
                                fs::mkdir(&dest_path);
                            } else {
                                reader.extract_to_file(j, &dest_path);
                            }
                            log(&format!("  Extracted {}", dest_path));
                        }
                        log(&format!("  Extracted {} entries", count));
                    }
                    None => {
                        // Archive not found — try directory copy fallback
                        let fallback_src = source.replace("/install/", "/").replace(".tar.gz", "");
                        let mut stat = [0u32; 7];
                        if fs::stat(&fallback_src, &mut stat) == 0 {
                            let stats = copy_recursive(&fallback_src, target, 0);
                            log(&format!(
                                "  Archive not found, copied {} files from {} ({} dirs, {} bytes, {} errors)",
                                stats.files, fallback_src, stats.dirs, stats.bytes, stats.errors
                            ));
                        } else {
                            log(&format!("  Warning: {} not found, skipped", source));
                        }
                    }
                }
            }
            StepAction::Copy { source, target } => {
                let mut stat = [0u32; 7];
                if fs::stat(source, &mut stat) != 0 {
                    log(&format!("  Skipped (source {} not found)", source));
                    continue;
                }
                log(&format!("  Copying {} -> {}", source, target));
                fs::mkdir(target);
                let stats = copy_recursive(source, target, 0);
                log(&format!(
                    "  Finished {} -> {} ({} files, {} dirs, {} bytes, {} errors)",
                    source, target, stats.files, stats.dirs, stats.bytes, stats.errors
                ));
            }
            StepAction::BootCfg => {
                let cfg = concat!(
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
                let _ = fs::write_bytes("/mnt/target/boot/boot.cfg", cfg.as_bytes());
                log("  boot.cfg written");
            }
            StepAction::Finish => {
                let elapsed = sys::uptime_ms().wrapping_sub(
                    INSTALL_START_MS.load(Ordering::Relaxed));
                let secs = elapsed / 1000;
                log("Unmounting target filesystem");
                log_raw(&format!("=== Installation complete ({}:{:02}) ===", secs / 60, secs % 60));
                close_log_file();
                fs::umount("/mnt/target");
            }
        }
    }

    set_progress(100);
    set_phase("Installation complete!");
    update_elapsed();

    unsafe { ui::marshal_set_visible(BTN_REBOOT_ID, true); }
    WORKER_DONE.store(true, Ordering::Release);
    WORKER_ACTIVE.store(false, Ordering::Release);
}

fn run_upgrade(dev_id: u32, disk_id: u8, mode: u32) -> Result<(), String> {
    set_step_prefix("Upgrade");
    log_raw("--- [upgrade] Preparing target ---");

    let part_id = mount_target(mode, dev_id, disk_id)
        .ok_or_else(|| String::from("Could not mount target filesystem"))?;
    log(&format!("  Mounted /mnt/target (dev={})", part_id));
    open_log_file();
    log_raw("--- anyOS Upgrade Log ---");

    let existing = libinstall::detect_existing_installation("/mnt/target")
        .ok_or_else(|| String::from("No existing anyOS installation found on target"))?;
    log(&format!("  Existing anyOS version: {}", existing.version));
    if existing.has_users {
        log("  Preserving /Users data");
    }

    let list = libinstall::load_upgrade_list("/install/upgrade.json")
        .unwrap_or_else(libinstall::default_upgrade_list);
    log(&format!(
        "  UpgradeList: {} -> {} ({} operations)",
        list.from_version,
        list.to_version,
        list.operations.len()
    ));

    set_phase("Running safety checks");
    let preflight = libinstall::preflight_upgrade("/mnt/target", &list);
    if let Some(free) = preflight.available_bytes {
        log(&format!(
            "  Preflight: need about {} bytes, free {} bytes",
            preflight.estimated_required_bytes, free
        ));
    } else {
        log(&format!(
            "  Preflight: need about {} bytes, free space unknown",
            preflight.estimated_required_bytes
        ));
    }
    for issue in &preflight.issues {
        let level = match issue.level {
            PreflightCheck::Info => "info",
            PreflightCheck::Warning => "warning",
            PreflightCheck::Error => "error",
        };
        log(&format!("  Preflight {}: {}", level, issue.message));
    }
    if !preflight.ok {
        close_log_file();
        fs::umount("/mnt/target");
        return Err(String::from("Upgrade aborted by preflight checks"));
    }

    set_phase("Applying upgrade plan");
    let stats = libinstall::apply_upgrade_list("/mnt/target", &list)
        .map_err(|e| format!("Upgrade failed: {}", e))?;
    log_upgrade_stats(stats);

    log("Unmounting target filesystem");
    close_log_file();
    fs::umount("/mnt/target");
    Ok(())
}

fn log_upgrade_stats(stats: ApplyStats) {
    log(&format!(
        "  Upgrade applied: {} files copied, {} configs merged, {} preserved, {} dirs, {} errors",
        stats.files_copied,
        stats.configs_merged,
        stats.files_preserved,
        stats.dirs_created,
        stats.errors
    ));
}

// ── Disk helpers ───────────────────────────────────────────────────────────

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

fn mount_target(mode: u32, dev_id: u32, disk_id: u8) -> Option<u8> {
    if mode == 0 {
        for attempt in 0..5 {
            log(&format!("  Rescanning partitions (attempt {}/5)", attempt + 1));
            sys::partition_rescan(disk_id as u32);
            anyos_std::process::sleep(700 + attempt * 300);
            let mut buf = [0u8; 64 * 32];
            let count = sys::disk_list(&mut buf);
            for i in 0..count as usize {
                let off = i * 64;
                if buf[off + 1] == disk_id && buf[off + 2] != 0xFF {
                    let pid = buf[off];
                    let dev_str = format!("{}", pid);
                    let mount_rc = fs::mount("/mnt/target", &dev_str, FS_TYPE_EXFAT);
                    if mount_rc == 0 {
                        return Some(pid);
                    }
                    log(&format!("  Mount attempt failed for device {} (rc={})", pid, mount_rc));
                }
            }
        }
        None
    } else {
        let dev_str = format!("{}", dev_id as u8);
        let mount_rc = fs::mount("/mnt/target", &dev_str, FS_TYPE_EXFAT);
        if mount_rc == 0 {
            Some(dev_id as u8)
        } else {
            log(&format!("  Mount failed for device {} (rc={})", dev_id, mount_rc));
            None
        }
    }
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
    let (spc, spc_shift): (u32, u8) = if fs_sectors > 512 * 1024 { (16, 4) } else { (SPC, SPC_SHIFT) };
    let cluster_size = spc * SECTOR_SIZE;
    let est_clusters = (fs_sectors - FAT_OFFSET) / spc;
    let fat_length = ((est_clusters + 2) * 4 + SECTOR_SIZE - 1) / SECTOR_SIZE;
    let cluster_heap_offset = FAT_OFFSET + fat_length;
    let cluster_count = (fs_sectors - cluster_heap_offset) / spc;
    let fat_length = ((cluster_count + 2) * 4 + SECTOR_SIZE - 1) / SECTOR_SIZE;
    let cluster_heap_offset = FAT_OFFSET + fat_length;
    let root_cluster: u32 = 4;
    let dev = dev_id;

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

    let fat_abs = fs_start + FAT_OFFSET;
    { let mut s = [0u8; 512];
      write_le32(&mut s, 0, 0xFFFFFFF8); write_le32(&mut s, 4, 0xFFFFFFFF);
      write_le32(&mut s, 8, 0xFFFFFFFF); write_le32(&mut s, 12, 0xFFFFFFFF);
      write_le32(&mut s, 16, 0xFFFFFFFF); disk_write_sector(dev, fat_abs, &s); }
    { let clear_end = (1u32 + 32).min(fat_length);
      let zero_batch = [0u8; 32 * 512];
      if clear_end > 1 { let n = clear_end - 1;
        sys::disk_write(dev, (fat_abs + 1) as u64, n, &zero_batch[..(n as usize * 512)]); } }

    let csz = cluster_size as usize;
    let mut upcase = anyos_std::vec![0u8; csz];
    for i in 0u16..128 { let u = if i >= 0x61 && i <= 0x7A { i - 0x20 } else { i };
        write_le16(&mut upcase, i as usize * 2, u); }
    let upcase_len: u32 = 256;
    let mut uc: u32 = 0;
    for i in 0..upcase_len as usize { uc = uc.rotate_right(1).wrapping_add(upcase[i] as u32); }
    write_cluster(dev, fs_start, cluster_heap_offset, 3, spc, &upcase);

    let mut root = anyos_std::vec![0u8; csz];
    let bitmap_size = (cluster_count + 7) / 8;
    root[0] = 0x81; write_le32(&mut root, 20, 2); write_le64(&mut root, 24, bitmap_size as u64);
    root[32] = 0x82; write_le32(&mut root, 36, uc); write_le32(&mut root, 52, 3);
    write_le64(&mut root, 56, upcase_len as u64);
    root[64] = 0x83; root[65] = 5;
    for (i, &ch) in b"anyOS".iter().enumerate() { write_le16(&mut root, 66 + i * 2, ch as u16); }
    write_cluster(dev, fs_start, cluster_heap_offset, 4, spc, &root);

    let mut bitmap = anyos_std::vec![0u8; csz]; bitmap[0] = 0x07;
    write_cluster(dev, fs_start, cluster_heap_offset, 2, spc, &bitmap);

    let zero = [0u8; 512];
    for s in 24..FAT_OFFSET { disk_write_sector(dev, fs_start + s, &zero); }
}

// ── Bootloader ─────────────────────────────────────────────────────────────

fn install_bootloader(dev_id: u32) -> bool {
    let mut stage1 = [0u8; 512];
    let s1 = fs::open("/boot/stage1.bin", 0);
    if s1 == u32::MAX { return false; }
    fs::read(s1, &mut stage1); fs::close(s1);

    let s2 = fs::open("/boot/stage2.bin", 0);
    if s2 == u32::MAX { return false; }
    let mut stage2 = anyos_std::vec![0u8; 63 * 512];
    let mut total = 0usize;
    loop { let n = fs::read(s2, &mut stage2[total..]); if n == 0 || n == u32::MAX { break; } total += n as usize; }
    fs::close(s2);
    if total == 0 { return false; }

    let mut mbr = [0u8; 512];
    sys::disk_read(dev_id, 0, 1, &mut mbr);
    mbr[..440].copy_from_slice(&stage1[..440]);
    mbr[510] = 0x55; mbr[511] = 0xAA;
    sys::disk_write(dev_id, 0, 1, &mbr);

    for s in 0..(total + 511) / 512 {
        let off = s * 512; let mut sector = [0u8; 512];
        let end = (off + 512).min(total);
        sector[..end - off].copy_from_slice(&stage2[off..end]);
        sys::disk_write(dev_id, (1 + s) as u64, 1, &sector);
    }
    true
}

// ── Partition table ────────────────────────────────────────────────────────

fn create_partition(dev_id: u32, _disk_id: u32, total_sectors: u64) {
    let part_size = total_sectors - PARTITION_START as u64;
    let mut mbr = [0u8; 512];
    sys::disk_read(dev_id, 0, 1, &mut mbr);
    mbr[510] = 0x55; mbr[511] = 0xAA;
    let off = 446;
    mbr[off] = 0x80; mbr[off+1] = 0xFE; mbr[off+2] = 0xFF; mbr[off+3] = 0xFF;
    mbr[off+4] = 0x07; mbr[off+5] = 0xFE; mbr[off+6] = 0xFF; mbr[off+7] = 0xFF;
    mbr[off+8..off+12].copy_from_slice(&(PARTITION_START).to_le_bytes());
    mbr[off+12..off+16].copy_from_slice(&(part_size as u32).to_le_bytes());
    for i in 1..4 { let o = 446 + i * 16; for b in &mut mbr[o..o+16] { *b = 0; } }
    sys::disk_write(dev_id, 0, 1, &mbr);
}

// ── Recursive file copy ────────────────────────────────────────────────────

fn copy_recursive(src: &str, dst: &str, depth: u32) -> CopyStats {
    let mut stats = CopyStats::new();
    if depth > 32 {
        log(&format!("  Warning: max recursion depth reached at {}", src));
        stats.errors += 1;
        return stats;
    }
    let mut buf = [0u8; 256 * 64];
    let count = fs::readdir(src, &mut buf);
    if count == u32::MAX {
        log(&format!("  Warning: could not read directory {}", src));
        stats.errors += 1;
        return stats;
    }
    log(&format!("  Scanning {} ({} entries)", src, count));
    for i in 0..count as usize {
        let off = i * 64;
        let entry_type = buf[off];
        let name_len = buf[off + 1] as usize;
        if name_len == 0 || name_len > 56 { continue; }
        let name = match core::str::from_utf8(&buf[off + 8..off + 8 + name_len]) {
            Ok(s) => s, Err(_) => continue,
        };
        if name == "." || name == ".." { continue; }
        if depth == 0 && (name == "src" || name == "apps" || name == "install") { continue; }
        let fixed = fix_case(name);
        let child_src = format!("{}/{}", src, name);
        let child_dst = format!("{}/{}", dst, fixed);
        if entry_type == 1 {
            fs::mkdir(&child_dst);
            stats.dirs += 1;
            stats.add(copy_recursive(&child_src, &child_dst, depth + 1));
        } else if entry_type == 2 {
            log(&format!("  Skipping symlink {}", child_src));
        } else {
            match copy_file(&child_src, &child_dst) {
                Ok(bytes) => {
                    stats.files += 1;
                    stats.bytes += bytes;
                    log(&format!("  Copied {} ({} bytes)", child_dst, bytes));
                }
                Err(()) => {
                    stats.errors += 1;
                    log(&format!("  Warning: failed to copy {}", child_src));
                }
            }
        }
    }
    stats
}

fn copy_file(src: &str, dst: &str) -> Result<u64, ()> {
    let src_fd = fs::open(src, 0);
    if src_fd == u32::MAX {
        return Err(());
    }
    let dst_fd = fs::open(dst, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
    if dst_fd == u32::MAX {
        fs::close(src_fd);
        return Err(());
    }

    let mut total = 0u64;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = fs::read(src_fd, &mut buf);
        if n == 0 {
            break;
        }
        if n == u32::MAX {
            fs::close(src_fd);
            fs::close(dst_fd);
            return Err(());
        }
        let chunk = &buf[..n as usize];
        let wrote = fs::write(dst_fd, chunk);
        if wrote == u32::MAX || wrote as usize != chunk.len() {
            fs::close(src_fd);
            fs::close(dst_fd);
            return Err(());
        }
        total += n as u64;
    }

    let _ = fs::fsync(dst_fd as i32);
    fs::close(src_fd);
    fs::close(dst_fd);
    Ok(total)
}
