//! Per-process Virtual Memory Area (VMA) registry.
//!
//! Tracks all mmap regions for each process as a start-address sorted list of
//! slab-backed VMA nodes. Provides gap-finding for `sys_mmap` (first-fit with
//! hint), split/trim for `sys_munmap`, deep-clone for `sys_fork`, and bulk
//! cleanup for process exit.
//!
//! Replaces the old bump-pointer (`mmap_next`) allocator that could never reuse
//! freed virtual address space.

use crate::memory::address::PhysAddr;
use crate::memory::slab::{KBox, KmemCache};
use crate::memory::user_vmap::{MMAP64_BASE, MMAP_BASE, MMAP_LIMIT};
use crate::sync::spinlock::Spinlock;
use alloc::vec::Vec;

/// End (exclusive) of the native 64-bit high mmap region.
const MMAP64_LIMIT: u64 = 0x0000_4000_0000_0000;

/// A single contiguous mapped region in user virtual address space.
#[derive(Clone)]
pub struct Vma {
    /// Page-aligned start address.
    pub start: u64,
    /// Size in bytes (page-aligned, >0).
    pub size: u64,
    /// Page table flags used when mapping (PAGE_WRITABLE | PAGE_USER, etc.).
    pub flags: u32,
}

struct VmaNode {
    vma: Vma,
    next: Option<VmaNodeBox>,
}

type VmaNodeBox = KBox<VmaNode>;

static VMA_NODE_CACHE: Spinlock<KmemCache> = Spinlock::new(KmemCache::new(
    "memory::VmaNode",
    core::mem::size_of::<VmaNode>(),
    core::mem::align_of::<VmaNode>(),
));

/// Per-process VMA state, identified by page directory physical address.
struct ProcessVmas {
    /// Page directory physical address (unique per process).
    pd: PhysAddr,
    /// All mmap regions, sorted by start address.
    vmas: Option<VmaNodeBox>,
    /// Hint for next allocation search (replaces the old mmap_next bump pointer).
    mmap_hint: u64,
    /// Hint for native 64-bit high mmap allocations.
    mmap64_hint: u64,
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
pub fn init_process(pd: PhysAddr, mmap_hint: u64) {
    let mut reg = VMA_REGISTRY.lock();
    // Guard: don't double-init (shouldn't happen, but be safe).
    if reg.iter().any(|p| p.pd == pd) {
        return;
    }
    reg.push(ProcessVmas {
        pd,
        vmas: None,
        mmap_hint: mmap_hint.max(MMAP_BASE),
        mmap64_hint: MMAP64_BASE,
    });
}

/// Allocate a contiguous region of `size` bytes in the mmap address space.
///
/// Uses first-fit gap search starting from `mmap_hint`.  If no gap is found
/// beyond the hint, wraps around to `MMAP_BASE` for a second pass.
/// Returns the start address on success, or `None` if the address space is
/// truly exhausted.
pub fn alloc_region(pd: PhysAddr, size: u32) -> Option<u32> {
    alloc_region_in(pd, size as u64, false).and_then(|addr| u32::try_from(addr).ok())
}

/// Allocate a native 64-bit mapping above 4 GiB.
pub fn alloc_region64(pd: PhysAddr, size: u64) -> Option<u64> {
    alloc_region_in(pd, size, true)
}

/// Reserve a specific mmap range after the caller has unmapped any overlap.
pub fn alloc_fixed_region64(pd: PhysAddr, addr: u64, size: u64) -> bool {
    if size == 0 || (addr & 0xFFF) != 0 {
        return false;
    }
    let end = match addr.checked_add(size) {
        Some(end) => end,
        None => return false,
    };
    let (base, limit) = if addr >= MMAP64_BASE {
        (MMAP64_BASE, MMAP64_LIMIT)
    } else {
        (MMAP_BASE, MMAP_LIMIT)
    };
    if addr < base || end > limit {
        return false;
    }

    let mut reg = VMA_REGISTRY.lock();
    let proc = match reg.iter_mut().find(|p| p.pd == pd) {
        Some(proc) => proc,
        None => return false,
    };
    if vma_overlaps(&proc.vmas, addr, end) {
        return false;
    }
    insert_vma_sorted(
        &mut proc.vmas,
        Vma {
            start: addr,
            size,
            flags: 0x02 | 0x04,
        },
    )
}

fn alloc_region_in(pd: PhysAddr, size: u64, high: bool) -> Option<u64> {
    if size == 0 {
        return None;
    }
    let mut reg = VMA_REGISTRY.lock();
    let proc = reg.iter_mut().find(|p| p.pd == pd)?;
    let (base, limit, hint) = if high {
        (MMAP64_BASE, MMAP64_LIMIT, proc.mmap64_hint)
    } else {
        (MMAP_BASE, MMAP_LIMIT, proc.mmap_hint)
    };

    // Try from hint first, then wrap around.
    if let Some(addr) = find_gap(&proc.vmas, hint, size, base, limit) {
        if !insert_vma_sorted(
            &mut proc.vmas,
            Vma {
                start: addr,
                size,
                flags: 0x02 | 0x04,
            },
        ) {
            return None;
        }
        if high {
            proc.mmap64_hint = addr + size;
        } else {
            proc.mmap_hint = addr + size;
        }
        return Some(addr);
    }
    // Wrap-around: search from MMAP_BASE up to the original hint.
    if hint > base {
        if let Some(addr) = find_gap(&proc.vmas, base, size, base, hint.min(limit)) {
            if !insert_vma_sorted(
                &mut proc.vmas,
                Vma {
                    start: addr,
                    size,
                    flags: 0x02 | 0x04,
                },
            ) {
                return None;
            }
            if high {
                proc.mmap64_hint = addr + size;
            } else {
                proc.mmap_hint = addr + size;
            }
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
    free_region64(pd, addr as u64, size as u64);
}

/// Remove a VMA allocation in either mmap region.
pub fn free_region64(pd: PhysAddr, addr: u64, size: u64) {
    if size == 0 {
        return;
    }
    let mut reg = VMA_REGISTRY.lock();
    let proc = match reg.iter_mut().find(|p| p.pd == pd) {
        Some(p) => p,
        None => return,
    };

    let free_end = addr + size;

    let old_vmas = collect_vmas(&mut proc.vmas);
    for vma in old_vmas {
        let vma_end = vma.start + vma.size;
        if vma.start >= free_end || vma_end <= addr {
            let _ = insert_vma_sorted(&mut proc.vmas, vma);
            continue;
        }

        // Left remainder: [vma.start, addr) survives.
        if vma.start < addr {
            let _ = insert_vma_sorted(
                &mut proc.vmas,
                Vma {
                    start: vma.start,
                    size: addr - vma.start,
                    flags: vma.flags,
                },
            );
        }
        // Right remainder: [free_end, vma_end) survives.
        if vma_end > free_end {
            let _ = insert_vma_sorted(
                &mut proc.vmas,
                Vma {
                    start: free_end,
                    size: vma_end - free_end,
                    flags: vma.flags,
                },
            );
        }
    }
}

/// Deep-copy all VMAs from `src_pd` to `dst_pd` (fork).
///
/// The child inherits the same VMA layout and `mmap_hint` as the parent.
pub fn clone_for_fork(src_pd: PhysAddr, dst_pd: PhysAddr) {
    let mut reg = VMA_REGISTRY.lock();
    // Find source and clone its data.
    let (cloned_vmas, hint, hint64) = match reg.iter().find(|p| p.pd == src_pd) {
        Some(src) => (snapshot_vmas(&src.vmas), src.mmap_hint, src.mmap64_hint),
        None => (Vec::new(), MMAP_BASE, MMAP64_BASE),
    };
    // Remove stale entry for dst_pd if one exists (shouldn't, but be safe).
    reg.retain(|p| p.pd != dst_pd);
    let mut child = ProcessVmas {
        pd: dst_pd,
        vmas: None,
        mmap_hint: hint,
        mmap64_hint: hint64,
    };
    for vma in cloned_vmas {
        let _ = insert_vma_sorted(&mut child.vmas, vma);
    }
    reg.push(child);
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
    reg.iter()
        .find(|p| p.pd == pd)
        .map_or(MMAP_BASE as u32, |p| p.mmap_hint as u32)
}

/// Set the mmap_hint for a process (used after fork child setup).
pub fn set_mmap_hint(pd: PhysAddr, hint: u32) {
    let mut reg = VMA_REGISTRY.lock();
    if let Some(proc) = reg.iter_mut().find(|p| p.pd == pd) {
        proc.mmap_hint = hint as u64;
    }
}

/// Return true if `addr` lies inside a live mmap VMA for `pd`.
pub fn contains_addr(pd: PhysAddr, addr: u64) -> bool {
    let reg = VMA_REGISTRY.lock();
    let proc = match reg.iter().find(|p| p.pd == pd) {
        Some(proc) => proc,
        None => return false,
    };

    proc.vmas
        .as_deref()
        .map_or(false, |head| contains_addr_in(head, addr))
}

// ─────────────────────────────────────────────────────────────
// Internal helpers
// ─────────────────────────────────────────────────────────────

fn insert_vma_sorted(head: &mut Option<VmaNodeBox>, vma: Vma) -> bool {
    let mut cursor = head;
    while cursor
        .as_ref()
        .map_or(false, |node| node.vma.start < vma.start)
    {
        cursor = &mut cursor.as_mut().expect("cursor vanished").next;
    }

    let mut node = match KBox::new_in(VmaNode { vma, next: None }, &VMA_NODE_CACHE) {
        Some(node) => node,
        None => return false,
    };
    node.next = cursor.take();
    *cursor = Some(node);
    true
}

fn vma_overlaps(head: &Option<VmaNodeBox>, start: u64, end: u64) -> bool {
    let mut cur = head.as_deref();
    while let Some(node) = cur {
        let vma_end = node.vma.start + node.vma.size;
        if node.vma.start < end && vma_end > start {
            return true;
        }
        cur = node.next.as_deref();
    }
    false
}

fn collect_vmas(head: &mut Option<VmaNodeBox>) -> Vec<Vma> {
    let mut out = Vec::new();
    let mut cur = head.take();
    while let Some(mut node) = cur {
        out.push(node.vma.clone());
        cur = node.next.take();
    }
    out
}

fn snapshot_vmas(head: &Option<VmaNodeBox>) -> Vec<Vma> {
    let mut out = Vec::new();
    let mut cur = head.as_deref();
    while let Some(node) = cur {
        out.push(node.vma.clone());
        cur = node.next.as_deref();
    }
    out
}

fn contains_addr_in(mut node: &VmaNode, addr: u64) -> bool {
    loop {
        if node.vma.start <= addr && addr < node.vma.start + node.vma.size {
            return true;
        }
        if node.vma.start > addr {
            return false;
        }
        match node.next.as_deref() {
            Some(next) => node = next,
            None => return false,
        }
    }
}

/// First-fit gap search within `[start_from, MMAP_LIMIT)`.
///
/// Walks the sorted VMA list and looks for gaps between consecutive regions
/// (and before the first / after the last region) that can fit `size` bytes.
fn find_gap(
    vmas: &Option<VmaNodeBox>,
    start_from: u64,
    size: u64,
    base: u64,
    limit: u64,
) -> Option<u64> {
    let start_from = start_from.max(base);

    // Check gap before the first VMA (or the entire range if empty).
    let mut cursor = start_from;

    let mut cur = vmas.as_deref();
    while let Some(node) = cur {
        let vma = &node.vma;
        let vma_end = vma.start + vma.size;
        cur = node.next.as_deref();

        // Skip VMAs entirely before our cursor.
        if vma_end <= cursor || vma.start >= limit {
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
    if cursor < limit {
        let gap = limit - cursor;
        if gap >= size {
            return Some(cursor);
        }
    }

    None
}
