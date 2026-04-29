use alloc::vec::Vec;

use libanyui_client as ui;

// ═══════════════════════════════════════════════════════════════════════════
// Tile cache (pixel data)
// ═══════════════════════════════════════════════════════════════════════════

pub(super) const TILE_HEIGHT: u32 = 256;
const MAX_CACHED_TILES: usize = 40;
pub(super) const BUFFER_ZONE: i32 = 768;
pub(super) const MAX_TILE_CANVASES: usize = 30;
pub(super) const MAX_TILES_PER_SCROLL_TICK: usize = 1;
pub(super) const MAX_TILES_PER_IDLE_TICK: usize = 4;
pub(super) const INITIAL_VISIBLE_EXTRA_ROWS: u32 = 1;

struct CachedTile {
    row: u32,
    pixels: Vec<u32>,
    generation: u64,
}

pub(super) struct TileCache {
    pub(super) tiles: Vec<CachedTile>,
    pub(super) generation: u64,
    /// Pool of reusable pixel buffers (avoids alloc per tile).
    pub(super) free_bufs: Vec<Vec<u32>>,
}

impl TileCache {
    pub(super) fn new() -> Self {
        Self {
            tiles: Vec::new(),
            generation: 0,
            free_bufs: Vec::new(),
        }
    }

    pub(super) fn get(&self, row: u32) -> Option<&[u32]> {
        self.tiles
            .iter()
            .find(|t| t.row == row)
            .map(|t| t.pixels.as_slice())
    }

    pub(super) fn insert(&mut self, row: u32, pixels: Vec<u32>) {
        self.generation += 1;
        let gen = self.generation;

        if let Some(tile) = self.tiles.iter_mut().find(|t| t.row == row) {
            // Return old buffer to pool before replacing.
            let old = core::mem::replace(&mut tile.pixels, pixels);
            if self.free_bufs.len() < 8 {
                self.free_bufs.push(old);
            }
            tile.generation = gen;
            return;
        }

        if self.tiles.len() >= MAX_CACHED_TILES {
            let min_idx = self
                .tiles
                .iter()
                .enumerate()
                .min_by_key(|(_, t)| t.generation)
                .map(|(i, _)| i)
                .unwrap_or(0);
            let evicted = self.tiles.swap_remove(min_idx);
            if self.free_bufs.len() < 8 {
                self.free_bufs.push(evicted.pixels);
            }
        }

        self.tiles.push(CachedTile {
            row,
            pixels,
            generation: gen,
        });
    }

    pub(super) fn invalidate_all(&mut self) {
        for tile in self.tiles.drain(..) {
            if self.free_bufs.len() < 8 {
                self.free_bufs.push(tile.pixels);
            }
        }
        self.generation = 0;
    }

    /// Take a buffer from the pool or allocate a new one.
    pub(super) fn take_buf(&mut self, pixel_count: usize, clear_color: u32) -> Vec<u32> {
        if let Some(mut buf) = self.free_bufs.pop() {
            buf.resize(pixel_count, clear_color);
            for px in buf.iter_mut() {
                *px = clear_color;
            }
            buf
        } else {
            let mut buf = Vec::with_capacity(pixel_count);
            buf.resize(pixel_count, clear_color);
            buf
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Tile canvas
// ═══════════════════════════════════════════════════════════════════════════

pub(super) struct TileCanvas {
    pub(super) row: u32,
    pub(super) canvas: ui::Canvas,
}
