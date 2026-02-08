//! Task State Segment (TSS) for Ring 3 to Ring 0 stack switching.
//!
//! The TSS stores the kernel stack pointer (`esp0`) that the CPU loads
//! automatically when an interrupt or syscall occurs in user mode. The
//! scheduler updates `esp0` on every context switch.

use core::arch::asm;
use core::mem::size_of;

/// i386 Task State Segment
#[repr(C, packed)]
pub struct Tss {
    pub link: u16,
    _reserved0: u16,
    pub esp0: u32,
    pub ss0: u16,
    _reserved1: u16,
    pub esp1: u32,
    pub ss1: u16,
    _reserved2: u16,
    pub esp2: u32,
    pub ss2: u16,
    _reserved3: u16,
    pub cr3: u32,
    pub eip: u32,
    pub eflags: u32,
    pub eax: u32,
    pub ecx: u32,
    pub edx: u32,
    pub ebx: u32,
    pub esp: u32,
    pub ebp: u32,
    pub esi: u32,
    pub edi: u32,
    pub es: u16,
    _reserved4: u16,
    pub cs: u16,
    _reserved5: u16,
    pub ss: u16,
    _reserved6: u16,
    pub ds: u16,
    _reserved7: u16,
    pub fs: u16,
    _reserved8: u16,
    pub gs: u16,
    _reserved9: u16,
    pub ldt: u16,
    _reserved10: u16,
    _reserved11: u16,
    pub iomap_base: u16,
}

static mut TSS: Tss = Tss {
    link: 0,
    _reserved0: 0,
    esp0: 0,
    ss0: 0x10, // Kernel data segment
    _reserved1: 0,
    esp1: 0,
    ss1: 0,
    _reserved2: 0,
    esp2: 0,
    ss2: 0,
    _reserved3: 0,
    cr3: 0,
    eip: 0,
    eflags: 0,
    eax: 0,
    ecx: 0,
    edx: 0,
    ebx: 0,
    esp: 0,
    ebp: 0,
    esi: 0,
    edi: 0,
    es: 0,
    _reserved4: 0,
    cs: 0,
    _reserved5: 0,
    ss: 0,
    _reserved6: 0,
    ds: 0,
    _reserved7: 0,
    fs: 0,
    _reserved8: 0,
    gs: 0,
    _reserved9: 0,
    ldt: 0,
    _reserved10: 0,
    _reserved11: 0,
    iomap_base: 0,
};

/// Initialize the TSS, install its descriptor in the GDT, and load the task register.
pub fn init() {
    unsafe {
        // Set the I/O map base to the size of the TSS (no I/O bitmap)
        TSS.iomap_base = size_of::<Tss>() as u16;
        TSS.ss0 = 0x10; // Kernel data segment

        // Set a default kernel stack (will be updated per-thread by scheduler)
        // Use the current stack as a reasonable default
        let esp: u32;
        asm!("mov {}, esp", out(reg) esp);
        TSS.esp0 = esp;

        // Install TSS descriptor in GDT entry 5 (selector 0x28)
        let tss_base = &TSS as *const Tss as u32;
        let tss_limit = (size_of::<Tss>() - 1) as u32;
        super::gdt::set_tss_entry(tss_base, tss_limit);

        // Load the Task Register with the TSS selector (0x28)
        // OR'd with RPL=0
        asm!(
            "ltr ax",
            in("ax") 0x28u16,
            options(nostack, preserves_flags)
        );
    }

    crate::serial_println!("[OK] TSS initialized (selector 0x28)");
}

/// Load the TSS selector into TR on the current CPU.
/// Called by APs after loading the kernel GDT.
/// All CPUs share the same TSS (only esp0 matters and is updated per context switch).
pub fn reload_tr() {
    // Clear TSS busy bit so `ltr` can be executed on this CPU
    super::gdt::clear_tss_busy_bit();

    unsafe {
        core::arch::asm!(
            "ltr ax",
            in("ax") 0x28u16,
            options(nostack, preserves_flags)
        );
    }
}

/// Update the kernel stack pointer in the TSS.
/// Called by the scheduler on context switch.
pub fn set_kernel_stack(esp0: u32) {
    unsafe {
        TSS.esp0 = esp0;
    }
}
