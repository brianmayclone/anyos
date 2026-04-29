use alloc::string::String;
use alloc::vec::Vec;
use core::cell::Cell;

// ═══════════════════════════════════════════════════════════════════════════
// Image cache
// ═══════════════════════════════════════════════════════════════════════════

/// Maximum total decoded image bytes in the cache.
///
/// Surf is a desktop browser and should use available RAM to avoid decoding
/// and re-fetching images while scrolling image-heavy pages.
const IMAGE_CACHE_MAX_BYTES: usize = 384 * 1024 * 1024;
/// When we cross the hard cap, trim to a softer target so we do not thrash
/// around the limit by evicting only one entry per insertion.
const IMAGE_CACHE_TRIM_TARGET_BYTES: usize = 288 * 1024 * 1024;
/// Small UI images and inline SVG logos are cheap but visually important.
/// Prefer evicting article-sized images before dropping these.
const IMAGE_CACHE_SMALL_IMAGE_PROTECT_BYTES: usize = 2 * 1024 * 1024;
pub const PROGRESSIVE_BAND_VIEWPORTS_BEFORE: i32 = 1;
pub const PROGRESSIVE_BAND_VIEWPORTS_AFTER: i32 = 3;

/// Image cache entry (decoded pixel data).
pub struct ImageEntry {
    pub src: String,
    pub pixels: Vec<u32>,
    pub width: u32,
    pub height: u32,
    /// LRU generation (higher = more recently used).
    generation: Cell<u64>,
}

impl ImageEntry {
    /// Size in bytes of the decoded pixel data.
    fn byte_size(&self) -> usize {
        self.pixels.len() * 4
    }

    pub(super) fn has_pixels(&self) -> bool {
        !self.pixels.is_empty() && self.width > 0 && self.height > 0
    }
}

/// LRU cache of decoded images with a total byte-size cap.
pub struct ImageCache {
    pub entries: Vec<ImageEntry>,
    generation: Cell<u64>,
    total_bytes: usize,
}

impl ImageCache {
    pub fn new() -> Self {
        ImageCache {
            entries: Vec::new(),
            generation: Cell::new(0),
            total_bytes: 0,
        }
    }

    fn bump_generation(&self) -> u64 {
        let next = self.generation.get().saturating_add(1);
        self.generation.set(next);
        next
    }

    /// Look up a cached image by URL.  Bumps the LRU generation on hit.
    pub fn get(&mut self, src: &str) -> Option<&ImageEntry> {
        let gen = self.bump_generation();
        if let Some(entry) = self.entries.iter_mut().find(|e| e.src == src) {
            entry.generation.set(gen);
            return Some(entry);
        }
        None
    }

    /// Read-only lookup. Bumps the LRU generation so painted images stay hot.
    pub fn get_ref(&self, src: &str) -> Option<&ImageEntry> {
        if let Some(entry) = self.entries.iter().find(|e| e.src == src) {
            let gen = self.bump_generation();
            entry.generation.set(gen);
            return Some(entry);
        }
        None
    }

    pub fn has_pixels_for(&self, src: &str) -> bool {
        self.get_ref(src).is_some_and(|e| e.has_pixels())
    }

    /// Add a decoded image.  Evicts LRU entries if the cache exceeds the byte cap.
    pub fn add(&mut self, src: String, pixels: Vec<u32>, width: u32, height: u32) {
        let new_bytes = pixels.len() * 4;

        if let Some(idx) = self.entries.iter().position(|e| e.src == src) {
            let gen = self.bump_generation();
            let entry = &mut self.entries[idx];
            self.total_bytes -= entry.byte_size();
            entry.pixels = pixels;
            entry.width = width;
            entry.height = height;
            entry.generation.set(gen);
            self.total_bytes += new_bytes;
            self.evict_to_budget();
            return;
        }

        let gen = self.bump_generation();
        self.entries.push(ImageEntry {
            src,
            pixels,
            width,
            height,
            generation: Cell::new(gen),
        });
        self.total_bytes += new_bytes;
        self.evict_to_budget();
    }

    /// Drop all cached images (called on page navigation).
    pub fn clear(&mut self) {
        self.entries.clear();
        self.total_bytes = 0;
    }

    fn evict_to_budget(&mut self) {
        let target = if self.total_bytes > IMAGE_CACHE_MAX_BYTES {
            IMAGE_CACHE_TRIM_TARGET_BYTES
        } else {
            IMAGE_CACHE_MAX_BYTES
        };
        while self.total_bytes > target {
            let evict_idx = self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, e)| !e.pixels.is_empty())
                .filter(|(_, e)| e.byte_size() > IMAGE_CACHE_SMALL_IMAGE_PROTECT_BYTES)
                .min_by_key(|(_, e)| e.generation.get())
                .map(|(i, _)| i)
                .or_else(|| {
                    self.entries
                        .iter()
                        .enumerate()
                        .filter(|(_, e)| !e.pixels.is_empty())
                        .min_by_key(|(_, e)| e.generation.get())
                        .map(|(i, _)| i)
                });

            let Some(min_idx) = evict_idx else { break };

            let entry = &mut self.entries[min_idx];
            self.total_bytes -= entry.byte_size();
            entry.pixels.clear();
        }
    }
}
