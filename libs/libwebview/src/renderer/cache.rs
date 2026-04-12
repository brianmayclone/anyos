use alloc::string::String;
use alloc::vec::Vec;

// ═══════════════════════════════════════════════════════════════════════════
// Image cache
// ═══════════════════════════════════════════════════════════════════════════

/// Maximum total decoded image bytes in the cache.
///
/// The old 512 MiB cap protected against aggressive evictions, but it also let
/// Surf accumulate far too much decoded image memory on pages like Heise. That
/// pushed allocator pressure and made failures around `sbrk` much more likely.
/// Keep the cache large enough for smooth scrolling, but substantially smaller.
const IMAGE_CACHE_MAX_BYTES: usize = 192 * 1024 * 1024;
/// When we cross the hard cap, trim to a softer target so we do not thrash
/// around the limit by evicting only one entry per insertion.
const IMAGE_CACHE_TRIM_TARGET_BYTES: usize = 144 * 1024 * 1024;
pub const PROGRESSIVE_BAND_VIEWPORTS_BEFORE: i32 = 1;
pub const PROGRESSIVE_BAND_VIEWPORTS_AFTER: i32 = 3;

/// Image cache entry (decoded pixel data).
pub struct ImageEntry {
    pub src: String,
    pub pixels: Vec<u32>,
    pub width: u32,
    pub height: u32,
    /// LRU generation (higher = more recently used).
    generation: u64,
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
    generation: u64,
    total_bytes: usize,
}

impl ImageCache {
    pub fn new() -> Self {
        ImageCache {
            entries: Vec::new(),
            generation: 0,
            total_bytes: 0,
        }
    }

    /// Look up a cached image by URL.  Bumps the LRU generation on hit.
    pub fn get(&mut self, src: &str) -> Option<&ImageEntry> {
        self.generation += 1;
        let gen = self.generation;
        if let Some(entry) = self.entries.iter_mut().find(|e| e.src == src) {
            entry.generation = gen;
            return Some(entry);
        }
        None
    }

    /// Read-only lookup (no LRU bump).
    pub fn get_ref(&self, src: &str) -> Option<&ImageEntry> {
        self.entries.iter().find(|e| e.src == src)
    }

    pub fn has_pixels_for(&self, src: &str) -> bool {
        self.get_ref(src).is_some_and(|e| e.has_pixels())
    }

    /// Add a decoded image.  Evicts LRU entries if the cache exceeds the byte cap.
    pub fn add(&mut self, src: String, pixels: Vec<u32>, width: u32, height: u32) {
        let new_bytes = pixels.len() * 4;

        if let Some(entry) = self.entries.iter_mut().find(|e| e.src == src) {
            self.total_bytes -= entry.byte_size();
            entry.pixels = pixels;
            entry.width = width;
            entry.height = height;
            self.generation += 1;
            entry.generation = self.generation;
            self.total_bytes += new_bytes;
            self.evict_to_budget();
            return;
        }

        self.generation += 1;
        let gen = self.generation;
        self.entries.push(ImageEntry {
            src,
            pixels,
            width,
            height,
            generation: gen,
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
            let Some(min_idx) = self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, e)| !e.pixels.is_empty())
                .min_by_key(|(_, e)| e.generation)
                .map(|(i, _)| i)
            else {
                break;
            };

            let entry = &mut self.entries[min_idx];
            self.total_bytes -= entry.byte_size();
            entry.pixels.clear();
        }
    }
}
