//! Kernel boot orchestration and boot-time state.
//!
//! Keeps the crate root thin by moving the staged boot sequence into dedicated
//! modules. Public boot flags remain exported so syscall handlers and other
//! subsystems can query boot/runtime mode without depending on the old
//! monolithic `main.rs`.

#[cfg(target_arch = "x86_64")]
mod x86;

use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

/// Boot mode: 0 = Legacy BIOS, 1 = UEFI.
static BOOT_MODE: AtomicU8 = AtomicU8::new(0);

/// GPU 2D acceleration available (queried by SYS_GPU_HAS_ACCEL).
pub static GPU_ACCEL: AtomicBool = AtomicBool::new(false);

/// GPU hardware cursor available (queried by SYS_GPU_HAS_HW_CURSOR).
pub static GPU_HW_CURSOR: AtomicBool = AtomicBool::new(false);

/// Set when the kernel is booted with "nogui" parameter.
/// Skips compositor and init; starts textmode_console instead.
pub static NOGUI: AtomicBool = AtomicBool::new(false);

/// Set when the kernel is booted with "setup" parameter (ISO installer mode).
/// Starts compositor + installer directly, skipping login/dock/init.
pub static SETUP_MODE: AtomicBool = AtomicBool::new(false);

/// Get the boot mode (0 = BIOS, 1 = UEFI).
pub fn boot_mode() -> u8 {
    BOOT_MODE.load(Ordering::Relaxed)
}

pub(crate) fn set_boot_mode(mode: u8) {
    BOOT_MODE.store(mode, Ordering::Relaxed);
}

/// Shared boot entry used by the crate-root wrapper.
pub fn kernel_main(boot_info_addr: u64) -> ! {
    #[cfg(target_arch = "x86_64")]
    {
        x86::kernel_main(boot_info_addr)
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = boot_info_addr;
        loop {
            crate::arch::hal::halt();
        }
    }
}
