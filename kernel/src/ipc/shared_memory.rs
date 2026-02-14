//! Shared memory regions for zero-copy IPC between processes.
//!
//! A shared region is a set of physical frames that can be mapped into multiple
//! process address spaces. Reference counting ensures frames are freed only when
//! all mappings are released and the owner has destroyed the region.
//!
//! Virtual address allocation for SHM mappings starts at [`SHM_BASE`] (0x10000000)
//! and bumps upward per-process, well above DLLs (0x04000000) and program code
//! (0x08000000).

use crate::memory::address::{PhysAddr, VirtAddr};
use crate::memory::physical;
use crate::memory::virtual_mem;
use crate::memory::FRAME_SIZE;
use crate::sync::spinlock::Spinlock;
use alloc::vec::Vec;

/// Base virtual address for SHM mappings in user processes.
const SHM_BASE: u64 = 0x1000_0000;

/// Tracks where an SHM region is mapped in a specific process.
struct ShmMapping {
    tid: u32,
    vaddr: u64,
}

/// A reference-counted shared memory region backed by physical frames.
pub struct SharedRegion {
    /// Unique region identifier (>0).
    pub id: u32,
    /// Backing physical frames in page order.
    pub physical_frames: Vec<PhysAddr>,
    /// Total size in bytes (page-aligned).
    pub size: usize,
    /// TID of the creating process (0 = owner released / destroyed).
    pub owner_tid: u32,
    /// Active per-process mappings.
    mappings: Vec<ShmMapping>,
}

static SHARED_REGIONS: Spinlock<Vec<SharedRegion>> = Spinlock::new(Vec::new());
static NEXT_REGION_ID: Spinlock<u32> = Spinlock::new(1);

/// Create a new shared memory region of the given size (rounded up to page size).
/// Returns the region ID (>0), or `None` on allocation failure.
pub fn create(size: usize, owner_tid: u32) -> Option<u32> {
    let pages = (size + FRAME_SIZE - 1) / FRAME_SIZE;
    if pages == 0 {
        return None;
    }

    let mut frames = Vec::new();
    for _ in 0..pages {
        match physical::alloc_frame() {
            Some(frame) => {
                // Zero the frame via identity mapping to prevent information leaks
                // and avoid garbage pixels in new window surfaces.
                unsafe {
                    core::ptr::write_bytes(frame.as_u64() as *mut u8, 0, FRAME_SIZE);
                }
                frames.push(frame);
            }
            None => {
                for f in &frames {
                    physical::free_frame(*f);
                }
                return None;
            }
        }
    }

    let id = {
        let mut guard = NEXT_REGION_ID.lock();
        let id = *guard;
        *guard += 1;
        id
    };

    let region = SharedRegion {
        id,
        physical_frames: frames,
        size: pages * FRAME_SIZE,
        owner_tid,
        mappings: Vec::new(),
    };

    SHARED_REGIONS.lock().push(region);
    Some(id)
}

/// Map a shared memory region into the calling process's address space.
///
/// Uses recursive mapping on the current CR3 (which is the caller's PD during
/// a syscall). Returns the virtual address where the region was mapped, or 0
/// on failure. If the region is already mapped by this TID, returns the
/// existing mapping address.
pub fn map_into_current(region_id: u32) -> u64 {
    let tid = crate::task::scheduler::current_tid();
    let mut regions = SHARED_REGIONS.lock();

    let idx = match regions.iter().position(|r| r.id == region_id) {
        Some(i) => i,
        None => return 0,
    };

    // Return existing mapping if already mapped by this TID
    if let Some(mapping) = regions[idx].mappings.iter().find(|m| m.tid == tid) {
        return mapping.vaddr;
    }

    // Find a free virtual address for this mapping
    let vaddr = find_free_shm_addr(tid, &regions);
    let pages = regions[idx].physical_frames.len();

    // Map each page into the current process's address space
    for i in 0..pages {
        let frame = regions[idx].physical_frames[i];
        let va = VirtAddr::new(vaddr + (i * FRAME_SIZE) as u64);
        virtual_mem::map_page(va, frame, 0x07); // Present + Writable + User
    }

    regions[idx].mappings.push(ShmMapping { tid, vaddr });
    vaddr
}

/// Unmap a shared memory region from the calling process's address space.
///
/// Clears PTEs via recursive mapping on the current CR3. Physical frames are
/// NOT freed here — they remain owned by the [`SharedRegion`] until the last
/// mapping is removed and the owner has released the region.
pub fn unmap_from_current(region_id: u32) -> bool {
    let tid = crate::task::scheduler::current_tid();
    let mut regions = SHARED_REGIONS.lock();

    let idx = match regions.iter().position(|r| r.id == region_id) {
        Some(i) => i,
        None => return false,
    };

    let mapping_pos = match regions[idx].mappings.iter().position(|m| m.tid == tid) {
        Some(p) => p,
        None => return false,
    };

    let vaddr = regions[idx].mappings[mapping_pos].vaddr;
    let pages = regions[idx].physical_frames.len();

    // Clear PTEs in the current address space
    for i in 0..pages {
        virtual_mem::unmap_page(VirtAddr::new(vaddr + (i * FRAME_SIZE) as u64));
    }

    regions[idx].mappings.remove(mapping_pos);

    // Free region if no mappings and no owner
    maybe_free_region(&mut regions, idx);

    true
}

/// Destroy a shared memory region (owner only).
///
/// Releases the owner's claim on the region. Physical frames are freed
/// immediately if no other process has the region mapped; otherwise they
/// are freed when the last mapping is released.
pub fn destroy(region_id: u32, caller_tid: u32) -> bool {
    let mut regions = SHARED_REGIONS.lock();

    let idx = match regions.iter().position(|r| r.id == region_id) {
        Some(i) => i,
        None => return false,
    };

    // Only the owner (or 0 = already orphaned) can destroy
    if regions[idx].owner_tid != caller_tid {
        return false;
    }

    // Release ownership
    regions[idx].owner_tid = 0;

    // Free if no mappings remain
    maybe_free_region(&mut regions, idx);

    true
}

/// Return the size of a shared memory region in bytes, or 0 if not found.
pub fn region_size(region_id: u32) -> usize {
    let regions = SHARED_REGIONS.lock();
    regions
        .iter()
        .find(|r| r.id == region_id)
        .map(|r| r.size)
        .unwrap_or(0)
}

/// Clean up all shared memory for a process that is exiting.
///
/// **Must be called while the process's page directory is still active**
/// (before switching to kernel CR3 in `sys_exit`). This ensures `unmap_page`
/// operates on the correct page tables via recursive mapping.
///
/// Unmaps all SHM pages from the exiting process's address space, removes
/// its mappings, releases ownership of any regions it created, and frees
/// regions that have no remaining mappings or owner.
pub fn cleanup_process(tid: u32) {
    let mut regions = SHARED_REGIONS.lock();

    let mut i = 0;
    while i < regions.len() {
        // Unmap if this process has the region mapped
        if let Some(pos) = regions[i].mappings.iter().position(|m| m.tid == tid) {
            let vaddr = regions[i].mappings[pos].vaddr;
            let pages = regions[i].physical_frames.len();
            for p in 0..pages {
                virtual_mem::unmap_page(VirtAddr::new(vaddr + (p * FRAME_SIZE) as u64));
            }
            regions[i].mappings.remove(pos);
        }

        // Release ownership if this TID owns the region
        if regions[i].owner_tid == tid {
            regions[i].owner_tid = 0;
        }

        // Free region if no mappings and no owner
        if regions[i].mappings.is_empty() && regions[i].owner_tid == 0 {
            let region = regions.remove(i);
            for frame in &region.physical_frames {
                physical::free_frame(*frame);
            }
            // Don't increment — next element shifted into this position
        } else {
            i += 1;
        }
    }
}

/// Free a region if it has no mappings and no owner.
fn maybe_free_region(regions: &mut Vec<SharedRegion>, idx: usize) {
    if regions[idx].mappings.is_empty() && regions[idx].owner_tid == 0 {
        let region = regions.remove(idx);
        for frame in &region.physical_frames {
            physical::free_frame(*frame);
        }
    }
}

/// Lock-free check if the shared memory lock is currently held.
pub fn is_shm_locked() -> bool {
    SHARED_REGIONS.is_locked()
}

/// Find a free virtual address for a new SHM mapping for the given TID.
///
/// Scans all existing mappings for this TID and returns the next page-aligned
/// address above the highest existing mapping.
fn find_free_shm_addr(tid: u32, regions: &[SharedRegion]) -> u64 {
    let mut next = SHM_BASE;
    for region in regions {
        for mapping in &region.mappings {
            if mapping.tid == tid {
                let end = mapping.vaddr + region.size as u64;
                if end > next {
                    next = end;
                }
            }
        }
    }
    // Ensure page alignment (should already be, but safety)
    (next + 0xFFF) & !0xFFF
}
