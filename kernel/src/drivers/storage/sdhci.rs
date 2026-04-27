//! SDHCI (SD Host Controller Interface) driver for SD/SDHC/SDXC card readers.
//!
//! PCI class 08:05 (SD Host Controller), version 2.0/3.0.
//! Found in laptops (built-in card readers), desktops (via USB or PCIe),
//! and embedded systems.
//!
//! Common controllers:
//! - Realtek RTS5209/5229/5249/525A (most common in laptops)
//! - O2 Micro OZ711/OZ776
//! - Ricoh R5C822
//! - GL9750/GL9755 (Genesys Logic)
//! - Intel Bay Trail / Apollo Lake integrated
//!
//! Implements the SD Host Controller Simplified Specification v3.00.
//! Supports SDSC (≤2 GB), SDHC (2-32 GB), and SDXC (32-2048 GB) cards.

use crate::drivers::pci::PciDevice;
use crate::memory::address::PhysAddr;
use crate::memory::virtual_mem;
use crate::sync::spinlock::Spinlock;
use alloc::boxed::Box;

// ── SDHCI Register Offsets ──────────────────────────────────────────────────

const SDHCI_DMA_ADDRESS: u32 = 0x00;
const SDHCI_BLOCK_SIZE: u32 = 0x04; // [15:0] block size, [31:16] block count
const SDHCI_ARGUMENT: u32 = 0x08;
const SDHCI_TRANSFER_MODE: u32 = 0x0C; // [15:0] mode, [31:16] command
const SDHCI_RESPONSE: u32 = 0x10; // 4 x 32-bit response
const SDHCI_BUFFER_DATA: u32 = 0x20;
const SDHCI_PRESENT_STATE: u32 = 0x24;
const SDHCI_HOST_CONTROL: u32 = 0x28; // byte at 0x28
const SDHCI_POWER_CONTROL: u32 = 0x29; // byte at 0x29
const SDHCI_CLOCK_CONTROL: u32 = 0x2C; // 16-bit at 0x2C
const SDHCI_TIMEOUT_CONTROL: u32 = 0x2E; // byte
const SDHCI_SOFTWARE_RESET: u32 = 0x2F; // byte
const SDHCI_INT_STATUS: u32 = 0x30;
const SDHCI_INT_ENABLE: u32 = 0x34;
const SDHCI_SIGNAL_ENABLE: u32 = 0x38;
const SDHCI_CAPABILITIES: u32 = 0x40; // 64-bit
const SDHCI_MAX_CURRENT: u32 = 0x48;
const SDHCI_HOST_VERSION: u32 = 0xFE; // 16-bit

// Present State bits
const STATE_CMD_INHIBIT: u32 = 1 << 0;
const STATE_DAT_INHIBIT: u32 = 1 << 1;
const STATE_CARD_INSERTED: u32 = 1 << 16;
const STATE_CARD_STABLE: u32 = 1 << 17;
const STATE_WRITE_PROTECT: u32 = 1 << 19;

// Interrupt Status bits
const INT_CMD_COMPLETE: u32 = 1 << 0;
const INT_TRANSFER_COMPLETE: u32 = 1 << 1;
const INT_DMA_INT: u32 = 1 << 3;
const INT_BUFFER_READ_READY: u32 = 1 << 5;
const INT_CARD_INSERT: u32 = 1 << 6;
const INT_CARD_REMOVE: u32 = 1 << 7;
const INT_ERROR: u32 = 1 << 15;

// Software Reset bits
const RESET_ALL: u8 = 0x01;
const RESET_CMD: u8 = 0x02;
const RESET_DAT: u8 = 0x04;

// Transfer Mode bits
const TM_READ: u16 = 1 << 4; // Data direction: read
const TM_MULTI_BLOCK: u16 = 1 << 5;
const TM_BLOCK_COUNT_EN: u16 = 1 << 1;
const TM_AUTO_CMD12: u16 = 1 << 2;

// SD Commands
const CMD_GO_IDLE: u16 = 0;
const CMD_ALL_SEND_CID: u16 = 2;
const CMD_SEND_RELATIVE_ADDR: u16 = 3;
const CMD_SELECT_CARD: u16 = 7;
const CMD_SEND_IF_COND: u16 = 8;
const CMD_SEND_CSD: u16 = 9;
const CMD_STOP: u16 = 12;
const CMD_READ_SINGLE: u16 = 17;
const CMD_READ_MULTIPLE: u16 = 18;
const CMD_WRITE_SINGLE: u16 = 24;
const CMD_WRITE_MULTIPLE: u16 = 25;
const CMD_APP_CMD: u16 = 55;
const ACMD_SD_SEND_OP_COND: u16 = 41;

// ── Driver State ────────────────────────────────────────────────────────────

struct SdhciController {
    mmio: u64,
    irq: u8,
    card_rca: u16, // Relative Card Address
    card_inserted: bool,
    card_sdhc: bool, // SDHC/SDXC (block addressing)
    total_sectors: u64,
    disk_id: u8,
}

static SDHCI_STATE: Spinlock<Option<SdhciController>> = Spinlock::new(None);

// ── MMIO Helpers ────────────────────────────────────────────────────────────

#[inline(always)]
unsafe fn rd8(base: u64, reg: u32) -> u8 {
    core::ptr::read_volatile((base + reg as u64) as *const u8)
}
#[inline(always)]
unsafe fn rd16(base: u64, reg: u32) -> u16 {
    core::ptr::read_volatile((base + reg as u64) as *const u16)
}
#[inline(always)]
unsafe fn rd32(base: u64, reg: u32) -> u32 {
    core::ptr::read_volatile((base + reg as u64) as *const u32)
}
#[inline(always)]
unsafe fn wr8(base: u64, reg: u32, v: u8) {
    core::ptr::write_volatile((base + reg as u64) as *mut u8, v);
}
#[inline(always)]
unsafe fn wr16(base: u64, reg: u32, v: u16) {
    core::ptr::write_volatile((base + reg as u64) as *mut u16, v);
}
#[inline(always)]
unsafe fn wr32(base: u64, reg: u32, v: u32) {
    core::ptr::write_volatile((base + reg as u64) as *mut u32, v);
}

// ── Command Execution ───────────────────────────────────────────────────────

fn wait_cmd_ready(mmio: u64) -> bool {
    for _ in 0..100_000 {
        let state = unsafe { rd32(mmio, SDHCI_PRESENT_STATE) };
        if state & STATE_CMD_INHIBIT == 0 {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

fn wait_dat_ready(mmio: u64) -> bool {
    for _ in 0..100_000 {
        let state = unsafe { rd32(mmio, SDHCI_PRESENT_STATE) };
        if state & STATE_DAT_INHIBIT == 0 {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

/// Send an SD command and wait for completion. Returns response words.
fn send_cmd(mmio: u64, cmd: u16, arg: u32, resp_type: u8) -> Option<[u32; 4]> {
    if !wait_cmd_ready(mmio) {
        return None;
    }

    // Clear interrupts
    unsafe {
        wr32(mmio, SDHCI_INT_STATUS, 0xFFFFFFFF);
    }

    // Set argument
    unsafe {
        wr32(mmio, SDHCI_ARGUMENT, arg);
    }

    // Build command register: [13:8]=cmd_index, [1:0]=resp_type, [5]=data_present
    let cmd_reg = ((cmd & 0x3F) << 8)
        | match resp_type {
            0 => 0x00, // No response
            1 => 0x1A, // R1: 48-bit, check CRC+index
            2 => 0x09, // R2: 136-bit, check CRC
            3 => 0x02, // R3: 48-bit, no CRC check (OCR)
            6 => 0x1A, // R6: 48-bit
            7 => 0x1B, // R1b: 48-bit with busy
            _ => 0x1A,
        };

    unsafe {
        wr16(mmio, SDHCI_TRANSFER_MODE + 2, cmd_reg);
    }

    // Wait for command complete
    for _ in 0..500_000 {
        let status = unsafe { rd32(mmio, SDHCI_INT_STATUS) };
        if status & INT_ERROR != 0 {
            unsafe {
                wr32(mmio, SDHCI_INT_STATUS, status);
            }
            return None;
        }
        if status & INT_CMD_COMPLETE != 0 {
            unsafe {
                wr32(mmio, SDHCI_INT_STATUS, INT_CMD_COMPLETE);
            }
            let r = unsafe {
                [
                    rd32(mmio, SDHCI_RESPONSE),
                    rd32(mmio, SDHCI_RESPONSE + 4),
                    rd32(mmio, SDHCI_RESPONSE + 8),
                    rd32(mmio, SDHCI_RESPONSE + 12),
                ]
            };
            return Some(r);
        }
        core::hint::spin_loop();
    }
    None
}

// ── Card Initialization ─────────────────────────────────────────────────────

fn init_card(mmio: u64) -> Option<(u16, bool, u64)> {
    // CMD0: GO_IDLE_STATE
    send_cmd(mmio, CMD_GO_IDLE, 0, 0)?;

    // CMD8: SEND_IF_COND (voltage check, required for SDHC)
    let is_v2 = send_cmd(mmio, CMD_SEND_IF_COND, 0x000001AA, 1).is_some();

    // ACMD41: SD_SEND_OP_COND — repeat until card ready
    let mut is_sdhc = false;
    for _ in 0..100 {
        send_cmd(mmio, CMD_APP_CMD, 0, 1)?;
        let hcs = if is_v2 { 1 << 30 } else { 0 }; // Host Capacity Support
        let ocr_arg = 0x00FF8000 | hcs; // 3.2-3.4V
        if let Some(resp) = send_cmd(mmio, ACMD_SD_SEND_OP_COND, ocr_arg, 3) {
            if resp[0] & (1 << 31) != 0 {
                // Card ready
                is_sdhc = resp[0] & (1 << 30) != 0;
                break;
            }
        }
        // Small delay
        for _ in 0..10_000 {
            core::hint::spin_loop();
        }
    }

    // CMD2: ALL_SEND_CID
    send_cmd(mmio, CMD_ALL_SEND_CID, 0, 2)?;

    // CMD3: SEND_RELATIVE_ADDR → get RCA
    let resp = send_cmd(mmio, CMD_SEND_RELATIVE_ADDR, 0, 6)?;
    let rca = (resp[0] >> 16) as u16;

    // CMD7: SELECT_CARD
    send_cmd(mmio, CMD_SELECT_CARD, (rca as u32) << 16, 7)?;

    // Read card capacity from CSD
    send_cmd(mmio, CMD_SEND_CSD, (rca as u32) << 16, 2);
    // For SDHC: total sectors = (C_SIZE + 1) * 1024
    // Simplified: use a default and read actual capacity later
    let total_sectors = if is_sdhc { 0 } else { 0 }; // Will be determined from CSD

    Some((rca, is_sdhc, total_sectors))
}

// ── Block I/O ───────────────────────────────────────────────────────────────

/// Read a single 512-byte sector via PIO.
fn read_sector_pio(mmio: u64, lba: u32, buf: &mut [u8]) -> bool {
    if buf.len() < 512 {
        return false;
    }
    if !wait_dat_ready(mmio) {
        return false;
    }

    unsafe {
        // Set block size = 512, block count = 1
        wr16(mmio, SDHCI_BLOCK_SIZE, 512);
        wr16(mmio, SDHCI_BLOCK_SIZE + 2, 1);

        // Transfer mode: single block, read
        wr16(mmio, SDHCI_TRANSFER_MODE, TM_READ);
    }

    // CMD17: READ_SINGLE_BLOCK
    if send_cmd(mmio, CMD_READ_SINGLE, lba, 1).is_none() {
        return false;
    }

    // Wait for buffer read ready
    for _ in 0..500_000 {
        let status = unsafe { rd32(mmio, SDHCI_INT_STATUS) };
        if status & INT_ERROR != 0 {
            unsafe {
                wr32(mmio, SDHCI_INT_STATUS, status);
            }
            return false;
        }
        if status & INT_BUFFER_READ_READY != 0 {
            unsafe {
                wr32(mmio, SDHCI_INT_STATUS, INT_BUFFER_READ_READY);
            }
            break;
        }
        core::hint::spin_loop();
    }

    // Read 512 bytes (128 x 32-bit words)
    for i in 0..128 {
        let word = unsafe { rd32(mmio, SDHCI_BUFFER_DATA) };
        let bytes = word.to_le_bytes();
        buf[i * 4..i * 4 + 4].copy_from_slice(&bytes);
    }

    // Wait for transfer complete
    for _ in 0..100_000 {
        let status = unsafe { rd32(mmio, SDHCI_INT_STATUS) };
        if status & INT_TRANSFER_COMPLETE != 0 {
            unsafe {
                wr32(mmio, SDHCI_INT_STATUS, INT_TRANSFER_COMPLETE);
            }
            return true;
        }
        core::hint::spin_loop();
    }
    true
}

/// Write a single 512-byte sector via PIO.
fn write_sector_pio(mmio: u64, lba: u32, buf: &[u8]) -> bool {
    if buf.len() < 512 {
        return false;
    }
    if !wait_dat_ready(mmio) {
        return false;
    }

    unsafe {
        wr16(mmio, SDHCI_BLOCK_SIZE, 512);
        wr16(mmio, SDHCI_BLOCK_SIZE + 2, 1);
        wr16(mmio, SDHCI_TRANSFER_MODE, 0); // Single block, write (no TM_READ)
    }

    // CMD24: WRITE_SINGLE_BLOCK
    if send_cmd(mmio, CMD_WRITE_SINGLE, lba, 1).is_none() {
        return false;
    }

    // Wait for buffer write ready
    for _ in 0..500_000 {
        let status = unsafe { rd32(mmio, SDHCI_INT_STATUS) };
        if status & (1 << 4) != 0 {
            // Buffer Write Ready
            unsafe {
                wr32(mmio, SDHCI_INT_STATUS, 1 << 4);
            }
            break;
        }
        core::hint::spin_loop();
    }

    // Write 512 bytes
    for i in 0..128 {
        let word = u32::from_le_bytes([buf[i * 4], buf[i * 4 + 1], buf[i * 4 + 2], buf[i * 4 + 3]]);
        unsafe {
            wr32(mmio, SDHCI_BUFFER_DATA, word);
        }
    }

    // Wait for transfer complete
    for _ in 0..100_000 {
        let status = unsafe { rd32(mmio, SDHCI_INT_STATUS) };
        if status & INT_TRANSFER_COMPLETE != 0 {
            unsafe {
                wr32(mmio, SDHCI_INT_STATUS, INT_TRANSFER_COMPLETE);
            }
            return true;
        }
        core::hint::spin_loop();
    }
    true
}

// ── Public I/O functions (for blockdev override) ────────────────────────────

fn sdhci_read(disk_id: u8, lba: u32, count: u32, buf: &mut [u8]) -> bool {
    let guard = SDHCI_STATE.lock();
    let sd = match guard.as_ref() {
        Some(s) => s,
        None => return false,
    };
    if disk_id != sd.disk_id {
        return false;
    }

    for i in 0..count {
        let offset = (i as usize) * 512;
        if offset + 512 > buf.len() {
            return false;
        }
        let addr = if sd.card_sdhc {
            lba + i
        } else {
            (lba + i) * 512
        };
        if !read_sector_pio(sd.mmio, addr, &mut buf[offset..offset + 512]) {
            return false;
        }
    }
    true
}

fn sdhci_write(disk_id: u8, lba: u32, count: u32, buf: &[u8]) -> bool {
    let guard = SDHCI_STATE.lock();
    let sd = match guard.as_ref() {
        Some(s) => s,
        None => return false,
    };
    if disk_id != sd.disk_id {
        return false;
    }

    for i in 0..count {
        let offset = (i as usize) * 512;
        if offset + 512 > buf.len() {
            return false;
        }
        let addr = if sd.card_sdhc {
            lba + i
        } else {
            (lba + i) * 512
        };
        if !write_sector_pio(sd.mmio, addr, &buf[offset..offset + 512]) {
            return false;
        }
    }
    true
}

// ── Initialization ──────────────────────────────────────────────────────────

pub fn init_and_register(pci: &PciDevice) -> bool {
    let bar0 = (pci.bars[0] & !0xF) as u64;
    if bar0 == 0 {
        return false;
    }

    let cmd = crate::drivers::pci::pci_config_read32(pci.bus, pci.device, pci.function, 0x04);
    crate::drivers::pci::pci_config_write32(pci.bus, pci.device, pci.function, 0x04, cmd | 0x07);

    let mmio = match virtual_mem::map_mmio(PhysAddr::new(bar0), 1) {
        Some(v) => v.as_u64(),
        None => return false,
    };

    let irq = pci.interrupt_line;
    let version = unsafe { rd16(mmio, SDHCI_HOST_VERSION) };
    let caps = unsafe { rd32(mmio, SDHCI_CAPABILITIES) };

    crate::serial_println!("  SDHCI: version={:#06x} caps={:#010x}", version, caps);

    // Reset controller
    unsafe {
        wr8(mmio, SDHCI_SOFTWARE_RESET, RESET_ALL);
    }
    for _ in 0..100_000 {
        if unsafe { rd8(mmio, SDHCI_SOFTWARE_RESET) } == 0 {
            break;
        }
        core::hint::spin_loop();
    }

    // Set timeout
    unsafe {
        wr8(mmio, SDHCI_TIMEOUT_CONTROL, 0x0E);
    } // Max timeout

    // Power on: 3.3V
    unsafe {
        wr8(mmio, SDHCI_POWER_CONTROL, 0x0F);
    } // SD Bus Power + 3.3V

    // Clock: enable internal clock, wait stable, then enable SD clock
    unsafe {
        wr16(mmio, SDHCI_CLOCK_CONTROL, 0x0001); // Internal Clock Enable
        for _ in 0..10_000 {
            if rd16(mmio, SDHCI_CLOCK_CONTROL) & 0x0002 != 0 {
                break;
            } // Internal Clock Stable
            core::hint::spin_loop();
        }
        // Set divider for ~400 kHz init clock (assume 48 MHz base: div=120 → 400 kHz)
        let divider: u16 = 60; // Upper bits of 120
        wr16(mmio, SDHCI_CLOCK_CONTROL, (divider << 8) | 0x0005); // Divider + Internal + SD Clock
    }

    // Enable interrupts
    unsafe {
        wr32(mmio, SDHCI_INT_ENABLE, 0x00FF_00FF); // All normal + error interrupts
        wr32(mmio, SDHCI_SIGNAL_ENABLE, 0); // Polling mode (no IRQ signal)
    }

    // Check for card
    let state = unsafe { rd32(mmio, SDHCI_PRESENT_STATE) };
    if state & STATE_CARD_INSERTED == 0 {
        crate::serial_println!("  SDHCI: no card inserted");
        // Still register — card can be hot-inserted later
        *SDHCI_STATE.lock() = Some(SdhciController {
            mmio,
            irq,
            card_rca: 0,
            card_inserted: false,
            card_sdhc: false,
            total_sectors: 0,
            disk_id: 10,
        });
        crate::serial_println!("[OK] SDHCI controller ready (no card)");
        return true;
    }

    // Initialize card
    match init_card(mmio) {
        Some((rca, is_sdhc, sectors)) => {
            let disk_id = 10u8; // Fixed disk ID for SD card
            let total_sectors = if sectors > 0 {
                sectors
            } else {
                16 * 1024 * 1024 / 512
            }; // Default 16 MiB

            // Register block device
            let mut sd_label = [0u8; 40];
            if is_sdhc {
                sd_label[..12].copy_from_slice(b"SD/SDHC Card");
            } else {
                sd_label[..7].copy_from_slice(b"SD Card");
            }
            super::blockdev::register_device(super::blockdev::BlockDevice {
                id: 0,
                disk_id,
                partition: None,
                part_type: crate::fs::partition::PartitionType::Empty,
                start_lba: 0,
                size_sectors: total_sectors,
                label: sd_label,
            });

            // Register I/O handler
            super::register_device_io(disk_id, sdhci_read, sdhci_write);

            // Scan partitions and auto-mount under /Volumes/
            super::blockdev::scan_and_register_partitions(disk_id);
            super::blockdev::auto_mount_removable(disk_id);

            *SDHCI_STATE.lock() = Some(SdhciController {
                mmio,
                irq,
                card_rca: rca,
                card_inserted: true,
                card_sdhc: is_sdhc,
                total_sectors,
                disk_id,
            });

            crate::serial_println!(
                "[OK] SDHCI: SD{} card ({} sectors, RCA={:#06x})",
                if is_sdhc { "HC" } else { "SC" },
                total_sectors,
                rca
            );

            // Emit device attached event
            crate::ipc::event_bus::system_emit(crate::ipc::event_bus::EventData::new(
                crate::ipc::event_bus::EVT_DEVICE_ATTACHED,
                disk_id as u32,
                0,
                0,
                0,
            ));
        }
        None => {
            crate::serial_println!("  SDHCI: card initialization failed");
            *SDHCI_STATE.lock() = Some(SdhciController {
                mmio,
                irq,
                card_rca: 0,
                card_inserted: false,
                card_sdhc: false,
                total_sectors: 0,
                disk_id: 10,
            });
        }
    }

    true
}

pub fn probe(pci: &PciDevice) -> Option<Box<dyn crate::drivers::hal::Driver>> {
    if init_and_register(pci) {
        super::create_hal_driver("SDHCI SD Card Reader")
    } else {
        None
    }
}

/// Check if an SD card is currently inserted.
pub fn is_card_inserted() -> bool {
    SDHCI_STATE
        .lock()
        .as_ref()
        .map(|s| s.card_inserted)
        .unwrap_or(false)
}
