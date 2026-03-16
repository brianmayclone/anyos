//! Intel E1000 network card emulation.
//!
//! Emulates a simplified Intel 82540EM Gigabit Ethernet controller
//! (PCI device 8086:100E) using MMIO-based register access.
//!
//! # EEPROM Access
//!
//! The 82540EM uses Microwire bit-banging via the EECD register for EEPROM
//! access. The driver clocks in a 3-bit opcode + 6-bit address via DI/SK,
//! then clocks out 16 data bits via DO/SK. We also support EERD for newer
//! drivers.
//!
//! # MMIO Layout
//!
//! The E1000 exposes a 128 KB MMIO register space. Key register offsets:
//!
//! | Offset | Name | Description |
//! |--------|------|-------------|
//! | 0x0000 | CTRL | Device Control |
//! | 0x0008 | STATUS | Device Status |
//! | 0x0010 | EECD | EEPROM/Flash Control |
//! | 0x0014 | EERD | EEPROM Read |
//! | 0x00C0 | ICR | Interrupt Cause Read |
//! | 0x00C4 | ICS | Interrupt Cause Set |
//! | 0x00C8 | IMS | Interrupt Mask Set |
//! | 0x00CC | IMC | Interrupt Mask Clear |
//! | 0x0100 | RCTL | Receive Control |
//! | 0x0400 | TCTL | Transmit Control |
//! | 0x2800 | RDBAL | RX Descriptor Base Low |
//! | 0x2804 | RDBAH | RX Descriptor Base High |
//! | 0x2808 | RDLEN | RX Descriptor Ring Length |
//! | 0x2810 | RDH | RX Descriptor Head |
//! | 0x2818 | RDT | RX Descriptor Tail |
//! | 0x3800 | TDBAL | TX Descriptor Base Low |
//! | 0x3804 | TDBAH | TX Descriptor Base High |
//! | 0x3808 | TDLEN | TX Descriptor Ring Length |
//! | 0x3810 | TDH | TX Descriptor Head |
//! | 0x3818 | TDT | TX Descriptor Tail |
//! | 0x5400 | RAL0 | Receive Address Low (MAC bytes 0-3) |
//! | 0x5404 | RAH0 | Receive Address High (MAC bytes 4-5 + flags) |

use alloc::collections::VecDeque;
use alloc::vec;
use alloc::vec::Vec;
use crate::error::Result;
use crate::memory::mmio::MmioHandler;

// Register offsets (dword-aligned).
const REG_CTRL: usize = 0x0000;
const REG_STATUS: usize = 0x0008;
const REG_EECD: usize = 0x0010;
const REG_EERD: usize = 0x0014;
const REG_MDIC: usize = 0x0020;
const REG_ICR: usize = 0x00C0;
const REG_ICS: usize = 0x00C8;
const REG_IMS: usize = 0x00D0;
const REG_IMC: usize = 0x00D8;
const REG_RCTL: usize = 0x0100;
const REG_TCTL: usize = 0x0400;
const REG_RDBAL: usize = 0x2800;
const REG_RDBAH: usize = 0x2804;
const REG_RDLEN: usize = 0x2808;
const REG_RDH: usize = 0x2810;
const REG_RDT: usize = 0x2818;
const REG_TDBAL: usize = 0x3800;
const REG_TDBAH: usize = 0x3804;
const REG_TDLEN: usize = 0x3808;
const REG_TDH: usize = 0x3810;
const REG_TDT: usize = 0x3818;
const REG_RAL0: usize = 0x5400;
const REG_RAH0: usize = 0x5404;

/// Total register space size: 128 KB (0x20000 bytes, 0x8000 dwords).
const REG_SPACE_DWORDS: usize = 0x8000;

/// STATUS register: link up (bit 1) + speed 1000 Mbps (bits 7:6 = 0b10).
const STATUS_LINK_UP: u32 = 0x02;
const STATUS_SPEED_1000: u32 = 0x80;

/// CTRL register: software reset bit (bit 26).
const CTRL_RST: u32 = 1 << 26;

/// EERD register: done bit (bit 4) indicates EEPROM read is complete.
const EERD_DONE: u32 = 1 << 4;
/// EERD register: start bit (bit 0).
const EERD_START: u32 = 1 << 0;

// EECD register bits for Microwire bit-bang EEPROM access.
const EECD_SK: u32 = 1 << 0;   // Clock
const EECD_CS: u32 = 1 << 1;   // Chip Select
const EECD_DI: u32 = 1 << 2;   // Data In (host → EEPROM)
const EECD_DO: u32 = 1 << 3;   // Data Out (EEPROM → host)
const EECD_FWE: u32 = 0x30;    // Flash Write Enable (bits 5:4)
const EECD_REQ: u32 = 1 << 6;  // Request EEPROM access
const EECD_GNT: u32 = 1 << 7;  // Grant EEPROM access
const EECD_PRES: u32 = 1 << 8; // EEPROM Present
// Bit 9: EEPROM size (0 = 64-word / 6-bit addr, 1 = 256-word / 8-bit addr)
// Bit 10: EEPROM type (0 = Microwire, 1 = SPI) — we use Microwire (0)

/// Microwire opcode: read = 0b110
const MW_OPCODE_READ: u8 = 0b110;

// MDIC register bits (PHY management via MDI).
const MDIC_DATA_MASK: u32 = 0xFFFF;       // Bits 15:0 — data
const MDIC_REG_SHIFT: u32 = 16;           // Bits 20:16 — PHY register
const MDIC_REG_MASK: u32 = 0x1F << 16;
const MDIC_PHY_SHIFT: u32 = 21;           // Bits 25:21 — PHY address
const MDIC_OP_WRITE: u32 = 1 << 26;       // Opcode: write
const MDIC_OP_READ: u32 = 2 << 26;        // Opcode: read
const MDIC_READY: u32 = 1 << 28;          // Ready bit (set by hardware)
const MDIC_ERROR: u32 = 1 << 30;          // Error bit

// PHY register addresses (M88E1000 / Marvell Alaska).
const PHY_CTRL: u32 = 0x00;
const PHY_STATUS: u32 = 0x01;
const PHY_ID1: u32 = 0x02;
const PHY_ID2: u32 = 0x03;
const PHY_AUTONEG_ADV: u32 = 0x04;
const PHY_LP_ABILITY: u32 = 0x05;
const PHY_1000T_CTRL: u32 = 0x09;
const PHY_1000T_STATUS: u32 = 0x0A;
// M88E1000 specific registers.
const M88_PHY_SPEC_CTRL: u32 = 0x10;
const M88_PHY_SPEC_STATUS: u32 = 0x11;
const M88_EXT_PHY_SPEC_CTRL: u32 = 0x14;

/// EEPROM checksum target: sum of all 64 words must equal this.
const EEPROM_CHECKSUM_TARGET: u16 = 0xBABA;

/// EEPROM word offsets.
const EEPROM_MAC0: usize = 0x00;  // MAC bytes 1:0
const EEPROM_MAC1: usize = 0x01;  // MAC bytes 3:2
const EEPROM_MAC2: usize = 0x02;  // MAC bytes 5:4
const EEPROM_SUBSYS_ID: usize = 0x0B; // Subsystem ID
const EEPROM_SUBSYS_VID: usize = 0x0C; // Subsystem Vendor ID
const EEPROM_DEVICE_ID: usize = 0x0D; // Device ID (optional)
const EEPROM_CHECKSUM: usize = 0x3F;  // Checksum word (last of 64)

/// State machine for Microwire EEPROM bit-bang access.
#[derive(Debug, Clone, Copy, PartialEq)]
enum EepromState {
    /// Idle: CS deasserted, waiting for transaction start.
    Idle,
    /// Receiving opcode + address bits (MSB first).
    /// We need 3 opcode bits + 6 address bits = 9 bits total.
    ReceivingCommand { bits_received: u8, shift_reg: u16 },
    /// Sending data bits back (MSB first), 16 bits.
    SendingData { bits_sent: u8, data: u16 },
}

/// PCI hole constants for guest physical → host offset translation.
/// Guest RAM > 3.5GB is split: 0..0xE0000000 and 0x100000000..
/// Host memory is contiguous, so GPA 0x100000000+ maps to host offset 0xE0000000+.
const PCI_HOLE_START: u64 = 0xE000_0000;
const PCI_HOLE_END: u64   = 0x1_0000_0000;

/// ICR bit: Transmit Descriptor Written Back.
const ICR_TXDW: u32 = 1 << 0;
/// ICR bit: Receive Timer Interrupt.
const ICR_RXT0: u32 = 1 << 7;

/// TX descriptor CMD bits.
const TXD_CMD_EOP: u8 = 1 << 0; // End of Packet
const TXD_CMD_RS: u8  = 1 << 3; // Report Status

/// TX descriptor STA bits.
const TXD_STA_DD: u8 = 1 << 0; // Descriptor Done

/// RX descriptor status bits.
const RXD_STA_DD: u8  = 1 << 0; // Descriptor Done
const RXD_STA_EOP: u8 = 1 << 1; // End of Packet
const RXD_STA_IXSM: u8 = 1 << 2; // Ignore Checksum Indication

/// Simplified Intel E1000 network interface card.
pub struct E1000 {
    /// Full 128 KB register space stored as 32-bit words.
    pub regs: Vec<u32>,
    /// MAC address (6 bytes).
    pub mac_address: [u8; 6],
    /// 64-entry EEPROM contents (16-bit words).
    pub eeprom: [u16; 64],
    /// Packets received from the network, waiting for the guest to consume.
    pub rx_buffer: VecDeque<Vec<u8>>,
    /// Packets transmitted by the guest, waiting for the host to send.
    pub tx_buffer: Vec<Vec<u8>>,
    /// EECD Microwire bit-bang state machine.
    ee_state: EepromState,
    /// Previous clock state for edge detection (rising edge triggers).
    ee_sk_prev: bool,
    /// Current EECD register value (managed separately from regs[]).
    ee_cd: u32,
    /// PHY registers (32 x 16-bit, accessed via MDIC).
    phy_regs: [u16; 32],
    /// I/O indirect register address (written via I/O BAR port+0, used at port+4).
    pub io_addr: u32,
    /// Guest memory pointer for DMA (TX/RX descriptor ring access).
    pub guest_mem_ptr: *mut u8,
    /// Guest memory length in bytes.
    pub guest_mem_len: usize,
    /// MSI state — set when the guest enables MSI via PCI config space.
    pub msi_enabled: bool,
    pub msi_address: u64,
    pub msi_data: u32,
    /// Callback to immediately assert/de-assert the interrupt line.
    /// Called from ICS/IMS writes so the interrupt arrives before the
    /// driver's next instruction (critical for the interrupt test).
    /// Arguments: (want_asserted: bool) → called with true when (ICR & IMS) != 0.
    #[cfg(feature = "std")]
    pub irq_callback: Option<alloc::boxed::Box<dyn FnMut(bool) + Send>>,
}

impl E1000 {
    /// Create a new E1000 NIC with the specified MAC address.
    ///
    /// The device starts with link up, 1000 Mbps speed, and the MAC
    /// address stored in both the EEPROM and the receive address registers.
    pub fn new(mac: [u8; 6]) -> Self {
        let mut regs = vec![0u32; REG_SPACE_DWORDS];

        // STATUS: link up, speed = 1000 Mbps.
        regs[REG_STATUS / 4] = STATUS_LINK_UP | STATUS_SPEED_1000;

        // Store MAC address in Receive Address registers (RAL0/RAH0).
        let ral = (mac[0] as u32)
            | ((mac[1] as u32) << 8)
            | ((mac[2] as u32) << 16)
            | ((mac[3] as u32) << 24);
        let rah = (mac[4] as u32) | ((mac[5] as u32) << 8) | (1 << 31); // AV (address valid) bit
        regs[REG_RAL0 / 4] = ral;
        regs[REG_RAH0 / 4] = rah;

        // Populate EEPROM with MAC address and metadata.
        let mut eeprom = [0u16; 64];
        eeprom[EEPROM_MAC0] = (mac[1] as u16) << 8 | (mac[0] as u16);
        eeprom[EEPROM_MAC1] = (mac[3] as u16) << 8 | (mac[2] as u16);
        eeprom[EEPROM_MAC2] = (mac[5] as u16) << 8 | (mac[4] as u16);

        // PCI IDs so the driver recognizes the device.
        eeprom[EEPROM_SUBSYS_VID] = 0x8086; // Intel
        eeprom[EEPROM_SUBSYS_ID] = 0x100E;  // 82540EM
        eeprom[EEPROM_DEVICE_ID] = 0x100E;

        // Init word (word 0x0A): misc config — set valid bits.
        eeprom[0x0A] = 0x0000;

        // Compute checksum: sum of words 0..63 + checksum word = 0xBABA.
        let mut sum: u16 = 0;
        for i in 0..EEPROM_CHECKSUM {
            sum = sum.wrapping_add(eeprom[i]);
        }
        eeprom[EEPROM_CHECKSUM] = EEPROM_CHECKSUM_TARGET.wrapping_sub(sum);

        // EECD: EEPROM present, Microwire type, 64-word size, grant access.
        let ee_cd = EECD_PRES | EECD_GNT | EECD_FWE;

        // PHY registers (M88E1000 / Marvell Alaska 88E1011).
        let mut phy_regs = [0u16; 32];
        // PHY Control: auto-negotiate enabled.
        phy_regs[PHY_CTRL as usize] = 0x1140;
        // PHY Status: link up, auto-negotiate complete, extended capability.
        phy_regs[PHY_STATUS as usize] = 0x796D;
        // PHY ID: Marvell 88E1011 (0x01410C20) — the 82540EM uses M88E1011,
        // NOT M88E1000.  The Linux driver's e1000_detect_gig_phy() checks for
        // M88E1011_I_PHY_ID = 0x01410C20 and rejects 0x01410C60.
        phy_regs[PHY_ID1 as usize] = 0x0141;
        phy_regs[PHY_ID2 as usize] = 0x0C20;
        // Auto-Negotiate Advertisement: 10/100 + selector.
        phy_regs[PHY_AUTONEG_ADV as usize] = 0x01E1;
        // Link Partner Ability: 10/100/1000.
        phy_regs[PHY_LP_ABILITY as usize] = 0x45E1;
        // 1000BASE-T Control: advertise 1000.
        phy_regs[PHY_1000T_CTRL as usize] = 0x0300;
        // 1000BASE-T Status: partner capable of 1000.
        phy_regs[PHY_1000T_STATUS as usize] = 0x3C00;
        // M88 PHY Specific Status: speed=1000, duplex=full, link=up, resolved.
        phy_regs[M88_PHY_SPEC_STATUS as usize] = 0xAC04;
        // M88 PHY Specific Control: defaults.
        phy_regs[M88_PHY_SPEC_CTRL as usize] = 0x0068;
        // M88 Extended PHY Specific Control.
        phy_regs[M88_EXT_PHY_SPEC_CTRL as usize] = 0x0D60;

        E1000 {
            regs,
            mac_address: mac,
            eeprom,
            rx_buffer: VecDeque::new(),
            tx_buffer: Vec::new(),
            ee_state: EepromState::Idle,
            ee_sk_prev: false,
            ee_cd,
            phy_regs,
            io_addr: 0,
            guest_mem_ptr: core::ptr::null_mut(),
            guest_mem_len: 0,
            msi_enabled: false,
            msi_address: 0,
            msi_data: 0,
            #[cfg(feature = "std")]
            irq_callback: None,
        }
    }

    /// Enqueue a packet received from the network for guest consumption.
    ///
    /// The packet will be delivered to the guest when it polls the RX
    /// descriptor ring.
    pub fn receive_packet(&mut self, data: &[u8]) {
        self.rx_buffer.push_back(data.to_vec());
        // Set RX interrupt cause (bit 7 = RXT0, receiver timer interrupt).
        let icr = self.regs[REG_ICR / 4];
        self.regs[REG_ICR / 4] = icr | (1 << 7);
    }

    /// Re-evaluate interrupt state and fire callback if needed.
    /// Called after any change to ICR or IMS.
    fn update_irq(&mut self) {
        // Interrupt delivery is handled by poll_irqs in the VM loop.
        // No callback needed — poll_irqs checks ICR & IMS every iteration.
    }

    /// Drain and return all packets transmitted by the guest.
    ///
    /// The host should forward these packets to the actual network or
    /// to another VM.
    pub fn take_tx_packets(&mut self) -> Vec<Vec<u8>> {
        let mut packets = Vec::new();
        core::mem::swap(&mut packets, &mut self.tx_buffer);
        packets
    }

    /// Perform a software reset, restoring registers to their power-on
    /// defaults while preserving the MAC address and EEPROM.
    fn reset(&mut self) {
        let mac = self.mac_address;
        let eeprom = self.eeprom;

        // Clear all registers.
        for reg in self.regs.iter_mut() {
            *reg = 0;
        }

        // Restore defaults.
        self.regs[REG_STATUS / 4] = STATUS_LINK_UP | STATUS_SPEED_1000;

        // Restore MAC in RAL0/RAH0.
        let ral = (mac[0] as u32)
            | ((mac[1] as u32) << 8)
            | ((mac[2] as u32) << 16)
            | ((mac[3] as u32) << 24);
        let rah = (mac[4] as u32) | ((mac[5] as u32) << 8) | (1 << 31);
        self.regs[REG_RAL0 / 4] = ral;
        self.regs[REG_RAH0 / 4] = rah;

        self.eeprom = eeprom;
        self.rx_buffer.clear();
        self.tx_buffer.clear();

        // Reset EEPROM bit-bang state.
        self.ee_state = EepromState::Idle;
        self.ee_sk_prev = false;
        self.ee_cd = EECD_PRES | EECD_GNT | EECD_FWE;
    }

    /// Translate guest physical address to host memory offset.
    #[inline]
    fn gpa_to_host(&self, gpa: u64) -> Option<*mut u8> {
        let offset = if gpa < PCI_HOLE_START {
            gpa as usize
        } else if gpa >= PCI_HOLE_END {
            (PCI_HOLE_START + (gpa - PCI_HOLE_END)) as usize
        } else {
            return None;
        };
        if offset >= self.guest_mem_len || self.guest_mem_ptr.is_null() {
            return None;
        }
        Some(unsafe { self.guest_mem_ptr.add(offset) })
    }

    /// Read bytes from guest physical memory.
    fn dma_read(&self, gpa: u64, buf: &mut [u8]) -> bool {
        let ptr = match self.gpa_to_host(gpa) {
            Some(p) => p,
            None => return false,
        };
        if (gpa as usize) + buf.len() > self.guest_mem_len {
            return false;
        }
        unsafe { core::ptr::copy_nonoverlapping(ptr, buf.as_mut_ptr(), buf.len()); }
        true
    }

    /// Write bytes to guest physical memory.
    fn dma_write(&self, gpa: u64, data: &[u8]) -> bool {
        let ptr = match self.gpa_to_host(gpa) {
            Some(p) => p,
            None => return false,
        };
        if (gpa as usize) + data.len() > self.guest_mem_len {
            return false;
        }
        unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len()); }
        true
    }

    /// Process TX descriptor ring: read descriptors, extract packet data,
    /// push complete packets to tx_buffer.
    ///
    /// Called when the guest writes TDT (TX Descriptor Tail).
    fn process_tx_ring(&mut self) {
        let tdbal = self.regs[REG_TDBAL / 4] as u64;
        let tdbah = self.regs[REG_TDBAH / 4] as u64;
        let td_base = (tdbah << 32) | tdbal;
        let tdlen = self.regs[REG_TDLEN / 4] as u64; // Ring length in bytes
        if tdlen == 0 || td_base == 0 { return; }

        let num_descs = (tdlen / 16) as u32; // Each descriptor is 16 bytes
        let mut head = self.regs[REG_TDH / 4];
        let tail = self.regs[REG_TDT / 4];

        let mut pkt_data: Vec<u8> = Vec::new();
        let mut processed = 0u32;

        while head != tail && processed < num_descs {
            // Read 16-byte legacy TX descriptor from guest memory.
            let desc_addr = td_base + (head as u64) * 16;
            let mut desc = [0u8; 16];
            if !self.dma_read(desc_addr, &mut desc) { break; }

            // Parse descriptor fields.
            let buf_addr = u64::from_le_bytes([
                desc[0], desc[1], desc[2], desc[3],
                desc[4], desc[5], desc[6], desc[7],
            ]);
            let length = u16::from_le_bytes([desc[8], desc[9]]) as usize;
            let cmd = desc[11];
            let eop = cmd & TXD_CMD_EOP != 0;
            let rs = cmd & TXD_CMD_RS != 0;

            if length > 0 && length <= 16384 && buf_addr != 0 {
                let start = pkt_data.len();
                pkt_data.resize(start + length, 0);
                if !self.dma_read(buf_addr, &mut pkt_data[start..]) {
                    pkt_data.truncate(start);
                    break;
                }
            }

            // Write back DD (descriptor done) bit if RS is set.
            if rs {
                desc[12] |= TXD_STA_DD;
                self.dma_write(desc_addr + 12, &desc[12..13]);
            }

            // Advance head (ring wrap).
            head = (head + 1) % num_descs;

            if eop {
                // Complete packet — push to tx_buffer.
                if !pkt_data.is_empty() {
                    self.tx_buffer.push(core::mem::take(&mut pkt_data));
                }
            }

            processed += 1;
        }

        // Update TDH.
        self.regs[REG_TDH / 4] = head;

        // Set TXDW interrupt if we processed any descriptors.
        if processed > 0 {
            self.regs[REG_ICR / 4] |= ICR_TXDW;
        }
    }

    /// Deliver pending RX packets to the guest via the RX descriptor ring.
    ///
    /// Called from the poll loop when rx_buffer has packets.
    pub fn process_rx_ring(&mut self) {
        let rdbal = self.regs[REG_RDBAL / 4] as u64;
        let rdbah = self.regs[REG_RDBAH / 4] as u64;
        let rd_base = (rdbah << 32) | rdbal;
        let rdlen = self.regs[REG_RDLEN / 4] as u64;
        if rdlen == 0 || rd_base == 0 { return; }

        let num_descs = (rdlen / 16) as u32;
        let mut head = self.regs[REG_RDH / 4];
        let tail = self.regs[REG_RDT / 4];

        // RX ring is empty when head == tail (no available descriptors).
        // Available descriptors: from head to tail (exclusive).
        // We need at least one descriptor to deliver a packet.

        // Deliver up to 128 packets per call — enough to drain a burst
        // without spinning forever if the ring is full. The guest updates
        // RDT after processing; we'll deliver more on the next poll.
        let mut delivered_count = 0u32;
        const MAX_RX_PER_POLL: u32 = 128;

        while let Some(_pkt) = self.rx_buffer.front() {
            if delivered_count >= MAX_RX_PER_POLL { break; }
            if head == tail {
                #[cfg(feature = "std")]
                if !self.rx_buffer.is_empty() {
                    static mut STALL_COUNT: u32 = 0;
                    unsafe { STALL_COUNT += 1; }
                    if unsafe { STALL_COUNT } % 1000 == 1 {
                        eprintln!("[e1000] RX STALL: head={} tail={} pending={} stalls={}",
                            head, tail, self.rx_buffer.len(), unsafe { STALL_COUNT });
                    }
                }
                break;
            }

            let desc_addr = rd_base + (head as u64) * 16;
            let mut desc = [0u8; 16];
            if !self.dma_read(desc_addr, &mut desc) {
                #[cfg(feature = "std")]
                eprintln!("[e1000] RX: dma_read desc FAILED at 0x{:X}", desc_addr);
                break;
            }

            // Read buffer address from descriptor.
            let buf_addr = u64::from_le_bytes([
                desc[0], desc[1], desc[2], desc[3],
                desc[4], desc[5], desc[6], desc[7],
            ]);

            if buf_addr == 0 {
                #[cfg(feature = "std")]
                eprintln!("[e1000] RX: buf_addr=0 at desc[{}]", head);
                break;
            }

            // Write packet data to guest buffer.
            // If RCTL.SECRC (bit 26) is not set, the driver expects a 4-byte
            // Ethernet FCS at the end of the frame. Append dummy CRC.
            let pkt = self.rx_buffer.pop_front().unwrap();
            let rctl = self.regs[0x0100 / 4];
            let secrc = rctl & (1 << 26) != 0;
            let mut frame = pkt;
            if !secrc {
                frame.extend_from_slice(&[0u8; 4]); // dummy FCS
            }
            let write_len = frame.len().min(2048);
            if !self.dma_write(buf_addr, &frame[..write_len]) {
                #[cfg(feature = "std")]
                eprintln!("[e1000] RX: dma_write FAILED buf=0x{:X} len={}", buf_addr, write_len);
                break;
            }
            #[cfg(feature = "std")]
            eprintln!("[e1000] RX: {} bytes desc[{}] secrc={}", write_len, head, secrc);

            // Update descriptor: length, status (DD + EOP), clear errors.
            let len_bytes = (write_len as u16).to_le_bytes();
            desc[8] = len_bytes[0]; // length low
            desc[9] = len_bytes[1]; // length high
            desc[10] = 0; // checksum (not computed)
            desc[11] = 0; // checksum high
            desc[12] = RXD_STA_DD | RXD_STA_EOP; // status: done + end of packet
            desc[13] = 0; // errors: none
            desc[14] = 0; // special low
            desc[15] = 0; // special high

            // Write back updated descriptor.
            self.dma_write(desc_addr, &desc);

            // Advance head.
            head = (head + 1) % num_descs;
            delivered_count += 1;
        }

        let delivered = head != self.regs[REG_RDH / 4];
        self.regs[REG_RDH / 4] = head;

        // Set RX interrupt if we delivered any packets or more are pending.
        if delivered || !self.rx_buffer.is_empty() {
            self.regs[REG_ICR / 4] |= ICR_RXT0;
        }
    }

    /// Handle a write to the EECD register (Microwire bit-bang protocol).
    ///
    /// The driver clocks data via DI on rising edges of SK while CS is
    /// asserted. After receiving 3 opcode bits + 6 address bits, the device
    /// responds with 16 data bits on DO (also clocked out on SK rising edges).
    fn write_eecd(&mut self, val: u32) {
        let cs = val & EECD_CS != 0;
        let sk = val & EECD_SK != 0;
        let di = val & EECD_DI != 0;
        let rising_edge = sk && !self.ee_sk_prev;

        // Preserve host-driven bits (SK, CS, DI), keep device-driven bits (DO, PRES, GNT).
        self.ee_cd = (val & (EECD_SK | EECD_CS | EECD_DI | EECD_REQ | EECD_FWE))
            | (self.ee_cd & EECD_DO)
            | EECD_PRES | EECD_GNT;

        if !cs {
            // CS deasserted: reset state machine.
            self.ee_state = EepromState::Idle;
            self.ee_cd &= !EECD_DO; // Clear DO
            self.ee_sk_prev = sk;
            return;
        }

        if rising_edge {
            match self.ee_state {
                EepromState::Idle => {
                    // First rising edge with CS: start receiving command.
                    let bit = if di { 1u16 } else { 0 };
                    self.ee_state = EepromState::ReceivingCommand {
                        bits_received: 1,
                        shift_reg: bit,
                    };
                    self.ee_cd &= !EECD_DO;
                }
                EepromState::ReceivingCommand { bits_received, shift_reg } => {
                    let bit = if di { 1u16 } else { 0 };
                    let new_reg = (shift_reg << 1) | bit;
                    let new_count = bits_received + 1;

                    if new_count == 9 {
                        // We have 3 opcode bits + 6 address bits.
                        let opcode = ((new_reg >> 6) & 0x07) as u8;
                        let addr = (new_reg & 0x3F) as usize;

                        if opcode == MW_OPCODE_READ {
                            let data = if addr < self.eeprom.len() {
                                self.eeprom[addr]
                            } else {
                                0xFFFF
                            };
                            // Transition to SendingData.  DO is NOT set here — the
                            // driver will clock it on the next rising edge, which
                            // outputs bit 15 (MSB).  Setting DO on this edge would
                            // cause an off-by-one: the driver reads DO after the
                            // NEXT rising edge, so bit 15 would be lost.
                            self.ee_cd &= !EECD_DO;
                            self.ee_state = EepromState::SendingData {
                                bits_sent: 0,
                                data,
                            };
                        } else {
                            // Write/erase/EWEN/EWDS — not needed, go idle.
                            self.ee_state = EepromState::Idle;
                            self.ee_cd &= !EECD_DO;
                        }
                    } else {
                        self.ee_state = EepromState::ReceivingCommand {
                            bits_received: new_count,
                            shift_reg: new_reg,
                        };
                        self.ee_cd &= !EECD_DO;
                    }
                }
                EepromState::SendingData { bits_sent, data } => {
                    if bits_sent >= 16 {
                        // All 16 bits sent, go idle (CS still high, driver
                        // will deassert CS to end transaction).
                        self.ee_state = EepromState::Idle;
                        self.ee_cd &= !EECD_DO;
                    } else {
                        // Output current bit (MSB first): bit (15 - bits_sent).
                        // The driver reads DO after this rising edge, so we must
                        // output the bit BEFORE advancing the counter.
                        let bit_pos = 15 - bits_sent;
                        if data & (1 << bit_pos) != 0 {
                            self.ee_cd |= EECD_DO;
                        } else {
                            self.ee_cd &= !EECD_DO;
                        }
                        self.ee_state = EepromState::SendingData {
                            bits_sent: bits_sent + 1,
                            data,
                        };
                    }
                }
            }
        }

        self.ee_sk_prev = sk;
    }
}

impl MmioHandler for E1000 {
    /// Read a register from the E1000 MMIO region.
    ///
    /// Handles special cases:
    /// - **EECD**: returns the current EEPROM control/data register (with DO bit).
    /// - **EERD**: if a read was started, returns the EEPROM data with
    ///   the done bit set.
    /// - **ICR**: reading clears the interrupt cause bits.
    fn read(&mut self, offset: u64, size: u8) -> Result<u64> {
        let dword_offset = (offset as usize) / 4;
        if dword_offset >= self.regs.len() {
            return Ok(0);
        }

        let val = match offset as usize {
            REG_EECD => {
                // Return the EEPROM control register (managed separately).
                self.ee_cd
            }
            REG_MDIC => {
                // Return the MDIC register — the write handler already placed
                // the result (data + READY) into regs[].
                self.regs[dword_offset]
            }
            REG_EERD => {
                // If a read was started, return the requested EEPROM word.
                let eerd = self.regs[dword_offset];
                if eerd & EERD_START != 0 {
                    let addr = ((eerd >> 8) & 0xFF) as usize;
                    let data = if addr < self.eeprom.len() {
                        self.eeprom[addr]
                    } else {
                        0xFFFF
                    };
                    let result = ((data as u32) << 16) | EERD_DONE;
                    result
                } else {
                    eerd
                }
            }
            REG_ICR => {
                // Reading ICR clears all interrupt cause bits.
                let icr = self.regs[dword_offset];
                self.regs[dword_offset] = 0;
                self.update_irq(); // de-assert interrupt
                icr
            }
            _ => self.regs[dword_offset],
        };

        // Handle sub-dword reads by extracting the requested bytes.
        let byte_offset = (offset as usize) & 3;
        let shifted = val >> (byte_offset * 8);
        let mask = match size {
            1 => 0xFF,
            2 => 0xFFFF,
            _ => 0xFFFF_FFFF,
        };
        Ok((shifted & mask) as u64)
    }

    /// Write a register in the E1000 MMIO region.
    ///
    /// Handles special cases:
    /// - **CTRL**: bit 26 (RST) triggers a software reset.
    /// - **EECD**: drives the Microwire bit-bang EEPROM state machine.
    /// - **ICS**: writing sets interrupt cause bits.
    /// - **IMS**: writing sets interrupt mask bits (OR semantics).
    /// - **IMC**: writing clears interrupt mask bits.
    /// - **TDT**: writing advances the TX descriptor tail, which
    ///   signals that new packets are ready (TX processing is deferred
    ///   to the host integration layer).
    fn write(&mut self, offset: u64, size: u8, val: u64) -> Result<()> {
        let dword_offset = (offset as usize) / 4;
        if dword_offset >= self.regs.len() {
            return Ok(());
        }

        // Assemble the full dword value for sub-dword writes.
        let byte_offset = (offset as usize) & 3;
        let mask = match size {
            1 => 0xFFu32,
            2 => 0xFFFFu32,
            _ => 0xFFFF_FFFFu32,
        };
        let shifted_mask = mask << (byte_offset * 8);
        let shifted_val = ((val as u32) & mask) << (byte_offset * 8);
        let new_val = (self.regs[dword_offset] & !shifted_mask) | shifted_val;

        match offset as usize & !3 {
            REG_CTRL => {
                self.regs[dword_offset] = new_val;
                if new_val & CTRL_RST != 0 {
                    self.reset();
                }
            }
            REG_STATUS => {
                // STATUS is mostly read-only; ignore writes.
            }
            REG_EECD => {
                // Drive the Microwire bit-bang state machine.
                self.write_eecd(new_val);
            }
            REG_MDIC => {
                // PHY register access via MDI.
                let phy_reg = ((new_val & MDIC_REG_MASK) >> MDIC_REG_SHIFT) as usize;
                if new_val & MDIC_OP_READ != 0 {
                    // Read: return PHY register value with READY bit.
                    let data = if phy_reg < self.phy_regs.len() {
                        self.phy_regs[phy_reg]
                    } else {
                        0
                    };
                    self.regs[dword_offset] = (new_val & !MDIC_DATA_MASK & !MDIC_ERROR)
                        | (data as u32)
                        | MDIC_READY;
                } else if new_val & MDIC_OP_WRITE != 0 {
                    // Write: store PHY register value.
                    let mut phy_data = (new_val & MDIC_DATA_MASK) as u16;
                    if phy_reg == PHY_CTRL as usize && phy_data & 0x8000 != 0 {
                        // PHY reset (bit 15) is self-clearing — the real PHY
                        // completes the reset instantly, so clear it immediately.
                        phy_data &= !0x8000;
                    }
                    if phy_reg < self.phy_regs.len() {
                        self.phy_regs[phy_reg] = phy_data;
                    }
                    self.regs[dword_offset] = (new_val & !MDIC_ERROR) | MDIC_READY;
                } else {
                    self.regs[dword_offset] = new_val;
                }
            }
            REG_ICS => {
                // Writing to ICS ORs cause bits into ICR (software-triggered interrupt).
                self.regs[REG_ICR / 4] |= new_val;
                self.update_irq();
            }
            REG_IMS => {
                // Writing to IMS sets (OR) interrupt mask bits.
                self.regs[dword_offset] |= new_val;
                self.update_irq();
            }
            REG_IMC => {
                // Writing to IMC clears interrupt mask bits.
                self.regs[REG_IMS / 4] &= !new_val;
                self.update_irq();
            }
            REG_TDBAH | REG_RDBAH => {
                // 82540EM is a 32-bit PCI device — ignore high address writes.
                // Reads return 0, so the driver knows this is 32-bit only.
            }
            REG_TDT => {
                // TX Descriptor Tail — guest signals new packets to transmit.
                self.regs[dword_offset] = new_val;
                self.process_tx_ring();
            }
            _ => {
                self.regs[dword_offset] = new_val;
            }
        }

        Ok(())
    }
}
