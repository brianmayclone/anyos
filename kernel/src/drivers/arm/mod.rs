//! ARM64 VirtIO MMIO device drivers.
//!
//! Self-contained VirtIO MMIO transport and device drivers for QEMU `-M virt`.
//! This module is compiled only on `aarch64` and has zero interaction with the
//! x86-specific `drivers::virtio` / `drivers::pci` modules.
//!
//! QEMU virt machine VirtIO MMIO layout:
//!   - 32 device slots at physical addresses `0x0a00_0000 + slot * 0x200`
//!   - IRQs: SPI 48 + slot  (GIC interrupt IDs 48..79)
//!
//! MMIO virtual address mapping uses boot.S TTBR1 PUD[3]:
//!   VA `0xFFFF_0000_C000_0000` → PA `0x0000_0000` (1 GiB Device block)
//!   So: `virt_addr = phys_addr + 0xFFFF_0000_C000_0000`

pub mod virtqueue;
pub mod blk;
pub mod storage;
pub mod gpu;
pub mod input;

use core::ptr;

// ---------------------------------------------------------------------------
// MMIO address translation
// ---------------------------------------------------------------------------

/// TTBR1 PUD[3] maps VA 0xFFFF_0000_C000_0000 → PA 0x0000_0000.
const DEVICE_VIRT_BASE: usize = 0xFFFF_0000_C000_0000;

/// Convert a physical MMIO address to its kernel virtual address.
#[inline]
pub fn phys_to_mmio_virt(phys: usize) -> usize {
    DEVICE_VIRT_BASE + phys
}

// ---------------------------------------------------------------------------
// VirtIO MMIO register offsets (VirtIO Spec 4.2.2)
// ---------------------------------------------------------------------------

const VIRTIO_MMIO_MAGIC: usize = 0x000;
const VIRTIO_MMIO_VERSION: usize = 0x004;
const VIRTIO_MMIO_DEVICE_ID: usize = 0x008;
const VIRTIO_MMIO_VENDOR_ID: usize = 0x00C;
const VIRTIO_MMIO_DEVICE_FEATURES: usize = 0x010;
const VIRTIO_MMIO_DEVICE_FEATURES_SEL: usize = 0x014;
const VIRTIO_MMIO_DRIVER_FEATURES: usize = 0x020;
const VIRTIO_MMIO_DRIVER_FEATURES_SEL: usize = 0x024;
const VIRTIO_MMIO_QUEUE_SEL: usize = 0x030;
const VIRTIO_MMIO_QUEUE_NUM_MAX: usize = 0x034;
const VIRTIO_MMIO_QUEUE_NUM: usize = 0x038;
// Version 1 (legacy) specific registers
const VIRTIO_MMIO_GUEST_PAGE_SIZE: usize = 0x028;
const VIRTIO_MMIO_QUEUE_PFN: usize = 0x040;
const VIRTIO_MMIO_QUEUE_READY: usize = 0x044;
const VIRTIO_MMIO_QUEUE_NOTIFY: usize = 0x050;
const VIRTIO_MMIO_INTERRUPT_STATUS: usize = 0x060;
const VIRTIO_MMIO_INTERRUPT_ACK: usize = 0x064;
const VIRTIO_MMIO_STATUS: usize = 0x070;
const VIRTIO_MMIO_QUEUE_DESC_LOW: usize = 0x080;
const VIRTIO_MMIO_QUEUE_DESC_HIGH: usize = 0x084;
const VIRTIO_MMIO_QUEUE_DRIVER_LOW: usize = 0x090;
const VIRTIO_MMIO_QUEUE_DRIVER_HIGH: usize = 0x094;
const VIRTIO_MMIO_QUEUE_DEVICE_LOW: usize = 0x0A0;
const VIRTIO_MMIO_QUEUE_DEVICE_HIGH: usize = 0x0A4;
const VIRTIO_MMIO_CONFIG_GENERATION: usize = 0x0FC;
const VIRTIO_MMIO_CONFIG: usize = 0x100;

/// Expected magic value ("virt" in little-endian).
const VIRTIO_MAGIC: u32 = 0x7472_6976;

/// VirtIO device status bits.
pub const STATUS_ACKNOWLEDGE: u32 = 1;
pub const STATUS_DRIVER: u32 = 2;
pub const STATUS_FEATURES_OK: u32 = 8;
pub const STATUS_DRIVER_OK: u32 = 4;
pub const STATUS_FAILED: u32 = 128;

// ---------------------------------------------------------------------------
// QEMU virt VirtIO MMIO layout
// ---------------------------------------------------------------------------

/// Physical base address of VirtIO MMIO region on QEMU virt.
const VIRTIO_MMIO_PHYS_BASE: usize = 0x0a00_0000;
/// Number of VirtIO device slots on QEMU virt.
const VIRTIO_MMIO_NUM_SLOTS: usize = 32;
/// Size of each VirtIO MMIO device register block.
const VIRTIO_MMIO_SLOT_SIZE: usize = 0x200;
/// First SPI IRQ number for VirtIO devices on QEMU virt.
const VIRTIO_MMIO_IRQ_BASE: u32 = 48;

// ---------------------------------------------------------------------------
// VirtioMmioDevice
// ---------------------------------------------------------------------------

/// A discovered VirtIO MMIO device.
pub struct VirtioMmioDevice {
    /// Kernel virtual base address of the device's register block.
    base: usize,
    /// VirtIO device ID (1=net, 2=blk, 4=rng, 16=gpu, 18=input).
    dev_id: u32,
    /// GIC SPI interrupt number.
    irq: u32,
}

impl VirtioMmioDevice {
    /// Read a 32-bit MMIO register.
    #[inline]
    pub fn read_reg(&self, offset: usize) -> u32 {
        unsafe { ptr::read_volatile((self.base + offset) as *const u32) }
    }

    /// Write a 32-bit MMIO register.
    #[inline]
    pub fn write_reg(&self, offset: usize, val: u32) {
        unsafe { ptr::write_volatile((self.base + offset) as *mut u32, val); }
    }

    /// Get the VirtIO device ID.
    #[inline]
    pub fn device_id(&self) -> u32 {
        self.dev_id
    }

    /// Get the GIC IRQ number for this device.
    #[inline]
    pub fn irq(&self) -> u32 {
        self.irq
    }

    /// Get the kernel virtual base address.
    #[inline]
    pub fn base(&self) -> usize {
        self.base
    }

    /// Read the device status register.
    pub fn get_status(&self) -> u32 {
        self.read_reg(VIRTIO_MMIO_STATUS)
    }

    /// Write the device status register.
    pub fn set_status(&self, status: u32) {
        self.write_reg(VIRTIO_MMIO_STATUS, status);
    }

    /// Reset the device (write 0 to status).
    pub fn reset(&self) {
        self.set_status(0);
    }

    /// Read device features (32 bits at a time, selected by `sel`).
    pub fn read_device_features(&self, sel: u32) -> u32 {
        self.write_reg(VIRTIO_MMIO_DEVICE_FEATURES_SEL, sel);
        self.read_reg(VIRTIO_MMIO_DEVICE_FEATURES)
    }

    /// Write driver features (32 bits at a time, selected by `sel`).
    pub fn write_driver_features(&self, sel: u32, features: u32) {
        self.write_reg(VIRTIO_MMIO_DRIVER_FEATURES_SEL, sel);
        self.write_reg(VIRTIO_MMIO_DRIVER_FEATURES, features);
    }

    /// Perform the standard VirtIO initialization handshake.
    ///
    /// Returns the negotiated feature bits (low 32 bits only for simplicity).
    /// On failure, sets STATUS_FAILED and returns None.
    pub fn init_device(&self, driver_features: u32) -> Option<u32> {
        // 1. Reset
        self.reset();

        // 2. Acknowledge
        self.set_status(STATUS_ACKNOWLEDGE);

        // 3. Driver
        self.set_status(self.get_status() | STATUS_DRIVER);

        // 4. Feature negotiation (low 32 bits)
        let device_features = self.read_device_features(0);
        let negotiated = device_features & driver_features;
        self.write_driver_features(0, negotiated);

        // 4b. For Version 2 (modern): negotiate VIRTIO_F_VERSION_1 (bit 32).
        // Version 1 (legacy) devices don't support high feature bits.
        if self.version() >= 2 {
            let device_features_hi = self.read_device_features(1);
            let version_1_bit = 1u32; // bit 0 of high word = bit 32 overall
            let negotiated_hi = device_features_hi & version_1_bit;
            self.write_driver_features(1, negotiated_hi);
        }

        // 5. Features OK
        self.set_status(self.get_status() | STATUS_FEATURES_OK);
        if self.get_status() & STATUS_FEATURES_OK == 0 {
            self.set_status(self.get_status() | STATUS_FAILED);
            return None;
        }

        Some(negotiated)
    }

    /// Mark the device as fully initialized (DRIVER_OK).
    pub fn driver_ok(&self) {
        self.set_status(self.get_status() | STATUS_DRIVER_OK);
    }

    /// Get the MMIO version (1 = legacy, 2 = modern).
    #[inline]
    pub fn version(&self) -> u32 {
        self.read_reg(VIRTIO_MMIO_VERSION)
    }

    /// Set up a virtqueue at the given index.
    ///
    /// Supports both VirtIO MMIO Version 1 (legacy) and Version 2 (modern).
    pub fn setup_queue_raw(&self, queue_idx: u16, num: u16,
                           desc_phys: u64, _driver_phys: u64, _device_phys: u64) -> bool {
        self.write_reg(VIRTIO_MMIO_QUEUE_SEL, queue_idx as u32);

        let max = self.read_reg(VIRTIO_MMIO_QUEUE_NUM_MAX);
        if max == 0 || (num as u32) > max {
            return false;
        }

        self.write_reg(VIRTIO_MMIO_QUEUE_NUM, num as u32);

        if self.version() >= 2 {
            // Version 2 (modern): separate addresses for desc/driver/device rings
            self.write_reg(VIRTIO_MMIO_QUEUE_DESC_LOW, desc_phys as u32);
            self.write_reg(VIRTIO_MMIO_QUEUE_DESC_HIGH, (desc_phys >> 32) as u32);
            self.write_reg(VIRTIO_MMIO_QUEUE_DRIVER_LOW, _driver_phys as u32);
            self.write_reg(VIRTIO_MMIO_QUEUE_DRIVER_HIGH, (_driver_phys >> 32) as u32);
            self.write_reg(VIRTIO_MMIO_QUEUE_DEVICE_LOW, _device_phys as u32);
            self.write_reg(VIRTIO_MMIO_QUEUE_DEVICE_HIGH, (_device_phys >> 32) as u32);
            self.write_reg(VIRTIO_MMIO_QUEUE_READY, 1);
        } else {
            // Version 1 (legacy): GuestPageSize + QueuePFN (single contiguous allocation)
            // The device expects all three ring components in a single contiguous area
            // starting at desc_phys, divided by GuestPageSize.
            self.write_reg(VIRTIO_MMIO_GUEST_PAGE_SIZE, 4096);
            self.write_reg(VIRTIO_MMIO_QUEUE_PFN, (desc_phys / 4096) as u32);
        }
        true
    }

    /// Notify the device that a queue has new buffers.
    ///
    /// Includes a DSB barrier to ensure all virtqueue memory writes (Normal
    /// cacheable) are complete before the MMIO notification write (Device memory).
    #[inline]
    pub fn notify_queue(&self, queue_idx: u16) {
        // DSB ensures all prior stores to Normal memory (virtqueue descriptors,
        // available ring) complete before the Device-nGnRnE MMIO write.
        unsafe { core::arch::asm!("dsb sy", options(nostack, preserves_flags)); }
        self.write_reg(VIRTIO_MMIO_QUEUE_NOTIFY, queue_idx as u32);
    }

    /// Acknowledge device interrupt.
    pub fn ack_interrupt(&self) -> u32 {
        let status = self.read_reg(VIRTIO_MMIO_INTERRUPT_STATUS);
        self.write_reg(VIRTIO_MMIO_INTERRUPT_ACK, status);
        status
    }

    /// Read a device-config register at the given byte offset.
    pub fn read_config_u32(&self, offset: usize) -> u32 {
        self.read_reg(VIRTIO_MMIO_CONFIG + offset)
    }

    /// Read a device-config u64 (low + high).
    pub fn read_config_u64(&self, offset: usize) -> u64 {
        let lo = self.read_config_u32(offset) as u64;
        let hi = self.read_config_u32(offset + 4) as u64;
        lo | (hi << 32)
    }

    /// Write a device-config register at the given byte offset.
    pub fn write_config_u32(&self, offset: usize, val: u32) {
        self.write_reg(VIRTIO_MMIO_CONFIG + offset, val);
    }

    /// Read a single byte from device config at the given byte offset.
    pub fn read_config_u8(&self, offset: usize) -> u8 {
        unsafe { ptr::read_volatile((self.base + VIRTIO_MMIO_CONFIG + offset) as *const u8) }
    }

    /// Write a single byte to device config at the given byte offset.
    pub fn write_config_u8(&self, offset: usize, val: u8) {
        unsafe { ptr::write_volatile((self.base + VIRTIO_MMIO_CONFIG + offset) as *mut u8, val); }
    }
}

// ---------------------------------------------------------------------------
// Device discovery
// ---------------------------------------------------------------------------

/// Scan all VirtIO MMIO slots on QEMU virt and return discovered devices.
pub fn probe_all() -> alloc::vec::Vec<VirtioMmioDevice> {
    let mut devices = alloc::vec::Vec::new();

    for slot in 0..VIRTIO_MMIO_NUM_SLOTS {
        let phys = VIRTIO_MMIO_PHYS_BASE + slot * VIRTIO_MMIO_SLOT_SIZE;
        let base = phys_to_mmio_virt(phys);

        let magic = unsafe { ptr::read_volatile(base as *const u32) };
        if magic != VIRTIO_MAGIC {
            continue;
        }

        let version = unsafe { ptr::read_volatile((base + VIRTIO_MMIO_VERSION) as *const u32) };
        if version < 1 {
            continue;
        }

        let dev_id = unsafe { ptr::read_volatile((base + VIRTIO_MMIO_DEVICE_ID) as *const u32) };
        if dev_id == 0 {
            continue;
        }

        let irq = VIRTIO_MMIO_IRQ_BASE + slot as u32;

        crate::serial_println!("    slot {}: id={} version={} base={:#x}", slot, dev_id, version, base);
        devices.push(VirtioMmioDevice { base, dev_id, irq });
    }

    devices
}

extern crate alloc;
