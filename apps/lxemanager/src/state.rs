use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use libanyui_client as ui;

pub const WIN_W: u32 = 820;
pub const WIN_H: u32 = 560;
pub const ACCENT: u32 = 0xFF007AFF;
pub const GREEN: u32 = 0xFF34C759;
pub const RED: u32 = 0xFFFF3B30;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ScreenMode {
    Welcome,
    Manage,
    Progress,
    Done,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Install = 1,
    Repair = 2,
    Delete = 3,
}

impl Operation {
    pub fn from_u32(value: u32) -> Self {
        match value {
            2 => Self::Repair,
            3 => Self::Delete,
            _ => Self::Install,
        }
    }
}

pub struct App {
    pub welcome: ui::View,
    pub manage: ui::View,
    pub progress: ui::View,
    pub done: ui::View,
    pub btn_back: ui::Button,
    pub btn_primary: ui::Button,
    pub btn_secondary: ui::Button,
    pub info_title: ui::Label,
    pub info_subtitle: ui::Label,
    pub info_root: ui::Label,
    pub info_files: ui::Label,
    pub info_packages: ui::Label,
    pub info_daemon: ui::Label,
    pub progress_bar: ui::ProgressBar,
    pub progress_title: ui::Label,
    pub progress_detail: ui::Label,
    pub progress_eta: ui::Label,
    pub done_title: ui::Label,
    pub done_detail: ui::Label,
    pub mode: ScreenMode,
    pub timer_id: u32,
    pub active_operation: Operation,
}

static mut APP: Option<App> = None;

pub fn app() -> &'static mut App {
    unsafe { APP.as_mut().unwrap() }
}

pub fn set_app(app: App) {
    unsafe {
        APP = Some(app);
    }
}

pub static WORKER_ACTIVE: AtomicBool = AtomicBool::new(false);
pub static WORKER_DONE: AtomicBool = AtomicBool::new(false);
pub static WORKER_ERROR: AtomicBool = AtomicBool::new(false);
pub static WORKER_OP: AtomicU32 = AtomicU32::new(0);
pub static PROGRESS_SEQ: AtomicU32 = AtomicU32::new(0);
pub static PROGRESS_PERCENT: AtomicU32 = AtomicU32::new(0);
pub static PROGRESS_STEP: AtomicU32 = AtomicU32::new(0);
pub static PROGRESS_STEPS: AtomicU32 = AtomicU32::new(1);
pub static PROGRESS_BYTES_DONE: AtomicU32 = AtomicU32::new(0);
pub static PROGRESS_BYTES_TOTAL: AtomicU32 = AtomicU32::new(0);
pub static PROGRESS_ETA: AtomicU32 = AtomicU32::new(0);
pub static PROGRESS_PHASE_LEN: AtomicU32 = AtomicU32::new(0);
pub static PROGRESS_DETAIL_LEN: AtomicU32 = AtomicU32::new(0);
pub static ERROR_LEN: AtomicU32 = AtomicU32::new(0);

const PHASE_BUF_LEN: usize = 128;
const DETAIL_BUF_LEN: usize = 192;
const ERROR_BUF_LEN: usize = 192;

pub static mut PROGRESS_PHASE_BUF: [u8; PHASE_BUF_LEN] = [0; PHASE_BUF_LEN];
pub static mut PROGRESS_DETAIL_BUF: [u8; DETAIL_BUF_LEN] = [0; DETAIL_BUF_LEN];
pub static mut ERROR_BUF: [u8; ERROR_BUF_LEN] = [0; ERROR_BUF_LEN];

pub fn reset_worker(op: Operation) {
    WORKER_OP.store(op as u32, Ordering::Release);
    WORKER_ACTIVE.store(true, Ordering::Release);
    WORKER_DONE.store(false, Ordering::Release);
    WORKER_ERROR.store(false, Ordering::Release);
    PROGRESS_SEQ.store(0, Ordering::Release);
    PROGRESS_PERCENT.store(0, Ordering::Release);
    PROGRESS_STEP.store(0, Ordering::Release);
    PROGRESS_STEPS.store(1, Ordering::Release);
    PROGRESS_BYTES_DONE.store(0, Ordering::Release);
    PROGRESS_BYTES_TOTAL.store(0, Ordering::Release);
    PROGRESS_ETA.store(0, Ordering::Release);
    PROGRESS_PHASE_LEN.store(0, Ordering::Release);
    PROGRESS_DETAIL_LEN.store(0, Ordering::Release);
    ERROR_LEN.store(0, Ordering::Release);
}

pub fn write_progress_text(phase: &str, detail: &str) {
    unsafe {
        let phase_len = phase.len().min(PHASE_BUF_LEN);
        PROGRESS_PHASE_BUF[..phase_len].copy_from_slice(&phase.as_bytes()[..phase_len]);
        PROGRESS_PHASE_LEN.store(phase_len as u32, Ordering::Release);

        let detail_len = detail.len().min(DETAIL_BUF_LEN);
        PROGRESS_DETAIL_BUF[..detail_len].copy_from_slice(&detail.as_bytes()[..detail_len]);
        PROGRESS_DETAIL_LEN.store(detail_len as u32, Ordering::Release);
    }
}

pub fn write_error(message: &str) {
    unsafe {
        let len = message.len().min(ERROR_BUF_LEN);
        ERROR_BUF[..len].copy_from_slice(&message.as_bytes()[..len]);
        ERROR_LEN.store(len as u32, Ordering::Release);
    }
}
