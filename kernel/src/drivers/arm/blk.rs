//! VirtIO Block Device driver (DeviceID = 2) over MMIO transport.
//!
//! Implements sector-level read/write using a single requestq (VirtQueue 0).
//! Each I/O operation uses a 3-descriptor chain: header → data → status.

use core::ptr;
use core::sync::atomic::{fence, Ordering};

use crate::memory::physical;
use crate::memory::FRAME_SIZE;
use crate::sync::spinlock::Spinlock;

use super::VirtioMmioDevice;
use super::virtqueue::{VirtQueue, VRING_DESC_F_WRITE, DEFAULT_QUEUE_SIZE};

// ---------------------------------------------------------------------------
// VirtIO Block request types
// ---------------------------------------------------------------------------

const VIRTIO_BLK_T_IN: u32 = 0;   // Read
const VIRTIO_BLK_T_OUT: u32 = 1;  // Write

/// Status byte values returned by the device.
const VIRTIO_BLK_S_OK: u8 = 0;

// ---------------------------------------------------------------------------
// Request header (16 bytes, matches VirtIO spec 5.2.6)
// ---------------------------------------------------------------------------

#[repr(C)]
struct VirtioBlkReqHeader {
    type_: u32,
    reserved: u32,
    sector: u64,
}

// ---------------------------------------------------------------------------
// VirtIO Block Device state
// ---------------------------------------------------------------------------

struct VirtioBlk {
    /// MMIO device base address (for notify + interrupt ack).
    base: usize,
    /// The request virtqueue.
    queue: VirtQueue,
    /// Disk capacity in 512-byte sectors.
    capacity: u64,
    /// Block size (typically 512).
    blk_size: u32,
}

static BLK_DEVICE: Spinlock<Option<VirtioBlk>> = Spinlock::new(None);

/// Physical address helper: convert RAM virtual to physical.
#[inline]
fn virt_to_phys(virt: usize) -> u64 {
    // Inverse of phys_to_virt: VA - 0xFFFF_0000_4000_0000 = PA
    (virt as u64).wrapping_sub(0xFFFF_0000_4000_0000)
}

// ---------------------------------------------------------------------------
// Initialization
// ---------------------------------------------------------------------------

/// Initialize the VirtIO block device.
pub fn init(dev: &VirtioMmioDevice) {
    // Feature negotiation — we don't need any special features
    if dev.init_device(0).is_none() {
        crate::serial_println!("  virtio-blk: feature negotiation failed");
        return;
    }

    // Read device config
    let capacity = dev.read_config_u64(0);  // offset 0: capacity (u64)
    let blk_size = if capacity > 0 { dev.read_config_u32(20) } else { 512 }; // offset 20: blk_size
    let blk_size = if blk_size == 0 { 512 } else { blk_size };

    crate::serial_println!("  virtio-blk: capacity={} sectors, blk_size={}", capacity, blk_size);

    // Set up requestq (queue 0)
    let queue = match VirtQueue::new(0, DEFAULT_QUEUE_SIZE) {
        Some(q) => q,
        None => {
            crate::serial_println!("  virtio-blk: failed to allocate virtqueue");
            return;
        }
    };

    let (desc_phys, avail_phys, used_phys) = queue.phys_addrs();
    crate::serial_println!("  virtio-blk: queue phys: desc={:#x} avail={:#x} used={:#x}",
        desc_phys, avail_phys, used_phys);
    if !dev.setup_queue_raw(0, DEFAULT_QUEUE_SIZE, desc_phys, avail_phys, used_phys) {
        crate::serial_println!("  virtio-blk: failed to setup queue");
        return;
    }

    crate::serial_println!("  virtio-blk: status after setup={:#x}", dev.get_status());

    // Mark device ready
    dev.driver_ok();
    crate::serial_println!("  virtio-blk: status after driver_ok={:#x}", dev.get_status());

    let blk = VirtioBlk {
        base: dev.base(),
        queue,
        capacity,
        blk_size,
    };

    *BLK_DEVICE.lock() = Some(blk);
}

// ---------------------------------------------------------------------------
// I/O operations
// ---------------------------------------------------------------------------

/// Read sectors from the VirtIO block device.
///
/// `sector`: starting sector (512-byte units).
/// `count`: number of sectors to read.
/// `buf`: output buffer, must be at least `count * 512` bytes.
pub fn read_sectors(sector: u64, count: u32, buf: &mut [u8]) -> bool {
    do_io(VIRTIO_BLK_T_IN, sector, count, buf)
}

/// Write sectors to the VirtIO block device.
pub fn write_sectors(sector: u64, count: u32, buf: &[u8]) -> bool {
    // Safe: write_sectors only reads from buf, the cast is for the shared do_io path
    let buf_mut = unsafe { core::slice::from_raw_parts_mut(buf.as_ptr() as *mut u8, buf.len()) };
    do_io(VIRTIO_BLK_T_OUT, sector, count, buf_mut)
}

/// Perform a block I/O request (read or write).
fn do_io(req_type: u32, sector: u64, count: u32, buf: &mut [u8]) -> bool {
    let byte_len = count as usize * 512;
    if buf.len() < byte_len {
        return false;
    }

    let mut guard = BLK_DEVICE.lock();
    let blk = match guard.as_mut() {
        Some(b) => b,
        None => return false,
    };

    if sector + count as u64 > blk.capacity {
        return false;
    }

    // Allocate a temporary physical page for the request header + status.
    // Layout: header (16 bytes) at offset 0, status byte (1 byte) at offset 16.
    let hdr_frame = match physical::alloc_frame() {
        Some(f) => f,
        None => return false,
    };
    let hdr_phys = hdr_frame.0;
    let hdr_virt = (hdr_phys + 0xFFFF_0000_4000_0000) as usize;

    // Write request header
    let header = VirtioBlkReqHeader {
        type_: req_type,
        reserved: 0,
        sector,
    };
    unsafe {
        ptr::write(hdr_virt as *mut VirtioBlkReqHeader, header);
        // Zero status byte
        ptr::write_bytes((hdr_virt + 16) as *mut u8, 0xFF, 1);
    }

    let status_phys = hdr_phys + 16;

    // For the data buffer, we need a physical address.
    // The buf is in kernel virtual space (stack or heap), convert to physical.
    let buf_phys = virt_to_phys(buf.as_ptr() as usize);

    // Build 3-descriptor chain: header(R) → data(R or W) → status(W)
    let data_flags = if req_type == VIRTIO_BLK_T_IN {
        VRING_DESC_F_WRITE // Device writes to buffer (read op)
    } else {
        0 // Device reads from buffer (write op)
    };

    let chain = [
        (hdr_phys, 16u32, 0u16),                         // header (device-readable)
        (buf_phys, byte_len as u32, data_flags),          // data
        (status_phys, 1u32, VRING_DESC_F_WRITE),          // status (device-writable)
    ];

    let _head = match blk.queue.push_chain(&chain) {
        Some(h) => h,
        None => {
            physical::free_frame(hdr_frame);
            return false;
        }
    };

    // Notify device (DSB + MMIO write)
    let dev_base = blk.base;
    unsafe {
        core::arch::asm!("dsb sy", options(nostack, preserves_flags));
        ptr::write_volatile((dev_base + 0x050) as *mut u32, 0);
    }

    // Poll for completion (busy-wait; interrupts handled separately)
    let mut timeout = 1_000_000u32;
    while !blk.queue.has_used() {
        core::hint::spin_loop();
        timeout -= 1;
        if timeout == 0 {
            crate::serial_println!("  virtio-blk: I/O timeout");
            physical::free_frame(hdr_frame);
            return false;
        }
    }

    // Pop the used descriptor
    blk.queue.pop_used();

    // Check status byte
    let status = unsafe { ptr::read_volatile((hdr_virt + 16) as *const u8) };

    // Free header page
    physical::free_frame(hdr_frame);

    status == VIRTIO_BLK_S_OK
}

/// Get the disk capacity in sectors, or 0 if no device.
pub fn capacity() -> u64 {
    BLK_DEVICE.lock().as_ref().map_or(0, |b| b.capacity)
}

/// Check if a VirtIO block device is available.
pub fn is_available() -> bool {
    BLK_DEVICE.lock().is_some()
}
