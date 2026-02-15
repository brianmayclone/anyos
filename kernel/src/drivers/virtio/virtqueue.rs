//! Split virtqueue implementation for VirtIO devices.
//!
//! Implements the split ring layout: descriptor table, available ring, used ring.
//! Supports synchronous polled I/O with descriptor chaining for request/response pairs.

use crate::memory::physical;

// ──────────────────────────────────────────────
// Descriptor Flags
// ──────────────────────────────────────────────

/// Buffer continues via the `next` field.
const VIRTQ_DESC_F_NEXT: u16 = 1;
/// Buffer is device-writable (response/read-back).
const VIRTQ_DESC_F_WRITE: u16 = 2;

// ──────────────────────────────────────────────
// Descriptor Entry (16 bytes)
// ──────────────────────────────────────────────

#[repr(C)]
#[derive(Clone, Copy)]
struct VirtqDesc {
    addr: u64,      // Physical address of buffer
    len: u32,       // Buffer length in bytes
    flags: u16,     // NEXT, WRITE, INDIRECT
    next: u16,      // Next descriptor index (if NEXT flag set)
}

// ──────────────────────────────────────────────
// VirtQueue
// ──────────────────────────────────────────────

/// A split virtqueue with physically contiguous descriptor table, available ring, and used ring.
pub struct VirtQueue {
    queue_size: u16,
    /// Physical address of the descriptor table.
    desc_phys: u64,
    /// Physical address of the available ring.
    avail_phys: u64,
    /// Physical address of the used ring.
    used_phys: u64,
    /// Virtual pointer to descriptor table (identity-mapped).
    desc: *mut VirtqDesc,
    /// Virtual pointer to available ring: [flags:u16, idx:u16, ring[queue_size]:u16, used_event:u16].
    avail: *mut u16,
    /// Virtual pointer to used ring: [flags:u16, idx:u16, ring[queue_size]:{id:u32,len:u32}, avail_event:u16].
    used: *mut u8,
    /// Head of the free descriptor chain.
    free_head: u16,
    /// Last consumed index in the used ring.
    last_used_idx: u16,
    /// Number of free descriptors available.
    num_free: u16,
}

// VirtQueue contains raw pointers but is only accessed under a Spinlock
unsafe impl Send for VirtQueue {}

impl VirtQueue {
    /// Allocate and initialize a new virtqueue.
    ///
    /// Allocates physically contiguous memory for the descriptor table, available ring,
    /// and used ring. All structures are placed in a single allocation when possible.
    pub fn new(queue_size: u16) -> Option<Self> {
        let qs = queue_size as usize;

        // Calculate sizes and layout
        let desc_size = qs * 16;                      // 16 bytes per descriptor
        let avail_size = 6 + qs * 2;                  // flags(2) + idx(2) + ring(2*qs) + used_event(2)
        let used_size = 6 + qs * 8;                   // flags(2) + idx(2) + ring(8*qs) + avail_event(2)

        // Align: desc at 16, avail at 2, used at 4
        let avail_offset = (desc_size + 15) & !15;    // Align to 16 (overshoot is fine)
        let used_offset_raw = avail_offset + avail_size;
        let used_offset = (used_offset_raw + 3) & !3; // Align to 4

        let total_size = used_offset + used_size;
        let num_pages = (total_size + 4095) / 4096;

        // Allocate contiguous physical pages
        let phys = physical::alloc_contiguous(num_pages)?;
        let base = phys.as_u64();

        // Zero the entire allocation (identity-mapped, so virt == phys for low memory)
        unsafe {
            core::ptr::write_bytes(base as *mut u8, 0, num_pages * 4096);
        }

        let desc_phys = base;
        let avail_phys = base + avail_offset as u64;
        let used_phys = base + used_offset as u64;

        let desc = desc_phys as *mut VirtqDesc;
        let avail = avail_phys as *mut u16;
        let used = used_phys as *mut u8;

        // Initialize free descriptor chain: each descriptor's `next` points to the next one
        unsafe {
            for i in 0..qs {
                let d = &mut *desc.add(i);
                d.addr = 0;
                d.len = 0;
                d.flags = if i + 1 < qs { VIRTQ_DESC_F_NEXT } else { 0 };
                d.next = if i + 1 < qs { (i + 1) as u16 } else { 0 };
            }
        }

        // Suppress device-to-driver notifications (interrupts).
        // We use polled I/O (execute_sync busy-waits on used ring), so interrupts
        // are unnecessary. Without this, every completed command asserts the PCI
        // INTx line. Since no ISR handler deasserts it, level-triggered delivery
        // causes an interrupt storm that starves the main thread.
        // VIRTQ_AVAIL_F_NO_INTERRUPT = 1
        unsafe {
            core::ptr::write_volatile(avail, 1u16);
        }

        Some(VirtQueue {
            queue_size,
            desc_phys,
            avail_phys,
            used_phys,
            desc,
            avail,
            used,
            free_head: 0,
            last_used_idx: 0,
            num_free: queue_size,
        })
    }

    /// Physical address of the descriptor table (for device config).
    pub fn desc_phys(&self) -> u64 { self.desc_phys }
    /// Physical address of the available ring (for device config).
    pub fn avail_phys(&self) -> u64 { self.avail_phys }
    /// Physical address of the used ring (for device config).
    pub fn used_phys(&self) -> u64 { self.used_phys }

    // ── Internal: available ring access ──

    /// Read available ring flags.
    fn avail_flags(&self) -> u16 {
        unsafe { core::ptr::read_volatile(self.avail) }
    }

    /// Read available ring index.
    fn avail_idx(&self) -> u16 {
        unsafe { core::ptr::read_volatile(self.avail.add(1)) }
    }

    /// Write available ring index.
    fn set_avail_idx(&self, val: u16) {
        unsafe { core::ptr::write_volatile(self.avail.add(1), val); }
    }

    /// Write to available ring entry at position `pos`.
    fn set_avail_ring(&self, pos: u16, desc_idx: u16) {
        // ring starts at offset 2 (after flags and idx)
        unsafe { core::ptr::write_volatile(self.avail.add(2 + pos as usize), desc_idx); }
    }

    // ── Internal: used ring access ──

    /// Read used ring index.
    fn used_idx(&self) -> u16 {
        let ptr = (self.used as u64 + 2) as *const u16;
        unsafe { core::ptr::read_volatile(ptr) }
    }

    /// Read used ring entry at position `pos`: returns (descriptor id, bytes written).
    fn used_ring_entry(&self, pos: u16) -> (u32, u32) {
        // Used ring entries start at offset 4: each entry is {id:u32, len:u32} = 8 bytes
        let entry_base = self.used as u64 + 4 + (pos as u64) * 8;
        let id = unsafe { core::ptr::read_volatile(entry_base as *const u32) };
        let len = unsafe { core::ptr::read_volatile((entry_base + 4) as *const u32) };
        (id, len)
    }

    // ── Descriptor management ──

    /// Allocate a free descriptor. Returns its index.
    fn alloc_desc(&mut self) -> Option<u16> {
        if self.num_free == 0 {
            return None;
        }
        let idx = self.free_head;
        let desc = unsafe { &*self.desc.add(idx as usize) };
        self.free_head = desc.next;
        self.num_free -= 1;
        Some(idx)
    }

    /// Free a descriptor chain starting at `head`.
    fn free_chain(&mut self, head: u16) {
        let mut idx = head;
        loop {
            let desc = unsafe { &mut *self.desc.add(idx as usize) };
            let has_next = desc.flags & VIRTQ_DESC_F_NEXT != 0;
            let next = desc.next;

            // Push back to free list
            desc.flags = if self.num_free > 0 { VIRTQ_DESC_F_NEXT } else { 0 };
            desc.next = self.free_head;
            self.free_head = idx;
            self.num_free += 1;

            if has_next {
                idx = next;
            } else {
                break;
            }
        }
    }

    // ── Public API ──

    /// Submit a request-response pair to the queue.
    ///
    /// `readable` buffers are device-readable (request data).
    /// `writable` buffers are device-writable (response data).
    /// Each entry is (physical_address, length).
    ///
    /// Returns the head descriptor index, or None if not enough descriptors.
    pub fn push(&mut self, readable: &[(u64, u32)], writable: &[(u64, u32)]) -> Option<u16> {
        let total = readable.len() + writable.len();
        if total == 0 || self.num_free < total as u16 {
            return None;
        }

        let mut head: u16 = 0;
        let mut prev: Option<u16> = None;

        // Chain readable (device-read) descriptors
        for (i, &(addr, len)) in readable.iter().enumerate() {
            let idx = self.alloc_desc()?;
            if i == 0 { head = idx; }

            let desc = unsafe { &mut *self.desc.add(idx as usize) };
            desc.addr = addr;
            desc.len = len;
            desc.flags = VIRTQ_DESC_F_NEXT; // Will fix last one below
            desc.next = 0;

            if let Some(p) = prev {
                let prev_desc = unsafe { &mut *self.desc.add(p as usize) };
                prev_desc.next = idx;
            }
            prev = Some(idx);
        }

        // Chain writable (device-write) descriptors
        for &(addr, len) in writable {
            let idx = self.alloc_desc()?;
            if readable.is_empty() && prev.is_none() { head = idx; }

            let desc = unsafe { &mut *self.desc.add(idx as usize) };
            desc.addr = addr;
            desc.len = len;
            desc.flags = VIRTQ_DESC_F_WRITE | VIRTQ_DESC_F_NEXT;
            desc.next = 0;

            if let Some(p) = prev {
                let prev_desc = unsafe { &mut *self.desc.add(p as usize) };
                prev_desc.next = idx;
            }
            prev = Some(idx);
        }

        // Clear NEXT flag on last descriptor
        if let Some(last) = prev {
            let desc = unsafe { &mut *self.desc.add(last as usize) };
            desc.flags &= !VIRTQ_DESC_F_NEXT;
        }

        // Memory fence to ensure descriptor writes are visible before avail ring update
        core::sync::atomic::fence(core::sync::atomic::Ordering::Release);

        // Add to available ring
        let avail_idx = self.avail_idx();
        self.set_avail_ring(avail_idx % self.queue_size, head);

        // Memory fence before incrementing avail_idx
        core::sync::atomic::fence(core::sync::atomic::Ordering::Release);

        self.set_avail_idx(avail_idx.wrapping_add(1));

        Some(head)
    }

    /// Check the used ring for completed requests.
    /// Returns (head_descriptor_index, bytes_written) if a request completed.
    pub fn poll_used(&mut self) -> Option<(u16, u32)> {
        // Memory fence to see device's used ring writes
        core::sync::atomic::fence(core::sync::atomic::Ordering::Acquire);

        let used_idx = self.used_idx();
        if self.last_used_idx == used_idx {
            return None;
        }

        let (id, len) = self.used_ring_entry(self.last_used_idx % self.queue_size);
        self.last_used_idx = self.last_used_idx.wrapping_add(1);

        // Free the descriptor chain
        self.free_chain(id as u16);

        Some((id as u16, len))
    }

    /// Submit a request and busy-wait for the response.
    /// `notify_fn` is called after pushing to wake the device.
    /// Returns the bytes written by the device to writable buffers.
    pub fn execute_sync<F: Fn()>(
        &mut self,
        readable: &[(u64, u32)],
        writable: &[(u64, u32)],
        notify_fn: F,
    ) -> Option<u32> {
        self.push(readable, writable)?;

        // Notify device
        notify_fn();

        // Busy-wait for completion
        let mut timeout = 0u32;
        loop {
            if let Some((_id, len)) = self.poll_used() {
                return Some(len);
            }
            core::hint::spin_loop();
            timeout += 1;
            if timeout > 10_000_000 {
                crate::serial_println!("  VirtQueue: timeout waiting for device response");
                return None;
            }
        }
    }
}
