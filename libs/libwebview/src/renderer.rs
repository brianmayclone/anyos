//! Display-list renderer with compositor-driven smooth scrolling.
//!
//! After layout, the tree is flattened into a sorted `Vec<DrawCmd>` (the
//! display list).  Each tile is rasterized by binary-searching for the
//! first command that overlaps the tile Y range, then linearly executing
//! commands until they fall below the tile.  This is O(k) per tile where
//! k = commands visible in the tile, compared to O(n) for a full tree walk.

use alloc::string::String;
use alloc::vec::Vec;

use libanyui_client::{self as ui, Widget};

use crate::layout::{LayoutBox, FormFieldKind};
use crate::style::TextDeco;

// ═══════════════════════════════════════════════════════════════════════════
// Image cache
// ═══════════════════════════════════════════════════════════════════════════

/// Maximum total decoded image bytes in the cache (128 MiB).
const IMAGE_CACHE_MAX_BYTES: usize = 128 * 1024 * 1024;

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
}

/// LRU cache of decoded images with a total byte-size cap.
pub struct ImageCache {
    pub entries: Vec<ImageEntry>,
    generation: u64,
    total_bytes: usize,
}

impl ImageCache {
    pub fn new() -> Self {
        ImageCache { entries: Vec::new(), generation: 0, total_bytes: 0 }
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
        self.entries.push(ImageEntry { src, pixels, width, height, generation: gen });
        self.total_bytes += new_bytes;
        self.evict_to_budget();
    }

    /// Drop all cached images (called on page navigation).
    pub fn clear(&mut self) {
        self.entries.clear();
        self.total_bytes = 0;
    }

    fn evict_to_budget(&mut self) {
        while self.total_bytes > IMAGE_CACHE_MAX_BYTES && !self.entries.is_empty() {
            let min_idx = self.entries.iter().enumerate()
                .min_by_key(|(_, e)| e.generation)
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.total_bytes -= self.entries[min_idx].byte_size();
            self.entries.swap_remove(min_idx);
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Hit regions
// ═══════════════════════════════════════════════════════════════════════════

pub struct HitRegion {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub kind: HitKind,
}

pub enum HitKind {
    Link(String),
    Submit(usize),
}

// ═══════════════════════════════════════════════════════════════════════════
// Persistent form controls
// ═══════════════════════════════════════════════════════════════════════════

pub struct FormControl {
    pub control_id: u32,
    pub node_id: usize,
    pub kind: FormFieldKind,
    pub name: String,
    seen: bool,
}

// ═══════════════════════════════════════════════════════════════════════════
// Display list — flat, Y-sorted draw commands
// ═══════════════════════════════════════════════════════════════════════════

/// A single draw command in absolute document coordinates.
struct DrawCmd {
    /// Absolute document X.
    x: i32,
    /// Absolute document Y (sort key).
    y: i32,
    /// Width of the drawn element.
    w: i32,
    /// Height of the drawn element.
    h: i32,
    /// The drawing operation.
    kind: DrawKind,
}

enum DrawKind {
    /// Fill a rectangle with a solid or alpha-blended color.
    Rect { color: u32 },
    /// Draw a text string.
    Text { color: u32, font_id: u32, font_size: u16, text: String },
    /// Blit an image (looked up from ImageCache by src URL at rasterize time).
    Image { src: String },
}

/// A Y-sorted display list built from the layout tree.
///
/// Replacing the recursive `walk_pixels` tree walk with a flat sorted list
/// allows O(log n + k) tile rasterization (binary search + k visible commands)
/// instead of O(n) per tile.
pub(crate) struct DisplayList {
    cmds: Vec<DrawCmd>,
    /// Maximum command height seen — used as search margin for binary search.
    max_h: i32,
}

impl DisplayList {
    pub fn new() -> Self {
        Self { cmds: Vec::new(), max_h: 0 }
    }

    /// Build the display list from a layout tree.  Walks the tree once,
    /// emitting DrawCmds for every visible element, then sorts by Y.
    pub fn build(root: &LayoutBox) -> Self {
        let mut dl = DisplayList { cmds: Vec::new(), max_h: 0 };
        dl.flatten(root, 0, 0);
        dl.cmds.sort_unstable_by_key(|c| c.y);
        dl
    }

    /// Clear the display list (called on relayout / navigation).
    pub fn clear(&mut self) {
        self.cmds.clear();
        self.max_h = 0;
    }

    /// Find the first command index whose Y >= `y_min` using binary search.
    fn search_start(&self, y_min: i32) -> usize {
        self.cmds.partition_point(|c| c.y < y_min)
    }

    /// Rasterize all commands overlapping `[tile_y_start, tile_y_end)` into `buf`.
    pub fn rasterize_tile(
        &self,
        images: &ImageCache,
        buf: *mut u32,
        stride: u32,
        buf_h: u32,
        tile_y_start: i32,
        tile_y_end: i32,
    ) {
        // Search for the first command that could overlap the tile.
        // A command at y with height h overlaps if y + h > tile_y_start,
        // i.e. y > tile_y_start - h.  We use max_h as conservative upper bound.
        let search_y = tile_y_start - self.max_h;
        let start = self.search_start(search_y);

        for i in start..self.cmds.len() {
            let cmd = &self.cmds[i];
            // Past the tile — all remaining commands are below (sorted by Y).
            if cmd.y >= tile_y_end {
                break;
            }
            // Check overlap: command bottom > tile top.
            if cmd.y + cmd.h <= tile_y_start {
                continue;
            }

            let draw_y = cmd.y - tile_y_start;

            match &cmd.kind {
                DrawKind::Rect { color } => {
                    fill_rect_buf(buf, stride, buf_h, cmd.x, draw_y, cmd.w, cmd.h, *color);
                }
                DrawKind::Text { color, font_id, font_size, text } => {
                    libfont_client::draw_string_buf(
                        buf, stride, buf_h,
                        cmd.x, draw_y,
                        *color, *font_id, *font_size,
                        text,
                    );
                }
                DrawKind::Image { src } => {
                    if let Some(entry) = images.get_ref(src) {
                        blit_image_buf(
                            buf, stride, buf_h,
                            cmd.x, draw_y, cmd.w, cmd.h,
                            &entry.pixels, entry.width, entry.height,
                        );
                    }
                }
            }
        }
    }

    /// Recursively flatten the layout tree into draw commands.
    fn flatten(&mut self, bx: &LayoutBox, offset_x: i32, offset_y: i32) {
        if bx.visibility_hidden {
            return;
        }

        let abs_x = if bx.is_fixed { bx.x } else { offset_x + bx.x };
        let abs_y = if bx.is_fixed { bx.y } else { offset_y + bx.y };

        // Background.
        if bx.bg_color != 0 && bx.bg_color != 0x00000000 {
            self.push(abs_x, abs_y, bx.width, bx.height, DrawKind::Rect { color: bx.bg_color });
        }

        // Border (4 edges).
        if bx.border_width > 0 && bx.border_color != 0 && bx.border_color != 0x00000000 {
            let bw = bx.border_width;
            let w = bx.width;
            let h = bx.height;
            self.push(abs_x, abs_y, w, bw, DrawKind::Rect { color: bx.border_color });
            self.push(abs_x, abs_y + h - bw, w, bw, DrawKind::Rect { color: bx.border_color });
            let inner_h = (h - bw * 2).max(0);
            self.push(abs_x, abs_y + bw, bw, inner_h, DrawKind::Rect { color: bx.border_color });
            self.push(abs_x + w - bw, abs_y + bw, bw, inner_h, DrawKind::Rect { color: bx.border_color });
        }

        // Horizontal rule.
        if bx.is_hr {
            self.push(abs_x, abs_y, bx.width, 1, DrawKind::Rect { color: 0xFF999999 });
        }

        // List marker.
        if let Some(ref marker) = bx.list_marker {
            let font_size = bx.font_size.max(1) as u16;
            let color = if bx.color != 0 { bx.color } else { 0xFF000000 };
            self.push(abs_x - 20, abs_y, 20, bx.height,
                DrawKind::Text { color, font_id: 0, font_size, text: marker.clone() });
        }

        // Text fragment.
        if let Some(ref text) = bx.text {
            if !text.is_empty() && bx.form_field.is_none() {
                let font_id = if bx.bold { 1u32 } else if bx.italic { 3u32 } else { 0u32 };
                let font_size = bx.font_size.max(1) as u16;
                let color = if bx.color != 0 { bx.color } else { 0xFF000000 };

                self.push(abs_x, abs_y, bx.width, bx.height,
                    DrawKind::Text { color, font_id, font_size, text: text.clone() });

                // Underline.
                if bx.text_decoration == TextDeco::Underline || bx.link_url.is_some() {
                    self.push(abs_x, abs_y + bx.height - 1, bx.width, 1,
                        DrawKind::Rect { color });
                }

                // Line-through.
                if bx.text_decoration == TextDeco::LineThrough {
                    self.push(abs_x, abs_y + bx.height / 2, bx.width, 1,
                        DrawKind::Rect { color });
                }
            }
        }

        // Image.
        if let Some(ref src) = bx.image_src {
            let dw = bx.image_width.unwrap_or(bx.width);
            let dh = bx.image_height.unwrap_or(bx.height);
            self.push(abs_x, abs_y, dw, dh, DrawKind::Image { src: src.clone() });
        }

        // Submit/button pixel drawing.
        if let Some(kind) = bx.form_field {
            if matches!(kind, FormFieldKind::Submit | FormFieldKind::ButtonEl) {
                self.emit_submit(abs_x, abs_y, bx);
            }
        }

        // Recurse into children.
        for child in &bx.children {
            let (cx, cy) = if bx.is_fixed { (bx.x, bx.y) } else { (abs_x, abs_y) };
            self.flatten(child, cx, cy);
        }
    }

    /// Emit draw commands for a submit/button element.
    fn emit_submit(&mut self, x: i32, y: i32, bx: &LayoutBox) {
        let label_text = if let Some(ref t) = bx.text { t.clone() } else { String::from("Submit") };

        // Default web button bg + border if no CSS styling.
        if bx.bg_color == 0 && bx.border_width == 0 {
            self.push(x, y, bx.width, bx.height, DrawKind::Rect { color: 0xFFE0E0E0 });
            self.push(x, y, bx.width, 1, DrawKind::Rect { color: 0xFF808080 });
            self.push(x, y + bx.height - 1, bx.width, 1, DrawKind::Rect { color: 0xFF808080 });
            self.push(x, y + 1, 1, (bx.height - 2).max(0), DrawKind::Rect { color: 0xFF808080 });
            self.push(x + bx.width - 1, y + 1, 1, (bx.height - 2).max(0), DrawKind::Rect { color: 0xFF808080 });
        }

        // Center text in button.
        let font_size = bx.font_size.max(1) as u16;
        let text_color = if bx.color != 0 { bx.color } else { 0xFF000000 };
        let (tw, _) = libfont_client::measure(0, font_size, &label_text);
        let tx = x + (bx.width - tw as i32) / 2;
        let ty = y + (bx.height - font_size as i32) / 2;
        self.push(tx, ty, tw as i32, font_size as i32,
            DrawKind::Text { color: text_color, font_id: 0, font_size, text: label_text });
    }

    #[inline]
    fn push(&mut self, x: i32, y: i32, w: i32, h: i32, kind: DrawKind) {
        if h > self.max_h { self.max_h = h; }
        self.cmds.push(DrawCmd { x, y, w, h, kind });
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Tile cache (pixel data)
// ═══════════════════════════════════════════════════════════════════════════

const TILE_HEIGHT: u32 = 256;
const MAX_CACHED_TILES: usize = 40;
const BUFFER_ZONE: i32 = 768;
const MAX_TILE_CANVASES: usize = 30;
const MAX_TILES_PER_TICK: usize = 8;

struct CachedTile {
    row: u32,
    pixels: Vec<u32>,
    generation: u64,
}

struct TileCache {
    tiles: Vec<CachedTile>,
    generation: u64,
    /// Pool of reusable pixel buffers (avoids alloc per tile).
    free_bufs: Vec<Vec<u32>>,
}

impl TileCache {
    fn new() -> Self {
        Self { tiles: Vec::new(), generation: 0, free_bufs: Vec::new() }
    }

    fn get(&self, row: u32) -> Option<&[u32]> {
        self.tiles.iter()
            .find(|t| t.row == row)
            .map(|t| t.pixels.as_slice())
    }

    fn insert(&mut self, row: u32, pixels: Vec<u32>) {
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
            let min_idx = self.tiles.iter().enumerate()
                .min_by_key(|(_, t)| t.generation)
                .map(|(i, _)| i)
                .unwrap_or(0);
            let evicted = self.tiles.swap_remove(min_idx);
            if self.free_bufs.len() < 8 {
                self.free_bufs.push(evicted.pixels);
            }
        }

        self.tiles.push(CachedTile { row, pixels, generation: gen });
    }

    fn invalidate_all(&mut self) {
        for tile in self.tiles.drain(..) {
            if self.free_bufs.len() < 8 {
                self.free_bufs.push(tile.pixels);
            }
        }
        self.generation = 0;
    }

    /// Take a buffer from the pool or allocate a new one.
    fn take_buf(&mut self, pixel_count: usize, clear_color: u32) -> Vec<u32> {
        if let Some(mut buf) = self.free_bufs.pop() {
            buf.resize(pixel_count, clear_color);
            for px in buf.iter_mut() { *px = clear_color; }
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

struct TileCanvas {
    row: u32,
    canvas: ui::Canvas,
}

// ═══════════════════════════════════════════════════════════════════════════
// Renderer
// ═══════════════════════════════════════════════════════════════════════════

pub(crate) struct Renderer {
    tile_canvases: Vec<TileCanvas>,
    tile_cache: TileCache,
    doc_w: u32,
    doc_h: u32,
    pub hit_regions: Vec<HitRegion>,
    pub form_controls: Vec<FormControl>,
    pub link_map: Vec<(u32, String)>,
    link_cb: Option<ui::Callback>,
    link_cb_ud: u64,
    last_scroll_y: i32,
    /// The display list — built once after layout, used for all tile rasterization.
    display_list: DisplayList,
}

impl Renderer {
    pub fn new() -> Self {
        Self {
            tile_canvases: Vec::new(),
            tile_cache: TileCache::new(),
            doc_w: 0,
            doc_h: 0,
            hit_regions: Vec::new(),
            form_controls: Vec::new(),
            link_map: Vec::new(),
            link_cb: None,
            link_cb_ud: 0,
            last_scroll_y: 0,
            display_list: DisplayList::new(),
        }
    }

    pub fn tile_hit_coords(&self, ctrl_id: u32) -> Option<(i32, i32)> {
        for tc in &self.tile_canvases {
            if tc.canvas.id() == ctrl_id {
                let (mx, my, _) = tc.canvas.get_mouse();
                let doc_y = my + (tc.row * TILE_HEIGHT) as i32;
                return Some((mx, doc_y));
            }
        }
        None
    }

    pub fn control_count(&self) -> usize {
        self.form_controls.len()
    }

    /// Soft clear: reset hit regions, invalidate tile cache, destroy canvases.
    pub fn clear(&mut self) {
        self.hit_regions.clear();
        self.link_map.clear();
        self.tile_cache.invalidate_all();
        for tc in self.tile_canvases.drain(..) {
            ui::Control::from_id(tc.canvas.id()).remove();
        }
        for fc in &mut self.form_controls {
            fc.seen = false;
        }
        self.display_list.clear();
    }

    /// Hard clear: destroy everything.
    pub fn clear_all(&mut self) {
        for fc in &self.form_controls {
            if fc.control_id != 0 {
                ui::Control::from_id(fc.control_id).remove();
            }
        }
        self.form_controls.clear();
        for tc in self.tile_canvases.drain(..) {
            ui::Control::from_id(tc.canvas.id()).remove();
        }
        self.doc_w = 0;
        self.doc_h = 0;
        self.hit_regions.clear();
        self.link_map.clear();
        self.tile_cache.invalidate_all();
        self.link_cb = None;
        self.link_cb_ud = 0;
        self.last_scroll_y = 0;
        self.display_list.clear();
    }

    pub fn hit_test_link_at(&self, x: i32, doc_y: i32) -> Option<&str> {
        for region in &self.hit_regions {
            if x >= region.x && x < region.x + region.w
                && doc_y >= region.y && doc_y < region.y + region.h
            {
                if let HitKind::Link(ref url) = region.kind {
                    return Some(url.as_str());
                }
            }
        }
        None
    }

    pub fn hit_test_submit_at(&self, x: i32, doc_y: i32) -> Option<usize> {
        for region in &self.hit_regions {
            if x >= region.x && x < region.x + region.w
                && doc_y >= region.y && doc_y < region.y + region.h
            {
                if let HitKind::Submit(node_id) = region.kind {
                    return Some(node_id);
                }
            }
        }
        None
    }

    // ─────────────────────────────────────────────────────────────────────
    // Full render (relayout path)
    // ─────────────────────────────────────────────────────────────────────

    pub fn render(
        &mut self,
        root: &LayoutBox,
        parent: &ui::View,
        images: &ImageCache,
        doc_w: u32,
        doc_h: u32,
        viewport_h: u32,
        scroll_y: i32,
        bg_color: u32,
        link_cb: Option<ui::Callback>,
        link_cb_ud: u64,
        submit_cb: Option<ui::Callback>,
        submit_cb_ud: u64,
    ) {
        crate::debug_surf!("[render] full render start ({}x{}, vp_h={}, scroll_y={})",
            doc_w, doc_h, viewport_h, scroll_y);

        let w = doc_w.max(1);
        let clear_color = if bg_color != 0 { bg_color } else { 0xFFFFFFFF };

        self.doc_w = w;
        self.doc_h = doc_h;
        self.link_cb = link_cb;
        self.link_cb_ud = link_cb_ud;
        self.last_scroll_y = scroll_y;

        // 1. Invalidate tile cache (layout has changed).
        self.tile_cache.invalidate_all();

        // 2. Walk full tree for form controls + hit regions.
        self.walk_controls(root, 0, 0, parent, submit_cb, submit_cb_ud);

        // 3. Build display list (flat, Y-sorted draw commands).
        self.display_list = DisplayList::build(root);
        crate::debug_surf!("[render] display list: {} commands, max_h={}",
            self.display_list.cmds.len(), self.display_list.max_h);

        // 4. Compute visible tile rows.
        let render_y_start = (scroll_y - BUFFER_ZONE).max(0);
        let render_y_end = (scroll_y + viewport_h as i32 + BUFFER_ZONE).min(doc_h as i32);
        let first_row = render_y_start as u32 / TILE_HEIGHT;
        let last_row = if render_y_end > 0 {
            ((render_y_end - 1) as u32) / TILE_HEIGHT
        } else {
            0
        };

        // 5. Rasterize visible tiles using the display list.
        for row in first_row..=last_row {
            let tile_buf = self.rasterize_tile_dl(images, w, row, doc_h, clear_color);
            self.tile_cache.insert(row, tile_buf);
            self.create_tile_canvas(row, w, doc_h, parent);
        }

        // 6. GC unseen form controls.
        self.form_controls.retain(|fc| {
            if !fc.seen && fc.control_id != 0 {
                ui::Control::from_id(fc.control_id).remove();
                false
            } else {
                fc.seen || fc.control_id == 0
            }
        });

        crate::debug_surf!("[render] full render done: {} tile canvases, {} hit_regions, {} form_controls",
            self.tile_canvases.len(), self.hit_regions.len(), self.form_controls.len());
    }

    // ─────────────────────────────────────────────────────────────────────
    // Scroll render (fast path)
    // ─────────────────────────────────────────────────────────────────────

    pub fn render_scroll(
        &mut self,
        _root: &LayoutBox,
        parent: &ui::View,
        images: &ImageCache,
        doc_w: u32,
        doc_h: u32,
        viewport_h: u32,
        scroll_y: i32,
        bg_color: u32,
        _link_cb: Option<ui::Callback>,
        _link_cb_ud: u64,
    ) -> bool {
        let w = doc_w.max(1);
        let clear_color = if bg_color != 0 { bg_color } else { 0xFFFFFFFF };

        self.doc_w = w;
        self.doc_h = doc_h;
        self.last_scroll_y = scroll_y;

        let render_y_start = (scroll_y - BUFFER_ZONE).max(0);
        let render_y_end = (scroll_y + viewport_h as i32 + BUFFER_ZONE).min(doc_h as i32);
        let first_row = render_y_start as u32 / TILE_HEIGHT;
        let last_row = if render_y_end > 0 {
            ((render_y_end - 1) as u32) / TILE_HEIGHT
        } else {
            0
        };

        let mut rasterized = 0usize;
        let mut pending = false;
        for row in first_row..=last_row {
            if self.tile_canvases.iter().any(|tc| tc.row == row) {
                continue;
            }

            if self.tile_cache.get(row).is_none() {
                if rasterized >= MAX_TILES_PER_TICK {
                    pending = true;
                    continue;
                }
                let tile_buf = self.rasterize_tile_dl(images, w, row, doc_h, clear_color);
                self.tile_cache.insert(row, tile_buf);
                rasterized += 1;
            }

            self.create_tile_canvas(row, w, doc_h, parent);
        }

        // Evict distant tile canvases.
        let keep_first = first_row.saturating_sub(4);
        let keep_last = (last_row + 4).min(if doc_h > 0 { (doc_h - 1) / TILE_HEIGHT } else { 0 });
        self.tile_canvases.retain(|tc| {
            if tc.row < keep_first || tc.row > keep_last {
                ui::Control::from_id(tc.canvas.id()).remove();
                false
            } else {
                true
            }
        });

        while self.tile_canvases.len() > MAX_TILE_CANVASES {
            let vp_center_row = ((scroll_y + viewport_h as i32 / 2).max(0) as u32) / TILE_HEIGHT;
            let farthest_idx = self.tile_canvases.iter().enumerate()
                .max_by_key(|(_, tc)| {
                    if tc.row > vp_center_row { tc.row - vp_center_row }
                    else { vp_center_row - tc.row }
                })
                .map(|(i, _)| i)
                .unwrap_or(0);
            let tc = self.tile_canvases.swap_remove(farthest_idx);
            ui::Control::from_id(tc.canvas.id()).remove();
        }

        pending
    }

    // ─────────────────────────────────────────────────────────────────────
    // Internal helpers
    // ─────────────────────────────────────────────────────────────────────

    /// Rasterize a tile using the display list (binary search + linear scan).
    fn rasterize_tile_dl(
        &mut self,
        images: &ImageCache,
        doc_w: u32,
        row: u32,
        doc_h: u32,
        clear_color: u32,
    ) -> Vec<u32> {
        let tile_y_start = (row * TILE_HEIGHT) as i32;
        let tile_y_end = (tile_y_start + TILE_HEIGHT as i32).min(doc_h as i32);

        let pixel_count = (doc_w as usize) * (TILE_HEIGHT as usize);
        let mut buf = self.tile_cache.take_buf(pixel_count, clear_color);

        self.display_list.rasterize_tile(
            images,
            buf.as_mut_ptr(), doc_w, TILE_HEIGHT,
            tile_y_start, tile_y_end,
        );

        buf
    }

    fn create_tile_canvas(&mut self, row: u32, doc_w: u32, doc_h: u32, parent: &ui::View) {
        let pixels = match self.tile_cache.get(row) {
            Some(px) => px,
            None => return,
        };

        let tile_y = (row * TILE_HEIGHT) as i32;
        let tile_h = TILE_HEIGHT.min(doc_h.saturating_sub(row * TILE_HEIGHT)).max(1);

        let c = ui::Canvas::new(doc_w, tile_h);
        c.set_position(0, tile_y);
        c.set_size(doc_w, tile_h);
        if let Some(cb) = self.link_cb {
            c.on_click_raw(cb, self.link_cb_ud);
        }
        parent.add(&c);
        c.copy_pixels_from(pixels);

        self.tile_canvases.push(TileCanvas { row, canvas: c });
    }

    // ─────────────────────────────────────────────────────────────────────
    // Walk: form controls + hit regions (unchanged)
    // ─────────────────────────────────────────────────────────────────────

    fn walk_controls(
        &mut self,
        bx: &LayoutBox,
        offset_x: i32,
        offset_y: i32,
        parent: &ui::View,
        submit_cb: Option<ui::Callback>,
        submit_cb_ud: u64,
    ) {
        if bx.visibility_hidden {
            return;
        }

        let (abs_x, abs_y) = if bx.is_fixed {
            (bx.x, bx.y)
        } else {
            (offset_x + bx.x, offset_y + bx.y)
        };

        if let Some(ref text) = bx.text {
            if !text.is_empty() && bx.form_field.is_none() {
                if let Some(ref url) = bx.link_url {
                    self.hit_regions.push(HitRegion {
                        x: abs_x, y: abs_y,
                        w: bx.width, h: bx.height,
                        kind: HitKind::Link(url.clone()),
                    });
                }
            }
        }

        if let Some(kind) = bx.form_field {
            self.emit_form_control(kind, bx, abs_x, abs_y, parent, submit_cb, submit_cb_ud);
        }

        for child in &bx.children {
            self.walk_controls(child, abs_x, abs_y, parent, submit_cb, submit_cb_ud);
        }
    }

    fn emit_form_control(
        &mut self,
        kind: FormFieldKind,
        bx: &LayoutBox,
        x: i32,
        y: i32,
        parent: &ui::View,
        _submit_cb: Option<ui::Callback>,
        _submit_cb_ud: u64,
    ) {
        let node_id = bx.node_id.unwrap_or(0);

        match kind {
            FormFieldKind::TextInput | FormFieldKind::Password => {
                if let Some(fc) = self.form_controls.iter_mut().find(|fc| fc.node_id == node_id && fc.kind == kind) {
                    let ctrl = ui::Control::from_id(fc.control_id);
                    ctrl.set_position(x, y);
                    ctrl.set_size(bx.width as u32, bx.height as u32);
                    let bg = if bx.bg_color != 0 { bx.bg_color } else { 0xFFFFFFFF };
                    let fg = if bx.color != 0 { bx.color } else { 0xFF000000 };
                    ctrl.set_color(bg);
                    ctrl.set_text_color(fg);
                    fc.seen = true;
                } else {
                    let tf = ui::TextField::new();
                    if kind == FormFieldKind::Password {
                        tf.set_password_mode(true);
                    }
                    tf.set_position(x, y);
                    tf.set_size(bx.width as u32, bx.height as u32);
                    let bg = if bx.bg_color != 0 { bx.bg_color } else { 0xFFFFFFFF };
                    let fg = if bx.color != 0 { bx.color } else { 0xFF000000 };
                    tf.set_color(bg);
                    tf.set_text_color(fg);
                    if let Some(ref ph) = bx.form_placeholder {
                        tf.set_placeholder(ph);
                    }
                    if let Some(ref val) = bx.form_value {
                        tf.set_text(val);
                    }
                    parent.add(&tf);
                    let id = tf.id();
                    self.form_controls.push(FormControl {
                        control_id: id, node_id, kind,
                        name: String::new(), seen: true,
                    });
                }
            }

            FormFieldKind::Submit | FormFieldKind::ButtonEl => {
                self.hit_regions.push(HitRegion {
                    x, y, w: bx.width, h: bx.height,
                    kind: HitKind::Submit(node_id),
                });
            }

            FormFieldKind::Checkbox => {
                if let Some(fc) = self.form_controls.iter_mut().find(|fc| fc.node_id == node_id && fc.kind == kind) {
                    let ctrl = ui::Control::from_id(fc.control_id);
                    ctrl.set_position(x, y);
                    ctrl.set_size(bx.width as u32, bx.height as u32);
                    fc.seen = true;
                } else {
                    let cb = ui::Checkbox::new("");
                    cb.set_position(x, y);
                    cb.set_size(bx.width as u32, bx.height as u32);
                    parent.add(&cb);
                    let id = cb.id();
                    self.form_controls.push(FormControl {
                        control_id: id, node_id, kind,
                        name: String::new(), seen: true,
                    });
                }
            }

            FormFieldKind::Radio => {
                if let Some(fc) = self.form_controls.iter_mut().find(|fc| fc.node_id == node_id && fc.kind == kind) {
                    let ctrl = ui::Control::from_id(fc.control_id);
                    ctrl.set_position(x, y);
                    ctrl.set_size(bx.width as u32, bx.height as u32);
                    fc.seen = true;
                } else {
                    let rb = ui::RadioButton::new("");
                    rb.set_position(x, y);
                    rb.set_size(bx.width as u32, bx.height as u32);
                    parent.add(&rb);
                    let id = rb.id();
                    self.form_controls.push(FormControl {
                        control_id: id, node_id, kind,
                        name: String::new(), seen: true,
                    });
                }
            }

            FormFieldKind::Textarea => {
                if let Some(fc) = self.form_controls.iter_mut().find(|fc| fc.node_id == node_id && fc.kind == kind) {
                    let ctrl = ui::Control::from_id(fc.control_id);
                    ctrl.set_position(x, y);
                    ctrl.set_size(bx.width as u32, bx.height as u32);
                    fc.seen = true;
                } else {
                    let ta = ui::TextArea::new();
                    ta.set_position(x, y);
                    ta.set_size(bx.width as u32, bx.height as u32);
                    ta.set_color(0xFFFFFFFF);
                    ta.set_text_color(0xFF000000);
                    parent.add(&ta);
                    let id = ta.id();
                    self.form_controls.push(FormControl {
                        control_id: id, node_id, kind,
                        name: String::new(), seen: true,
                    });
                }
            }

            FormFieldKind::Hidden => {
                if !self.form_controls.iter().any(|fc| fc.node_id == node_id && fc.kind == kind) {
                    self.form_controls.push(FormControl {
                        control_id: 0, node_id, kind,
                        name: String::new(), seen: true,
                    });
                } else {
                    if let Some(fc) = self.form_controls.iter_mut().find(|fc| fc.node_id == node_id && fc.kind == kind) {
                        fc.seen = true;
                    }
                }
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Buffer drawing helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Fill a rectangle directly in the ARGB pixel buffer with clipping.
fn fill_rect_buf(buf: *mut u32, stride: u32, buf_h: u32, x: i32, y: i32, w: i32, h: i32, color: u32) {
    if w <= 0 || h <= 0 || buf.is_null() { return; }
    let s = stride as i32;
    let bh = buf_h as i32;

    let x0 = x.max(0);
    let y0 = y.max(0);
    let x1 = (x + w).min(s);
    let y1 = (y + h).min(bh);
    if x0 >= x1 || y0 >= y1 { return; }

    let cw = (x1 - x0) as usize;
    let alpha = (color >> 24) & 0xFF;
    unsafe {
        for row in y0..y1 {
            let offset = row as usize * stride as usize + x0 as usize;
            let ptr = buf.add(offset);
            if alpha >= 255 {
                // Fast path: 4-pixel unrolled opaque fill.
                let mut i = 0usize;
                let cw4 = cw & !3;
                while i < cw4 {
                    *ptr.add(i) = color;
                    *ptr.add(i + 1) = color;
                    *ptr.add(i + 2) = color;
                    *ptr.add(i + 3) = color;
                    i += 4;
                }
                while i < cw {
                    *ptr.add(i) = color;
                    i += 1;
                }
            } else if alpha > 0 {
                let inv_a = 255 - alpha;
                let sr = (color >> 16) & 0xFF;
                let sg = (color >> 8) & 0xFF;
                let sb = color & 0xFF;
                for i in 0..cw {
                    let dst = *ptr.add(i);
                    let dr = (dst >> 16) & 0xFF;
                    let dg = (dst >> 8) & 0xFF;
                    let db = dst & 0xFF;
                    let r = (sr * alpha + dr * inv_a) / 255;
                    let g = (sg * alpha + dg * inv_a) / 255;
                    let b = (sb * alpha + db * inv_a) / 255;
                    *ptr.add(i) = 0xFF000000 | (r << 16) | (g << 8) | b;
                }
            }
        }
    }
}

/// Blit image pixels into the buffer with scaling and clipping.
fn blit_image_buf(
    buf: *mut u32, stride: u32, buf_h: u32,
    dx: i32, dy: i32, dw: i32, dh: i32,
    src: &[u32], src_w: u32, src_h: u32,
) {
    if dw <= 0 || dh <= 0 || src.is_empty() || src_w == 0 || src_h == 0 || buf.is_null() {
        return;
    }
    let s = stride as i32;
    let bh = buf_h as i32;

    let x0 = dx.max(0);
    let y0 = dy.max(0);
    let x1 = (dx + dw).min(s);
    let y1 = (dy + dh).min(bh);
    if x0 >= x1 || y0 >= y1 { return; }

    unsafe {
        for row in y0..y1 {
            let sy = ((row - dy) as u64 * src_h as u64 / dh as u64) as usize;
            if sy >= src_h as usize { continue; }
            let dst_offset = row as usize * stride as usize;
            let src_row = sy * src_w as usize;
            for col in x0..x1 {
                let sx = ((col - dx) as u64 * src_w as u64 / dw as u64) as usize;
                if sx >= src_w as usize { continue; }
                let src_idx = src_row + sx;
                if src_idx >= src.len() { continue; }
                let pixel = src[src_idx];
                let alpha = (pixel >> 24) & 0xFF;
                let dst_idx = dst_offset + col as usize;
                if alpha >= 255 {
                    *buf.add(dst_idx) = pixel;
                } else if alpha > 0 {
                    let dst = *buf.add(dst_idx);
                    let inv_a = 255 - alpha;
                    let r = (((pixel >> 16) & 0xFF) * alpha + ((dst >> 16) & 0xFF) * inv_a) / 255;
                    let g = (((pixel >> 8) & 0xFF) * alpha + ((dst >> 8) & 0xFF) * inv_a) / 255;
                    let b = ((pixel & 0xFF) * alpha + (dst & 0xFF) * inv_a) / 255;
                    *buf.add(dst_idx) = 0xFF000000 | (r << 16) | (g << 8) | b;
                }
            }
        }
    }
}
