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
///
/// A cached `min_base` / `max_end` pair provides fast rejection for
/// addresses that fall entirely outside any MMIO region.
pub struct MmioDispatch {
    /// Registered MMIO regions, searched in insertion order.
    regions: Vec<MmioRegion>,
    /// Lowest base address across all regions (for fast rejection).
    min_base: u64,
    /// Highest end address (base + size) across all regions.
    max_end: u64,
}

impl MmioDispatch {
    /// Create an empty MMIO dispatcher with no registered regions.
    pub fn new() -> Self {
        MmioDispatch {
            regions: Vec::new(),
            min_base: u64::MAX,
            max_end: 0,
        }
    }

    /// Register an MMIO region.
    ///
    /// `base` is the starting physical address and `size` is the length in
    /// bytes. The caller must ensure regions do not overlap.
    pub fn register(&mut self, base: u64, size: u64, handler: Box<dyn MmioHandler>) {
        if base < self.min_base {
            self.min_base = base;
        }
        let end = base + size;
        if end > self.max_end {
            self.max_end = end;
        }
        self.regions.push(MmioRegion {
            base,
            size,
            handler,
        });
    }

    /// Find the MMIO region containing `addr`, if any.
    ///
    /// Returns a mutable reference so the caller can invoke the handler's
    /// `read` or `write` method. Fast-rejects addresses outside the
    /// aggregate MMIO range.
    /// Return the number of registered MMIO regions.
    pub fn region_count(&self) -> usize {
        self.regions.len()
    }

    /// Return the fast-reject bounds for diagnostics.
    pub fn bounds(&self) -> (u64, u64) {
        (self.min_base, self.max_end)
    }

    #[inline]
    pub fn find(&mut self, addr: u64) -> Option<&mut MmioRegion> {
        // Fast rejection: skip linear scan if address is outside all MMIO regions.
        if addr < self.min_base || addr >= self.max_end {
            return None;
        }
        self.regions
            .iter_mut()
            .find(|r| addr >= r.base && addr < r.base + r.size)
    }
}
