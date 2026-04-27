//! Qualcomm Atheros WiFi driver (ath10k/ath11k family).
//!
//! Second most common WiFi chipset in laptops (~15% market share).
//! PCIe devices, PCI vendor 0x168C (Atheros) or 0x17CB (Qualcomm).
//!
//! # Supported Chips
//! - QCA6174 (0x003E) — 802.11ac, very common in 2016-2020 laptops
//! - QCA9377 (0x0042) — 802.11ac, budget laptops
//! - QCA6390 (0x1101) — WiFi 6
//! - WCN6855 (0x1103) — WiFi 6E
//! - QCA9984 (0x0046) — 802.11ac Wave 2
//!
//! Like Intel WiFi, these use firmware-driven architecture with host-loaded
//! firmware blobs (ath10k/ or ath11k/ firmware directory).

use crate::drivers::pci::PciDevice;
use crate::memory::address::PhysAddr;
use crate::memory::{physical, virtual_mem};
use crate::sync::spinlock::Spinlock;
use alloc::boxed::Box;
use alloc::vec::Vec;

// ── Register Offsets ────────────────────────────────────────────────────────

const SOC_GLOBAL_RESET: u32 = 0x0008;
const SOC_CORE_CTRL: u32 = 0x0000;
const PCIE_INTR_ENABLE: u32 = 0x0008;
const PCIE_INTR_CLR: u32 = 0x0014;
const PCIE_INTR_CAUSE: u32 = 0x000C;
const SOC_CHIP_ID: u32 = 0x00EC;

// ── Driver State ────────────────────────────────────────────────────────────

struct AthState {
    mmio: u64,
    irq: u8,
    device_id: u16,
    chip_id: u32,
    mac: [u8; 6],
    fw_loaded: bool,
}

static ATH_STATE: Spinlock<Option<AthState>> = Spinlock::new(None);

pub fn init_and_register(pci: &PciDevice) -> bool {
    let bar0 = (pci.bars[0] & !0xF) as u64;
    if bar0 == 0 {
        return false;
    }

    let cmd = crate::drivers::pci::pci_config_read32(pci.bus, pci.device, pci.function, 0x04);
    crate::drivers::pci::pci_config_write32(pci.bus, pci.device, pci.function, 0x04, cmd | 0x07);

    let mmio = match virtual_mem::map_mmio(PhysAddr::new(bar0), 64) {
        Some(v) => v.as_u64(),
        None => return false,
    };

    let irq = pci.interrupt_line;

    unsafe {
        let chip_id = core::ptr::read_volatile((mmio + SOC_CHIP_ID as u64) as *const u32);
        crate::serial_println!(
            "  Atheros WiFi: device={:#06x} chip_id={:#010x}",
            pci.device_id,
            chip_id
        );

        // Disable interrupts
        core::ptr::write_volatile((mmio + PCIE_INTR_ENABLE as u64) as *mut u32, 0);
        core::ptr::write_volatile((mmio + PCIE_INTR_CLR as u64) as *mut u32, 0xFFFFFFFF);

        // Read MAC (varies by chip, often in OTP or board data)
        let mac = [0x00, 0x03, 0x7F, 0x00, 0x00, 0x00]; // Placeholder

        let chip_name = match pci.device_id {
            0x003E => "QCA6174",
            0x0042 => "QCA9377",
            0x0046 => "QCA9984",
            0x1101 => "QCA6390",
            0x1103 => "WCN6855",
            _ => "QCA",
        };

        *ATH_STATE.lock() = Some(AthState {
            mmio,
            irq,
            device_id: pci.device_id,
            chip_id,
            mac,
            fw_loaded: false,
        });

        // Try MSI
        let _ = crate::drivers::pci_msi::enable_msi(pci);

        crate::serial_println!(
            "[OK] Qualcomm {} WiFi (firmware loading required)",
            chip_name
        );
    }

    super::register_wifi(Box::new(AthWifiDriver));
    true
}

struct AthWifiDriver;
impl super::NetworkDriver for AthWifiDriver {
    fn name(&self) -> &str {
        "Qualcomm Atheros WiFi"
    }
    fn transmit(&mut self, _data: &[u8]) -> bool {
        false
    }
    fn get_mac(&self) -> [u8; 6] {
        ATH_STATE.lock().as_ref().map(|s| s.mac).unwrap_or([0; 6])
    }
    fn link_up(&self) -> bool {
        crate::net::wifi::is_connected()
    }
}

pub fn probe(pci: &PciDevice) -> Option<Box<dyn crate::drivers::hal::Driver>> {
    if init_and_register(pci) {
        super::create_hal_driver("Qualcomm Atheros WiFi")
    } else {
        None
    }
}
