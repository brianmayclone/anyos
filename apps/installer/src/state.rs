//! Shared installer state: global app struct, atomic worker flags, copy-progress buffers.

use anyos_std::{String, Vec};
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use libanyui_client as ui;

// ── Worker thread atomics ──────────────────────────────────────────────────

pub static WORKER_ACTIVE: AtomicBool = AtomicBool::new(false);
pub static WORKER_DONE: AtomicBool = AtomicBool::new(false);
pub static WORKER_ERROR: AtomicBool = AtomicBool::new(false);
pub static WORKER_PROGRESS: AtomicU32 = AtomicU32::new(0);
pub static INSTALL_DISK_ID: AtomicU32 = AtomicU32::new(u32::MAX);
pub static INSTALL_MODE: AtomicU32 = AtomicU32::new(0); // 0=whole disk (partition+format), 1=existing partition (format only)
/// If installing to an existing partition, this is the partition's start LBA.
pub static INSTALL_PART_START: AtomicU32 = AtomicU32::new(0);

// Marshal IDs for cross-thread UI updates
pub static mut PROGRESS_BAR_ID: u32 = 0;
pub static mut STATUS_LABEL_ID: u32 = 0;
pub static mut PHASE_LABEL_ID: u32 = 0;
pub static mut BTN_REBOOT_ID: u32 = 0;

// Shared buffer: current file being copied (worker → UI thread)
pub static COPY_FILE_SEQ: AtomicU32 = AtomicU32::new(0);
pub static COPY_FILE_LEN: AtomicU32 = AtomicU32::new(0);
pub static mut COPY_FILE_BUF: [u8; 256] = [0u8; 256];

// Copy throughput tracking
pub static COPY_BYTES_TOTAL: AtomicU32 = AtomicU32::new(0);
pub static COPY_START_MS: AtomicU32 = AtomicU32::new(0);
pub static COPY_FILES_SINCE_UPDATE: AtomicU32 = AtomicU32::new(0);
pub static mut COPY_CURRENT_LABEL: [u8; 32] = [0u8; 32];
pub static COPY_CURRENT_LABEL_LEN: AtomicU32 = AtomicU32::new(0);

// Password chosen on the password page (worker reads after install starts)
pub static mut ROOT_PASSWORD: [u8; 128] = [0u8; 128];
pub static ROOT_PASSWORD_LEN: AtomicU32 = AtomicU32::new(0);

// ── Constants ──────────────────────────────────────────────────────────────

pub const SECTOR_SIZE: u32 = 512;
pub const PARTITION_START: u32 = 128;
pub const FAT_OFFSET: u32 = 32;
pub const SPC: u32 = 8;
pub const SPC_SHIFT: u8 = 3;
pub const CLUSTER_SIZE: u32 = SPC * SECTOR_SIZE;
pub const FS_TYPE_EXFAT: u32 = 7;

pub const WIN_W: u32 = 780;
pub const WIN_H: u32 = 520;

pub const ACCENT: u32 = 0xFF007AFF;
pub const ACCENT_DIM: u32 = 0xFF3A3A5C;
pub const STEP_DONE: u32 = 0xFF34C759;

// ── Disk entry ─────────────────────────────────────────────────────────────

pub struct DiskEntry {
    pub device_id: u8,
    pub disk_id: u8,
    /// None = whole disk, Some(n) = partition index n on this disk.
    pub partition_index: Option<u8>,
    pub size_sectors: u64,
    /// Number of partitions (only set for whole-disk entries).
    pub partition_count: u32,
    /// Disk model / device label from kernel (e.g. "QEMU HARDDISK").
    pub label: String,
    /// MBR partition type byte (0 for whole disks).
    pub part_type_id: u8,
}

// ── App state singleton ────────────────────────────────────────────────────

pub struct InstallerApp {
    pub win: ui::Window,
    // Pages: 0=Welcome, 1=License, 2=Select Disk, 3=Password, 4=Installing/Done
    pub page0: ui::View,
    pub page1: ui::View,
    pub page2: ui::View,
    pub page3: ui::View,  // Password setup
    pub page4: ui::View,  // Installing → Complete
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
    pub btn_details: ui::Button,
    pub details_card: ui::Card,
    pub details_log: ui::TextArea,
    pub details_text: String,
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
    pub details_visible: bool,
    pub last_copy_seq: u32,
    pub license_accepted: bool,
}

static mut APP: Option<InstallerApp> = None;

pub fn app() -> &'static mut InstallerApp {
    unsafe { APP.as_mut().unwrap() }
}

pub fn set_app(a: InstallerApp) {
    unsafe { APP = Some(a); }
}

/// Store the root password for the worker thread to pick up.
pub fn store_root_password(pw: &str) {
    let len = pw.len().min(127);
    unsafe { ROOT_PASSWORD[..len].copy_from_slice(&pw.as_bytes()[..len]); }
    ROOT_PASSWORD_LEN.store(len as u32, Ordering::Release);
}

/// Read back the stored root password (called from worker thread).
pub fn read_root_password() -> &'static str {
    let len = ROOT_PASSWORD_LEN.load(Ordering::Acquire) as usize;
    unsafe { core::str::from_utf8(&ROOT_PASSWORD[..len]).unwrap_or("") }
}
