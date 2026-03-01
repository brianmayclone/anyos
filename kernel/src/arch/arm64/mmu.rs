//! ARM64 MMU configuration (VMSAv8-A).
//!
//! Configures TCR_EL1, MAIR_EL1, and provides helpers for TTBR0/TTBR1 management.
//! The actual page table manipulation (map/unmap/walk) lives in `memory::paging::arm64`.

/// MAIR_EL1 attribute indices.
pub const MAIR_DEVICE_NGNRNE: u8 = 0; // Device-nGnRnE (strongly-ordered MMIO)
pub const MAIR_NORMAL_NC: u8 = 1;     // Normal Non-Cacheable
pub const MAIR_NORMAL_WB: u8 = 2;     // Normal Write-Back Cacheable

/// MAIR_EL1 value — encodes memory attribute types at indices 0-2.
///
/// - Attr0 (0x00): Device-nGnRnE
/// - Attr1 (0x44): Normal Non-Cacheable
/// - Attr2 (0xFF): Normal Write-Back, Read-Allocate, Write-Allocate
const MAIR_VALUE: u64 =
    (0x00u64) |          // Attr0: Device-nGnRnE
    (0x44u64 << 8) |     // Attr1: Normal NC (Inner NC, Outer NC)
    (0xFFu64 << 16);     // Attr2: Normal WB (Inner WB-RW-Alloc, Outer WB-RW-Alloc)

/// TCR_EL1 configuration for 48-bit VA, 4 KiB granule.
///
/// - T0SZ = 16 (48-bit user VA)
/// - T1SZ = 16 (48-bit kernel VA)
/// - TG0 = 0b00 (4 KiB granule for TTBR0)
/// - TG1 = 0b10 (4 KiB granule for TTBR1)
/// - IPS = 0b101 (48-bit PA)
/// - SH0/SH1 = 0b11 (Inner Shareable)
/// - ORGN0/IRGN0/ORGN1/IRGN1 = 0b01 (WB-RW-Alloc)
const TCR_VALUE: u64 = {
    let t0sz: u64 = 16;
    let t1sz: u64 = 16;
    let tg0: u64 = 0b00;   // 4 KiB
    let tg1: u64 = 0b10;   // 4 KiB
    let sh0: u64 = 0b11;   // Inner Shareable
    let sh1: u64 = 0b11;
    let orgn0: u64 = 0b01; // WB-RW-Alloc
    let irgn0: u64 = 0b01;
    let orgn1: u64 = 0b01;
    let irgn1: u64 = 0b01;
    let ips: u64 = 0b101;  // 48-bit PA (256 TiB)

    t0sz
        | (t1sz << 16)
        | (tg0 << 14)
        | (tg1 << 30)
        | (sh0 << 12)
        | (sh1 << 28)
        | (orgn0 << 10)
        | (irgn0 << 8)
        | (orgn1 << 26)
        | (irgn1 << 24)
        | (ips << 32)
};

/// Initialize the MMU configuration registers (TCR_EL1, MAIR_EL1).
///
/// This must be called early in boot, before enabling the MMU.
/// The actual TTBR0/TTBR1 setup and SCTLR_EL1.M enable happens in boot.S.
pub fn init() {
    unsafe {
        // Set MAIR_EL1
        core::arch::asm!("msr mair_el1, {}", in(reg) MAIR_VALUE, options(nostack));

        // Set TCR_EL1
        core::arch::asm!("msr tcr_el1, {}", in(reg) TCR_VALUE, options(nostack));

        // Barrier to ensure configuration is visible
        core::arch::asm!("isb", options(nostack));
    }
    crate::serial_println!("[OK] MMU configured: TCR={:#018x} MAIR={:#018x}", TCR_VALUE, MAIR_VALUE);
}

/// Read TTBR0_EL1 (user page table base).
#[inline]
pub fn read_ttbr0() -> u64 {
    let val: u64;
    unsafe { core::arch::asm!("mrs {}, ttbr0_el1", out(reg) val, options(nomem, nostack)); }
    val
}

/// Write TTBR0_EL1 (user page table base) with barrier.
#[inline]
pub fn write_ttbr0(val: u64) {
    unsafe {
        core::arch::asm!(
            "msr ttbr0_el1, {}",
            "isb",
            in(reg) val,
            options(nostack),
        );
    }
}

/// Read TTBR1_EL1 (kernel page table base).
#[inline]
pub fn read_ttbr1() -> u64 {
    let val: u64;
    unsafe { core::arch::asm!("mrs {}, ttbr1_el1", out(reg) val, options(nomem, nostack)); }
    val
}

/// Write TTBR1_EL1 (kernel page table base) with barrier.
#[inline]
pub fn write_ttbr1(val: u64) {
    unsafe {
        core::arch::asm!(
            "msr ttbr1_el1, {}",
            "isb",
            in(reg) val,
            options(nostack),
        );
    }
}

/// Invalidate the entire TLB on this core (all ASIDs).
#[inline]
pub fn flush_tlb_all() {
    unsafe {
        core::arch::asm!(
            "tlbi vmalle1is",
            "dsb ish",
            "isb",
            options(nostack),
        );
    }
}

/// Invalidate TLB entry for a specific virtual address.
#[inline]
pub fn flush_tlb_va(vaddr: u64) {
    unsafe {
        core::arch::asm!(
            "tlbi vale1is, {}",
            "dsb ish",
            "isb",
            in(reg) vaddr >> 12,
            options(nostack),
        );
    }
}
