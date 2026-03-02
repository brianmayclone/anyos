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
pub struct FlatMemory {
    /// Backing storage.
    data: Vec<u8>,
    /// Logical size in bytes (always equals `data.len()`).
    size: usize,
}

impl FlatMemory {
    /// Allocate `size` bytes of zeroed guest RAM.
    pub fn new(size: usize) -> Self {
        FlatMemory {
            data: vec![0u8; size],
            size,
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
        self.data[offset..end].copy_from_slice(src);
    }

    /// Borrow the entire guest RAM as a byte slice.
    pub fn as_slice(&self) -> &[u8] {
        &self.data
    }

    /// Borrow the entire guest RAM as a mutable byte slice.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.data
    }

    /// Returns the size of guest RAM in bytes.
    pub fn size(&self) -> usize {
        self.size
    }
}

impl MemoryBus for FlatMemory {
    fn read_u8(&self, addr: u64) -> Result<u8> {
        let a = addr as usize;
        if a >= self.size {
            return Ok(0xFF); // floating bus
        }
        Ok(self.data[a])
    }

    fn read_u16(&self, addr: u64) -> Result<u16> {
        let a = addr as usize;
        let end = a.wrapping_add(2);
        if end > self.size || end < a {
            return Ok(0xFFFF); // floating bus
        }
        let bytes: [u8; 2] = [self.data[a], self.data[a + 1]];
        Ok(u16::from_le_bytes(bytes))
    }

    fn read_u32(&self, addr: u64) -> Result<u32> {
        let a = addr as usize;
        let end = a.wrapping_add(4);
        if end > self.size || end < a {
            return Ok(0xFFFF_FFFF); // floating bus
        }
        let bytes: [u8; 4] = [
            self.data[a],
            self.data[a + 1],
            self.data[a + 2],
            self.data[a + 3],
        ];
        Ok(u32::from_le_bytes(bytes))
    }

    fn read_u64(&self, addr: u64) -> Result<u64> {
        let a = addr as usize;
        let end = a.wrapping_add(8);
        if end > self.size || end < a {
            return Ok(0xFFFF_FFFF_FFFF_FFFF); // floating bus
        }
        let bytes: [u8; 8] = [
            self.data[a],
            self.data[a + 1],
            self.data[a + 2],
            self.data[a + 3],
            self.data[a + 4],
            self.data[a + 5],
            self.data[a + 6],
            self.data[a + 7],
        ];
        Ok(u64::from_le_bytes(bytes))
    }

    fn write_u8(&mut self, addr: u64, val: u8) -> Result<()> {
        let a = addr as usize;
        if a >= self.size {
            return Ok(()); // ignore write to unmapped physical memory
        }
        self.data[a] = val;
        Ok(())
    }

    fn write_u16(&mut self, addr: u64, val: u16) -> Result<()> {
        let a = addr as usize;
        let end = a.wrapping_add(2);
        if end > self.size || end < a {
            return Ok(()); // ignore write to unmapped physical memory
        }
        let bytes = val.to_le_bytes();
        self.data[a] = bytes[0];
        self.data[a + 1] = bytes[1];
        Ok(())
    }

    fn write_u32(&mut self, addr: u64, val: u32) -> Result<()> {
        let a = addr as usize;
        let end = a.wrapping_add(4);
        if end > self.size || end < a {
            return Ok(()); // ignore write to unmapped physical memory
        }
        let bytes = val.to_le_bytes();
        self.data[a] = bytes[0];
        self.data[a + 1] = bytes[1];
        self.data[a + 2] = bytes[2];
        self.data[a + 3] = bytes[3];
        Ok(())
    }

    fn write_u64(&mut self, addr: u64, val: u64) -> Result<()> {
        let a = addr as usize;
        let end = a.wrapping_add(8);
        if end > self.size || end < a {
            return Ok(()); // ignore write to unmapped physical memory
        }
        let bytes = val.to_le_bytes();
        self.data[a] = bytes[0];
        self.data[a + 1] = bytes[1];
        self.data[a + 2] = bytes[2];
        self.data[a + 3] = bytes[3];
        self.data[a + 4] = bytes[4];
        self.data[a + 5] = bytes[5];
        self.data[a + 6] = bytes[6];
        self.data[a + 7] = bytes[7];
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
        buf.copy_from_slice(&self.data[a..end]);
        Ok(())
    }

    fn write_bytes(&mut self, addr: u64, buf: &[u8]) -> Result<()> {
        let a = addr as usize;
        let end = a.wrapping_add(buf.len());
        if end > self.size || end < a {
            return Ok(()); // ignore write to unmapped physical memory
        }
        self.data[a..end].copy_from_slice(buf);
        Ok(())
    }
}
