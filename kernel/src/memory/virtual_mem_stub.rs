//! ARM64 virtual memory — VMSAv8-A 4-level page table implementation.
//!
//! Uses TTBR0_EL1 for user-space (lower half) and TTBR1_EL1 for kernel (upper half).
//! The kernel is mapped via a 1 GiB block in TTBR1 by boot.S, so kernel-range
//! `map_page` calls are effectively no-ops.  User-space pages use full L0→L3 walks.
//!
//! Physical-to-virtual conversion for page table access uses the 1 GiB block mapping:
//!   VA 0xFFFF_0000_8000_0000 → PA 0x4000_0000 (boot.S PUD[0])

use crate::boot_info::BootInfo;
use crate::memory::address::{PhysAddr, VirtAddr};
use crate::memory::physical;
use crate::memory::FRAME_SIZE;

// ==========================================================================
// Page flag constants (x86-compatible interface used by rest of kernel)
// ==========================================================================

/// Page is present / valid.
pub const PAGE_PRESENT: u64 = 1 << 0;
/// Page is writable.
pub const PAGE_WRITABLE: u64 = 1 << 1;
/// Page is accessible from user mode (EL0).
pub const PAGE_USER: u64 = 1 << 2;
/// Page is not executable (NX).
pub const PAGE_NX: u64 = 1 << 63;

// ==========================================================================
// Address layout constants
// ==========================================================================

/// Physical-to-virtual offset for kernel RAM access.
/// boot.S maps: VA 0xFFFF_0000_8000_0000 → PA 0x4000_0000 (1 GiB block).
pub const PHYS_TO_VIRT_OFFSET: u64 = 0xFFFF_0000_4000_0000;

/// Start of kernel higher-half mapping (1 GiB block via TTBR1 PUD[0]).
const KERNEL_VIRT_BASE: u64 = 0xFFFF_0000_8000_0000;

/// End of kernel higher-half RAM mapping (1 GiB block boundary).
const KERNEL_VIRT_END: u64 = 0xFFFF_0000_C000_0000;

/// Physical RAM base on QEMU virt machine.
const RAM_PHYS_BASE: u64 = 0x4000_0000;

// ==========================================================================
// VMSAv8-A descriptor bits
// ==========================================================================

const DESC_VALID: u64 = 1 << 0;
/// L0-L2: marks a table descriptor (next-level pointer).
const DESC_TABLE: u64 = 1 << 1;
/// L3: marks a page descriptor (final 4 KiB page).
const DESC_PAGE: u64 = 1 << 1;
/// Access Flag — must be set to avoid access-flag faults.
const DESC_AF: u64 = 1 << 10;
/// Inner Shareable (required for SMP coherency).
const DESC_SH_ISH: u64 = 3 << 8;

// MAIR attribute indices (matching boot.S MAIR_EL1 setup)
/// AttrIndx=0: Device-nGnRnE (strongly-ordered MMIO).
const DESC_ATTR_DEV: u64 = 0 << 2;
/// AttrIndx=2: Normal Write-Back Cacheable.
const DESC_ATTR_WB: u64 = 2 << 2;

// Access permissions (AP[7:6])
/// EL1 R/W, EL0 no access.
const DESC_AP_RW_EL1: u64 = 0b00 << 6;
/// EL1+EL0 R/W.
const DESC_AP_RW_ALL: u64 = 0b01 << 6;
/// EL1 R/O, EL0 no access.
const DESC_AP_RO_EL1: u64 = 0b10 << 6;
/// EL1+EL0 R/O.
const DESC_AP_RO_ALL: u64 = 0b11 << 6;

/// Privileged Execute-Never.
const DESC_PXN: u64 = 1 << 53;
/// Unprivileged Execute-Never.
const DESC_UXN: u64 = 1 << 54;
/// Not-Global — ASID-specific TLB entries for user pages.
const DESC_NG: u64 = 1 << 11;

/// Entries per 4 KiB page table (512 × 8 bytes = 4096).
const ENTRIES_PER_TABLE: usize = 512;

/// Physical address mask for descriptors (bits 47:12).
const ADDR_MASK: u64 = 0x0000_FFFF_FFFF_F000;

// ==========================================================================
// Page table index extraction (48-bit VA, 4 KiB granule)
// ==========================================================================

#[inline]
fn l0_index(va: u64) -> usize {
    ((va >> 39) & 0x1FF) as usize
}
#[inline]
fn l1_index(va: u64) -> usize {
    ((va >> 30) & 0x1FF) as usize
}
#[inline]
fn l2_index(va: u64) -> usize {
    ((va >> 21) & 0x1FF) as usize
}
#[inline]
fn l3_index(va: u64) -> usize {
    ((va >> 12) & 0x1FF) as usize
}

// ==========================================================================
// Physical ↔ Virtual address conversion
// ==========================================================================

/// Convert a physical address (in RAM range) to a kernel virtual pointer.
#[inline]
fn phys_to_virt_ptr(phys: u64) -> *mut u64 {
    (phys + PHYS_TO_VIRT_OFFSET) as *mut u64
}

/// Convert a kernel virtual address to a physical address.
#[inline]
fn virt_to_phys(virt: u64) -> u64 {
    virt - PHYS_TO_VIRT_OFFSET
}

/// Check if a virtual address falls within the kernel 1 GiB block mapping.
#[inline]
fn is_kernel_addr(virt: u64) -> bool {
    virt >= KERNEL_VIRT_BASE && virt < KERNEL_VIRT_END
}

// ==========================================================================
// Flag translation: x86-compatible flags → ARM64 descriptor attributes
// ==========================================================================

/// Translate the kernel's x86-compatible page flags to ARM64 lower/upper
/// descriptor attributes for an L3 page entry.
fn flags_to_arm64_attrs(flags: u64) -> u64 {
    let mut attrs: u64 = DESC_AF | DESC_SH_ISH | DESC_ATTR_WB;

    let user = flags & PAGE_USER != 0;
    let writable = flags & PAGE_WRITABLE != 0;

    if user && writable {
        attrs |= DESC_AP_RW_ALL;
    } else if user {
        attrs |= DESC_AP_RO_ALL;
    } else if writable {
        attrs |= DESC_AP_RW_EL1;
    } else {
        attrs |= DESC_AP_RO_EL1;
    }

    if flags & PAGE_NX != 0 {
        attrs |= DESC_UXN | DESC_PXN;
    }

    // User pages are not-global (use ASID for TLB isolation).
    if user {
        attrs |= DESC_NG;
    }

    attrs
}

// ==========================================================================
// Low-level table helpers
// ==========================================================================

/// Allocate a zeroed 4 KiB page-table frame from the physical allocator.
fn alloc_table_frame() -> Option<u64> {
    let frame = physical::alloc_frame()?;
    let phys = frame.as_u64();
    let ptr = phys_to_virt_ptr(phys) as *mut u8;
    unsafe {
        core::ptr::write_bytes(ptr, 0, FRAME_SIZE);
    }
    Some(phys)
}

/// Read one 64-bit entry from a page table at `table_phys[index]`.
#[inline]
unsafe fn read_entry(table_phys: u64, index: usize) -> u64 {
    phys_to_virt_ptr(table_phys).add(index).read_volatile()
}

/// Write one 64-bit entry to a page table at `table_phys[index]`.
#[inline]
unsafe fn write_entry(table_phys: u64, index: usize, value: u64) {
    phys_to_virt_ptr(table_phys).add(index).write_volatile(value);
}

/// Is this descriptor valid (bit 0)?
#[inline]
fn is_valid(desc: u64) -> bool {
    desc & DESC_VALID != 0
}

/// Is this descriptor a table pointer (bits [1:0] == 0b11)?
/// At L0-L2 this means "next-level table"; at L3 it means "page".
#[inline]
fn is_table(desc: u64) -> bool {
    (desc & 0b11) == 0b11
}

/// Extract the physical address from a descriptor.
#[inline]
fn desc_addr(desc: u64) -> u64 {
    desc & ADDR_MASK
}

/// Walk the page table from L0 down to L3, optionally allocating
/// intermediate table frames.  Returns the physical address of the
/// L3 table that covers `va`, or `None` if allocation fails or a
/// required table is missing (when `allocate == false`).
fn walk_to_l3(l0_phys: u64, va: u64, allocate: bool) -> Option<u64> {
    // L0 → L1
    let i0 = l0_index(va);
    let entry0 = unsafe { read_entry(l0_phys, i0) };
    let l1_phys = if is_valid(entry0) && is_table(entry0) {
        desc_addr(entry0)
    } else if allocate {
        let f = alloc_table_frame()?;
        unsafe { write_entry(l0_phys, i0, f | DESC_VALID | DESC_TABLE); }
        f
    } else {
        return None;
    };

    // L1 → L2
    let i1 = l1_index(va);
    let entry1 = unsafe { read_entry(l1_phys, i1) };
    let l2_phys = if is_valid(entry1) && is_table(entry1) {
        desc_addr(entry1)
    } else if allocate {
        let f = alloc_table_frame()?;
        unsafe { write_entry(l1_phys, i1, f | DESC_VALID | DESC_TABLE); }
        f
    } else {
        return None;
    };

    // L2 → L3
    let i2 = l2_index(va);
    let entry2 = unsafe { read_entry(l2_phys, i2) };
    let l3_phys = if is_valid(entry2) && is_table(entry2) {
        desc_addr(entry2)
    } else if allocate {
        let f = alloc_table_frame()?;
        unsafe { write_entry(l2_phys, i2, f | DESC_VALID | DESC_TABLE); }
        f
    } else {
        return None;
    };

    Some(l3_phys)
}

// ==========================================================================
// Public API — matches the interface consumed by the rest of the kernel
// ==========================================================================

/// Initialize virtual memory (ARM64 — boot.S already configured page tables).
pub fn init(_boot_info: &BootInfo) {
    // Nothing to do: boot.S set up TTBR0 (identity) + TTBR1 (kernel).
}

/// Enable PCID (x86-only concept — no-op on ARM64).
pub fn enable_pcid() {}

/// Get the NX flag value for page table entries.
#[inline]
pub fn page_nx_flag() -> u64 {
    PAGE_NX
}

/// Get the kernel page table base (TTBR1_EL1).
pub fn kernel_cr3() -> u64 {
    let ttbr1: u64;
    unsafe {
        core::arch::asm!("mrs {}, ttbr1_el1", out(reg) ttbr1, options(nomem, nostack));
    }
    ttbr1
}

/// Get the current user page table base (TTBR0_EL1).
pub fn current_cr3() -> u64 {
    let ttbr0: u64;
    unsafe {
        core::arch::asm!("mrs {}, ttbr0_el1", out(reg) ttbr0, options(nomem, nostack));
    }
    ttbr0
}

/// Check if a virtual address is mapped.
pub fn is_page_mapped(virt: VirtAddr) -> bool {
    let va = virt.as_u64();
    if is_kernel_addr(va) {
        return true; // 1 GiB block covers the entire kernel range
    }
    let ttbr0 = current_cr3();
    if ttbr0 == 0 {
        return false;
    }
    is_mapped_in_pd(PhysAddr::new(ttbr0), virt)
}

/// Map a single 4 KiB page.
///
/// For kernel addresses (within the 1 GiB block): reserves the backing
/// physical frame in the allocator but does not modify page tables.
/// For user addresses: maps in the current TTBR0.
pub fn map_page(virt: VirtAddr, phys: PhysAddr, flags: u64) {
    let va = virt.as_u64();
    if is_kernel_addr(va) {
        // 1 GiB block already maps this VA to a fixed PA.
        // Reserve the backing frame so the allocator won't hand it out.
        let backing = virt_to_phys(va);
        physical::reserve_frame(PhysAddr::new(backing));
        return;
    }
    let ttbr0 = current_cr3();
    if ttbr0 != 0 {
        map_page_in_pd(PhysAddr::new(ttbr0), virt, phys, flags);
    }
}

/// Unmap a single 4 KiB page.
pub fn unmap_page(virt: VirtAddr) {
    let va = virt.as_u64();
    if is_kernel_addr(va) {
        return; // Cannot unmap within a 1 GiB block
    }
    let ttbr0 = current_cr3();
    if ttbr0 == 0 {
        return;
    }
    if let Some(l3_phys) = walk_to_l3(ttbr0, va, false) {
        let idx = l3_index(va);
        unsafe { write_entry(l3_phys, idx, 0); }
        crate::arch::arm64::mmu::flush_tlb_va(va);
    }
}

/// Read the raw page table entry for a virtual address.
pub fn read_pte(virt: VirtAddr) -> u64 {
    let va = virt.as_u64();
    let ttbr0 = current_cr3();
    if ttbr0 == 0 {
        return 0;
    }
    match walk_to_l3(ttbr0, va, false) {
        Some(l3_phys) => unsafe { read_entry(l3_phys, l3_index(va)) },
        None => 0,
    }
}

/// Check if a page is mapped in a specific user page directory.
pub fn is_mapped_in_pd(pd_phys: PhysAddr, virt: VirtAddr) -> bool {
    let l0 = pd_phys.as_u64();
    match walk_to_l3(l0, virt.as_u64(), false) {
        Some(l3_phys) => {
            let entry = unsafe { read_entry(l3_phys, l3_index(virt.as_u64())) };
            is_valid(entry)
        }
        None => false,
    }
}

/// Map a 4 KiB page in a specific user page directory.
pub fn map_page_in_pd(
    pd_phys: PhysAddr,
    virt: VirtAddr,
    phys: PhysAddr,
    flags: u64,
) {
    let va = virt.as_u64();
    let l3_phys = match walk_to_l3(pd_phys.as_u64(), va, true) {
        Some(l3) => l3,
        None => {
            crate::serial_println!(
                "map_page_in_pd: alloc failed for VA {:#018x}",
                va
            );
            return;
        }
    };

    let attrs = flags_to_arm64_attrs(flags);
    let desc = (phys.as_u64() & ADDR_MASK) | attrs | DESC_VALID | DESC_PAGE;
    unsafe { write_entry(l3_phys, l3_index(va), desc); }
}

/// Map `count` pages starting at `virt_start`, allocating physical frames.
/// When `zero` is true each frame is zeroed after mapping.
pub fn map_pages_range_in_pd(
    pd_phys: PhysAddr,
    virt_start: VirtAddr,
    count: u64,
    flags: u64,
    zero: bool,
) -> Result<u32, &'static str> {
    let mut mapped = 0u32;
    for i in 0..count {
        let va = virt_start.as_u64() + i * FRAME_SIZE as u64;
        let frame = physical::alloc_frame().ok_or("out of physical memory")?;
        map_page_in_pd(pd_phys, VirtAddr::new(va), frame, flags);
        if zero {
            let ptr = phys_to_virt_ptr(frame.as_u64()) as *mut u8;
            unsafe { core::ptr::write_bytes(ptr, 0, FRAME_SIZE); }
        }
        mapped += 1;
    }
    Ok(mapped)
}

/// Create a new (empty) user page directory (L0 table).
pub fn create_user_page_directory() -> Option<PhysAddr> {
    let frame = alloc_table_frame()?;
    Some(PhysAddr::new(frame))
}

/// Deep-copy a user page directory (all levels + page contents).
pub fn clone_user_page_directory(src_pd: PhysAddr) -> Option<PhysAddr> {
    let src_l0 = src_pd.as_u64();
    let dst_l0 = alloc_table_frame()?;

    for i0 in 0..ENTRIES_PER_TABLE {
        let e0 = unsafe { read_entry(src_l0, i0) };
        if !is_valid(e0) || !is_table(e0) {
            continue;
        }
        let src_l1 = desc_addr(e0);
        let dst_l1 = alloc_table_frame()?;
        unsafe { write_entry(dst_l0, i0, dst_l1 | DESC_VALID | DESC_TABLE); }

        for i1 in 0..ENTRIES_PER_TABLE {
            let e1 = unsafe { read_entry(src_l1, i1) };
            if !is_valid(e1) || !is_table(e1) {
                continue;
            }
            let src_l2 = desc_addr(e1);
            let dst_l2 = alloc_table_frame()?;
            unsafe { write_entry(dst_l1, i1, dst_l2 | DESC_VALID | DESC_TABLE); }

            for i2 in 0..ENTRIES_PER_TABLE {
                let e2 = unsafe { read_entry(src_l2, i2) };
                if !is_valid(e2) || !is_table(e2) {
                    continue;
                }
                let src_l3 = desc_addr(e2);
                let dst_l3 = alloc_table_frame()?;
                unsafe { write_entry(dst_l2, i2, dst_l3 | DESC_VALID | DESC_TABLE); }

                for i3 in 0..ENTRIES_PER_TABLE {
                    let e3 = unsafe { read_entry(src_l3, i3) };
                    if !is_valid(e3) {
                        continue;
                    }
                    let src_page = desc_addr(e3);
                    let dst_frame = physical::alloc_frame()?;
                    let dst_page = dst_frame.as_u64();
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            phys_to_virt_ptr(src_page) as *const u8,
                            phys_to_virt_ptr(dst_page) as *mut u8,
                            FRAME_SIZE,
                        );
                    }
                    // Preserve attributes, update physical address
                    let attrs = e3 & !ADDR_MASK;
                    unsafe {
                        write_entry(dst_l3, i3, (dst_page & ADDR_MASK) | attrs);
                    }
                }
            }
        }
    }

    Some(PhysAddr::new(dst_l0))
}

/// Destroy a user page directory, freeing all page tables and mapped pages.
pub fn destroy_user_page_directory(pd: PhysAddr) {
    let l0 = pd.as_u64();
    if l0 == 0 {
        return;
    }

    for i0 in 0..ENTRIES_PER_TABLE {
        let e0 = unsafe { read_entry(l0, i0) };
        if !is_valid(e0) || !is_table(e0) {
            continue;
        }
        let l1 = desc_addr(e0);
        for i1 in 0..ENTRIES_PER_TABLE {
            let e1 = unsafe { read_entry(l1, i1) };
            if !is_valid(e1) || !is_table(e1) {
                continue;
            }
            let l2 = desc_addr(e1);
            for i2 in 0..ENTRIES_PER_TABLE {
                let e2 = unsafe { read_entry(l2, i2) };
                if !is_valid(e2) || !is_table(e2) {
                    continue;
                }
                let l3 = desc_addr(e2);
                // Free all mapped pages
                for i3 in 0..ENTRIES_PER_TABLE {
                    let e3 = unsafe { read_entry(l3, i3) };
                    if is_valid(e3) {
                        physical::free_frame(PhysAddr::new(desc_addr(e3)));
                    }
                }
                physical::free_frame(PhysAddr::new(l3));
            }
            physical::free_frame(PhysAddr::new(l2));
        }
        physical::free_frame(PhysAddr::new(l1));
    }
    physical::free_frame(PhysAddr::new(l0));
}

/// Mark a page as not-present (guard page).
pub fn set_guard_page(virt: VirtAddr) {
    let va = virt.as_u64();
    if is_kernel_addr(va) {
        return;
    }
    let ttbr0 = current_cr3();
    if ttbr0 == 0 {
        return;
    }
    if let Some(l3_phys) = walk_to_l3(ttbr0, va, false) {
        let idx = l3_index(va);
        let entry = unsafe { read_entry(l3_phys, idx) };
        if is_valid(entry) {
            unsafe { write_entry(l3_phys, idx, entry & !DESC_VALID); }
            crate::arch::arm64::mmu::flush_tlb_va(va);
        }
    }
}

/// Restore a guard page to accessible.
pub fn restore_guard_page(virt: VirtAddr) {
    let va = virt.as_u64();
    if is_kernel_addr(va) {
        return;
    }
    let ttbr0 = current_cr3();
    if ttbr0 == 0 {
        return;
    }
    if let Some(l3_phys) = walk_to_l3(ttbr0, va, false) {
        let idx = l3_index(va);
        let entry = unsafe { read_entry(l3_phys, idx) };
        if entry != 0 && !is_valid(entry) {
            unsafe { write_entry(l3_phys, idx, entry | DESC_VALID); }
            crate::arch::arm64::mmu::flush_tlb_va(va);
        }
    }
}

/// Alias for `set_guard_page`.
pub fn guard_page(virt: VirtAddr) {
    set_guard_page(virt);
}

/// Alias for `restore_guard_page`.
pub fn unguard_page(virt: VirtAddr) {
    restore_guard_page(virt);
}

/// Count the number of mapped pages in a VA range within a page directory.
pub fn count_mapped_pages_in_pd(
    pd_phys: PhysAddr,
    start: VirtAddr,
    end: VirtAddr,
) -> usize {
    let l0 = pd_phys.as_u64();
    if l0 == 0 {
        return 0;
    }
    let mut count = 0usize;
    let mut va = start.as_u64() & !0xFFF;
    let end_va = end.as_u64();

    while va < end_va {
        if let Some(l3_phys) = walk_to_l3(l0, va, false) {
            let entry = unsafe { read_entry(l3_phys, l3_index(va)) };
            if is_valid(entry) {
                count += 1;
            }
        }
        va += FRAME_SIZE as u64;
    }
    count
}
