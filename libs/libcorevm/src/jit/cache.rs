//! Decode cache for pre-decoded basic blocks.
//!
//! Maps [`BlockKey`] → [`BasicBlock`] using a `BTreeMap` (available in
//! `alloc`, no external dependency needed). When the cache reaches its
//! capacity limit, the oldest half of entries are evicted (simple bulk
//! eviction avoids per-access LRU overhead).

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::sync::Arc;
use alloc::vec::Vec;
use super::block::{BasicBlock, BlockKey};

/// Default maximum number of cached blocks.
const DEFAULT_MAX_ENTRIES: usize = 16384;

/// Cache of pre-decoded basic blocks keyed by physical address + mode.
pub struct DecodeCache {
    /// Cached blocks, keyed by (phys_addr, mode, cs_base).
    blocks: BTreeMap<BlockKey, Arc<BasicBlock>>,
    /// Set of 4K-aligned physical pages that contain at least one cached block.
    /// Used to fast-reject SMC invalidation for non-code pages.
    code_pages: BTreeSet<u64>,
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
            code_pages: BTreeSet::new(),
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
        // Track which pages contain code blocks
        let page = key.phys_addr & !0xFFF;
        self.code_pages.insert(page);
        crate::memory::smc::mark_code_page(page);
        // Also track the end page if the block spans a page boundary
        let end_page = (key.phys_addr + block.byte_len as u64 - 1) & !0xFFF;
        if end_page != page {
            self.code_pages.insert(end_page);
            crate::memory::smc::mark_code_page(end_page);
        }
        self.blocks.insert(key, Arc::new(block));
    }

    /// Invalidate all blocks whose physical address range overlaps a given
    /// page (4 KiB aligned). Used for self-modifying code detection.
    pub fn invalidate_page(&mut self, page_phys: u64) {
        let page_start = page_phys & !0xFFF;
        // Fast reject: if this page has no code blocks, skip the scan
        if !self.code_pages.contains(&page_start) {
            return;
        }
        let page_end = page_start.saturating_add(0x1000);
        self.blocks.retain(|key, block| {
            let block_start = key.phys_addr;
            let block_end = block_start.saturating_add(block.byte_len as u64);
            block_end <= page_start || block_start >= page_end
        });
        // Remove from local code_pages if no blocks remain on this page.
        // Don't touch the global SMC bitmap — keep it conservative.
        let still_has = self.blocks.iter().any(|(key, block)| {
            let s = key.phys_addr;
            let e = s + block.byte_len as u64;
            (s < page_end) && (e > page_start)
        });
        if !still_has {
            self.code_pages.remove(&page_start);
        }
    }

    /// Flush the entire cache (e.g., on CR3 change or mode switch).
    pub fn flush(&mut self) {
        self.blocks.clear();
        self.code_pages.clear();
        // Don't unmark the global SMC bitmap — the JIT engine may still
        // have compiled blocks on these pages. The bitmap is conservative.
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
        // Rebuild code_pages from remaining blocks
        self.code_pages.clear();
        for (key, block) in &self.blocks {
            let page = key.phys_addr & !0xFFF;
            self.code_pages.insert(page);
            let end_page = (key.phys_addr + block.byte_len as u64 - 1) & !0xFFF;
            if end_page != page {
                self.code_pages.insert(end_page);
            }
        }
    }
}
