use core::arch::asm;
use core::mem::size_of;

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
    offset: u32,
}

const GDT_ENTRIES: usize = 6;

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

/// Install the TSS descriptor in GDT entry 5 (selector 0x28).
pub fn set_tss_entry(base: u32, limit: u32) {
    unsafe {
        // Access: 0x89 = Present, DPL=0, Available 32-bit TSS
        // Flags: 0x0 = byte granularity
        GDT[5] = make_entry(base, limit, 0x89, 0x0);

        // Reload GDTR (entries changed)
        GDT_DESC = GdtDescriptor {
            size: (GDT_ENTRIES * size_of::<GdtEntry>() - 1) as u16,
            offset: GDT.as_ptr() as u32,
        };
        asm!(
            "lgdt [{}]",
            in(reg) &GDT_DESC as *const GdtDescriptor,
            options(nostack, preserves_flags)
        );
    }
}

/// Reload the kernel GDT and segment registers on the current CPU.
/// Used by APs after trampoline to switch from the minimal trampoline GDT
/// to the full kernel GDT (which includes user segments and TSS).
pub fn reload() {
    unsafe {
        asm!(
            "lgdt [{}]",
            in(reg) &raw const GDT_DESC,
            options(nostack, preserves_flags)
        );

        // Reload segment registers
        asm!(
            "mov ax, 0x10",
            "mov ds, ax",
            "mov es, ax",
            "mov fs, ax",
            "mov gs, ax",
            "mov ss, ax",
            options(nostack)
        );

        // Far jump to reload CS
        asm!(
            "push 0x08",
            "lea eax, [2f]",
            "push eax",
            "retf",
            "2:",
            options(nostack)
        );
    }
}

/// Clear the TSS busy bit in the GDT so `ltr` can be executed again (e.g. by an AP).
/// The busy bit is bit 1 of the access byte in GDT entry 5.
pub fn clear_tss_busy_bit() {
    unsafe {
        let access_ptr = (GDT.as_ptr() as *const u8).add(5 * 8 + 5) as *mut u8;
        let access = core::ptr::read_volatile(access_ptr);
        core::ptr::write_volatile(access_ptr, access & !0x02); // 0x8B -> 0x89
    }
}

pub fn init() {
    unsafe {
        // Entry 0: Null descriptor
        GDT[0] = make_entry(0, 0, 0, 0);

        // Entry 1 (0x08): Kernel Code - base=0, limit=4GiB, ring 0, execute/read
        GDT[1] = make_entry(0, 0xFFFFF, 0x9A, 0xC);

        // Entry 2 (0x10): Kernel Data - base=0, limit=4GiB, ring 0, read/write
        GDT[2] = make_entry(0, 0xFFFFF, 0x92, 0xC);

        // Entry 3 (0x18): User Code - base=0, limit=4GiB, ring 3, execute/read
        GDT[3] = make_entry(0, 0xFFFFF, 0xFA, 0xC);

        // Entry 4 (0x20): User Data - base=0, limit=4GiB, ring 3, read/write
        GDT[4] = make_entry(0, 0xFFFFF, 0xF2, 0xC);

        // Entry 5 (0x28): TSS (will be filled later)
        GDT[5] = make_entry(0, 0, 0, 0);

        GDT_DESC = GdtDescriptor {
            size: (GDT_ENTRIES * size_of::<GdtEntry>() - 1) as u16,
            offset: GDT.as_ptr() as u32,
        };

        // Load GDT
        asm!(
            "lgdt [{}]",
            in(reg) &GDT_DESC as *const GdtDescriptor,
            options(nostack, preserves_flags)
        );

        // Reload segment registers
        asm!(
            "mov ax, 0x10",  // Kernel data segment
            "mov ds, ax",
            "mov es, ax",
            "mov fs, ax",
            "mov gs, ax",
            "mov ss, ax",
            options(nostack)
        );

        // Far jump to reload CS with kernel code segment
        asm!(
            "push 0x08",
            "lea eax, [2f]",
            "push eax",
            "retf",
            "2:",
            options(nostack)
        );
    }
}
