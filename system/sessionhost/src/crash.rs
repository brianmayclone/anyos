use anyos_std::prelude::*;

const CRASH_DIALOG_PATH: &str = "/System/CrashDialog.app/CrashDialog";

/// Mirrors `kernel/src/task/crash_info::CrashReport` layout.
#[repr(C)]
pub struct CrashReport {
    pub tid: u32,
    pub signal: u32,
    pub rip: u64,
    pub rsp: u64,
    pub rbp: u64,
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub cr2: u64,
    pub cs: u64,
    pub ss: u64,
    pub rflags: u64,
    pub err_code: u64,
    pub stack_frames: [u64; 16],
    pub num_frames: u32,
    pub name: [u8; 32],
    pub valid: bool,
}

pub fn launch_crash_dialog(tid: u32, exit_code: u32) {
    let mut buf = [0u8; core::mem::size_of::<CrashReport>()];
    let bytes = anyos_std::sys::get_crash_info(tid, &mut buf);

    if bytes > 0 {
        let report = unsafe { &*(buf.as_ptr() as *const CrashReport) };
        let name_len = report.name.iter().position(|&b| b == 0).unwrap_or(32);
        let name = core::str::from_utf8(&report.name[..name_len]).unwrap_or("unknown");
        let args = format!("{} {} {:x} {}", tid, report.signal, report.rip, name);
        anyos_std::process::spawn(CRASH_DIALOG_PATH, &args);
    } else {
        let args = format!("{} {} 0 unknown", tid, exit_code);
        anyos_std::process::spawn(CRASH_DIALOG_PATH, &args);
    }
}
