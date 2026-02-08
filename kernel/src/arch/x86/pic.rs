//! 8259 Programmable Interrupt Controller (PIC) driver.
//!
//! Remaps IRQs 0-15 to INT 32-47 and provides masking and EOI helpers.
//! Superseded by the I/O APIC when ACPI is available, but kept as a
//! fallback for legacy mode.

use crate::arch::x86::port::{inb, outb, io_wait};

const PIC1_CMD: u16 = 0x20;
const PIC1_DATA: u16 = 0x21;
const PIC2_CMD: u16 = 0xA0;
const PIC2_DATA: u16 = 0xA1;

const ICW1_INIT: u8 = 0x10;
const ICW1_ICW4: u8 = 0x01;
const ICW4_8086: u8 = 0x01;

const PIC1_OFFSET: u8 = 32;  // IRQ 0-7  -> INT 32-39
const PIC2_OFFSET: u8 = 40;  // IRQ 8-15 -> INT 40-47

/// Remap both PICs and mask all IRQs. Individual IRQs are unmasked later.
pub fn init() {
    unsafe {
        // Save current masks
        let mask1 = inb(PIC1_DATA);
        let mask2 = inb(PIC2_DATA);

        // ICW1: Begin initialization sequence
        outb(PIC1_CMD, ICW1_INIT | ICW1_ICW4);
        io_wait();
        outb(PIC2_CMD, ICW1_INIT | ICW1_ICW4);
        io_wait();

        // ICW2: Set interrupt vector offsets
        outb(PIC1_DATA, PIC1_OFFSET);
        io_wait();
        outb(PIC2_DATA, PIC2_OFFSET);
        io_wait();

        // ICW3: Tell PICs about each other
        outb(PIC1_DATA, 4); // PIC1: slave on IRQ2 (bit 2)
        io_wait();
        outb(PIC2_DATA, 2); // PIC2: cascade identity = 2
        io_wait();

        // ICW4: 8086 mode
        outb(PIC1_DATA, ICW4_8086);
        io_wait();
        outb(PIC2_DATA, ICW4_8086);
        io_wait();

        // Mask all IRQs initially (kernel will unmask as needed)
        outb(PIC1_DATA, 0xFF);
        outb(PIC2_DATA, 0xFF);

        let _ = (mask1, mask2); // Suppress unused warning
    }
}

/// Unmask (enable) a specific IRQ line on the appropriate PIC.
pub fn unmask(irq: u8) {
    unsafe {
        if irq < 8 {
            let mask = inb(PIC1_DATA) & !(1 << irq);
            outb(PIC1_DATA, mask);
        } else {
            let mask = inb(PIC2_DATA) & !(1 << (irq - 8));
            outb(PIC2_DATA, mask);
            // Also unmask cascade IRQ2 on PIC1
            let mask1 = inb(PIC1_DATA) & !(1 << 2);
            outb(PIC1_DATA, mask1);
        }
    }
}

/// Send End-of-Interrupt to the PIC(s) for the given IRQ.
pub fn send_eoi(irq: u8) {
    unsafe {
        if irq >= 8 {
            outb(PIC2_CMD, 0x20); // EOI to slave
        }
        outb(PIC1_CMD, 0x20); // EOI to master
    }
}
