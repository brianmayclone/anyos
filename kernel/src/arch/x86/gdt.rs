//! Global Descriptor Table (GDT) for x86-64 long mode.
//!
//! Defines eight base entries: null, kernel code64 (Ring 0), kernel data (Ring 0),
//! user code32 compat (Ring 3), user data (Ring 3), user code64 (Ring 3),
//! plus per-CPU 16-byte TSS descriptors (2 GDT slots each).
//!
//! GDT layout (designed for SYSCALL/SYSRET):
//!   0x00: Null
//!   0x08: Kernel Code 64 (L=1, D=0, DPL=0)
//!   0x10: Kernel Data (DPL=0)
//!   0x18: User Code 32 compat (L=0, D=1, DPL=3) — SYSRET base
//!   0x20: User Data (DPL=3)
//!   0x28: User Code 64 (L=1, D=0, DPL=3)
//!   0x30+N*0x10: TSS for CPU N (16 bytes, 2 entries per CPU)

use core::arch::asm;
use core::mem::size_of;
use crate::arch::x86::smp::MAX_CPUS;

/// GDT segment selectors (without RPL bits).
pub const KERNEL_CODE64_SEL: u16 = 0x08;
pub const KERNEL_DATA_SEL: u16 = 0x10;
pub const USER_CODE32_SEL: u16 = 0x18;
pub const USER_DATA_SEL: u16 = 0x20;
pub const USER_CODE64_SEL: u16 = 0x28;
pub const TSS_SEL: u16 = 0x30; // CPU 0 TSS selector (backward compat)

/// STAR MSR value for SYSCALL/SYSRET.
/// Bits 47:32 = SYSCALL kernel CS base (0x08 → CS=0x08, SS=0x10)
/// Bits 63:48 = SYSRET user CS base (0x18 → compat CS=0x1B, SS=0x23, 64-bit CS=0x2B)
pub const STAR_MSR_VALUE: u64 = ((USER_CODE32_SEL as u64) << 48) | ((KERNEL_CODE64_SEL as u64) << 32);

/// Get the TSS selector for a given CPU index.
pub const fn tss_sel_for(cpu_id: usize) -> u16 {
    (0x30 + cpu_id * 0x10) as u16
}

#[repr(C, packed)]
#[derive(Copy, Clone)]
struct GdtEntry {
    limit_low: u16,
    base_low: u16,
    base_mid: u8,
    access: u8,
    flags_limit_high: u8,
    base_high: u8,
}

#[repr(C, packed)]
struct GdtDescriptor {
    size: u16,
    offset: u64,
}

/// Number of GDT entries: 6 code/data segments + 2 per CPU for TSS
const GDT_ENTRIES: usize = 6 + 2 * MAX_CPUS;

static mut GDT: [GdtEntry; GDT_ENTRIES] = [GdtEntry {
    limit_low: 0,
    base_low: 0,
    base_mid: 0,
    access: 0,
    flags_limit_high: 0,
    base_high: 0,
}; GDT_ENTRIES];

static mut GDT_DESC: GdtDescriptor = GdtDescriptor { size: 0, offset: 0 };

fn make_entry(base: u32, limit: u32, access: u8, flags: u8) -> GdtEntry {
    GdtEntry {
        limit_low: (limit & 0xFFFF) as u16,
        base_low: (base & 0xFFFF) as u16,
        base_mid: ((base >> 16) & 0xFF) as u8,
        access,
        flags_limit_high: ((limit >> 16) & 0x0F) as u8 | (flags << 4),
        base_high: ((base >> 24) & 0xFF) as u8,
    }
}

/// Install a 16-byte TSS descriptor for the given CPU into the GDT.
/// The TSS descriptor spans 2 consecutive GDT entries at offset 6 + 2*cpu_id.
pub fn set_tss_entry_for_cpu(cpu_id: usize, base: u64, limit: u32) {
    let entry_idx = 6 + 2 * cpu_id;
    if entry_idx + 1 >= GDT_ENTRIES {
        return;
    }
    unsafe {
        let base_lo = base as u32;
        let base_hi = (base >> 32) as u32;

        // Low 8 bytes: standard TSS descriptor
        // Access: 0x89 = Present, DPL=0, Available 64-bit TSS
        GDT[entry_idx] = GdtEntry {
            limit_low: (limit & 0xFFFF) as u16,
            base_low: (base_lo & 0xFFFF) as u16,
            base_mid: ((base_lo >> 16) & 0xFF) as u8,
            access: 0x89,
            flags_limit_high: ((limit >> 16) & 0x0F) as u8,
            base_high: ((base_lo >> 24) & 0xFF) as u8,
        };

        // High 8 bytes: upper 32 bits of base + reserved
        let tss_high_ptr = &mut GDT[entry_idx + 1] as *mut GdtEntry as *mut u32;
        core::ptr::write(tss_high_ptr, base_hi);
        core::ptr::write(tss_high_ptr.add(1), 0);
    }
}

/// Install the 16-byte TSS descriptor in GDT (CPU 0 for backward compatibility).
pub fn set_tss_entry(base: u64, limit: u32) {
    set_tss_entry_for_cpu(0, base, limit);
    reload_gdtr();
}

/// Reload the GDTR with the current GDT address and size.
fn reload_gdtr() {
    unsafe {
        GDT_DESC = GdtDescriptor {
            size: (GDT_ENTRIES * size_of::<GdtEntry>() - 1) as u16,
            offset: GDT.as_ptr() as u64,
        };
        asm!(
            "lgdt [{}]",
            in(reg) &GDT_DESC as *const GdtDescriptor,
            options(nostack, preserves_flags)
        );
    }
}

/// Clear the TSS busy bit for a given CPU so `ltr` can be executed.
pub fn clear_tss_busy_bit_for_cpu(cpu_id: usize) {
    let entry_idx = 6 + 2 * cpu_id;
    if entry_idx >= GDT_ENTRIES { return; }
    unsafe {
        let access_ptr = (GDT.as_ptr() as *const u8).add(entry_idx * 8 + 5) as *mut u8;
        let access = core::ptr::read_volatile(access_ptr);
        core::ptr::write_volatile(access_ptr, access & !0x02); // 0x8B -> 0x89
    }
}

/// Clear the TSS busy bit in the GDT (CPU 0 for backward compatibility).
pub fn clear_tss_busy_bit() {
    clear_tss_busy_bit_for_cpu(0);
}

/// Reload the kernel GDT and segment registers on the current CPU.
/// Used by APs after trampoline to switch to the full kernel GDT.
pub fn reload() {
    unsafe {
        asm!(
            "lgdt [{}]",
            in(reg) &raw const GDT_DESC,
            options(nostack, preserves_flags)
        );

        // Reload data segment registers
        asm!(
            "mov ax, 0x10",
            "mov ds, ax",
            "mov es, ax",
            "mov fs, ax",
            "mov gs, ax",
            "mov ss, ax",
            options(nostack)
        );

        // Far jump to reload CS with kernel code64 segment (0x08)
        asm!(
            "push 0x08",
            "lea {tmp}, [rip + 2f]",
            "push {tmp}",
            ".byte 0x48, 0xCB", // retfq (REX.W + RETF)
            "2:",
            tmp = out(reg) _,
            options(nostack)
        );
    }
}

/// Initialize the GDT with kernel/user segments and load it via `lgdt`.
pub fn init() {
    unsafe {
        // Entry 0 (0x00): Null descriptor
        GDT[0] = make_entry(0, 0, 0, 0);

        // Entry 1 (0x08): Kernel Code 64 — L=1, D=0, Ring 0
        // Access 0x9A = P=1, DPL=0, S=1, Type=1010 (code, exec/read)
        // Flags 0x2 = G=0, D=0, L=1, AVL=0
        GDT[1] = make_entry(0, 0, 0x9A, 0x2);

        // Entry 2 (0x10): Kernel Data — Ring 0
        // Access 0x92 = P=1, DPL=0, S=1, Type=0010 (data, r/w)
        GDT[2] = make_entry(0, 0xFFFFF, 0x92, 0xC);

        // Entry 3 (0x18): User Code 32 compat — L=0, D=1, Ring 3
        // Access 0xFA = P=1, DPL=3, S=1, Type=1010 (code, exec/read)
        // Flags 0xC = G=1, D=1, L=0, AVL=0
        GDT[3] = make_entry(0, 0xFFFFF, 0xFA, 0xC);

        // Entry 4 (0x20): User Data — Ring 3
        // Access 0xF2 = P=1, DPL=3, S=1, Type=0010 (data, r/w)
        GDT[4] = make_entry(0, 0xFFFFF, 0xF2, 0xC);

        // Entry 5 (0x28): User Code 64 — L=1, D=0, Ring 3
        // Access 0xFA = P=1, DPL=3, S=1, Type=1010 (code, exec/read)
        // Flags 0x2 = G=0, D=0, L=1, AVL=0
        GDT[5] = make_entry(0, 0, 0xFA, 0x2);

        // Entries 6+ reserved for per-CPU TSS (filled by tss::init / tss::init_for_cpu)

        GDT_DESC = GdtDescriptor {
            size: (GDT_ENTRIES * size_of::<GdtEntry>() - 1) as u16,
            offset: GDT.as_ptr() as u64,
        };

        // Load GDT
        asm!(
            "lgdt [{}]",
            in(reg) &GDT_DESC as *const GdtDescriptor,
            options(nostack, preserves_flags)
        );

        // Reload data segment registers
        asm!(
            "mov ax, 0x10",  // Kernel data segment
            "mov ds, ax",
            "mov es, ax",
            "mov fs, ax",
            "mov gs, ax",
            "mov ss, ax",
            options(nostack)
        );

        // Far jump to reload CS with kernel code64 segment (0x08)
        asm!(
            "push 0x08",
            "lea {tmp}, [rip + 2f]",
            "push {tmp}",
            ".byte 0x48, 0xCB", // retfq (REX.W + RETF)
            "2:",
            tmp = out(reg) _,
            options(nostack)
        );
    }
}
