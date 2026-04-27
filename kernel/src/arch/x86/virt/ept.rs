//! EPT (Intel) / NPT (AMD) page table management.
//!
//! Provides 4-level page tables for guest physical → host physical translation.
//! EPT uses Intel-specific entry format; NPT uses standard x86-64 page table format.
//! Both support 4KB pages and 2MB large pages.

use super::{alloc_page_zeroed, free_page, phys_to_virt};

const PAGE_SIZE_4K: u64 = 0x1000;
const PAGE_SIZE_2M: u64 = 0x20_0000;
const ENTRIES_PER_TABLE: usize = 512;
const ADDR_MASK: u64 = 0x000F_FFFF_FFFF_F000; // bits [51:12]

// ── EPT (Intel) ──────────────────────────────────────────────────────────

// EPT entry bits
const EPT_READ: u64 = 1 << 0;
const EPT_WRITE: u64 = 1 << 1;
const EPT_EXECUTE: u64 = 1 << 2;
const EPT_MEMTYPE_WB: u64 = 6 << 3; // Write-back memory type (bits [5:3])
/// Large page bit in an EPT PD entry (2MB).
const EPT_LARGE_PAGE: u64 = 1 << 7;

/// Create a new EPT PML4 root. Returns physical address of the PML4.
pub fn create_ept_root() -> Option<u64> {
    alloc_page_zeroed()
}

/// Destroy an EPT hierarchy, freeing all pages.
pub fn destroy_ept(root: u64) {
    unsafe {
        free_ept_table(root, 4);
    }
}

/// Map a range of guest physical addresses to host physical addresses in EPT.
///
/// Uses 2MB large pages wherever the GPA, HPA, and size are 2MB-aligned;
/// falls back to 4KB pages otherwise.
pub fn ept_map_range(root: u64, gpa: u64, hpa: u64, size: u64, writable: bool, executable: bool) {
    let mut offset: u64 = 0;
    while offset < size {
        let guest = gpa + offset;
        let host = hpa + offset;
        let remaining = size - offset;

        // Use a 2MB page if everything is aligned and enough space remains.
        if remaining >= PAGE_SIZE_2M
            && (guest & (PAGE_SIZE_2M - 1)) == 0
            && (host & (PAGE_SIZE_2M - 1)) == 0
        {
            ept_map_large_page(root, guest, host, writable, executable);
            offset += PAGE_SIZE_2M;
        } else {
            ept_map_page(root, guest, host, writable, executable);
            offset += PAGE_SIZE_4K;
        }
    }
}

/// Map a single 4KB page in EPT.
fn ept_map_page(root: u64, gpa: u64, hpa: u64, writable: bool, executable: bool) {
    let indices = [
        ((gpa >> 39) & 0x1FF) as usize, // PML4
        ((gpa >> 30) & 0x1FF) as usize, // PDPT
        ((gpa >> 21) & 0x1FF) as usize, // PD
        ((gpa >> 12) & 0x1FF) as usize, // PT
    ];

    unsafe {
        let mut table = phys_to_virt(root) as *mut u64;

        // Walk PML4 → PDPT → PD, creating intermediate tables as needed.
        for level in 0..3 {
            let entry = table.add(indices[level]);
            if *entry == 0 {
                let new_phys = match alloc_page_zeroed() {
                    Some(p) => p,
                    None => return,
                };
                // Intermediate EPT entries: R+W+X so the walk succeeds.
                *entry = new_phys | EPT_READ | EPT_WRITE | EPT_EXECUTE;
            }
            let child_phys = *entry & ADDR_MASK;
            table = phys_to_virt(child_phys) as *mut u64;
        }

        // Write leaf (PT) entry.
        let entry = table.add(indices[3]);
        let mut flags = EPT_MEMTYPE_WB | EPT_READ;
        if writable {
            flags |= EPT_WRITE;
        }
        if executable {
            flags |= EPT_EXECUTE;
        }
        *entry = (hpa & ADDR_MASK) | flags;
    }
}

/// Map a single 2MB large page in EPT (PD-level leaf).
fn ept_map_large_page(root: u64, gpa: u64, hpa: u64, writable: bool, executable: bool) {
    let indices = [
        ((gpa >> 39) & 0x1FF) as usize, // PML4
        ((gpa >> 30) & 0x1FF) as usize, // PDPT
        ((gpa >> 21) & 0x1FF) as usize, // PD  ← leaf
    ];

    unsafe {
        let mut table = phys_to_virt(root) as *mut u64;

        // Walk PML4 → PDPT, creating intermediate tables as needed.
        for level in 0..2 {
            let entry = table.add(indices[level]);
            if *entry == 0 {
                let new_phys = match alloc_page_zeroed() {
                    Some(p) => p,
                    None => return,
                };
                *entry = new_phys | EPT_READ | EPT_WRITE | EPT_EXECUTE;
            }
            let child_phys = *entry & ADDR_MASK;
            table = phys_to_virt(child_phys) as *mut u64;
        }

        // Write PD large-page leaf.  Bit 7 = EPT_LARGE_PAGE signals a 2MB leaf.
        let entry = table.add(indices[2]);
        let mut flags = EPT_MEMTYPE_WB | EPT_READ | EPT_LARGE_PAGE;
        if writable {
            flags |= EPT_WRITE;
        }
        if executable {
            flags |= EPT_EXECUTE;
        }
        // 2MB-aligned physical address: bits [51:21].
        *entry = (hpa & 0x000F_FFFF_FFE0_0000) | flags;
    }
}

/// Translate a guest physical address to host physical address using the EPT.
///
/// Walks the 4-level EPT structure and returns the HPA, or `None` if the GPA
/// is not mapped. Handles both 4KB leaf pages and 2MB large pages.
pub fn ept_translate(root: u64, gpa: u64) -> Option<u64> {
    unsafe {
        let pml4_idx = ((gpa >> 39) & 0x1FF) as usize;
        let pdpt_idx = ((gpa >> 30) & 0x1FF) as usize;
        let pd_idx = ((gpa >> 21) & 0x1FF) as usize;
        let pt_idx = ((gpa >> 12) & 0x1FF) as usize;

        let pml4 = phys_to_virt(root) as *const u64;
        let pml4e = *pml4.add(pml4_idx);
        if pml4e & EPT_READ == 0 {
            return None;
        }

        let pdpt = phys_to_virt(pml4e & ADDR_MASK) as *const u64;
        let pdpte = *pdpt.add(pdpt_idx);
        if pdpte & EPT_READ == 0 {
            return None;
        }
        if pdpte & EPT_LARGE_PAGE != 0 {
            // 1GB page (rare — not mapped by our ept_map_range but handle defensively)
            return Some((pdpte & 0x000F_FFFC_0000_0000) | (gpa & 0x3FFF_FFFF));
        }

        let pd = phys_to_virt(pdpte & ADDR_MASK) as *const u64;
        let pde = *pd.add(pd_idx);
        if pde & EPT_READ == 0 {
            return None;
        }
        if pde & EPT_LARGE_PAGE != 0 {
            // 2MB page
            return Some((pde & 0x000F_FFFF_FFE0_0000) | (gpa & 0x1F_FFFF));
        }

        let pt = phys_to_virt(pde & ADDR_MASK) as *const u64;
        let pte = *pt.add(pt_idx);
        if pte & EPT_READ == 0 {
            return None;
        }
        Some((pte & ADDR_MASK) | (gpa & 0xFFF))
    }
}

/// Translate a guest physical address to host physical address using the NPT.
///
/// Uses the standard x86-64 PTE format (NPT_PRESENT = bit 0). Handles 4KB
/// and 2MB large pages.
pub fn npt_translate(root: u64, gpa: u64) -> Option<u64> {
    unsafe {
        let pml4_idx = ((gpa >> 39) & 0x1FF) as usize;
        let pdpt_idx = ((gpa >> 30) & 0x1FF) as usize;
        let pd_idx = ((gpa >> 21) & 0x1FF) as usize;
        let pt_idx = ((gpa >> 12) & 0x1FF) as usize;

        let pml4 = phys_to_virt(root) as *const u64;
        let pml4e = *pml4.add(pml4_idx);
        if pml4e & NPT_PRESENT == 0 {
            return None;
        }

        let pdpt = phys_to_virt(pml4e & ADDR_MASK) as *const u64;
        let pdpte = *pdpt.add(pdpt_idx);
        if pdpte & NPT_PRESENT == 0 {
            return None;
        }
        // 1GB page (PS bit in PDPT, same bit position as NPT_LARGE)
        if pdpte & NPT_LARGE != 0 {
            return Some((pdpte & 0x000F_FFFC_0000_0000) | (gpa & 0x3FFF_FFFF));
        }

        let pd = phys_to_virt(pdpte & ADDR_MASK) as *const u64;
        let pde = *pd.add(pd_idx);
        if pde & NPT_PRESENT == 0 {
            return None;
        }
        if pde & NPT_LARGE != 0 {
            return Some((pde & 0x000F_FFFF_FFE0_0000) | (gpa & 0x1F_FFFF));
        }

        let pt = phys_to_virt(pde & ADDR_MASK) as *const u64;
        let pte = *pt.add(pt_idx);
        if pte & NPT_PRESENT == 0 {
            return None;
        }
        Some((pte & ADDR_MASK) | (gpa & 0xFFF))
    }
}

/// Perform an INVEPT (all-context, type 2) to flush EPT-derived TLB entries.
///
/// Must be called after any EPT mapping change to avoid stale translations.
/// Safe to call even if the CPU doesn't advertise INVEPT support — the
/// secondary-controls bit for EPT implies it.
#[inline]
pub unsafe fn invept_all_context() {
    // INVEPT descriptor: 16 bytes, both fields zero for all-context flush.
    let descriptor: [u64; 2] = [0, 0];
    core::arch::asm!(
        "invept {0}, [{1}]",
        in(reg) 2u64,          // type 2 = all-context
        in(reg) descriptor.as_ptr(),
        options(nostack, preserves_flags),
    );
}

// ── NPT (AMD) ────────────────────────────────────────────────────────────

// NPT uses standard x86-64 PTE format.
const NPT_PRESENT: u64 = 1 << 0;
const NPT_RW: u64 = 1 << 1;
const NPT_USER: u64 = 1 << 2;
const NPT_LARGE: u64 = 1 << 7; // PS bit — 2MB page at PD level
const NPT_ACCESSED: u64 = 1 << 5;
const NPT_DIRTY: u64 = 1 << 6;

/// Create a new NPT PML4 root. Returns physical address.
pub fn create_npt_root() -> Option<u64> {
    alloc_page_zeroed()
}

/// Destroy an NPT hierarchy.
pub fn destroy_npt(root: u64) {
    unsafe {
        free_npt_table(root, 4);
    }
}

/// Map a range in NPT.
///
/// Uses 2MB large pages wherever GPA, HPA, and size are 2MB-aligned.
pub fn npt_map_range(root: u64, gpa: u64, hpa: u64, size: u64, writable: bool, _executable: bool) {
    let mut offset: u64 = 0;
    while offset < size {
        let guest = gpa + offset;
        let host = hpa + offset;
        let remaining = size - offset;

        if remaining >= PAGE_SIZE_2M
            && (guest & (PAGE_SIZE_2M - 1)) == 0
            && (host & (PAGE_SIZE_2M - 1)) == 0
        {
            npt_map_large_page(root, guest, host, writable);
            offset += PAGE_SIZE_2M;
        } else {
            npt_map_page(root, guest, host, writable);
            offset += PAGE_SIZE_4K;
        }
    }
}

/// Map a single 4KB page in NPT.
fn npt_map_page(root: u64, gpa: u64, hpa: u64, writable: bool) {
    let indices = [
        ((gpa >> 39) & 0x1FF) as usize,
        ((gpa >> 30) & 0x1FF) as usize,
        ((gpa >> 21) & 0x1FF) as usize,
        ((gpa >> 12) & 0x1FF) as usize,
    ];

    unsafe {
        let mut table = phys_to_virt(root) as *mut u64;

        for level in 0..3 {
            let entry = table.add(indices[level]);
            if *entry & NPT_PRESENT == 0 {
                let new_phys = match alloc_page_zeroed() {
                    Some(p) => p,
                    None => return,
                };
                *entry = new_phys | NPT_PRESENT | NPT_RW | NPT_USER;
            }
            let child_phys = *entry & ADDR_MASK;
            table = phys_to_virt(child_phys) as *mut u64;
        }

        let entry = table.add(indices[3]);
        let mut flags = NPT_PRESENT | NPT_USER | NPT_ACCESSED | NPT_DIRTY;
        if writable {
            flags |= NPT_RW;
        }
        *entry = (hpa & ADDR_MASK) | flags;
    }
}

/// Map a single 2MB large page in NPT (PD-level PS leaf).
fn npt_map_large_page(root: u64, gpa: u64, hpa: u64, writable: bool) {
    let indices = [
        ((gpa >> 39) & 0x1FF) as usize, // PML4
        ((gpa >> 30) & 0x1FF) as usize, // PDPT
        ((gpa >> 21) & 0x1FF) as usize, // PD  ← leaf
    ];

    unsafe {
        let mut table = phys_to_virt(root) as *mut u64;

        for level in 0..2 {
            let entry = table.add(indices[level]);
            if *entry & NPT_PRESENT == 0 {
                let new_phys = match alloc_page_zeroed() {
                    Some(p) => p,
                    None => return,
                };
                *entry = new_phys | NPT_PRESENT | NPT_RW | NPT_USER;
            }
            let child_phys = *entry & ADDR_MASK;
            table = phys_to_virt(child_phys) as *mut u64;
        }

        let entry = table.add(indices[2]);
        let mut flags = NPT_PRESENT | NPT_USER | NPT_LARGE | NPT_ACCESSED | NPT_DIRTY;
        if writable {
            flags |= NPT_RW;
        }
        // 2MB-aligned physical address.
        *entry = (hpa & 0x000F_FFFF_FFE0_0000) | flags;
    }
}

// ── Teardown helpers ─────────────────────────────────────────────────────

/// Recursively free an EPT hierarchy.
///
/// EPT leaf entries (level 1 = PT) have WB memory-type bits that must not be
/// confused with child-table addresses.  A PT entry is a leaf and carries no
/// child pointer, so we stop recursion at level 2 and only free the PT page
/// itself (its entries are physical frames owned by the guest, not by us).
unsafe fn free_ept_table(phys: u64, level: u32) {
    if phys == 0 {
        return;
    }

    if level > 1 {
        let table = phys_to_virt(phys) as *const u64;
        for i in 0..ENTRIES_PER_TABLE {
            let entry = *table.add(i);
            if entry & EPT_READ != 0 {
                // Skip large-page leaves at PD level (bit 7 set).
                if level == 2 && (entry & EPT_LARGE_PAGE != 0) {
                    continue;
                }
                let child = entry & ADDR_MASK;
                if child != 0 {
                    free_ept_table(child, level - 1);
                }
            }
        }
    }

    free_page(phys);
}

/// Recursively free an NPT hierarchy.
///
/// Large-page PD entries (bit 7 = PS) are leaves and must not be walked.
unsafe fn free_npt_table(phys: u64, level: u32) {
    if phys == 0 {
        return;
    }

    if level > 1 {
        let table = phys_to_virt(phys) as *const u64;
        for i in 0..ENTRIES_PER_TABLE {
            let entry = *table.add(i);
            if entry & NPT_PRESENT != 0 {
                // PD-level large-page leaf (level == 2, bit 7 set): skip recursion.
                if level == 2 && (entry & NPT_LARGE != 0) {
                    continue;
                }
                let child = entry & ADDR_MASK;
                if child != 0 {
                    free_npt_table(child, level - 1);
                }
            }
        }
    }

    free_page(phys);
}
