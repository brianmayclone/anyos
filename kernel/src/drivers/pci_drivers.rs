// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! PCI device-to-driver mapping table.
//!
//! Central registry of all supported PCI devices and their corresponding kernel
//! drivers. Edit this file to add or remove hardware support.
//!
//! Each driver module provides its own `probe()` function that initialises the
//! hardware and returns a `Box<dyn hal::Driver>`.  This file only wires PCI IDs
//! to those probe functions — no driver names, no detection logic.
//!
//! ## Match Types
//! - **VendorDevice** (specificity 2): Exact vendor:device ID match — highest priority.
//! - **Class** (specificity 1): PCI class:subclass match — fallback for generic drivers.
//!
//! When multiple entries match, the highest specificity wins.

use alloc::boxed::Box;
use crate::drivers::pci::PciDevice;
use super::hal::Driver;

// ──────────────────────────────────────────────────────────────────────────────
// PCI Match Engine
// ──────────────────────────────────────────────────────────────────────────────

pub(super) enum PciMatch {
    Class { class: u8, subclass: u8 },
    VendorDevice { vendor: u16, device: u16 },
}

pub(super) struct PciDriverEntry {
    pub match_rule: PciMatch,
    pub factory: fn(&PciDevice) -> Option<Box<dyn Driver>>,
    /// Higher = more specific match (vendor/device beats class)
    pub specificity: u8,
}

pub(super) fn matches_pci(rule: &PciMatch, dev: &PciDevice) -> bool {
    match rule {
        PciMatch::Class { class, subclass } => {
            dev.class_code == *class && dev.subclass == *subclass
        }
        PciMatch::VendorDevice { vendor, device } => {
            dev.vendor_id == *vendor && dev.device_id == *device
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// PCI Driver Table
//
// ┌──────────────┬──────────┬────────────────────────────────────────────────┐
// │ Match        │ ID       │ Driver                                         │
// ├──────────────┼──────────┼────────────────────────────────────────────────┤
// │ VendorDevice │ 1234:1111│ Bochs/QEMU VGA (GPU)                          │
// │ VendorDevice │ 80EE:BEEF│ VBoxVGA / VBoxSVGA (GPU, auto-detect)         │
// │ VendorDevice │ 15AD:0405│ VMware SVGA II (GPU)                          │
// │ VendorDevice │ 1AF4:1050│ VirtIO GPU                                    │
// │ VendorDevice │ 1000:0030│ LSI Logic Fusion-MPT SCSI (Storage)           │
// │ VendorDevice │ 80EE:4E56│ VirtualBox NVMe (Storage)                     │
// │ VendorDevice │ 80EE:CAFE│ VirtualBox VMMDev (Guest Integration)         │
// ├──────────────┼──────────┼────────────────────────────────────────────────┤
// │ Class        │ 01:01    │ IDE Controller (Storage)                       │
// │ Class        │ 01:06    │ AHCI SATA Controller (Storage)                │
// │ Class        │ 01:08    │ NVMe Controller (Storage)                     │
// │ Class        │ 02:00    │ Ethernet Controller (Network)                 │
// │ Class        │ 03:00    │ Generic VGA (Display, fallback)               │
// │ Class        │ 04:01    │ Intel AC'97 Audio                             │
// │ Class        │ 04:03    │ Intel HDA Audio                               │
// │ Class        │ 0C:03    │ USB Controller (UHCI/OHCI/EHCI/xHCI)         │
// │ Class        │ 0C:05    │ SMBus Controller                              │
// └──────────────┴──────────┴────────────────────────────────────────────────┘
// ──────────────────────────────────────────────────────────────────────────────

pub(super) static PCI_DRIVER_TABLE: &[PciDriverEntry] = &[
    // ── Vendor/Device matches (specificity 2) ──

    PciDriverEntry {
        match_rule: PciMatch::VendorDevice { vendor: 0x1234, device: 0x1111 },
        factory: |pci| crate::drivers::gpu::bochs_probe(pci),
        specificity: 2,
    },
    PciDriverEntry {
        match_rule: PciMatch::VendorDevice { vendor: 0x80EE, device: 0xBEEF },
        factory: |pci| crate::drivers::gpu::vbox_probe(pci),
        specificity: 2,
    },
    PciDriverEntry {
        match_rule: PciMatch::VendorDevice { vendor: 0x15AD, device: 0x0405 },
        factory: |pci| crate::drivers::gpu::vmware_svga::probe(pci),
        specificity: 2,
    },
    PciDriverEntry {
        match_rule: PciMatch::VendorDevice { vendor: 0x1AF4, device: 0x1050 },
        factory: |pci| crate::drivers::gpu::virtio_gpu::probe(pci),
        specificity: 2,
    },
    PciDriverEntry {
        match_rule: PciMatch::VendorDevice { vendor: 0x1000, device: 0x0030 },
        factory: |pci| crate::drivers::storage::lsi_scsi::probe(pci),
        specificity: 2,
    },
    PciDriverEntry {
        match_rule: PciMatch::VendorDevice { vendor: 0x80EE, device: 0x4E56 },
        factory: |pci| crate::drivers::storage::nvme::probe(pci),
        specificity: 2,
    },
    PciDriverEntry {
        match_rule: PciMatch::VendorDevice { vendor: 0x80EE, device: 0xCAFE },
        factory: |pci| crate::drivers::vmmdev::probe(pci),
        specificity: 2,
    },

    // ── Class-based matches (specificity 1) ──

    PciDriverEntry {
        match_rule: PciMatch::Class { class: 0x01, subclass: 0x01 },
        factory: |pci| crate::drivers::storage::ide_probe(pci),
        specificity: 1,
    },
    PciDriverEntry {
        match_rule: PciMatch::Class { class: 0x01, subclass: 0x06 },
        factory: |pci| crate::drivers::storage::ahci::probe(pci),
        specificity: 1,
    },
    PciDriverEntry {
        match_rule: PciMatch::Class { class: 0x01, subclass: 0x08 },
        factory: |pci| crate::drivers::storage::nvme::probe(pci),
        specificity: 1,
    },
    PciDriverEntry {
        match_rule: PciMatch::Class { class: 0x02, subclass: 0x00 },
        factory: |pci| crate::drivers::network::e1000::probe(pci),
        specificity: 1,
    },
    PciDriverEntry {
        match_rule: PciMatch::Class { class: 0x03, subclass: 0x00 },
        factory: |pci| crate::drivers::gpu::generic_vga_probe(pci),
        specificity: 1,
    },
    PciDriverEntry {
        match_rule: PciMatch::Class { class: 0x04, subclass: 0x01 },
        factory: |pci| crate::drivers::audio::ac97::probe(pci),
        specificity: 1,
    },
    PciDriverEntry {
        match_rule: PciMatch::Class { class: 0x04, subclass: 0x03 },
        factory: |pci| crate::drivers::audio::hda::probe(pci),
        specificity: 1,
    },
    PciDriverEntry {
        match_rule: PciMatch::Class { class: 0x0C, subclass: 0x03 },
        factory: |pci| crate::drivers::usb::probe(pci),
        specificity: 1,
    },
    PciDriverEntry {
        match_rule: PciMatch::Class { class: 0x0C, subclass: 0x05 },
        factory: |pci| crate::drivers::usb::smbus_probe(pci),
        specificity: 1,
    },
];
