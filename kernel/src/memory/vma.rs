//! Per-process Virtual Memory Area (VMA) registry.
//!
//! Tracks all mmap regions for each process using a `BTreeMap<u32, Vma>` sorted
//! by start address.  Provides gap-finding for `sys_mmap` (first-fit with hint),
//! split/trim for `sys_munmap`, deep-clone for `sys_fork`, and bulk cleanup for
//! process exit.
//!
//! Replaces the old bump-pointer (`mmap_next`) allocator that could never reuse
//! freed virtual address space.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use crate::memory::address::PhysAddr;
use crate::sync::spinlock::Spinlock;

/// Start of the user-space mmap virtual address region.
const MMAP_BASE: u32 = 0x7000_0000;
/// End (exclusive) of the user-space mmap virtual address region.
const MMAP_LIMIT: u32 = 0xBF00_0000;

/// A single contiguous mapped region in user virtual address space.
#[derive(Clone)]
pub struct Vma {
    /// Page-aligned start address.
    pub start: u32,
    /// Size in bytes (page-aligned, >0).
    pub size: u32,
    /// Page table flags used when mapping (PAGE_WRITABLE | PAGE_USER, etc.).
    pub flags: u32,
}

/// Per-process VMA state, identified by page directory physical address.
struct ProcessVmas {
    /// Page directory physical address (unique per process).
    pd: PhysAddr,
    /// All mmap regions, keyed by start address.
    vmas: BTreeMap<u32, Vma>,
    /// Hint for next allocation search (replaces the old mmap_next bump pointer).
    mmap_hint: u32,
}

/// Global registry of per-process VMA tables.
///
/// Typical system has <20 processes, so `Vec` with linear scan by PD address
/// is simpler and faster than a `BTreeMap<PhysAddr, _>`.
static VMA_REGISTRY: Spinlock<Vec<ProcessVmas>> = Spinlock::new(Vec::new());

// ─────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────

/// Create an empty VMA table for a new process.
///
/// Called from `load_and_run_with_args()` after the page directory is created.
/// `mmap_hint` includes ASLR randomization.
pub fn init_process(pd: PhysAddr, mmap_hint: u32) {
    let mut reg = VMA_REGISTRY.lock();
    // Guard: don't double-init (shouldn't happen, but be safe).
    if reg.iter().any(|p| p.pd == pd) {
        return;
    }
    reg.push(ProcessVmas {
        pd,
        vmas: BTreeMap::new(),
        mmap_hint: mmap_hint.max(MMAP_BASE),
    });
}

/// Allocate a contiguous region of `size` bytes in the mmap address space.
///
/// Uses first-fit gap search starting from `mmap_hint`.  If no gap is found
/// beyond the hint, wraps around to `MMAP_BASE` for a second pass.
/// Returns the start address on success, or `None` if the address space is
/// truly exhausted.
pub fn alloc_region(pd: PhysAddr, size: u32) -> Option<u32> {
    if size == 0 {
        return None;
    }
    let mut reg = VMA_REGISTRY.lock();
    let proc = reg.iter_mut().find(|p| p.pd == pd)?;

    // Try from hint first, then wrap around.
    if let Some(addr) = find_gap(&proc.vmas, proc.mmap_hint, size) {
        proc.vmas.insert(addr, Vma { start: addr, size, flags: 0x02 | 0x04 });
        proc.mmap_hint = addr + size;
        return Some(addr);
    }
    // Wrap-around: search from MMAP_BASE up to the original hint.
    if proc.mmap_hint > MMAP_BASE {
        if let Some(addr) = find_gap(&proc.vmas, MMAP_BASE, size) {
            proc.vmas.insert(addr, Vma { start: addr, size, flags: 0x02 | 0x04 });
            proc.mmap_hint = addr + size;
            return Some(addr);
        }
    }
    None
}

/// Remove a VMA allocation, supporting partial unmaps (trim / hole-punch).
///
/// The physical page unmap + free is done by the caller (`sys_munmap`); this
/// function only updates the bookkeeping.
pub fn free_region(pd: PhysAddr, addr: u32, size: u32) {
    if size == 0 {
        return;
    }
    let mut reg = VMA_REGISTRY.lock();
    let proc = match reg.iter_mut().find(|p| p.pd == pd) {
        Some(p) => p,
        None => return,
    };

    let free_end = addr + size;

    // Collect keys of VMAs that overlap [addr, free_end).
    let overlapping: Vec<u32> = proc.vmas.range(..free_end)
        .filter(|(_, v)| v.start + v.size > addr)
        .map(|(&k, _)| k)
        .collect();

    for key in overlapping {
        let vma = match proc.vmas.remove(&key) {
            Some(v) => v,
            None => continue,
        };
        let vma_end = vma.start + vma.size;

        // Left remainder: [vma.start, addr) survives.
        if vma.start < addr {
            proc.vmas.insert(vma.start, Vma {
                start: vma.start,
                size: addr - vma.start,
                flags: vma.flags,
            });
        }
        // Right remainder: [free_end, vma_end) survives.
        if vma_end > free_end {
            proc.vmas.insert(free_end, Vma {
                start: free_end,
                size: vma_end - free_end,
                flags: vma.flags,
            });
        }
    }
}

/// Deep-copy all VMAs from `src_pd` to `dst_pd` (fork).
///
/// The child inherits the same VMA layout and `mmap_hint` as the parent.
pub fn clone_for_fork(src_pd: PhysAddr, dst_pd: PhysAddr) {
    let mut reg = VMA_REGISTRY.lock();
    // Find source and clone its data.
    let (cloned_vmas, hint) = match reg.iter().find(|p| p.pd == src_pd) {
        Some(src) => (src.vmas.clone(), src.mmap_hint),
        None => (BTreeMap::new(), MMAP_BASE),
    };
    // Remove stale entry for dst_pd if one exists (shouldn't, but be safe).
    reg.retain(|p| p.pd != dst_pd);
    reg.push(ProcessVmas {
        pd: dst_pd,
        vmas: cloned_vmas,
        mmap_hint: hint,
    });
}

/// Remove the entire VMA table for a terminated process.
///
/// Called from `DEFERRED_PD_DESTROY` drain after `destroy_user_page_directory`.
pub fn destroy_process(pd: PhysAddr) {
    let mut reg = VMA_REGISTRY.lock();
    reg.retain(|p| p.pd != pd);
}

/// Read the current mmap_hint for a process (used by fork snapshot).
pub fn get_mmap_hint(pd: PhysAddr) -> u32 {
    let reg = VMA_REGISTRY.lock();
    reg.iter().find(|p| p.pd == pd).map_or(MMAP_BASE, |p| p.mmap_hint)
}

/// Set the mmap_hint for a process (used after fork child setup).
pub fn set_mmap_hint(pd: PhysAddr, hint: u32) {
    let mut reg = VMA_REGISTRY.lock();
    if let Some(proc) = reg.iter_mut().find(|p| p.pd == pd) {
        proc.mmap_hint = hint;
    }
}

// ─────────────────────────────────────────────────────────────
// Internal helpers
// ─────────────────────────────────────────────────────────────

/// First-fit gap search within `[start_from, MMAP_LIMIT)`.
///
/// Walks the sorted VMA map and looks for gaps between consecutive regions
/// (and before the first / after the last region) that can fit `size` bytes.
fn find_gap(vmas: &BTreeMap<u32, Vma>, start_from: u32, size: u32) -> Option<u32> {
    let start_from = start_from.max(MMAP_BASE);

    // Check gap before the first VMA (or the entire range if empty).
    let mut cursor = start_from;

    // Iterate VMAs that could affect gaps at or after start_from.
    // We need all VMAs whose end > start_from, but BTreeMap is keyed by start.
    // The simplest correct approach: iterate all VMAs in order.
    for vma in vmas.values() {
        let vma_end = vma.start + vma.size;

        // Skip VMAs entirely before our cursor.
        if vma_end <= cursor {
            continue;
        }

        // If this VMA starts after cursor, there's a gap [cursor, vma.start).
        if vma.start > cursor {
            let gap = vma.start - cursor;
            if gap >= size {
                return Some(cursor);
            }
        }

        // Advance cursor past this VMA.
        cursor = cursor.max(vma_end);
    }

    // Check trailing gap after all VMAs.
    if cursor < MMAP_LIMIT {
        let gap = MMAP_LIMIT - cursor;
        if gap >= size {
            return Some(cursor);
        }
    }

    None
}
