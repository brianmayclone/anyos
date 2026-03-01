//! ARM64 syscall entry/exit.
//!
//! Syscalls use `SVC #0` from EL0, with:
//! - X8 = syscall number
//! - X0-X5 = arguments
//! - X0 = return value
//!
//! The SVC exception handler in exceptions.S saves the user context,
//! extracts the syscall number from X8, and calls `arm64_syscall_dispatch`.

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
    crate::syscall::dispatch_inner(
        nr as u32,
        arg0 as u32, arg1 as u32, arg2 as u32,
        arg3 as u32, arg4 as u32,
    ) as u64
}

/// Initialize syscall handling for the BSP.
///
/// On ARM64, syscalls are handled via the SVC exception vector,
/// which is set up in `exceptions::init()`. No MSR configuration needed
/// (unlike x86's LSTAR/STAR/SFMASK).
pub fn init_bsp() {
    crate::serial_println!("[OK] Syscall: SVC #0 handler active (via VBAR_EL1)");
}
