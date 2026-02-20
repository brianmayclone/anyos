//! AHCI (SATA) driver for DMA-based disk I/O.
//!
//! Supports AHCI 1.0+ host controllers (PCI class 01:06, prog IF 01).
//! Uses DMA transfers via MMIO, replacing legacy ATA PIO when available.

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

// ── Bounce buffer: 128 KiB = 256 sectors ────────────
const BOUNCE_BUF_SECTORS: u32 = 256;
const BOUNCE_BUF_SIZE: usize = BOUNCE_BUF_SECTORS as usize * 512;
const BOUNCE_BUF_FRAMES: usize = BOUNCE_BUF_SIZE / 4096; // 32

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

fn ahci_irq_handler(_irq: u8) {
    use core::sync::atomic::Ordering;

    let ahci = match unsafe { AHCI.as_ref() } {
        Some(a) => a,
        None => return,
    };

    unsafe {
        // Check HBA global interrupt status
        let hba_is = mmio_read32(ahci.mmio_base, REG_IS);
        if hba_is & (1 << ahci.active_port) == 0 {
            return; // Not our port
        }

        // Clear port interrupt status
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
        crate::task::scheduler::wake_thread(tid);
    }
}

// ── Command Issue (IRQ-driven, slot 0 only) ─────────

unsafe fn issue_command(
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

    // Fast path: brief spin — QEMU DMA completes in microseconds.
    // Keep iteration count low: each MMIO read is a VM exit on VirtualBox
    // NEM/Hyper-V (~1-50μs each). 50k iterations wasted seconds; 1k is
    // enough to catch QEMU's near-instant DMA while falling through
    // quickly to the IRQ-driven slow path on real hardware and VBox.
    for _ in 0..1_000 {
        let ci = port_read(ahci.mmio_base, ahci.active_port, PORT_CI);
        if ci & 1 == 0 {
            let tfd = port_read(ahci.mmio_base, ahci.active_port, PORT_TFD);
            if tfd & 0x01 != 0 {
                crate::serial_println!("AHCI: command error, TFD={:#x}", tfd);
                return false;
            }
            return true;
        }
        core::hint::spin_loop();
    }

    // Slow path: yield-and-poll with IRQ assist.
    // Uses sleep_until(tick+1) which auto-wakes on the next PIT tick (~1ms)
    // even if the AHCI IRQ never fires (e.g., IOAPIC routing mismatch in
    // VirtualBox). The IRQ handler can still wake us sooner via wake_thread.
    // Also checks CI directly — no sole reliance on AHCI_IRQ_FIRED.
    let tid = crate::task::scheduler::current_tid();
    if tid > 0 {
        if ahci.irq > 0 {
            AHCI_WAITER.store(tid, core::sync::atomic::Ordering::Release);
        }

        let start = crate::arch::x86::pit::get_ticks();
        loop {
            // Check command completion directly (no dependency on IRQ)
            let ci = port_read(ahci.mmio_base, ahci.active_port, PORT_CI);
            if ci & 1 == 0 {
                break;
            }
            if crate::arch::x86::pit::get_ticks().wrapping_sub(start) > 2000 {
                AHCI_WAITER.store(0, core::sync::atomic::Ordering::Release);
                crate::serial_println!("AHCI: command timeout");
                return false;
            }
            // Sleep for 1 PIT tick (~1ms). PIT tick handler auto-wakes us.
            // AHCI IRQ handler can also wake us sooner via wake_thread.
            let now = crate::arch::x86::pit::get_ticks();
            crate::task::scheduler::sleep_until(now.wrapping_add(1));
        }

        AHCI_WAITER.store(0, core::sync::atomic::Ordering::Release);

        let tfd = port_read(ahci.mmio_base, ahci.active_port, PORT_TFD);
        if tfd & 0x01 != 0 {
            crate::serial_println!("AHCI: command error, TFD={:#x}", tfd);
            return false;
        }
        return true;
    }

    // Fallback: extended poll (boot thread or no IRQ)
    poll_completion(ahci)
}

/// Polled completion check (used during boot or as IRQ timeout fallback).
unsafe fn poll_completion(ahci: &AhciController) -> bool {
    for _ in 0..10_000_000 {
        let ci = port_read(ahci.mmio_base, ahci.active_port, PORT_CI);
        if ci & 1 == 0 {
            let tfd = port_read(ahci.mmio_base, ahci.active_port, PORT_TFD);
            if tfd & 0x01 != 0 {
                crate::serial_println!("AHCI: command error, TFD={:#x}", tfd);
                return false;
            }
            return true;
        }

        let is = port_read(ahci.mmio_base, ahci.active_port, PORT_IS);
        if is & (1 << 30) != 0 {
            crate::serial_println!("AHCI: task file error, IS={:#x}", is);
            return false;
        }

        core::hint::spin_loop();
    }

    crate::serial_println!("AHCI: command timeout");
    false
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

    // Flush cache
    unsafe {
        let _ = issue_command(ahci, ATA_CMD_FLUSH_EXT, 0, 0, 0, 0, false);
    }

    true
}

// ── Initialization ──────────────────────────────────

/// Initialize the AHCI controller from a PCI device and register as active storage backend.
pub fn init_and_register(pci: &PciDevice) {
    // BAR5 = ABAR (AHCI Base Address Register)
    let abar_raw = pci.bars[5];
    if abar_raw == 0 {
        crate::serial_println!("  AHCI: BAR5 is zero, cannot initialize");
        return;
    }
    let abar_phys = (abar_raw & !0xF) as u64;

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

        crate::serial_println!(
            "  AHCI: version {}.{:02x}, {} ports, PI={:#06x}",
            vs_major, vs_minor, num_ports, pi
        );

        // Find a port with a connected SATA disk
        let mut found_port: Option<u32> = None;
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
            crate::serial_println!(
                "  AHCI: port {} — device present (sig={:#010x}, {})",
                port,
                sig,
                if sig == SATA_SIG_ATA { "SATA disk" } else { "other" }
            );

            if sig == SATA_SIG_ATA && found_port.is_none() {
                found_port = Some(port);
            }
        }

        let active_port = match found_port {
            Some(p) => p,
            None => {
                crate::serial_println!("  AHCI: No SATA disk found");
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
                crate::serial_println!("  AHCI: Failed to allocate CLB frame");
                return;
            }
        };

        // FIS Receive Area: 256 bytes (1 frame)
        let fb_phys = match physical::alloc_frame() {
            Some(f) => f.as_u64(),
            None => {
                crate::serial_println!("  AHCI: Failed to allocate FB frame");
                return;
            }
        };

        // Command Table: ~256 bytes (1 frame, 128-byte aligned by nature of 4K frame)
        let ctba_phys = match physical::alloc_frame() {
            Some(f) => f.as_u64(),
            None => {
                crate::serial_println!("  AHCI: Failed to allocate CT frame");
                return;
            }
        };

        // Bounce buffer: 128 KiB = 32 contiguous frames
        let bounce_phys = match physical::alloc_contiguous(BOUNCE_BUF_FRAMES) {
            Some(f) => f.as_u64(),
            None => {
                crate::serial_println!("  AHCI: Failed to allocate bounce buffer ({} frames)", BOUNCE_BUF_FRAMES);
                return;
            }
        };

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
        let irq = pci.interrupt_line;

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
            crate::serial_println!(
                "  AHCI: '{}', {} sectors ({} MiB)",
                model_str,
                total_sectors,
                total_sectors / 2048
            );
        } else {
            crate::serial_println!("  AHCI: IDENTIFY DEVICE failed");
        }

        // Enable interrupt-driven I/O
        if irq > 0 && irq < 32 {
            // Only enable command-completion + error interrupts
            // (NOT PIO Setup / DMA Setup which fire mid-transfer)
            let port_ie = (1u32 << 0)  // D2H Register FIS Interrupt (command complete)
                        | (1 << 30)    // Task File Error Status
                        | (1 << 31);   // Host Bus Fatal Error
            port_write(mmio_base, active_port, PORT_IE, port_ie);

            // Enable HBA global interrupts
            let ghc = mmio_read32(mmio_base, REG_GHC);
            mmio_write32(mmio_base, REG_GHC, ghc | GHC_IE);

            // Register shared IRQ handler (IRQ 11 may be shared with E1000)
            crate::arch::x86::irq::register_irq_chain(irq, ahci_irq_handler);
            if crate::arch::x86::apic::is_initialized() {
                crate::arch::x86::ioapic::unmask_irq(irq);
            } else {
                crate::arch::x86::pic::unmask(irq);
            }
            crate::serial_println!("  AHCI: IRQ {} registered (interrupt-driven I/O)", irq);
        } else {
            crate::serial_println!("  AHCI: No valid IRQ ({}), using polled I/O", irq);
        }

        // Switch storage backend to AHCI
        super::set_backend_ahci();

        crate::serial_println!("[OK] AHCI initialized (port {}, DMA mode)", active_port);
    }
}
