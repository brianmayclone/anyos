use alloc::string::String;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};

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

/// iframe snapshot pseudo-images are produced by the snapshot renderer, not
/// fetched from the network; once evicted they cannot be re-decoded on
/// demand. They must never be auto-evicted (see `iframe_snapshot_key`).
fn is_iframe_snapshot_src(src: &str) -> bool {
    src.starts_with("__iframe_")
}

/// LRU cache of decoded images with a total byte-size cap.
pub struct ImageCache {
    pub entries: Vec<ImageEntry>,
    generation: Cell<u64>,
    total_bytes: usize,
    /// Sources whose pixels were evicted but were looked up again during
    /// rasterization. Drained by the embedder to re-fetch/re-decode them.
    evicted_misses: RefCell<Vec<String>>,
}

impl ImageCache {
    pub fn new() -> Self {
        ImageCache {
            entries: Vec::new(),
            generation: Cell::new(0),
            total_bytes: 0,
            evicted_misses: RefCell::new(Vec::new()),
        }
    }

    fn note_evicted_miss(&self, src: &str) {
        let mut misses = self.evicted_misses.borrow_mut();
        if misses.len() >= 64 || misses.iter().any(|s| s == src) {
            return;
        }
        misses.push(String::from(src));
    }

    /// Drain the evicted-but-needed sources recorded during rasterization.
    pub fn take_evicted_misses(&mut self) -> Vec<String> {
        core::mem::take(&mut *self.evicted_misses.borrow_mut())
    }

    fn bump_generation(&self) -> u64 {
        let next = self.generation.get().saturating_add(1);
        self.generation.set(next);
        next
    }

    /// Look up a cached image by URL.  Bumps the LRU generation on hit.
    pub fn get(&mut self, src: &str) -> Option<&ImageEntry> {
        let gen = self.bump_generation();
        if let Some(entry) = self.entries.iter().find(|e| e.src == src) {
            entry.generation.set(gen);
            if !entry.has_pixels() {
                self.note_evicted_miss(src);
            }
            return Some(entry);
        }
        None
    }

    /// Read-only lookup. Bumps the LRU generation so painted images stay hot.
    pub fn get_ref(&self, src: &str) -> Option<&ImageEntry> {
        if let Some(entry) = self.entries.iter().find(|e| e.src == src) {
            let gen = self.bump_generation();
            entry.generation.set(gen);
            if !entry.has_pixels() {
                // Pixels were evicted under memory pressure but the painter
                // still needs them: ask the embedder to re-fetch.
                self.note_evicted_miss(src);
            }
            return Some(entry);
        }
        None
    }

    pub fn has_pixels_for(&self, src: &str) -> bool {
        self.get_ref(src).is_some_and(|e| e.has_pixels())
    }

    /// Add a decoded image.  Evicts LRU entries if the cache exceeds the byte cap.
    pub fn add(&mut self, src: String, pixels: Vec<u32>, width: u32, height: u32) {
        let expected_pixels = (width as usize).saturating_mul(height as usize);
        if width == 0 || height == 0 || pixels.len() < expected_pixels {
            return;
        }
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
        self.evicted_misses.borrow_mut().clear();
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
                .filter(|(_, e)| !is_iframe_snapshot_src(&e.src))
                .filter(|(_, e)| e.byte_size() > IMAGE_CACHE_SMALL_IMAGE_PROTECT_BYTES)
                .min_by_key(|(_, e)| e.generation.get())
                .map(|(i, _)| i)
                .or_else(|| {
                    self.entries
                        .iter()
                        .enumerate()
                        .filter(|(_, e)| !e.pixels.is_empty())
                        .filter(|(_, e)| !is_iframe_snapshot_src(&e.src))
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
