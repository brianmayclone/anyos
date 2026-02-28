//! Signal handling syscall handlers and delivery functions.
//!
//! Covers signal registration (sigaction), mask manipulation (sigprocmask),
//! signal return (sigreturn), and pending signal delivery after syscalls.

use super::helpers::is_valid_user_ptr;

// =========================================================================
// Signal syscalls
// =========================================================================

/// sys_sigaction - Set or query a signal handler.
/// sig: signal number, handler_addr: new handler address (SIG_DFL=0, SIG_IGN=1, or user addr).
/// Returns the old handler address.
pub fn sys_sigaction(sig: u32, handler_addr: u32) -> u32 {
    use crate::ipc::signal::{SIGKILL, SIGSTOP};
    if sig == 0 || sig >= 32 || sig == SIGKILL || sig == SIGSTOP {
        return u32::MAX;
    }
    let old = crate::task::scheduler::current_signal_set_handler(sig, handler_addr as u64);
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
        0 => old_mask | sanitized,          // SIG_BLOCK
        1 => old_mask & !sanitized,         // SIG_UNBLOCK
        2 => sanitized,                     // SIG_SETMASK
        _ => return old_mask,               // Invalid how — just return old mask
    };
    crate::task::scheduler::current_signal_set_blocked(new_mask);
    old_mask
}

/// sys_sigreturn_32 - Restore context saved by signal trampoline (32-bit compat path).
///
/// Called from `syscall_dispatch_32` when SYS_SIGRETURN is intercepted.
/// Reads the signal frame from the user stack and restores the saved registers
/// into the IRET frame so the thread resumes where it was before the signal.
///
/// User stack layout at this point (ESP = frame + 4, after handler did `ret`):
///   [ESP + 12] = saved_eflags
///   [ESP + 8]  = saved_eip
///   [ESP + 4]  = saved_eax (syscall result before signal)
///   [ESP + 0]  = sigreturn code bytes (trampoline)
///   [ESP - 4]  = signum
///   original ESP = ESP + 16 (past the frame)
///
/// Wait — let me recalculate based on the actual layout.
/// After handler's `ret`: ESP = new_esp + 4 (popped return addr).
/// The sigreturn code is at new_esp + 8, but the int 0x80 fires with ESP = new_esp + 4.
///
/// Signal frame (from new_esp):
///   +0:  return addr (points to trampoline at +8)
///   +4:  signum
///   +8:  trampoline code (8 bytes)
///   +16: saved_eax
///   +20: saved_eip
///   +24: saved_eflags
///   +28: [original ESP was here]
///
/// At int 0x80: user ESP = new_esp + 4.
///   saved_eax    at user_esp + 12
///   saved_eip    at user_esp + 16
///   saved_eflags at user_esp + 20
///   restore_esp  = user_esp + 24
pub fn sys_sigreturn_32(regs: &mut super::super::SyscallRegs) -> u32 {
    let user_esp = regs.rsp as u32;

    // Read saved context from user stack
    let saved_eax = unsafe { *((user_esp + 12) as *const u32) };
    let saved_eip = unsafe { *((user_esp + 16) as *const u32) };
    let saved_eflags = unsafe { *((user_esp + 20) as *const u32) };

    // Restore the IRET frame
    regs.rip = saved_eip as u64;
    regs.rflags = saved_eflags as u64;
    regs.rsp = (user_esp + 24) as u64; // restore original ESP

    // Return the saved syscall result (goes into RAX via asm stub)
    saved_eax
}

/// Deliver pending signals for 32-bit compat-mode processes.
///
/// Called after every syscall returns (INT 0x80 path). If there's a pending,
/// unblocked signal, this modifies the IRET register frame to redirect
/// execution to the signal handler before returning to user space.
///
/// For SIG_DFL: terminate (if default_is_terminate) or ignore.
/// For SIG_IGN: do nothing (signal already dequeued).
/// For user handler: set up signal trampoline on user stack.
pub fn deliver_pending_signal_32(regs: &mut super::super::SyscallRegs, syscall_result: u32) {
    use crate::ipc::signal::{SignalState, SIG_DFL, SIG_IGN};

    // Quick check without dequeuing — avoid unnecessary lock
    if !crate::task::scheduler::current_has_pending_signal() {
        return;
    }

    // Dequeue the signal
    let sig = match crate::task::scheduler::current_signal_dequeue() {
        Some(s) => s,
        None => return,
    };

    let handler = crate::task::scheduler::current_signal_handler(sig);

    if handler == SIG_DFL {
        // Default action
        if SignalState::default_is_terminate(sig) {
            // Terminate the process with the signal number as exit code
            crate::serial_println!("Signal {}: default terminate for T{}",
                sig, crate::task::scheduler::current_tid());
            // Use sys_exit to properly clean up
            super::sys_exit(128 + sig);
            return;
        }
        // Default ignore (SIGCHLD, SIGCONT) — do nothing
        return;
    }

    if handler == SIG_IGN {
        // Explicitly ignored — do nothing
        return;
    }

    // User handler — set up signal trampoline on the user stack.
    // Build a signal frame so the handler can execute and then sigreturn
    // restores the original context.
    //
    // Frame layout (32 bytes, pushing onto user stack):
    //   +0:  return addr → &trampoline (at new_esp + 8)
    //   +4:  signum (argument to handler)
    //   +8:  trampoline code: mov eax, SYS_SIGRETURN(246); int 0x80; nop  (8 bytes)
    //   +16: saved_eax (syscall result)
    //   +20: saved_eip (original return address)
    //   +24: saved_eflags
    //   +28: [original ESP]

    let old_esp = regs.rsp as u32;
    let new_esp = old_esp - 28; // 28 bytes of frame

    // Write the signal frame onto the user stack
    let frame = new_esp as *mut u32;
    unsafe {
        // +0: return address → points to trampoline at new_esp + 8
        *frame.offset(0) = new_esp + 8;
        // +4: signal number (handler argument)
        *frame.offset(1) = sig;
        // +8: sigreturn trampoline code (8 bytes):
        //   B8 F6 00 00 00    mov eax, 246 (SYS_SIGRETURN)
        //   CD 80             int 0x80
        //   90                nop (pad)
        let trampoline = (new_esp + 8) as *mut u8;
        *trampoline.offset(0) = 0xB8; // mov eax, imm32
        *trampoline.offset(1) = 246;  // SYS_SIGRETURN = 246
        *trampoline.offset(2) = 0x00;
        *trampoline.offset(3) = 0x00;
        *trampoline.offset(4) = 0x00;
        *trampoline.offset(5) = 0xCD; // int
        *trampoline.offset(6) = 0x80; // 0x80
        *trampoline.offset(7) = 0x90; // nop
        // +16: saved syscall result (EAX)
        *frame.offset(4) = syscall_result;
        // +20: saved EIP (where the process would have returned to)
        *frame.offset(5) = regs.rip as u32;
        // +24: saved EFLAGS
        *frame.offset(6) = regs.rflags as u32;
    }

    // Redirect IRET to the signal handler
    regs.rip = handler;
    regs.rsp = new_esp as u64;
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
            crate::serial_println!("Signal {}: default terminate for T{} (64-bit)",
                sig, crate::task::scheduler::current_tid());
            super::sys_exit(128 + sig);
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
        crate::serial_println!("Signal {}: no 64-bit trampoline, terminate T{}",
            sig, crate::task::scheduler::current_tid());
        super::sys_exit(128 + sig);
    }
}
