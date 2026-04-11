//! ARM64 power management via PSCI.
//!
//! Provides system reset and shutdown through the detected PSCI conduit.

/// PSCI function IDs.
const PSCI_SYSTEM_OFF: u64 = 0x8400_0008;
const PSCI_SYSTEM_RESET: u64 = 0x8400_0009;

/// Shut down the system via PSCI SYSTEM_OFF.
pub fn shutdown() -> ! {
    crate::serial_verbose_println!("PSCI: System shutdown...");
    let _ = super::psci::call(PSCI_SYSTEM_OFF, 0, 0, 0);
    loop {
        crate::arch::hal::halt();
    }
}

/// Reset the system via PSCI SYSTEM_RESET.
pub fn reset() -> ! {
    crate::serial_verbose_println!("PSCI: System reset...");
    let _ = super::psci::call(PSCI_SYSTEM_RESET, 0, 0, 0);
    loop {
        crate::arch::hal::halt();
    }
}

/// Initialize power management.
pub fn init() {
    super::psci::init();
    crate::serial_verbose_println!(
        "[OK] Power management: PSCI available ({})",
        super::psci::conduit_name(),
    );
}
