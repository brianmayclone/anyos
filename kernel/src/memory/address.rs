//! Typed wrappers for physical and virtual addresses.
//!
//! Provides [`PhysAddr`] and [`VirtAddr`] newtypes to prevent accidental mixing
//! of address spaces, along with alignment and indexing helpers for 4 KiB frames.

use crate::memory::FRAME_SIZE;

/// A 64-bit physical memory address.
///
/// Wraps a raw `u64` to distinguish physical addresses from virtual ones at the
/// type level. Provides frame-alignment and indexing utilities.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct PhysAddr(pub u64);

/// A 64-bit virtual memory address.
///
/// Wraps a raw `u64` to distinguish virtual addresses from physical ones at the
/// type level. Provides page table index extraction for x86-64 four-level paging.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct VirtAddr(pub u64);

impl PhysAddr {
    /// Create a new physical address from a raw `u64` value.
    pub const fn new(addr: u64) -> Self {
        PhysAddr(addr)
    }

    /// Return the raw `u64` value of this physical address.
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    /// Return the raw value truncated to `u32` (for legacy interfaces).
    pub const fn as_u32(self) -> u32 {
        self.0 as u32
    }

    /// Return the frame index (address divided by `FRAME_SIZE`).
    pub fn frame_index(self) -> usize {
        (self.0 as usize) / FRAME_SIZE
    }

    /// Returns `true` if this address is aligned to a 4 KiB frame boundary.
    pub fn is_frame_aligned(self) -> bool {
        self.0 % FRAME_SIZE as u64 == 0
    }

    /// Round this address down to the nearest frame boundary.
    pub fn frame_align_down(self) -> Self {
        PhysAddr(self.0 & !(FRAME_SIZE as u64 - 1))
    }

    /// Round this address up to the nearest frame boundary.
    pub fn frame_align_up(self) -> Self {
        PhysAddr((self.0 + FRAME_SIZE as u64 - 1) & !(FRAME_SIZE as u64 - 1))
    }
}

impl VirtAddr {
    /// Create a new virtual address from a raw `u64` value.
    pub const fn new(addr: u64) -> Self {
        VirtAddr(addr)
    }

    /// Return the raw `u64` value of this virtual address.
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    /// Return the raw value truncated to `u32` (for legacy interfaces).
    pub const fn as_u32(self) -> u32 {
        self.0 as u32
    }

    /// Extract the PML4 index (bits 39-47) for x86-64 four-level paging.
    pub fn pml4_index(self) -> usize {
        ((self.0 >> 39) & 0x1FF) as usize
    }

    /// Extract the PDPT index (bits 30-38) for x86-64 four-level paging.
    pub fn pdpt_index(self) -> usize {
        ((self.0 >> 30) & 0x1FF) as usize
    }

    /// Extract the page directory index (bits 21-29) for x86-64 four-level paging.
    pub fn pd_index(self) -> usize {
        ((self.0 >> 21) & 0x1FF) as usize
    }

    /// Extract the page table index (bits 12-20) for x86-64 four-level paging.
    pub fn pt_index(self) -> usize {
        ((self.0 >> 12) & 0x1FF) as usize
    }

    /// Extract the offset within the page (bits 0-11).
    pub fn page_offset(self) -> usize {
        (self.0 & 0xFFF) as usize
    }

    /// Returns `true` if this address is aligned to a 4 KiB page boundary.
    pub fn is_page_aligned(self) -> bool {
        self.0 % FRAME_SIZE as u64 == 0
    }
}
