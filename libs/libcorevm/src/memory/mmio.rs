//! Memory-mapped I/O dispatch layer.
//!
//! MMIO regions intercept reads and writes to specific physical address ranges
//! and route them to device handlers instead of guest RAM. This allows
//! emulated devices (PCI BARs, APIC, IOAPIC, etc.) to be memory-mapped
//! without any changes to the core memory bus logic.

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::error::Result;

/// Trait implemented by device models that handle MMIO accesses.
///
/// Each handler covers a contiguous physical address range. The `offset`
/// parameter passed to `read` and `write` is relative to the region base,
/// not an absolute physical address.
pub trait MmioHandler {
    /// Read `size` bytes (1, 2, 4, or 8) from `offset` within the region.
    ///
    /// Returns the value zero-extended to `u64`.
    fn read(&mut self, offset: u64, size: u8) -> Result<u64>;

    /// Write `size` bytes (1, 2, 4, or 8) of `val` to `offset` within the region.
    fn write(&mut self, offset: u64, size: u8, val: u64) -> Result<()>;
}

/// A registered MMIO region binding an address range to a handler.
pub struct MmioRegion {
    /// Base physical address of this MMIO region.
    pub base: u64,
    /// Size of the region in bytes.
    pub size: u64,
    /// Device handler for accesses within this region.
    pub handler: Box<dyn MmioHandler>,
}

/// Dispatcher that routes physical addresses to the correct MMIO handler.
///
/// Regions must not overlap. The dispatcher performs a linear scan, which
/// is efficient for the small number of MMIO regions typical in an emulated
/// PC (usually fewer than 16).
pub struct MmioDispatch {
    /// Registered MMIO regions, searched in insertion order.
    regions: Vec<MmioRegion>,
}

impl MmioDispatch {
    /// Create an empty MMIO dispatcher with no registered regions.
    pub fn new() -> Self {
        MmioDispatch {
            regions: Vec::new(),
        }
    }

    /// Register an MMIO region.
    ///
    /// `base` is the starting physical address and `size` is the length in
    /// bytes. The caller must ensure regions do not overlap.
    pub fn register(&mut self, base: u64, size: u64, handler: Box<dyn MmioHandler>) {
        self.regions.push(MmioRegion {
            base,
            size,
            handler,
        });
    }

    /// Find the MMIO region containing `addr`, if any.
    ///
    /// Returns a mutable reference so the caller can invoke the handler's
    /// `read` or `write` method.
    pub fn find(&mut self, addr: u64) -> Option<&mut MmioRegion> {
        self.regions
            .iter_mut()
            .find(|r| addr >= r.base && addr < r.base + r.size)
    }
}
