//! Crash report storage for user-space thread faults.
//!
//! When a user thread crashes (SIGSEGV, SIGILL, etc.), the ISR handler stores a
//! `CrashReport` in a fixed-size ring buffer. The compositor (or any process) can
//! later retrieve this report via `SYS_GET_CRASH_INFO` to display a crash dialog.

use crate::sync::spinlock::Spinlock;

/// Maximum number of crash reports retained (ring buffer).
const MAX_REPORTS: usize = 8;

/// Crash report containing register state and stack trace at time of fault.
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

impl CrashReport {
    const fn empty() -> Self {
        CrashReport {
            tid: 0,
            signal: 0,
            rip: 0,
            rsp: 0,
            rbp: 0,
            rax: 0,
            rbx: 0,
            rcx: 0,
            rdx: 0,
            rsi: 0,
            rdi: 0,
            r8: 0,
            r9: 0,
            r10: 0,
            r11: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
            cr2: 0,
            cs: 0,
            ss: 0,
            rflags: 0,
            err_code: 0,
            stack_frames: [0; 16],
            num_frames: 0,
            name: [0; 32],
            valid: false,
        }
    }
}

static CRASH_REPORTS: Spinlock<[CrashReport; MAX_REPORTS]> = Spinlock::new(
    [
        CrashReport::empty(),
        CrashReport::empty(),
        CrashReport::empty(),
        CrashReport::empty(),
        CrashReport::empty(),
        CrashReport::empty(),
        CrashReport::empty(),
        CrashReport::empty(),
    ],
);

/// Next write index in the ring buffer.
static NEXT_IDX: Spinlock<usize> = Spinlock::new(0);

/// Store a crash report from a faulting thread (x86_64).
///
/// Called from `try_kill_faulting_thread()` in idt.rs. Uses lock-free thread name
/// query to avoid deadlock (scheduler lock may be held during a page fault).
#[cfg(target_arch = "x86_64")]
pub fn store_crash(
    tid: u32,
    signal: u32,
    frame: &crate::arch::x86::idt::InterruptFrame,
) {
    // Get thread name via lock-free path (safe during fault handling)
    let name = crate::task::scheduler::debug_current_thread_name();

    // Read CR2 for page faults
    let cr2 = if signal == 139 && frame.int_no == 14 {
        let val: u64;
        unsafe { core::arch::asm!("mov {}, cr2", out(reg) val); }
        val
    } else {
        0
    };

    // Walk RBP chain for stack trace
    let mut stack_frames = [0u64; 16];
    let mut num_frames = 0u32;
    let mut bp = frame.rbp;
    while bp > 0x1000 && bp < 0x0000_8000_0000_0000 && bp & 7 == 0 && num_frames < 16 {
        let ret_addr = unsafe { *((bp + 8) as *const u64) };
        if ret_addr == 0 || ret_addr >= 0x0000_8000_0000_0000 {
            break;
        }
        stack_frames[num_frames as usize] = ret_addr;
        bp = unsafe { *(bp as *const u64) };
        num_frames += 1;
    }

    let report = CrashReport {
        tid,
        signal,
        rip: frame.rip,
        rsp: frame.rsp,
        rbp: frame.rbp,
        rax: frame.rax,
        rbx: frame.rbx,
        rcx: frame.rcx,
        rdx: frame.rdx,
        rsi: frame.rsi,
        rdi: frame.rdi,
        r8: frame.r8,
        r9: frame.r9,
        r10: frame.r10,
        r11: frame.r11,
        r12: frame.r12,
        r13: frame.r13,
        r14: frame.r14,
        r15: frame.r15,
        cr2,
        cs: frame.cs,
        ss: frame.ss,
        rflags: frame.rflags,
        err_code: frame.err_code,
        stack_frames,
        num_frames,
        name,
        valid: true,
    };

    // Store in ring buffer
    let mut idx = NEXT_IDX.lock();
    let mut reports = CRASH_REPORTS.lock();
    reports[*idx] = report;
    *idx = (*idx + 1) % MAX_REPORTS;
}

/// Retrieve and consume the crash report for a given TID.
/// Returns None if no report exists for that TID.
pub fn take_crash(tid: u32) -> Option<CrashReport> {
    let mut reports = CRASH_REPORTS.lock();
    for report in reports.iter_mut() {
        if report.valid && report.tid == tid {
            report.valid = false;
            // Copy out before invalidating
            let mut result = CrashReport::empty();
            result.tid = report.tid;
            result.signal = report.signal;
            result.rip = report.rip;
            result.rsp = report.rsp;
            result.rbp = report.rbp;
            result.rax = report.rax;
            result.rbx = report.rbx;
            result.rcx = report.rcx;
            result.rdx = report.rdx;
            result.rsi = report.rsi;
            result.rdi = report.rdi;
            result.r8 = report.r8;
            result.r9 = report.r9;
            result.r10 = report.r10;
            result.r11 = report.r11;
            result.r12 = report.r12;
            result.r13 = report.r13;
            result.r14 = report.r14;
            result.r15 = report.r15;
            result.cr2 = report.cr2;
            result.cs = report.cs;
            result.ss = report.ss;
            result.rflags = report.rflags;
            result.err_code = report.err_code;
            result.stack_frames = report.stack_frames;
            result.num_frames = report.num_frames;
            result.name = report.name;
            result.valid = true;
            return Some(result);
        }
    }
    None
}

/// Size of `CrashReport` in bytes (for syscall buffer validation).
pub const CRASH_REPORT_SIZE: usize = core::mem::size_of::<CrashReport>();
