//! Task State Segment (TSS) for x86-64 long mode — per-CPU.
//!
//! In 64-bit mode the TSS is used only for stack switching (RSP0 for
//! Ring 3→0 transitions, IST entries for dedicated interrupt stacks).
//! Each CPU has its own TSS so that interrupt entry from Ring 3 uses
//! the correct kernel stack for the thread running on that CPU.

use core::arch::asm;
use core::mem::size_of;
use crate::arch::x86::smp::MAX_CPUS;

/// x86-64 Task State Segment (104 bytes).
#[repr(C, packed)]
pub struct Tss64 {
    _reserved0: u32,
    pub rsp0: u64,          // Ring 0 stack pointer
    pub rsp1: u64,          // Ring 1 stack pointer (unused)
    pub rsp2: u64,          // Ring 2 stack pointer (unused)
    _reserved1: u64,
    pub ist1: u64,          // Interrupt Stack Table entry 1
    pub ist2: u64,          // IST 2
    pub ist3: u64,          // IST 3
    pub ist4: u64,          // IST 4
    pub ist5: u64,          // IST 5
    pub ist6: u64,          // IST 6
    pub ist7: u64,          // IST 7
    _reserved2: u64,
    _reserved3: u16,
    pub iomap_base: u16,    // Offset to I/O permission bitmap
}

/// Per-CPU TSS array. Each CPU gets its own TSS so RSP0 is independent.
static mut TSS_ARRAY: [Tss64; MAX_CPUS] = {
    const INIT: Tss64 = Tss64 {
        _reserved0: 0,
        rsp0: 0,
        rsp1: 0,
        rsp2: 0,
        _reserved1: 0,
        ist1: 0, ist2: 0, ist3: 0, ist4: 0, ist5: 0, ist6: 0, ist7: 0,
        _reserved2: 0,
        _reserved3: 0,
        iomap_base: 0,
    };
    [INIT; MAX_CPUS]
};

/// Initialize the TSS for CPU 0 (BSP), install its descriptor in the GDT, and load TR.
pub fn init() {
    init_for_cpu(0);
    crate::serial_println!("[OK] TSS initialized (CPU 0, selector {:#06x}, 64-bit)", super::gdt::TSS_SEL);
}

/// Initialize the TSS for a specific CPU, install the GDT descriptor, and load TR.
pub fn init_for_cpu(cpu_id: usize) {
    if cpu_id >= MAX_CPUS { return; }

    unsafe {
        TSS_ARRAY[cpu_id].iomap_base = size_of::<Tss64>() as u16;

        // Set a default kernel stack (updated per-thread by scheduler)
        if cpu_id == 0 {
            let rsp: u64;
            asm!("mov {}, rsp", out(reg) rsp);
            TSS_ARRAY[cpu_id].rsp0 = rsp;
        }

        // Install 16-byte TSS descriptor in GDT for this CPU
        let tss_base = &TSS_ARRAY[cpu_id] as *const Tss64 as u64;
        let tss_limit = (size_of::<Tss64>() - 1) as u32;
        super::gdt::set_tss_entry_for_cpu(cpu_id, tss_base, tss_limit);

        // Clear busy bit and load Task Register with this CPU's TSS selector
        let selector = super::gdt::tss_sel_for(cpu_id);
        super::gdt::clear_tss_busy_bit_for_cpu(cpu_id);
        asm!(
            "ltr ax",
            in("ax") selector,
            options(nostack, preserves_flags)
        );
    }
}

/// Load the TSS selector into TR on the current CPU (CPU 0).
/// Called by APs after loading the kernel GDT — use init_for_cpu() instead for APs.
pub fn reload_tr() {
    super::gdt::clear_tss_busy_bit();

    unsafe {
        asm!(
            "ltr ax",
            in("ax") super::gdt::TSS_SEL,
            options(nostack, preserves_flags)
        );
    }
}

/// Update the kernel stack pointer (RSP0) in the TSS for a specific CPU.
/// Called by the scheduler on context switch.
pub fn set_kernel_stack_for_cpu(cpu_id: usize, rsp0: u64) {
    if cpu_id < MAX_CPUS {
        unsafe {
            TSS_ARRAY[cpu_id].rsp0 = rsp0;
        }
    }
}

/// Read the TSS RSP0 for a given CPU (diagnostic use).
pub fn get_kernel_stack_for_cpu(cpu_id: usize) -> u64 {
    if cpu_id < MAX_CPUS {
        unsafe { TSS_ARRAY[cpu_id].rsp0 }
    } else {
        0
    }
}

/// Update the kernel stack pointer (RSP0) in CPU 0's TSS.
/// Backward-compatible wrapper used by existing scheduler code.
pub fn set_kernel_stack(rsp0: u64) {
    set_kernel_stack_for_cpu(0, rsp0);
}
