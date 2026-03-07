//! Flat open-addressing hashtable for JIT block lookup.
//!
//! Designed to be accessed directly from emitted JIT code:
//! the entry layout is #[repr(C)] with known field offsets.

use alloc::vec::Vec;

#[repr(C)]
pub struct BlockEntry {
    pub phys_addr: u64, // 0 = empty slot
    pub code_ptr: u64,  // pointer into JIT buffer (as u64)
    pub mode_cs: u64,   // mode (low byte) | cs_base << 8
}

// Compile-time layout assertions
const _: () = {
    assert!(core::mem::size_of::<BlockEntry>() == 24);
    assert!(core::mem::offset_of!(BlockEntry, phys_addr) == 0);
    assert!(core::mem::offset_of!(BlockEntry, code_ptr) == 8);
    assert!(core::mem::offset_of!(BlockEntry, mode_cs) == 16);
};

pub const ENTRY_SIZE: usize = 24;
pub const ENTRY_PHYS_OFF: i32 = 0;
pub const ENTRY_CODE_OFF: i32 = 8;
pub const ENTRY_MODE_CS_OFF: i32 = 16;

const DEFAULT_TABLE_SIZE: usize = 16384; // power of 2
const MAX_PROBES: usize = 4;

/// Multiplicative hash for physical addresses.
/// Spreads sequential addresses across the table.
#[inline]
fn hash_phys(phys: u64, mask: usize) -> usize {
    // Fibonacci hashing: multiply by golden ratio constant, take high bits
    let h = phys.wrapping_mul(0x9E3779B97F4A7C15);
    (h >> 48) as usize & mask
}

pub struct JitLookupTable {
    entries: Vec<BlockEntry>,
    table_size: usize,
}

impl JitLookupTable {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_TABLE_SIZE)
    }

    pub fn with_capacity(size: usize) -> Self {
        assert!(size.is_power_of_two(), "table size must be a power of 2");
        let mut entries = Vec::with_capacity(size);
        for _ in 0..size {
            entries.push(BlockEntry {
                phys_addr: 0,
                code_ptr: 0,
                mode_cs: 0,
            });
        }
        Self {
            entries,
            table_size: size,
        }
    }

    #[inline]
    pub fn make_mode_cs(mode: u8, cs_base: u64) -> u64 {
        (mode as u64) | (cs_base << 8)
    }

    /// Insert a block entry. Returns `true` if inserted, `false` if table is
    /// full at that hash bucket (max 4 probes).
    pub fn insert(&mut self, phys_addr: u64, mode: u8, cs_base: u64, code_ptr: u64) -> bool {
        debug_assert!(phys_addr != 0, "phys_addr 0 is reserved for empty slots");
        let mask = self.table_size - 1;
        let mode_cs = Self::make_mode_cs(mode, cs_base);
        let mut idx = hash_phys(phys_addr, mask);

        for _ in 0..MAX_PROBES {
            let entry = &mut self.entries[idx];
            if entry.phys_addr == 0 || (entry.phys_addr == phys_addr && entry.mode_cs == mode_cs) {
                entry.phys_addr = phys_addr;
                entry.code_ptr = code_ptr;
                entry.mode_cs = mode_cs;
                return true;
            }
            idx = (idx + 1) & mask;
        }

        false
    }

    /// Zero all entries whose phys_addr falls on the given 4K page.
    pub fn invalidate_page(&mut self, page_phys: u64) {
        let page_base = page_phys & !0xFFF;
        for entry in self.entries.iter_mut() {
            if entry.phys_addr != 0 && (entry.phys_addr & !0xFFF) == page_base {
                entry.phys_addr = 0;
                entry.code_ptr = 0;
                entry.mode_cs = 0;
            }
        }
    }

    /// Zero all entries.
    pub fn flush(&mut self) {
        for entry in self.entries.iter_mut() {
            entry.phys_addr = 0;
            entry.code_ptr = 0;
            entry.mode_cs = 0;
        }
    }

    /// Pointer to the entries array, for embedding in JIT code.
    #[inline]
    pub fn entries_ptr(&self) -> *const BlockEntry {
        self.entries.as_ptr()
    }

    /// Mask for index computation (table_size - 1).
    #[inline]
    pub fn mask(&self) -> u32 {
        (self.table_size - 1) as u32
    }

    /// Count of non-empty entries.
    pub fn len(&self) -> usize {
        self.entries.iter().filter(|e| e.phys_addr != 0).count()
    }
}
