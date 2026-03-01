//! ARM64 power management via PSCI.
//!
//! Provides system reset and shutdown through PSCI calls.

/// PSCI function IDs.
const PSCI_SYSTEM_OFF: u64 = 0x8400_0008;
const PSCI_SYSTEM_RESET: u64 = 0x8400_0009;

/// Shut down the system via PSCI SYSTEM_OFF.
pub fn shutdown() -> ! {
    crate::serial_println!("PSCI: System shutdown...");
    unsafe {
        core::arch::asm!(
            "mov x0, {fn_id}",
            "hvc #0",
            fn_id = in(reg) PSCI_SYSTEM_OFF,
            options(nostack, noreturn),
        );
    }
}

/// Reset the system via PSCI SYSTEM_RESET.
pub fn reset() -> ! {
    crate::serial_println!("PSCI: System reset...");
    unsafe {
        core::arch::asm!(
            "mov x0, {fn_id}",
            "hvc #0",
            fn_id = in(reg) PSCI_SYSTEM_RESET,
            options(nostack, noreturn),
        );
    }
}

/// Initialize power management.
pub fn init() {
    crate::serial_println!("[OK] Power management: PSCI available");
}
