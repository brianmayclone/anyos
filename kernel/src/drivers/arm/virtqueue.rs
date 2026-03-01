//! VirtIO split virtqueue implementation for MMIO transport.
//!
//! Allocates descriptor table, available ring, and used ring from physical memory.
//! Works with any VirtIO MMIO device.

use core::ptr;
use core::sync::atomic::{fence, Ordering};

use crate::memory::physical;
use crate::memory::FRAME_SIZE;

/// Default queue size (must be power of 2).
pub const DEFAULT_QUEUE_SIZE: u16 = 128;

// ---------------------------------------------------------------------------
// Descriptor flags
// ---------------------------------------------------------------------------

/// Buffer is continued via the `next` field.
pub const VRING_DESC_F_NEXT: u16 = 1;
/// Buffer is device-writable (otherwise device-readable).
pub const VRING_DESC_F_WRITE: u16 = 2;

// ---------------------------------------------------------------------------
// Descriptor table entry (16 bytes)
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct VringDesc {
    pub addr: u64,
    pub len: u32,
    pub flags: u16,
    pub next: u16,
}

// ---------------------------------------------------------------------------
// Available ring
// ---------------------------------------------------------------------------

#[repr(C)]
pub struct VringAvail {
    pub flags: u16,
    pub idx: u16,
    // Followed by ring[num] entries (u16 each)
}

// ---------------------------------------------------------------------------
// Used ring element
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct VringUsedElem {
    pub id: u32,
    pub len: u32,
}

#[repr(C)]
pub struct VringUsed {
    pub flags: u16,
    pub idx: u16,
    // Followed by ring[num] VringUsedElem entries
}

// ---------------------------------------------------------------------------
// VirtQueue
// ---------------------------------------------------------------------------

/// A split virtqueue with descriptor table, available ring, and used ring.
pub struct VirtQueue {
    /// Number of descriptors (power of 2).
    pub num: u16,
    /// Index of the queue (0, 1, ...).
    pub queue_idx: u16,

    // Virtual addresses of the three ring components
    desc_virt: usize,
    avail_virt: usize,
    used_virt: usize,

    // Physical addresses (for programming the device)
    desc_phys: u64,
    avail_phys: u64,
    used_phys: u64,

    /// Next free descriptor index.
    free_head: u16,
    /// Number of free descriptors.
    num_free: u16,
    /// Last seen used index.
    last_used_idx: u16,
}

/// Convert a physical address in RAM to a kernel virtual address.
#[inline]
fn phys_to_virt(phys: u64) -> usize {
    // RAM: PA 0x4000_0000 → VA 0xFFFF_0000_8000_0000 (PHYS_TO_VIRT_OFFSET = 0xFFFF_0000_4000_0000)
    (phys + 0xFFFF_0000_4000_0000) as usize
}

impl VirtQueue {
    /// Allocate and initialize a new split virtqueue.
    ///
    /// Uses the VirtIO legacy (Version 1) contiguous memory layout:
    ///   - Descriptor table at offset 0 (num × 16 bytes)
    ///   - Available ring immediately after (6 + 2×num bytes)
    ///   - Used ring at the next page boundary (6 + 8×num bytes)
    ///
    /// This layout is also valid for Version 2 (modern) when separate addresses
    /// are programmed via setup_queue_raw().
    pub fn new(queue_idx: u16, num: u16) -> Option<Self> {
        let n = num as usize;

        // VirtIO legacy layout (Spec 2.6.2):
        //   desc:  offset 0, size = num * 16
        //   avail: offset = num * 16, size = 6 + 2 * num
        //   used:  offset = align_up(num * 16 + 6 + 2 * num, PAGE_SIZE)
        let desc_size = n * 16;
        let avail_offset = desc_size;
        let avail_end = avail_offset + 6 + 2 * n;
        let used_offset = (avail_end + FRAME_SIZE - 1) & !(FRAME_SIZE - 1); // page-aligned
        let used_size = 6 + 8 * n;
        let total_size = used_offset + used_size;
        let total_pages = (total_size + FRAME_SIZE - 1) / FRAME_SIZE;

        // Allocate contiguous physical frames
        let first_frame = physical::alloc_contiguous(total_pages)?;
        let base_phys = first_frame.0;

        let desc_phys = base_phys;
        let avail_phys = base_phys + avail_offset as u64;
        let used_phys = base_phys + used_offset as u64;

        let desc_virt = phys_to_virt(desc_phys);
        let avail_virt = phys_to_virt(avail_phys);
        let used_virt = phys_to_virt(used_phys);

        // Zero all memory
        unsafe {
            ptr::write_bytes(desc_virt as *mut u8, 0, total_pages * FRAME_SIZE);
        }

        // Initialize free descriptor chain
        let descs = desc_virt as *mut VringDesc;
        for i in 0..n {
            unsafe {
                (*descs.add(i)).next = if i + 1 < n { (i + 1) as u16 } else { 0 };
            }
        }

        Some(VirtQueue {
            num,
            queue_idx,
            desc_virt,
            avail_virt,
            used_virt,
            desc_phys,
            avail_phys,
            used_phys,
            free_head: 0,
            num_free: num,
            last_used_idx: 0,
        })
    }

    /// Get the physical addresses for programming the MMIO device.
    pub fn phys_addrs(&self) -> (u64, u64, u64) {
        (self.desc_phys, self.avail_phys, self.used_phys)
    }

    /// Allocate a descriptor from the free list.
    fn alloc_desc(&mut self) -> Option<u16> {
        if self.num_free == 0 {
            return None;
        }
        let idx = self.free_head;
        let desc = self.desc(idx);
        self.free_head = desc.next;
        self.num_free -= 1;
        Some(idx)
    }

    /// Return a descriptor to the free list.
    fn free_desc(&mut self, idx: u16) {
        let head = self.free_head;
        let desc = self.desc_mut(idx);
        desc.flags = 0;
        desc.next = head;
        self.free_head = idx;
        self.num_free += 1;
    }

    /// Get a reference to a descriptor.
    fn desc(&self, idx: u16) -> &VringDesc {
        unsafe { &*(self.desc_virt as *const VringDesc).add(idx as usize) }
    }

    /// Get a mutable reference to a descriptor.
    fn desc_mut(&mut self, idx: u16) -> &mut VringDesc {
        unsafe { &mut *(self.desc_virt as *mut VringDesc).add(idx as usize) }
    }

    /// Get the available ring's `idx` field.
    fn avail_idx(&self) -> u16 {
        unsafe { ptr::read_volatile(&(*(self.avail_virt as *const VringAvail)).idx) }
    }

    /// Write the available ring's `idx` field.
    fn set_avail_idx(&self, val: u16) {
        unsafe { ptr::write_volatile(&mut (*(self.avail_virt as *mut VringAvail)).idx, val); }
    }

    /// Write an entry into the available ring.
    fn set_avail_ring(&self, ring_idx: u16, desc_idx: u16) {
        let ring_base = self.avail_virt + 4; // skip flags(2) + idx(2)
        let slot = (ring_idx % self.num) as usize;
        unsafe {
            ptr::write_volatile((ring_base + slot * 2) as *mut u16, desc_idx);
        }
    }

    /// Get the used ring's `idx` field.
    fn used_idx(&self) -> u16 {
        unsafe { ptr::read_volatile(&(*(self.used_virt as *const VringUsed)).idx) }
    }

    /// Get a used ring element.
    fn used_elem(&self, ring_idx: u16) -> VringUsedElem {
        let ring_base = self.used_virt + 4; // skip flags(2) + idx(2)
        let slot = (ring_idx % self.num) as usize;
        unsafe { ptr::read_volatile((ring_base + slot * 8) as *const VringUsedElem) }
    }

    /// Push a single buffer (one descriptor) and make it available.
    ///
    /// Returns the descriptor index, or None if no free descriptors.
    pub fn push_buf(&mut self, phys_addr: u64, len: u32, flags: u16) -> Option<u16> {
        let idx = self.alloc_desc()?;
        let d = self.desc_mut(idx);
        d.addr = phys_addr;
        d.len = len;
        d.flags = flags;
        d.next = 0;

        let avail_idx = self.avail_idx();
        self.set_avail_ring(avail_idx, idx);
        fence(Ordering::Release);
        self.set_avail_idx(avail_idx.wrapping_add(1));
        fence(Ordering::Release);

        Some(idx)
    }

    /// Push a chain of 2 or 3 descriptors (e.g., header + data + status for block I/O).
    ///
    /// `bufs` is a slice of (phys_addr, len, flags). The chain is linked via VRING_DESC_F_NEXT.
    /// Returns the head descriptor index, or None if not enough free descriptors.
    pub fn push_chain(&mut self, bufs: &[(u64, u32, u16)]) -> Option<u16> {
        if bufs.is_empty() || self.num_free < bufs.len() as u16 {
            return None;
        }

        let mut head = 0u16;
        let mut prev_idx: Option<u16> = None;

        for (i, &(phys, len, flags)) in bufs.iter().enumerate() {
            let idx = self.alloc_desc()?;
            if i == 0 { head = idx; }

            let d = self.desc_mut(idx);
            d.addr = phys;
            d.len = len;
            d.flags = flags;
            if i + 1 < bufs.len() {
                d.flags |= VRING_DESC_F_NEXT;
            }
            d.next = 0;

            if let Some(prev) = prev_idx {
                self.desc_mut(prev).next = idx;
            }
            prev_idx = Some(idx);
        }

        let avail_idx = self.avail_idx();
        self.set_avail_ring(avail_idx, head);
        fence(Ordering::Release);
        self.set_avail_idx(avail_idx.wrapping_add(1));
        fence(Ordering::Release);

        Some(head)
    }

    /// Pop a completed buffer from the used ring.
    ///
    /// Returns `(descriptor_head_index, bytes_written)` or None if nothing new.
    pub fn pop_used(&mut self) -> Option<(u16, u32)> {
        fence(Ordering::Acquire);
        let used_idx = self.used_idx();
        if self.last_used_idx == used_idx {
            return None;
        }

        let elem = self.used_elem(self.last_used_idx);
        self.last_used_idx = self.last_used_idx.wrapping_add(1);

        // Free the descriptor chain
        let mut idx = elem.id as u16;
        loop {
            let d = self.desc(idx);
            let next = d.next;
            let has_next = d.flags & VRING_DESC_F_NEXT != 0;
            self.free_desc(idx);
            if has_next {
                idx = next;
            } else {
                break;
            }
        }

        Some((elem.id as u16, elem.len))
    }

    /// Check if there are completed buffers in the used ring.
    pub fn has_used(&self) -> bool {
        fence(Ordering::Acquire);
        self.last_used_idx != self.used_idx()
    }
}
