//! Intel I225/I226/I210/I211 Ethernet driver (igc family).
//!
//! Covers Intel's 2.5 GbE and 1 GbE controllers found in modern desktops,
//! NUCs, and servers. Register-compatible with E1000 but with enhanced features.
//!
//! PCI vendor 0x8086, devices: 0x15F2, 0x15F3 (I225-V/LM), 0x15F4, 0x15F5,
//! 0x15F9, 0x15FA, 0x15FB, 0x15FC (I225 variants),
//! 0x1533 (I210), 0x1539 (I211AT), 0x15B7, 0x15B8 (I219-V/LM).
//!
//! Architecture is very close to E1000: MMIO registers, TX/RX descriptor
//! rings, DMA, interrupt coalescing. Main differences:
//! - 2.5 Gbps PHY support (I225)
//! - Extended registers at higher offsets
//! - Slightly different reset sequence

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use alloc::collections::VecDeque;
use crate::drivers::pci::PciDevice;
use crate::memory::address::{PhysAddr, VirtAddr};
use crate::memory::{physical, virtual_mem};
use crate::sync::spinlock::Spinlock;

// ── Register Offsets (E1000-compatible) ─────────────────────────────────────

const REG_CTRL: u32     = 0x0000;
const REG_STATUS: u32   = 0x0008;
const REG_EERD: u32     = 0x0012;
const REG_ICR: u32      = 0x00C0;
const REG_IMS: u32      = 0x00D0;
const REG_IMC: u32      = 0x00D8;
const REG_RCTL: u32     = 0x0100;
const REG_TCTL: u32     = 0x0400;
const REG_TIPG: u32     = 0x0410;
const REG_RDBAL: u32    = 0x2800;
const REG_RDBAH: u32    = 0x2804;
const REG_RDLEN: u32    = 0x2808;
const REG_RDH: u32      = 0x2810;
const REG_RDT: u32      = 0x2818;
const REG_RDTR: u32     = 0x2820;
const REG_TDBAL: u32    = 0x3800;
const REG_TDBAH: u32    = 0x3804;
const REG_TDLEN: u32    = 0x3808;
const REG_TDH: u32      = 0x3810;
const REG_TDT: u32      = 0x3818;
const REG_RAL0: u32     = 0x5400;
const REG_RAH0: u32     = 0x5404;
const REG_MTA: u32      = 0x5200;

// Control register bits
const CTRL_SLU: u32     = 1 << 6;   // Set Link Up
const CTRL_RST: u32     = 1 << 26;  // Device Reset
const CTRL_PHY_RST: u32 = 1 << 31;  // PHY Reset

// RX Control bits
const RCTL_EN: u32      = 1 << 1;   // Receiver Enable
const RCTL_BAM: u32     = 1 << 15;  // Broadcast Accept Mode
const RCTL_BSIZE_2048: u32 = 0;     // Buffer Size 2048 bytes
const RCTL_SECRC: u32   = 1 << 26;  // Strip Ethernet CRC

// TX Control bits
const TCTL_EN: u32      = 1 << 1;   // Transmitter Enable
const TCTL_PSP: u32     = 1 << 3;   // Pad Short Packets

// Interrupt Cause bits
const ICR_TXDW: u32     = 1 << 0;
const ICR_LSC: u32      = 1 << 2;
const ICR_RXT0: u32     = 1 << 7;

// TX Descriptor command bits
const TDESC_CMD_EOP: u8  = 1 << 0;
const TDESC_CMD_IFCS: u8 = 1 << 1;
const TDESC_CMD_RS: u8   = 1 << 3;

// ── Descriptors ─────────────────────────────────────────────────────────────

const NUM_RX_DESC: usize = 256;
const NUM_TX_DESC: usize = 256;
const RX_BUF_SIZE: usize = 2048;

#[repr(C)]
struct RxDesc {
    buffer_addr: u64,
    length: u16,
    checksum: u16,
    status: u8,
    errors: u8,
    special: u16,
}

#[repr(C)]
struct TxDesc {
    buffer_addr: u64,
    length: u16,
    cso: u8,
    cmd: u8,
    status: u8,
    css: u8,
    special: u16,
}

// ── Driver State ────────────────────────────────────────────────────────────

struct IgcState {
    mmio: u64,
    mac: [u8; 6],
    rx_descs_phys: u64,
    rx_descs_virt: u64,
    rx_bufs_phys: [u64; NUM_RX_DESC],
    rx_tail: u16,
    tx_descs_phys: u64,
    tx_descs_virt: u64,
    tx_bufs_phys: [u64; NUM_TX_DESC],
    tx_bufs_virt: [u64; NUM_TX_DESC],
    tx_tail: u16,
    rx_queue: VecDeque<Vec<u8>>,
    irq: u8,
    rx_packets: u64,
    tx_packets: u64,
}

static IGC_STATE: Spinlock<Option<IgcState>> = Spinlock::new(None);

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
    if bar0 == 0 { return false; }

    // Enable bus mastering + memory space
    let cmd = crate::drivers::pci::pci_config_read32(pci.bus, pci.device, pci.function, 0x04);
    crate::drivers::pci::pci_config_write32(pci.bus, pci.device, pci.function, 0x04, cmd | 0x07);

    // Map MMIO (128 KiB for igc)
    let mmio = match virtual_mem::map_mmio(PhysAddr::new(bar0), 32) {
        Some(v) => v.as_u64(),
        None => return false,
    };

    let irq = pci.interrupt_line;

    unsafe {
        // Device reset
        wr(mmio, REG_CTRL, rd(mmio, REG_CTRL) | CTRL_RST);
        for _ in 0..100_000 { core::hint::spin_loop(); }
        // Set link up
        wr(mmio, REG_CTRL, rd(mmio, REG_CTRL) | CTRL_SLU);

        // Disable interrupts during setup
        wr(mmio, REG_IMC, 0xFFFF_FFFF);
        rd(mmio, REG_ICR); // Clear pending

        // Read MAC from RAL/RAH registers
        let ral = rd(mmio, REG_RAL0);
        let rah = rd(mmio, REG_RAH0);
        let mac = [
            (ral & 0xFF) as u8, ((ral >> 8) & 0xFF) as u8,
            ((ral >> 16) & 0xFF) as u8, ((ral >> 24) & 0xFF) as u8,
            (rah & 0xFF) as u8, ((rah >> 8) & 0xFF) as u8,
        ];

        // If MAC is all zeros/FF, try reading from EEPROM
        let mac = if mac == [0; 6] || mac == [0xFF; 6] {
            read_mac_eeprom(mmio)
        } else {
            mac
        };

        crate::serial_println!("  IGC: MAC {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]);

        // Clear MTA
        for i in 0..128u32 { wr(mmio, REG_MTA + i * 4, 0); }

        // Allocate RX ring
        let rx_frame = physical::alloc_frame().expect("IGC: RX ring");
        let rx_phys = rx_frame.as_u64();
        core::ptr::write_bytes(rx_phys as *mut u8, 0, 4096);

        let mut rx_bufs = [0u64; NUM_RX_DESC];
        for i in 0..NUM_RX_DESC {
            let buf = physical::alloc_frame().expect("IGC: RX buf");
            rx_bufs[i] = buf.as_u64();
            let desc = (rx_phys + (i * 16) as u64) as *mut RxDesc;
            (*desc).buffer_addr = rx_bufs[i];
            (*desc).status = 0;
        }

        // Allocate TX ring
        let tx_frame = physical::alloc_frame().expect("IGC: TX ring");
        let tx_phys = tx_frame.as_u64();
        core::ptr::write_bytes(tx_phys as *mut u8, 0, 4096);

        let mut tx_bufs_phys = [0u64; NUM_TX_DESC];
        let mut tx_bufs_virt = [0u64; NUM_TX_DESC];
        for i in 0..NUM_TX_DESC {
            let buf = physical::alloc_frame().expect("IGC: TX buf");
            tx_bufs_phys[i] = buf.as_u64();
            tx_bufs_virt[i] = buf.as_u64();
            let desc = (tx_phys + (i * 16) as u64) as *mut TxDesc;
            (*desc).buffer_addr = tx_bufs_phys[i];
            (*desc).status = 1; // DD=1 (available)
        }

        // Program RX
        wr(mmio, REG_RDBAL, rx_phys as u32);
        wr(mmio, REG_RDBAH, (rx_phys >> 32) as u32);
        wr(mmio, REG_RDLEN, (NUM_RX_DESC * 16) as u32);
        wr(mmio, REG_RDH, 0);
        wr(mmio, REG_RDT, (NUM_RX_DESC - 1) as u32);

        // Program TX
        wr(mmio, REG_TDBAL, tx_phys as u32);
        wr(mmio, REG_TDBAH, (tx_phys >> 32) as u32);
        wr(mmio, REG_TDLEN, (NUM_TX_DESC * 16) as u32);
        wr(mmio, REG_TDH, 0);
        wr(mmio, REG_TDT, 0);

        // IPG
        wr(mmio, REG_TIPG, 10 | (8 << 10) | (6 << 20));

        // Enable RX
        wr(mmio, REG_RCTL, RCTL_EN | RCTL_BAM | RCTL_BSIZE_2048 | RCTL_SECRC);

        // Enable TX
        wr(mmio, REG_TCTL, TCTL_EN | TCTL_PSP | (15 << 4) | (64 << 12));

        // Enable interrupts
        wr(mmio, REG_RDTR, 64); // RX coalescing 64µs
        wr(mmio, REG_IMS, ICR_RXT0 | ICR_LSC | ICR_TXDW);

        // Store state
        *IGC_STATE.lock() = Some(IgcState {
            mmio, mac,
            rx_descs_phys: rx_phys, rx_descs_virt: rx_phys,
            rx_bufs_phys: rx_bufs, rx_tail: 0,
            tx_descs_phys: tx_phys, tx_descs_virt: tx_phys,
            tx_bufs_phys, tx_bufs_virt, tx_tail: 0,
            rx_queue: VecDeque::new(), irq,
            rx_packets: 0, tx_packets: 0,
        });

        // Register IRQ
        if irq > 0 && irq < 32 {
            crate::arch::x86::irq::register_irq(irq, igc_irq_handler);
            if crate::arch::x86::apic::is_initialized() {
                crate::arch::x86::ioapic::unmask_irq(irq);
            }
        }

        let link = rd(mmio, REG_STATUS) & (1 << 1) != 0;
        crate::serial_println!("[OK] Intel IGC Ethernet (IRQ={}, link={})", irq, link);
    }

    super::register(Box::new(IgcDriver));
    true
}

unsafe fn read_mac_eeprom(mmio: u64) -> [u8; 6] {
    let mut mac = [0u8; 6];
    for i in 0..3u32 {
        wr(mmio, REG_EERD, (i << 8) | 1); // Start read
        for _ in 0..10_000 {
            let val = rd(mmio, REG_EERD);
            if val & (1 << 4) != 0 {
                let data = (val >> 16) as u16;
                mac[i as usize * 2] = (data & 0xFF) as u8;
                mac[i as usize * 2 + 1] = (data >> 8) as u8;
                break;
            }
            core::hint::spin_loop();
        }
    }
    mac
}

// ── TX / RX ─────────────────────────────────────────────────────────────────

fn transmit(data: &[u8]) -> bool {
    if data.is_empty() || data.len() > 1536 { return false; }
    let mut guard = IGC_STATE.lock();
    let igc = match guard.as_mut() { Some(s) => s, None => return false };
    let idx = igc.tx_tail as usize;
    let desc = unsafe { &mut *((igc.tx_descs_virt + (idx * 16) as u64) as *mut TxDesc) };
    if desc.status & 1 == 0 { return false; } // DD not set — busy
    unsafe {
        core::ptr::copy_nonoverlapping(data.as_ptr(), igc.tx_bufs_virt[idx] as *mut u8, data.len());
    }
    desc.length = data.len() as u16;
    desc.cmd = TDESC_CMD_EOP | TDESC_CMD_IFCS | TDESC_CMD_RS;
    desc.status = 0;
    igc.tx_tail = ((idx + 1) % NUM_TX_DESC) as u16;
    igc.tx_packets += 1;
    unsafe { wr(igc.mmio, REG_TDT, igc.tx_tail as u32); }
    true
}

fn process_rx(igc: &mut IgcState) {
    loop {
        let idx = ((igc.rx_tail + 1) % NUM_RX_DESC as u16) as usize;
        let desc = unsafe { &mut *((igc.rx_descs_virt + (idx * 16) as u64) as *mut RxDesc) };
        if desc.status & 1 == 0 { break; } // DD not set
        let len = desc.length as usize;
        if len > 0 && len <= RX_BUF_SIZE && desc.status & 2 != 0 { // EOP
            let mut pkt = vec![0u8; len];
            unsafe {
                core::ptr::copy_nonoverlapping(igc.rx_bufs_phys[idx] as *const u8, pkt.as_mut_ptr(), len);
            }
            if igc.rx_queue.len() < 1024 { igc.rx_queue.push_back(pkt); }
            igc.rx_packets += 1;
        }
        desc.status = 0;
        igc.rx_tail = idx as u16;
    }
    unsafe { wr(igc.mmio, REG_RDT, igc.rx_tail as u32); }
}

pub fn recv_packet() -> Option<Vec<u8>> {
    IGC_STATE.lock().as_mut()?.rx_queue.pop_front()
}

fn igc_irq_handler(_irq: u8) {
    if let Some(mut guard) = IGC_STATE.try_lock() {
        if let Some(igc) = guard.as_mut() {
            let icr = unsafe { rd(igc.mmio, REG_ICR) };
            if icr & ICR_RXT0 != 0 { process_rx(igc); }
        }
    }
    while let Some(pkt) = recv_packet() {
        crate::net::ethernet::handle_frame(&pkt);
    }
}

// ── NetworkDriver trait ─────────────────────────────────────────────────────

struct IgcDriver;
impl super::NetworkDriver for IgcDriver {
    fn name(&self) -> &str { "Intel I225" }
    fn transmit(&mut self, data: &[u8]) -> bool { transmit(data) }
    fn get_mac(&self) -> [u8; 6] { IGC_STATE.lock().as_ref().map(|s| s.mac).unwrap_or([0; 6]) }
    fn link_up(&self) -> bool {
        IGC_STATE.lock().as_ref().map(|s| unsafe { rd(s.mmio, REG_STATUS) & 2 != 0 }).unwrap_or(false)
    }
}

pub fn probe(pci: &PciDevice) -> Option<Box<dyn crate::drivers::hal::Driver>> {
    if init_and_register(pci) {
        super::create_hal_driver("Intel I225/I210 Ethernet")
    } else {
        None
    }
}
