//! Intel E1000 NIC driver (82540EM / 82545EM).
//!
//! MMIO-based Ethernet controller with DMA ring buffers for RX/TX.
//! Supports IRQ-driven and polled packet reception, transmit via descriptor rings,
//! and MAC address reading from EEPROM/RAL registers.
//! Supports: 82540EM (8086:100E, QEMU) and 82545EM (8086:100F, VMware).

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use crate::drivers::pci::PciDevice;
use crate::sync::spinlock::Spinlock;
use crate::memory::address::{PhysAddr, VirtAddr};
use crate::memory::{physical, virtual_mem, FRAME_SIZE};

// ──────────────────────────────────────────────
// E1000 Register Offsets
// ──────────────────────────────────────────────

const REG_CTRL: u32       = 0x0000; // Device Control
const REG_STATUS: u32     = 0x0008; // Device Status
const REG_EERD: u32       = 0x0014; // EEPROM Read
const REG_ICR: u32        = 0x00C0; // Interrupt Cause Read
const REG_IMS: u32        = 0x00D0; // Interrupt Mask Set
const REG_IMC: u32        = 0x00D8; // Interrupt Mask Clear
const REG_RCTL: u32       = 0x0100; // Receive Control
const REG_RDBAL: u32      = 0x2800; // RX Descriptor Base Low
const REG_RDBAH: u32      = 0x2804; // RX Descriptor Base High
const REG_RDLEN: u32      = 0x2808; // RX Descriptor Length
const REG_RDH: u32        = 0x2810; // RX Descriptor Head
const REG_RDT: u32        = 0x2818; // RX Descriptor Tail
const REG_TCTL: u32       = 0x0400; // Transmit Control
const REG_TDBAL: u32      = 0x3800; // TX Descriptor Base Low
const REG_TDBAH: u32      = 0x3804; // TX Descriptor Base High
const REG_TDLEN: u32      = 0x3808; // TX Descriptor Length
const REG_TDH: u32        = 0x3810; // TX Descriptor Head
const REG_TDT: u32        = 0x3818; // TX Descriptor Tail
const REG_RAL0: u32       = 0x5400; // Receive Address Low (MAC bytes 0-3)
const REG_RAH0: u32       = 0x5404; // Receive Address High (MAC bytes 4-5 + flags)
const REG_MTA: u32        = 0x5200; // Multicast Table Array (128 u32s)
const REG_TIPG: u32       = 0x0410; // Transmit IPG

// CTRL bits
const CTRL_SLU: u32       = 1 << 6;  // Set Link Up
const CTRL_RST: u32       = 1 << 26; // Device Reset

// RCTL bits
const RCTL_EN: u32        = 1 << 1;  // Receiver Enable
const RCTL_BAM: u32       = 1 << 15; // Broadcast Accept Mode
const RCTL_BSIZE_2048: u32 = 0;      // Buffer size 2048 (bits 16-17 = 00)
const RCTL_SECRC: u32     = 1 << 26; // Strip Ethernet CRC

// TCTL bits
const TCTL_EN: u32        = 1 << 1;  // Transmit Enable
const TCTL_PSP: u32       = 1 << 3;  // Pad Short Packets
const TCTL_CT_SHIFT: u32  = 4;       // Collision Threshold shift
const TCTL_COLD_SHIFT: u32 = 12;     // Collision Distance shift

// ICR / IMS bits
const ICR_TXDW: u32       = 1 << 0;  // TX Descriptor Written Back
const ICR_RXT0: u32       = 1 << 7;  // RX Timer Interrupt
const ICR_LSC: u32        = 1 << 2;  // Link Status Change

// TX descriptor command bits
const TDESC_CMD_EOP: u8   = 1 << 0;  // End of Packet
const TDESC_CMD_IFCS: u8  = 1 << 1;  // Insert FCS
const TDESC_CMD_RS: u8    = 1 << 3;  // Report Status

// TX descriptor status bits
const TDESC_STA_DD: u8    = 1 << 0;  // Descriptor Done

// RX descriptor status bits
const RDESC_STA_DD: u8    = 1 << 0;  // Descriptor Done
const RDESC_STA_EOP: u8   = 1 << 1;  // End of Packet

// ──────────────────────────────────────────────
// DMA Descriptors
// Each descriptor is exactly 16 bytes with repr(C), no padding needed.
// ──────────────────────────────────────────────

/// Number of receive descriptors in the RX ring.
/// 256 descriptors prevents ring starvation during burst traffic.
const NUM_RX_DESC: usize = 256;
/// Number of transmit descriptors in the TX ring.
/// 256 descriptors allows batching many segments with a single tail update.
const NUM_TX_DESC: usize = 256;
/// Size of each receive buffer in bytes.
const RX_BUFFER_SIZE: usize = 2048;

#[repr(C)]
#[derive(Clone, Copy)]
struct RxDescriptor {
    buffer_addr: u64,  // Physical address of receive buffer
    length: u16,       // Length of received data
    checksum: u16,     // Packet checksum
    status: u8,        // Status bits
    errors: u8,        // Error bits
    special: u16,      // VLAN tag
}

#[repr(C)]
#[derive(Clone, Copy)]
struct TxDescriptor {
    buffer_addr: u64,  // Physical address of transmit buffer
    length: u16,       // Data length
    cso: u8,           // Checksum offset
    cmd: u8,           // Command field
    status: u8,        // Status bits
    css: u8,           // Checksum start
    special: u16,      // Special field
}

/// 6-byte MAC address type alias.
pub type MacBytes = [u8; 6];

// ──────────────────────────────────────────────
// E1000 Driver State
// ──────────────────────────────────────────────

/// Virtual address where the E1000 MMIO region is mapped (128 KiB).
const E1000_MMIO_VIRT: u64 = 0xFFFF_FFFF_D000_0000;

struct E1000 {
    mmio_base: u64,       // Virtual address of MMIO region
    mac: [u8; 6],

    // RX ring
    rx_descs_phys: u32,   // Physical address of RX descriptor ring (32-bit DMA)
    rx_descs_virt: u64,   // Virtual address of RX descriptor ring
    rx_bufs_phys: [u32; NUM_RX_DESC], // Physical addr of each RX buffer (32-bit DMA)
    rx_tail: u16,

    // TX ring
    tx_descs_phys: u32,   // Physical address of TX descriptor ring (32-bit DMA)
    tx_descs_virt: u64,   // Virtual address of TX descriptor ring
    tx_bufs_phys: [u32; NUM_TX_DESC], // Physical addr of each TX buffer (32-bit DMA)
    tx_bufs_virt: [u64; NUM_TX_DESC], // Virtual addr of each TX buffer (for memcpy)
    tx_tail: u16,

    // Received packets queue
    rx_queue: VecDeque<Vec<u8>>,

    // IRQ line
    irq: u8,

    // Statistics
    rx_packets: u64,
    tx_packets: u64,
    rx_bytes: u64,
    tx_bytes: u64,
    rx_errors: u64,
    tx_errors: u64,
}

static E1000_STATE: Spinlock<Option<E1000>> = Spinlock::new(None);

// ──────────────────────────────────────────────
// MMIO helpers
// ──────────────────────────────────────────────

unsafe fn mmio_read(base: u64, reg: u32) -> u32 {
    let addr = (base + reg as u64) as *const u32;
    core::ptr::read_volatile(addr)
}

unsafe fn mmio_write(base: u64, reg: u32, value: u32) {
    let addr = (base + reg as u64) as *mut u32;
    core::ptr::write_volatile(addr, value);
}

// ──────────────────────────────────────────────
// Public API
// ──────────────────────────────────────────────

/// Initialize the E1000 NIC. Call after PCI scan, heap init, and virtual memory init.
pub fn init() -> bool {
    // Find the E1000 on the PCI bus:
    //   82540EM (8086:100E) — QEMU default
    //   82545EM (8086:100F) — VMware Workstation default
    let pci_dev = match crate::drivers::pci::find_by_id(0x8086, 0x100E)
        .or_else(|| crate::drivers::pci::find_by_id(0x8086, 0x100F))
    {
        Some(dev) => dev,
        None => {
            crate::serial_println!("  E1000: device not found on PCI bus");
            return false;
        }
    };

    crate::serial_println!("  E1000: found at PCI {:02x}:{:02x}.{}",
        pci_dev.bus, pci_dev.device, pci_dev.function);

    // Enable bus mastering for DMA
    crate::drivers::pci::enable_bus_master(&pci_dev);

    // Get BAR0 (MMIO base physical address)
    let bar0 = pci_dev.bars[0] & 0xFFFFFFF0;
    if bar0 == 0 {
        crate::serial_println!("  E1000: BAR0 is zero, cannot map MMIO");
        return false;
    }
    crate::serial_println!("  E1000: BAR0 = {:#010x}", bar0);

    // Map MMIO region (128 KiB should be enough for E1000 registers)
    let mmio_virt = E1000_MMIO_VIRT;
    let mmio_pages = 32; // 128 KiB = 32 pages
    for i in 0..mmio_pages {
        let phys = PhysAddr::new(bar0 as u64 + (i as u64) * FRAME_SIZE as u64);
        let virt = VirtAddr::new(mmio_virt + (i as u64) * FRAME_SIZE as u64);
        virtual_mem::map_page(virt, phys, 0x03); // Present + Writable, no cache
    }

    // Get IRQ line from PCI config
    let irq = pci_dev.interrupt_line;
    crate::serial_println!("  E1000: IRQ = {}", irq);

    // --- Device Reset ---
    unsafe {
        let ctrl = mmio_read(mmio_virt, REG_CTRL);
        mmio_write(mmio_virt, REG_CTRL, ctrl | CTRL_RST);
        // Wait for reset to complete (busy loop ~10ms)
        for _ in 0..100_000 {
            core::hint::spin_loop();
        }
        // Set Link Up, clear reset
        let ctrl = mmio_read(mmio_virt, REG_CTRL);
        mmio_write(mmio_virt, REG_CTRL, (ctrl & !CTRL_RST) | CTRL_SLU);
    }

    // Small delay for link
    for _ in 0..100_000 {
        core::hint::spin_loop();
    }

    // --- Read MAC address from RAL0/RAH0 ---
    let mac: [u8; 6] = unsafe {
        let ral = mmio_read(mmio_virt, REG_RAL0);
        let rah = mmio_read(mmio_virt, REG_RAH0);
        [
            (ral & 0xFF) as u8,
            ((ral >> 8) & 0xFF) as u8,
            ((ral >> 16) & 0xFF) as u8,
            ((ral >> 24) & 0xFF) as u8,
            (rah & 0xFF) as u8,
            ((rah >> 8) & 0xFF) as u8,
        ]
    };
    crate::serial_println!("  E1000: MAC = {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]);

    // --- Clear Multicast Table Array ---
    unsafe {
        for i in 0..128 {
            mmio_write(mmio_virt, REG_MTA + i * 4, 0);
        }
    }

    // --- Disable all interrupts during setup ---
    unsafe {
        mmio_write(mmio_virt, REG_IMC, 0xFFFFFFFF);
        mmio_read(mmio_virt, REG_ICR); // Clear pending
    }

    // --- Allocate RX descriptor ring and buffers ---
    // We need physically contiguous memory for the descriptor ring.
    // Each RxDescriptor is 16 bytes. 32 descriptors = 512 bytes = fits in one page.
    let rx_desc_frame = physical::alloc_frame().expect("E1000: failed to alloc RX desc ring");
    let rx_descs_phys = rx_desc_frame.as_u32();
    // Identity-mapped region (< 8MiB) so virt == phys for these DMA pages
    let rx_descs_virt = rx_desc_frame.as_u64();

    // Zero out the descriptor ring
    unsafe {
        core::ptr::write_bytes(rx_descs_virt as *mut u8, 0, FRAME_SIZE);
    }

    // Allocate RX buffers (one 4K page per buffer, each buffer is 2048 bytes)
    let mut rx_bufs_phys = [0u32; NUM_RX_DESC];
    for i in 0..NUM_RX_DESC {
        let buf_frame = physical::alloc_frame().expect("E1000: failed to alloc RX buffer");
        rx_bufs_phys[i] = buf_frame.as_u32();
        // Zero the buffer
        unsafe {
            core::ptr::write_bytes(buf_frame.as_u64() as *mut u8, 0, FRAME_SIZE);
        }
        // Write descriptor
        unsafe {
            let desc_ptr = (rx_descs_virt as *mut RxDescriptor).add(i);
            (*desc_ptr).buffer_addr = buf_frame.as_u64();
            (*desc_ptr).status = 0;
        }
    }

    // --- Allocate TX descriptor ring and buffers ---
    let tx_desc_frame = physical::alloc_frame().expect("E1000: failed to alloc TX desc ring");
    let tx_descs_phys = tx_desc_frame.as_u32();
    let tx_descs_virt = tx_desc_frame.as_u64();

    unsafe {
        core::ptr::write_bytes(tx_descs_virt as *mut u8, 0, FRAME_SIZE);
    }

    let mut tx_bufs_phys = [0u32; NUM_TX_DESC];
    let mut tx_bufs_virt = [0u64; NUM_TX_DESC];
    for i in 0..NUM_TX_DESC {
        let buf_frame = physical::alloc_frame().expect("E1000: failed to alloc TX buffer");
        tx_bufs_phys[i] = buf_frame.as_u32();
        tx_bufs_virt[i] = buf_frame.as_u64(); // Identity-mapped
        unsafe {
            core::ptr::write_bytes(buf_frame.as_u64() as *mut u8, 0, FRAME_SIZE);
            let desc_ptr = (tx_descs_virt as *mut TxDescriptor).add(i);
            (*desc_ptr).buffer_addr = buf_frame.as_u64();
            (*desc_ptr).status = TDESC_STA_DD; // Mark as done (available for use)
            (*desc_ptr).cmd = 0;
        }
    }

    // --- Program RX ring registers ---
    unsafe {
        mmio_write(mmio_virt, REG_RDBAL, rx_descs_phys);
        mmio_write(mmio_virt, REG_RDBAH, 0);
        mmio_write(mmio_virt, REG_RDLEN, (NUM_RX_DESC * core::mem::size_of::<RxDescriptor>()) as u32);
        mmio_write(mmio_virt, REG_RDH, 0);
        mmio_write(mmio_virt, REG_RDT, (NUM_RX_DESC - 1) as u32);
    }

    // --- Program TX ring registers ---
    unsafe {
        mmio_write(mmio_virt, REG_TDBAL, tx_descs_phys);
        mmio_write(mmio_virt, REG_TDBAH, 0);
        mmio_write(mmio_virt, REG_TDLEN, (NUM_TX_DESC * core::mem::size_of::<TxDescriptor>()) as u32);
        mmio_write(mmio_virt, REG_TDH, 0);
        mmio_write(mmio_virt, REG_TDT, 0);
    }

    // --- Set Transmit IPG (Inter Packet Gap) ---
    unsafe {
        // Standard values: IPGT=10, IPGR1=8, IPGR2=6
        mmio_write(mmio_virt, REG_TIPG, 10 | (8 << 10) | (6 << 20));
    }

    // --- Enable RX ---
    unsafe {
        mmio_write(mmio_virt, REG_RCTL,
            RCTL_EN | RCTL_BAM | RCTL_BSIZE_2048 | RCTL_SECRC);
    }

    // --- Enable TX ---
    unsafe {
        mmio_write(mmio_virt, REG_TCTL,
            TCTL_EN | TCTL_PSP
            | (15 << TCTL_CT_SHIFT)    // Collision Threshold
            | (64 << TCTL_COLD_SHIFT)  // Collision Distance (full duplex)
        );
    }

    // --- Enable interrupts ---
    unsafe {
        mmio_read(mmio_virt, REG_ICR); // Clear any pending
        mmio_write(mmio_virt, REG_IMS, ICR_RXT0 | ICR_LSC | ICR_TXDW);
    }

    // Store driver state
    let e1000 = E1000 {
        mmio_base: mmio_virt,
        mac,
        rx_descs_phys: rx_descs_phys,
        rx_descs_virt: rx_descs_virt,
        rx_bufs_phys,
        rx_tail: (NUM_RX_DESC - 1) as u16,
        tx_descs_phys: tx_descs_phys,
        tx_descs_virt: tx_descs_virt,
        tx_bufs_phys,
        tx_bufs_virt,
        tx_tail: 0,
        rx_queue: VecDeque::new(),
        irq,
        rx_packets: 0,
        tx_packets: 0,
        rx_bytes: 0,
        tx_bytes: 0,
        rx_errors: 0,
        tx_errors: 0,
    };

    {
        let mut state = E1000_STATE.lock();
        *state = Some(e1000);
    }

    // Register IRQ handler
    crate::arch::x86::irq::register_irq(irq, e1000_irq_handler);
    if crate::arch::x86::apic::is_initialized() {
        crate::arch::x86::ioapic::unmask_irq(irq);
    } else {
        crate::arch::x86::pic::unmask(irq);
    }

    // Check link status
    let link_up = unsafe { mmio_read(mmio_virt, REG_STATUS) & 2 != 0 };
    crate::serial_println!("  E1000: link {}", if link_up { "UP" } else { "DOWN" });
    crate::serial_println!("[OK] E1000 NIC initialized ({} RX + {} TX descriptors)",
        NUM_RX_DESC, NUM_TX_DESC);

    // Register with the generic network subsystem
    super::register(Box::new(E1000NetworkDriver));

    true
}

/// Transmit a raw Ethernet frame (including Ethernet header).
/// Returns true on success.
pub fn transmit(data: &[u8]) -> bool {
    if data.len() > RX_BUFFER_SIZE {
        return false;
    }

    let mut state = E1000_STATE.lock();
    let e1000 = match state.as_mut() {
        Some(e) => e,
        None => return false,
    };

    let idx = e1000.tx_tail as usize;
    let desc_ptr = (e1000.tx_descs_virt as *mut TxDescriptor).wrapping_add(idx);

    // Check if descriptor is available (DD bit set means hardware is done with it)
    let status = unsafe { core::ptr::read_volatile(&(*desc_ptr).status) };
    if status & TDESC_STA_DD == 0 {
        // Descriptor not yet processed by hardware
        return false;
    }

    // Copy data to TX buffer
    let buf_virt = e1000.tx_bufs_virt[idx] as *mut u8;
    unsafe {
        core::ptr::copy_nonoverlapping(data.as_ptr(), buf_virt, data.len());
    }

    // Update descriptor
    unsafe {
        (*desc_ptr).length = data.len() as u16;
        (*desc_ptr).cmd = TDESC_CMD_EOP | TDESC_CMD_IFCS | TDESC_CMD_RS;
        (*desc_ptr).status = 0; // Clear DD
    }

    // Statistics
    e1000.tx_packets += 1;
    e1000.tx_bytes += data.len() as u64;

    // Advance tail
    e1000.tx_tail = ((idx + 1) % NUM_TX_DESC) as u16;
    unsafe {
        mmio_write(e1000.mmio_base, REG_TDT, e1000.tx_tail as u32);
    }

    true
}

/// Transmit multiple Ethernet frames in a single batch (one MMIO tail write).
///
/// Takes a slice of frame slices. Returns the number of frames successfully queued.
/// This is significantly faster than calling `transmit()` in a loop because it
/// avoids per-frame lock acquisition and MMIO tail register writes.
pub fn transmit_batch(frames: &[&[u8]]) -> usize {
    if frames.is_empty() {
        return 0;
    }

    let mut state = E1000_STATE.lock();
    let e1000 = match state.as_mut() {
        Some(e) => e,
        None => return 0,
    };

    let mut queued = 0usize;

    for frame in frames {
        if frame.len() > RX_BUFFER_SIZE || frame.is_empty() {
            continue;
        }

        let idx = e1000.tx_tail as usize;
        let desc_ptr = (e1000.tx_descs_virt as *mut TxDescriptor).wrapping_add(idx);

        // Check if descriptor is available
        let status = unsafe { core::ptr::read_volatile(&(*desc_ptr).status) };
        if status & TDESC_STA_DD == 0 {
            break; // No more available descriptors
        }

        // Copy data to TX buffer
        let buf_virt = e1000.tx_bufs_virt[idx] as *mut u8;
        unsafe {
            core::ptr::copy_nonoverlapping(frame.as_ptr(), buf_virt, frame.len());
        }

        // Update descriptor
        unsafe {
            (*desc_ptr).length = frame.len() as u16;
            (*desc_ptr).cmd = TDESC_CMD_EOP | TDESC_CMD_IFCS | TDESC_CMD_RS;
            (*desc_ptr).status = 0;
        }

        // Statistics
        e1000.tx_packets += 1;
        e1000.tx_bytes += frame.len() as u64;

        // Advance tail (but don't write MMIO yet)
        e1000.tx_tail = ((idx + 1) % NUM_TX_DESC) as u16;
        queued += 1;
    }

    // Single MMIO write to kick off all queued frames at once.
    if queued > 0 {
        unsafe {
            mmio_write(e1000.mmio_base, REG_TDT, e1000.tx_tail as u32);
        }
    }

    queued
}

/// Dequeue a received packet. Returns None if no packets available.
pub fn recv_packet() -> Option<Vec<u8>> {
    let mut state = E1000_STATE.lock();
    let e1000 = state.as_mut()?;
    e1000.rx_queue.pop_front()
}

/// Get the MAC address of the NIC.
pub fn get_mac() -> Option<[u8; 6]> {
    let state = E1000_STATE.lock();
    state.as_ref().map(|e| e.mac)
}

/// Disable the E1000 NIC (stop RX/TX). Returns true if state changed.
pub fn set_enabled(enabled: bool) -> bool {
    let state = E1000_STATE.lock();
    if let Some(e) = state.as_ref() {
        unsafe {
            if enabled {
                // Re-enable RX and TX
                let rctl = mmio_read(e.mmio_base, REG_RCTL);
                mmio_write(e.mmio_base, REG_RCTL, rctl | RCTL_EN);
                let tctl = mmio_read(e.mmio_base, REG_TCTL);
                mmio_write(e.mmio_base, REG_TCTL, tctl | TCTL_EN);
            } else {
                // Disable RX and TX
                let rctl = mmio_read(e.mmio_base, REG_RCTL);
                mmio_write(e.mmio_base, REG_RCTL, rctl & !RCTL_EN);
                let tctl = mmio_read(e.mmio_base, REG_TCTL);
                mmio_write(e.mmio_base, REG_TCTL, tctl & !TCTL_EN);
            }
        }
        true
    } else {
        false
    }
}

/// Check if the E1000 NIC RX is enabled.
pub fn is_enabled() -> bool {
    let state = E1000_STATE.lock();
    if let Some(e) = state.as_ref() {
        unsafe { mmio_read(e.mmio_base, REG_RCTL) & RCTL_EN != 0 }
    } else {
        false
    }
}

/// Check if E1000 hardware was detected and initialized.
pub fn is_available() -> bool {
    E1000_STATE.lock().is_some()
}

/// Check if the E1000 is initialized and link is up.
pub fn is_link_up() -> bool {
    let state = E1000_STATE.lock();
    if let Some(e) = state.as_ref() {
        unsafe { mmio_read(e.mmio_base, REG_STATUS) & 2 != 0 }
    } else {
        false
    }
}

/// Get NIC statistics: (rx_packets, tx_packets, rx_bytes, tx_bytes, rx_errors, tx_errors).
pub fn get_stats() -> (u64, u64, u64, u64, u64, u64) {
    let state = E1000_STATE.lock();
    if let Some(e) = state.as_ref() {
        (e.rx_packets, e.tx_packets, e.rx_bytes, e.tx_bytes, e.rx_errors, e.tx_errors)
    } else {
        (0, 0, 0, 0, 0, 0)
    }
}

/// Poll for received packets (non-interrupt driven).
/// Call this to process any packets that arrived since last check.
pub fn poll_rx() {
    let mut state = E1000_STATE.lock();
    let e1000 = match state.as_mut() {
        Some(e) => e,
        None => return,
    };
    process_rx_ring(e1000);
}

// ──────────────────────────────────────────────
// Internal: RX ring processing
// ──────────────────────────────────────────────

fn process_rx_ring(e1000: &mut E1000) {
    loop {
        let idx = ((e1000.rx_tail + 1) % NUM_RX_DESC as u16) as usize;
        let desc_ptr = (e1000.rx_descs_virt as *mut RxDescriptor).wrapping_add(idx);

        let status = unsafe { core::ptr::read_volatile(&(*desc_ptr).status) };
        if status & RDESC_STA_DD == 0 {
            // No more completed descriptors
            break;
        }

        // Read the packet
        let length = unsafe { core::ptr::read_volatile(&(*desc_ptr).length) } as usize;
        if length > 0 && length <= RX_BUFFER_SIZE && (status & RDESC_STA_EOP != 0) {
            let buf_phys = e1000.rx_bufs_phys[idx] as u64;
            let buf_ptr = buf_phys as *const u8; // Identity-mapped

            let mut packet = Vec::with_capacity(length);
            unsafe {
                packet.set_len(length);
                core::ptr::copy_nonoverlapping(buf_ptr, packet.as_mut_ptr(), length);
            }

            // Statistics
            e1000.rx_packets += 1;
            e1000.rx_bytes += length as u64;

            // Don't grow the queue unboundedly
            if e1000.rx_queue.len() < 256 {
                e1000.rx_queue.push_back(packet);
            }
        } else if length > 0 {
            e1000.rx_errors += 1;
        }

        // Reset descriptor for reuse
        unsafe {
            (*desc_ptr).status = 0;
        }

        // Advance tail
        e1000.rx_tail = idx as u16;
        unsafe {
            mmio_write(e1000.mmio_base, REG_RDT, e1000.rx_tail as u32);
        }
    }
}

// ──────────────────────────────────────────────
// IRQ Handler
// ──────────────────────────────────────────────

fn e1000_irq_handler(_irq: u8) {
    let mut has_rx = false;

    // Use try_lock to avoid deadlock if we're already holding the lock
    if let Some(mut state) = E1000_STATE.try_lock() {
        if let Some(e1000) = state.as_mut() {
            // Read and acknowledge interrupt cause
            let icr = unsafe { mmio_read(e1000.mmio_base, REG_ICR) };

            if icr & ICR_LSC != 0 {
                let link = unsafe { mmio_read(e1000.mmio_base, REG_STATUS) & 2 != 0 };
                crate::serial_println!("  E1000: link status changed: {}",
                    if link { "UP" } else { "DOWN" });
            }

            if icr & ICR_RXT0 != 0 {
                process_rx_ring(e1000);
                has_rx = !e1000.rx_queue.is_empty();
            }
        }
    }
    // E1000_STATE lock dropped here

    // Process received packets through the network stack (Ethernet → IP → TCP).
    // This wakes any threads blocked on tcp::recv/accept/connect.
    if has_rx {
        crate::net::poll();
    }
}

// ── NetworkDriver trait implementation ──────────────────────────────────────

/// Thin wrapper that delegates to the E1000 static state.
pub struct E1000NetworkDriver;

impl super::NetworkDriver for E1000NetworkDriver {
    fn name(&self) -> &str { "Intel E1000" }
    fn transmit(&mut self, data: &[u8]) -> bool { transmit(data) }
    fn get_mac(&self) -> [u8; 6] { get_mac().unwrap_or([0; 6]) }
    fn link_up(&self) -> bool { is_link_up() }
}

/// Probe: return a HAL driver wrapper for the E1000 Ethernet controller.
pub fn probe(_pci: &PciDevice) -> Option<Box<dyn crate::drivers::hal::Driver>> {
    super::create_hal_driver("Intel E1000 Ethernet")
}
