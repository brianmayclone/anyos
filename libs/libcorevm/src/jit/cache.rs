//! Decode cache for pre-decoded basic blocks.
//!
//! Maps [`BlockKey`] → [`BasicBlock`] using a `BTreeMap` (available in
//! `alloc`, no external dependency needed). When the cache reaches its
//! capacity limit, the oldest half of entries are evicted (simple bulk
//! eviction avoids per-access LRU overhead).

use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use super::block::{BasicBlock, BlockKey};

/// Default maximum number of cached blocks.
const DEFAULT_MAX_ENTRIES: usize = 16384;

/// Cache of pre-decoded basic blocks keyed by physical address + mode.
pub struct DecodeCache {
    /// Cached blocks, keyed by (phys_addr, mode, cs_base).
    blocks: BTreeMap<BlockKey, Arc<BasicBlock>>,
    /// Maximum number of entries before eviction.
    max_entries: usize,
    /// Cache hit counter (diagnostics).
    hits: u64,
    /// Cache miss counter (diagnostics).
    misses: u64,
}

impl DecodeCache {
    /// Create a new decode cache with the default capacity.
    pub fn new() -> Self {
        DecodeCache {
            blocks: BTreeMap::new(),
            max_entries: DEFAULT_MAX_ENTRIES,
            hits: 0,
            misses: 0,
        }
    }

    /// Look up a cached basic block by its key.
    ///
    /// Returns `Some(Arc<BasicBlock>)` on hit, `None` on miss.
    #[inline]
    pub fn lookup(&mut self, key: &BlockKey) -> Option<Arc<BasicBlock>> {
        if let Some(block) = self.blocks.get(key) {
            self.hits += 1;
            Some(Arc::clone(block))
        } else {
            self.misses += 1;
            None
        }
    }

    /// Insert a decoded basic block into the cache.
    ///
    /// If the cache is full, evicts the oldest half of entries.
    pub fn insert(&mut self, key: BlockKey, block: BasicBlock) {
        if self.blocks.len() >= self.max_entries {
            self.evict();
        }
        self.blocks.insert(key, Arc::new(block));
    }

    /// Invalidate all blocks whose physical address range overlaps a given
    /// page (4 KiB aligned). Used for self-modifying code detection.
    pub fn invalidate_page(&mut self, page_phys: u64) {
        let page_start = page_phys & !0xFFF;
        let page_end = page_start + 0x1000;
        // Collect keys to remove (can't mutate during iteration).
        let to_remove: Vec<BlockKey> = self
            .blocks
            .range(
                BlockKey { phys_addr: page_start, mode: crate::decoder::CpuMode::Real16, cs_base: 0 }
                    ..BlockKey { phys_addr: page_end, mode: crate::decoder::CpuMode::Real16, cs_base: 0 },
            )
            .map(|(k, _)| *k)
            .collect();
        for key in to_remove {
            self.blocks.remove(&key);
        }
    }

    /// Flush the entire cache (e.g., on CR3 change or mode switch).
    pub fn flush(&mut self) {
        self.blocks.clear();
    }

    /// Return the number of cached blocks.
    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    /// Return cache hit count.
    pub fn hits(&self) -> u64 {
        self.hits
    }

    /// Return cache miss count.
    pub fn misses(&self) -> u64 {
        self.misses
    }

    /// Evict the first half of entries (by key order = lowest addresses).
    ///
    /// Simple bulk eviction: removes the lower half of the address space,
    /// which is cheap (O(n/2) BTree splits) and avoids per-access overhead.
    fn evict(&mut self) {
        let half = self.blocks.len() / 2;
        let keys: Vec<BlockKey> = self.blocks.keys().take(half).copied().collect();
        for key in keys {
            self.blocks.remove(&key);
        }
    }
}
