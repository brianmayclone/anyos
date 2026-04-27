//! ARM64 syscall entry/exit.
//!
//! Syscalls use `SVC #0` from EL0, with:
//! - X8 = syscall number
//! - X0-X5 = arguments
//! - X0 = return value
//!
//! The SVC exception handler in exceptions.S saves the user context,
//! extracts the syscall number from X8, and calls `arm64_syscall_dispatch`.

use crate::arch::arm64::exceptions::ExceptionFrame;
use crate::syscall::{SYS_FORK, SYS_MAP_FRAMEBUFFER};

/// Dispatch a syscall from user space.
///
/// Called from exceptions.S after saving the user context.
/// Arguments are passed in registers X0-X5, syscall number in X8.
#[no_mangle]
pub extern "C" fn arm64_syscall_dispatch(
    nr: u64,
    arg0: u64, arg1: u64, arg2: u64,
    arg3: u64, arg4: u64, arg5: u64,
) -> u64 {
    // Forward to the common syscall dispatcher (5 args max)
    let _ = arg5; // reserved for future use

    // Pointer-bearing syscalls that must keep the full 64-bit argument
    // (cannot go through the u32-truncating dispatch_inner path).
    if nr as u32 == SYS_MAP_FRAMEBUFFER {
        let r = crate::syscall::handlers::sys_map_framebuffer(arg0);
        return if r == u32::MAX { u64::MAX } else { r as u64 };
    }

    let ret = crate::syscall::dispatch_inner(
        nr as u32,
        arg0, arg1, arg2, arg3, arg4,
    );
    if ret == u32::MAX {
        u64::MAX
    } else {
        ret as u64
    }
}

/// Dispatch `fork()` from the ARM64 SVC path.
///
/// `fork()` needs the full saved EL0 register frame so the child can resume
/// with an `eret`-based return path and X0=0.
#[no_mangle]
pub extern "C" fn arm64_syscall_fork(frame: *const ExceptionFrame) -> u64 {
    let cpu_id = crate::arch::hal::cpu_id();
    crate::task::scheduler::set_last_syscall(cpu_id, SYS_FORK);
    let frame = unsafe { &*frame };
    let result = crate::syscall::handlers::sys_fork(frame);
    crate::task::scheduler::check_current_stack_canary(SYS_FORK);
    if result == u32::MAX {
        u64::MAX
    } else {
        result as u64
    }
}

/// Initialize syscall handling for the BSP.
///
/// On ARM64, syscalls are handled via the SVC exception vector,
/// which is set up in `exceptions::init()`. No MSR configuration needed
/// (unlike x86's LSTAR/STAR/SFMASK).
pub fn init_bsp() {
    crate::serial_verbose_println!("[OK] Syscall: SVC #0 handler active (via VBAR_EL1)");
}

/// Initialize syscall handling for an Application Processor.
///
/// VBAR_EL1 is already set by `exceptions::init()` called from `arm64_ap_init`.
/// Nothing additional is needed per-CPU on ARM64.
pub fn init_cpu(_cpu_id: usize) {
    // VBAR_EL1 set by exceptions::init() — syscalls via SVC #0 are ready.
}
