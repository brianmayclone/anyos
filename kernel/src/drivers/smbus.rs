//! SMBus/I²C Host Controller driver.
//!
//! Supports Intel ICH/PCH SMBus (PCI class 0x0C, subclass 0x05) and
//! AMD FCH SMBus (same class/subclass). Provides byte, word, and block
//! data read/write operations using the host controller's I/O registers.
//!
//! Intel ICH SMBus register layout (I/O base from BAR4 or config offset 0x20):
//!   base+0x00 = SMBHSTSTS  (status)
//!   base+0x02 = SMBHSTCNT  (control)
//!   base+0x03 = SMBHSTCMD  (command byte)
//!   base+0x04 = SMBHSTADD  (slave address, bit 0 = read/write)
//!   base+0x05 = SMBHSTDAT0 (data byte 0)
//!   base+0x06 = SMBHSTDAT1 (data byte 1)
//!
//! AMD FCH: identical register layout; base from BAR or config offset 0x90.

use crate::sync::spinlock::Spinlock;
use crate::drivers::pci::PciDevice;
use crate::arch::x86::port::{inb, outb};

// ── Register Offsets ─────────────────────────────────────────────────────────

const SMBHSTSTS:  u16 = 0x00;  // Host Status
const SMBHSTCNT:  u16 = 0x02;  // Host Control
const SMBHSTCMD:  u16 = 0x03;  // Host Command
const SMBHSTADD:  u16 = 0x04;  // Host Address (slave addr | r/w bit)
const SMBHSTDAT0: u16 = 0x05;  // Data byte 0
const SMBHSTDAT1: u16 = 0x06;  // Data byte 1 (word reads / block)
const SMBBLKDAT:  u16 = 0x07;  // Block data port

// ── Status Bits (SMBHSTSTS) ──────────────────────────────────────────────────

const STS_INUSE:    u8 = 0x40; // Bus In Use (another master active)
const STS_FAILED:   u8 = 0x10; // Failed
const STS_BUSERR:   u8 = 0x08; // Bus Error
const STS_DEVNACK:  u8 = 0x04; // Device NAK
const STS_INTR:     u8 = 0x02; // Interrupt (transaction complete)
const STS_HOSTBUSY: u8 = 0x01; // Host Busy

const STS_ERROR_BITS: u8 = STS_FAILED | STS_BUSERR | STS_DEVNACK;

// ── Control Bits (SMBHSTCNT) ─────────────────────────────────────────────────

const CNT_START:     u8 = 0x40; // Start transaction
const CNT_KILL:      u8 = 0x20; // Kill/abort transaction
const CNT_LAST_BYTE: u8 = 0x10; // Last byte (block reads)

// SMB_CMD protocol field (bits [4:2] of SMBHSTCNT)
const CMD_QUICK:    u8 = 0x00 << 2; // Quick Command
const CMD_BYTE:     u8 = 0x01 << 2; // Byte
const CMD_BYTE_DATA:u8 = 0x02 << 2; // Byte Data
const CMD_WORD_DATA:u8 = 0x03 << 2; // Word Data
const CMD_BLOCK:    u8 = 0x05 << 2; // Block Data

// Polling timeout in loop iterations (~25 ms at typical CPU speeds)
const POLL_TIMEOUT: u32 = 250_000;

// ── Public Types ─────────────────────────────────────────────────────────────

/// Identifies the vendor of the detected SMBus host controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmBusVendor {
    Intel,
    Amd,
}

/// SMBus host controller state.
pub struct SmBusController {
    /// I/O base port for the SMBus registers.
    pub base_port: u16,
    /// Controller vendor (affects which config register to read the base from).
    pub vendor: SmBusVendor,
}

/// Global singleton SMBus controller, initialised by `init()`.
static SMBUS: Spinlock<Option<SmBusController>> = Spinlock::new(None);

// ── Initialization ────────────────────────────────────────────────────────────

/// Find an SMBus controller in the PCI device list (class 0x0C / subclass 0x05)
/// and record its I/O base port.
///
/// Must be called after `drivers::pci::scan_all()`.
pub fn init(pci_devices: &[PciDevice]) {
    for dev in pci_devices {
        if dev.class_code != 0x0C || dev.subclass != 0x05 {
            continue;
        }

        // Determine vendor and I/O base address.
        // Intel: I/O BAR is BAR4 (offset 0x20 in config space); lower bits are BAR type flags.
        // AMD:   I/O BAR is BAR0 or from config offset 0x90.
        let (base_port, vendor) = if dev.vendor_id == 0x8086 {
            // Intel: BAR4 (bars[4]) holds the I/O base; mask off lower 4 bits (I/O BAR flags)
            let raw_bar = dev.bars[4];
            let port = (raw_bar & 0xFFFC) as u16;
            crate::serial_verbose_println!(
                "  SMBus: Intel ICH/PCH at {:02x}:{:02x}.{} I/O={:#06x}",
                dev.bus, dev.device, dev.function, port
            );
            (port, SmBusVendor::Intel)
        } else if dev.vendor_id == 0x1022 || dev.vendor_id == 0x1002 {
            // AMD/ATI: read from config offset 0x90 (or BAR0)
            let cfg90 = crate::drivers::pci::pci_config_read32(
                dev.bus, dev.device, dev.function, 0x90,
            );
            let port_cfg = (cfg90 & 0xFFFC) as u16;
            // If config 0x90 looks like a valid I/O port, use it; otherwise fall back to BAR0
            let port = if port_cfg > 0x100 { port_cfg } else {
                (dev.bars[0] & 0xFFFC) as u16
            };
            crate::serial_verbose_println!(
                "  SMBus: AMD FCH at {:02x}:{:02x}.{} I/O={:#06x}",
                dev.bus, dev.device, dev.function, port
            );
            (port, SmBusVendor::Amd)
        } else {
            // Unknown vendor: try BAR4 as a best effort (same as Intel layout)
            let port = (dev.bars[4] & 0xFFFC) as u16;
            crate::serial_verbose_println!(
                "  SMBus: Unknown vendor {:04x} at {:02x}:{:02x}.{} I/O={:#06x}",
                dev.vendor_id, dev.bus, dev.device, dev.function, port
            );
            (port, SmBusVendor::Intel)
        };

        if base_port == 0 {
            crate::serial_verbose_println!("  SMBus: I/O base is 0, skipping");
            continue;
        }

        // Clear any stale status bits before use.
        clear_status(base_port);

        let mut guard = SMBUS.lock();
        *guard = Some(SmBusController { base_port, vendor });
        crate::serial_verbose_println!("[OK] SMBus host controller initialized at {:#06x}", base_port);
        return;
    }

    crate::serial_verbose_println!("  SMBus: no host controller found");
}

// ── Public API ────────────────────────────────────────────────────────────────

/// SMBus Byte Data Read: sends `cmd` to slave at `addr` and reads one byte back.
/// Returns `None` if the bus is busy, the device NAK'd, or a timeout occurred.
pub fn read_byte(addr: u8, cmd: u8) -> Option<u8> {
    let guard = SMBUS.lock();
    let ctrl = guard.as_ref()?;
    let base = ctrl.base_port;

    if !wait_for_idle(base) { return None; }
    clear_status(base);

    // Set up transaction: slave address with read bit, command register, Byte Data protocol.
    unsafe {
        outb(base + SMBHSTADD, (addr << 1) | 0x01); // bit 0 = read
        outb(base + SMBHSTCMD, cmd);
        outb(base + SMBHSTCNT, CMD_BYTE_DATA | CNT_START);
    }

    // Poll until complete or error.
    if !poll_complete(base) { return None; }

    let data = unsafe { inb(base + SMBHSTDAT0) };
    Some(data)
}

/// SMBus Byte Data Write: sends `cmd` and `value` to slave at `addr`.
/// Returns `true` on success.
pub fn write_byte(addr: u8, cmd: u8, value: u8) -> bool {
    let guard = SMBUS.lock();
    let ctrl = match guard.as_ref() {
        Some(c) => c,
        None => return false,
    };
    let base = ctrl.base_port;

    if !wait_for_idle(base) { return false; }
    clear_status(base);

    unsafe {
        outb(base + SMBHSTADD, (addr << 1) | 0x00); // bit 0 = write
        outb(base + SMBHSTCMD, cmd);
        outb(base + SMBHSTDAT0, value);
        outb(base + SMBHSTCNT, CMD_BYTE_DATA | CNT_START);
    }

    poll_complete(base)
}

/// SMBus Word Data Read: reads two bytes from slave at `addr`, register `cmd`.
/// Returns the 16-bit value (low byte = DAT0, high byte = DAT1) or `None` on error.
pub fn read_word(addr: u8, cmd: u8) -> Option<u16> {
    let guard = SMBUS.lock();
    let ctrl = guard.as_ref()?;
    let base = ctrl.base_port;

    if !wait_for_idle(base) { return None; }
    clear_status(base);

    unsafe {
        outb(base + SMBHSTADD, (addr << 1) | 0x01); // read
        outb(base + SMBHSTCMD, cmd);
        outb(base + SMBHSTCNT, CMD_WORD_DATA | CNT_START);
    }

    if !poll_complete(base) { return None; }

    let lo = unsafe { inb(base + SMBHSTDAT0) } as u16;
    let hi = unsafe { inb(base + SMBHSTDAT1) } as u16;
    Some(lo | (hi << 8))
}

/// SMBus Block Read: reads up to 32 bytes from slave `addr`, register `cmd`.
/// The first byte the device returns is the block length (per SMBus spec).
/// Returns the number of data bytes written into `buf`, or `None` on error.
pub fn read_block(addr: u8, cmd: u8, buf: &mut [u8]) -> Option<usize> {
    let guard = SMBUS.lock();
    let ctrl = guard.as_ref()?;
    let base = ctrl.base_port;

    if !wait_for_idle(base) { return None; }
    clear_status(base);

    unsafe {
        outb(base + SMBHSTADD, (addr << 1) | 0x01); // read
        outb(base + SMBHSTCMD, cmd);
        // Enable block mode and start; some controllers need the block-count register cleared
        outb(base + SMBHSTDAT1, 0x00); // reset block count
        outb(base + SMBHSTCNT, CMD_BLOCK | CNT_START);
    }

    // The ICH block data port transfers bytes one at a time; after each byte the
    // host controller updates STS_INTR when the next byte is ready (cleared by reading).
    // We poll for the final INTR after the last byte.
    let block_len_raw = unsafe { inb(base + SMBHSTDAT0) } as usize;
    let block_len = block_len_raw.min(32).min(buf.len());

    for i in 0..block_len {
        // Wait for byte ready (STS_INTR asserted between each byte in block mode).
        let mut timeout = POLL_TIMEOUT;
        loop {
            let sts = unsafe { inb(base + SMBHSTSTS) };
            if sts & STS_ERROR_BITS != 0 {
                clear_status(base);
                return None;
            }
            if sts & STS_INTR != 0 {
                // Clear INTR to acknowledge the byte
                unsafe { outb(base + SMBHSTSTS, STS_INTR); }
                break;
            }
            if timeout == 0 {
                // Kill the transaction
                unsafe { outb(base + SMBHSTCNT, CNT_KILL); }
                return None;
            }
            timeout -= 1;
            core::hint::spin_loop();
        }
        buf[i] = unsafe { inb(base + SMBBLKDAT) };
    }

    Some(block_len)
}

/// SMBus Quick Command: sends a start condition and slave address with the R/W
/// bit cleared (write), then a stop. Used to detect device presence.
/// Returns `true` if the device ACK'd its address.
pub fn quick_write(addr: u8) -> bool {
    let guard = SMBUS.lock();
    let ctrl = match guard.as_ref() {
        Some(c) => c,
        None => return false,
    };
    let base = ctrl.base_port;

    if !wait_for_idle(base) { return false; }
    clear_status(base);

    unsafe {
        outb(base + SMBHSTADD, (addr << 1) | 0x00); // write direction for Quick
        outb(base + SMBHSTCNT, CMD_QUICK | CNT_START);
    }

    poll_complete(base)
}

// ── Private Helpers ───────────────────────────────────────────────────────────

/// Poll SMBHSTSTS until the host is not busy.
/// Returns `true` if idle within timeout, `false` if timeout expired.
pub(crate) fn wait_for_idle(base: u16) -> bool {
    let mut timeout = POLL_TIMEOUT;
    loop {
        let sts = unsafe { inb(base + SMBHSTSTS) };
        if sts & (STS_HOSTBUSY | STS_INUSE) == 0 {
            return true;
        }
        if timeout == 0 {
            crate::serial_verbose_println!("  SMBus: wait_for_idle timeout (sts={:#04x})", sts);
            return false;
        }
        timeout -= 1;
        core::hint::spin_loop();
    }
}

/// Clear all error and completion status bits in SMBHSTSTS.
pub(crate) fn clear_status(base: u16) {
    // Writing 1 to each status bit clears it (W1C — write-one-to-clear).
    unsafe { outb(base + SMBHSTSTS, STS_INUSE | STS_FAILED | STS_BUSERR | STS_DEVNACK | STS_INTR); }
}

/// Poll for transaction completion (STS_INTR) or an error condition.
/// Returns `true` on successful completion, `false` on error or timeout.
fn poll_complete(base: u16) -> bool {
    let mut timeout = POLL_TIMEOUT;
    loop {
        let sts = unsafe { inb(base + SMBHSTSTS) };

        if sts & STS_ERROR_BITS != 0 {
            // Transaction failed; clear error bits
            clear_status(base);
            return false;
        }
        if sts & STS_INTR != 0 {
            // Transaction complete; clear the interrupt bit
            unsafe { outb(base + SMBHSTSTS, STS_INTR); }
            return true;
        }
        if timeout == 0 {
            crate::serial_verbose_println!("  SMBus: poll_complete timeout (sts={:#04x})", sts);
            // Abort the transaction
            unsafe { outb(base + SMBHSTCNT, CNT_KILL); }
            clear_status(base);
            return false;
        }
        timeout -= 1;
        core::hint::spin_loop();
    }
}
