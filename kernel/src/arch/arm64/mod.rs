//! ARM64 (AArch64) architecture-specific modules for QEMU virt machine.

pub mod boot;
pub mod context;
pub mod cpu_features;
pub mod exceptions;
pub mod generic_timer;
pub mod gic;
pub mod mmu;
pub mod power;
pub mod psci;
pub mod serial;
pub mod smp;
pub mod syscall;
