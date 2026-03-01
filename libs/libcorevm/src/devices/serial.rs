//! 16550 UART serial port (COM1) emulation.
//!
//! Emulates an NS16550A-compatible UART at the standard COM1 base address.
//! The device supports DLAB (Divisor Latch Access Bit) for baud rate
//! configuration, FIFO mode, and modem control/status registers.
//!
//! Characters written by the guest to the Transmit Holding Register (THR)
//! are collected in an output buffer. Characters injected via [`Serial::send_input`]
//! become available for the guest to read from the Receive Buffer Register (RBR).
//!
//! # I/O Ports (COM1: 0x3F8-0x3FF)
//!
//! | Offset | DLAB=0 Read | DLAB=0 Write | DLAB=1 Read | DLAB=1 Write |
//! |--------|-------------|--------------|-------------|--------------|
//! | +0     | RBR         | THR          | DLL         | DLL          |
//! | +1     | IER         | IER          | DLM         | DLM          |
//! | +2     | IIR         | FCR          | IIR         | FCR          |
//! | +3     | LCR         | LCR          | LCR         | LCR          |
//! | +4     | MCR         | MCR          | MCR         | MCR          |
//! | +5     | LSR         | —            | LSR         | —            |
//! | +6     | MSR         | —            | MSR         | —            |
//! | +7     | SCR         | SCR          | SCR         | SCR          |

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use crate::error::Result;
use crate::io::IoHandler;

/// Line Status Register bit masks.
const LSR_DATA_READY: u8 = 0x01;
const LSR_THR_EMPTY: u8 = 0x20;
const LSR_XMIT_EMPTY: u8 = 0x40;

/// 16550 UART serial port emulation (COM1).
#[derive(Debug)]
pub struct Serial {
    /// Receive Buffer Register — last byte received from input.
    pub rbr: u8,
    /// Transmit Holding Register — last byte written by guest.
    pub thr: u8,
    /// Interrupt Enable Register.
    pub ier: u8,
    /// Interrupt Identification Register (read-only to guest).
    pub iir: u8,
    /// FIFO Control Register (write-only from guest).
    pub fcr: u8,
    /// Line Control Register (bit 7 = DLAB).
    pub lcr: u8,
    /// Modem Control Register.
    pub mcr: u8,
    /// Line Status Register.
    pub lsr: u8,
    /// Modem Status Register.
    pub msr: u8,
    /// Scratch Register.
    pub scratch: u8,
    /// Divisor Latch Low byte.
    pub dll: u8,
    /// Divisor Latch High byte.
    pub dlm: u8,
    /// Characters written by the guest (THR output), available for the
    /// host to consume via [`take_output`](Serial::take_output).
    pub output: VecDeque<u8>,
    /// Characters available for the guest to read (RBR input), injected
    /// by the host via [`send_input`](Serial::send_input).
    pub input: VecDeque<u8>,
}

impl Serial {
    /// Create a new serial port in its power-on default state.
    ///
    /// The Line Status Register starts with THR empty and transmitter
    /// empty flags set, indicating the port is ready to accept data.
    pub fn new() -> Self {
        Serial {
            rbr: 0,
            thr: 0,
            ier: 0,
            iir: 0x01, // no interrupt pending
            fcr: 0,
            lcr: 0,
            mcr: 0,
            lsr: LSR_THR_EMPTY | LSR_XMIT_EMPTY,
            msr: 0,
            scratch: 0,
            dll: 0x0C, // 9600 baud default (115200 / 9600 = 12)
            dlm: 0,
            output: VecDeque::new(),
            input: VecDeque::new(),
        }
    }

    /// Push characters into the input buffer for the guest to read.
    ///
    /// After calling this method, the guest will see `LSR_DATA_READY` set
    /// and can read the characters via the RBR register.
    pub fn send_input(&mut self, data: &[u8]) {
        for &b in data {
            self.input.push_back(b);
        }
        if !self.input.is_empty() {
            self.lsr |= LSR_DATA_READY;
        }
    }

    /// Drain and return all characters written by the guest.
    ///
    /// This is the host-side interface for consuming serial output (e.g.,
    /// to display in a terminal or log file).
    pub fn take_output(&mut self) -> Vec<u8> {
        self.output.drain(..).collect()
    }

    /// Returns `true` if DLAB (Divisor Latch Access Bit) is set in the LCR.
    #[inline]
    fn dlab(&self) -> bool {
        self.lcr & 0x80 != 0
    }
}

impl IoHandler for Serial {
    /// Read from serial port registers.
    ///
    /// Register selection depends on the port offset and DLAB state.
    fn read(&mut self, port: u16, _size: u8) -> Result<u32> {
        let offset = port - 0x3F8;
        let val = match offset {
            0 => {
                if self.dlab() {
                    // DLAB=1: Divisor Latch Low
                    self.dll
                } else {
                    // DLAB=0: Receive Buffer Register
                    let byte = self.input.pop_front().unwrap_or(0);
                    self.rbr = byte;
                    if self.input.is_empty() {
                        self.lsr &= !LSR_DATA_READY;
                    }
                    byte
                }
            }
            1 => {
                if self.dlab() {
                    self.dlm
                } else {
                    self.ier
                }
            }
            2 => self.iir,
            3 => self.lcr,
            4 => self.mcr,
            5 => self.lsr,
            6 => self.msr,
            7 => self.scratch,
            _ => 0xFF,
        };
        Ok(val as u32)
    }

    /// Write to serial port registers.
    ///
    /// Register selection depends on the port offset and DLAB state.
    fn write(&mut self, port: u16, _size: u8, val: u32) -> Result<()> {
        let offset = port - 0x3F8;
        let byte = val as u8;
        match offset {
            0 => {
                if self.dlab() {
                    // DLAB=1: Divisor Latch Low
                    self.dll = byte;
                } else {
                    // DLAB=0: Transmit Holding Register
                    self.thr = byte;
                    self.output.push_back(byte);
                    // THR is immediately "empty" again (infinite speed UART).
                    self.lsr |= LSR_THR_EMPTY | LSR_XMIT_EMPTY;
                }
            }
            1 => {
                if self.dlab() {
                    self.dlm = byte;
                } else {
                    self.ier = byte & 0x0F; // only low 4 bits are writable
                }
            }
            2 => {
                // FIFO Control Register (write-only).
                self.fcr = byte;
                if byte & 0x01 != 0 {
                    // FIFOs enabled — update IIR to reflect FIFO mode.
                    self.iir |= 0xC0;
                } else {
                    self.iir &= !0xC0;
                }
                if byte & 0x02 != 0 {
                    // Clear receive FIFO.
                    self.input.clear();
                    self.lsr &= !LSR_DATA_READY;
                }
                if byte & 0x04 != 0 {
                    // Clear transmit FIFO.
                    self.output.clear();
                }
            }
            3 => self.lcr = byte,
            4 => self.mcr = byte,
            5 => { /* LSR is read-only */ }
            6 => { /* MSR is read-only */ }
            7 => self.scratch = byte,
            _ => {}
        }
        Ok(())
    }
}
