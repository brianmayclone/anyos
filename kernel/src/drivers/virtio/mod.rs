//! VirtIO PCI modern transport layer.
//!
//! Provides PCI capability discovery, BAR mapping, device initialization,
//! and notification for VirtIO devices over PCI modern transport (capabilities-based).

pub mod virtqueue;

use crate::drivers::pci::{self, PciDevice};
use crate::memory::address::{PhysAddr, VirtAddr};
use crate::memory::virtual_mem;

// ──────────────────────────────────────────────
// PCI Capability Constants
// ──────────────────────────────────────────────

/// PCI capability ID for vendor-specific capabilities (used by VirtIO).
const PCI_CAP_ID_VENDOR: u8 = 0x09;

/// VirtIO PCI capability types (cfg_type field in the VirtIO PCI cap header).
const VIRTIO_PCI_CAP_COMMON_CFG: u8 = 1;
const VIRTIO_PCI_CAP_NOTIFY_CFG: u8 = 2;
const VIRTIO_PCI_CAP_ISR_CFG: u8 = 3;
const VIRTIO_PCI_CAP_DEVICE_CFG: u8 = 4;

// ──────────────────────────────────────────────
// Device Status Bits
// ──────────────────────────────────────────────

pub const STATUS_ACKNOWLEDGE: u8 = 1;
pub const STATUS_DRIVER: u8 = 2;
pub const STATUS_DRIVER_OK: u8 = 4;
pub const STATUS_FEATURES_OK: u8 = 8;
pub const STATUS_NEEDS_RESET: u8 = 64;
pub const STATUS_FAILED: u8 = 128;

// ──────────────────────────────────────────────
// Feature Bits
// ──────────────────────────────────────────────

/// VirtIO 1.0+ modern device compliance flag (feature bit 32).
pub const VIRTIO_F_VERSION_1: u64 = 1 << 32;

// ──────────────────────────────────────────────
// Common Config Offsets (from VirtIO spec §4.1.4.3)
// ──────────────────────────────────────────────

const COMMON_DEVICE_FEATURE_SELECT: usize = 0x00;
const COMMON_DEVICE_FEATURE: usize = 0x04;
const COMMON_DRIVER_FEATURE_SELECT: usize = 0x08;
const COMMON_DRIVER_FEATURE: usize = 0x0C;
const COMMON_MSIX_CONFIG: usize = 0x10;
const COMMON_NUM_QUEUES: usize = 0x12;
const COMMON_DEVICE_STATUS: usize = 0x14;
const COMMON_CONFIG_GENERATION: usize = 0x15;
const COMMON_QUEUE_SELECT: usize = 0x16;
const COMMON_QUEUE_SIZE: usize = 0x18;
const COMMON_QUEUE_MSIX_VECTOR: usize = 0x1A;
const COMMON_QUEUE_ENABLE: usize = 0x1C;
const COMMON_QUEUE_NOTIFY_OFF: usize = 0x1E;
const COMMON_QUEUE_DESC_LO: usize = 0x20;
const COMMON_QUEUE_DESC_HI: usize = 0x24;
const COMMON_QUEUE_AVAIL_LO: usize = 0x28;
const COMMON_QUEUE_AVAIL_HI: usize = 0x2C;
const COMMON_QUEUE_USED_LO: usize = 0x30;
const COMMON_QUEUE_USED_HI: usize = 0x34;

// ──────────────────────────────────────────────
// MMIO Virtual Address Base for VirtIO Devices
// ──────────────────────────────────────────────

/// Base virtual address for mapping VirtIO BARs (after AHCI ABAR).
const VIRTIO_MMIO_VIRT_BASE: u64 = 0xFFFF_FFFF_D008_0000;
/// Maximum pages to map per BAR.
const VIRTIO_MMIO_MAX_PAGES: usize = 16; // 64 KiB

// Track mapped BARs to avoid double-mapping
static mut MAPPED_BARS: [u64; 6] = [0; 6]; // virt addr per BAR index, 0 = not mapped

// ──────────────────────────────────────────────
// Discovered PCI Capabilities
// ──────────────────────────────────────────────

/// Locations of VirtIO PCI capability structures within BARs.
pub struct VirtioPciCaps {
    pub common_bar: u8,
    pub common_offset: u32,
    pub common_len: u32,
    pub notify_bar: u8,
    pub notify_offset: u32,
    pub notify_off_multiplier: u32,
    pub isr_bar: u8,
    pub isr_offset: u32,
    pub device_bar: u8,
    pub device_offset: u32,
    pub device_len: u32,
}

/// Walk the PCI capabilities list and find all VirtIO PCI capability structures.
pub fn find_capabilities(pci: &PciDevice) -> Option<VirtioPciCaps> {
    // Check if device has capabilities list (Status register bit 4)
    let status = pci::pci_config_read16(pci.bus, pci.device, pci.function, 0x06);
    if status & (1 << 4) == 0 {
        crate::serial_println!("  VirtIO: device has no capabilities list");
        return None;
    }

    let mut caps = VirtioPciCaps {
        common_bar: 0, common_offset: 0, common_len: 0,
        notify_bar: 0, notify_offset: 0, notify_off_multiplier: 0,
        isr_bar: 0, isr_offset: 0,
        device_bar: 0, device_offset: 0, device_len: 0,
    };

    let mut found_common = false;
    let mut found_notify = false;
    let mut found_isr = false;
    let mut _found_device = false;

    // Capabilities pointer at config offset 0x34
    let mut cap_offset = pci::pci_config_read8(pci.bus, pci.device, pci.function, 0x34);
    cap_offset &= 0xFC; // Must be dword-aligned

    let mut iterations = 0;
    while cap_offset != 0 && iterations < 48 {
        iterations += 1;

        let cap_id = pci::pci_config_read8(pci.bus, pci.device, pci.function, cap_offset);
        let cap_next = pci::pci_config_read8(pci.bus, pci.device, pci.function, cap_offset + 1);

        if cap_id == PCI_CAP_ID_VENDOR {
            // VirtIO PCI capability structure:
            // offset+2: cfg_type (u8)
            // offset+3: bar (u8)
            // offset+4: padding (3 bytes) + id (1 byte) — we skip
            // offset+8: offset within BAR (u32)
            // offset+12: length (u32)
            let cfg_type = pci::pci_config_read8(pci.bus, pci.device, pci.function, cap_offset + 3);
            let bar = pci::pci_config_read8(pci.bus, pci.device, pci.function, cap_offset + 4);
            let bar_offset = pci::pci_config_read32(pci.bus, pci.device, pci.function, cap_offset + 8);
            let length = pci::pci_config_read32(pci.bus, pci.device, pci.function, cap_offset + 12);

            match cfg_type {
                VIRTIO_PCI_CAP_COMMON_CFG => {
                    caps.common_bar = bar;
                    caps.common_offset = bar_offset;
                    caps.common_len = length;
                    found_common = true;
                    crate::serial_println!("    COMMON_CFG: BAR{} offset={:#x} len={}", bar, bar_offset, length);
                }
                VIRTIO_PCI_CAP_NOTIFY_CFG => {
                    caps.notify_bar = bar;
                    caps.notify_offset = bar_offset;
                    // Notify has an extra u32 at cap_offset+16: notify_off_multiplier
                    caps.notify_off_multiplier = pci::pci_config_read32(
                        pci.bus, pci.device, pci.function, cap_offset + 16
                    );
                    found_notify = true;
                    crate::serial_println!(
                        "    NOTIFY_CFG: BAR{} offset={:#x} mul={}",
                        bar, bar_offset, caps.notify_off_multiplier
                    );
                }
                VIRTIO_PCI_CAP_ISR_CFG => {
                    caps.isr_bar = bar;
                    caps.isr_offset = bar_offset;
                    found_isr = true;
                    crate::serial_println!("    ISR_CFG: BAR{} offset={:#x}", bar, bar_offset);
                }
                VIRTIO_PCI_CAP_DEVICE_CFG => {
                    caps.device_bar = bar;
                    caps.device_offset = bar_offset;
                    caps.device_len = length;
                    _found_device = true;
                    crate::serial_println!("    DEVICE_CFG: BAR{} offset={:#x} len={}", bar, bar_offset, length);
                }
                _ => {}
            }
        }

        cap_offset = cap_next & 0xFC;
    }

    if found_common && found_notify && found_isr {
        Some(caps)
    } else {
        crate::serial_println!("  VirtIO: missing required capabilities (common={} notify={} isr={})",
            found_common, found_notify, found_isr);
        None
    }
}

/// Map a PCI BAR's MMIO region into kernel virtual address space.
/// Returns the virtual base address of the mapped region.
/// Each BAR is mapped only once; subsequent calls return the cached address.
pub fn map_bar(pci: &PciDevice, bar_idx: u8) -> u64 {
    let idx = bar_idx as usize;
    if idx >= 6 {
        return 0;
    }

    // Check if already mapped
    unsafe {
        if MAPPED_BARS[idx] != 0 {
            return MAPPED_BARS[idx];
        }
    }

    let bar_value = pci.bars[idx];
    if bar_value == 0 {
        return 0;
    }

    // Check if MMIO (bit 0 = 0) or I/O (bit 0 = 1)
    if bar_value & 1 != 0 {
        // I/O space — return raw I/O base (no MMIO mapping needed)
        return (bar_value & !0x3) as u64;
    }

    let phys_base = (bar_value & !0xF) as u64;

    // Check for 64-bit BAR (type field bits 2:1)
    let bar_type = (bar_value >> 1) & 0x3;
    let phys_base = if bar_type == 2 && idx + 1 < 6 {
        // 64-bit BAR: combine with next BAR for upper 32 bits
        let hi = pci.bars[idx + 1] as u64;
        phys_base | (hi << 32)
    } else {
        phys_base
    };

    // Calculate virtual address for this BAR
    let virt_base = VIRTIO_MMIO_VIRT_BASE + (idx as u64) * (VIRTIO_MMIO_MAX_PAGES as u64 * 4096);

    // Map pages
    for i in 0..VIRTIO_MMIO_MAX_PAGES {
        let virt = VirtAddr::new(virt_base + (i as u64) * 4096);
        let phys = PhysAddr::new(phys_base + (i as u64) * 4096);
        virtual_mem::map_page(virt, phys, 0x03); // Present | Writable
    }

    unsafe {
        MAPPED_BARS[idx] = virt_base;
    }

    crate::serial_println!("  VirtIO: BAR{} phys={:#x} mapped to virt={:#x} ({} pages)",
        bar_idx, phys_base, virt_base, VIRTIO_MMIO_MAX_PAGES);

    virt_base
}

// ──────────────────────────────────────────────
// MMIO Volatile Access Helpers
// ──────────────────────────────────────────────

/// Read a u8 from MMIO at the given virtual address.
#[inline(always)]
pub fn mmio_read8(addr: u64) -> u8 {
    unsafe { core::ptr::read_volatile(addr as *const u8) }
}

/// Read a u16 from MMIO at the given virtual address.
#[inline(always)]
pub fn mmio_read16(addr: u64) -> u16 {
    unsafe { core::ptr::read_volatile(addr as *const u16) }
}

/// Read a u32 from MMIO at the given virtual address.
#[inline(always)]
pub fn mmio_read32(addr: u64) -> u32 {
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

/// Write a u8 to MMIO at the given virtual address.
#[inline(always)]
pub fn mmio_write8(addr: u64, val: u8) {
    unsafe { core::ptr::write_volatile(addr as *mut u8, val); }
}

/// Write a u16 to MMIO at the given virtual address.
#[inline(always)]
pub fn mmio_write16(addr: u64, val: u16) {
    unsafe { core::ptr::write_volatile(addr as *mut u16, val); }
}

/// Write a u32 to MMIO at the given virtual address.
#[inline(always)]
pub fn mmio_write32(addr: u64, val: u32) {
    unsafe { core::ptr::write_volatile(addr as *mut u32, val); }
}

// ──────────────────────────────────────────────
// VirtIO Device Handle
// ──────────────────────────────────────────────

/// Handle for accessing a VirtIO device's common config, notify, and ISR registers.
pub struct VirtioDevice {
    /// Virtual address of the common configuration structure.
    pub common_cfg: u64,
    /// Virtual address of the notification region base.
    pub notify_base: u64,
    /// Notification offset multiplier (bytes between queue notification addresses).
    pub notify_off_mul: u32,
    /// Virtual address of the ISR status register.
    pub isr_addr: u64,
    /// Virtual address of device-specific config (may be 0 if not present).
    pub device_cfg: u64,
}

impl VirtioDevice {
    /// Create a VirtioDevice by mapping BARs and computing MMIO addresses.
    pub fn new(pci: &PciDevice, caps: &VirtioPciCaps) -> Self {
        // Enable PCI bus mastering + memory + I/O
        let cmd = pci::pci_config_read32(pci.bus, pci.device, pci.function, 0x04);
        pci::pci_config_write32(pci.bus, pci.device, pci.function, 0x04, cmd | 0x07);

        // Map BARs
        let common_bar_virt = map_bar(pci, caps.common_bar);
        let notify_bar_virt = map_bar(pci, caps.notify_bar);
        let isr_bar_virt = map_bar(pci, caps.isr_bar);
        let device_cfg = if caps.device_len > 0 {
            let bar_virt = map_bar(pci, caps.device_bar);
            bar_virt + caps.device_offset as u64
        } else {
            0
        };

        VirtioDevice {
            common_cfg: common_bar_virt + caps.common_offset as u64,
            notify_base: notify_bar_virt + caps.notify_offset as u64,
            notify_off_mul: caps.notify_off_multiplier,
            isr_addr: isr_bar_virt + caps.isr_offset as u64,
            device_cfg,
        }
    }

    // ── Common config register access ──

    pub fn read_device_status(&self) -> u8 {
        mmio_read8(self.common_cfg + COMMON_DEVICE_STATUS as u64)
    }

    pub fn write_device_status(&self, status: u8) {
        mmio_write8(self.common_cfg + COMMON_DEVICE_STATUS as u64, status);
    }

    pub fn read_num_queues(&self) -> u16 {
        mmio_read16(self.common_cfg + COMMON_NUM_QUEUES as u64)
    }

    /// Read device feature bits (select which 32-bit block with `select`).
    pub fn read_device_features(&self, select: u32) -> u32 {
        mmio_write32(self.common_cfg + COMMON_DEVICE_FEATURE_SELECT as u64, select);
        mmio_read32(self.common_cfg + COMMON_DEVICE_FEATURE as u64)
    }

    /// Write driver feature bits (select which 32-bit block with `select`).
    pub fn write_driver_features(&self, select: u32, features: u32) {
        mmio_write32(self.common_cfg + COMMON_DRIVER_FEATURE_SELECT as u64, select);
        mmio_write32(self.common_cfg + COMMON_DRIVER_FEATURE as u64, features);
    }

    /// Select a virtqueue for configuration.
    pub fn select_queue(&self, queue_idx: u16) {
        mmio_write16(self.common_cfg + COMMON_QUEUE_SELECT as u64, queue_idx);
    }

    /// Read the maximum queue size for the currently selected queue.
    pub fn read_queue_size(&self) -> u16 {
        mmio_read16(self.common_cfg + COMMON_QUEUE_SIZE as u64)
    }

    /// Set the queue size for the currently selected queue.
    pub fn write_queue_size(&self, size: u16) {
        mmio_write16(self.common_cfg + COMMON_QUEUE_SIZE as u64, size);
    }

    /// Read the notification offset for the currently selected queue.
    pub fn read_queue_notify_off(&self) -> u16 {
        mmio_read16(self.common_cfg + COMMON_QUEUE_NOTIFY_OFF as u64)
    }

    /// Write the physical addresses of the virtqueue structures.
    pub fn write_queue_addresses(&self, desc: u64, avail: u64, used: u64) {
        mmio_write32(self.common_cfg + COMMON_QUEUE_DESC_LO as u64, desc as u32);
        mmio_write32(self.common_cfg + COMMON_QUEUE_DESC_HI as u64, (desc >> 32) as u32);
        mmio_write32(self.common_cfg + COMMON_QUEUE_AVAIL_LO as u64, avail as u32);
        mmio_write32(self.common_cfg + COMMON_QUEUE_AVAIL_HI as u64, (avail >> 32) as u32);
        mmio_write32(self.common_cfg + COMMON_QUEUE_USED_LO as u64, used as u32);
        mmio_write32(self.common_cfg + COMMON_QUEUE_USED_HI as u64, (used >> 32) as u32);
    }

    /// Enable the currently selected queue.
    pub fn enable_queue(&self) {
        mmio_write16(self.common_cfg + COMMON_QUEUE_ENABLE as u64, 1);
    }

    /// Notify the device that a virtqueue has new buffers.
    pub fn notify_queue(&self, queue_idx: u16) {
        let notify_off = {
            self.select_queue(queue_idx);
            self.read_queue_notify_off()
        };
        let addr = self.notify_base + (notify_off as u64) * (self.notify_off_mul as u64);
        mmio_write16(addr, queue_idx);
    }

    // ── Device initialization sequence ──

    /// Reset the device (write 0 to status).
    pub fn reset(&self) {
        self.write_device_status(0);
        // Wait for status to read back 0
        while self.read_device_status() != 0 {
            core::hint::spin_loop();
        }
    }

    /// Perform the standard VirtIO initialization up to DRIVER_OK.
    /// Returns the negotiated feature set (64-bit).
    pub fn init_device(&self, desired_features: u64) -> Result<u64, &'static str> {
        // 1. Reset
        self.reset();

        // 2. Acknowledge
        self.write_device_status(STATUS_ACKNOWLEDGE);

        // 3. Driver
        self.write_device_status(STATUS_ACKNOWLEDGE | STATUS_DRIVER);

        // 4. Read device features
        let dev_feat_lo = self.read_device_features(0) as u64;
        let dev_feat_hi = self.read_device_features(1) as u64;
        let device_features = dev_feat_lo | (dev_feat_hi << 32);

        crate::serial_println!("  VirtIO: device features = {:#018x}", device_features);

        // 5. Negotiate features
        let negotiated = device_features & desired_features;

        // VIRTIO_F_VERSION_1 is mandatory for modern devices
        if negotiated & VIRTIO_F_VERSION_1 == 0 {
            crate::serial_println!("  VirtIO: VIRTIO_F_VERSION_1 not available!");
            self.write_device_status(STATUS_FAILED);
            return Err("VIRTIO_F_VERSION_1 not available");
        }

        self.write_driver_features(0, negotiated as u32);
        self.write_driver_features(1, (negotiated >> 32) as u32);

        crate::serial_println!("  VirtIO: negotiated features = {:#018x}", negotiated);

        // 6. Set FEATURES_OK
        self.write_device_status(STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK);

        // 7. Verify FEATURES_OK stuck
        let status = self.read_device_status();
        if status & STATUS_FEATURES_OK == 0 {
            crate::serial_println!("  VirtIO: FEATURES_OK not accepted by device");
            self.write_device_status(STATUS_FAILED);
            return Err("FEATURES_OK rejected");
        }

        Ok(negotiated)
    }

    /// Set DRIVER_OK to signal the device that the driver is ready.
    pub fn set_driver_ok(&self) {
        let status = self.read_device_status();
        self.write_device_status(status | STATUS_DRIVER_OK);
    }

    /// Set up a virtqueue: allocate memory, configure addresses, enable.
    /// Returns the VirtQueue instance.
    pub fn setup_queue(&self, queue_idx: u16) -> Option<virtqueue::VirtQueue> {
        self.select_queue(queue_idx);

        let max_size = self.read_queue_size();
        if max_size == 0 {
            crate::serial_println!("  VirtIO: queue {} not available (max_size=0)", queue_idx);
            return None;
        }

        // Use min of max_size and 128
        let queue_size = max_size.min(128);
        self.write_queue_size(queue_size);

        // Allocate the queue
        let vq = virtqueue::VirtQueue::new(queue_size)?;

        // Write physical addresses to device
        self.write_queue_addresses(vq.desc_phys(), vq.avail_phys(), vq.used_phys());

        // Disable MSI-X for this queue (use legacy interrupts)
        mmio_write16(self.common_cfg + COMMON_QUEUE_MSIX_VECTOR as u64, 0xFFFF);

        // Enable
        self.enable_queue();

        crate::serial_println!("  VirtIO: queue {} enabled (size={})", queue_idx, queue_size);

        Some(vq)
    }
}
