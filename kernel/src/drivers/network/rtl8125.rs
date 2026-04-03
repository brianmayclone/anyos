//! Realtek RTL8125 2.5 Gigabit Ethernet driver.
//!
//! The RTL8125 is the successor to RTL8168 with 2.5 GbE support.
//! Found on most gaming motherboards manufactured since 2020.
//! PCI vendor 0x10EC, devices 0x8125 (RTL8125B), 0x3000 (RTL8125BG).
//!
//! Uses a similar architecture to RTL8168 but with larger descriptor rings,
//! extended register space, and 2.5 Gbps PHY. Register layout is mostly
//! compatible with RTL8168 for basic operation.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use alloc::collections::VecDeque;
use crate::drivers::pci::PciDevice;
use crate::memory::address::PhysAddr;
use crate::memory::{physical, virtual_mem};
use crate::sync::spinlock::Spinlock;

// ── Register Offsets (RTL8125-specific) ─────────────────────────────────────
// Mostly compatible with RTL8168, but some registers moved or extended.

const REG_MAC0: u32         = 0x00;
const REG_MAC4: u32         = 0x04;
const REG_TNPDS_LO: u32    = 0x20;
const REG_TNPDS_HI: u32    = 0x24;
const REG_CMD: u32          = 0x37;
const REG_TPPOLL: u32       = 0x38;
const REG_IMR0: u32         = 0x38;   // Interrupt Mask (RTL8125 uses 32-bit at 0x38 in new layout)
const REG_ISR0: u32         = 0x3C;   // Interrupt Status
// RTL8125 uses extended interrupt registers
const REG_IMR_8125: u32     = 0x0800; // RTL8125B Interrupt Mask Register
const REG_ISR_8125: u32     = 0x0804; // RTL8125B Interrupt Status Register
const REG_TX_CONFIG: u32    = 0x40;
const REG_RX_CONFIG: u32    = 0x44;
const REG_9346CR: u32       = 0x50;
const REG_PHY_STATUS: u32   = 0x6C;
const REG_RMS: u32          = 0xDA;
const REG_CPCR: u32         = 0xE0;
const REG_RDSAR_LO: u32     = 0xE4;
const REG_RDSAR_HI: u32     = 0xE8;
const REG_MTPS: u32         = 0xEC;

// RTL8125 extended TX descriptor address (for queue 0)
const REG_Q0_TNPDS_LO: u32 = 0x2000;
const REG_Q0_TNPDS_HI: u32 = 0x2004;
const REG_Q0_RDT: u32      = 0x2010; // TX doorbell

// Command register bits
const CMD_TX_ENABLE: u8     = 1 << 2;
const CMD_RX_ENABLE: u8     = 1 << 3;
const CMD_RESET: u8         = 1 << 4;

// Interrupt bits
const INT_ROK: u32          = 1 << 0;
const INT_TOK: u32          = 1 << 2;
const INT_LINK_CHG: u32     = 1 << 5;
const INT_RDU: u32          = 1 << 4;

// Descriptor bits
const TX_OWN: u32           = 1 << 31;
const TX_EOR: u32           = 1 << 30;
const TX_FS: u32            = 1 << 29;
const TX_LS: u32            = 1 << 28;
const RX_OWN: u32           = 1 << 31;
const RX_EOR: u32           = 1 << 30;
const PHY_LINK_STATUS: u32  = 1 << 1;

const NUM_RX_DESC: usize = 256;
const NUM_TX_DESC: usize = 256;
const RX_BUF_SIZE: usize = 2048;

#[repr(C)]
#[derive(Clone, Copy)]
struct RtlDescriptor {
    opts1: u32,
    opts2: u32,
    addr_lo: u32,
    addr_hi: u32,
}

struct Rtl8125 {
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
    use_extended_regs: bool,
}

static RTL8125_STATE: Spinlock<Option<Rtl8125>> = Spinlock::new(None);

#[inline(always)]
unsafe fn rd8(base: u64, reg: u32) -> u8 { core::ptr::read_volatile((base + reg as u64) as *const u8) }
#[inline(always)]
unsafe fn rd16(base: u64, reg: u32) -> u16 { core::ptr::read_volatile((base + reg as u64) as *const u16) }
#[inline(always)]
unsafe fn rd32(base: u64, reg: u32) -> u32 { core::ptr::read_volatile((base + reg as u64) as *const u32) }
#[inline(always)]
unsafe fn wr8(base: u64, reg: u32, v: u8) { core::ptr::write_volatile((base + reg as u64) as *mut u8, v); }
#[inline(always)]
unsafe fn wr16(base: u64, reg: u32, v: u16) { core::ptr::write_volatile((base + reg as u64) as *mut u16, v); }
#[inline(always)]
unsafe fn wr32(base: u64, reg: u32, v: u32) { core::ptr::write_volatile((base + reg as u64) as *mut u32, v); }

pub fn init_and_register(pci: &PciDevice) -> bool {
    let bar2 = (pci.bars[2] & !0xF) as u64;
    let bar0 = (pci.bars[0] & !0xF) as u64;
    let bar_phys = if bar2 != 0 { bar2 } else { bar0 };
    if bar_phys == 0 { return false; }

    let cmd = crate::drivers::pci::pci_config_read32(pci.bus, pci.device, pci.function, 0x04);
    crate::drivers::pci::pci_config_write32(pci.bus, pci.device, pci.function, 0x04, cmd | 0x07);

    // RTL8125 needs more MMIO space (extended registers at 0x0800+)
    let mmio_virt = match virtual_mem::map_mmio(PhysAddr::new(bar_phys), 16) {
        Some(v) => v.as_u64(),
        None => return false,
    };
    let irq = pci.interrupt_line;

    unsafe {
        wr8(mmio_virt, REG_CMD, CMD_RESET);
        for _ in 0..10_000 {
            if rd8(mmio_virt, REG_CMD) & CMD_RESET == 0 { break; }
            core::hint::spin_loop();
        }

        wr8(mmio_virt, REG_9346CR, 0xC0); // Unlock

        let mac_lo = rd32(mmio_virt, REG_MAC0);
        let mac_hi = rd16(mmio_virt, REG_MAC4);
        let mac = [
            (mac_lo & 0xFF) as u8, ((mac_lo >> 8) & 0xFF) as u8,
            ((mac_lo >> 16) & 0xFF) as u8, ((mac_lo >> 24) & 0xFF) as u8,
            (mac_hi & 0xFF) as u8, ((mac_hi >> 8) & 0xFF) as u8,
        ];

        crate::serial_println!("  RTL8125: MAC {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]);

        wr16(mmio_virt, REG_RMS, RX_BUF_SIZE as u16);
        wr8(mmio_virt, REG_MTPS, 0x3B);

        // Allocate rings (same as RTL8168)
        let rx_frame = physical::alloc_frame().expect("RTL8125: RX ring");
        let rx_phys = rx_frame.as_u64();
        core::ptr::write_bytes(rx_phys as *mut u8, 0, 4096);
        let mut rx_bufs = [0u64; NUM_RX_DESC];
        for i in 0..NUM_RX_DESC {
            let f = physical::alloc_frame().expect("RTL8125: RX buf");
            rx_bufs[i] = f.as_u64();
            let desc = (rx_phys + (i * 16) as u64) as *mut RtlDescriptor;
            (*desc).opts1 = RX_OWN | if i == NUM_RX_DESC - 1 { RX_EOR } else { 0 } | RX_BUF_SIZE as u32;
            (*desc).addr_lo = rx_bufs[i] as u32;
            (*desc).addr_hi = (rx_bufs[i] >> 32) as u32;
        }

        let tx_frame = physical::alloc_frame().expect("RTL8125: TX ring");
        let tx_phys = tx_frame.as_u64();
        core::ptr::write_bytes(tx_phys as *mut u8, 0, 4096);
        let mut tx_bufs_phys = [0u64; NUM_TX_DESC];
        let mut tx_bufs_virt = [0u64; NUM_TX_DESC];
        for i in 0..NUM_TX_DESC {
            let f = physical::alloc_frame().expect("RTL8125: TX buf");
            tx_bufs_phys[i] = f.as_u64();
            tx_bufs_virt[i] = f.as_u64();
            let desc = (tx_phys + (i * 16) as u64) as *mut RtlDescriptor;
            (*desc).opts1 = if i == NUM_TX_DESC - 1 { TX_EOR } else { 0 };
            (*desc).addr_lo = tx_bufs_phys[i] as u32;
            (*desc).addr_hi = (tx_bufs_phys[i] >> 32) as u32;
        }

        wr32(mmio_virt, REG_RDSAR_LO, rx_phys as u32);
        wr32(mmio_virt, REG_RDSAR_HI, (rx_phys >> 32) as u32);
        wr32(mmio_virt, REG_TNPDS_LO, tx_phys as u32);
        wr32(mmio_virt, REG_TNPDS_HI, (tx_phys >> 32) as u32);

        wr32(mmio_virt, REG_RX_CONFIG, 0x0000_E70F);
        wr32(mmio_virt, REG_TX_CONFIG, 0x0300_0700);

        let cpcr = rd16(mmio_virt, REG_CPCR);
        wr16(mmio_virt, REG_CPCR, cpcr | (1 << 5) | (1 << 6));

        wr8(mmio_virt, REG_CMD, CMD_TX_ENABLE | CMD_RX_ENABLE);

        // Use legacy interrupt registers (compatible mode)
        wr16(mmio_virt, 0x3C, (INT_ROK | INT_TOK | INT_LINK_CHG | INT_RDU) as u16);

        wr8(mmio_virt, REG_9346CR, 0x00); // Lock

        *RTL8125_STATE.lock() = Some(Rtl8125 {
            mmio_base: mmio_virt, mac,
            rx_descs_phys: rx_phys, rx_descs_virt: rx_phys, rx_bufs_phys: rx_bufs, rx_tail: 0,
            tx_descs_phys: tx_phys, tx_descs_virt: tx_phys, tx_bufs_phys, tx_bufs_virt, tx_tail: 0,
            rx_queue: VecDeque::new(), irq,
            use_extended_regs: false,
        });

        if irq > 0 && irq < 32 {
            crate::arch::x86::irq::register_irq(irq, rtl8125_irq);
            if crate::arch::x86::apic::is_initialized() {
                crate::arch::x86::ioapic::unmask_irq(irq);
            }
        }

        let link = rd32(mmio_virt, REG_PHY_STATUS) & PHY_LINK_STATUS != 0;
        crate::serial_println!("[OK] RTL8125 2.5GbE (IRQ={}, link={})", irq, link);
    }

    super::register(Box::new(Rtl8125Driver));
    true
}

fn transmit(data: &[u8]) -> bool {
    if data.is_empty() || data.len() > 1536 { return false; }
    let mut guard = RTL8125_STATE.lock();
    let rtl = match guard.as_mut() { Some(r) => r, None => return false };
    let idx = rtl.tx_tail as usize;
    let desc = unsafe { &mut *((rtl.tx_descs_virt + (idx * 16) as u64) as *mut RtlDescriptor) };
    if desc.opts1 & TX_OWN != 0 { return false; }
    unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), rtl.tx_bufs_virt[idx] as *mut u8, data.len()); }
    desc.opts1 = TX_OWN | TX_FS | TX_LS | if idx == NUM_TX_DESC - 1 { TX_EOR } else { 0 } | data.len() as u32;
    rtl.tx_tail = ((idx + 1) % NUM_TX_DESC) as u16;
    unsafe { wr8(rtl.mmio_base, REG_TPPOLL, 0x40); }
    true
}

fn process_rx(rtl: &mut Rtl8125) {
    for _ in 0..NUM_RX_DESC {
        let idx = rtl.rx_tail as usize;
        let desc = unsafe { &mut *((rtl.rx_descs_virt + (idx * 16) as u64) as *mut RtlDescriptor) };
        if desc.opts1 & RX_OWN != 0 { break; }
        let len = (desc.opts1 & 0x3FFF) as usize;
        if len > 0 && len <= RX_BUF_SIZE {
            let mut pkt = vec![0u8; len];
            unsafe { core::ptr::copy_nonoverlapping(rtl.rx_bufs_phys[idx] as *const u8, pkt.as_mut_ptr(), len); }
            if rtl.rx_queue.len() < 1024 { rtl.rx_queue.push_back(pkt); }
        }
        desc.opts1 = RX_OWN | if idx == NUM_RX_DESC - 1 { RX_EOR } else { 0 } | RX_BUF_SIZE as u32;
        rtl.rx_tail = ((idx + 1) % NUM_RX_DESC) as u16;
    }
}

fn recv_packet() -> Option<Vec<u8>> { RTL8125_STATE.lock().as_mut()?.rx_queue.pop_front() }

fn rtl8125_irq(_irq: u8) {
    if let Some(mut g) = RTL8125_STATE.try_lock() {
        if let Some(rtl) = g.as_mut() {
            let isr = unsafe { rd16(rtl.mmio_base, 0x3E) };
            if isr != 0 {
                unsafe { wr16(rtl.mmio_base, 0x3E, isr); }
                process_rx(rtl);
            }
        }
    }
    while let Some(pkt) = recv_packet() { crate::net::ethernet::handle_frame(&pkt); }
}

struct Rtl8125Driver;
impl super::NetworkDriver for Rtl8125Driver {
    fn name(&self) -> &str { "Realtek RTL8125" }
    fn transmit(&mut self, data: &[u8]) -> bool { transmit(data) }
    fn get_mac(&self) -> [u8; 6] { RTL8125_STATE.lock().as_ref().map(|r| r.mac).unwrap_or([0; 6]) }
    fn link_up(&self) -> bool {
        RTL8125_STATE.lock().as_ref().map(|r| unsafe { rd32(r.mmio_base, REG_PHY_STATUS) & PHY_LINK_STATUS != 0 }).unwrap_or(false)
    }
}

pub fn probe(pci: &PciDevice) -> Option<Box<dyn crate::drivers::hal::Driver>> {
    if init_and_register(pci) { super::create_hal_driver("Realtek RTL8125 2.5GbE") } else { None }
}
