//! x86-64 architecture support.
//!
//! Provides GDT, IDT, TSS, PIC/APIC interrupt controllers, PIT timer,
//! I/O port access, IRQ management, ACPI parsing, and SMP startup.

pub mod acpi;
pub mod acpi_pm;
pub mod apic;
pub mod cpuid;
pub mod gdt;
pub mod idt;
pub mod ioapic;
pub mod irq;
pub mod pat;
pub mod pic;
pub mod pit;
pub mod port;
pub mod power;
pub mod smp;
pub mod syscall_msr;
pub mod tss;
pub mod virt;
