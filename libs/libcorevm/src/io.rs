//! Port I/O emulation framework.
//!
//! Provides a dispatch table mapping I/O port ranges to handler objects.
//! Guest `IN`/`OUT` instructions are routed through [`IoDispatch`] to the
//! registered [`IoHandler`] for that port, or silently handled with default
//! bus behavior (all-ones on read, ignore on write) when no handler exists.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};
use crate::error::Result;

/// Diagnostic counter for total CF8 writes reaching the dispatcher.
static CF8_DISPATCH_COUNT: AtomicU32 = AtomicU32::new(0);

/// Trait implemented by devices that respond to x86 port I/O.
///
/// Each handler covers a contiguous range of ports registered via
/// [`IoDispatch::register`]. The `port` parameter passed to `read`/`write`
/// is the absolute port number (not relative to the region base).
pub trait IoHandler {
    /// Read `size` bytes (1, 2, or 4) from the given I/O port.
    ///
    /// Returns the value zero-extended to `u32`. The handler should only
    /// populate the low `size` bytes of the return value.
    fn read(&mut self, port: u16, size: u8) -> Result<u32>;

    /// Write `size` bytes (1, 2, or 4) to the given I/O port.
    ///
    /// Only the low `size` bytes of `val` are meaningful.
    fn write(&mut self, port: u16, size: u8, val: u32) -> Result<()>;
}

/// A registered I/O port region backed by a handler.
struct IoRegion {
    /// First port in the region (inclusive).
    base: u16,
    /// Number of consecutive ports covered by this region.
    count: u16,
    /// The device handler for this port range.
    handler: Box<dyn IoHandler>,
}

impl IoRegion {
    /// Returns `true` if `port` falls within this region.
    #[inline]
    fn contains(&self, port: u16) -> bool {
        port >= self.base && port < self.base.wrapping_add(self.count)
    }
}

/// Central dispatch table for guest port I/O.
///
/// Devices register their port ranges at VM setup time. During execution,
/// the CPU emulation core calls [`port_in`](IoDispatch::port_in) and
/// [`port_out`](IoDispatch::port_out) which route to the appropriate handler.
pub struct IoDispatch {
    /// Registered I/O regions, searched linearly on each access.
    regions: Vec<IoRegion>,
}

impl IoDispatch {
    /// Create an empty I/O dispatch table with no registered handlers.
    pub fn new() -> Self {
        IoDispatch {
            regions: Vec::new(),
        }
    }

    /// Register a handler for a contiguous range of I/O ports.
    ///
    /// `base` is the first port number and `count` is the number of
    /// consecutive ports handled by `handler`. Overlapping registrations
    /// are not checked; the first matching region wins on lookup.
    pub fn register(&mut self, base: u16, count: u16, handler: Box<dyn IoHandler>) {
        self.regions.push(IoRegion {
            base,
            count,
            handler,
        });
    }

    /// Perform a port read (guest `IN` instruction).
    ///
    /// Searches for a handler covering `port`. If found, delegates to
    /// [`IoHandler::read`]. If no handler is registered, returns the
    /// default x86 bus float value: all bits set for the requested size
    /// (0xFF for byte, 0xFFFF for word, 0xFFFFFFFF for dword).
    pub fn port_in(&mut self, port: u16, size: u8) -> Result<u32> {
        for region in self.regions.iter_mut() {
            if region.contains(port) {
                return region.handler.read(port, size);
            }
        }
        // No handler — return bus float (all ones) matching the access size.
        let val = match size {
            1 => 0xFF,
            2 => 0xFFFF,
            _ => 0xFFFF_FFFF,
        };
        Ok(val)
    }

    /// Perform a port write (guest `OUT` instruction).
    ///
    /// Searches for a handler covering `port`. If found, delegates to
    /// [`IoHandler::write`]. If no handler is registered, the write is
    /// silently ignored (standard x86 bus behavior).
    pub fn port_out(&mut self, port: u16, size: u8, val: u32) -> Result<()> {
        // Diagnostic: track ALL writes to PCI config address port.
        if port == 0xCF8 {
            let n = CF8_DISPATCH_COUNT.fetch_add(1, Ordering::Relaxed);
            // Log writes 15-45 (skip early ones we already see, catch probe phase).
            if n >= 15 && n < 45 {
                libsyscall::serial_print(format_args!(
                    "[io-diag] CF8 dispatch #{}: size={} val=0x{:08X}\n",
                    n, size, val
                ));
            }
        }
        for region in self.regions.iter_mut() {
            if region.contains(port) {
                return region.handler.write(port, size, val);
            }
        }
        // No handler — silently discard the write.
        Ok(())
    }

    /// Return the number of registered I/O regions (diagnostic).
    pub fn region_count(&self) -> usize {
        self.regions.len()
    }
}
