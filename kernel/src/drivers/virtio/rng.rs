//! VirtIO RNG (Random Number Generator) PCI driver.
//!
//! Provides hardware entropy from the hypervisor via a single virtqueue.
//! Used by `sys_random()` as a high-quality entropy source before falling
//! back to RDRAND or TSC-based PRNG.
//!
//! VirtIO device IDs:
//! - 0x1005 (transitional) / 0x1044 (modern) — virtio-rng

use crate::drivers::pci::PciDevice;
use crate::drivers::virtio::virtqueue::VirtQueue;
use crate::drivers::virtio::{self, VirtioDevice, VIRTIO_F_VERSION_1};
use crate::memory::physical;
use crate::sync::spinlock::Spinlock;

// ── Constants ────────────────────────────────────────────────────────────────

/// Buffer size for RNG requests (1 page = 4096 bytes).
const BUF_SIZE: usize = 4096;

// ── Global RNG Device ────────────────────────────────────────────────────────

struct RngDevice {
    vdev: VirtioDevice,
    requestq: VirtQueue,
    buf_phys: u64,
}

// RngDevice is only accessed under the DEVICE spinlock.
unsafe impl Send for RngDevice {}

static DEVICE: Spinlock<Option<RngDevice>> = Spinlock::new(None);

// ── Public API ──────────────────────────────────────────────────────────────

/// Fill `buf` with random bytes from the VirtIO RNG device.
/// Returns the number of bytes filled, or 0 if no device is available.
pub fn fill_random(buf: &mut [u8]) -> usize {
    let mut dev = DEVICE.lock();
    let dev = match dev.as_mut() {
        Some(d) => d,
        None => return 0,
    };

    let request_len = buf.len().min(BUF_SIZE);
    if request_len == 0 {
        return 0;
    }

    // Post a writable buffer for the device to fill with random bytes.
    let writable = [(dev.buf_phys, request_len as u32)];
    let result = dev
        .requestq
        .execute_sync(&[], &writable, || dev.vdev.notify_queue(0));

    match result {
        Some(bytes_written) => {
            let copy_len = (bytes_written as usize).min(request_len).min(buf.len());
            unsafe {
                core::ptr::copy_nonoverlapping(
                    dev.buf_phys as *const u8,
                    buf.as_mut_ptr(),
                    copy_len,
                );
            }
            copy_len
        }
        None => 0,
    }
}

/// Check if a VirtIO RNG device is available.
pub fn is_available() -> bool {
    DEVICE.lock().is_some()
}

// ── Probe Function ──────────────────────────────────────────────────────────

/// Probe and initialize a VirtIO RNG device from the PCI bus.
pub fn probe(pci: &PciDevice) -> Option<alloc::boxed::Box<dyn crate::drivers::hal::Driver>> {
    crate::serial_verbose_println!(
        "VirtIO RNG: probing PCI {:04x}:{:04x}",
        pci.vendor_id,
        pci.device_id
    );

    // 1. Find VirtIO PCI capabilities.
    let caps = virtio::find_capabilities(pci)?;

    // 2. Create device handle (maps BARs).
    let vdev = VirtioDevice::new(pci, &caps);

    // 3. Initialize device, negotiate features.
    let desired_features = VIRTIO_F_VERSION_1;
    if let Err(e) = vdev.init_device(desired_features) {
        crate::serial_verbose_println!("VirtIO RNG: init failed: {}", e);
        return None;
    }

    // 4. Set up virtqueue: requestq (0) — the only queue for RNG.
    let requestq = vdev.setup_queue(0)?;

    // 5. Mark device ready.
    vdev.set_driver_ok();

    // 6. Allocate DMA buffer.
    let buf_phys = physical::alloc_contiguous(1)?.as_u64();
    unsafe {
        core::ptr::write_bytes(buf_phys as *mut u8, 0, BUF_SIZE);
    }

    // 7. Store globally.
    *DEVICE.lock() = Some(RngDevice {
        vdev,
        requestq,
        buf_phys,
    });

    crate::serial_verbose_println!("VirtIO RNG: initialized, hardware entropy available");

    // Return None — we store the device globally, not in HAL.
    None
}
