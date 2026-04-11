//! ARM64 network subsystem shim.
//!
//! The common kernel still expects the x86-era `crate::drivers::network::*`
//! API. For ARM64 we provide the same facade, ready to be backed by a native
//! MMIO/PCIe implementation later without touching higher layers.

pub trait NetworkDriver {
    fn transmit(&mut self, data: &[u8]) -> bool;
    fn get_mac(&self) -> [u8; 6];
    fn link_up(&self) -> bool;
}

/// No network device is wired up on ARM64 yet.
pub fn transmit(_data: &[u8]) -> bool {
    false
}

/// No network device is wired up on ARM64 yet.
pub fn get_mac() -> Option<[u8; 6]> {
    None
}

/// No network device is wired up on ARM64 yet.
pub fn is_available() -> bool {
    false
}

/// No network device is wired up on ARM64 yet.
pub fn link_up() -> bool {
    false
}

/// No-op until an ARM64 NIC backend exists.
pub fn set_enabled(_enabled: bool) {}

/// No network device is wired up on ARM64 yet.
pub fn is_enabled() -> bool {
    false
}

/// No WiFi backend is wired up on ARM64 yet.
pub fn with_wifi<F, R>(_f: F) -> Option<R>
where
    F: FnOnce(&mut dyn NetworkDriver) -> R,
{
    None
}

/// No WiFi backend is wired up on ARM64 yet.
pub fn wifi_available() -> bool {
    false
}
