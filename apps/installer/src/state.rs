//! Shared installer state: global app struct, atomic worker flags.

use anyos_std::{String, Vec};
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use libanyui_client as ui;

// ── Worker thread atomics ──────────────────────────────────────────────────

pub static WORKER_ACTIVE: AtomicBool = AtomicBool::new(false);
pub static WORKER_DONE: AtomicBool = AtomicBool::new(false);
pub static WORKER_ERROR: AtomicBool = AtomicBool::new(false);
pub static WORKER_PROGRESS: AtomicU32 = AtomicU32::new(0);
pub static INSTALL_DISK_ID: AtomicU32 = AtomicU32::new(u32::MAX);
pub static INSTALL_MODE: AtomicU32 = AtomicU32::new(0); // 0=whole disk, 1=existing partition

/// System-filesystem layout chosen by the user on Page 2.
///
///  * `SYSTEM_FS_EXFAT` (0, default) — classic single-partition install:
///    one exFAT partition spanning the whole disk.
///  * `SYSTEM_FS_COREFS` (1) — dual-partition install: exFAT boot partition
///    plus a CoreFS system partition at the tail of the disk, sized by
///    [`SYSTEM_FS_COREFS_SIZE_MIB`].  Matches the layout the image-build
///    pipeline produces with `cmake -DANYOS_SYSTEM_FS=corefs`.
pub static SYSTEM_FS: AtomicU32 = AtomicU32::new(0);
pub const SYSTEM_FS_EXFAT: u32 = 0;
pub const SYSTEM_FS_COREFS: u32 = 1;

/// Size (MiB) of the CoreFS system partition in dual-partition mode.
/// Only read when `SYSTEM_FS == SYSTEM_FS_COREFS`.  Default: 128 MiB.
pub static SYSTEM_FS_COREFS_SIZE_MIB: AtomicU32 = AtomicU32::new(128);

// Marshal IDs for cross-thread UI updates
pub static mut PROGRESS_BAR_ID: u32 = 0;
pub static mut STATUS_LABEL_ID: u32 = 0;
pub static mut PHASE_LABEL_ID: u32 = 0;
pub static mut BTN_REBOOT_ID: u32 = 0;

// Installation start time (for elapsed display)
pub static INSTALL_START_MS: AtomicU32 = AtomicU32::new(0);

// Install log: worker appends lines, UI reads via marshal
pub static LOG_SEQ: AtomicU32 = AtomicU32::new(0);
pub static LOG_LINE_LEN: AtomicU32 = AtomicU32::new(0);
pub static mut LOG_LINE_BUF: [u8; 512] = [0u8; 512];

// Password chosen on the password page (worker reads after install starts)
pub static mut ROOT_PASSWORD: [u8; 128] = [0u8; 128];
pub static ROOT_PASSWORD_LEN: AtomicU32 = AtomicU32::new(0);

// ── Constants ──────────────────────────────────────────────────────────────

pub const SECTOR_SIZE: u32 = 512;
pub const PARTITION_START: u32 = 128;
pub const FAT_OFFSET: u32 = 32;
pub const SPC: u32 = 8;
pub const SPC_SHIFT: u8 = 3;
pub const FS_TYPE_EXFAT: u32 = 7;

/// MBR partition-type byte for CoreFS volumes (matches
/// `kernel/src/fs/partition.rs:115`).
pub const FS_TYPE_COREFS: u32 = 0xCF;

pub const WIN_W: u32 = 780;
pub const WIN_H: u32 = 520;

pub const ACCENT: u32 = 0xFF007AFF;

// ── Disk entry ─────────────────────────────────────────────────────────────

pub struct DiskEntry {
    pub device_id: u8,
    pub disk_id: u8,
    pub partition_index: Option<u8>,
    pub size_sectors: u64,
    pub partition_count: u32,
    pub label: String,
    pub part_type_id: u8,
}

// ── App state singleton ────────────────────────────────────────────────────

pub struct InstallerApp {
    pub win: ui::Window,
    // Pages: 0=Welcome, 1=License, 2=Select Disk, 3=Password, 4=Installing/Done
    pub page0: ui::View,
    pub page1: ui::View,
    pub page2: ui::View,
    pub page3: ui::View,
    pub page4: ui::View,
    // Page 2 (disk selection)
    pub disk_grid: ui::DataGrid,
    pub disk_container_id: u32,
    pub disk_card_ids: Vec<u32>,
    // Page 3 (password)
    pub pw_field1: ui::TextField,
    pub pw_field2: ui::TextField,
    pub pw_error_label: ui::Label,
    // Page 4 (progress + completion)
    pub progress_bar: ui::ProgressBar,
    pub phase_label: ui::Label,
    pub status_label: ui::Label,
    pub btn_reboot: ui::Button,
    pub progress_card_id: u32,
    pub complete_label_id: u32,
    pub complete_sub_id: u32,
    // Bottom bar buttons
    pub btn_back_id: u32,
    pub btn_next_id: u32,
    // State
    pub disks: Vec<DiskEntry>,
    pub selected_disk: Option<usize>,
    pub current_step: u32,
    pub timer_id: u32,
    pub last_log_seq: u32,
    pub license_accepted: bool,
    // Install log (full text, shown in log window)
    pub log_text: String,
    pub log_win: Option<ui::Window>,
    pub log_textarea: Option<ui::TextArea>,
}

static mut APP: Option<InstallerApp> = None;

pub fn app() -> &'static mut InstallerApp {
    unsafe { APP.as_mut().unwrap() }
}

pub fn set_app(a: InstallerApp) {
    unsafe { APP = Some(a); }
}

pub fn store_root_password(pw: &str) {
    let len = pw.len().min(127);
    unsafe { ROOT_PASSWORD[..len].copy_from_slice(&pw.as_bytes()[..len]); }
    ROOT_PASSWORD_LEN.store(len as u32, Ordering::Release);
}

pub fn read_root_password() -> &'static str {
    let len = ROOT_PASSWORD_LEN.load(Ordering::Acquire) as usize;
    unsafe { core::str::from_utf8(&ROOT_PASSWORD[..len]).unwrap_or("") }
}
