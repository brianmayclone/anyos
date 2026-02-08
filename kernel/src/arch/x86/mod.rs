//! x86 (i686) architecture support.
//!
//! Provides GDT, IDT, TSS, PIC/APIC interrupt controllers, PIT timer,
//! I/O port access, IRQ management, ACPI parsing, and SMP startup.

pub mod acpi;
pub mod apic;
pub mod gdt;
pub mod idt;
pub mod ioapic;
pub mod irq;
pub mod pic;
pub mod pit;
pub mod port;
pub mod smp;
pub mod tss;
