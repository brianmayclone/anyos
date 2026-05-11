//! Signal handling syscall handlers and delivery functions.
//!
//! Covers signal registration (sigaction), mask manipulation (sigprocmask),
//! signal return (sigreturn), and pending signal delivery after syscalls.

#[allow(unused_imports)]
use super::helpers::is_valid_user_ptr;

// =========================================================================
// Signal syscalls
// =========================================================================

/// sys_sigaction - Set or query a signal handler.
/// sig: signal number, handler_addr: new handler address (SIG_DFL=0, SIG_IGN=1, or user addr).
/// Returns the old handler address.
pub fn sys_sigaction(sig: u32, handler_addr: u64) -> u32 {
    use crate::ipc::signal::{SIGKILL, SIGSTOP};
    if sig == 0 || sig >= 32 || sig == SIGKILL || sig == SIGSTOP {
        return u32::MAX;
    }
    let old = crate::task::scheduler::current_signal_set_handler(sig, handler_addr);
    old as u32
}

/// sys_sigprocmask - Get/set the signal blocked mask.
/// how: 0=SIG_BLOCK (add set to mask), 1=SIG_UNBLOCK (remove set from mask),
///      2=SIG_SETMASK (set mask to set).
/// set: bitmask of signals to add/remove/set.
/// Returns the old blocked mask.
pub fn sys_sigprocmask(how: u32, set: u32) -> u32 {
    use crate::ipc::signal::{SIGKILL, SIGSTOP};
    let old_mask = crate::task::scheduler::current_signal_get_blocked();
    // SIGKILL and SIGSTOP can never be blocked
    let sanitized = set & !((1 << SIGKILL) | (1 << SIGSTOP));
    let new_mask = match how {
        0 => old_mask | sanitized,  // SIG_BLOCK
        1 => old_mask & !sanitized, // SIG_UNBLOCK
        2 => sanitized,             // SIG_SETMASK
        _ => return old_mask,       // Invalid how — just return old mask
    };
    crate::task::scheduler::current_signal_set_blocked(new_mask);
    old_mask
}

/// sys_sigreturn - Restore context saved by the signal trampoline.
///
/// Called from `syscall_dispatch_64` when SYS_SIGRETURN is intercepted.
/// Reads the signal frame from the user stack and restores the saved registers
/// into the syscall return frame so the thread resumes where it was before
/// the signal.
///
/// NOTE: the existing frame layout (4-byte slots, 32-bit-style) is a leftover
/// from the now-removed INT 0x80 / 32-bit user-space path. It still works as
/// long as user RIP/RSP fit in 32 bits, but should be widened to 8-byte slots
/// when proper 64-bit user-handler signal delivery is implemented in
/// `deliver_pending_signal_default`. There is currently no caller path that
/// produces such a frame — 64-bit user handlers are not delivered today —
/// so this function is effectively a placeholder until that work lands.
///
/// Signal frame (from new_esp):
///   +0:  return addr → SIGRETURN_TRAMPOLINE_ADDR
///   +4:  signum
///   +8:  saved syscall result (RAX low 32 bits)
///   +12: saved RIP (low 32 bits)
///   +16: saved RFLAGS (low 32 bits)
pub fn sys_sigreturn(regs: &mut super::super::SyscallRegs) -> u32 {
    let user_esp = regs.rsp as u32;

    // Read saved context from user stack
    let saved_eax = unsafe { *((user_esp + 4) as *const u32) };
    let saved_eip = unsafe { *((user_esp + 8) as *const u32) };
    let saved_eflags = unsafe { *((user_esp + 12) as *const u32) };

    // Restore the return frame
    regs.rip = saved_eip as u64;
    regs.rflags = saved_eflags as u64;
    regs.rsp = (user_esp + 16) as u64; // restore original ESP

    // Return the saved syscall result (goes into RAX via asm stub)
    saved_eax
}

/// Deliver pending signals using default actions only (64-bit native path).
///
/// Called after every syscall on the SYSCALL path. Since 64-bit native programs
/// (compositor, terminal, Rust apps) don't currently use signal handlers,
/// this only handles SIG_DFL terminate/ignore and SIG_IGN.
pub fn deliver_pending_signal_default() {
    use crate::ipc::signal::{SignalState, SIG_DFL, SIG_IGN};

    if !crate::task::scheduler::current_has_pending_signal() {
        return;
    }

    let sig = match crate::task::scheduler::current_signal_dequeue() {
        Some(s) => s,
        None => return,
    };

    let handler = crate::task::scheduler::current_signal_handler(sig);

    if handler == SIG_DFL {
        if SignalState::default_is_terminate(sig) {
            crate::serial_verbose_println!(
                "Signal {}: default terminate for T{} (64-bit)",
                sig,
                crate::task::scheduler::current_tid()
            );
            super::sys_exit(128 + sig);
        }
        if SignalState::default_is_stop(sig) {
            crate::task::scheduler::stop_current_thread(sig);
            return;
        }
        // Default ignore — do nothing
        return;
    }

    if handler == SIG_IGN {
        return;
    }

    // User handler for 64-bit path — not implemented yet.
    // For now, treat as SIG_DFL.
    if SignalState::default_is_terminate(sig) {
        crate::serial_verbose_println!(
            "Signal {}: no 64-bit trampoline, terminate T{}",
            sig,
            crate::task::scheduler::current_tid()
        );
        super::sys_exit(128 + sig);
    }
}

const LINUX_RT_SIGFRAME_MAGIC: u64 = 0x5349_4746_524d_3634;
const LINUX_RT_SIGFRAME_REGS_OFFSET: u64 = 8;
const LINUX_RT_SIGFRAME_BLOCKED_OFFSET: u64 = LINUX_RT_SIGFRAME_REGS_OFFSET + 20 * 8;
const LINUX_RT_SIGFRAME_SIZE: u64 = LINUX_RT_SIGFRAME_BLOCKED_OFFSET + 8;
const LINUX_X86_64_RED_ZONE_SIZE: u64 = 128;
const LINUX_SA_NODEFER: u64 = 0x4000_0000;

unsafe fn write_sigframe_regs(base: u64, regs: &super::super::SyscallRegs, result: u64) {
    let values = [
        regs.r15,
        regs.r14,
        regs.r13,
        regs.r12,
        regs.r11,
        regs.r10,
        regs.r9,
        regs.r8,
        regs.rbp,
        regs.rdi,
        regs.rsi,
        regs.rdx,
        regs.rcx,
        regs.rbx,
        result,
        regs.rip,
        regs.cs,
        regs.rflags,
        regs.rsp,
        regs.ss,
    ];
    for (idx, value) in values.iter().enumerate() {
        *((base + LINUX_RT_SIGFRAME_REGS_OFFSET + (idx as u64) * 8) as *mut u64) = *value;
    }
}

unsafe fn read_sigframe_reg(base: u64, idx: u64) -> u64 {
    *((base + LINUX_RT_SIGFRAME_REGS_OFFSET + idx * 8) as *const u64)
}

/// Deliver one pending Linux x86_64 signal by building a minimal rt_sigframe.
///
/// The userspace restorer still performs the real `rt_sigreturn` syscall. Our
/// frame is intentionally compact, but it is sufficient for libc restorers
/// because only the kernel consumes the frame contents.
pub fn deliver_pending_signal_linux64(regs: &mut super::super::SyscallRegs, result: u64) -> u64 {
    use crate::ipc::signal::{SignalState, SIG_DFL, SIG_IGN};

    if !crate::task::scheduler::current_has_pending_signal() {
        return result;
    }

    let sig = match crate::task::scheduler::current_signal_dequeue() {
        Some(s) => s,
        None => return result,
    };

    let (handler, flags, restorer, mask) = crate::task::scheduler::current_signal_action(sig);

    if handler == SIG_DFL {
        if SignalState::default_is_terminate(sig) {
            super::sys_exit(128 + sig);
        }
        if SignalState::default_is_stop(sig) {
            crate::task::scheduler::stop_current_thread(sig);
        }
        return result;
    }
    if handler == SIG_IGN {
        return result;
    }
    if restorer == 0 {
        crate::serial_println!(
            "licof linux signal: cannot deliver sig={} handler={:#x} without restorer",
            sig,
            handler
        );
        return result;
    }

    // Linux signal delivery must preserve the SysV x86_64 red zone. Shells
    // like dash can keep wait/job state there across a blocking wait4; placing
    // the rt frame directly below RSP corrupts that state before rt_sigreturn.
    let signal_stack_top = regs.rsp.wrapping_sub(LINUX_X86_64_RED_ZONE_SIZE);
    let frame_slot = ((signal_stack_top.wrapping_sub(LINUX_RT_SIGFRAME_SIZE + 16)) & !0xf) + 8;
    let frame_base = frame_slot + 8;
    if !super::helpers::is_user_range_accessible(frame_slot, LINUX_RT_SIGFRAME_SIZE + 8) {
        super::sys_exit(128 + crate::ipc::signal::SIGSEGV);
        return result;
    }

    let old_blocked = crate::task::scheduler::current_signal_get_blocked();
    unsafe {
        *(frame_slot as *mut u64) = restorer;
        *(frame_base as *mut u64) = LINUX_RT_SIGFRAME_MAGIC;
        write_sigframe_regs(frame_base, regs, result);
        *((frame_base + LINUX_RT_SIGFRAME_BLOCKED_OFFSET) as *mut u64) = old_blocked as u64;
    }

    let mut new_blocked = old_blocked | (mask as u32);
    if (flags & LINUX_SA_NODEFER) == 0 {
        new_blocked |= 1u32 << sig;
    }
    crate::task::scheduler::current_signal_set_blocked(new_blocked);

    crate::serial_println!(
        "licof linux signal: deliver tid={} sig={} handler={:#x} restorer={:#x} frame={:#x} result={:#x}",
        crate::task::scheduler::current_tid(),
        sig,
        handler,
        restorer,
        frame_base,
        result
    );

    regs.rip = handler;
    regs.rsp = frame_slot;
    regs.rdi = sig as u64;
    regs.rsi = 0;
    regs.rdx = 0;
    result
}

pub fn linux_rt_sigreturn(regs: &mut super::super::SyscallRegs) -> u64 {
    let frame_base = regs.rsp;
    if frame_base == 0
        || !super::helpers::is_user_range_accessible(frame_base, LINUX_RT_SIGFRAME_SIZE)
    {
        return (-14i64) as u64;
    }
    let magic = unsafe { *(frame_base as *const u64) };
    if magic != LINUX_RT_SIGFRAME_MAGIC {
        crate::serial_println!(
            "licof linux rt_sigreturn: bad frame magic={:#x} rsp={:#x}",
            magic,
            frame_base
        );
        return (-14i64) as u64;
    }

    let old_blocked = unsafe { *((frame_base + LINUX_RT_SIGFRAME_BLOCKED_OFFSET) as *const u64) };
    crate::task::scheduler::current_signal_set_blocked(old_blocked as u32);

    unsafe {
        regs.r15 = read_sigframe_reg(frame_base, 0);
        regs.r14 = read_sigframe_reg(frame_base, 1);
        regs.r13 = read_sigframe_reg(frame_base, 2);
        regs.r12 = read_sigframe_reg(frame_base, 3);
        regs.r11 = read_sigframe_reg(frame_base, 4);
        regs.r10 = read_sigframe_reg(frame_base, 5);
        regs.r9 = read_sigframe_reg(frame_base, 6);
        regs.r8 = read_sigframe_reg(frame_base, 7);
        regs.rbp = read_sigframe_reg(frame_base, 8);
        regs.rdi = read_sigframe_reg(frame_base, 9);
        regs.rsi = read_sigframe_reg(frame_base, 10);
        regs.rdx = read_sigframe_reg(frame_base, 11);
        regs.rcx = read_sigframe_reg(frame_base, 12);
        regs.rbx = read_sigframe_reg(frame_base, 13);
        regs.rax = read_sigframe_reg(frame_base, 14);
        regs.rip = read_sigframe_reg(frame_base, 15);
        regs.cs = read_sigframe_reg(frame_base, 16);
        regs.rflags = read_sigframe_reg(frame_base, 17);
        regs.rsp = read_sigframe_reg(frame_base, 18);
        regs.ss = read_sigframe_reg(frame_base, 19);
    }

    regs.rax
}
