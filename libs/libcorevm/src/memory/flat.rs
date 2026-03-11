//! Flat guest physical memory backed by a contiguous byte vector.
//!
//! `FlatMemory` is the simplest guest RAM implementation: a single zeroed
//! allocation that maps guest physical addresses 1:1 to host offsets.
//! Out-of-bounds reads return `0xFF` (floating bus), matching real x86
//! hardware behavior for accesses to unmapped physical address space.
//! Out-of-bounds writes are silently ignored.

use alloc::vec;
use alloc::vec::Vec;

use super::MemoryBus;
use crate::error::Result;

/// Flat, contiguous guest physical memory.
///
/// Addresses `0..size` are valid; anything beyond is out-of-bounds.
/// All multi-byte reads and writes use little-endian byte order,
/// matching the x86 memory model.
///
/// On `std` targets, memory is allocated with 4KB page alignment
/// (required by KVM and WHP for guest RAM mapping).
pub struct FlatMemory {
    /// Backing storage.
    #[cfg(not(feature = "std"))]
    data: Vec<u8>,
    #[cfg(feature = "std")]
    data: AlignedBuffer,
    /// Logical size in bytes.
    size: usize,
}

/// Page-aligned memory buffer for hypervisor backends.
#[cfg(feature = "std")]
struct AlignedBuffer {
    ptr: *mut u8,
    len: usize,
}

#[cfg(feature = "std")]
impl AlignedBuffer {
    fn new(size: usize) -> Self {
        use core::alloc::Layout;
        let layout = Layout::from_size_align(size, 4096).expect("invalid layout");
        let ptr = unsafe { alloc::alloc::alloc_zeroed(layout) };
        if ptr.is_null() {
            panic!("Failed to allocate {} bytes of page-aligned guest RAM", size);
        }
        AlignedBuffer { ptr, len: size }
    }

    fn as_slice(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.ptr, self.len) }
    }

    fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.ptr, self.len) }
    }
}

#[cfg(feature = "std")]
impl Drop for AlignedBuffer {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            use core::alloc::Layout;
            let layout = Layout::from_size_align(self.len, 4096).unwrap();
            unsafe { alloc::alloc::dealloc(self.ptr, layout); }
        }
    }
}

impl FlatMemory {
    /// Allocate `size` bytes of zeroed guest RAM.
    pub fn new(size: usize) -> Self {
        // Round up to page boundary for hypervisor compatibility
        let aligned_size = (size + 4095) & !4095;
        #[cfg(not(feature = "std"))]
        let data = vec![0u8; aligned_size];
        #[cfg(feature = "std")]
        let data = AlignedBuffer::new(aligned_size);
        FlatMemory {
            data,
            size: aligned_size,
        }
    }

    /// Copy `data` into guest memory starting at `offset`.
    ///
    /// # Panics
    ///
    /// Panics if `offset + data.len()` exceeds the memory size.
    pub fn load_at(&mut self, offset: usize, src: &[u8]) {
        let end = offset + src.len();
        assert!(
            end <= self.size,
            "load_at: offset 0x{:X} + len 0x{:X} exceeds memory size 0x{:X}",
            offset,
            src.len(),
            self.size,
        );
        self.as_mut_slice()[offset..end].copy_from_slice(src);
    }

    /// Borrow the entire guest RAM as a byte slice.
    pub fn as_slice(&self) -> &[u8] {
        #[cfg(not(feature = "std"))]
        { &self.data }
        #[cfg(feature = "std")]
        { self.data.as_slice() }
    }

    /// Borrow the entire guest RAM as a mutable byte slice.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        #[cfg(not(feature = "std"))]
        { &mut self.data }
        #[cfg(feature = "std")]
        { self.data.as_mut_slice() }
    }

    /// Returns the size of guest RAM in bytes.
    pub fn size(&self) -> usize {
        self.size
    }
}

impl MemoryBus for FlatMemory {
    fn read_u8(&self, addr: u64) -> Result<u8> {
        let a = addr as usize;
        let s = self.as_slice();
        if a >= self.size {
            return Ok(0xFF); // floating bus
        }
        Ok(s[a])
    }

    fn read_u16(&self, addr: u64) -> Result<u16> {
        let a = addr as usize;
        let end = a.wrapping_add(2);
        if end > self.size || end < a {
            return Ok(0xFFFF); // floating bus
        }
        let s = self.as_slice();
        let bytes: [u8; 2] = [s[a], s[a + 1]];
        Ok(u16::from_le_bytes(bytes))
    }

    fn read_u32(&self, addr: u64) -> Result<u32> {
        let a = addr as usize;
        let end = a.wrapping_add(4);
        if end > self.size || end < a {
            return Ok(0xFFFF_FFFF); // floating bus
        }
        let s = self.as_slice();
        let bytes: [u8; 4] = [s[a], s[a + 1], s[a + 2], s[a + 3]];
        Ok(u32::from_le_bytes(bytes))
    }

    fn read_u64(&self, addr: u64) -> Result<u64> {
        let a = addr as usize;
        let end = a.wrapping_add(8);
        if end > self.size || end < a {
            return Ok(0xFFFF_FFFF_FFFF_FFFF); // floating bus
        }
        let s = self.as_slice();
        let bytes: [u8; 8] = [s[a], s[a+1], s[a+2], s[a+3], s[a+4], s[a+5], s[a+6], s[a+7]];
        Ok(u64::from_le_bytes(bytes))
    }

    fn write_u8(&mut self, addr: u64, val: u8) -> Result<()> {
        let a = addr as usize;
        if a >= self.size {
            return Ok(()); // ignore write to unmapped physical memory
        }
        self.as_mut_slice()[a] = val;
        Ok(())
    }

    fn write_u16(&mut self, addr: u64, val: u16) -> Result<()> {
        let a = addr as usize;
        let end = a.wrapping_add(2);
        if end > self.size || end < a {
            return Ok(()); // ignore write to unmapped physical memory
        }
        let bytes = val.to_le_bytes();
        let s = self.as_mut_slice();
        s[a] = bytes[0];
        s[a + 1] = bytes[1];
        Ok(())
    }

    fn write_u32(&mut self, addr: u64, val: u32) -> Result<()> {
        let a = addr as usize;
        let end = a.wrapping_add(4);
        if end > self.size || end < a {
            return Ok(()); // ignore write to unmapped physical memory
        }
        let bytes = val.to_le_bytes();
        let s = self.as_mut_slice();
        s[a] = bytes[0];
        s[a + 1] = bytes[1];
        s[a + 2] = bytes[2];
        s[a + 3] = bytes[3];
        Ok(())
    }

    fn write_u64(&mut self, addr: u64, val: u64) -> Result<()> {
        let a = addr as usize;
        let end = a.wrapping_add(8);
        if end > self.size || end < a {
            return Ok(()); // ignore write to unmapped physical memory
        }
        let bytes = val.to_le_bytes();
        let s = self.as_mut_slice();
        s[a] = bytes[0];
        s[a + 1] = bytes[1];
        s[a + 2] = bytes[2];
        s[a + 3] = bytes[3];
        s[a + 4] = bytes[4];
        s[a + 5] = bytes[5];
        s[a + 6] = bytes[6];
        s[a + 7] = bytes[7];
        Ok(())
    }

    fn read_bytes(&self, addr: u64, buf: &mut [u8]) -> Result<()> {
        let a = addr as usize;
        let end = a.wrapping_add(buf.len());
        if end > self.size || end < a {
            // Fill with 0xFF for unmapped physical memory
            buf.fill(0xFF);
            return Ok(());
        }
        buf.copy_from_slice(&self.as_slice()[a..end]);
        Ok(())
    }

    fn write_bytes(&mut self, addr: u64, buf: &[u8]) -> Result<()> {
        let a = addr as usize;
        let end = a.wrapping_add(buf.len());
        if end > self.size || end < a {
            return Ok(()); // ignore write to unmapped physical memory
        }
        self.as_mut_slice()[a..end].copy_from_slice(buf);
        Ok(())
    }
}
