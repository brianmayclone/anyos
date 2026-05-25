use anyos_std::prelude::*;

const CRASH_DIALOG_PATH: &str = "/System/CrashDialog.app";
const CRASH_DIALOG_ARG0: &str = "CrashDialog";

/// Mirrors `kernel/src/task/crash_info::CrashReport` layout.
#[repr(C)]
struct CrashReport {
    tid: u32,
    signal: u32,
    rip: u64,
    rsp: u64,
    rbp: u64,
    rax: u64,
    rbx: u64,
    rcx: u64,
    rdx: u64,
    rsi: u64,
    rdi: u64,
    r8: u64,
    r9: u64,
    r10: u64,
    r11: u64,
    r12: u64,
    r13: u64,
    r14: u64,
    r15: u64,
    cr2: u64,
    cs: u64,
    ss: u64,
    rflags: u64,
    err_code: u64,
    stack_frames: [u64; 16],
    num_frames: u32,
    name: [u8; 32],
    valid: bool,
}

fn crash_log_path(tid: u32) -> String {
    let uptime = anyos_std::sys::uptime_ms();
    let mut random = [0u8; 8];
    let got_random = anyos_std::sys::random(&mut random);
    let mut nonce = 0u64;
    for byte in random {
        nonce = (nonce << 8) | byte as u64;
    }
    if got_random == 0 || nonce == 0 {
        nonce = ((uptime as u64) << 32) ^ tid as u64;
    }
    format!(
        "/tmp/crashlog_{:08x}_{:08x}_{:016x}.tmp",
        tid, uptime, nonce
    )
}

pub fn launch_crash_dialog(tid: u32, signal: u32) {
    let mut report = [0u8; core::mem::size_of::<CrashReport>()];
    let bytes_read = anyos_std::sys::get_crash_info(tid, &mut report) as usize;
    if bytes_read < report.len() {
        report[0..4].copy_from_slice(&tid.to_le_bytes());
        report[4..8].copy_from_slice(&signal.to_le_bytes());
        anyos_std::println!(
            "[sessionhost/crash] no full crash report for tid={} signal={}",
            tid,
            signal
        );
    }

    let path = crash_log_path(tid);
    let _ = anyos_std::fs::mkdir("/tmp");
    match anyos_std::fs::write_bytes(&path, &report) {
        Ok(()) => {
            anyos_std::println!("[sessionhost/crash] wrote crash report '{}'", path);
        }
        Err(_) => {
            anyos_std::println!(
                "[sessionhost/crash] failed to write crash report '{}'",
                path
            );
        }
    }
    let args = format!("{} {}", CRASH_DIALOG_ARG0, path);
    anyos_std::println!("[sessionhost/crash] launching crashdialog args='{}'", args);
    anyos_std::process::spawn(CRASH_DIALOG_PATH, &args);
}
