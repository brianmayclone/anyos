//! Per-tile canvas renderer with compositor-driven smooth scrolling.
//!
//! Static content (text, backgrounds, borders, images) is drawn into cached
//! tile strips (doc_width × 256px).  Each tile is a separate Canvas control
//! positioned at its document Y coordinate inside the content_view.  The
//! compositor's ScrollView handles smooth pixel-level scrolling natively —
//! zero per-frame work from the application.  Only new tiles entering the
//! pre-render zone are rasterized and created (~900 KB each).

use alloc::string::String;
use alloc::vec::Vec;

use libanyui_client::{self as ui, Widget};

use crate::layout::{LayoutBox, FormFieldKind};
use crate::style::TextDeco;

// ═══════════════════════════════════════════════════════════════════════════
// Image cache
// ═══════════════════════════════════════════════════════════════════════════

/// Maximum total decoded image bytes in the cache (128 MiB).
/// With the mmap-backed allocator providing access to the full 512 MiB mmap
/// region, we can afford a generous image cache.  LRU eviction ensures the
/// cache stays within budget even on image-heavy pages.
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
///
/// When inserting a new image would exceed `IMAGE_CACHE_MAX_BYTES`,
/// the least-recently-used entries are evicted until there is room.
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

    /// Read-only lookup (no LRU bump).  Used by the pixel walk where we
    /// cannot hold a `&mut ImageCache`.
    pub fn get_ref(&self, src: &str) -> Option<&ImageEntry> {
        self.entries.iter().find(|e| e.src == src)
    }

    /// Add a decoded image.  Evicts LRU entries if the cache exceeds the byte cap.
    pub fn add(&mut self, src: String, pixels: Vec<u32>, width: u32, height: u32) {
        let new_bytes = pixels.len() * 4;

        // Replace existing entry for the same URL.
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

    /// Evict LRU entries until total_bytes ≤ IMAGE_CACHE_MAX_BYTES.
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
// Hit regions (for link/submit click handling on the canvas)
// ═══════════════════════════════════════════════════════════════════════════

/// A clickable region on the canvas.
///
/// Coordinates are in **absolute document space** (not canvas-local).
pub struct HitRegion {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub kind: HitKind,
}

/// The kind of a clickable hit region.
pub enum HitKind {
    /// A hyperlink with URL.
    Link(String),
    /// A form submit button with DOM node_id.
    Submit(usize),
}

// ═══════════════════════════════════════════════════════════════════════════
// Persistent form controls
// ═══════════════════════════════════════════════════════════════════════════

/// A persistent form control — created once, updated on relayout.
pub struct FormControl {
    /// The libanyui control ID.
    pub control_id: u32,
    /// The DOM node ID of the form element.
    pub node_id: usize,
    /// The form field kind.
    pub kind: FormFieldKind,
    /// The input name attribute (for form submission).
    pub name: String,
    /// Whether this control was seen during the current render pass.
    seen: bool,
}

// ═══════════════════════════════════════════════════════════════════════════
// Tile cache (pixel data)
// ═══════════════════════════════════════════════════════════════════════════

/// Tile height in pixels.  Each tile covers `[row * 256, (row+1) * 256)`.
const TILE_HEIGHT: u32 = 256;

/// Maximum number of cached tile pixel buffers.
const MAX_CACHED_TILES: usize = 40;

/// Buffer zone above/below the viewport for pre-rendering (pixels).
const BUFFER_ZONE: i32 = 512;

/// Maximum number of tile canvases to keep alive.
const MAX_TILE_CANVASES: usize = 30;

/// Maximum number of tiles to rasterize per tick (avoids blocking the event loop).
const MAX_TILES_PER_TICK: usize = 2;

/// A cached rasterized tile strip: doc_width × TILE_HEIGHT pixels.
struct CachedTile {
    /// Tile row index (y_start = row * TILE_HEIGHT).
    row: u32,
    /// Pixel data: doc_width × TILE_HEIGHT u32 values.
    pixels: Vec<u32>,
    /// Insertion generation (for LRU eviction — higher = more recent).
    generation: u64,
}

/// LRU tile cache for rasterized tile strips.
struct TileCache {
    tiles: Vec<CachedTile>,
    generation: u64,
}

impl TileCache {
    fn new() -> Self {
        Self { tiles: Vec::new(), generation: 0 }
    }

    /// Look up a cached tile by row index.  Returns the pixel slice or None.
    fn get(&self, row: u32) -> Option<&[u32]> {
        self.tiles.iter()
            .find(|t| t.row == row)
            .map(|t| t.pixels.as_slice())
    }

    /// Insert a rasterized tile into the cache.  Evicts the LRU tile if full.
    fn insert(&mut self, row: u32, pixels: Vec<u32>) {
        self.generation += 1;
        let gen = self.generation;

        // Replace existing tile for this row.
        if let Some(tile) = self.tiles.iter_mut().find(|t| t.row == row) {
            tile.pixels = pixels;
            tile.generation = gen;
            return;
        }

        // Evict LRU if at capacity.
        if self.tiles.len() >= MAX_CACHED_TILES {
            let min_idx = self.tiles.iter().enumerate()
                .min_by_key(|(_, t)| t.generation)
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.tiles.swap_remove(min_idx);
        }

        self.tiles.push(CachedTile { row, pixels, generation: gen });
    }

    /// Invalidate all cached tiles (called on relayout, resize, navigation).
    fn invalidate_all(&mut self) {
        self.tiles.clear();
        self.generation = 0;
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Per-tile canvas (for smooth compositor-driven scrolling)
// ═══════════════════════════════════════════════════════════════════════════

/// A Canvas control linked to a tile row.  Positioned at (0, row * TILE_HEIGHT)
/// in the content_view.  The compositor's ScrollView scrolls the content_view
/// natively, so tile canvases require zero per-frame work during scroll.
struct TileCanvas {
    /// Tile row index.
    row: u32,
    /// The Canvas control.
    canvas: ui::Canvas,
}

// ═══════════════════════════════════════════════════════════════════════════
// Renderer
// ═══════════════════════════════════════════════════════════════════════════

/// Per-tile canvas renderer with compositor-driven smooth scrolling.
///
/// Each tile strip (doc_width × 256px) gets its own Canvas control positioned
/// inside the content_view.  The compositor's ScrollView clips and scrolls
/// natively — zero work from the application during scroll.  Only tiles
/// entering the pre-render zone are rasterized (~900 KB per tile).
pub(crate) struct Renderer {
    /// Per-tile canvases — each tile is a separate Canvas in the content_view.
    tile_canvases: Vec<TileCanvas>,
    /// Tile pixel data cache (survives canvas eviction for fast recreation).
    tile_cache: TileCache,
    /// Current document width (for tile sizing).
    doc_w: u32,
    /// Current document height.
    doc_h: u32,
    /// Clickable regions (links, submit buttons) — absolute document coordinates.
    pub hit_regions: Vec<HitRegion>,
    /// Persistent form controls — only destroyed on full page navigation.
    pub form_controls: Vec<FormControl>,
    /// Compatibility: control_id → link URL (for submit button Labels).
    pub link_map: Vec<(u32, String)>,
    /// Link callback (set on each tile canvas for click handling).
    link_cb: Option<ui::Callback>,
    link_cb_ud: u64,
    /// Last scroll Y that triggered tile management.
    last_scroll_y: i32,
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
        }
    }

    /// Check if a control ID belongs to any tile canvas, and if so return
    /// the mouse position translated to absolute document coordinates.
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

    /// Return the number of form controls currently tracked.
    pub fn control_count(&self) -> usize {
        self.form_controls.len()
    }

    /// Soft clear: reset hit regions and link map, invalidate tile cache,
    /// destroy tile canvases, and mark form controls for GC.
    /// Called on each relayout.
    pub fn clear(&mut self) {
        self.hit_regions.clear();
        self.link_map.clear();
        self.tile_cache.invalidate_all();
        // Destroy all tile canvases (content is stale after relayout).
        for tc in self.tile_canvases.drain(..) {
            ui::Control::from_id(tc.canvas.id()).remove();
        }
        for fc in &mut self.form_controls {
            fc.seen = false;
        }
    }

    /// Hard clear: destroy everything including tile canvases, form controls,
    /// and tile cache.  Called on full page navigation (new URL).
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
    }

    /// Hit-test at absolute document coordinates for a link URL.
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

    /// Hit-test at absolute document coordinates for a submit button.
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

    /// Render the layout tree using per-tile canvases.
    ///
    /// Called after relayout.  Invalidates the tile cache, walks the full
    /// tree for form controls and hit regions, creates tile canvases for
    /// visible rows, and GCs unseen form controls.
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

        // 2. Walk full tree for form controls + hit regions (document coords).
        self.walk_controls(root, 0, 0, parent, submit_cb, submit_cb_ud);

        // 3. Compute visible tile rows.
        let render_y_start = (scroll_y - BUFFER_ZONE).max(0);
        let render_y_end = (scroll_y + viewport_h as i32 + BUFFER_ZONE).min(doc_h as i32);
        let first_row = render_y_start as u32 / TILE_HEIGHT;
        let last_row = if render_y_end > 0 {
            ((render_y_end - 1) as u32) / TILE_HEIGHT
        } else {
            0
        };

        // 4. Rasterize visible tile rows, cache them, and create canvases.
        for row in first_row..=last_row {
            let tile_buf = rasterize_tile(root, images, w, row, doc_h, clear_color);
            self.tile_cache.insert(row, tile_buf);
            self.create_tile_canvas(row, w, doc_h, parent);
        }

        // 5. GC unseen form controls.
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
    // Scroll render (fast path — compositor-driven)
    // ─────────────────────────────────────────────────────────────────────

    /// Ensure tile canvases exist for the visible viewport range.
    ///
    /// The compositor's ScrollView handles smooth pixel-level scrolling.
    /// This method only needs to create canvases for newly visible tile rows
    /// and remove distant ones.  Tiles already in the cache are free to
    /// create (just a ~900 KB `copy_pixels_from`).  Cache-miss tiles are
    /// rasterized incrementally (max 2 per call to avoid blocking the
    /// event loop).
    ///
    /// Returns `true` if there are still pending tiles that need creation.
    pub fn render_scroll(
        &mut self,
        root: &LayoutBox,
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

        // 1. Compute tile rows that should have canvases (viewport + buffer).
        let render_y_start = (scroll_y - BUFFER_ZONE).max(0);
        let render_y_end = (scroll_y + viewport_h as i32 + BUFFER_ZONE).min(doc_h as i32);
        let first_row = render_y_start as u32 / TILE_HEIGHT;
        let last_row = if render_y_end > 0 {
            ((render_y_end - 1) as u32) / TILE_HEIGHT
        } else {
            0
        };

        // 2. Create canvases for new tile rows (limit rasterization to avoid blocking).
        let mut rasterized = 0usize;
        let mut pending = false;
        for row in first_row..=last_row {
            // Skip if canvas already exists.
            if self.tile_canvases.iter().any(|tc| tc.row == row) {
                continue;
            }

            // Rasterize if not in pixel cache.
            if self.tile_cache.get(row).is_none() {
                if rasterized >= MAX_TILES_PER_TICK {
                    pending = true;
                    continue;
                }
                let tile_buf = rasterize_tile(root, images, w, row, doc_h, clear_color);
                self.tile_cache.insert(row, tile_buf);
                rasterized += 1;
            }

            // Create canvas from cached pixel data.
            self.create_tile_canvas(row, w, doc_h, parent);
        }

        // 3. Evict tile canvases that are far from the viewport.
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

        // Also enforce max tile canvases (LRU by distance from viewport center).
        while self.tile_canvases.len() > MAX_TILE_CANVASES {
            let vp_center_row = ((scroll_y + viewport_h as i32 / 2).max(0) as u32) / TILE_HEIGHT;
            let farthest_idx = self.tile_canvases.iter().enumerate()
                .max_by_key(|(_, tc)| {
                    if tc.row > vp_center_row {
                        tc.row - vp_center_row
                    } else {
                        vp_center_row - tc.row
                    }
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

    /// Create a Canvas control for a tile row from cached pixel data.
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
    // Walk: form controls + hit regions (full tree, no pixels)
    // ─────────────────────────────────────────────────────────────────────

    /// Walk the full layout tree for form controls and hit regions.
    ///
    /// Form controls are created/updated at absolute document coordinates.
    /// Hit regions are registered in absolute document coordinates.
    /// No pixel drawing — that happens in `rasterize_tile()`.
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

        // Register link hit regions (absolute document coordinates).
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

        // Form controls.
        if let Some(kind) = bx.form_field {
            self.emit_form_control(kind, bx, abs_x, abs_y, parent, submit_cb, submit_cb_ud);
        }

        // Recurse into children.
        for child in &bx.children {
            self.walk_controls(child, abs_x, abs_y, parent, submit_cb, submit_cb_ud);
        }
    }

    /// Create or update a persistent form control, or register a submit hit region.
    ///
    /// - `x`, `y`: absolute document coordinates.
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
                // Register submit hit region (absolute document coords).
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
// Free functions: tile rasterization, pixel helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Rasterize a single tile row (pixel-only, no form controls or hit regions).
///
/// Allocates a `doc_w × TILE_HEIGHT` buffer, walks the layout tree with
/// culling to the tile's Y range, and returns the pixel buffer for caching.
fn rasterize_tile(
    root: &LayoutBox,
    images: &ImageCache,
    doc_w: u32,
    row: u32,
    doc_h: u32,
    clear_color: u32,
) -> Vec<u32> {
    let tile_y_start = (row * TILE_HEIGHT) as i32;
    let tile_y_end = (tile_y_start + TILE_HEIGHT as i32).min(doc_h as i32);
    let tile_h = ((tile_y_end - tile_y_start) as u32).max(1);

    let pixel_count = (doc_w as usize) * (TILE_HEIGHT as usize);
    let mut buf = Vec::with_capacity(pixel_count);
    buf.resize(pixel_count, clear_color);

    walk_pixels(
        root, buf.as_mut_ptr(), doc_w, TILE_HEIGHT,
        images, 0, 0, tile_y_start, tile_y_start + tile_h as i32,
    );

    buf
}

/// Pixel-only tree walk — draws backgrounds, borders, text, images, and
/// submit button appearances into the tile buffer.
///
/// Skips form controls and hit regions (handled by `walk_controls()`).
fn walk_pixels(
    bx: &LayoutBox,
    buf: *mut u32,
    stride: u32,
    buf_h: u32,
    images: &ImageCache,
    offset_x: i32,
    offset_y: i32,
    tile_y_start: i32,
    tile_y_end: i32,
) {
    if bx.visibility_hidden {
        return;
    }

    let abs_x = if bx.is_fixed { bx.x } else { offset_x + bx.x };
    let abs_y = if bx.is_fixed { bx.y } else { offset_y + bx.y };

    // Cull boxes entirely outside the tile.
    let in_tile = abs_y + bx.height > tile_y_start && abs_y < tile_y_end;

    // Translate Y to tile-local coordinates.
    let draw_y = abs_y - tile_y_start;

    if in_tile {
        // Background.
        if bx.bg_color != 0 && bx.bg_color != 0x00000000 {
            fill_rect_buf(buf, stride, buf_h, abs_x, draw_y, bx.width, bx.height, bx.bg_color);
        }

        // Border (4 edges).
        if bx.border_width > 0 && bx.border_color != 0 && bx.border_color != 0x00000000 {
            let bw = bx.border_width;
            let w = bx.width;
            let h = bx.height;
            fill_rect_buf(buf, stride, buf_h, abs_x, draw_y, w, bw, bx.border_color);
            fill_rect_buf(buf, stride, buf_h, abs_x, draw_y + h - bw, w, bw, bx.border_color);
            let inner_h = (h - bw * 2).max(0);
            fill_rect_buf(buf, stride, buf_h, abs_x, draw_y + bw, bw, inner_h, bx.border_color);
            fill_rect_buf(buf, stride, buf_h, abs_x + w - bw, draw_y + bw, bw, inner_h, bx.border_color);
        }

        // Horizontal rule.
        if bx.is_hr {
            fill_rect_buf(buf, stride, buf_h, abs_x, draw_y, bx.width, 1, 0xFF999999);
        }

        // List marker.
        if let Some(ref marker) = bx.list_marker {
            let font_id = 0u32;
            let font_size = bx.font_size.max(1) as u16;
            let color = if bx.color != 0 { bx.color } else { 0xFF000000 };
            libfont_client::draw_string_buf(
                buf, stride, buf_h,
                abs_x - 20, draw_y,
                color, font_id, font_size,
                marker,
            );
        }

        // Text fragment.
        if let Some(ref text) = bx.text {
            if !text.is_empty() && bx.form_field.is_none() {
                let font_id = if bx.bold && bx.italic {
                    1u32
                } else if bx.bold {
                    1u32
                } else if bx.italic {
                    3u32
                } else {
                    0u32
                };
                let font_size = bx.font_size.max(1) as u16;
                let color = if bx.color != 0 { bx.color } else { 0xFF000000 };

                libfont_client::draw_string_buf(
                    buf, stride, buf_h,
                    abs_x, draw_y,
                    color, font_id, font_size,
                    text,
                );

                // Underline for links or text-decoration.
                if bx.text_decoration == TextDeco::Underline || bx.link_url.is_some() {
                    fill_rect_buf(buf, stride, buf_h,
                        abs_x, draw_y + bx.height - 1,
                        bx.width, 1, color);
                }

                // Line-through.
                if bx.text_decoration == TextDeco::LineThrough {
                    fill_rect_buf(buf, stride, buf_h,
                        abs_x, draw_y + bx.height / 2,
                        bx.width, 1, color);
                }
            }
        }

        // Image.
        if let Some(ref src) = bx.image_src {
            if let Some(entry) = images.get_ref(src) {
                let dw = bx.image_width.unwrap_or(bx.width);
                let dh = bx.image_height.unwrap_or(bx.height);
                blit_image_buf(
                    buf, stride, buf_h,
                    abs_x, draw_y, dw, dh,
                    &entry.pixels, entry.width, entry.height,
                );
            }
        }

        // Submit/button pixel drawing (hit region is in walk_controls).
        if let Some(kind) = bx.form_field {
            if matches!(kind, FormFieldKind::Submit | FormFieldKind::ButtonEl) {
                draw_submit_pixels(buf, stride, buf_h, abs_x, draw_y, bx);
            }
        }
    }

    // Recurse into children.
    for child in &bx.children {
        let (cx, cy) = if bx.is_fixed { (bx.x, bx.y) } else { (abs_x, abs_y) };
        walk_pixels(child, buf, stride, buf_h, images, cx, cy, tile_y_start, tile_y_end);
    }
}

/// Draw submit button appearance into the pixel buffer.
fn draw_submit_pixels(buf: *mut u32, stride: u32, buf_h: u32, x: i32, y: i32, bx: &LayoutBox) {
    let label_text = if let Some(ref t) = bx.text { t.as_str() } else { "Submit" };

    // Default web button bg + border if no CSS styling.
    if bx.bg_color == 0 && bx.border_width == 0 {
        fill_rect_buf(buf, stride, buf_h, x, y, bx.width, bx.height, 0xFFE0E0E0);
        fill_rect_buf(buf, stride, buf_h, x, y, bx.width, 1, 0xFF808080);
        fill_rect_buf(buf, stride, buf_h, x, y + bx.height - 1, bx.width, 1, 0xFF808080);
        fill_rect_buf(buf, stride, buf_h, x, y + 1, 1, (bx.height - 2).max(0), 0xFF808080);
        fill_rect_buf(buf, stride, buf_h, x + bx.width - 1, y + 1, 1, (bx.height - 2).max(0), 0xFF808080);
    }

    // Center text in button.
    let font_size = bx.font_size.max(1) as u16;
    let text_color = if bx.color != 0 { bx.color } else { 0xFF000000 };
    let (tw, _) = libfont_client::measure(0, font_size, label_text);
    let tx = x + (bx.width - tw as i32) / 2;
    let ty = y + (bx.height - font_size as i32) / 2;
    libfont_client::draw_string_buf(buf, stride, buf_h, tx, ty, text_color, 0, font_size, label_text);
}

// ═══════════════════════════════════════════════════════════════════════════
// Buffer drawing helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Fill a rectangle directly in the ARGB pixel buffer with clipping.
fn fill_rect_buf(buf: *mut u32, stride: u32, buf_h: u32, x: i32, y: i32, w: i32, h: i32, color: u32) {
    if w <= 0 || h <= 0 || buf.is_null() { return; }
    let s = stride as i32;
    let bh = buf_h as i32;

    // Clip to buffer bounds.
    let x0 = x.max(0);
    let y0 = y.max(0);
    let x1 = (x + w).min(s);
    let y1 = (y + h).min(bh);
    if x0 >= x1 || y0 >= y1 { return; }

    let cw = (x1 - x0) as usize;
    unsafe {
        for row in y0..y1 {
            let offset = row as usize * stride as usize + x0 as usize;
            let ptr = buf.add(offset);
            let alpha = (color >> 24) & 0xFF;
            if alpha >= 255 {
                for i in 0..cw {
                    *ptr.add(i) = color;
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
