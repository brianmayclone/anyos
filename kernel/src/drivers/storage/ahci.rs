//! AHCI (SATA) driver for DMA-based disk I/O.
//!
//! Supports AHCI 1.0+ host controllers (PCI class 01:06, prog IF 01).
//! Uses DMA transfers via MMIO, replacing legacy ATA PIO when available.

use alloc::boxed::Box;
use crate::drivers::pci::{PciDevice, pci_config_read32, pci_config_write32};
use crate::memory::address::{PhysAddr, VirtAddr};
use crate::memory::{virtual_mem, physical};

// AHCI MMIO virtual base — after E1000 (0xD000_0000) and VMware SVGA FIFO (0xD002_0000)
const AHCI_MMIO_VIRT: u64 = 0xFFFF_FFFF_D006_0000;
const AHCI_MMIO_PAGES: usize = 8; // 32 KiB

// ── HBA Generic Registers ───────────────────────────
const REG_CAP: u64 = 0x00;
const REG_GHC: u64 = 0x04;
const REG_IS: u64 = 0x08;
const REG_PI: u64 = 0x0C;
const REG_VS: u64 = 0x10;

const GHC_AE: u32 = 1 << 31;

// ── Per-Port Registers (base = 0x100 + port * 0x80) ─
const PORT_CLB: u64 = 0x00;
const PORT_CLBU: u64 = 0x04;
const PORT_FB: u64 = 0x08;
const PORT_FBU: u64 = 0x0C;
const PORT_IS: u64 = 0x10;
const PORT_IE: u64 = 0x14;
const PORT_CMD: u64 = 0x18;
const PORT_TFD: u64 = 0x20;
const PORT_SIG: u64 = 0x24;
const PORT_SSTS: u64 = 0x28;
const PORT_SERR: u64 = 0x30;
const PORT_CI: u64 = 0x38;

const CMD_ST: u32 = 1 << 0;
const CMD_FRE: u32 = 1 << 4;
const CMD_FR: u32 = 1 << 14;
const CMD_CR: u32 = 1 << 15;

// ── ATA Commands ────────────────────────────────────
const ATA_CMD_READ_DMA_EXT: u8 = 0x25;
const ATA_CMD_WRITE_DMA_EXT: u8 = 0x35;
const ATA_CMD_FLUSH_EXT: u8 = 0xEA;
const ATA_CMD_IDENTIFY: u8 = 0xEC;

const FIS_TYPE_REG_H2D: u8 = 0x27;

const SATA_SIG_ATA: u32 = 0x00000101;
const SATA_SIG_ATAPI: u32 = 0xEB140101;
const ATA_CMD_PACKET: u8 = 0xA0;

// ── Bounce buffer: 512 KiB = 1024 sectors ────────────
// Enlarged from 128 KiB to amortize DMA setup overhead for large sequential I/O.
const BOUNCE_BUF_SECTORS: u32 = 1024;
const BOUNCE_BUF_SIZE: usize = BOUNCE_BUF_SECTORS as usize * 512;
const BOUNCE_BUF_FRAMES: usize = BOUNCE_BUF_SIZE / 4096; // 128

const MAX_PRDT: usize = 8;

// ── HBA Data Structures (all DMA-accessible) ────────

/// Command List Header (32 bytes, 32 slots per port).
#[repr(C)]
struct CmdHeader {
    flags: u16,
    prdtl: u16,
    prdbc: u32,
    ctba: u32,
    ctbau: u32,
    _reserved: [u32; 4],
}

/// Physical Region Descriptor Table Entry (16 bytes).
#[repr(C)]
struct PrdtEntry {
    dba: u32,
    dbau: u32,
    _reserved: u32,
    dbc: u32, // bit 31 = IOC, bits 21:0 = byte count minus 1
}

/// Command Table (128-byte header + PRDT entries).
#[repr(C)]
struct CmdTable {
    cfis: [u8; 64],
    acmd: [u8; 16],
    _reserved: [u8; 48],
    prdt: [PrdtEntry; MAX_PRDT],
}

/// Register Host-to-Device FIS (20 bytes, placed in cfis[]).
#[repr(C)]
struct FisRegH2D {
    fis_type: u8,
    flags: u8,      // bit 7 = C (command)
    command: u8,
    features_lo: u8,
    lba0: u8,
    lba1: u8,
    lba2: u8,
    device: u8,
    lba3: u8,
    lba4: u8,
    lba5: u8,
    features_hi: u8,
    count_lo: u8,
    count_hi: u8,
    _reserved: [u8; 6],
}

const GHC_IE: u32 = 1 << 1;

// ── Controller State ────────────────────────────────

/// DMA structures for a single AHCI port.
struct PortDma {
    port: u32,
    clb_phys: u64,
    fb_phys: u64,
    ctba_phys: u64,
    total_sectors: u64,
}

/// Maximum number of additional ATA disks (beyond the primary active_port).
const MAX_EXTRA_DISKS: usize = 7;

struct AhciController {
    mmio_base: u64,
    active_port: u32,
    clb_phys: u64,
    fb_phys: u64,
    ctba_phys: u64,
    bounce_phys: u64,
    bounce_virt: u64,   // = bounce_phys (identity-mapped)
    total_sectors: u64,
    irq: u8,
    // Additional ATA disks (ports beyond the primary)
    extra_disks: [Option<PortDma>; MAX_EXTRA_DISKS],
    extra_disk_count: usize,
    // ATAPI CD-ROM support (separate port with own DMA structures)
    atapi_port: Option<u32>,
    atapi_clb_phys: u64,
    atapi_fb_phys: u64,
    atapi_ctba_phys: u64,
}

static mut AHCI: Option<AhciController> = None;

/// TID of the thread currently waiting for AHCI I/O completion.
/// 0 means no thread is waiting.
static AHCI_WAITER: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
/// Set to true by the IRQ handler when the command completes.
static AHCI_IRQ_FIRED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

// ── MMIO Helpers ────────────────────────────────────

#[inline(always)]
unsafe fn mmio_read32(base: u64, offset: u64) -> u32 {
    core::ptr::read_volatile((base + offset) as *const u32)
}

#[inline(always)]
unsafe fn mmio_write32(base: u64, offset: u64, val: u32) {
    core::ptr::write_volatile((base + offset) as *mut u32, val);
}

#[inline(always)]
fn port_base(port: u32) -> u64 {
    0x100 + (port as u64) * 0x80
}

#[inline(always)]
unsafe fn port_read(base: u64, port: u32, reg: u64) -> u32 {
    mmio_read32(base, port_base(port) + reg)
}

#[inline(always)]
unsafe fn port_write(base: u64, port: u32, reg: u64, val: u32) {
    mmio_write32(base, port_base(port) + reg, val);
}

// ── Port Start / Stop ───────────────────────────────

unsafe fn stop_port(base: u64, port: u32) {
    // Clear ST
    let mut cmd = port_read(base, port, PORT_CMD);
    cmd &= !CMD_ST;
    port_write(base, port, PORT_CMD, cmd);

    // Wait for CR (Command List Running) to clear
    for _ in 0..1_000_000 {
        if port_read(base, port, PORT_CMD) & CMD_CR == 0 {
            break;
        }
        core::hint::spin_loop();
    }

    // Clear FRE
    cmd = port_read(base, port, PORT_CMD);
    cmd &= !CMD_FRE;
    port_write(base, port, PORT_CMD, cmd);

    // Wait for FR (FIS Receive Running) to clear
    for _ in 0..1_000_000 {
        if port_read(base, port, PORT_CMD) & CMD_FR == 0 {
            break;
        }
        core::hint::spin_loop();
    }
}

unsafe fn start_port(base: u64, port: u32) {
    // Wait for CR to clear first
    for _ in 0..1_000_000 {
        if port_read(base, port, PORT_CMD) & CMD_CR == 0 {
            break;
        }
        core::hint::spin_loop();
    }

    let mut cmd = port_read(base, port, PORT_CMD);
    cmd |= CMD_FRE;
    port_write(base, port, PORT_CMD, cmd);

    cmd = port_read(base, port, PORT_CMD);
    cmd |= CMD_ST;
    port_write(base, port, PORT_CMD, cmd);
}

// ── IRQ Handler ─────────────────────────────────────

/// Port Interrupt Status bit for PhyRdy Change (device connect/disconnect).
const PORT_IS_PRCS: u32 = 1 << 22;
/// Port Interrupt Status bit for device-to-host register FIS (command completion).
const PORT_IS_DHRS: u32 = 1 << 0;

/// Atomically tracks ports with pending hot-plug events.
/// Each bit corresponds to a port number (0-31). Checked by a deferred handler.
static HOTPLUG_PENDING: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

fn ahci_irq_handler(_irq: u8) {
    use core::sync::atomic::Ordering;

    let ahci = match unsafe { AHCI.as_ref() } {
        Some(a) => a,
        None => return,
    };

    unsafe {
        // Check HBA global interrupt status
        let hba_is = mmio_read32(ahci.mmio_base, REG_IS);
        if hba_is == 0 {
            return;
        }

        // ── Hot-plug detection on ALL implemented ports ─────────────
        // Check every port that triggered an interrupt for PhyRdy Change (PRCS).
        let pi = mmio_read32(ahci.mmio_base, REG_PI);
        for port in 0..32u32 {
            if pi & (1 << port) == 0 || hba_is & (1 << port) == 0 {
                continue;
            }
            let pis = port_read(ahci.mmio_base, port, PORT_IS);
            if pis & PORT_IS_PRCS != 0 {
                // Clear PhyRdy Change Status
                port_write(ahci.mmio_base, port, PORT_IS, PORT_IS_PRCS);
                // Clear SERR.DIAG.N (PhyRdy change bit in error register)
                let serr = port_read(ahci.mmio_base, port, PORT_SERR);
                port_write(ahci.mmio_base, port, PORT_SERR, serr);
                // Mark port for deferred hot-plug processing
                HOTPLUG_PENDING.fetch_or(1 << port, Ordering::Release);
                crate::serial_println!("  AHCI: hot-plug event on port {}", port);
            }
        }

        // Clear ATAPI port interrupt (polled, just ack it)
        if let Some(ap) = ahci.atapi_port {
            if hba_is & (1 << ap) != 0 {
                let pis = port_read(ahci.mmio_base, ap, PORT_IS);
                port_write(ahci.mmio_base, ap, PORT_IS, pis);
            }
        }

        // Clear extra disk port interrupts (polled I/O, just ack)
        for ed in ahci.extra_disks.iter().flatten() {
            if hba_is & (1 << ed.port) != 0 {
                let pis = port_read(ahci.mmio_base, ed.port, PORT_IS);
                port_write(ahci.mmio_base, ed.port, PORT_IS, pis);
            }
        }

        if hba_is & (1 << ahci.active_port) == 0 {
            // Clear global IS and return — not our ATA port
            mmio_write32(ahci.mmio_base, REG_IS, hba_is);
            return;
        }

        // Clear ATA port interrupt status
        let port_is = port_read(ahci.mmio_base, ahci.active_port, PORT_IS);
        port_write(ahci.mmio_base, ahci.active_port, PORT_IS, port_is);

        // Clear HBA global interrupt status
        mmio_write32(ahci.mmio_base, REG_IS, hba_is);

        // Only signal completion when the command is actually done (CI bit 0 clear)
        let ci = port_read(ahci.mmio_base, ahci.active_port, PORT_CI);
        if ci & 1 != 0 {
            return; // Command still in progress — mid-transfer interrupt, ignore
        }
    }

    // Command complete — signal and wake
    AHCI_IRQ_FIRED.store(true, Ordering::Release);

    let tid = AHCI_WAITER.load(Ordering::Acquire);
    if tid != 0 {
        // Non-blocking: try_wake avoids spinning on SCHEDULER lock in IRQ context.
        // If contended, deferred_wake queues the TID for the next timer tick.
        if !crate::task::scheduler::try_wake_thread(tid) {
            crate::task::scheduler::deferred_wake(tid);
        }
    }
}

/// MSI handler — identical logic to ahci_irq_handler but called via MSI vector.
/// MSI doesn't require EOI to IOAPIC (LAPIC EOI is handled by the IDT stub).
fn ahci_msi_handler(_vector: u8) {
    ahci_irq_handler(0);
}

/// Process deferred hot-plug events. Called from a non-IRQ context
/// (e.g. periodic task or on next I/O operation).
/// Detects newly connected or disconnected SATA devices and updates the
/// block device registry.
pub fn process_hotplug() {
    use core::sync::atomic::Ordering;

    let pending = HOTPLUG_PENDING.swap(0, Ordering::AcqRel);
    if pending == 0 { return; }

    let ahci = match unsafe { AHCI.as_ref() } {
        Some(a) => a,
        None => return,
    };

    for port in 0..32u32 {
        if pending & (1 << port) == 0 { continue; }

        let ssts = unsafe { port_read(ahci.mmio_base, port, PORT_SSTS) };
        let det = ssts & 0x0F;

        if det == 3 {
            // Device connected — check signature
            let sig = unsafe { port_read(ahci.mmio_base, port, PORT_SIG) };
            if sig == SATA_SIG_ATA {
                crate::serial_println!("  AHCI hot-plug: SATA disk connected on port {}", port);
                // Identify the disk to get capacity
                let sectors = unsafe { identify_disk_on_port(ahci, port) };
                if sectors > 0 {
                    let disk_id = (port + 1) as u8; // disk_id 0 = primary, 1+ = hot-plugged
                    super::blockdev::register_device(super::blockdev::BlockDevice {
                        id: 0, // auto-assigned
                        disk_id,
                        partition: None,
                        start_lba: 0,
                        size_sectors: sectors,
                    });
                    super::blockdev::scan_and_register_partitions(disk_id);
                    super::blockdev::auto_mount_removable(disk_id);
                    crate::serial_println!("  AHCI hot-plug: disk registered (port={}, {} sectors)",
                        port, sectors);
                }
            } else if sig == SATA_SIG_ATAPI {
                crate::serial_println!("  AHCI hot-plug: ATAPI device connected on port {}", port);
            }
        } else {
            // Device disconnected
            crate::serial_println!("  AHCI hot-plug: device disconnected from port {} (det={})", port, det);
            // Note: full hot-unplug (removing blockdev entries, unmounting) is left
            // for future work — we only log the event for now.
        }
    }
}

/// Issue IDENTIFY DEVICE on a specific port to get disk capacity.
/// Returns total sectors (0 on failure).
unsafe fn identify_disk_on_port(ahci: &AhciController, port: u32) -> u64 {
    // For hot-plugged disks we use the bounce buffer (serialized by IO_LOCK)
    let identify_buf = ahci.bounce_virt as *mut u8;
    core::ptr::write_bytes(identify_buf, 0, 512);

    let ok = issue_command(
        ahci,
        ATA_CMD_IDENTIFY,
        0, 0,
        ahci.bounce_phys,
        512,
        false,
    );

    if !ok { return 0; }

    // LBA48 capacity at words 100-103 (offset 200-207)
    let word100 = core::ptr::read_unaligned((ahci.bounce_virt + 200) as *const u64);
    if word100 > 0 {
        return word100;
    }
    // LBA28 capacity at words 60-61 (offset 120-123)
    let word60 = core::ptr::read_unaligned((ahci.bounce_virt + 120) as *const u32);
    word60 as u64
}

// ── Command Issue (IRQ-driven, slot 0 only) ─────────

/// Timeout for AHCI commands: ~5 seconds expressed in timer ticks.
/// Computed from the HAL timer frequency at call time.
const AHCI_TIMEOUT_MS: u64 = 5000;

/// Number of retries after a timeout before giving up.
const AHCI_MAX_RETRIES: u32 = 1;

/// Set up the command table and FIS for a single AHCI command, then issue it.
/// Returns true if the command completed successfully. Does NOT retry.
unsafe fn issue_command_once(
    ahci: &AhciController,
    command: u8,
    lba: u64,
    count: u16,
    dma_phys: u64,
    dma_size: u32,
    write: bool,
) -> bool {
    // Set up command header (slot 0)
    let cmd_header = ahci.clb_phys as *mut CmdHeader;
    let cfl: u16 = 5; // 5 DWORDs for Register H2D FIS
    let w_bit: u16 = if write { 1 << 6 } else { 0 };
    (*cmd_header).flags = cfl | w_bit;
    (*cmd_header).prdtl = if dma_size > 0 { 1 } else { 0 };
    (*cmd_header).prdbc = 0;
    // ctba/ctbau already set during init

    // Set up command table
    let cmd_table = ahci.ctba_phys as *mut CmdTable;

    // Zero CFIS + ACMD
    core::ptr::write_bytes((*cmd_table).cfis.as_mut_ptr(), 0, 64);
    core::ptr::write_bytes((*cmd_table).acmd.as_mut_ptr(), 0, 16);

    // Fill Register H2D FIS
    let fis = (*cmd_table).cfis.as_mut_ptr() as *mut FisRegH2D;
    (*fis).fis_type = FIS_TYPE_REG_H2D;
    (*fis).flags = 0x80; // C bit = this is a command
    (*fis).command = command;
    (*fis).device = 0x40; // LBA mode
    (*fis).lba0 = (lba & 0xFF) as u8;
    (*fis).lba1 = ((lba >> 8) & 0xFF) as u8;
    (*fis).lba2 = ((lba >> 16) & 0xFF) as u8;
    (*fis).lba3 = ((lba >> 24) & 0xFF) as u8;
    (*fis).lba4 = ((lba >> 32) & 0xFF) as u8;
    (*fis).lba5 = ((lba >> 40) & 0xFF) as u8;
    (*fis).count_lo = (count & 0xFF) as u8;
    (*fis).count_hi = ((count >> 8) & 0xFF) as u8;
    (*fis).features_lo = 0;
    (*fis).features_hi = 0;

    // Fill PRDT[0] if data transfer
    if dma_size > 0 {
        (*cmd_table).prdt[0].dba = dma_phys as u32;
        (*cmd_table).prdt[0].dbau = (dma_phys >> 32) as u32;
        (*cmd_table).prdt[0]._reserved = 0;
        (*cmd_table).prdt[0].dbc = (dma_size - 1) | (1 << 31); // IOC + byte count
    }

    // Clear port interrupt status
    port_write(ahci.mmio_base, ahci.active_port, PORT_IS, 0xFFFF_FFFF);

    // Reset IRQ completion flag
    AHCI_IRQ_FIRED.store(false, core::sync::atomic::Ordering::Release);

    // Issue command (slot 0)
    port_write(ahci.mmio_base, ahci.active_port, PORT_CI, 1);

    // Fast path: spin-wait for command completion.
    for _ in 0..50_000 {
        let ci = port_read(ahci.mmio_base, ahci.active_port, PORT_CI);
        if ci & 1 == 0 {
            let tfd = port_read(ahci.mmio_base, ahci.active_port, PORT_TFD);
            if tfd & 0x01 != 0 {
                crate::serial_verbose_println!("AHCI: command error, TFD={:#x}", tfd);
                return false;
            }
            return true;
        }
        core::hint::spin_loop();
    }

    // Slow path: block on IRQ with timeout.
    let tid = crate::task::scheduler::current_tid();
    if tid > 0 {
        let hz = crate::arch::hal::timer_frequency_hz() as u32;
        let timeout_ticks = (AHCI_TIMEOUT_MS as u32 * hz / 1000).max(1);
        let start = crate::arch::hal::timer_current_ticks();

        // Sleep in short intervals (50ms), checking for completion each time.
        // The IRQ handler will wake us early via try_wake_thread/deferred_wake.
        let sleep_interval = (hz / 20).max(1); // 50ms

        if ahci.irq > 0 {
            AHCI_WAITER.store(tid, core::sync::atomic::Ordering::Release);
        }

        loop {
            let now = crate::arch::hal::timer_current_ticks();
            let elapsed = now.wrapping_sub(start);
            if elapsed >= timeout_ticks {
                // Timeout reached
                AHCI_WAITER.store(0, core::sync::atomic::Ordering::Release);
                crate::serial_println!("AHCI: command timeout (cmd={:#x}, lba={}, slow path)", command, lba);
                return false;
            }

            let wake_at = now.wrapping_add(sleep_interval);
            crate::task::scheduler::sleep_until(wake_at);

            // Check if command completed (either via IRQ or spontaneously)
            let ci = port_read(ahci.mmio_base, ahci.active_port, PORT_CI);
            if ci & 1 == 0 {
                AHCI_WAITER.store(0, core::sync::atomic::Ordering::Release);
                let tfd = port_read(ahci.mmio_base, ahci.active_port, PORT_TFD);
                if tfd & 0x01 != 0 {
                    crate::serial_verbose_println!("AHCI: command error, TFD={:#x}", tfd);
                    return false;
                }
                return true;
            }

            // Check for task file error (device reported failure)
            let is = port_read(ahci.mmio_base, ahci.active_port, PORT_IS);
            if is & (1 << 30) != 0 {
                AHCI_WAITER.store(0, core::sync::atomic::Ordering::Release);
                crate::serial_verbose_println!("AHCI: task file error in slow path, IS={:#x}", is);
                return false;
            }
        }
    }

    // Fallback: extended poll with timeout (boot thread or no IRQ)
    poll_completion(ahci)
}

unsafe fn issue_command(
    ahci: &AhciController,
    command: u8,
    lba: u64,
    count: u16,
    dma_phys: u64,
    dma_size: u32,
    write: bool,
) -> bool {
    // First attempt
    if issue_command_once(ahci, command, lba, count, dma_phys, dma_size, write) {
        return true;
    }

    // On failure, retry once after resetting the port
    for retry in 0..AHCI_MAX_RETRIES {
        crate::serial_println!(
            "AHCI: retrying command (cmd={:#x}, lba={}, attempt {}/{})",
            command, lba, retry + 1, AHCI_MAX_RETRIES
        );

        // Reset the port to clear any stuck state
        stop_port(ahci.mmio_base, ahci.active_port);
        // Clear all errors
        port_write(ahci.mmio_base, ahci.active_port, PORT_SERR, 0xFFFF_FFFF);
        port_write(ahci.mmio_base, ahci.active_port, PORT_IS, 0xFFFF_FFFF);
        start_port(ahci.mmio_base, ahci.active_port);

        if issue_command_once(ahci, command, lba, count, dma_phys, dma_size, write) {
            return true;
        }
    }

    crate::serial_println!("AHCI: command failed after retries (cmd={:#x}, lba={})", command, lba);
    false
}

/// Issue a command on a specific port with given DMA structures (polled I/O).
/// Used for extra disks that don't use the primary port's DMA.
/// Includes a ~5 second timeout and one retry on failure.
unsafe fn issue_command_on_port(
    mmio_base: u64,
    port: u32,
    clb_phys: u64,
    ctba_phys: u64,
    dma_phys: u64,
    command: u8,
    lba: u64,
    count: u16,
    dma_size: u32,
    write: bool,
) -> bool {
    for attempt in 0..2u32 {
        if attempt > 0 {
            crate::serial_println!(
                "AHCI: retrying port {} command (cmd={:#x}, lba={}, attempt {})",
                port, command, lba, attempt
            );
            // Reset port to clear stuck state before retry
            stop_port(mmio_base, port);
            port_write(mmio_base, port, PORT_SERR, 0xFFFF_FFFF);
            port_write(mmio_base, port, PORT_IS, 0xFFFF_FFFF);
            start_port(mmio_base, port);
        }

        let cmd_header = clb_phys as *mut CmdHeader;
        let cfl: u16 = 5;
        let w_bit: u16 = if write { 1 << 6 } else { 0 };
        (*cmd_header).flags = cfl | w_bit;
        (*cmd_header).prdtl = if dma_size > 0 { 1 } else { 0 };
        (*cmd_header).prdbc = 0;

        let cmd_table = ctba_phys as *mut CmdTable;
        core::ptr::write_bytes((*cmd_table).cfis.as_mut_ptr(), 0, 64);
        core::ptr::write_bytes((*cmd_table).acmd.as_mut_ptr(), 0, 16);

        let fis = (*cmd_table).cfis.as_mut_ptr() as *mut FisRegH2D;
        (*fis).fis_type = FIS_TYPE_REG_H2D;
        (*fis).flags = 0x80;
        (*fis).command = command;
        (*fis).device = 0x40;
        (*fis).lba0 = (lba & 0xFF) as u8;
        (*fis).lba1 = ((lba >> 8) & 0xFF) as u8;
        (*fis).lba2 = ((lba >> 16) & 0xFF) as u8;
        (*fis).lba3 = ((lba >> 24) & 0xFF) as u8;
        (*fis).lba4 = ((lba >> 32) & 0xFF) as u8;
        (*fis).lba5 = ((lba >> 40) & 0xFF) as u8;
        (*fis).count_lo = (count & 0xFF) as u8;
        (*fis).count_hi = ((count >> 8) & 0xFF) as u8;

        if dma_size > 0 {
            (*cmd_table).prdt[0].dba = dma_phys as u32;
            (*cmd_table).prdt[0].dbau = (dma_phys >> 32) as u32;
            (*cmd_table).prdt[0]._reserved = 0;
            (*cmd_table).prdt[0].dbc = (dma_size - 1) | (1 << 31);
        }

        port_write(mmio_base, port, PORT_IS, 0xFFFF_FFFF);
        port_write(mmio_base, port, PORT_CI, 1);

        // Polled wait with timeout (~5 seconds).
        // Use tick-based timeout when the timer is available, otherwise fall back
        // to a bounded iteration count.
        let hz = crate::arch::hal::timer_frequency_hz() as u32;
        if hz > 0 {
            let timeout_ticks = (AHCI_TIMEOUT_MS as u32 * hz / 1000).max(1);
            let start = crate::arch::hal::timer_current_ticks();
            loop {
                let ci = port_read(mmio_base, port, PORT_CI);
                if ci & 1 == 0 {
                    let tfd = port_read(mmio_base, port, PORT_TFD);
                    if tfd & 0x01 != 0 {
                        break; // error — will retry
                    }
                    return true;
                }
                let now = crate::arch::hal::timer_current_ticks();
                if now.wrapping_sub(start) >= timeout_ticks {
                    crate::serial_println!(
                        "AHCI: port {} command timeout (cmd={:#x}, lba={})",
                        port, command, lba
                    );
                    break; // timeout — will retry
                }
                core::hint::spin_loop();
            }
        } else {
            // Timer not yet running (early boot) — use iteration count
            for _ in 0..10_000_000 {
                let ci = port_read(mmio_base, port, PORT_CI);
                if ci & 1 == 0 {
                    let tfd = port_read(mmio_base, port, PORT_TFD);
                    if tfd & 0x01 != 0 {
                        break; // error — will retry
                    }
                    return true;
                }
                core::hint::spin_loop();
            }
        }
    }

    crate::serial_println!("AHCI: port {} command failed after retries", port);
    false
}

/// Polled completion check with timeout (used during boot or as IRQ timeout fallback).
/// Uses tick-based timing when available, falls back to iteration count during early boot.
unsafe fn poll_completion(ahci: &AhciController) -> bool {
    let hz = crate::arch::hal::timer_frequency_hz() as u32;

    if hz > 0 {
        // Tick-based timeout (~5 seconds)
        let timeout_ticks = (AHCI_TIMEOUT_MS as u32 * hz / 1000).max(1);
        let start = crate::arch::hal::timer_current_ticks();

        loop {
            let ci = port_read(ahci.mmio_base, ahci.active_port, PORT_CI);
            if ci & 1 == 0 {
                let tfd = port_read(ahci.mmio_base, ahci.active_port, PORT_TFD);
                if tfd & 0x01 != 0 {
                    crate::serial_verbose_println!("AHCI: command error, TFD={:#x}", tfd);
                    return false;
                }
                return true;
            }

            let is = port_read(ahci.mmio_base, ahci.active_port, PORT_IS);
            if is & (1 << 30) != 0 {
                crate::serial_verbose_println!("AHCI: task file error, IS={:#x}", is);
                return false;
            }

            let now = crate::arch::hal::timer_current_ticks();
            if now.wrapping_sub(start) >= timeout_ticks {
                crate::serial_println!("AHCI: poll_completion timeout");
                return false;
            }

            core::hint::spin_loop();
        }
    } else {
        // Early boot fallback — iteration count (no timer yet)
        for _ in 0..10_000_000 {
            let ci = port_read(ahci.mmio_base, ahci.active_port, PORT_CI);
            if ci & 1 == 0 {
                let tfd = port_read(ahci.mmio_base, ahci.active_port, PORT_TFD);
                if tfd & 0x01 != 0 {
                    crate::serial_verbose_println!("AHCI: command error, TFD={:#x}", tfd);
                    return false;
                }
                return true;
            }

            let is = port_read(ahci.mmio_base, ahci.active_port, PORT_IS);
            if is & (1 << 30) != 0 {
                crate::serial_verbose_println!("AHCI: task file error, IS={:#x}", is);
                return false;
            }

            core::hint::spin_loop();
        }

        crate::serial_verbose_println!("AHCI: command timeout");
        false
    }
}

// ── Public Read / Write API ─────────────────────────

/// Read `count` sectors starting at `lba` into `buf` via AHCI DMA.
pub fn read_sectors(lba: u32, count: u32, buf: &mut [u8]) -> bool {
    let ahci = match unsafe { AHCI.as_ref() } {
        Some(a) => a,
        None => return false,
    };

    let mut offset = 0usize;
    let mut remaining = count;
    let mut cur_lba = lba as u64;

    while remaining > 0 {
        let batch = remaining.min(BOUNCE_BUF_SECTORS);
        let byte_count = batch as usize * 512;

        let ok = unsafe {
            issue_command(
                ahci,
                ATA_CMD_READ_DMA_EXT,
                cur_lba,
                batch as u16,
                ahci.bounce_phys,
                byte_count as u32,
                false,
            )
        };

        if !ok {
            return false;
        }

        // Copy from bounce buffer to caller buffer
        unsafe {
            core::ptr::copy_nonoverlapping(
                ahci.bounce_virt as *const u8,
                buf.as_mut_ptr().add(offset),
                byte_count,
            );
        }

        offset += byte_count;
        cur_lba += batch as u64;
        remaining -= batch;
    }

    true
}

/// Write `count` sectors starting at `lba` from `buf` via AHCI DMA.
pub fn write_sectors(lba: u32, count: u32, buf: &[u8]) -> bool {
    let ahci = match unsafe { AHCI.as_ref() } {
        Some(a) => a,
        None => return false,
    };

    let mut offset = 0usize;
    let mut remaining = count;
    let mut cur_lba = lba as u64;

    while remaining > 0 {
        let batch = remaining.min(BOUNCE_BUF_SECTORS);
        let byte_count = batch as usize * 512;

        // Copy caller data to bounce buffer
        unsafe {
            core::ptr::copy_nonoverlapping(
                buf.as_ptr().add(offset),
                ahci.bounce_virt as *mut u8,
                byte_count,
            );
        }

        let ok = unsafe {
            issue_command(
                ahci,
                ATA_CMD_WRITE_DMA_EXT,
                cur_lba,
                batch as u16,
                ahci.bounce_phys,
                byte_count as u32,
                true,
            )
        };

        if !ok {
            return false;
        }

        offset += byte_count;
        cur_lba += batch as u64;
        remaining -= batch;
    }

    true
}

/// Flush the drive's write cache to persistent storage.
pub fn flush() -> bool {
    let ahci = match unsafe { AHCI.as_ref() } {
        Some(a) => a,
        None => return false,
    };
    unsafe { issue_command(ahci, ATA_CMD_FLUSH_EXT, 0, 0, 0, 0, false) }
}

// ── Initialization ──────────────────────────────────

/// Initialize the AHCI controller from a PCI device and register as active storage backend.
pub fn init_and_register(pci: &PciDevice) {
    // BAR5 = ABAR (AHCI Base Address Register)
    let abar_raw = pci.bars[5];

    // Debug: write BAR5 hex to VGA text buffer (row 24, visible even over boot splash)
    unsafe {
        let vga = 0xFFFF_FFFF_8000_0000u64 + 0xB8000; // kernel-mapped VGA text buffer
        let row = 24;
        let base = vga + (row * 160) as u64;
        let msg = b"BAR5=";
        for (i, &c) in msg.iter().enumerate() {
            *((base + i as u64 * 2) as *mut u8) = c;
            *((base + i as u64 * 2 + 1) as *mut u8) = 0x4F; // white on red
        }
        let hex = [
            b"0123456789ABCDEF"[((abar_raw >> 28) & 0xF) as usize],
            b"0123456789ABCDEF"[((abar_raw >> 24) & 0xF) as usize],
            b"0123456789ABCDEF"[((abar_raw >> 20) & 0xF) as usize],
            b"0123456789ABCDEF"[((abar_raw >> 16) & 0xF) as usize],
            b"0123456789ABCDEF"[((abar_raw >> 12) & 0xF) as usize],
            b"0123456789ABCDEF"[((abar_raw >> 8) & 0xF) as usize],
            b"0123456789ABCDEF"[((abar_raw >> 4) & 0xF) as usize],
            b"0123456789ABCDEF"[(abar_raw & 0xF) as usize],
        ];
        for (i, &c) in hex.iter().enumerate() {
            let off = (5 + i) as u64;
            *((base + off * 2) as *mut u8) = c;
            *((base + off * 2 + 1) as *mut u8) = 0x4F;
        }
    }

    if abar_raw == 0 {
        crate::serial_verbose_println!("  AHCI: BAR5 is zero, cannot initialize");
        return;
    }
    let abar_phys = (abar_raw & !0xF) as u64;
    crate::serial_println!("  AHCI: ABAR phys = {:#010x}", abar_phys);

    // Enable PCI bus mastering + memory space + I/O space
    let cmd = pci_config_read32(pci.bus, pci.device, pci.function, 0x04);
    pci_config_write32(pci.bus, pci.device, pci.function, 0x04, cmd | 0x07);

    // Map ABAR to kernel virtual space
    let mmio_base = AHCI_MMIO_VIRT;
    for i in 0..AHCI_MMIO_PAGES {
        let phys = PhysAddr::new(abar_phys + (i as u64) * 4096);
        let virt = VirtAddr::new(mmio_base + (i as u64) * 4096);
        virtual_mem::map_page(virt, phys, 0x03); // Present + Writable
    }

    unsafe {
        // Enable AHCI mode
        let ghc = mmio_read32(mmio_base, REG_GHC);
        mmio_write32(mmio_base, REG_GHC, ghc | GHC_AE);

        // Read capabilities
        let cap = mmio_read32(mmio_base, REG_CAP);
        let num_ports = (cap & 0x1F) + 1;
        let pi = mmio_read32(mmio_base, REG_PI);
        let vs = mmio_read32(mmio_base, REG_VS);
        let vs_major = (vs >> 16) & 0xFFFF;
        let vs_minor = vs & 0xFFFF;

        crate::serial_verbose_println!(
            "  AHCI: version {}.{:02x}, {} ports, PI={:#06x}",
            vs_major, vs_minor, num_ports, pi
        );

        // Debug marker: write 0xAA to port 0 IE to confirm AHCI init runs.
        // Visible in CoreVM heartbeat as "ie=0xaa". Changed to 0xBB after
        // ATAPI detection, 0xCC after ATAPI DMA setup.
        port_write(mmio_base, 0, PORT_IE, 0xAA);

        // Find ports with connected devices (ATA disks and ATAPI CD-ROM)
        let mut found_port: Option<u32> = None;
        let mut extra_ata_ports: [Option<u32>; MAX_EXTRA_DISKS] = [None; MAX_EXTRA_DISKS];
        let mut extra_ata_count: usize = 0;
        let mut found_atapi: Option<u32> = None;
        for port in 0..32u32 {
            if pi & (1 << port) == 0 {
                continue;
            }

            let ssts = port_read(mmio_base, port, PORT_SSTS);
            let det = ssts & 0x0F;
            if det != 3 {
                continue; // No device or PHY not established
            }

            let sig = port_read(mmio_base, port, PORT_SIG);
            let desc = match sig {
                SATA_SIG_ATA => "SATA disk",
                SATA_SIG_ATAPI => "ATAPI CD-ROM",
                _ => "other",
            };
            crate::serial_println!(
                "  AHCI: port {} sig={:#010x} det={} ({})",
                port, sig, det, desc
            );
            // Debug via I/O port 0x504 (visible in CoreVM log as "unhandled write")
            // Value encodes: high nibble = event, low nibble = port
            // 0xA0+port = port found, 0xB0+port = ATAPI detected
            unsafe { core::arch::asm!("out dx, al", in("dx") 0x504u16, in("al") (0xA0u8 + port as u8)); }

            if sig == SATA_SIG_ATA {
                if found_port.is_none() {
                    found_port = Some(port);
                } else if extra_ata_count < MAX_EXTRA_DISKS {
                    extra_ata_ports[extra_ata_count] = Some(port);
                    extra_ata_count += 1;
                    crate::serial_println!("  AHCI: additional disk on port {}", port);
                }
            } else if sig == SATA_SIG_ATAPI && found_atapi.is_none() {
                found_atapi = Some(port);
                port_write(mmio_base, 0, PORT_IE, 0xBB); // marker: ATAPI found
            }
        }

        // Marker: encode scan results in IE (visible in heartbeat)
        // 0xAA = init ran but nothing found beyond that
        // 0xBB = ATAPI found
        // If still 0xAA here, no ATAPI was found
        if found_atapi.is_none() {
            // Encode PI value in IE so we can see what ports are implemented
            port_write(mmio_base, 0, PORT_IE, 0xAA00 | (pi & 0xFF));
        }

        let active_port = match found_port {
            Some(p) => p,
            None => {
                crate::serial_verbose_println!("  AHCI: No SATA disk found");
                return;
            }
        };

        // Stop port before configuring
        stop_port(mmio_base, active_port);

        // ── Allocate DMA structures (identity-mapped, phys < 128 MiB) ──

        // Command List: 1 KiB (1 frame)
        let clb_phys = match physical::alloc_frame() {
            Some(f) => f.as_u64(),
            None => {
                crate::serial_verbose_println!("  AHCI: Failed to allocate CLB frame");
                return;
            }
        };

        // FIS Receive Area: 256 bytes (1 frame)
        let fb_phys = match physical::alloc_frame() {
            Some(f) => f.as_u64(),
            None => {
                crate::serial_verbose_println!("  AHCI: Failed to allocate FB frame");
                return;
            }
        };

        // Command Table: ~256 bytes (1 frame, 128-byte aligned by nature of 4K frame)
        let ctba_phys = match physical::alloc_frame() {
            Some(f) => f.as_u64(),
            None => {
                crate::serial_verbose_println!("  AHCI: Failed to allocate CT frame");
                return;
            }
        };

        // Bounce buffer: 128 KiB = 32 contiguous frames
        let bounce_phys = match physical::alloc_contiguous(BOUNCE_BUF_FRAMES) {
            Some(f) => f.as_u64(),
            None => {
                crate::serial_verbose_println!("  AHCI: Failed to allocate bounce buffer ({} frames)", BOUNCE_BUF_FRAMES);
                return;
            }
        };

        crate::serial_println!("  AHCI: DMA alloc: CLB={:#x} FB={:#x} CT={:#x} bounce={:#x}",
            clb_phys, fb_phys, ctba_phys, bounce_phys);

        // Zero all DMA structures
        core::ptr::write_bytes(clb_phys as *mut u8, 0, 4096);
        core::ptr::write_bytes(fb_phys as *mut u8, 0, 4096);
        core::ptr::write_bytes(ctba_phys as *mut u8, 0, 4096);
        core::ptr::write_bytes(bounce_phys as *mut u8, 0, BOUNCE_BUF_SIZE);

        // Pre-configure CmdHeader[0] to point to our command table
        let cmd_header = clb_phys as *mut CmdHeader;
        (*cmd_header).ctba = ctba_phys as u32;
        (*cmd_header).ctbau = (ctba_phys >> 32) as u32;

        // Configure port DMA addresses
        port_write(mmio_base, active_port, PORT_CLB, clb_phys as u32);
        port_write(mmio_base, active_port, PORT_CLBU, (clb_phys >> 32) as u32);
        port_write(mmio_base, active_port, PORT_FB, fb_phys as u32);
        port_write(mmio_base, active_port, PORT_FBU, (fb_phys >> 32) as u32);

        // Clear errors and interrupts
        port_write(mmio_base, active_port, PORT_SERR, 0xFFFF_FFFF);
        port_write(mmio_base, active_port, PORT_IS, 0xFFFF_FFFF);

        // Start the port
        start_port(mmio_base, active_port);

        // Get PCI interrupt line for IRQ-driven I/O
        let mut irq = pci.interrupt_line;

        // ── ATAPI port setup (if CD-ROM detected) ──
        let mut atapi_clb_phys = 0u64;
        let mut atapi_fb_phys = 0u64;
        let mut atapi_ctba_phys = 0u64;

        if let Some(atapi_port) = found_atapi {
            // Each AHCI port needs its own CLB, FB, and CT
            if let (Some(clb), Some(fb), Some(ct)) = (
                physical::alloc_frame(),
                physical::alloc_frame(),
                physical::alloc_frame(),
            ) {
                atapi_clb_phys = clb.as_u64();
                atapi_fb_phys = fb.as_u64();
                atapi_ctba_phys = ct.as_u64();

                core::ptr::write_bytes(atapi_clb_phys as *mut u8, 0, 4096);
                core::ptr::write_bytes(atapi_fb_phys as *mut u8, 0, 4096);
                core::ptr::write_bytes(atapi_ctba_phys as *mut u8, 0, 4096);

                // Pre-configure CmdHeader[0] for ATAPI port
                let cmd_header = atapi_clb_phys as *mut CmdHeader;
                (*cmd_header).ctba = atapi_ctba_phys as u32;
                (*cmd_header).ctbau = (atapi_ctba_phys >> 32) as u32;

                // Configure ATAPI port DMA addresses
                stop_port(mmio_base, atapi_port);
                port_write(mmio_base, atapi_port, PORT_CLB, atapi_clb_phys as u32);
                port_write(mmio_base, atapi_port, PORT_CLBU, (atapi_clb_phys >> 32) as u32);
                port_write(mmio_base, atapi_port, PORT_FB, atapi_fb_phys as u32);
                port_write(mmio_base, atapi_port, PORT_FBU, (atapi_fb_phys >> 32) as u32);
                port_write(mmio_base, atapi_port, PORT_SERR, 0xFFFF_FFFF);
                port_write(mmio_base, atapi_port, PORT_IS, 0xFFFF_FFFF);
                start_port(mmio_base, atapi_port);

                crate::serial_println!("  AHCI: ATAPI port {} initialized (DMA OK)", atapi_port);
            } else {
                crate::serial_verbose_println!("  AHCI: Failed to allocate ATAPI DMA frames");
            }
        }

        // Store controller state
        AHCI = Some(AhciController {
            mmio_base,
            active_port,
            clb_phys,
            fb_phys,
            ctba_phys,
            bounce_phys,
            bounce_virt: bounce_phys, // identity-mapped
            total_sectors: 0,
            irq,
            extra_disks: [None, None, None, None, None, None, None],
            extra_disk_count: 0,
            atapi_port: if atapi_clb_phys != 0 { found_atapi } else { None },
            atapi_clb_phys,
            atapi_fb_phys,
            atapi_ctba_phys,
        });

        // Issue IDENTIFY DEVICE (polled — scheduler not yet running)
        let identify_ok = issue_command(
            AHCI.as_ref().unwrap(),
            ATA_CMD_IDENTIFY,
            0,  // LBA = 0
            1,  // count = 1
            bounce_phys,
            512,
            false,
        );

        if identify_ok {
            let identify = bounce_phys as *const u16;

            // Parse model string (words 27-46, byte-swapped)
            let mut model = [0u8; 40];
            for i in 0..20 {
                let word = *identify.add(27 + i);
                model[i * 2] = (word >> 8) as u8;
                model[i * 2 + 1] = word as u8;
            }

            // Sector count — LBA48 (words 100-103), fallback to LBA28 (words 60-61)
            let sectors_lo = *identify.add(100) as u64 | ((*identify.add(101) as u64) << 16);
            let sectors_hi = *identify.add(102) as u64 | ((*identify.add(103) as u64) << 16);
            let mut total_sectors = sectors_lo | (sectors_hi << 32);
            if total_sectors == 0 {
                total_sectors = (*identify.add(60) as u64) | ((*identify.add(61) as u64) << 16);
            }

            if let Some(ahci) = AHCI.as_mut() {
                ahci.total_sectors = total_sectors;
            }

            let model_str = core::str::from_utf8(&model).unwrap_or("???").trim();
            crate::serial_verbose_println!(
                "  AHCI: '{}', {} sectors ({} MiB)",
                model_str,
                total_sectors,
                total_sectors / 2048
            );
        } else {
            crate::serial_verbose_println!("  AHCI: IDENTIFY DEVICE failed");
        }

        // Enable interrupt-driven I/O — prefer MSI over legacy IRQ
        {
            // Enable command-completion, error, and hot-plug interrupts on port
            let port_ie = (1u32 << 0)  // D2H Register FIS Interrupt (command complete)
                        | (1 << 22)    // PhyRdy Change Status (hot-plug)
                        | (1 << 30)    // Task File Error Status
                        | (1 << 31);   // Host Bus Fatal Error
            port_write(mmio_base, active_port, PORT_IE, port_ie);

            // Enable hot-plug interrupts on ALL implemented ports (not just active)
            for hp_port in 0..32u32 {
                if pi & (1 << hp_port) != 0 && hp_port != active_port {
                    port_write(mmio_base, hp_port, PORT_IE, 1 << 22); // PRCS only
                }
            }

            // Enable HBA global interrupts
            let ghc = mmio_read32(mmio_base, REG_GHC);
            mmio_write32(mmio_base, REG_GHC, ghc | GHC_IE);

            // Try MSI first — dedicated vector, no IRQ sharing
            let msi_vector = crate::drivers::pci_msi::enable_msi(pci);
            if let Some(vec) = msi_vector {
                crate::drivers::pci_msi::register_msi_handler(vec, ahci_msi_handler);
                irq = vec; // Store for diagnostics
                crate::serial_println!("  AHCI: MSI vector {} registered (dedicated interrupt)", vec);
            } else if irq > 0 && irq < 32 {
                // Fallback: legacy IRQ via IOAPIC
                crate::arch::x86::irq::register_irq_chain(irq, ahci_irq_handler);
                if crate::arch::x86::apic::is_initialized() {
                    crate::arch::x86::ioapic::unmask_irq(irq);
                } else {
                    crate::arch::x86::pic::unmask(irq);
                }
                crate::serial_println!("  AHCI: legacy IRQ {} registered (shared interrupt)", irq);
            } else {
                crate::serial_verbose_println!("  AHCI: No valid IRQ ({}), using polled I/O", irq);
            }
        }

        // Switch storage backend to AHCI
        super::set_backend_ahci();

        crate::serial_verbose_println!("[OK] AHCI initialized (port {}, DMA mode)", active_port);

        // ── Initialize additional ATA disks ──
        for i in 0..extra_ata_count {
            let extra_port = match extra_ata_ports[i] {
                Some(p) => p,
                None => continue,
            };

            // Allocate DMA structures for this port
            let (e_clb, e_fb, e_ct) = match (
                physical::alloc_frame(),
                physical::alloc_frame(),
                physical::alloc_frame(),
            ) {
                (Some(clb), Some(fb), Some(ct)) => (clb.as_u64(), fb.as_u64(), ct.as_u64()),
                _ => {
                    crate::serial_verbose_println!("  AHCI: Failed to allocate DMA for port {}", extra_port);
                    continue;
                }
            };

            core::ptr::write_bytes(e_clb as *mut u8, 0, 4096);
            core::ptr::write_bytes(e_fb as *mut u8, 0, 4096);
            core::ptr::write_bytes(e_ct as *mut u8, 0, 4096);

            // Point CmdHeader[0] to command table
            let cmd_header = e_clb as *mut CmdHeader;
            (*cmd_header).ctba = e_ct as u32;
            (*cmd_header).ctbau = (e_ct >> 32) as u32;

            // Configure port registers
            stop_port(mmio_base, extra_port);
            port_write(mmio_base, extra_port, PORT_CLB, e_clb as u32);
            port_write(mmio_base, extra_port, PORT_CLBU, (e_clb >> 32) as u32);
            port_write(mmio_base, extra_port, PORT_FB, e_fb as u32);
            port_write(mmio_base, extra_port, PORT_FBU, (e_fb >> 32) as u32);
            port_write(mmio_base, extra_port, PORT_SERR, 0xFFFF_FFFF);
            port_write(mmio_base, extra_port, PORT_IS, 0xFFFF_FFFF);
            start_port(mmio_base, extra_port);

            // IDENTIFY this disk using the extra port's DMA structures
            let mut extra_total_sectors: u64 = 0;
            let id_ok = issue_command_on_port(
                mmio_base, extra_port, e_clb, e_ct,
                bounce_phys, // shared bounce buffer (only one port active at a time due to IO_LOCK)
                ATA_CMD_IDENTIFY, 0, 1, 512, false,
            );
            if id_ok {
                let identify = bounce_phys as *const u16;
                let lo = *identify.add(100) as u64 | ((*identify.add(101) as u64) << 16);
                let hi = *identify.add(102) as u64 | ((*identify.add(103) as u64) << 16);
                extra_total_sectors = lo | (hi << 32);
                if extra_total_sectors == 0 {
                    extra_total_sectors = (*identify.add(60) as u64) | ((*identify.add(61) as u64) << 16);
                }
                crate::serial_println!(
                    "  AHCI: port {} disk: {} sectors ({} MiB)",
                    extra_port, extra_total_sectors, extra_total_sectors / 2048
                );
            }

            // Enable interrupts on this port too
            let port_ie = (1u32 << 0) | (1 << 30) | (1 << 31);
            port_write(mmio_base, extra_port, PORT_IE, port_ie);

            // Store in controller state
            if let Some(ahci) = AHCI.as_mut() {
                let idx = ahci.extra_disk_count;
                if idx < MAX_EXTRA_DISKS {
                    ahci.extra_disks[idx] = Some(PortDma {
                        port: extra_port,
                        clb_phys: e_clb,
                        fb_phys: e_fb,
                        ctba_phys: e_ct,
                        total_sectors: extra_total_sectors,
                    });
                    ahci.extra_disk_count += 1;
                }
            }

            // Register as block device via the per-device I/O override system.
            // disk_id = 1 + i (primary disk is 0)
            let disk_id = (1 + i) as u8;
            // Store port→disk_id mapping for I/O dispatch
            unsafe {
                if i < MAX_EXTRA_DISKS {
                    EXTRA_DISK_MAP[i] = (disk_id, extra_port, extra_total_sectors);
                }
            }
            super::register_device_io(disk_id, extra_disk_read, extra_disk_write);

            // Register in blockdev
            use super::blockdev;
            blockdev::register_device(blockdev::BlockDevice {
                id: disk_id, disk_id, partition: None,
                start_lba: 0, size_sectors: extra_total_sectors,
            });
            blockdev::scan_and_register_partitions(disk_id);

            crate::serial_println!("  AHCI: registered extra disk {} on port {}", disk_id, extra_port);
        }
    }
}

// ── Extra disk I/O dispatch ──────────────────────────

/// Mapping: (disk_id, ahci_port, total_sectors) for extra disks.
static mut EXTRA_DISK_MAP: [(u8, u32, u64); MAX_EXTRA_DISKS] = [(0, 0, 0); MAX_EXTRA_DISKS];

fn extra_disk_read(disk_id: u8, lba: u32, count: u32, buf: &mut [u8]) -> bool {
    let ahci = match unsafe { AHCI.as_ref() } {
        Some(a) => a,
        None => return false,
    };
    // Find the port for this disk_id
    let port_dma = match ahci.extra_disks.iter().flatten().find(|d| {
        unsafe { EXTRA_DISK_MAP.iter().any(|&(did, p, _)| did == disk_id && p == d.port) }
    }) {
        Some(d) => d,
        None => return false,
    };

    let mut offset = 0usize;
    let mut remaining = count;
    let mut cur_lba = lba as u64;

    while remaining > 0 {
        let batch = remaining.min(BOUNCE_BUF_SECTORS);
        let byte_count = batch as usize * 512;

        let ok = unsafe {
            issue_command_on_port(
                ahci.mmio_base, port_dma.port,
                port_dma.clb_phys, port_dma.ctba_phys,
                ahci.bounce_phys,
                ATA_CMD_READ_DMA_EXT, cur_lba, batch as u16,
                byte_count as u32, false,
            )
        };
        if !ok { return false; }

        unsafe {
            core::ptr::copy_nonoverlapping(
                ahci.bounce_virt as *const u8,
                buf.as_mut_ptr().add(offset),
                byte_count,
            );
        }
        offset += byte_count;
        cur_lba += batch as u64;
        remaining -= batch;
    }
    true
}

fn extra_disk_write(disk_id: u8, lba: u32, count: u32, buf: &[u8]) -> bool {
    let ahci = match unsafe { AHCI.as_ref() } {
        Some(a) => a,
        None => return false,
    };
    let port_dma = match ahci.extra_disks.iter().flatten().find(|d| {
        unsafe { EXTRA_DISK_MAP.iter().any(|&(did, p, _)| did == disk_id && p == d.port) }
    }) {
        Some(d) => d,
        None => return false,
    };

    let mut offset = 0usize;
    let mut remaining = count;
    let mut cur_lba = lba as u64;

    while remaining > 0 {
        let batch = remaining.min(BOUNCE_BUF_SECTORS);
        let byte_count = batch as usize * 512;

        unsafe {
            core::ptr::copy_nonoverlapping(
                buf.as_ptr().add(offset),
                ahci.bounce_virt as *mut u8,
                byte_count,
            );
        }

        let ok = unsafe {
            issue_command_on_port(
                ahci.mmio_base, port_dma.port,
                port_dma.clb_phys, port_dma.ctba_phys,
                ahci.bounce_phys,
                ATA_CMD_WRITE_DMA_EXT, cur_lba, batch as u16,
                byte_count as u32, true,
            )
        };
        if !ok { return false; }

        offset += byte_count;
        cur_lba += batch as u64;
        remaining -= batch;
    }
    true
}

// ── ATAPI CD-ROM Support ──────────────────────────

/// Check if an AHCI ATAPI CD-ROM device is available.
pub fn atapi_is_present() -> bool {
    unsafe { AHCI.as_ref().map_or(false, |a| a.atapi_port.is_some()) }
}

/// Return the total number of 512-byte sectors on the SATA disk (0 if not initialized).
pub fn disk_total_sectors() -> u64 {
    unsafe { AHCI.as_ref().map_or(0, |a| a.total_sectors) }
}

/// Read `count` 2048-byte CD blocks starting at `lba` into `buf` via AHCI ATAPI.
/// Uses SCSI READ(10) command encapsulated in an ATA PACKET command.
pub fn read_cd_sectors(lba: u32, count: u32, buf: &mut [u8]) -> bool {
    let ahci = match unsafe { AHCI.as_ref() } {
        Some(a) => a,
        None => return false,
    };
    let atapi_port = match ahci.atapi_port {
        Some(p) => p,
        None => return false,
    };

    let mut offset = 0usize;
    let mut remaining = count;
    let mut cur_lba = lba;

    // Max 32 CD sectors per batch (64 KiB, fits in bounce buffer)
    const MAX_CD_BATCH: u32 = 32;

    while remaining > 0 {
        let batch = remaining.min(MAX_CD_BATCH);
        let byte_count = batch as usize * 2048;

        let ok = unsafe {
            issue_atapi_read(ahci, atapi_port, cur_lba, batch, byte_count as u32)
        };

        if !ok {
            return false;
        }

        unsafe {
            core::ptr::copy_nonoverlapping(
                ahci.bounce_virt as *const u8,
                buf.as_mut_ptr().add(offset),
                byte_count,
            );
        }

        offset += byte_count;
        cur_lba += batch;
        remaining -= batch;
    }

    true
}

/// Issue an ATAPI READ(10) command via AHCI PACKET to the ATAPI port.
unsafe fn issue_atapi_read(
    ahci: &AhciController,
    port: u32,
    lba: u32,
    count: u32,
    byte_count: u32,
) -> bool {
    // Set up command header (slot 0) on the ATAPI port's CLB
    let cmd_header = ahci.atapi_clb_phys as *mut CmdHeader;
    let cfl: u16 = 5; // 5 DWORDs for Register H2D FIS
    let a_bit: u16 = 1 << 5; // ATAPI flag
    (*cmd_header).flags = cfl | a_bit;
    (*cmd_header).prdtl = 1;
    (*cmd_header).prdbc = 0;

    // Set up command table on the ATAPI port
    let cmd_table = ahci.atapi_ctba_phys as *mut CmdTable;

    // Zero CFIS + ACMD
    core::ptr::write_bytes((*cmd_table).cfis.as_mut_ptr(), 0, 64);
    core::ptr::write_bytes((*cmd_table).acmd.as_mut_ptr(), 0, 16);

    // Fill Register H2D FIS for ATA PACKET command
    let fis = (*cmd_table).cfis.as_mut_ptr() as *mut FisRegH2D;
    (*fis).fis_type = FIS_TYPE_REG_H2D;
    (*fis).flags = 0x80; // C bit = command
    (*fis).command = ATA_CMD_PACKET;
    (*fis).features_lo = 1; // DMA transfer
    (*fis).device = 0;
    // Byte count limit in LBA1:LBA0 (for PIO, but set anyway)
    (*fis).lba0 = (byte_count & 0xFF) as u8;
    (*fis).lba1 = ((byte_count >> 8) & 0xFF) as u8;

    // Fill SCSI READ(10) CDB in ACMD (0x28 — widely supported by emulators)
    let acmd = (*cmd_table).acmd.as_mut_ptr();
    *acmd.add(0) = 0x28; // READ(10) opcode
    *acmd.add(1) = 0x00;
    *acmd.add(2) = ((lba >> 24) & 0xFF) as u8; // LBA MSB
    *acmd.add(3) = ((lba >> 16) & 0xFF) as u8;
    *acmd.add(4) = ((lba >> 8) & 0xFF) as u8;
    *acmd.add(5) = (lba & 0xFF) as u8;         // LBA LSB
    *acmd.add(6) = 0x00;                         // Reserved
    *acmd.add(7) = ((count >> 8) & 0xFF) as u8;  // Transfer length MSB
    *acmd.add(8) = (count & 0xFF) as u8;          // Transfer length LSB
    *acmd.add(9) = 0x00;                          // Control

    // Fill PRDT[0]: data goes into shared bounce buffer
    (*cmd_table).prdt[0].dba = ahci.bounce_phys as u32;
    (*cmd_table).prdt[0].dbau = (ahci.bounce_phys >> 32) as u32;
    (*cmd_table).prdt[0]._reserved = 0;
    (*cmd_table).prdt[0].dbc = (byte_count - 1) | (1 << 31); // IOC + byte count

    // Clear port interrupt status
    port_write(ahci.mmio_base, port, PORT_IS, 0xFFFF_FFFF);

    // Issue command (slot 0)
    port_write(ahci.mmio_base, port, PORT_CI, 1);

    // Spin-wait for completion with timeout.
    // ATAPI is slower than ATA, but still bounded to ~5 seconds.
    let hz = crate::arch::hal::timer_frequency_hz() as u32;
    if hz > 0 {
        let timeout_ticks = (AHCI_TIMEOUT_MS as u32 * hz / 1000).max(1);
        let start = crate::arch::hal::timer_current_ticks();
        loop {
            let ci = port_read(ahci.mmio_base, port, PORT_CI);
            if ci & 1 == 0 {
                let tfd = port_read(ahci.mmio_base, port, PORT_TFD);
                if tfd & 0x01 != 0 {
                    crate::serial_verbose_println!("AHCI ATAPI: command error, TFD={:#x}", tfd);
                    return false;
                }
                return true;
            }
            let now = crate::arch::hal::timer_current_ticks();
            if now.wrapping_sub(start) >= timeout_ticks {
                crate::serial_println!("AHCI ATAPI: command timeout (lba={}, count={})", lba, count);
                return false;
            }
            core::hint::spin_loop();
        }
    } else {
        // Early boot fallback
        for _ in 0..500_000 {
            let ci = port_read(ahci.mmio_base, port, PORT_CI);
            if ci & 1 == 0 {
                let tfd = port_read(ahci.mmio_base, port, PORT_TFD);
                if tfd & 0x01 != 0 {
                    crate::serial_verbose_println!("AHCI ATAPI: command error, TFD={:#x}", tfd);
                    return false;
                }
                return true;
            }
            core::hint::spin_loop();
        }
        crate::serial_verbose_println!("AHCI ATAPI: command timeout");
        false
    }
}

/// Probe: initialize AHCI and return a HAL driver.
pub fn probe(pci: &PciDevice) -> Option<Box<dyn crate::drivers::hal::Driver>> {
    init_and_register(pci);
    super::create_hal_driver("AHCI SATA Controller")
}
