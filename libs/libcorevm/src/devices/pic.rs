//! Intel 8259A Programmable Interrupt Controller (PIC) emulation.
//!
//! Emulates a dual-PIC configuration (master + slave) as found in the
//! IBM PC/AT architecture. The master PIC handles IRQ 0-7 and the slave
//! PIC handles IRQ 8-15, cascaded through IRQ 2 on the master.
//!
//! # I/O Ports
//!
//! | Port | Description |
//! |------|-------------|
//! | 0x20 | Master command |
//! | 0x21 | Master data |
//! | 0xA0 | Slave command |
//! | 0xA1 | Slave data |

use crate::error::Result;
use crate::io::IoHandler;

/// State for a single 8259A PIC chip.
#[derive(Debug)]
pub struct Pic {
    /// Interrupt Request Register — one bit per IRQ line, set when an IRQ
    /// is asserted and waiting to be serviced.
    pub irr: u8,
    /// In-Service Register — one bit per IRQ line, set while the
    /// corresponding interrupt handler is executing.
    pub isr: u8,
    /// Interrupt Mask Register — masked bits prevent the corresponding
    /// IRQ from being delivered.
    pub imr: u8,
    /// Initialization Command Words collected during the ICW sequence.
    pub icw: [u8; 4],
    /// Current step in the ICW initialization sequence.
    /// 0 = not initializing, 1-4 = expecting ICW1..ICW4.
    pub icw_step: u8,
    /// Base interrupt vector number. IRQ N maps to vector `vector_offset + N`.
    /// Typically 0x08 for master, 0x70 for slave.
    pub vector_offset: u8,
    /// OCW3 state: when `true`, reads from the command port return the ISR;
    /// when `false`, they return the IRR.
    pub read_isr: bool,
    /// ICW4 auto-EOI mode. When enabled, the ISR bit is automatically
    /// cleared at the end of the second INTA pulse.
    pub auto_eoi: bool,
}

impl Pic {
    /// Create a new PIC in its power-on default state.
    pub fn new() -> Self {
        Pic {
            irr: 0,
            isr: 0,
            imr: 0xFF, // all IRQs masked by default
            icw: [0; 4],
            icw_step: 0,
            vector_offset: 0,
            read_isr: false,
            auto_eoi: false,
        }
    }
}

/// Dual 8259A PIC pair (master + slave) with full ICW/OCW protocol.
///
/// The master PIC services IRQ 0-7 and the slave PIC services IRQ 8-15.
/// IRQ 2 on the master is wired to the slave's cascade output.
#[derive(Debug)]
pub struct PicPair {
    /// Master PIC (IRQ 0-7, ports 0x20-0x21).
    pub master: Pic,
    /// Slave PIC (IRQ 8-15, ports 0xA0-0xA1).
    pub slave: Pic,
}

impl PicPair {
    /// Create a new dual PIC pair in the power-on default state.
    ///
    /// Both PICs start with all IRQs masked, no initialization sequence
    /// in progress, and default vector offsets (0x08 master, 0x70 slave).
    pub fn new() -> Self {
        let mut master = Pic::new();
        let mut slave = Pic::new();
        master.vector_offset = 0x08;
        slave.vector_offset = 0x70;
        PicPair { master, slave }
    }

    /// Assert an IRQ line (edge-triggered).
    ///
    /// IRQ 0-7 are routed to the master PIC, IRQ 8-15 to the slave PIC.
    /// When a slave IRQ is raised, IRQ 2 (cascade) is also raised on the
    /// master so the CPU sees the slave's pending interrupt.
    pub fn raise_irq(&mut self, irq: u8) {
        if irq < 8 {
            self.master.irr |= 1 << irq;
        } else if irq < 16 {
            self.slave.irr |= 1 << (irq - 8);
            // Cascade: assert IRQ 2 on master so the slave interrupt
            // propagates through the master's priority logic.
            self.master.irr |= 1 << 2;
        }
    }

    /// De-assert an IRQ line.
    ///
    /// Clears the IRR bit for the specified IRQ.
    pub fn lower_irq(&mut self, irq: u8) {
        if irq < 8 {
            self.master.irr &= !(1 << irq);
        } else if irq < 16 {
            self.slave.irr &= !(1 << (irq - 8));
            // If no slave IRQs remain pending, clear the cascade line.
            if self.slave.irr & !self.slave.imr == 0 {
                self.master.irr &= !(1 << 2);
            }
        }
    }

    /// Get the vector number of the highest-priority pending interrupt.
    ///
    /// Scans the master PIC for the lowest-numbered unmasked IRQ with a
    /// pending request not already in service. If the winning IRQ is the
    /// cascade line (IRQ 2), the slave PIC is queried instead.
    ///
    /// Returns `None` if no interrupt is pending or all pending IRQs are
    /// masked or already in service.
    pub fn get_interrupt_vector(&self) -> Option<u8> {
        let master_pending = self.master.irr & !self.master.imr & !self.master.isr;
        if master_pending == 0 {
            return None;
        }

        let master_irq = master_pending.trailing_zeros() as u8;

        // If the winning master IRQ is the cascade input, consult the slave.
        if master_irq == 2 {
            let slave_pending = self.slave.irr & !self.slave.imr & !self.slave.isr;
            if slave_pending == 0 {
                return None;
            }
            let slave_irq = slave_pending.trailing_zeros() as u8;
            Some(self.slave.vector_offset + slave_irq)
        } else {
            Some(self.master.vector_offset + master_irq)
        }
    }

    /// Acknowledge delivery of an interrupt to the CPU.
    ///
    /// Moves the IRQ from pending (IRR) to in-service (ISR). For auto-EOI
    /// PICs the ISR bit is immediately cleared again.
    pub fn acknowledge(&mut self, irq: u8) {
        if irq < 8 {
            let bit = 1 << irq;
            self.master.irr &= !bit;
            self.master.isr |= bit;
            if self.master.auto_eoi {
                self.master.isr &= !bit;
            }
        } else if irq < 16 {
            let slave_bit = 1 << (irq - 8);
            self.slave.irr &= !slave_bit;
            self.slave.isr |= slave_bit;
            if self.slave.auto_eoi {
                self.slave.isr &= !slave_bit;
            }
            // Also acknowledge cascade IRQ 2 on master.
            let cascade_bit = 1 << 2;
            self.master.irr &= !cascade_bit;
            self.master.isr |= cascade_bit;
            if self.master.auto_eoi {
                self.master.isr &= !cascade_bit;
            }
        }
    }

    /// Handle a write to a PIC command port (0x20 or 0xA0).
    fn write_command(pic: &mut Pic, val: u8) {
        if val & 0x10 != 0 {
            // ICW1: bit 4 set starts initialization sequence.
            pic.icw[0] = val;
            pic.icw_step = 1;
            pic.imr = 0;
            pic.isr = 0;
            pic.irr = 0;
            pic.read_isr = false;
            pic.auto_eoi = false;
        } else if val & 0x08 != 0 {
            // OCW3: read ISR/IRR control.
            if val & 0x02 != 0 {
                pic.read_isr = val & 0x01 != 0;
            }
        } else if val == 0x20 {
            // OCW2: non-specific EOI — clear highest-priority ISR bit.
            if pic.isr != 0 {
                let bit = 1 << pic.isr.trailing_zeros();
                pic.isr &= !bit;
            }
        } else if val & 0xE0 == 0x60 {
            // OCW2: specific EOI for IRQ N (N = low 3 bits).
            let irq = val & 0x07;
            pic.isr &= !(1 << irq);
        }
    }

    /// Handle a write to a PIC data port (0x21 or 0xA1).
    fn write_data(pic: &mut Pic, val: u8) {
        match pic.icw_step {
            1 => {
                // ICW2: vector offset (must be aligned to 8).
                pic.icw[1] = val;
                pic.vector_offset = val;
                pic.icw_step = 2;
            }
            2 => {
                // ICW3: cascade configuration.
                pic.icw[2] = val;
                // If ICW1 bit 0 indicates ICW4 is needed, advance to step 3;
                // otherwise initialization is complete.
                if pic.icw[0] & 0x01 != 0 {
                    pic.icw_step = 3;
                } else {
                    pic.icw_step = 0;
                }
            }
            3 => {
                // ICW4: mode configuration.
                pic.icw[3] = val;
                pic.auto_eoi = val & 0x02 != 0;
                pic.icw_step = 0;
            }
            _ => {
                // Not in initialization — this is an IMR update.
                pic.imr = val;
            }
        }
    }

    /// Handle a read from a PIC command port (0x20 or 0xA0).
    fn read_command(pic: &Pic) -> u8 {
        if pic.read_isr {
            pic.isr
        } else {
            pic.irr
        }
    }
}

impl IoHandler for PicPair {
    /// Read from PIC ports.
    ///
    /// - 0x20: master command (ISR or IRR depending on OCW3)
    /// - 0x21: master data (IMR)
    /// - 0xA0: slave command (ISR or IRR depending on OCW3)
    /// - 0xA1: slave data (IMR)
    fn read(&mut self, port: u16, _size: u8) -> Result<u32> {
        let val = match port {
            0x20 => Self::read_command(&self.master),
            0x21 => self.master.imr,
            0xA0 => Self::read_command(&self.slave),
            0xA1 => self.slave.imr,
            _ => 0xFF,
        };
        Ok(val as u32)
    }

    /// Write to PIC ports.
    ///
    /// Dispatches to the ICW/OCW protocol handler for the appropriate PIC.
    fn write(&mut self, port: u16, _size: u8, val: u32) -> Result<()> {
        let byte = val as u8;
        match port {
            0x20 => Self::write_command(&mut self.master, byte),
            0x21 => Self::write_data(&mut self.master, byte),
            0xA0 => Self::write_command(&mut self.slave, byte),
            0xA1 => Self::write_data(&mut self.slave, byte),
            _ => {}
        }
        Ok(())
    }
}
