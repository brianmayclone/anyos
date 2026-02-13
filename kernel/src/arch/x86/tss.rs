//! Task State Segment (TSS) for x86-64 long mode — per-CPU.
//!
//! In 64-bit mode the TSS is used only for stack switching (RSP0 for
//! Ring 3→0 transitions, IST entries for dedicated interrupt stacks).
//! Each CPU has its own TSS so that interrupt entry from Ring 3 uses
//! the correct kernel stack for the thread running on that CPU.

use core::arch::asm;
use core::mem::size_of;
use crate::arch::x86::smp::MAX_CPUS;

/// Size of each per-CPU IST stack (for Double Fault handler).
const IST_STACK_SIZE: usize = 8192; // 8 KiB — enough for exception diagnostics

/// Per-CPU IST1 stacks (used by #DF so it always gets a valid stack).
/// Statically allocated to avoid heap dependency during early init.
#[repr(C, align(16))]
struct IstStack([u8; IST_STACK_SIZE]);
static mut IST1_STACKS: [IstStack; MAX_CPUS] = {
    const INIT: IstStack = IstStack([0u8; IST_STACK_SIZE]);
    [INIT; MAX_CPUS]
};

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

        // Configure IST1 for Double Fault (#DF) — always gets a valid stack
        // even if TSS.RSP0 is corrupt. Stack grows down, so IST points to top.
        let ist1_top = IST1_STACKS[cpu_id].0.as_ptr().add(IST_STACK_SIZE) as u64;
        TSS_ARRAY[cpu_id].ist1 = ist1_top;

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

/// Byte offset of `rsp0` within the packed `Tss64` struct.
/// Layout: _reserved0(u32=4 bytes) then rsp0(u64=8 bytes).
const RSP0_OFFSET: usize = 4;

/// Raw pointer to TSS.RSP0 for a given CPU (bypasses packed-struct UB).
#[inline]
unsafe fn rsp0_ptr(cpu_id: usize) -> *mut u64 {
    let base = &raw mut TSS_ARRAY[cpu_id] as *mut u8;
    base.add(RSP0_OFFSET) as *mut u64
}

/// Update the kernel stack pointer (RSP0) in the TSS for a specific CPU.
/// Called by the scheduler on context switch.
///
/// Uses raw pointer writes to avoid packed-struct reference UB, and verifies
/// the write with a read-back check.
pub fn set_kernel_stack_for_cpu(cpu_id: usize, rsp0: u64) {
    if cpu_id < MAX_CPUS {
        // Guard: RSP0 must be in kernel higher-half and non-zero.
        // A corrupt RSP0 causes the CPU to load a garbage stack on Ring 3→0
        // transitions, leading to an unrecoverable ISR infinite loop with IF=0.
        if rsp0 == 0 || rsp0 < 0xFFFF_FFFF_8000_0000 {
            // Print via direct UART (lock-free) — this is a critical bug
            unsafe {
                use crate::arch::x86::port::{inb, outb};
                let msg = b"\r\n!!! BUG: set_kernel_stack_for_cpu rsp0=0 cpu=";
                for &c in msg { while inb(0x3FD) & 0x20 == 0 {} outb(0x3F8, c); }
                outb(0x3F8, b'0' + cpu_id as u8);
                let msg2 = b"\r\n";
                for &c in msg2 { while inb(0x3FD) & 0x20 == 0 {} outb(0x3F8, c); }
            }
            return; // Do NOT update TSS with garbage — keep the previous valid RSP0
        }
        unsafe {
            // Temporarily disable DR0 watchpoint for this legitimate write.
            // DR7 bit 0 = local enable for DR0. Clear it, write, re-enable.
            let dr7: u64;
            asm!("mov {}, dr7", out(reg) dr7, options(nostack, nomem, preserves_flags));
            if dr7 & 1 != 0 {
                asm!("mov dr7, {}", in(reg) dr7 & !1u64, options(nostack, nomem, preserves_flags));
            }

            let ptr = rsp0_ptr(cpu_id);
            core::ptr::write_unaligned(ptr, rsp0);
            // Read-back verification: ensure the write actually landed.
            // If something (wild pointer, DMA, memory corruption) is racing
            // with us, this catches it immediately.
            let readback = core::ptr::read_unaligned(ptr as *const u64);
            if readback != rsp0 {
                use crate::arch::x86::port::{inb, outb};
                let msg = b"\r\n!!! TSS WRITE MISMATCH cpu=";
                for &c in msg { while inb(0x3FD) & 0x20 == 0 {} outb(0x3F8, c); }
                outb(0x3F8, b'0' + cpu_id as u8);
                let msg2 = b"\r\n";
                for &c in msg2 { while inb(0x3FD) & 0x20 == 0 {} outb(0x3F8, c); }
                // Retry the write
                core::ptr::write_unaligned(ptr, rsp0);
            }

            // Re-enable DR0 watchpoint if it was enabled
            if dr7 & 1 != 0 {
                asm!("mov dr7, {}", in(reg) dr7, options(nostack, nomem, preserves_flags));
            }
        }
    }
}

/// Read the TSS RSP0 for a given CPU (diagnostic use).
/// Uses raw pointer to avoid packed-struct reference UB.
pub fn get_kernel_stack_for_cpu(cpu_id: usize) -> u64 {
    if cpu_id < MAX_CPUS {
        unsafe { core::ptr::read_unaligned(rsp0_ptr(cpu_id) as *const u64) }
    } else {
        0
    }
}

/// Update the kernel stack pointer (RSP0) in CPU 0's TSS.
/// Backward-compatible wrapper used by existing scheduler code.
pub fn set_kernel_stack(rsp0: u64) {
    set_kernel_stack_for_cpu(0, rsp0);
}

/// Return the virtual address of TSS.RSP0 for a given CPU.
/// Used to set up hardware watchpoints (DR0) for corruption detection.
pub fn rsp0_address(cpu_id: usize) -> u64 {
    if cpu_id >= MAX_CPUS { return 0; }
    unsafe { rsp0_ptr(cpu_id) as u64 }
}

/// Set a hardware write watchpoint (DR0) on TSS.RSP0 for the current CPU.
/// When ANY instruction writes to this address, CPU raises #DB (ISR 1).
/// DR7 is configured for: local DR0 enable, write-only, 8-byte width.
pub fn enable_rsp0_watchpoint(cpu_id: usize) {
    let addr = rsp0_address(cpu_id);
    if addr == 0 { return; }

    unsafe {
        // DR0 = address to watch
        asm!("mov dr0, {}", in(reg) addr, options(nostack, preserves_flags));

        // DR7: bit 0 = local enable DR0
        //      bits 17:16 = condition for DR0 (01 = write-only)
        //      bits 19:18 = length for DR0 (11 = 8 bytes / qword)
        //      bits 8 = LE (local exact breakpoint enable, legacy — set to 1)
        // DR7 = (01 << 16) | (11 << 18) | (1 << 8) | (1 << 0)
        //     = 0x0001_0000 | 0x000C_0000 | 0x0000_0100 | 0x0000_0001
        //     = 0x000D_0101
        let dr7: u64 = 0x000D_0101;
        asm!("mov dr7, {}", in(reg) dr7, options(nostack, preserves_flags));
    }

    crate::serial_println!(
        "  TSS.RSP0 watchpoint enabled on CPU{}: DR0={:#018x} (8-byte write)",
        cpu_id, addr
    );
}
