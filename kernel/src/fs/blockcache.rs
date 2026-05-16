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
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Number of cached sectors. 16384 × 512 = 8 MiB cache.
const CACHE_SECTORS: usize = 16384;

/// Flush dirty data before write-back pressure gets close to eviction.
const DIRTY_FLUSH_HIGH_WATERMARK: u32 = (CACHE_SECTORS as u32 * 3) / 4;

/// Hash table size (power of two, ~2× slots for low collision rate).
const HASH_SIZE: usize = 32768;

/// Sentinel value for empty hash slots.
const EMPTY: u16 = 0xFFFF;

/// Sentinel value for deleted hash slots.
///
/// Cache slot indices are `0..CACHE_SECTORS` (currently 16K), so `0xFFFE`
/// is safely outside the valid slot range. Keeping tombstones means removing
/// one colliding entry no longer breaks lookup for later entries in that probe
/// chain, and ordinary misses can stop at the first never-used slot instead of
/// scanning the whole cache.
const TOMBSTONE: u16 = 0xFFFE;

/// Maximum linear probes before falling back to a rare full-table/slot scan.
const MAX_PROBES: usize = 32;

const MAX_DISKS: usize = 16;
const KEY_SEAL: u64 = 0xB10C_CACE_D15C_5AFE;

static WRITE_RANGE_START: [AtomicU64; MAX_DISKS] = {
    const INIT: AtomicU64 = AtomicU64::new(0);
    [INIT; MAX_DISKS]
};
static WRITE_RANGE_END: [AtomicU64; MAX_DISKS] = {
    const INIT: AtomicU64 = AtomicU64::new(0);
    [INIT; MAX_DISKS]
};
static DIRTY_ESTIMATE: [AtomicU32; MAX_DISKS] = {
    const INIT: AtomicU32 = AtomicU32::new(0);
    [INIT; MAX_DISKS]
};
static CORRUPT_CACHE_LOGS: AtomicU32 = AtomicU32::new(0);

/// A cached sector entry.
struct CacheEntry {
    /// Composite key: (disk_id << 32) | lba.  0 = empty slot.
    key: u64,
    /// Integrity seal for `key`. If random RAM scribbles turn a cache entry
    /// into a different LBA, writeback must drop it instead of persisting it.
    key_check: u64,
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
    next_free_hint: usize,
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
            next_free_hint: 0,
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
                key_check: 0,
                tick: 0,
                dirty: false,
                data: [0u8; 512],
            });
        }
        self.hash_table = vec![EMPTY; HASH_SIZE];
        self.next_free_hint = 0;
    }

    #[inline(always)]
    fn make_key(disk_id: u8, lba: u32) -> u64 {
        ((disk_id as u64) << 32) | (lba as u64)
    }

    #[inline(always)]
    fn make_key_check(key: u64, slot: usize) -> u64 {
        key ^ KEY_SEAL ^ (slot as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
    }

    #[inline(always)]
    fn slot_key_valid(&self, slot: usize) -> bool {
        slot < self.slots.len()
            && self.slots[slot].key != 0
            && self.slots[slot].key_check == Self::make_key_check(self.slots[slot].key, slot)
    }

    #[inline(always)]
    fn set_slot_key(&mut self, slot: usize, key: u64) {
        self.slots[slot].key = key;
        self.slots[slot].key_check = if key == 0 {
            0
        } else {
            Self::make_key_check(key, slot)
        };
    }

    fn report_corrupt_slot(&self, slot: usize, context: &str) {
        if CORRUPT_CACHE_LOGS.fetch_add(1, Ordering::Relaxed) < 16 {
            let entry = &self.slots[slot];
            let slot_virt = entry as *const CacheEntry as u64;
            let slot_phys = crate::memory::virtual_mem::virt_to_phys(
                crate::memory::address::VirtAddr::new(slot_virt),
            )
            .unwrap_or(0);
            crate::serial_println!(
                "[blockcache] corrupt cache slot during {}: slot={} virt={:#018x} phys={:#018x} key={:#018x} check={:#018x} expected={:#018x} dirty={}",
                context,
                slot,
                slot_virt,
                slot_phys,
                entry.key,
                entry.key_check,
                Self::make_key_check(entry.key, slot),
                entry.dirty
            );
        }
    }

    fn quarantine_slot(&mut self, slot: usize, context: &str) {
        if slot >= self.slots.len() {
            return;
        }
        if self.slots[slot].key != 0 && !self.slot_key_valid(slot) {
            self.report_corrupt_slot(slot, context);
            let old_key = self.slots[slot].key;
            self.remove_from_hash(old_key, slot);
            self.set_slot_key(slot, 0);
            self.slots[slot].dirty = false;
            self.slots[slot].tick = 0;
        }
    }

    #[inline(always)]
    fn hash(key: u64) -> usize {
        // Fibonacci hashing for good distribution
        (key.wrapping_mul(11400714819323198485)) as usize >> (64 - 15) // >> to get HASH_SIZE=32768 index
    }

    /// Look up a sector in the cache. Returns true if found and copies data to `buf`.
    pub fn lookup(&mut self, disk_id: u8, lba: u32, buf: &mut [u8]) -> bool {
        if self.hash_table.is_empty() {
            return false;
        } // Not initialized yet
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
            if slot_idx == TOMBSTONE {
                continue;
            }
            let si = slot_idx as usize;
            if si < self.slots.len() && self.slots[si].key == key && self.slot_key_valid(si) {
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
            self.quarantine_slot(i, "lookup");
            if self.slots[i].key == key && self.slot_key_valid(i) {
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

    /// Overlay all cached sectors in a range onto `buf`, without stopping at
    /// the first miss. This keeps disk reads coherent with dirty write-back
    /// entries that may live later in the requested range.
    pub fn overlay_range(&mut self, disk_id: u8, lba: u32, count: u32, buf: &mut [u8]) -> u32 {
        if count == 0 {
            return 0;
        }
        if count > MAX_PROBES as u32 {
            return self.overlay_range_by_slot_scan(disk_id, lba, count, buf);
        }

        let mut found = 0u32;
        for i in 0..count {
            let offset = i as usize * 512;
            if offset + 512 > buf.len() {
                break;
            }
            if self.lookup(disk_id, lba + i, &mut buf[offset..offset + 512]) {
                found += 1;
            }
        }
        found
    }

    fn overlay_range_by_slot_scan(
        &mut self,
        disk_id: u8,
        lba: u32,
        count: u32,
        buf: &mut [u8],
    ) -> u32 {
        let end = (lba as u64).saturating_add(count as u64);
        let mut found = 0u32;
        for i in 0..self.slots.len() {
            self.quarantine_slot(i, "overlay-range");
            if !self.slot_key_valid(i) || (self.slots[i].key >> 32) != disk_id as u64 {
                continue;
            }
            let slot_lba = self.slots[i].key & 0xFFFF_FFFF;
            if slot_lba < lba as u64 || slot_lba >= end {
                continue;
            }
            let offset = (slot_lba - lba as u64) as usize * 512;
            if offset + 512 > buf.len() {
                continue;
            }
            buf[offset..offset + 512].copy_from_slice(&self.slots[i].data);
            self.tick = self.tick.wrapping_add(1);
            self.slots[i].tick = self.tick;
            self.hits += 1;
            found += 1;
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
        let mut first_reusable: Option<usize> = None;
        let mut hit_never_used = false;
        for probe in 0..MAX_PROBES {
            let idx = (h + probe) & (HASH_SIZE - 1);
            let slot_idx = self.hash_table[idx];
            if slot_idx == EMPTY {
                if first_reusable.is_none() {
                    first_reusable = Some(idx);
                }
                hit_never_used = true;
                break;
            }
            if slot_idx == TOMBSTONE {
                if first_reusable.is_none() {
                    first_reusable = Some(idx);
                }
                continue;
            }
            let si = slot_idx as usize;
            if si < self.slots.len() && self.slots[si].key == key && self.slot_key_valid(si) {
                let n = data.len().min(512);
                self.slots[si].data[..n].copy_from_slice(&data[..n]);
                self.tick = self.tick.wrapping_add(1);
                self.slots[si].tick = self.tick;
                return;
            }
        }
        // Only fall back to a full duplicate scan when the normal probe window
        // was packed. On the common miss path this avoids scanning all 16K cache
        // slots for every sector of a streaming write.
        if !hit_never_used && first_reusable.is_none() {
            for i in 0..self.slots.len() {
                self.quarantine_slot(i, "insert-scan");
                if self.slots[i].key == key && self.slot_key_valid(i) {
                    let n = data.len().min(512);
                    self.slots[i].data[..n].copy_from_slice(&data[..n]);
                    self.tick = self.tick.wrapping_add(1);
                    self.slots[i].tick = self.tick;
                    return;
                }
            }
        }

        // Find LRU victim (lowest tick, prefer clean over dirty)
        let mut victim = 0usize;
        let mut min_tick = u32::MAX;
        // First pass: find an empty slot, starting from a rolling hint so filling
        // the cache is O(n) instead of O(n^2).
        for step in 0..self.slots.len() {
            let i = (self.next_free_hint + step) % self.slots.len();
            self.quarantine_slot(i, "victim-scan");
            if self.slots[i].key == 0 {
                victim = i;
                min_tick = 0;
                self.next_free_hint = (i + 1) % self.slots.len();
                break;
            }
        }
        // Second pass: find oldest clean entry
        if min_tick != 0 {
            for i in 0..self.slots.len() {
                self.quarantine_slot(i, "victim-scan");
                if !self.slots[i].dirty && self.slots[i].tick < min_tick {
                    min_tick = self.slots[i].tick;
                    victim = i;
                }
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
            for b in &mut self.slots[victim].data[n..] {
                *b = 0;
            }
        }
        self.set_slot_key(victim, key);
        self.slots[victim].dirty = false;
        self.tick = self.tick.wrapping_add(1);
        self.slots[victim].tick = self.tick;

        // Insert into hash table
        if let Some(idx) = first_reusable {
            self.hash_table[idx] = victim as u16;
        } else {
            self.insert_hash(key, victim as u16);
        }
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
        if self.slots.is_empty() {
            return;
        }
        self.insert(disk_id, lba, data);
        let key = Self::make_key(disk_id, lba);
        // Mark as dirty
        let h = Self::hash(key);
        for probe in 0..MAX_PROBES {
            let idx = (h + probe) & (HASH_SIZE - 1);
            let slot_idx = self.hash_table[idx];
            if slot_idx == EMPTY {
                break;
            }
            if slot_idx == TOMBSTONE {
                continue;
            }
            let si = slot_idx as usize;
            if si < self.slots.len() && self.slots[si].key == key && self.slot_key_valid(si) {
                self.slots[si].dirty = true;
                return;
            }
        }
        // Fallback linear scan
        for i in 0..self.slots.len() {
            self.quarantine_slot(i, "insert-dirty-scan");
            if self.slots[i].key == key && self.slot_key_valid(i) {
                self.slots[i].dirty = true;
                return;
            }
        }
    }

    /// Invalidate a cached sector (e.g. after direct disk write).
    pub fn invalidate(&mut self, disk_id: u8, lba: u32) {
        if self.hash_table.is_empty() {
            return;
        }
        let key = Self::make_key(disk_id, lba);
        let h = Self::hash(key);
        for probe in 0..MAX_PROBES {
            let idx = (h + probe) & (HASH_SIZE - 1);
            let slot_idx = self.hash_table[idx];
            if slot_idx == EMPTY {
                return;
            }
            if slot_idx == TOMBSTONE {
                continue;
            }
            let si = slot_idx as usize;
            if si < self.slots.len() && self.slots[si].key == key && self.slot_key_valid(si) {
                self.set_slot_key(si, 0);
                self.slots[si].dirty = false;
                self.hash_table[idx] = TOMBSTONE;
                return;
            }
        }
        // Fallback linear scan
        for i in 0..self.slots.len() {
            self.quarantine_slot(i, "invalidate-scan");
            if self.slots[i].key == key && self.slot_key_valid(i) {
                self.remove_from_hash(key, i);
                self.set_slot_key(i, 0);
                self.slots[i].dirty = false;
                return;
            }
        }
    }

    /// Invalidate a range of sectors.
    pub fn invalidate_range(&mut self, disk_id: u8, lba: u32, count: u32) {
        if count == 0 {
            return;
        }
        if count > MAX_PROBES as u32 {
            let end = (lba as u64).saturating_add(count as u64);
            for i in 0..self.slots.len() {
                self.quarantine_slot(i, "invalidate-range");
                if !self.slot_key_valid(i) || (self.slots[i].key >> 32) != disk_id as u64 {
                    continue;
                }
                let slot_lba = self.slots[i].key & 0xFFFF_FFFF;
                if slot_lba < lba as u64 || slot_lba >= end {
                    continue;
                }
                let key = self.slots[i].key;
                self.remove_from_hash(key, i);
                self.set_slot_key(i, 0);
                self.slots[i].dirty = false;
                self.slots[i].tick = 0;
                self.next_free_hint = i;
            }
            return;
        }

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
            self.quarantine_slot(i, "flush-dirty");
            if self.slots[i].dirty
                && self.slot_key_valid(i)
                && (self.slots[i].key >> 32) == disk_id as u64
            {
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
        let rate = if total > 0 {
            (self.hits * 100 / total) as u32
        } else {
            0
        };
        (self.hits, self.misses, rate)
    }

    /// Mark a specific key as dirty (internal helper).
    fn mark_dirty(&mut self, key: u64) {
        if self.hash_table.is_empty() {
            return;
        }
        let h = Self::hash(key);
        for probe in 0..MAX_PROBES {
            let idx = (h + probe) & (HASH_SIZE - 1);
            let slot_idx = self.hash_table[idx];
            if slot_idx == EMPTY {
                return;
            }
            if slot_idx == TOMBSTONE {
                continue;
            }
            let si = slot_idx as usize;
            if si < self.slots.len() && self.slots[si].key == key && self.slot_key_valid(si) {
                self.slots[si].dirty = true;
                return;
            }
        }
        // Fallback linear
        for i in 0..self.slots.len() {
            self.quarantine_slot(i, "mark-dirty-scan");
            if self.slots[i].key == key && self.slot_key_valid(i) {
                self.slots[i].dirty = true;
                return;
            }
        }
    }

    // ── Hash table helpers ──────────────────────────────────────────────

    fn insert_hash(&mut self, key: u64, slot: u16) {
        let h = Self::hash(key);
        let mut first_tombstone = EMPTY;
        for probe in 0..MAX_PROBES {
            let idx = (h + probe) & (HASH_SIZE - 1);
            if self.hash_table[idx] == EMPTY {
                let target = if first_tombstone != EMPTY {
                    first_tombstone as usize
                } else {
                    idx
                };
                self.hash_table[target] = slot;
                return;
            }
            if self.hash_table[idx] == TOMBSTONE && first_tombstone == EMPTY {
                first_tombstone = idx as u16;
            }
        }
        if first_tombstone != EMPTY {
            self.hash_table[first_tombstone as usize] = slot;
            return;
        }

        // Rare pathological collision case: keep correctness by scanning the
        // hash table for reusable space instead of overwriting a live mapping.
        for idx in 0..self.hash_table.len() {
            if self.hash_table[idx] == EMPTY || self.hash_table[idx] == TOMBSTONE {
                self.hash_table[idx] = slot;
                return;
            }
        }

        crate::serial_println!(
            "[blockcache] hash table full; dropping cache mapping for key={:#x}",
            key
        );
    }

    fn remove_from_hash(&mut self, key: u64, slot_idx: usize) {
        let h = Self::hash(key);
        for probe in 0..MAX_PROBES {
            let idx = (h + probe) & (HASH_SIZE - 1);
            if self.hash_table[idx] == EMPTY {
                return;
            }
            if self.hash_table[idx] == slot_idx as u16 {
                self.hash_table[idx] = TOMBSTONE;
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
    next_free_hint: 0,
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

/// Limit lazy write-back for a physical disk to the mounted system partition.
///
/// Reads may still cover the whole disk (partition scanning, ESP access), but
/// write-back should never be able to persist a corrupted cache key into GPT,
/// the ESP, or LBA0.
pub fn set_write_range(disk_id: u8, start_lba: u64, sector_count: u64) {
    let idx = disk_id as usize;
    if idx >= MAX_DISKS || sector_count == 0 {
        return;
    }
    let end = start_lba.saturating_add(sector_count);
    WRITE_RANGE_START[idx].store(start_lba, Ordering::Release);
    WRITE_RANGE_END[idx].store(end, Ordering::Release);
    crate::serial_verbose_println!(
        "[blockcache] disk {} write-back range set to [{}..{})",
        disk_id,
        start_lba,
        end
    );
}

#[inline]
pub fn write_range_allows(disk_id: u8, lba: u32, count: u32) -> bool {
    let idx = disk_id as usize;
    if idx >= MAX_DISKS {
        return true;
    }
    let end = WRITE_RANGE_END[idx].load(Ordering::Acquire);
    if end == 0 {
        return true;
    }
    let start = WRITE_RANGE_START[idx].load(Ordering::Acquire);
    let req_start = lba as u64;
    let req_end = req_start.saturating_add(count as u64);
    req_start >= start && req_end <= end
}

/// Try to read `count` sectors from cache. Returns number of sectors served
/// from cache (starting from lba, stops at first miss).
pub fn cached_read(disk_id: u8, lba: u32, count: u32, buf: &mut [u8]) -> u32 {
    if !CACHE_READY.load(Ordering::Acquire) {
        return 0;
    }
    BLOCK_CACHE.lock().lookup_range(disk_id, lba, count, buf)
}

/// Overlay cached sectors anywhere in the range onto `buf`.
///
/// Unlike `cached_read`, this intentionally scans the whole range. It is used
/// after a backend read to restore read-after-write coherence for dirty
/// write-back sectors when the range had an earlier cache miss.
pub fn overlay_cached(disk_id: u8, lba: u32, count: u32, buf: &mut [u8]) -> u32 {
    if !CACHE_READY.load(Ordering::Acquire) {
        return 0;
    }
    BLOCK_CACHE.lock().overlay_range(disk_id, lba, count, buf)
}

/// Insert sectors into cache after a disk read.
pub fn populate(disk_id: u8, lba: u32, count: u32, data: &[u8]) {
    if !CACHE_READY.load(Ordering::Acquire) {
        return;
    }
    BLOCK_CACHE.lock().insert_range(disk_id, lba, count, data);
}

/// Invalidate cached sectors (after a direct write to disk).
pub fn invalidate(disk_id: u8, lba: u32, count: u32) {
    if !CACHE_READY.load(Ordering::Acquire) {
        return;
    }
    BLOCK_CACHE.lock().invalidate_range(disk_id, lba, count);
}

/// Invalidate all cached sectors for a disk.
pub fn invalidate_disk(disk_id: u8) {
    if !CACHE_READY.load(Ordering::Acquire) {
        return;
    }
    let mut cache = BLOCK_CACHE.lock();
    for i in 0..cache.slots.len() {
        cache.quarantine_slot(i, "invalidate-disk");
        if cache.slot_key_valid(i) && (cache.slots[i].key >> 32) == disk_id as u64 {
            let key = cache.slots[i].key;
            cache.remove_from_hash(key, i);
            cache.set_slot_key(i, 0);
            cache.slots[i].dirty = false;
        }
    }
}

/// Insert sectors as dirty (write-back caching). The data is cached in RAM
/// and will be written to disk later during writeback.
pub fn write_back(disk_id: u8, lba: u32, count: u32, data: &[u8]) {
    if !CACHE_READY.load(Ordering::Acquire) {
        return;
    }
    if !write_range_allows(disk_id, lba, count) {
        crate::serial_println!(
            "[blockcache] refusing write-back outside system partition: disk={} lba={} count={}",
            disk_id,
            lba,
            count
        );
        return;
    }
    let mut cache = BLOCK_CACHE.lock();
    let mut accepted = 0u32;
    for i in 0..count {
        let offset = i as usize * 512;
        if offset + 512 <= data.len() {
            cache.insert(disk_id, lba + i, &data[offset..offset + 512]);
            // Mark as dirty
            let key = BlockCache::make_key(disk_id, lba + i);
            cache.mark_dirty(key);
            accepted += 1;
        }
    }
    if (disk_id as usize) < MAX_DISKS && accepted > 0 {
        DIRTY_ESTIMATE[disk_id as usize].fetch_add(accepted, Ordering::Relaxed);
    }
}

/// Return true when another write-back batch would push the cache into a
/// range where dirty eviction becomes likely.
pub fn should_flush_before_write_back(disk_id: u8, incoming_count: u32) -> bool {
    if !CACHE_READY.load(Ordering::Acquire) {
        return false;
    }
    let idx = disk_id as usize;
    if idx >= MAX_DISKS {
        return false;
    }
    let estimate = DIRTY_ESTIMATE[idx].load(Ordering::Relaxed);
    if estimate.saturating_add(incoming_count) < DIRTY_FLUSH_HIGH_WATERMARK {
        return false;
    }

    let mut cache = BLOCK_CACHE.lock();
    let mut dirty = 0u32;
    for i in 0..cache.slots.len() {
        cache.quarantine_slot(i, "dirty-pressure");
        let slot = &cache.slots[i];
        if slot.dirty && cache.slot_key_valid(i) && (slot.key >> 32) == disk_id as u64 {
            dirty += 1;
        }
    }
    DIRTY_ESTIMATE[idx].store(dirty, Ordering::Relaxed);
    dirty.saturating_add(incoming_count) >= DIRTY_FLUSH_HIGH_WATERMARK
}

/// Flush all dirty sectors to disk. Uses the provided write function to
/// perform the actual I/O. Coalesces consecutive dirty sectors into single
/// multi-sector writes for efficiency.
/// Returns number of sectors flushed.
pub fn writeback_flush(disk_id: u8) -> u32 {
    if !CACHE_READY.load(Ordering::Acquire) {
        return 0;
    }

    // Snapshot dirty blocks as (lba, 512-byte data) pairs under a single lock.
    //
    // Previously this function collected `(lba, slot_index)`, dropped the lock,
    // then re-acquired it later and read cache.slots[slot_index].data. Between
    // the two lock windows another CPU could call `insert()` / `populate()`
    // and EVICT the dirty slot, filling it with data for a different LBA.
    // Writeback would then flush the wrong bytes to the wrong sector — silent
    // data corruption plus a free-list-level inconsistency when the evictor's
    // allocator touched the same memory region. Snapshotting the data while
    // we still hold the lock removes the window entirely.
    //
    // `key` is captured together with (lba,data) so the re-lock below can
    // verify the slot still holds the same entry before clearing the dirty
    // flag. If it was evicted and repopulated we leave dirty=true on the new
    // occupant so a subsequent writeback cycle flushes whatever belongs there.
    struct DirtyCopy {
        key: u64,
        lba: u32,
        slot: usize,
        data: [u8; 512],
    }

    let mut snapshot: alloc::vec::Vec<DirtyCopy> = alloc::vec::Vec::new();
    {
        let mut cache = BLOCK_CACHE.lock();
        for i in 0..cache.slots.len() {
            cache.quarantine_slot(i, "writeback-snapshot");
            let entry = &cache.slots[i];
            if entry.dirty && cache.slot_key_valid(i) && (entry.key >> 32) == disk_id as u64 {
                let lba = (entry.key & 0xFFFFFFFF) as u32;
                snapshot.push(DirtyCopy {
                    key: entry.key,
                    lba,
                    slot: i,
                    data: entry.data,
                });
            }
        }
    }

    if snapshot.is_empty() {
        if (disk_id as usize) < MAX_DISKS {
            DIRTY_ESTIMATE[disk_id as usize].store(0, Ordering::Relaxed);
        }
        return 0;
    }

    // Sort by LBA so coalescing works regardless of slot-order.
    snapshot.sort_unstable_by_key(|c| c.lba);

    let mut flushed = 0u32;
    let mut i = 0;
    while i < snapshot.len() {
        // Find run of consecutive LBAs.
        let run_start_lba = snapshot[i].lba;
        let mut run_len = 1usize;
        while i + run_len < snapshot.len()
            && snapshot[i + run_len].lba == run_start_lba + run_len as u32
        {
            run_len += 1;
        }

        let disk_sectors = crate::drivers::storage::disk_sector_count(disk_id);
        if !write_range_allows(disk_id, run_start_lba, run_len as u32) {
            crate::serial_println!(
                "[blockcache] dropping dirty run outside system partition: disk={} lba={} count={}",
                disk_id,
                run_start_lba,
                run_len
            );
            {
                let mut cache = BLOCK_CACHE.lock();
                for j in 0..run_len {
                    let snap = &snapshot[i + j];
                    if snap.slot >= cache.slots.len() {
                        continue;
                    }
                    if cache.slots[snap.slot].key == snap.key && cache.slot_key_valid(snap.slot) {
                        cache.remove_from_hash(snap.key, snap.slot);
                        cache.set_slot_key(snap.slot, 0);
                        cache.slots[snap.slot].dirty = false;
                    }
                }
            }
            i += run_len;
            continue;
        }

        if disk_sectors > 0 && (run_start_lba as u64).saturating_add(run_len as u64) > disk_sectors
        {
            crate::serial_println!(
                "[blockcache] dropping corrupt dirty run: disk={} lba={} count={} sectors={}",
                disk_id,
                run_start_lba,
                run_len,
                disk_sectors
            );
            {
                let mut cache = BLOCK_CACHE.lock();
                for j in 0..run_len {
                    let snap = &snapshot[i + j];
                    if snap.slot >= cache.slots.len() {
                        continue;
                    }
                    if cache.slots[snap.slot].key == snap.key && cache.slot_key_valid(snap.slot) {
                        cache.remove_from_hash(snap.key, snap.slot);
                        cache.set_slot_key(snap.slot, 0);
                        cache.slots[snap.slot].dirty = false;
                    }
                }
            }
            i += run_len;
            continue;
        }

        // Assemble the write buffer from the in-band snapshot — no cache lock
        // needed here, so the disk write path cannot contend with concurrent
        // cache readers/writers.
        let mut buf = alloc::vec![0u8; run_len * 512];
        for j in 0..run_len {
            buf[j * 512..(j + 1) * 512].copy_from_slice(&snapshot[i + j].data);
        }

        // Write coalesced run to disk (bypasses cache, goes direct to hardware).
        let ok = crate::drivers::storage::write_sectors_direct_on_disk(
            disk_id,
            run_start_lba,
            run_len as u32,
            &buf,
        );
        if ok {
            flushed += run_len as u32;
        }

        // Reconcile dirty flags under the lock, checking key identity per slot:
        //   * write succeeded + slot still holds our key      → clear dirty
        //   * write failed  + slot still holds our key        → leave dirty
        //   * slot was evicted/repopulated (key differs)      → don't touch;
        //     whatever is there now has its own dirty flag lifecycle
        {
            let mut cache = BLOCK_CACHE.lock();
            for j in 0..run_len {
                let snap = &snapshot[i + j];
                if snap.slot >= cache.slots.len() {
                    continue;
                }
                if cache.slots[snap.slot].key != snap.key
                    || cache.slots[snap.slot].key_check
                        != BlockCache::make_key_check(snap.key, snap.slot)
                {
                    continue;
                }
                if ok {
                    cache.remove_from_hash(snap.key, snap.slot);
                    cache.set_slot_key(snap.slot, 0);
                    cache.slots[snap.slot].dirty = false;
                    cache.slots[snap.slot].tick = 0;
                    cache.next_free_hint = snap.slot;
                }
            }
        }

        i += run_len;
    }

    if (disk_id as usize) < MAX_DISKS && flushed > 0 {
        let estimate = DIRTY_ESTIMATE[disk_id as usize].load(Ordering::Relaxed);
        DIRTY_ESTIMATE[disk_id as usize].store(estimate.saturating_sub(flushed), Ordering::Relaxed);
    }

    flushed
}

/// Return the number of dirty sectors in the cache.
pub fn dirty_count(disk_id: u8) -> u32 {
    if !CACHE_READY.load(Ordering::Acquire) {
        return 0;
    }
    let mut cache = BLOCK_CACHE.lock();
    let mut count = 0u32;
    for i in 0..cache.slots.len() {
        cache.quarantine_slot(i, "dirty-count");
        let slot = &cache.slots[i];
        if slot.dirty && cache.slot_key_valid(i) && (slot.key >> 32) == disk_id as u64 {
            count += 1;
        }
    }
    count
}

/// Get cache stats: (hits, misses, hit_rate_percent).
pub fn stats() -> (u64, u64, u32) {
    if !CACHE_READY.load(Ordering::Acquire) {
        return (0, 0, 0);
    }
    BLOCK_CACHE.lock().stats()
}
