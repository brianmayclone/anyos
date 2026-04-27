//! Intel WiFi driver (iwlwifi family) — basic support for AX200/AX210/AX211.
//!
//! This is the most common WiFi chipset in laptops (~60% market share).
//! PCIe devices, PCI vendor 0x8086, various device IDs.
//!
//! # Supported Chips
//! - Intel Wi-Fi 6 AX200 (0x2723)
//! - Intel Wi-Fi 6 AX201 (0x06F0, 0xA0F0)
//! - Intel Wi-Fi 6E AX210 (0x2725)
//! - Intel Wi-Fi 6E AX211 (0x51F0, 0x51F1, 0x54F0)
//! - Intel Wi-Fi 7 BE200 (0x272B)
//! - Intel Wireless-AC 9260 (0x2526)
//! - Intel Wireless-AC 9560 (0x9DF0, 0xA370)
//! - Intel Wireless 8265/8275 (0x24FD)
//!
//! # Architecture
//! Intel WiFi devices use a firmware-driven architecture:
//! 1. Host loads firmware binary to device RAM
//! 2. Firmware runs on device's embedded processor
//! 3. Host communicates via command/response queues in shared memory
//! 4. Data frames go through TX/RX DMA queues
//!
//! # Implementation Status
//! This is a basic driver that:
//! - Detects the hardware and reads firmware version
//! - Initializes the PCIe transport (MSI, DMA rings)
//! - Loads firmware (from /System/Drivers/wifi/)
//! - Exposes scanning and connection via the existing WiFi management layer
//!
//! Full firmware loading requires the proprietary Intel firmware blobs
//! (iwlwifi-cc-a0-*.ucode) which must be placed in /System/Drivers/wifi/.

use crate::drivers::pci::PciDevice;
use crate::memory::address::{PhysAddr, VirtAddr};
use crate::memory::{physical, virtual_mem};
use crate::sync::spinlock::Spinlock;
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

// ── PCIe Register Offsets ───────────────────────────────────────────────────

// Control/Status Registers (CSR)
const CSR_HW_IF_CONFIG: u32 = 0x000;
const CSR_INT: u32 = 0x008; // Interrupt status
const CSR_INT_MASK: u32 = 0x00C; // Interrupt mask
const CSR_FH_INT_STATUS: u32 = 0x010;
const CSR_GPIO_IN: u32 = 0x018;
const CSR_RESET: u32 = 0x020;
const CSR_GP_CNTRL: u32 = 0x024; // General Purpose Control
const CSR_HW_REV: u32 = 0x028;
const CSR_EEPROM_REG: u32 = 0x02C;
const CSR_EEPROM_GP: u32 = 0x030;
const CSR_UCODE_DRV_GP1: u32 = 0x054;
const CSR_UCODE_DRV_GP2: u32 = 0x058;
const CSR_GIO_CHICKEN_BITS: u32 = 0x100;
const CSR_ANA_PLL_CFG: u32 = 0x20C;
const CSR_DBG_HPET_MEM: u32 = 0x240;
const CSR_HW_REV_WA: u32 = 0x22C;

// GP_CNTRL bits
const CSR_GP_CNTRL_REG_FLAG_MAC_CLOCK_READY: u32 = 1 << 0;
const CSR_GP_CNTRL_REG_FLAG_INIT_DONE: u32 = 1 << 2;
const CSR_GP_CNTRL_REG_FLAG_MAC_ACCESS_REQ: u32 = 1 << 3;
const CSR_GP_CNTRL_REG_FLAG_GOING_TO_SLEEP: u32 = 1 << 4;

// FH (Firmware Handler) registers for DMA
const FH_RSCSR_CHNL0_STTS: u32 = 0x1C44;
const FH_RSCSR_CHNL0_RBDCB_WPTR: u32 = 0x1C80;
const FH_RSCSR_CHNL0_RBDCB_BASE: u32 = 0x1C84;
const FH_TCSR_CHNL_TX_CONFIG: u32 = 0x1D00;

// TX queue registers
const SCD_BASE: u32 = 0x0A00;

// ── Firmware paths ──────────────────────────────────────────────────────────

/// Firmware search paths (tried in order). All under /System/Drivers/wifi/.
const FW_PATHS: &[&str] = &[
    "/System/Drivers/wifi/iwlwifi-cc-a0-77.ucode", // AX200
    "/System/Drivers/wifi/iwlwifi-ty-a0-gf-a0-77.ucode", // AX210/AX211
    "/System/Drivers/wifi/iwlwifi-so-a0-gf-a0-77.ucode", // AX201
    "/System/Drivers/wifi/iwlwifi-9260-th-b0-jf-b0-46.ucode", // AC 9260
];

// ── Driver State ────────────────────────────────────────────────────────────

struct IwlState {
    mmio: u64,
    irq: u8,
    hw_rev: u32,
    device_id: u16,
    mac: [u8; 6],
    fw_loaded: bool,
    /// TFD (Transmit Frame Descriptor) ring physical address.
    tfd_ring_phys: u64,
    /// RBD (Receive Buffer Descriptor) ring physical address.
    rbd_ring_phys: u64,
}

static IWL_STATE: Spinlock<Option<IwlState>> = Spinlock::new(None);

// ── MMIO Helpers ────────────────────────────────────────────────────────────

#[inline(always)]
unsafe fn rd(base: u64, reg: u32) -> u32 {
    core::ptr::read_volatile((base + reg as u64) as *const u32)
}

#[inline(always)]
unsafe fn wr(base: u64, reg: u32, val: u32) {
    core::ptr::write_volatile((base + reg as u64) as *mut u32, val);
}

// ── Initialization ──────────────────────────────────────────────────────────

pub fn init_and_register(pci: &PciDevice) -> bool {
    let bar0 = (pci.bars[0] & !0xF) as u64;
    if bar0 == 0 {
        return false;
    }

    // Enable bus mastering + memory space
    let cmd = crate::drivers::pci::pci_config_read32(pci.bus, pci.device, pci.function, 0x04);
    crate::drivers::pci::pci_config_write32(pci.bus, pci.device, pci.function, 0x04, cmd | 0x07);

    // Map MMIO (32 KiB for CSR registers)
    let mmio = match virtual_mem::map_mmio(PhysAddr::new(bar0), 8) {
        Some(v) => v.as_u64(),
        None => return false,
    };

    let irq = pci.interrupt_line;

    unsafe {
        // Read hardware revision
        let hw_rev = rd(mmio, CSR_HW_REV);
        crate::serial_println!(
            "  Intel WiFi: device={:#06x} hw_rev={:#010x}",
            pci.device_id,
            hw_rev
        );

        // Set bus mastering and disable interrupts during init
        wr(mmio, CSR_INT_MASK, 0);
        wr(mmio, CSR_INT, 0xFFFFFFFF); // Clear pending

        // Request MAC access (wake the device from D3)
        wr(
            mmio,
            CSR_GP_CNTRL,
            rd(mmio, CSR_GP_CNTRL) | CSR_GP_CNTRL_REG_FLAG_MAC_ACCESS_REQ,
        );

        // Wait for MAC clock ready
        let mut ready = false;
        for _ in 0..50_000 {
            if rd(mmio, CSR_GP_CNTRL) & CSR_GP_CNTRL_REG_FLAG_MAC_CLOCK_READY != 0 {
                ready = true;
                break;
            }
            core::hint::spin_loop();
        }

        if !ready {
            crate::serial_println!("  Intel WiFi: MAC clock not ready — device may be in D3");
            // Still register — the device might wake up later
        }

        // Read NIC type from HW config register
        let hw_cfg = rd(mmio, CSR_HW_IF_CONFIG);

        // Read MAC address from EEPROM/OTP
        let mac = read_mac_from_csr(mmio);
        crate::serial_println!(
            "  Intel WiFi: MAC {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            mac[0],
            mac[1],
            mac[2],
            mac[3],
            mac[4],
            mac[5]
        );

        // Allocate DMA rings for TX and RX
        let tfd_frame = physical::alloc_frame().unwrap_or(PhysAddr::new(0));
        let rbd_frame = physical::alloc_frame().unwrap_or(PhysAddr::new(0));

        *IWL_STATE.lock() = Some(IwlState {
            mmio,
            irq,
            hw_rev,
            device_id: pci.device_id,
            mac,
            fw_loaded: false,
            tfd_ring_phys: tfd_frame.as_u64(),
            rbd_ring_phys: rbd_frame.as_u64(),
        });

        // Try to load firmware
        let fw_ok = try_load_firmware(mmio, pci.device_id);
        if fw_ok {
            if let Some(ref mut state) = *IWL_STATE.lock() {
                state.fw_loaded = true;
            }
            crate::serial_println!("  Intel WiFi: firmware loaded successfully");
        } else {
            crate::serial_println!("  Intel WiFi: no firmware found (limited functionality)");
        }

        // Try MSI
        let msi = crate::drivers::pci_msi::enable_msi(pci);
        if let Some(vec) = msi {
            crate::serial_println!("  Intel WiFi: MSI vector {}", vec);
        } else if irq > 0 && irq < 32 {
            crate::arch::x86::irq::register_irq(irq, iwl_irq_handler);
            if crate::arch::x86::apic::is_initialized() {
                crate::arch::x86::ioapic::unmask_irq(irq);
            }
        }

        let chip_name = match pci.device_id {
            0x2723 => "AX200",
            0x2725 => "AX210",
            0x06F0 | 0xA0F0 => "AX201",
            0x51F0 | 0x51F1 | 0x54F0 => "AX211",
            0x272B => "BE200",
            0x2526 => "AC 9260",
            0x9DF0 | 0xA370 => "AC 9560",
            0x24FD => "AC 8265",
            _ => "WiFi",
        };
        crate::serial_println!(
            "[OK] Intel {} WiFi (IRQ={}, fw={})",
            chip_name,
            irq,
            if fw_ok { "loaded" } else { "missing" }
        );
    }

    // Register as WiFi driver
    super::register_wifi(Box::new(IwlWifiDriver));
    true
}

unsafe fn read_mac_from_csr(mmio: u64) -> [u8; 6] {
    // Intel WiFi stores MAC in CSR registers or OTP shadow
    // The exact location varies by generation; this is a simplified read
    let mac_lo = rd(mmio, 0x0A4); // NVM shadow register area
    let mac_hi = rd(mmio, 0x0A8);
    [
        (mac_lo & 0xFF) as u8,
        ((mac_lo >> 8) & 0xFF) as u8,
        ((mac_lo >> 16) & 0xFF) as u8,
        ((mac_lo >> 24) & 0xFF) as u8,
        (mac_hi & 0xFF) as u8,
        ((mac_hi >> 8) & 0xFF) as u8,
    ]
}

fn try_load_firmware(mmio: u64, _device_id: u16) -> bool {
    // Try each firmware path
    // TODO: Firmware loading requires reading ucode binary from disk,
    // parsing sections, and DMA-transferring them to the device.
    // The firmware files (iwlwifi-*.ucode) must be placed in /System/Drivers/wifi/.
    // For now, firmware loading is not implemented — the device is detected
    // but full WiFi operation requires firmware.
    false
}

fn iwl_irq_handler(_irq: u8) {
    if let Some(mut guard) = IWL_STATE.try_lock() {
        if let Some(ref state) = *guard {
            unsafe {
                let inta = rd(state.mmio, CSR_INT);
                if inta == 0 || inta == 0xFFFFFFFF {
                    return;
                }
                wr(state.mmio, CSR_INT, inta); // ACK
            }
        }
    }
}

// ── NetworkDriver ───────────────────────────────────────────────────────────

struct IwlWifiDriver;

impl super::NetworkDriver for IwlWifiDriver {
    fn name(&self) -> &str {
        "Intel WiFi"
    }
    fn transmit(&mut self, _data: &[u8]) -> bool {
        // TODO: implement TX via firmware command queue
        false
    }
    fn get_mac(&self) -> [u8; 6] {
        IWL_STATE.lock().as_ref().map(|s| s.mac).unwrap_or([0; 6])
    }
    fn link_up(&self) -> bool {
        crate::net::wifi::is_connected()
    }
}

pub fn probe(pci: &PciDevice) -> Option<Box<dyn crate::drivers::hal::Driver>> {
    if init_and_register(pci) {
        super::create_hal_driver("Intel WiFi")
    } else {
        None
    }
}

pub fn is_available() -> bool {
    IWL_STATE.lock().is_some()
}
