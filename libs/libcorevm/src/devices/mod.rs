//! Virtual hardware device emulation.
//!
//! Each device implements [`IoHandler`](crate::io::IoHandler) and/or
//! [`MmioHandler`](crate::memory::mmio::MmioHandler) to respond to guest
//! port I/O and memory-mapped I/O accesses.
//!
//! Devices emulated:
//! - [`pic`] — Intel 8259A dual PIC (Programmable Interrupt Controller)
//! - [`pit`] — Intel 8253/8254 PIT (Programmable Interval Timer)
//! - [`cmos`] — CMOS RTC and NVRAM
//! - [`ps2`] — PS/2 controller (keyboard + mouse)
//! - [`serial`] — 16550 UART serial port (COM1)
//! - [`svga`] — Simple VGA/SVGA framebuffer
//! - [`e1000`] — Intel E1000 network card
//! - [`bus`] — PCI configuration space and system bus

pub mod pic;
pub mod pit;
pub mod cmos;
pub mod ps2;
pub mod serial;
pub mod svga;
pub mod e1000;
pub mod bus;
pub mod fw_cfg;
pub mod ide;
pub mod debug_port;
pub mod ioapic;
