//! Realtek RTL8111/8168 Gigabit Ethernet driver.
//!
//! Covers the most common Ethernet chipset family found in consumer hardware.
//! PCI vendor 0x10EC, devices 0x8168 (RTL8111B+), 0x8136 (RTL8101/8102),
//! 0x8161 (RTL8111C), 0x8167 (RTL8169SC).
//!
//! Uses MMIO registers (BAR2) for control, and descriptor rings for DMA.
//! Supports basic TX/RX with interrupt-driven receive.

use crate::drivers::pci::PciDevice;
use crate::memory::address::{PhysAddr, VirtAddr};
use crate::memory::{physical, virtual_mem};
use crate::sync::spinlock::Spinlock;
use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::vec;
use alloc::vec::Vec;

// ── MMIO Register Offsets ───────────────────────────────────────────────────

const REG_MAC0: u32 = 0x00; // MAC address bytes 0-3
const REG_MAC4: u32 = 0x04; // MAC address bytes 4-5
const REG_TNPDS_LO: u32 = 0x20; // TX Normal Priority Desc Start Addr (low)
const REG_TNPDS_HI: u32 = 0x24; // TX Normal Priority Desc Start Addr (high)
const REG_CMD: u32 = 0x37; // Command register (8-bit)
const REG_TPPOLL: u32 = 0x38; // TX Priority Poll (8-bit)
const REG_IMR: u32 = 0x3C; // Interrupt Mask Register (16-bit)
const REG_ISR: u32 = 0x3E; // Interrupt Status Register (16-bit)
const REG_TX_CONFIG: u32 = 0x40; // TX Configuration
const REG_RX_CONFIG: u32 = 0x44; // RX Configuration
const REG_9346CR: u32 = 0x50; // 93C46 Command Register
const REG_CONFIG1: u32 = 0x52; // Configuration Register 1
const REG_PHYAR: u32 = 0x60; // PHY Access Register
const REG_PHY_STATUS: u32 = 0x6C; // PHY Status
const REG_RMS: u32 = 0xDA; // RX Max Size (16-bit)
const REG_CPCR: u32 = 0xE0; // C+ Command Register (16-bit)
const REG_RDSAR_LO: u32 = 0xE4; // RX Desc Start Addr (low)
const REG_RDSAR_HI: u32 = 0xE8; // RX Desc Start Addr (high)
const REG_MTPS: u32 = 0xEC; // Max TX Packet Size (8-bit)

// Command register bits
const CMD_TX_ENABLE: u8 = 1 << 2;
const CMD_RX_ENABLE: u8 = 1 << 3;
const CMD_RESET: u8 = 1 << 4;

// Interrupt bits
const INT_ROK: u16 = 1 << 0; // RX OK
const INT_TOK: u16 = 1 << 2; // TX OK
const INT_LINK_CHG: u16 = 1 << 5; // Link Change
const INT_RDU: u16 = 1 << 4; // RX Descriptor Unavailable

// TX descriptor command bits
const TX_OWN: u32 = 1 << 31; // Owned by hardware
const TX_EOR: u32 = 1 << 30; // End Of Ring
const TX_FS: u32 = 1 << 29; // First Segment
const TX_LS: u32 = 1 << 28; // Last Segment

// RX descriptor status bits
const RX_OWN: u32 = 1 << 31; // Owned by hardware
const RX_EOR: u32 = 1 << 30; // End Of Ring

// PHY status bits
const PHY_LINK_STATUS: u32 = 1 << 1;

// ── Descriptor Rings ────────────────────────────────────────────────────────

const NUM_RX_DESC: usize = 256;
const NUM_TX_DESC: usize = 256;
const RX_BUF_SIZE: usize = 2048;

/// RTL8168 RX/TX descriptor (16 bytes each).
#[repr(C)]
#[derive(Clone, Copy)]
struct RtlDescriptor {
    opts1: u32,   // OWN, EOR, FS, LS, length
    opts2: u32,   // VLAN, checksum offload
    addr_lo: u32, // Buffer physical address (low)
    addr_hi: u32, // Buffer physical address (high)
}

// ── Driver State ────────────────────────────────────────────────────────────

struct Rtl8168 {
    mmio_base: u64,
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

static RTL_STATE: Spinlock<Option<Rtl8168>> = Spinlock::new(None);

// ── MMIO Helpers ────────────────────────────────────────────────────────────

#[inline(always)]
unsafe fn mmio_read8(base: u64, reg: u32) -> u8 {
    core::ptr::read_volatile((base + reg as u64) as *const u8)
}

#[inline(always)]
unsafe fn mmio_read16(base: u64, reg: u32) -> u16 {
    core::ptr::read_volatile((base + reg as u64) as *const u16)
}

#[inline(always)]
unsafe fn mmio_read32(base: u64, reg: u32) -> u32 {
    core::ptr::read_volatile((base + reg as u64) as *const u32)
}

#[inline(always)]
unsafe fn mmio_write8(base: u64, reg: u32, val: u8) {
    core::ptr::write_volatile((base + reg as u64) as *mut u8, val);
}

#[inline(always)]
unsafe fn mmio_write16(base: u64, reg: u32, val: u16) {
    core::ptr::write_volatile((base + reg as u64) as *mut u16, val);
}

#[inline(always)]
unsafe fn mmio_write32(base: u64, reg: u32, val: u32) {
    core::ptr::write_volatile((base + reg as u64) as *mut u32, val);
}

// ── Initialization ──────────────────────────────────────────────────────────

pub fn init_and_register(pci: &PciDevice) -> bool {
    // Get BAR — RTL8168 uses BAR2 (MMIO) or BAR1 depending on variant
    let bar2 = pci.bars[2] & !0xF;
    let bar1 = pci.bars[1] & !0xF;
    let bar_phys = if bar2 != 0 { bar2 as u64 } else { bar1 as u64 };
    if bar_phys == 0 {
        crate::serial_println!("  RTL8168: no MMIO BAR found");
        return false;
    }

    // Enable bus mastering + memory space
    let cmd = crate::drivers::pci::pci_config_read32(pci.bus, pci.device, pci.function, 0x04);
    crate::drivers::pci::pci_config_write32(pci.bus, pci.device, pci.function, 0x04, cmd | 0x07);

    // Map MMIO (4 KiB is enough for RTL8168 registers)
    let mmio_virt = match virtual_mem::map_mmio(PhysAddr::new(bar_phys), 4) {
        Some(v) => v.as_u64(),
        None => {
            crate::serial_println!("  RTL8168: MMIO mapping failed");
            return false;
        }
    };

    let irq = pci.interrupt_line;

    unsafe {
        // Software reset
        mmio_write8(mmio_virt, REG_CMD, CMD_RESET);
        for _ in 0..10_000 {
            if mmio_read8(mmio_virt, REG_CMD) & CMD_RESET == 0 {
                break;
            }
            core::hint::spin_loop();
        }

        // Unlock config registers (9346CR = 0xC0)
        mmio_write8(mmio_virt, REG_9346CR, 0xC0);

        // Read MAC address
        let mac_lo = mmio_read32(mmio_virt, REG_MAC0);
        let mac_hi = mmio_read16(mmio_virt, REG_MAC4);
        let mac = [
            (mac_lo & 0xFF) as u8,
            ((mac_lo >> 8) & 0xFF) as u8,
            ((mac_lo >> 16) & 0xFF) as u8,
            ((mac_lo >> 24) & 0xFF) as u8,
            (mac_hi & 0xFF) as u8,
            ((mac_hi >> 8) & 0xFF) as u8,
        ];

        crate::serial_println!(
            "  RTL8168: MAC {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            mac[0],
            mac[1],
            mac[2],
            mac[3],
            mac[4],
            mac[5]
        );

        // Set RX max packet size
        mmio_write16(mmio_virt, REG_RMS, RX_BUF_SIZE as u16);
        // Set TX max packet size (in units of 128 bytes, so 0x3B = 7552 bytes)
        mmio_write8(mmio_virt, REG_MTPS, 0x3B);

        // Allocate RX descriptor ring
        let rx_ring_frame = physical::alloc_frame().expect("RTL8168: RX ring alloc");
        let rx_descs_phys = rx_ring_frame.as_u64();
        let rx_descs_virt = rx_descs_phys; // identity-mapped
        core::ptr::write_bytes(rx_descs_virt as *mut u8, 0, 4096);

        // Allocate RX buffers
        let mut rx_bufs_phys = [0u64; NUM_RX_DESC];
        for i in 0..NUM_RX_DESC {
            let frame = physical::alloc_frame().expect("RTL8168: RX buf alloc");
            rx_bufs_phys[i] = frame.as_u64();

            let desc = (rx_descs_virt + (i * 16) as u64) as *mut RtlDescriptor;
            let is_last = i == NUM_RX_DESC - 1;
            (*desc).opts1 = RX_OWN | if is_last { RX_EOR } else { 0 } | (RX_BUF_SIZE as u32);
            (*desc).opts2 = 0;
            (*desc).addr_lo = rx_bufs_phys[i] as u32;
            (*desc).addr_hi = (rx_bufs_phys[i] >> 32) as u32;
        }

        // Allocate TX descriptor ring
        let tx_ring_frame = physical::alloc_frame().expect("RTL8168: TX ring alloc");
        let tx_descs_phys = tx_ring_frame.as_u64();
        let tx_descs_virt = tx_descs_phys;
        core::ptr::write_bytes(tx_descs_virt as *mut u8, 0, 4096);

        // Allocate TX buffers
        let mut tx_bufs_phys = [0u64; NUM_TX_DESC];
        let mut tx_bufs_virt = [0u64; NUM_TX_DESC];
        for i in 0..NUM_TX_DESC {
            let frame = physical::alloc_frame().expect("RTL8168: TX buf alloc");
            tx_bufs_phys[i] = frame.as_u64();
            tx_bufs_virt[i] = frame.as_u64(); // identity-mapped

            let desc = (tx_descs_virt + (i * 16) as u64) as *mut RtlDescriptor;
            let is_last = i == NUM_TX_DESC - 1;
            (*desc).opts1 = if is_last { TX_EOR } else { 0 }; // OWN=0 (host owns)
            (*desc).opts2 = 0;
            (*desc).addr_lo = tx_bufs_phys[i] as u32;
            (*desc).addr_hi = (tx_bufs_phys[i] >> 32) as u32;
        }

        // Program descriptor ring addresses
        mmio_write32(mmio_virt, REG_RDSAR_LO, rx_descs_phys as u32);
        mmio_write32(mmio_virt, REG_RDSAR_HI, (rx_descs_phys >> 32) as u32);
        mmio_write32(mmio_virt, REG_TNPDS_LO, tx_descs_phys as u32);
        mmio_write32(mmio_virt, REG_TNPDS_HI, (tx_descs_phys >> 32) as u32);

        // RX config: accept broadcast, accept physical match, no wrap
        mmio_write32(mmio_virt, REG_RX_CONFIG, 0x0000_E70F);
        // TX config: IFG standard, DMA burst
        mmio_write32(mmio_virt, REG_TX_CONFIG, 0x0300_0700);

        // C+ command: RX checksum offload, RX VLAN detagging
        let cpcr = mmio_read16(mmio_virt, REG_CPCR);
        mmio_write16(mmio_virt, REG_CPCR, cpcr | (1 << 5) | (1 << 6));

        // Enable TX and RX
        mmio_write8(mmio_virt, REG_CMD, CMD_TX_ENABLE | CMD_RX_ENABLE);

        // Enable interrupts: ROK, TOK, Link Change, RDU
        mmio_write16(
            mmio_virt,
            REG_IMR,
            INT_ROK | INT_TOK | INT_LINK_CHG | INT_RDU,
        );

        // Lock config registers
        mmio_write8(mmio_virt, REG_9346CR, 0x00);

        // Store state
        *RTL_STATE.lock() = Some(Rtl8168 {
            mmio_base: mmio_virt,
            mac,
            rx_descs_phys,
            rx_descs_virt,
            rx_bufs_phys,
            rx_tail: 0,
            tx_descs_phys,
            tx_descs_virt,
            tx_bufs_phys,
            tx_bufs_virt,
            tx_tail: 0,
            rx_queue: VecDeque::new(),
            irq,
            rx_packets: 0,
            tx_packets: 0,
        });

        // Register IRQ handler
        if irq > 0 && irq < 32 {
            crate::arch::x86::irq::register_irq(irq, rtl_irq_handler);
            if crate::arch::x86::apic::is_initialized() {
                crate::arch::x86::ioapic::unmask_irq(irq);
            } else {
                crate::arch::x86::pic::unmask(irq);
            }
        }

        // Check link status
        let phy = mmio_read32(mmio_virt, REG_PHY_STATUS);
        let link = phy & PHY_LINK_STATUS != 0;
        crate::serial_println!("[OK] RTL8168 Gigabit Ethernet (IRQ={}, link={})", irq, link);
    }

    // Register with network subsystem
    super::register(Box::new(Rtl8168Driver));
    true
}

// ── TX ──────────────────────────────────────────────────────────────────────

fn transmit(data: &[u8]) -> bool {
    if data.len() > 1536 || data.is_empty() {
        return false;
    }
    let mut state = RTL_STATE.lock();
    let rtl = match state.as_mut() {
        Some(r) => r,
        None => return false,
    };

    let idx = rtl.tx_tail as usize;
    let desc = unsafe { &mut *((rtl.tx_descs_virt + (idx * 16) as u64) as *mut RtlDescriptor) };

    // Check OWN bit — if set, hardware still owns this descriptor
    if desc.opts1 & TX_OWN != 0 {
        return false;
    }

    // Copy data to TX buffer
    let dst = rtl.tx_bufs_virt[idx] as *mut u8;
    unsafe {
        core::ptr::copy_nonoverlapping(data.as_ptr(), dst, data.len());
    }

    // Set descriptor
    let is_last = idx == NUM_TX_DESC - 1;
    desc.opts1 = TX_OWN | TX_FS | TX_LS | if is_last { TX_EOR } else { 0 } | (data.len() as u32);
    desc.opts2 = 0;

    rtl.tx_tail = ((idx + 1) % NUM_TX_DESC) as u16;
    rtl.tx_packets += 1;

    // Notify hardware
    unsafe {
        mmio_write8(rtl.mmio_base, REG_TPPOLL, 0x40);
    } // NPQ poll

    true
}

// ── RX ──────────────────────────────────────────────────────────────────────

fn process_rx(rtl: &mut Rtl8168) {
    for _ in 0..NUM_RX_DESC {
        let idx = rtl.rx_tail as usize;
        let desc = unsafe { &mut *((rtl.rx_descs_virt + (idx * 16) as u64) as *mut RtlDescriptor) };

        if desc.opts1 & RX_OWN != 0 {
            break;
        } // Hardware still owns it

        let len = (desc.opts1 & 0x3FFF) as usize;
        if len > 0 && len <= RX_BUF_SIZE {
            let src = rtl.rx_bufs_phys[idx] as *const u8;
            let mut pkt = vec![0u8; len];
            unsafe {
                core::ptr::copy_nonoverlapping(src, pkt.as_mut_ptr(), len);
            }
            if rtl.rx_queue.len() < 1024 {
                rtl.rx_queue.push_back(pkt);
            }
            rtl.rx_packets += 1;
        }

        // Return descriptor to hardware
        let is_last = idx == NUM_RX_DESC - 1;
        desc.opts1 = RX_OWN | if is_last { RX_EOR } else { 0 } | (RX_BUF_SIZE as u32);

        rtl.rx_tail = ((idx + 1) % NUM_RX_DESC) as u16;
    }
}

pub fn recv_packet() -> Option<Vec<u8>> {
    RTL_STATE.lock().as_mut()?.rx_queue.pop_front()
}

pub fn poll_rx() {
    if let Some(rtl) = RTL_STATE.lock().as_mut() {
        process_rx(rtl);
    }
}

// ── IRQ Handler ─────────────────────────────────────────────────────────────

fn rtl_irq_handler(_irq: u8) {
    if let Some(rtl) =
        RTL_STATE
            .try_lock()
            .and_then(|mut g| if g.is_some() { Some(g) } else { None })
    {
        // We need to use try_lock properly
        drop(rtl);
    }

    if let Some(mut guard) = RTL_STATE.try_lock() {
        if let Some(rtl) = guard.as_mut() {
            let isr = unsafe { mmio_read16(rtl.mmio_base, REG_ISR) };
            if isr == 0 {
                return;
            }
            // Acknowledge all interrupts
            unsafe {
                mmio_write16(rtl.mmio_base, REG_ISR, isr);
            }

            if isr & (INT_ROK | INT_RDU) != 0 {
                process_rx(rtl);
            }
        }
    }

    // Route packets to network stack
    while let Some(pkt) = recv_packet() {
        crate::net::ethernet::handle_frame(&pkt);
    }
}

// ── NetworkDriver trait ─────────────────────────────────────────────────────

pub struct Rtl8168Driver;

impl super::NetworkDriver for Rtl8168Driver {
    fn name(&self) -> &str {
        "Realtek RTL8168"
    }
    fn transmit(&mut self, data: &[u8]) -> bool {
        transmit(data)
    }
    fn get_mac(&self) -> [u8; 6] {
        RTL_STATE.lock().as_ref().map(|r| r.mac).unwrap_or([0; 6])
    }
    fn link_up(&self) -> bool {
        RTL_STATE
            .lock()
            .as_ref()
            .map(|r| unsafe { mmio_read32(r.mmio_base, REG_PHY_STATUS) & PHY_LINK_STATUS != 0 })
            .unwrap_or(false)
    }
}

pub fn is_available() -> bool {
    RTL_STATE.lock().is_some()
}

pub fn probe(pci: &PciDevice) -> Option<Box<dyn crate::drivers::hal::Driver>> {
    if init_and_register(pci) {
        super::create_hal_driver("Realtek RTL8168 Gigabit Ethernet")
    } else {
        None
    }
}
