//! Font cache — maps file paths to SHM IDs.
//!
//! Once a font is loaded into SHM, it stays for the lifetime of fontd.
//! All lookups are by full path (case-sensitive).

use anyos_std::String;

const MAX_ENTRIES: usize = 64;

struct CacheEntry {
    path: String,
    shm_id: u32,
    size: u32,
}

static mut ENTRIES: Option<Entries> = None;

struct Entries {
    items: [Option<CacheEntry>; MAX_ENTRIES],
}

pub fn init() {
    unsafe {
        ENTRIES = Some(Entries {
            items: core::array::from_fn(|_| None),
        });
    }
}

fn entries() -> &'static mut Entries {
    unsafe { ENTRIES.as_mut().unwrap() }
}

/// Look up a font by path. Returns (shm_id, size) if cached.
pub fn lookup(path: &str) -> Option<(u32, u32)> {
    for slot in entries().items.iter() {
        if let Some(e) = slot {
            if e.path == path {
                return Some((e.shm_id, e.size));
            }
        }
    }
    None
}

/// Insert a newly loaded font into the cache.
pub fn insert(path: &str, shm_id: u32, size: u32) {
    for slot in entries().items.iter_mut() {
        if slot.is_none() {
            *slot = Some(CacheEntry {
                path: String::from(path),
                shm_id,
                size,
            });
            return;
        }
    }
    // Cache full — overwrite the last slot
    let last = &mut entries().items[MAX_ENTRIES - 1];
    *last = Some(CacheEntry {
        path: String::from(path),
        shm_id,
        size,
    });
}
