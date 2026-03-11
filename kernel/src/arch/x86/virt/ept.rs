//! EPT (Intel) / NPT (AMD) page table management.
//!
//! Provides 4-level page tables for guest physical → host physical translation.
//! EPT uses Intel-specific entry format; NPT uses standard x86-64 page table format.

use super::{alloc_page_zeroed, free_page};

const PAGE_SIZE: u64 = 4096;
const ENTRIES_PER_TABLE: usize = 512;
const ADDR_MASK: u64 = 0x000F_FFFF_FFFF_F000; // bits [51:12]

// ── EPT (Intel) ──────────────────────────────────────────────────────────

// EPT entry bits
const EPT_READ: u64 = 1 << 0;
const EPT_WRITE: u64 = 1 << 1;
const EPT_EXECUTE: u64 = 1 << 2;
const EPT_MEMTYPE_WB: u64 = 6 << 3; // Write-back memory type (bits [5:3])

/// Create a new EPT PML4 root. Returns physical address of the PML4.
pub fn create_ept_root() -> Option<u64> {
    alloc_page_zeroed()
}

/// Destroy an EPT hierarchy, freeing all pages.
pub fn destroy_ept(root: u64) {
    unsafe { free_table(root, 4); }
}

/// Map a range of guest physical addresses to host physical addresses in EPT.
pub fn ept_map_range(root: u64, gpa: u64, hpa: u64, size: u64, writable: bool, executable: bool) {
    let mut offset: u64 = 0;
    while offset < size {
        let guest = gpa + offset;
        let host = hpa + offset;
        ept_map_page(root, guest, host, writable, executable);
        offset += PAGE_SIZE;
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
        let mut table = root as *mut u64;

        // Walk PML4 → PDPT → PD, creating intermediate tables as needed
        for level in 0..3 {
            let entry = table.add(indices[level]);
            if *entry == 0 {
                let new_table = match alloc_page_zeroed() {
                    Some(p) => p,
                    None => return,
                };
                // Intermediate EPT entries: R+W+X so the walk succeeds
                *entry = new_table | EPT_READ | EPT_WRITE | EPT_EXECUTE;
            }
            table = (*entry & ADDR_MASK) as *mut u64;
        }

        // Write leaf entry
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

// ── NPT (AMD) ────────────────────────────────────────────────────────────

// NPT uses standard x86-64 PTE format
const NPT_PRESENT: u64 = 1 << 0;
const NPT_RW: u64 = 1 << 1;
const NPT_USER: u64 = 1 << 2;
const NPT_ACCESSED: u64 = 1 << 5;
const NPT_DIRTY: u64 = 1 << 6;

/// Create a new NPT PML4 root. Returns physical address.
pub fn create_npt_root() -> Option<u64> {
    alloc_page_zeroed()
}

/// Destroy an NPT hierarchy.
pub fn destroy_npt(root: u64) {
    unsafe { free_table(root, 4); }
}

/// Map a range in NPT.
pub fn npt_map_range(root: u64, gpa: u64, hpa: u64, size: u64, writable: bool, _executable: bool) {
    let mut offset: u64 = 0;
    while offset < size {
        npt_map_page(root, gpa + offset, hpa + offset, writable);
        offset += PAGE_SIZE;
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
        let mut table = root as *mut u64;

        for level in 0..3 {
            let entry = table.add(indices[level]);
            if *entry & NPT_PRESENT == 0 {
                let new_table = match alloc_page_zeroed() {
                    Some(p) => p,
                    None => return,
                };
                *entry = new_table | NPT_PRESENT | NPT_RW | NPT_USER;
            }
            table = (*entry & ADDR_MASK) as *mut u64;
        }

        let entry = table.add(indices[3]);
        let mut flags = NPT_PRESENT | NPT_USER | NPT_ACCESSED | NPT_DIRTY;
        if writable {
            flags |= NPT_RW;
        }
        *entry = (hpa & ADDR_MASK) | flags;
    }
}

// ── Shared teardown ──────────────────────────────────────────────────────

/// Recursively free a page table hierarchy.
/// `level`: 4 = PML4, 3 = PDPT, 2 = PD, 1 = PT (leaf, just free the page).
unsafe fn free_table(phys: u64, level: u32) {
    if phys == 0 {
        return;
    }

    if level > 1 {
        let table = phys as *const u64;
        for i in 0..ENTRIES_PER_TABLE {
            let entry = *table.add(i);
            if entry != 0 {
                let child = entry & ADDR_MASK;
                if child != 0 {
                    free_table(child, level - 1);
                }
            }
        }
    }

    free_page(phys);
}
