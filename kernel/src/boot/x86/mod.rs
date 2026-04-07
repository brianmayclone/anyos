//! x86_64 boot flow orchestration.
//!
//! This module keeps the architecture-specific boot entry short by delegating
//! each phase to a focused submodule.

mod early;
mod platform;
mod storage;
mod userspace;

pub(super) fn kernel_main(boot_info_addr: u64) -> ! {
    crate::drivers::serial::init();

    early::early_output();
    let boot_info = early::read_boot_info(boot_info_addr);

    platform::init_cpu();
    platform::init_memory(boot_info);
    platform::run_unit_tests();
    let acpi_info = platform::init_platform(boot_info);
    platform::init_virtualization();
    platform::init_early_drivers(boot_info);
    platform::init_scheduler();
    platform::enable_interrupts_and_bind(acpi_info.is_some());
    platform::run_integration_tests();
    storage::init_storage_and_userspace(boot_info, acpi_info);
}
