//! Block-level disk I/O cache with LRU read cache and write-back buffering.
//!
//! The cache sits between the AHCI device and the host file descriptor,
//! intercepting `read_at` / `write_at` calls.  It maintains:
//!
//! - **Read cache**: LRU cache of recently-read blocks.  Hits avoid a host
//!   syscall entirely.
//! - **Write-back buffer**: Dirty blocks are held in memory and flushed to
//!   the host periodically (or when the buffer is full).  This coalesces
//!   many small writes into fewer large writes.
//! - **Write-through mode**: Every write goes to host immediately (safe but
//!   slower).  Selected via `CacheMode::WriteThrough`.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

/// Block size for the cache (4 KiB = 8 sectors).
const CACHE_BLOCK_SIZE: usize = 4096;
/// Block size as u64 for offset math.
const BLOCK_SIZE_U64: u64 = CACHE_BLOCK_SIZE as u64;

/// Cache operating mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheMode {
    /// Writes are buffered and flushed periodically.  Best throughput.
    WriteBack,
    /// Writes go to host immediately.  Reads are still cached.  Safest.
    WriteThrough,
    /// No caching at all — every access hits the host.
    None,
}

impl Default for CacheMode {
    fn default() -> Self { CacheMode::WriteBack }
}

/// Per-block metadata.
struct CacheBlock {
    data: Vec<u8>,
    dirty: bool,
    /// Simple LRU counter — higher = more recently used.
    access_tick: u64,
}

/// Block-level cache for a single disk image.
pub struct DiskCache {
    mode: CacheMode,
    blocks: BTreeMap<u64, CacheBlock>, // key = block index (offset / BLOCK_SIZE)
    max_blocks: usize,
    tick: u64,
    dirty_count: usize,
    /// Maximum dirty blocks before forced flush.
    max_dirty: usize,
    /// Stats
    pub read_hits: u64,
    pub read_misses: u64,
    pub write_hits: u64,
    pub write_new: u64,
    pub flushes: u64,
}

impl DiskCache {
    /// Create a new cache.
    ///
    /// - `cache_mb`: Total cache size in MiB (0 = disabled).
    /// - `mode`: Write caching strategy.
    pub fn new(cache_mb: u32, mode: CacheMode) -> Self {
        let cache_mb = if mode == CacheMode::None { 0 } else { cache_mb };
        let max_blocks = if cache_mb == 0 { 0 } else {
            (cache_mb as usize) * 1024 * 1024 / CACHE_BLOCK_SIZE
        };
        // Flush when 50% of cache is dirty
        let max_dirty = (max_blocks / 2).max(64);
        DiskCache {
            mode,
            blocks: BTreeMap::new(),
            max_blocks,
            tick: 0,
            dirty_count: 0,
            max_dirty,
            read_hits: 0,
            read_misses: 0,
            write_hits: 0,
            write_new: 0,
            flushes: 0,
        }
    }

    /// Check if caching is enabled.
    #[inline]
    pub fn enabled(&self) -> bool { self.max_blocks > 0 }

    /// Read `buf.len()` bytes from offset `offset`.
    ///
    /// Returns `true` if fully served from cache, `false` if the caller
    /// must perform the actual host read (the cache is populated afterwards).
    pub fn read(&mut self, offset: u64, buf: &mut [u8]) -> bool {
        if !self.enabled() { return false; }

        let block_idx = offset / BLOCK_SIZE_U64;
        let block_off = (offset % BLOCK_SIZE_U64) as usize;

        // Only handle single-block requests from cache
        if block_off + buf.len() > CACHE_BLOCK_SIZE {
            return false;
        }

        self.tick += 1;

        if let Some(block) = self.blocks.get_mut(&block_idx) {
            block.access_tick = self.tick;
            buf.copy_from_slice(&block.data[block_off..block_off + buf.len()]);
            self.read_hits += 1;
            true
        } else {
            self.read_misses += 1;
            false
        }
    }

    /// Insert a block into the read cache after a host read.
    ///
    /// Call this after `read()` returns `false` and you've read the data
    /// from the host.  `block_data` must be exactly `CACHE_BLOCK_SIZE` bytes
    /// starting at `block_offset` (aligned to CACHE_BLOCK_SIZE).
    pub fn populate_read(&mut self, block_offset: u64, block_data: &[u8]) {
        if !self.enabled() || block_data.len() != CACHE_BLOCK_SIZE { return; }
        let block_idx = block_offset / BLOCK_SIZE_U64;

        self.tick += 1;
        self.evict_if_full();

        self.blocks.entry(block_idx).or_insert_with(|| CacheBlock {
            data: block_data.to_vec(),
            dirty: false,
            access_tick: self.tick,
        });
    }

    /// Write `buf.len()` bytes at offset `offset`.
    ///
    /// - `WriteBack`: data is buffered; returns `true` (caller skips host write).
    /// - `WriteThrough`: data is cached and updated; returns `false` (caller must also write to host).
    /// - `None`: returns `false`.
    pub fn write(&mut self, offset: u64, buf: &[u8]) -> bool {
        if !self.enabled() { return false; }

        let block_idx = offset / BLOCK_SIZE_U64;
        let block_off = (offset % BLOCK_SIZE_U64) as usize;

        if block_off + buf.len() > CACHE_BLOCK_SIZE {
            // Spans multiple blocks — invalidate all affected cached blocks
            // so stale data is never returned on subsequent reads.
            let first_block = offset / BLOCK_SIZE_U64;
            let last_block = (offset + buf.len() as u64 - 1) / BLOCK_SIZE_U64;
            for bi in first_block..=last_block {
                if let Some(b) = self.blocks.remove(&bi) {
                    if b.dirty { self.dirty_count = self.dirty_count.saturating_sub(1); }
                }
            }
            return false;
        }

        self.tick += 1;

        if let Some(block) = self.blocks.get_mut(&block_idx) {
            // Update existing cached block
            block.data[block_off..block_off + buf.len()].copy_from_slice(buf);
            block.access_tick = self.tick;
            if self.mode == CacheMode::WriteBack && !block.dirty {
                block.dirty = true;
                self.dirty_count += 1;
            }
            self.write_hits += 1;
        } else if block_off == 0 && buf.len() == CACHE_BLOCK_SIZE {
            // Full-block write — we have all the data, safe to cache
            self.evict_if_full();
            let dirty = self.mode == CacheMode::WriteBack;
            self.blocks.insert(block_idx, CacheBlock {
                data: buf.to_vec(),
                dirty,
                access_tick: self.tick,
            });
            if dirty { self.dirty_count += 1; }
            self.write_new += 1;
        } else {
            // Partial block write to a block NOT in cache.
            // We don't have the rest of the block data, so we cannot
            // cache this safely (the unwritten bytes would be wrong).
            // Just let it go to the host.
            return false;
        }

        // In WriteBack mode, skip the host write (return true)
        self.mode == CacheMode::WriteBack
    }

    /// Collect all dirty blocks that need flushing.
    ///
    /// Returns a Vec of (byte_offset, data) pairs.  The caller must write
    /// these to the host and then call `mark_flushed()`.
    pub fn collect_dirty(&mut self) -> Vec<(u64, Vec<u8>)> {
        let mut result = Vec::new();
        for (&block_idx, block) in self.blocks.iter_mut() {
            if block.dirty {
                result.push((block_idx * BLOCK_SIZE_U64, block.data.clone()));
                block.dirty = false;
            }
        }
        self.dirty_count = 0;
        self.flushes += 1;
        result
    }

    /// Check if a flush is needed (too many dirty blocks).
    #[inline]
    pub fn needs_flush(&self) -> bool {
        self.mode == CacheMode::WriteBack && self.dirty_count >= self.max_dirty
    }

    /// Number of dirty (unflushed) blocks.
    #[inline]
    pub fn dirty_count(&self) -> usize { self.dirty_count }

    /// Evict the least-recently-used clean block if cache is full.
    fn evict_if_full(&mut self) {
        if self.blocks.len() < self.max_blocks { return; }

        // Find the LRU clean block
        let mut lru_idx = None;
        let mut lru_tick = u64::MAX;
        for (&idx, block) in &self.blocks {
            if !block.dirty && block.access_tick < lru_tick {
                lru_tick = block.access_tick;
                lru_idx = Some(idx);
            }
        }

        if let Some(idx) = lru_idx {
            self.blocks.remove(&idx);
        } else if self.blocks.len() >= self.max_blocks {
            // All blocks dirty — evict oldest dirty block (force flush needed)
            let mut oldest_idx = None;
            let mut oldest_tick = u64::MAX;
            for (&idx, block) in &self.blocks {
                if block.access_tick < oldest_tick {
                    oldest_tick = block.access_tick;
                    oldest_idx = Some(idx);
                }
            }
            if let Some(idx) = oldest_idx {
                if self.blocks.get(&idx).map_or(false, |b| b.dirty) {
                    self.dirty_count = self.dirty_count.saturating_sub(1);
                }
                self.blocks.remove(&idx);
            }
        }
    }

    /// Total cache memory usage in bytes.
    pub fn memory_usage(&self) -> usize {
        self.blocks.len() * CACHE_BLOCK_SIZE
    }

    /// Cache mode.
    pub fn mode(&self) -> CacheMode { self.mode }
}
