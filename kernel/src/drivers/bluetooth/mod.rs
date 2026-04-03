//! Bluetooth subsystem.
//!
//! Implements the Bluetooth Host Controller Interface (HCI) over USB transport,
//! plus the L2CAP signaling layer needed for higher-level profiles.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────┐  ┌──────────────┐
//! │ HID Profile  │  │ Future: A2DP │
//! └──────┬───────┘  └──────┬───────┘
//!        │                  │
//! ┌──────┴──────────────────┴──────┐
//! │            L2CAP               │
//! └──────────────┬─────────────────┘
//!                │
//! ┌──────────────┴─────────────────┐
//! │         HCI Core               │
//! └──────────────┬─────────────────┘
//!                │
//! ┌──────────────┴─────────────────┐
//! │       USB HCI Transport        │
//! └────────────────────────────────┘
//! ```
//!
//! # Supported Features (initial)
//! - USB Bluetooth adapter detection (class E0:01:01)
//! - HCI command/event processing
//! - HCI inquiry (device discovery)
//! - L2CAP connection management
//! - HID profile (keyboard, mouse) — future

pub mod hci;
pub mod usb_transport;
pub mod l2cap;

use alloc::vec::Vec;
use crate::sync::spinlock::Spinlock;

/// Bluetooth Device Address (BD_ADDR), 6 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BdAddr(pub [u8; 6]);

impl BdAddr {
    pub fn is_zero(&self) -> bool {
        self.0 == [0; 6]
    }
}

impl core::fmt::Display for BdAddr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            self.0[5], self.0[4], self.0[3], self.0[2], self.0[1], self.0[0])
    }
}

/// A discovered Bluetooth device.
#[derive(Debug, Clone)]
pub struct BtDevice {
    pub addr: BdAddr,
    pub class_of_device: u32,
    pub rssi: i8,
    pub name: [u8; 248],
    pub name_len: u8,
}

/// Global list of discovered devices.
static DISCOVERED: Spinlock<Vec<BtDevice>> = Spinlock::new(Vec::new());

/// Check if a Bluetooth adapter is available.
pub fn is_available() -> bool {
    usb_transport::is_attached()
}

/// Start device discovery (inquiry). Non-blocking — results arrive via HCI events.
pub fn start_discovery() {
    hci::inquiry_start();
}

/// Get list of discovered devices.
pub fn discovered_devices() -> Vec<BtDevice> {
    DISCOVERED.lock().clone()
}

/// Add a discovered device (called from HCI event handler).
pub(crate) fn add_discovered(dev: BtDevice) {
    let mut list = DISCOVERED.lock();
    // Update if already known, otherwise add
    if let Some(existing) = list.iter_mut().find(|d| d.addr == dev.addr) {
        existing.rssi = dev.rssi;
        if dev.name_len > 0 {
            existing.name = dev.name;
            existing.name_len = dev.name_len;
        }
    } else {
        list.push(dev);
    }
}
