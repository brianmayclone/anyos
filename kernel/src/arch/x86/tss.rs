//! Task State Segment (TSS) for x86-64 long mode.
//!
//! In 64-bit mode the TSS is used only for stack switching (RSP0 for
//! Ring 3→0 transitions, IST entries for dedicated interrupt stacks).
//! Hardware task switching is not supported.

use core::arch::asm;
use core::mem::size_of;

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

static mut TSS: Tss64 = Tss64 {
    _reserved0: 0,
    rsp0: 0,
    rsp1: 0,
    rsp2: 0,
    _reserved1: 0,
    ist1: 0,
    ist2: 0,
    ist3: 0,
    ist4: 0,
    ist5: 0,
    ist6: 0,
    ist7: 0,
    _reserved2: 0,
    _reserved3: 0,
    iomap_base: 0,
};

/// Initialize the TSS, install its descriptor in the GDT, and load the task register.
pub fn init() {
    unsafe {
        // Set the I/O map base to the size of the TSS (no I/O bitmap)
        TSS.iomap_base = size_of::<Tss64>() as u16;

        // Set a default kernel stack (will be updated per-thread by scheduler)
        let rsp: u64;
        asm!("mov {}, rsp", out(reg) rsp);
        TSS.rsp0 = rsp;

        // Install 16-byte TSS descriptor in GDT (selector 0x30)
        let tss_base = &TSS as *const Tss64 as u64;
        let tss_limit = (size_of::<Tss64>() - 1) as u32;
        super::gdt::set_tss_entry(tss_base, tss_limit);

        // Load the Task Register with the TSS selector (0x30)
        asm!(
            "ltr ax",
            in("ax") super::gdt::TSS_SEL,
            options(nostack, preserves_flags)
        );
    }

    crate::serial_println!("[OK] TSS initialized (selector 0x30, 64-bit)");
}

/// Load the TSS selector into TR on the current CPU.
/// Called by APs after loading the kernel GDT.
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

/// Update the kernel stack pointer (RSP0) in the TSS.
/// Called by the scheduler on context switch.
pub fn set_kernel_stack(rsp0: u64) {
    unsafe {
        TSS.rsp0 = rsp0;
    }
}
