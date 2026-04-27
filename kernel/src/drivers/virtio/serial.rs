// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! VirtIO Serial (Console) PCI driver.
//!
//! Provides a character device (`/dev/vport0`) for data exchange with the host
//! via VirtIO serial ports. Used by the SPICE vdagent daemon for clipboard
//! synchronization between guest and host.
//!
//! VirtIO device IDs:
//! - 0x1003 (transitional) / 0x1043 (modern) — virtio-serial / virtio-console

use alloc::boxed::Box;
use alloc::string::String;
use core::cell::UnsafeCell;

use crate::drivers::hal::{self, Driver, DriverError, DriverType};
use crate::drivers::pci::PciDevice;
use crate::drivers::virtio::virtqueue::VirtQueue;
use crate::drivers::virtio::{self, VirtioDevice, VIRTIO_F_VERSION_1};
use crate::memory::physical;
use crate::sync::spinlock::Spinlock;

// ── Constants ────────────────────────────────────────────────────────────────

/// Number of pages for RX and TX buffers (1 page = 4096 bytes each).
const BUF_PAGES: usize = 1;
const BUF_SIZE: usize = BUF_PAGES * 4096;

/// VirtIO console feature: device has multiple ports (not used yet).
#[allow(dead_code)]
const VIRTIO_CONSOLE_F_MULTIPORT: u64 = 1 << 1;

/// Track how many serial ports have been registered (for unique naming).
static PORT_COUNT: Spinlock<usize> = Spinlock::new(0);

// ── Driver State ─────────────────────────────────────────────────────────────

struct Inner {
    vdev: VirtioDevice,
    receiveq: VirtQueue,
    transmitq: VirtQueue,
    rx_buf_phys: u64,
    tx_buf_phys: u64,
    rx_pending: [u8; BUF_SIZE],
    rx_pending_len: usize,
    rx_pending_pos: usize,
    rx_posted: bool,
}

pub struct VirtioSerialDriver {
    inner: UnsafeCell<Inner>,
}

// VirtioSerialDriver is only accessed under kernel synchronization.
unsafe impl Send for VirtioSerialDriver {}
unsafe impl Sync for VirtioSerialDriver {}

impl Inner {
    fn post_rx_buffer(&mut self) {
        if self.rx_posted {
            return;
        }
        let writable = [(self.rx_buf_phys, BUF_SIZE as u32)];
        if self.receiveq.push(&[], &writable).is_some() {
            self.vdev.notify_queue(0);
            self.rx_posted = true;
        }
    }

    fn poll_rx(&mut self) {
        if let Some((_id, len)) = self.receiveq.poll_used() {
            self.rx_posted = false;
            let bytes = len as usize;
            if bytes > 0 && bytes <= BUF_SIZE {
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        self.rx_buf_phys as *const u8,
                        self.rx_pending.as_mut_ptr(),
                        bytes,
                    );
                }
                self.rx_pending_len = bytes;
                self.rx_pending_pos = 0;
            }
            self.post_rx_buffer();
        }
    }
}

impl VirtioSerialDriver {
    fn inner(&self) -> &mut Inner {
        unsafe { &mut *self.inner.get() }
    }
}

impl Driver for VirtioSerialDriver {
    fn name(&self) -> &str {
        "VirtIO Serial"
    }

    fn driver_type(&self) -> DriverType {
        DriverType::Char
    }

    fn init(&mut self) -> Result<(), DriverError> {
        self.inner().post_rx_buffer();
        Ok(())
    }

    fn read(&self, _offset: usize, buf: &mut [u8]) -> Result<usize, DriverError> {
        let this = self.inner();

        if this.rx_pending_pos < this.rx_pending_len {
            let avail = this.rx_pending_len - this.rx_pending_pos;
            let copy_len = avail.min(buf.len());
            buf[..copy_len].copy_from_slice(
                &this.rx_pending[this.rx_pending_pos..this.rx_pending_pos + copy_len],
            );
            this.rx_pending_pos += copy_len;
            return Ok(copy_len);
        }

        this.poll_rx();

        if this.rx_pending_pos < this.rx_pending_len {
            let avail = this.rx_pending_len - this.rx_pending_pos;
            let copy_len = avail.min(buf.len());
            buf[..copy_len].copy_from_slice(
                &this.rx_pending[this.rx_pending_pos..this.rx_pending_pos + copy_len],
            );
            this.rx_pending_pos += copy_len;
            Ok(copy_len)
        } else {
            Ok(0)
        }
    }

    fn write(&self, _offset: usize, buf: &[u8]) -> Result<usize, DriverError> {
        if buf.is_empty() {
            return Ok(0);
        }

        let this = self.inner();

        let send_len = buf.len().min(BUF_SIZE);
        unsafe {
            core::ptr::copy_nonoverlapping(buf.as_ptr(), this.tx_buf_phys as *mut u8, send_len);
        }

        let readable = [(this.tx_buf_phys, send_len as u32)];
        let result = this
            .transmitq
            .execute_sync(&readable, &[], || this.vdev.notify_queue(1));

        match result {
            Some(_) => Ok(send_len),
            None => Err(DriverError::IoError),
        }
    }

    fn ioctl(&mut self, _cmd: u32, _arg: u32) -> Result<u32, DriverError> {
        Err(DriverError::NotSupported)
    }
}

// ── Probe Function ───────────────────────────────────────────────────────────

/// Probe and initialize a VirtIO Serial device from the PCI bus.
///
/// Called by the PCI driver table when a matching device is found.
pub fn probe(pci: &PciDevice) -> Option<Box<dyn Driver>> {
    crate::serial_verbose_println!(
        "VirtIO Serial: probing PCI {:04x}:{:04x}",
        pci.vendor_id,
        pci.device_id
    );

    // 1. Find VirtIO PCI capabilities.
    let caps = virtio::find_capabilities(pci)?;

    // 2. Create device handle (maps BARs).
    let vdev = VirtioDevice::new(pci, &caps);

    // 3. Initialize device, negotiate features.
    // We only need VERSION_1. Skip MULTIPORT for simplicity (single port mode).
    let desired_features = VIRTIO_F_VERSION_1;
    if let Err(e) = vdev.init_device(desired_features) {
        crate::serial_verbose_println!("VirtIO Serial: init failed: {}", e);
        return None;
    }

    // 4. Set up virtqueues: receiveq (0) and transmitq (1).
    let receiveq = vdev.setup_queue(0)?;
    let transmitq = vdev.setup_queue(1)?;

    // 5. Mark device ready.
    vdev.set_driver_ok();

    // 6. Allocate RX and TX DMA buffers.
    let rx_phys = physical::alloc_contiguous(BUF_PAGES)?.as_u64();
    let tx_phys = physical::alloc_contiguous(BUF_PAGES)?.as_u64();

    // Zero the buffers.
    unsafe {
        core::ptr::write_bytes(rx_phys as *mut u8, 0, BUF_SIZE);
        core::ptr::write_bytes(tx_phys as *mut u8, 0, BUF_SIZE);
    }

    let mut driver = VirtioSerialDriver {
        inner: UnsafeCell::new(Inner {
            vdev,
            receiveq,
            transmitq,
            rx_buf_phys: rx_phys,
            tx_buf_phys: tx_phys,
            rx_pending: [0u8; BUF_SIZE],
            rx_pending_len: 0,
            rx_pending_pos: 0,
            rx_posted: false,
        }),
    };

    // Initialize (posts initial RX buffer).
    let _ = driver.init();

    // Register as /dev/vportN with custom path (HAL would use /dev/charN).
    let mut count = PORT_COUNT.lock();
    let port_num = *count;
    *count += 1;
    drop(count);

    let path = {
        let mut s = String::from("/dev/vport");
        use core::fmt::Write;
        let _ = write!(s, "{}", port_num);
        s
    };

    crate::serial_verbose_println!("VirtIO Serial: registered as {}", path);
    hal::register_device(&path, Box::new(driver), Some(pci.clone()));

    // Return None — we already registered via HAL with a custom path.
    None
}
