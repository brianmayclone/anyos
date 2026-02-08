//! Typed wrappers for physical and virtual addresses.
//!
//! Provides [`PhysAddr`] and [`VirtAddr`] newtypes to prevent accidental mixing
//! of address spaces, along with alignment and indexing helpers for 4 KiB frames.

use crate::memory::FRAME_SIZE;

/// A 32-bit physical memory address.
///
/// Wraps a raw `u32` to distinguish physical addresses from virtual ones at the
/// type level. Provides frame-alignment and indexing utilities.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct PhysAddr(pub u32);

/// A 32-bit virtual memory address.
///
/// Wraps a raw `u32` to distinguish virtual addresses from physical ones at the
/// type level. Provides page directory/table index extraction for x86 two-level paging.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct VirtAddr(pub u32);

impl PhysAddr {
    /// Create a new physical address from a raw `u32` value.
    pub const fn new(addr: u32) -> Self {
        PhysAddr(addr)
    }

    /// Return the raw `u32` value of this physical address.
    pub const fn as_u32(self) -> u32 {
        self.0
    }

    /// Return the frame index (address divided by `FRAME_SIZE`).
    pub fn frame_index(self) -> usize {
        (self.0 as usize) / FRAME_SIZE
    }

    /// Returns `true` if this address is aligned to a 4 KiB frame boundary.
    pub fn is_frame_aligned(self) -> bool {
        self.0 % FRAME_SIZE as u32 == 0
    }

    /// Round this address down to the nearest frame boundary.
    pub fn frame_align_down(self) -> Self {
        PhysAddr(self.0 & !(FRAME_SIZE as u32 - 1))
    }

    /// Round this address up to the nearest frame boundary.
    pub fn frame_align_up(self) -> Self {
        PhysAddr((self.0 + FRAME_SIZE as u32 - 1) & !(FRAME_SIZE as u32 - 1))
    }
}

impl VirtAddr {
    /// Create a new virtual address from a raw `u32` value.
    pub const fn new(addr: u32) -> Self {
        VirtAddr(addr)
    }

    /// Return the raw `u32` value of this virtual address.
    pub const fn as_u32(self) -> u32 {
        self.0
    }

    /// Extract the page directory index (bits 22-31) for x86 two-level paging.
    pub fn page_directory_index(self) -> usize {
        ((self.0 >> 22) & 0x3FF) as usize
    }

    /// Extract the page table index (bits 12-21) for x86 two-level paging.
    pub fn page_table_index(self) -> usize {
        ((self.0 >> 12) & 0x3FF) as usize
    }

    /// Extract the offset within the page (bits 0-11).
    pub fn page_offset(self) -> usize {
        (self.0 & 0xFFF) as usize
    }

    /// Returns `true` if this address is aligned to a 4 KiB page boundary.
    pub fn is_page_aligned(self) -> bool {
        self.0 % FRAME_SIZE as u32 == 0
    }
}
