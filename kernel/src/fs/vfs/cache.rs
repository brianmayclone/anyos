//! Small VFS caches that do not belong in the main operation flow.

use core::sync::atomic::{AtomicU32, Ordering};

const DIR_CACHE_SIZE: usize = 64;

#[derive(Clone, Copy)]
struct DirCacheEntry {
    path_hash: u32,
    cluster: u32,
}

static DIR_CACHE: crate::sync::mutex::Mutex<[DirCacheEntry; DIR_CACHE_SIZE]> =
    crate::sync::mutex::Mutex::new(
        [DirCacheEntry {
            path_hash: 0,
            cluster: 0,
        }; DIR_CACHE_SIZE],
    );
static DIR_CACHE_TICK: AtomicU32 = AtomicU32::new(0);

/// Simple DJB2 hash for path strings.
pub fn path_hash(path: &str) -> u32 {
    let mut hash = 5381u32;
    for &byte in path.as_bytes() {
        hash = hash.wrapping_mul(33).wrapping_add(byte as u32);
    }
    hash
}

/// Look up a cached directory cluster for a path.
pub fn dir_cache_lookup(hash: u32) -> Option<u32> {
    let idx = (hash as usize) & (DIR_CACHE_SIZE - 1);
    let cache = DIR_CACHE.lock();
    let entry = cache[idx];
    if entry.path_hash == hash && entry.cluster != 0 {
        Some(entry.cluster)
    } else {
        None
    }
}

/// Cache a directory cluster for a path.
pub fn dir_cache_insert(hash: u32, cluster: u32) {
    let idx = (hash as usize) & (DIR_CACHE_SIZE - 1);
    let mut cache = DIR_CACHE.lock();
    let _tick = DIR_CACHE_TICK.fetch_add(1, Ordering::Relaxed);
    cache[idx] = DirCacheEntry {
        path_hash: hash,
        cluster,
    };
}

/// Invalidate all directory cache entries (called on mkdir/rmdir/rename).
pub fn dir_cache_invalidate() {
    let mut cache = DIR_CACHE.lock();
    for entry in cache.iter_mut() {
        entry.path_hash = 0;
        entry.cluster = 0;
    }
}
