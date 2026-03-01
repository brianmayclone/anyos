//! Intel E1000 network card emulation.
//!
//! Emulates a simplified Intel 82540EM Gigabit Ethernet controller
//! (PCI device 8086:100E) using MMIO-based register access.
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
const REG_ICR: usize = 0x00C0;
const REG_ICS: usize = 0x00C4;
const REG_IMS: usize = 0x00C8;
const REG_IMC: usize = 0x00CC;
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

/// Simplified Intel E1000 network interface card.
#[derive(Debug)]
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

        // Populate EEPROM with MAC address.
        let mut eeprom = [0u16; 64];
        eeprom[0] = (mac[1] as u16) << 8 | (mac[0] as u16);
        eeprom[1] = (mac[3] as u16) << 8 | (mac[2] as u16);
        eeprom[2] = (mac[5] as u16) << 8 | (mac[4] as u16);

        E1000 {
            regs,
            mac_address: mac,
            eeprom,
            rx_buffer: VecDeque::new(),
            tx_buffer: Vec::new(),
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
    }
}

impl MmioHandler for E1000 {
    /// Read a register from the E1000 MMIO region.
    ///
    /// Handles special cases:
    /// - **EERD**: if a read was started, returns the EEPROM data with
    ///   the done bit set.
    /// - **ICR**: reading clears the interrupt cause bits.
    fn read(&mut self, offset: u64, size: u8) -> Result<u64> {
        let dword_offset = (offset as usize) / 4;
        if dword_offset >= self.regs.len() {
            return Ok(0);
        }

        let val = match offset as usize {
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
                    // Return data in bits [31:16], done in bit 4.
                    ((data as u32) << 16) | EERD_DONE
                } else {
                    eerd
                }
            }
            REG_ICR => {
                // Reading ICR clears all interrupt cause bits.
                let icr = self.regs[dword_offset];
                self.regs[dword_offset] = 0;
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
            REG_ICS => {
                // Writing to ICS sets interrupt cause bits.
                self.regs[REG_ICR / 4] |= new_val;
            }
            REG_IMS => {
                // Writing to IMS sets (OR) interrupt mask bits.
                self.regs[dword_offset] |= new_val;
            }
            REG_IMC => {
                // Writing to IMC clears interrupt mask bits.
                self.regs[REG_IMS / 4] &= !new_val;
            }
            _ => {
                self.regs[dword_offset] = new_val;
            }
        }

        Ok(())
    }
}
