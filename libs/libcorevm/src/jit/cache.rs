//! Decode cache for pre-decoded basic blocks.
//!
//! Maps [`BlockKey`] → [`BasicBlock`] using an open-addressing hash table
//! for O(1) lookup in the hot path. When the cache reaches its capacity
//! limit, it is flushed entirely.

use alloc::sync::Arc;
use alloc::vec::Vec;
use super::block::{BasicBlock, BlockKey};

/// Hash table capacity — must be a power of 2.
const TABLE_SIZE: usize = 32768;
const TABLE_MASK: usize = TABLE_SIZE - 1;

/// Maximum number of cached blocks before flush.
const MAX_ENTRIES: usize = 16384;

/// A slot in the hash table.
struct Slot {
    key: BlockKey,
    block: Arc<BasicBlock>,
    occupied: bool,
}

impl Slot {
    const fn empty() -> Self {
        Slot {
            key: BlockKey {
                phys_addr: 0,
                mode: crate::decoder::CpuMode::Real16,
                cs_base: 0,
            },
            block: unsafe { core::mem::MaybeUninit::uninit().assume_init() },
            occupied: false,
        }
    }
}

/// Cache of pre-decoded basic blocks keyed by physical address + mode.
pub struct DecodeCache {
    /// Hash table slots.
    table: Vec<Slot>,
    /// Number of occupied slots.
    count: usize,
    /// Set of 4K-aligned physical pages that contain at least one cached block.
    /// Simple sorted Vec for SMC invalidation (rare path).
    code_pages: Vec<u64>,
    /// Cache hit counter (diagnostics).
    hits: u64,
    /// Cache miss counter (diagnostics).
    misses: u64,
}

#[inline(always)]
fn hash_key(key: &BlockKey) -> usize {
    // Fibonacci hashing on phys_addr, mixed with mode.
    let h = key.phys_addr.wrapping_mul(11400714819323198485) ^ (key.cs_base.wrapping_mul(2654435769)) ^ (key.mode as u64);
    (h >> 49) as usize & TABLE_MASK
}

impl DecodeCache {
    /// Create a new decode cache.
    pub fn new() -> Self {
        let mut table = Vec::with_capacity(TABLE_SIZE);
        for _ in 0..TABLE_SIZE {
            table.push(Slot {
                key: BlockKey {
                    phys_addr: u64::MAX,
                    mode: crate::decoder::CpuMode::Real16,
                    cs_base: 0,
                },
                block: Arc::new(BasicBlock {
                    instructions: Vec::new(),
                    byte_len: 0,
                    exits_with_branch: false,
                }),
                occupied: false,
            });
        }
        DecodeCache {
            table,
            count: 0,
            code_pages: Vec::new(),
            hits: 0,
            misses: 0,
        }
    }

    /// Look up a cached basic block by its key.
    #[inline(always)]
    pub fn lookup(&mut self, key: &BlockKey) -> Option<Arc<BasicBlock>> {
        let mut idx = hash_key(key);
        // Linear probing with a small limit to avoid long chains.
        for _ in 0..8 {
            let slot = unsafe { self.table.get_unchecked(idx) };
            if !slot.occupied {
                self.misses += 1;
                return None;
            }
            if slot.key.phys_addr == key.phys_addr
                && slot.key.mode == key.mode
                && slot.key.cs_base == key.cs_base
            {
                self.hits += 1;
                return Some(Arc::clone(&slot.block));
            }
            idx = (idx + 1) & TABLE_MASK;
        }
        self.misses += 1;
        None
    }

    /// Insert a decoded basic block into the cache.
    pub fn insert(&mut self, key: BlockKey, block: BasicBlock) {
        if self.count >= MAX_ENTRIES {
            self.flush();
        }

        // Track code pages
        let page = key.phys_addr & !0xFFF;
        insert_sorted(&mut self.code_pages, page);
        crate::memory::smc::mark_code_page(page);
        let end_page = (key.phys_addr + block.byte_len as u64 - 1) & !0xFFF;
        if end_page != page {
            insert_sorted(&mut self.code_pages, end_page);
            crate::memory::smc::mark_code_page(end_page);
        }

        let arc = Arc::new(block);
        let mut idx = hash_key(&key);
        for _ in 0..8 {
            let slot = &mut self.table[idx];
            if !slot.occupied {
                slot.key = key;
                slot.block = arc;
                slot.occupied = true;
                self.count += 1;
                return;
            }
            // Replace existing entry with same key.
            if slot.key.phys_addr == key.phys_addr
                && slot.key.mode == key.mode
                && slot.key.cs_base == key.cs_base
            {
                slot.block = arc;
                return;
            }
            idx = (idx + 1) & TABLE_MASK;
        }
        // Probe chain full — replace first slot.
        let slot = &mut self.table[hash_key(&key)];
        slot.key = key;
        slot.block = arc;
        slot.occupied = true;
    }

    /// Invalidate all blocks on a given page.
    pub fn invalidate_page(&mut self, page_phys: u64) {
        let page_start = page_phys & !0xFFF;
        // Fast reject
        if self.code_pages.binary_search(&page_start).is_err() {
            return;
        }
        let page_end = page_start + 0x1000;
        for slot in self.table.iter_mut() {
            if slot.occupied {
                let s = slot.key.phys_addr;
                let e = s + slot.block.byte_len as u64;
                if s < page_end && e > page_start {
                    slot.occupied = false;
                    self.count = self.count.saturating_sub(1);
                }
            }
        }
        // Check if page still has blocks
        let still_has = self.table.iter().any(|slot| {
            if !slot.occupied { return false; }
            let s = slot.key.phys_addr;
            let e = s + slot.block.byte_len as u64;
            s < page_end && e > page_start
        });
        if !still_has {
            if let Ok(idx) = self.code_pages.binary_search(&page_start) {
                self.code_pages.remove(idx);
            }
        }
    }

    /// Flush the entire cache.
    pub fn flush(&mut self) {
        for slot in self.table.iter_mut() {
            slot.occupied = false;
        }
        self.count = 0;
        self.code_pages.clear();
    }

    pub fn len(&self) -> usize {
        self.count
    }

    pub fn hits(&self) -> u64 {
        self.hits
    }

    pub fn misses(&self) -> u64 {
        self.misses
    }
}

fn insert_sorted(v: &mut Vec<u64>, val: u64) {
    match v.binary_search(&val) {
        Ok(_) => {} // already present
        Err(pos) => v.insert(pos, val),
    }
}
