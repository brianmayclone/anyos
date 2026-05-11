// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Network driver subsystem.
//!
//! Provides a unified [`NetworkDriver`] trait for NIC drivers (E1000, VirtIO-Net, etc.).
//! Drivers register dynamically via PCI detection in the HAL.
//! RX polling remains driver-specific (IRQ-driven, not routed through the trait).

pub mod ath_wifi;
pub mod e1000;
pub mod igc;
pub mod iwl_wifi;
pub mod rtl8125;
pub mod rtl8168;
pub mod rtl8188eu;
pub mod virtio_net;

use crate::sync::spinlock::Spinlock;
use alloc::boxed::Box;
use alloc::vec::Vec;

/// Unified network driver interface.
pub trait NetworkDriver: Send {
    /// Human-readable driver name.
    fn name(&self) -> &str;
    /// Transmit a packet. Returns true on success.
    fn transmit(&mut self, data: &[u8]) -> bool;
    /// Get the MAC address.
    fn get_mac(&self) -> [u8; 6];
    /// Check if the network link is up.
    fn link_up(&self) -> bool;
    /// Poll this NIC and append all received Ethernet frames to `out`.
    fn poll_rx_into(&mut self, _out: &mut Vec<Vec<u8>>) {}
    /// Enable the NIC. Default: no-op (always enabled).
    fn set_enabled(&mut self, _enabled: bool) {}
    /// Check if the NIC is enabled. Default: true if registered.
    fn is_enabled(&self) -> bool {
        true
    }
    /// Get NIC statistics: (rx_packets, tx_packets, rx_bytes, tx_bytes, rx_errors, tx_errors).
    fn get_stats(&self) -> (u64, u64, u64, u64, u64, u64) {
        (0, 0, 0, 0, 0, 0)
    }
    /// Get the NIC driver name for display (e.g. "e1000", "virtio-net").
    fn driver_name(&self) -> &str {
        self.name()
    }
}

/// Global network driver instance, set during PCI probe.
static NET: Spinlock<Option<Box<dyn NetworkDriver>>> = Spinlock::new(None);

/// Register a network driver (called from driver init during PCI probe).
pub fn register(mut driver: Box<dyn NetworkDriver>) {
    crate::serial_verbose_println!("  Network: registered '{}'", driver.name());
    let mut net = NET.lock();
    if let Some(old) = net.as_mut() {
        old.set_enabled(false);
    }
    driver.set_enabled(true);
    *net = Some(driver);
}

/// Access the registered network driver within a closure.
pub fn with_net<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut dyn NetworkDriver) -> R,
{
    let mut net = NET.lock();
    let driver = net.as_mut()?;
    Some(f(driver.as_mut()))
}

/// Transmit a packet via the registered network driver.
pub fn transmit(data: &[u8]) -> bool {
    with_net(|d| d.transmit(data)).unwrap_or(false)
}

/// Poll the registered primary NIC for received Ethernet frames.
pub fn poll_rx_into(out: &mut Vec<Vec<u8>>) {
    let _ = with_net(|d| d.poll_rx_into(out));
}

/// Get the MAC address of the registered NIC.
pub fn get_mac() -> Option<[u8; 6]> {
    with_net(|d| d.get_mac())
}

/// Check if network hardware is available and initialized.
pub fn is_available() -> bool {
    NET.lock().is_some()
}

/// Check if the network link is up.
pub fn link_up() -> bool {
    with_net(|d| d.link_up()).unwrap_or(false)
}

/// Enable or disable the NIC.
pub fn set_enabled(enabled: bool) {
    with_net(|d| d.set_enabled(enabled));
}

/// Check if the NIC is enabled.
pub fn is_enabled() -> bool {
    with_net(|d| d.is_enabled()).unwrap_or(false)
}

/// Get NIC statistics.
pub fn get_stats() -> (u64, u64, u64, u64, u64, u64) {
    with_net(|d| d.get_stats()).unwrap_or((0, 0, 0, 0, 0, 0))
}

/// Get NIC driver name for display.
pub fn driver_name() -> Option<alloc::string::String> {
    with_net(|d| alloc::string::String::from(d.driver_name()))
}

/// Check whether the named driver is the active primary NIC.
pub fn is_primary_driver(name: &str) -> bool {
    with_net(|d| d.driver_name() == name).unwrap_or(false)
}

// ── WiFi (second network slot) ───────────────────────────────────────────────

/// Global WiFi driver instance (e.g. RTL8188EU). Set during USB probe.
static WIFI: Spinlock<Option<Box<dyn NetworkDriver>>> = Spinlock::new(None);

/// Register a WiFi network driver (called from USB WiFi driver probe).
pub fn register_wifi(driver: Box<dyn NetworkDriver>) {
    crate::serial_verbose_println!("  Network: registered WiFi '{}'", driver.name());
    *WIFI.lock() = Some(driver);
}

/// Access the registered WiFi driver within a closure.
pub fn with_wifi<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut dyn NetworkDriver) -> R,
{
    let mut wifi = WIFI.lock();
    let driver = wifi.as_mut()?;
    Some(f(driver.as_mut()))
}

/// Transmit a packet via the registered WiFi driver.
pub fn wifi_transmit(data: &[u8]) -> bool {
    with_wifi(|d| d.transmit(data)).unwrap_or(false)
}

/// Check if a WiFi driver is registered.
pub fn wifi_available() -> bool {
    WIFI.lock().is_some()
}

// ── HAL integration ─────────────────────────────────────────────────────────

use crate::drivers::hal::{Driver, DriverError, DriverType};

struct NetworkHalDriver {
    name: &'static str,
}

impl Driver for NetworkHalDriver {
    fn name(&self) -> &str {
        self.name
    }
    fn driver_type(&self) -> DriverType {
        DriverType::Network
    }
    fn init(&mut self) -> Result<(), DriverError> {
        Ok(())
    }
    fn read(&self, _offset: usize, _buf: &mut [u8]) -> Result<usize, DriverError> {
        Err(DriverError::NotSupported)
    }
    fn write(&self, _offset: usize, buf: &[u8]) -> Result<usize, DriverError> {
        if transmit(buf) {
            Ok(buf.len())
        } else {
            Err(DriverError::IoError)
        }
    }
    fn ioctl(&mut self, _cmd: u32, _arg: u32) -> Result<u32, DriverError> {
        Err(DriverError::NotSupported)
    }
}

/// Create a HAL Driver wrapper for the network subsystem (called from driver probe).
pub(crate) fn create_hal_driver(name: &'static str) -> Option<Box<dyn Driver>> {
    Some(Box::new(NetworkHalDriver { name }))
}
