//! Global block cache for sector-level read caching.
//!
//! Transparently caches disk sectors in RAM to avoid redundant I/O.
//! Uses LRU eviction with a hash table for O(1) lookups.
//! Dirty blocks support write-back for deferred metadata flushing.
//!
//! The cache is global and protected by its own spinlock (not the VFS mutex),
//! so lookups can proceed without holding the heavyweight VFS lock.

use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

/// Number of cached sectors. 16384 × 512 = 8 MiB cache.
const CACHE_SECTORS: usize = 16384;

/// Hash table size (power of two, ~2× slots for low collision rate).
const HASH_SIZE: usize = 32768;

/// Sentinel value for empty hash slots.
const EMPTY: u16 = 0xFFFF;

/// Maximum linear probes before falling back.
const MAX_PROBES: usize = 8;

/// A cached sector entry.
struct CacheEntry {
    /// Composite key: (disk_id << 32) | lba.  0 = empty slot.
    key: u64,
    /// LRU tick (higher = more recently used).
    tick: u32,
    /// Dirty flag — if true, must be flushed before eviction.
    dirty: bool,
    /// Sector data (512 bytes, allocated on first use).
    data: [u8; 512],
}

/// Global block cache with hash-accelerated LRU.
pub struct BlockCache {
    slots: Vec<CacheEntry>,
    hash_table: Vec<u16>,
    tick: u32,
    /// Statistics
    hits: u64,
    misses: u64,
}

impl BlockCache {
    pub fn new() -> Self {
        BlockCache {
            slots: Vec::new(),
            hash_table: Vec::new(),
            tick: 0,
            hits: 0,
            misses: 0,
        }
    }

    /// Initialize the cache (called once during boot, after heap is available).
    pub fn init(&mut self) {
        if !self.slots.is_empty() {
            return; // Already initialized
        }
        self.slots.reserve(CACHE_SECTORS);
        for _ in 0..CACHE_SECTORS {
            self.slots.push(CacheEntry {
                key: 0,
                tick: 0,
                dirty: false,
                data: [0u8; 512],
            });
        }
        self.hash_table = vec![EMPTY; HASH_SIZE];
    }

    #[inline(always)]
    fn make_key(disk_id: u8, lba: u32) -> u64 {
        ((disk_id as u64) << 32) | (lba as u64)
    }

    #[inline(always)]
    fn hash(key: u64) -> usize {
        // Fibonacci hashing for good distribution
        (key.wrapping_mul(11400714819323198485)) as usize >> (64 - 15) // >> to get HASH_SIZE=32768 index
    }

    /// Look up a sector in the cache. Returns true if found and copies data to `buf`.
    pub fn lookup(&mut self, disk_id: u8, lba: u32, buf: &mut [u8]) -> bool {
        if self.hash_table.is_empty() { return false; } // Not initialized yet
        let key = Self::make_key(disk_id, lba);
        let h = Self::hash(key);

        // Hash table probe
        for probe in 0..MAX_PROBES {
            let idx = (h + probe) & (HASH_SIZE - 1);
            let slot_idx = self.hash_table[idx];
            if slot_idx == EMPTY {
                self.misses += 1;
                return false;
            }
            let si = slot_idx as usize;
            if si < self.slots.len() && self.slots[si].key == key {
                // Hit — copy data and update LRU tick
                let n = buf.len().min(512);
                buf[..n].copy_from_slice(&self.slots[si].data[..n]);
                self.tick = self.tick.wrapping_add(1);
                self.slots[si].tick = self.tick;
                self.hits += 1;
                return true;
            }
        }

        // Fallback: linear scan for hash collisions beyond MAX_PROBES
        for i in 0..self.slots.len() {
            if self.slots[i].key == key {
                let n = buf.len().min(512);
                buf[..n].copy_from_slice(&self.slots[i].data[..n]);
                self.tick = self.tick.wrapping_add(1);
                self.slots[i].tick = self.tick;
                self.hits += 1;
                return true;
            }
        }

        self.misses += 1;
        false
    }

    /// Look up multiple consecutive sectors. Returns number of sectors found
    /// starting from lba (stops at first miss).
    pub fn lookup_range(&mut self, disk_id: u8, lba: u32, count: u32, buf: &mut [u8]) -> u32 {
        let mut found = 0u32;
        for i in 0..count {
            let offset = i as usize * 512;
            if offset + 512 > buf.len() {
                break;
            }
            if self.lookup(disk_id, lba + i, &mut buf[offset..offset + 512]) {
                found += 1;
            } else {
                break; // Stop at first miss for sequential access patterns
            }
        }
        found
    }

    /// Insert a sector into the cache (clean, read-cache entry).
    pub fn insert(&mut self, disk_id: u8, lba: u32, data: &[u8]) {
        if self.slots.is_empty() {
            return; // Not initialized
        }
        let key = Self::make_key(disk_id, lba);

        // Check if already cached — update in place
        let h = Self::hash(key);
        for probe in 0..MAX_PROBES {
            let idx = (h + probe) & (HASH_SIZE - 1);
            let slot_idx = self.hash_table[idx];
            if slot_idx == EMPTY {
                break;
            }
            let si = slot_idx as usize;
            if si < self.slots.len() && self.slots[si].key == key {
                let n = data.len().min(512);
                self.slots[si].data[..n].copy_from_slice(&data[..n]);
                self.tick = self.tick.wrapping_add(1);
                self.slots[si].tick = self.tick;
                return;
            }
        }

        // Find LRU victim (lowest tick, prefer clean over dirty)
        let mut victim = 0usize;
        let mut min_tick = u32::MAX;
        // First pass: find oldest clean entry
        for i in 0..self.slots.len() {
            if self.slots[i].key == 0 {
                victim = i;
                min_tick = 0;
                break;
            }
            if !self.slots[i].dirty && self.slots[i].tick < min_tick {
                min_tick = self.slots[i].tick;
                victim = i;
            }
        }
        // If all entries are dirty, evict oldest dirty entry
        if min_tick == u32::MAX {
            for i in 0..self.slots.len() {
                if self.slots[i].tick < min_tick {
                    min_tick = self.slots[i].tick;
                    victim = i;
                }
            }
        }

        // Remove old entry from hash table
        let old_key = self.slots[victim].key;
        if old_key != 0 {
            self.remove_from_hash(old_key, victim);
        }

        // Write new entry
        let n = data.len().min(512);
        self.slots[victim].data[..n].copy_from_slice(&data[..n]);
        if n < 512 {
            // Zero-fill remainder
            for b in &mut self.slots[victim].data[n..] { *b = 0; }
        }
        self.slots[victim].key = key;
        self.slots[victim].dirty = false;
        self.tick = self.tick.wrapping_add(1);
        self.slots[victim].tick = self.tick;

        // Insert into hash table
        self.insert_hash(key, victim as u16);
    }

    /// Insert multiple consecutive sectors from a buffer.
    pub fn insert_range(&mut self, disk_id: u8, lba: u32, count: u32, data: &[u8]) {
        for i in 0..count {
            let offset = i as usize * 512;
            if offset + 512 <= data.len() {
                self.insert(disk_id, lba + i, &data[offset..offset + 512]);
            }
        }
    }

    /// Insert a dirty sector (for write-back caching).
    pub fn insert_dirty(&mut self, disk_id: u8, lba: u32, data: &[u8]) {
        if self.slots.is_empty() { return; }
        self.insert(disk_id, lba, data);
        let key = Self::make_key(disk_id, lba);
        // Mark as dirty
        let h = Self::hash(key);
        for probe in 0..MAX_PROBES {
            let idx = (h + probe) & (HASH_SIZE - 1);
            let slot_idx = self.hash_table[idx];
            if slot_idx == EMPTY { break; }
            let si = slot_idx as usize;
            if si < self.slots.len() && self.slots[si].key == key {
                self.slots[si].dirty = true;
                return;
            }
        }
        // Fallback linear scan
        for i in 0..self.slots.len() {
            if self.slots[i].key == key {
                self.slots[i].dirty = true;
                return;
            }
        }
    }

    /// Invalidate a cached sector (e.g. after direct disk write).
    pub fn invalidate(&mut self, disk_id: u8, lba: u32) {
        if self.hash_table.is_empty() { return; }
        let key = Self::make_key(disk_id, lba);
        let h = Self::hash(key);
        for probe in 0..MAX_PROBES {
            let idx = (h + probe) & (HASH_SIZE - 1);
            let slot_idx = self.hash_table[idx];
            if slot_idx == EMPTY { return; }
            let si = slot_idx as usize;
            if si < self.slots.len() && self.slots[si].key == key {
                self.slots[si].key = 0;
                self.slots[si].dirty = false;
                self.hash_table[idx] = EMPTY;
                return;
            }
        }
        // Fallback linear scan
        for i in 0..self.slots.len() {
            if self.slots[i].key == key {
                self.remove_from_hash(key, i);
                self.slots[i].key = 0;
                self.slots[i].dirty = false;
                return;
            }
        }
    }

    /// Invalidate a range of sectors.
    pub fn invalidate_range(&mut self, disk_id: u8, lba: u32, count: u32) {
        for i in 0..count {
            self.invalidate(disk_id, lba + i);
        }
    }

    /// Flush all dirty sectors for a given disk by calling the provided write function.
    /// Returns true if all writes succeeded.
    pub fn flush_dirty<F>(&mut self, disk_id: u8, mut write_fn: F) -> bool
    where
        F: FnMut(u32, &[u8]) -> bool, // (lba, data) -> success
    {
        let disk_prefix = (disk_id as u64) << 32;
        // Collect dirty entries, coalesce consecutive sectors
        let mut i = 0;
        let mut all_ok = true;
        while i < self.slots.len() {
            if self.slots[i].dirty && (self.slots[i].key >> 32) == disk_id as u64 {
                let start_lba = (self.slots[i].key & 0xFFFFFFFF) as u32;
                // Try to coalesce consecutive dirty sectors
                let mut run_len = 1u32;
                self.slots[i].dirty = false;

                // Simple single-sector flush (coalescing would require sorting)
                if !write_fn(start_lba, &self.slots[i].data) {
                    all_ok = false;
                    self.slots[i].dirty = true; // Re-mark on failure
                }
            }
            i += 1;
        }
        all_ok
    }

    /// Get cache statistics: (hits, misses, hit_rate_percent).
    pub fn stats(&self) -> (u64, u64, u32) {
        let total = self.hits + self.misses;
        let rate = if total > 0 { (self.hits * 100 / total) as u32 } else { 0 };
        (self.hits, self.misses, rate)
    }

    /// Mark a specific key as dirty (internal helper).
    fn mark_dirty(&mut self, key: u64) {
        if self.hash_table.is_empty() { return; }
        let h = Self::hash(key);
        for probe in 0..MAX_PROBES {
            let idx = (h + probe) & (HASH_SIZE - 1);
            let slot_idx = self.hash_table[idx];
            if slot_idx == EMPTY { return; }
            let si = slot_idx as usize;
            if si < self.slots.len() && self.slots[si].key == key {
                self.slots[si].dirty = true;
                return;
            }
        }
        // Fallback linear
        for slot in &mut self.slots {
            if slot.key == key { slot.dirty = true; return; }
        }
    }

    // ── Hash table helpers ──────────────────────────────────────────────

    fn insert_hash(&mut self, key: u64, slot: u16) {
        let h = Self::hash(key);
        for probe in 0..MAX_PROBES {
            let idx = (h + probe) & (HASH_SIZE - 1);
            if self.hash_table[idx] == EMPTY {
                self.hash_table[idx] = slot;
                return;
            }
        }
        // All probe slots full — overwrite the first one (degrades but doesn't break)
        let idx = h & (HASH_SIZE - 1);
        self.hash_table[idx] = slot;
    }

    fn remove_from_hash(&mut self, key: u64, slot_idx: usize) {
        let h = Self::hash(key);
        for probe in 0..MAX_PROBES {
            let idx = (h + probe) & (HASH_SIZE - 1);
            if self.hash_table[idx] == slot_idx as u16 {
                self.hash_table[idx] = EMPTY;
                return;
            }
            if self.hash_table[idx] == EMPTY {
                return;
            }
        }
    }
}

// ── Global instance ─────────────────────────────────────────────────────────

use crate::sync::spinlock::Spinlock;
use core::sync::atomic::AtomicBool;

/// Fast check — avoids touching the Spinlock before the cache is ready.
static CACHE_READY: AtomicBool = AtomicBool::new(false);

static BLOCK_CACHE: Spinlock<BlockCache> = Spinlock::new(BlockCache {
    slots: Vec::new(),
    hash_table: Vec::new(),
    tick: 0,
    hits: 0,
    misses: 0,
});

/// Returns true if the block cache has been initialized and is ready to use.
#[inline]
pub fn is_ready() -> bool {
    CACHE_READY.load(Ordering::Acquire)
}

/// Initialize the global block cache (call once after heap init).
pub fn init() {
    BLOCK_CACHE.lock().init();
    CACHE_READY.store(true, Ordering::Release);
}

/// Try to read `count` sectors from cache. Returns number of sectors served
/// from cache (starting from lba, stops at first miss).
pub fn cached_read(disk_id: u8, lba: u32, count: u32, buf: &mut [u8]) -> u32 {
    if !CACHE_READY.load(Ordering::Acquire) { return 0; }
    BLOCK_CACHE.lock().lookup_range(disk_id, lba, count, buf)
}

/// Insert sectors into cache after a disk read.
pub fn populate(disk_id: u8, lba: u32, count: u32, data: &[u8]) {
    if !CACHE_READY.load(Ordering::Acquire) { return; }
    BLOCK_CACHE.lock().insert_range(disk_id, lba, count, data);
}

/// Invalidate cached sectors (after a direct write to disk).
pub fn invalidate(disk_id: u8, lba: u32, count: u32) {
    if !CACHE_READY.load(Ordering::Acquire) { return; }
    BLOCK_CACHE.lock().invalidate_range(disk_id, lba, count);
}

/// Invalidate all cached sectors for a disk.
pub fn invalidate_disk(disk_id: u8) {
    if !CACHE_READY.load(Ordering::Acquire) { return; }
    let mut cache = BLOCK_CACHE.lock();
    for i in 0..cache.slots.len() {
        if (cache.slots[i].key >> 32) == disk_id as u64 {
            let key = cache.slots[i].key;
            cache.remove_from_hash(key, i);
            cache.slots[i].key = 0;
            cache.slots[i].dirty = false;
        }
    }
}

/// Insert sectors as dirty (write-back caching). The data is cached in RAM
/// and will be written to disk later during writeback.
pub fn write_back(disk_id: u8, lba: u32, count: u32, data: &[u8]) {
    if !CACHE_READY.load(Ordering::Acquire) { return; }
    let mut cache = BLOCK_CACHE.lock();
    for i in 0..count {
        let offset = i as usize * 512;
        if offset + 512 <= data.len() {
            cache.insert(disk_id, lba + i, &data[offset..offset + 512]);
            // Mark as dirty
            let key = BlockCache::make_key(disk_id, lba + i);
            cache.mark_dirty(key);
        }
    }
}

/// Flush all dirty sectors to disk. Uses the provided write function to
/// perform the actual I/O. Coalesces consecutive dirty sectors into single
/// multi-sector writes for efficiency.
/// Returns number of sectors flushed.
pub fn writeback_flush(disk_id: u8) -> u32 {
    if !CACHE_READY.load(Ordering::Acquire) { return 0; }

    // Collect dirty (lba, slot_index) pairs, then sort by LBA for coalescing
    let mut dirty_entries: alloc::vec::Vec<(u32, usize)> = alloc::vec::Vec::new();
    {
        let cache = BLOCK_CACHE.lock();
        let disk_prefix = (disk_id as u64) << 32;
        for i in 0..cache.slots.len() {
            if cache.slots[i].dirty && (cache.slots[i].key >> 32) == disk_id as u64 {
                let lba = (cache.slots[i].key & 0xFFFFFFFF) as u32;
                dirty_entries.push((lba, i));
            }
        }
    }

    if dirty_entries.is_empty() { return 0; }

    // Sort by LBA for coalescing
    dirty_entries.sort_unstable_by_key(|&(lba, _)| lba);

    let mut flushed = 0u32;
    let mut i = 0;
    while i < dirty_entries.len() {
        // Find run of consecutive LBAs
        let run_start_lba = dirty_entries[i].0;
        let mut run_len = 1usize;
        while i + run_len < dirty_entries.len()
            && dirty_entries[i + run_len].0 == run_start_lba + run_len as u32
        {
            run_len += 1;
        }

        // Build coalesced write buffer and clear dirty flags
        let mut buf = alloc::vec![0u8; run_len * 512];
        {
            let mut cache = BLOCK_CACHE.lock();
            for j in 0..run_len {
                let slot = dirty_entries[i + j].1;
                if slot < cache.slots.len() {
                    buf[j * 512..(j + 1) * 512].copy_from_slice(&cache.slots[slot].data);
                    cache.slots[slot].dirty = false;
                }
            }
        }

        // Write coalesced run to disk (bypasses cache, goes direct to hardware)
        if crate::drivers::storage::write_sectors_direct_on_disk(disk_id, run_start_lba, run_len as u32, &buf) {
            flushed += run_len as u32;
        } else {
            let mut cache = BLOCK_CACHE.lock();
            for j in 0..run_len {
                let slot = dirty_entries[i + j].1;
                if slot < cache.slots.len() {
                    cache.slots[slot].dirty = true;
                }
            }
        }
        i += run_len;
    }

    flushed
}

/// Return the number of dirty sectors in the cache.
pub fn dirty_count(disk_id: u8) -> u32 {
    if !CACHE_READY.load(Ordering::Acquire) { return 0; }
    let cache = BLOCK_CACHE.lock();
    let mut count = 0u32;
    for slot in &cache.slots {
        if slot.dirty && (slot.key >> 32) == disk_id as u64 {
            count += 1;
        }
    }
    count
}

/// Get cache stats: (hits, misses, hit_rate_percent).
pub fn stats() -> (u64, u64, u32) {
    if !CACHE_READY.load(Ordering::Acquire) { return (0, 0, 0); }
    BLOCK_CACHE.lock().stats()
}
