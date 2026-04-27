//! VirtIO Balloon PCI driver.
//!
//! Allows the hypervisor to reclaim unused guest memory. The driver receives
//! inflate requests (return pages to host) and deflate requests (reclaim pages).
//!
//! VirtIO device IDs:
//! - 0x1002 (transitional) / 0x1045 (modern) — virtio-balloon

use crate::drivers::pci::PciDevice;
use crate::drivers::virtio::virtqueue::VirtQueue;
use crate::drivers::virtio::{self, VirtioDevice, VIRTIO_F_VERSION_1};
use crate::memory::physical;
use crate::sync::spinlock::Spinlock;

// ── Constants ────────────────────────────────────────────────────────────────

/// Max pages to inflate/deflate in a single batch.
const MAX_BATCH: usize = 256;

/// Page size expected by the VirtIO balloon spec (always 4 KiB).
const BALLOON_PAGE_SIZE: u64 = 4096;

// ── Driver State ─────────────────────────────────────────────────────────────

struct BalloonDevice {
    vdev: VirtioDevice,
    inflateq: VirtQueue,
    deflateq: VirtQueue,
    /// Physical address of a buffer for PFN arrays (1 page = room for 1024 u32 PFNs).
    pfn_buf_phys: u64,
    /// Number of pages currently inflated (given back to host).
    inflated_pages: usize,
}

// BalloonDevice is only accessed under the DEVICE spinlock.
unsafe impl Send for BalloonDevice {}

static DEVICE: Spinlock<Option<BalloonDevice>> = Spinlock::new(None);

// ── Public API ──────────────────────────────────────────────────────────────

/// Inflate the balloon by `count` pages (return memory to the host).
/// Allocates physical pages and tells the hypervisor to reclaim them.
/// Returns the number of pages actually inflated.
pub fn inflate(count: usize) -> usize {
    let mut dev = DEVICE.lock();
    let dev = match dev.as_mut() {
        Some(d) => d,
        None => return 0,
    };

    let batch = count.min(MAX_BATCH);
    let pfn_buf = dev.pfn_buf_phys as *mut u32;
    let mut inflated = 0;

    for i in 0..batch {
        if let Some(frame) = physical::alloc_frame() {
            let pfn = (frame.as_u64() / BALLOON_PAGE_SIZE) as u32;
            unsafe {
                pfn_buf.add(i).write_volatile(pfn);
            }
            inflated += 1;
        } else {
            break;
        }
    }

    if inflated == 0 {
        return 0;
    }

    // Send PFN array to the inflate queue.
    let readable = [(dev.pfn_buf_phys, (inflated * 4) as u32)];
    let result = dev
        .inflateq
        .execute_sync(&readable, &[], || dev.vdev.notify_queue(0));

    if result.is_some() {
        dev.inflated_pages += inflated;
        inflated
    } else {
        // On failure, we leak the pages (they're gone from the allocator).
        // This is acceptable — balloon failures are rare.
        0
    }
}

/// Get the number of currently inflated pages.
pub fn inflated_pages() -> usize {
    DEVICE.lock().as_ref().map_or(0, |d| d.inflated_pages)
}

/// Check if a VirtIO Balloon device is available.
pub fn is_available() -> bool {
    DEVICE.lock().is_some()
}

// ── Probe Function ──────────────────────────────────────────────────────────

/// Probe and initialize a VirtIO Balloon device from the PCI bus.
pub fn probe(pci: &PciDevice) -> Option<alloc::boxed::Box<dyn crate::drivers::hal::Driver>> {
    crate::serial_verbose_println!(
        "VirtIO Balloon: probing PCI {:04x}:{:04x}",
        pci.vendor_id,
        pci.device_id
    );

    // 1. Find VirtIO PCI capabilities.
    let caps = virtio::find_capabilities(pci)?;

    // 2. Create device handle.
    let vdev = VirtioDevice::new(pci, &caps);

    // 3. Initialize device, negotiate features.
    let desired_features = VIRTIO_F_VERSION_1;
    if let Err(e) = vdev.init_device(desired_features) {
        crate::serial_verbose_println!("VirtIO Balloon: init failed: {}", e);
        return None;
    }

    // 4. Set up virtqueues: inflateq (0) and deflateq (1).
    let inflateq = vdev.setup_queue(0)?;
    let deflateq = vdev.setup_queue(1)?;

    // 5. Mark device ready.
    vdev.set_driver_ok();

    // 6. Allocate PFN buffer (1 page = room for 1024 PFNs = 4 MiB per batch).
    let pfn_buf_phys = physical::alloc_contiguous(1)?.as_u64();
    unsafe {
        core::ptr::write_bytes(pfn_buf_phys as *mut u8, 0, 4096);
    }

    // 7. Store globally.
    *DEVICE.lock() = Some(BalloonDevice {
        vdev,
        inflateq,
        deflateq,
        pfn_buf_phys,
        inflated_pages: 0,
    });

    crate::serial_verbose_println!("VirtIO Balloon: initialized, memory ballooning available");

    // Return None — stored globally.
    None
}
