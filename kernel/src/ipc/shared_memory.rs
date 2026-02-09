//! Shared memory regions for zero-copy IPC between processes.
//!
//! A shared region is a set of physical frames that can be mapped into multiple
//! process address spaces. Reference counting ensures frames are freed only when
//! all users have released the region.

use crate::memory::address::{PhysAddr, VirtAddr};
use crate::memory::physical;
use crate::memory::FRAME_SIZE;
use crate::sync::spinlock::Spinlock;
use alloc::vec::Vec;

static SHARED_REGIONS: Spinlock<Vec<SharedRegion>> = Spinlock::new(Vec::new());

/// A reference-counted shared memory region backed by physical frames.
pub struct SharedRegion {
    /// Unique region identifier.
    pub id: u32,
    /// Backing physical frames in page order.
    pub physical_frames: Vec<PhysAddr>,
    /// Total size in bytes (page-aligned).
    pub size: usize,
    /// Number of active mappings; frames are freed when this reaches zero.
    pub ref_count: u32,
    /// PID of the process that created the region.
    pub owner_pid: u32,
}

static mut NEXT_REGION_ID: u32 = 1;

/// Create a new shared memory region of the given size (rounded up to page size).
pub fn create(size: usize, owner_pid: u32) -> Option<u32> {
    let pages = (size + FRAME_SIZE - 1) / FRAME_SIZE;
    let mut frames = Vec::new();

    for _ in 0..pages {
        match physical::alloc_frame() {
            Some(frame) => frames.push(frame),
            None => {
                // Release already allocated frames
                for f in &frames {
                    physical::free_frame(*f);
                }
                return None;
            }
        }
    }

    let id = unsafe {
        let i = NEXT_REGION_ID;
        NEXT_REGION_ID += 1;
        i
    };

    let region = SharedRegion {
        id,
        physical_frames: frames,
        size: pages * FRAME_SIZE,
        ref_count: 1,
        owner_pid,
    };

    let mut regions = SHARED_REGIONS.lock();
    regions.push(region);

    Some(id)
}

/// Map a shared region into a process address space at the given virtual address.
pub fn map_region(region_id: u32, base_virt: VirtAddr) -> bool {
    let regions = SHARED_REGIONS.lock();
    let region = match regions.iter().find(|r| r.id == region_id) {
        Some(r) => r,
        None => return false,
    };

    for (i, frame) in region.physical_frames.iter().enumerate() {
        let virt = VirtAddr::new(base_virt.as_u64() + (i * FRAME_SIZE) as u64);
        crate::memory::virtual_mem::map_page(virt, *frame, 0x07); // Present + Writable + User
    }

    true
}

/// Release a reference to a shared region.
pub fn release(region_id: u32) {
    let mut regions = SHARED_REGIONS.lock();
    if let Some(pos) = regions.iter().position(|r| r.id == region_id) {
        regions[pos].ref_count -= 1;
        if regions[pos].ref_count == 0 {
            let region = regions.remove(pos);
            for frame in &region.physical_frames {
                physical::free_frame(*frame);
            }
        }
    }
}
