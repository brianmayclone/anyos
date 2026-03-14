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
    /// Whether a new IRQ 4 edge needs to be fired by the next poll_irqs call.
    /// Set when interrupt conditions change; cleared after the IRQ is pulsed.
    pub irq_pending: bool,
    /// Tracks whether the THRE interrupt has been raised since the last THR write.
    /// Prevents IRQ storms from the always-empty THR in our infinite-speed UART.
    thre_raised: bool,
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
            irq_pending: false,
            thre_raised: false,
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
            self.update_iir();
        }
    }

    /// Recalculate the IIR based on current interrupt conditions and
    /// request an IRQ 4 edge if a new interrupt became pending.
    fn update_iir(&mut self) {
        let old_pending = (self.iir & 0x01) == 0;
        // Priority: Receiver Line Status > Received Data > THR Empty > Modem Status
        if (self.ier & 0x01) != 0 && (self.lsr & LSR_DATA_READY) != 0 {
            // Received Data Available interrupt
            self.iir = (self.iir & 0xC0) | 0x04; // IIR = 0b0100, bit 0=0 = pending
        } else if (self.ier & 0x02) != 0 && (self.lsr & LSR_THR_EMPTY) != 0 && !self.thre_raised {
            // THR Empty interrupt — only if not already raised since last THR write
            self.iir = (self.iir & 0xC0) | 0x02;
            self.thre_raised = true;
        } else {
            // No interrupt pending
            self.iir = (self.iir & 0xC0) | 0x01; // bit 0=1 = no pending
        }
        let new_pending = (self.iir & 0x01) == 0;
        // Request IRQ edge on any new interrupt condition
        if new_pending && !old_pending {
            self.irq_pending = true;
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
                    self.update_iir();
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
            2 => {
                let val = self.iir;
                // Reading IIR with THR Empty pending clears the THRE condition
                if val & 0x0F == 0x02 {
                    self.iir = (self.iir & 0xC0) | 0x01; // no interrupt pending
                    // After clearing THRE, check if a lower-priority interrupt is pending
                    // (Don't call update_iir here — THRE stays cleared until next THR write)
                }
                val
            }
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
                    if self.mcr & 0x10 != 0 {
                        // MCR bit 4: Loopback mode — data loops back to RBR.
                        // Used by 8250 driver's IRQ probe to verify interrupts work.
                        self.input.push_back(byte);
                        self.lsr |= LSR_DATA_READY;
                    } else {
                        self.output.push_back(byte);
                    }
                    // THR is immediately "empty" again (infinite speed UART).
                    self.lsr |= LSR_THR_EMPTY | LSR_XMIT_EMPTY;
                    // Reset THRE raised flag — next update_iir can raise THRE again.
                    self.thre_raised = false;
                    self.update_iir();
                }
            }
            1 => {
                if self.dlab() {
                    self.dlm = byte;
                } else {
                    let old_ier = self.ier;
                    self.ier = byte & 0x0F; // only low 4 bits are writable
                    // If THRE interrupt was just enabled, reset thre_raised to allow
                    // the initial THRE interrupt when THR is already empty.
                    if (byte & 0x02) != 0 && (old_ier & 0x02) == 0 {
                        self.thre_raised = false;
                    }
                    self.update_iir();
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
            4 => {
                self.mcr = byte;
                // In loopback mode (bit 4), MCR outputs feed back to MSR inputs:
                //   MCR bit 1 (RTS)  → MSR bit 4 (CTS)
                //   MCR bit 0 (DTR)  → MSR bit 5 (DSR)
                //   MCR bit 2 (OUT1) → MSR bit 6 (RI)
                //   MCR bit 3 (OUT2) → MSR bit 7 (DCD)
                if byte & 0x10 != 0 {
                    self.msr = ((byte & 0x02) << 3)  // RTS→CTS (bit 1→bit 4)
                            | ((byte & 0x01) << 5)   // DTR→DSR (bit 0→bit 5)
                            | ((byte & 0x04) << 4)   // OUT1→RI (bit 2→bit 6)
                            | ((byte & 0x08) << 4);  // OUT2→DCD (bit 3→bit 7)
                }
            }
            5 => { /* LSR is read-only */ }
            6 => { /* MSR is read-only */ }
            7 => self.scratch = byte,
            _ => {}
        }
        Ok(())
    }
}
