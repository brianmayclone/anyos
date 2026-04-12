//! Display-list renderer with compositor-driven smooth scrolling.
//!
//! After layout, the tree is flattened into a sorted `Vec<DrawCmd>` (the
//! display list).  Each tile is rasterized by binary-searching for the
//! first command that overlaps the tile Y range, then linearly executing
//! commands until they fall below the tile.  This is O(k) per tile where
//! k = commands visible in the tile, compared to O(n) for a full tree walk.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use libanyui_client::{self as ui, Widget};

use crate::layout::{FormFieldKind, LayoutBox};
use crate::style::{
    BackgroundClipVal, BackgroundImageVal, BackgroundRepeatVal, BackgroundSizeVal, TextDeco,
};
mod cache;
mod display_list;
mod forms;
mod raster;
mod raster_utils;
mod tile;
mod types;

pub use raster::parse_color_value;
pub use cache::{ImageCache, ImageEntry, PROGRESSIVE_BAND_VIEWPORTS_AFTER, PROGRESSIVE_BAND_VIEWPORTS_BEFORE};
pub use types::{FormControl, HitKind};
use raster::{parse_date_value, parse_time_value, rasterize_draw_cmd, rasterize_masked_cmd};
use raster_utils::{
    alpha_blend, cos_approx, darken_color, interpolate_gradient_color, lighten_color,
    resolve_axis_origin, sin_approx,
};
use tile::{BUFFER_ZONE, INITIAL_VISIBLE_EXTRA_ROWS, MAX_TILE_CANVASES, MAX_TILES_PER_TICK, TILE_HEIGHT, TileCache, TileCanvas};
use types::{DrawCmd, DrawKind, DrawRotation, DisplayList, HitRegion, MaskLayer, StickyContext};

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
    submit_cb: Option<ui::Callback>,
    submit_cb_ud: u64,
    last_scroll_y: i32,
    /// The display list — built once after layout, used for all tile rasterization.
    display_list: DisplayList,
    display_list_complete: bool,
    display_list_y_range: Option<(i32, i32)>,
}


impl Renderer {
    fn default_accent_color(&self) -> u32 { 0xFF0A84FF }

    fn effective_accent_color(&self, bx: &LayoutBox) -> u32 {
        if bx.accent_color != 0 {
            bx.accent_color
        } else {
            self.default_accent_color()
        }
    }

    fn default_control_bg(&self, bx: &LayoutBox) -> u32 {
        if bx.bg_color != 0 {
            bx.bg_color
        } else if bx.uses_dark_color_scheme {
            0xFF1E1E1E
        } else {
            0xFFFFFFFF
        }
    }

    fn default_control_fg(&self, bx: &LayoutBox) -> u32 {
        if bx.color != 0 {
            bx.color
        } else if bx.uses_dark_color_scheme {
            0xFFF5F5F5
        } else {
            0xFF000000
        }
    }

    fn progressive_band_range(scroll_y: i32, viewport_h: u32, doc_h: u32) -> (i32, i32) {
        let vp = viewport_h.max(1) as i32;
        let start = (scroll_y - vp * PROGRESSIVE_BAND_VIEWPORTS_BEFORE).max(0);
        let end = (scroll_y + vp * PROGRESSIVE_BAND_VIEWPORTS_AFTER).min(doc_h as i32).max(start + 1);
        (start, end)
    }

    fn prioritized_tile_rows(
        first_row: u32,
        last_row: u32,
        scroll_y: i32,
        viewport_h: u32,
    ) -> Vec<u32> {
        if last_row < first_row {
            return Vec::new();
        }
        let mut rows: Vec<u32> = (first_row..=last_row).collect();
        let vp_center_row = ((scroll_y + viewport_h as i32 / 2).max(0) as u32) / TILE_HEIGHT;
        rows.sort_by_key(|row| {
            if *row > vp_center_row {
                *row - vp_center_row
            } else {
                vp_center_row - *row
            }
        });
        rows
    }

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
            submit_cb: None,
            submit_cb_ud: 0,
            last_scroll_y: 0,
            display_list: DisplayList::new(),
            display_list_complete: true,
            display_list_y_range: None,
        }
    }

    fn visible_tile_row_range(scroll_y: i32, viewport_h: u32, doc_h: u32) -> (u32, u32) {
        let visible_y_start = scroll_y.max(0);
        let visible_y_end = (scroll_y + viewport_h as i32).min(doc_h as i32);
        let first_row = visible_y_start as u32 / TILE_HEIGHT;
        let last_visible_row = if visible_y_end > 0 {
            ((visible_y_end - 1) as u32) / TILE_HEIGHT
        } else {
            first_row
        };
        let start = first_row.saturating_sub(INITIAL_VISIBLE_EXTRA_ROWS);
        let max_row = if doc_h > 0 {
            (doc_h - 1) / TILE_HEIGHT
        } else {
            0
        };
        let end = (last_visible_row + INITIAL_VISIBLE_EXTRA_ROWS).min(max_row);
        (start, end)
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
        self.display_list_complete = true;
        self.display_list_y_range = None;
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
        self.submit_cb = None;
        self.submit_cb_ud = 0;
        self.last_scroll_y = 0;
        self.display_list.clear();
        self.display_list_complete = true;
        self.display_list_y_range = None;
    }

    pub fn hit_test_link_at(&self, x: i32, doc_y: i32) -> Option<&str> {
        for region in &self.hit_regions {
            if x >= region.x
                && x < region.x + region.w
                && doc_y >= region.y
                && doc_y < region.y + region.h
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
            if x >= region.x
                && x < region.x + region.w
                && doc_y >= region.y
                && doc_y < region.y + region.h
            {
                if let HitKind::Submit(node_id) = region.kind {
                    return Some(node_id);
                }
            }
        }
        None
    }

    pub fn hit_test_reset_at(&self, x: i32, doc_y: i32) -> Option<usize> {
        for region in &self.hit_regions {
            if x >= region.x
                && x < region.x + region.w
                && doc_y >= region.y
                && doc_y < region.y + region.h
            {
                if let HitKind::Reset(node_id) = region.kind {
                    return Some(node_id);
                }
            }
        }
        None
    }

    pub fn hit_test_checkbox_at(&self, x: i32, doc_y: i32) -> Option<usize> {
        for region in &self.hit_regions {
            if x >= region.x
                && x < region.x + region.w
                && doc_y >= region.y
                && doc_y < region.y + region.h
            {
                if let HitKind::Checkbox(node_id) = region.kind {
                    return Some(node_id);
                }
            }
        }
        None
    }

    pub fn hit_test_select_at(&self, x: i32, doc_y: i32) -> Option<usize> {
        for region in &self.hit_regions {
            if x >= region.x
                && x < region.x + region.w
                && doc_y >= region.y
                && doc_y < region.y + region.h
            {
                if let HitKind::Select(node_id) = region.kind {
                    return Some(node_id);
                }
            }
        }
        None
    }

    pub fn hit_test_radio_at(&self, x: i32, doc_y: i32) -> Option<usize> {
        for region in &self.hit_regions {
            if x >= region.x
                && x < region.x + region.w
                && doc_y >= region.y
                && doc_y < region.y + region.h
            {
                if let HitKind::Radio(node_id) = region.kind {
                    return Some(node_id);
                }
            }
        }
        None
    }

    pub fn hit_test_range_at(&self, x: i32, doc_y: i32) -> Option<usize> {
        for region in &self.hit_regions {
            if x >= region.x
                && x < region.x + region.w
                && doc_y >= region.y
                && doc_y < region.y + region.h
            {
                if let HitKind::Range(node_id) = region.kind {
                    return Some(node_id);
                }
            }
        }
        None
    }

    pub fn hit_test_file_input_at(&self, x: i32, doc_y: i32) -> Option<usize> {
        for region in &self.hit_regions {
            if x >= region.x
                && x < region.x + region.w
                && doc_y >= region.y
                && doc_y < region.y + region.h
            {
                if let HitKind::FileInput(node_id) = region.kind {
                    return Some(node_id);
                }
            }
        }
        None
    }

    pub fn hit_test_color_input_at(&self, x: i32, doc_y: i32) -> Option<usize> {
        for region in &self.hit_regions {
            if x >= region.x
                && x < region.x + region.w
                && doc_y >= region.y
                && doc_y < region.y + region.h
            {
                if let HitKind::ColorInput(node_id) = region.kind {
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
        allow_progressive_display_list: bool,
    ) -> bool {
        crate::debug_surf!(
            "[render] full render start ({}x{}, vp_h={}, scroll_y={})",
            doc_w,
            doc_h,
            viewport_h,
            scroll_y
        );

        let w = doc_w.max(1);
        let clear_color = if bg_color != 0 { bg_color } else { 0xFFFFFFFF };

        self.doc_w = w;
        self.doc_h = doc_h;
        self.link_cb = link_cb;
        self.link_cb_ud = link_cb_ud;
        self.submit_cb = submit_cb;
        self.submit_cb_ud = submit_cb_ud;
        self.last_scroll_y = scroll_y;

        // 1. Invalidate tile cache (layout has changed).
        self.tile_cache.invalidate_all();

        // 4. Compute visible tile rows.
        let render_y_start = (scroll_y - BUFFER_ZONE).max(0);
        let render_y_end = (scroll_y + viewport_h as i32 + BUFFER_ZONE).min(doc_h as i32);
        let first_row = render_y_start as u32 / TILE_HEIGHT;
        let last_row = if render_y_end > 0 {
            ((render_y_end - 1) as u32) / TILE_HEIGHT
        } else {
            0
        };
        let prioritized_rows =
            Self::prioritized_tile_rows(first_row, last_row, scroll_y, viewport_h);
        let (visible_first_row, visible_last_row) =
            Self::visible_tile_row_range(scroll_y, viewport_h, doc_h);
        let immediate_rows =
            Self::prioritized_tile_rows(visible_first_row, visible_last_row, scroll_y, viewport_h);

        let fast_initial_display_list = doc_h > viewport_h.saturating_mul(3)
            || root.subtree_bottom > viewport_h as i32 * 3;
        let (initial_visible_y_start, initial_visible_y_end) =
            Self::progressive_band_range(scroll_y, viewport_h, doc_h);

        // 3.5 For very tall documents, build only the visible display list first.
        // This makes the first styled paint arrive quickly; the full list is
        // materialized later on demand when the user scrolls outside the
        // initial viewport band.
        if fast_initial_display_list {
            self.walk_controls_visible(
                root,
                0,
                0,
                parent,
                self.submit_cb,
                self.submit_cb_ud,
                initial_visible_y_start,
                initial_visible_y_end,
            );
            self.display_list =
                DisplayList::build_visible(root, initial_visible_y_start, initial_visible_y_end);
            self.display_list_complete = false;
            self.display_list_y_range = Some((initial_visible_y_start, initial_visible_y_end));
            crate::debug_surf!(
                "[render] initial visible display list: {} commands in [{}..{})",
                self.display_list.cmds.len(),
                initial_visible_y_start,
                initial_visible_y_end
            );
        } else {
            if !allow_progressive_display_list {
                crate::debug_surf!(
                    "[render] progressive display list disabled for correctness-sensitive initial render"
                );
            }
            // 2. Walk full tree for form controls + hit regions.
            self.walk_controls(root, 0, 0, parent, self.submit_cb, self.submit_cb_ud);

            // 3. Build display list (flat, Y-sorted draw commands).
            self.display_list = DisplayList::build(root);
            self.display_list_complete = true;
            self.display_list_y_range = None;
            crate::debug_surf!(
                "[render] display list: {} commands, max_h={}",
                self.display_list.cmds.len(),
                self.display_list.max_h
            );
        }

        // 5. Rasterize visible tiles using the display list.
        for row in immediate_rows.iter().copied() {
            let tile_buf = self.rasterize_tile_dl(images, w, row, doc_h, clear_color);
            self.tile_cache.insert(row, tile_buf);
            self.create_tile_canvas(row, w, doc_h, parent);
        }

        // 6. Bring form controls in front of tile canvases so they are
        //    visible and interactive (canvases were just added on top).
        for fc in &self.form_controls {
            if fc.control_id != 0 && fc.seen {
                ui::Control::from_id(fc.control_id).bring_to_front();
            }
        }

        // 7. GC unseen form controls.
        self.form_controls.retain(|fc| {
            if !fc.seen && fc.control_id != 0 {
                ui::Control::from_id(fc.control_id).remove();
                false
            } else {
                fc.seen || fc.control_id == 0
            }
        });

        crate::debug_surf!(
            "[render] full render done: {} tile canvases, {} hit_regions, {} form_controls",
            self.tile_canvases.len(),
            self.hit_regions.len(),
            self.form_controls.len()
        );
        !self.display_list_complete || immediate_rows.len() < prioritized_rows.len()
    }

    /// Paint-only refresh path.
    ///
    /// Reuses the existing display list / controls / hit regions and only
    /// invalidates tile pixels for the current viewport. This is the fast path
    /// for late image arrivals and pure paint mutations after layout is already
    /// stable.
    pub fn repaint(
        &mut self,
        root: &LayoutBox,
        parent: &ui::View,
        images: &ImageCache,
        doc_w: u32,
        doc_h: u32,
        viewport_h: u32,
        scroll_y: i32,
        bg_color: u32,
    ) -> bool {
        let w = doc_w.max(1);
        let clear_color = if bg_color != 0 { bg_color } else { 0xFFFFFFFF };

        self.doc_w = w;
        self.doc_h = doc_h;
        self.last_scroll_y = scroll_y;

        self.tile_cache.invalidate_all();
        for tc in self.tile_canvases.drain(..) {
            ui::Control::from_id(tc.canvas.id()).remove();
        }

        let (visible_first_row, visible_last_row) =
            Self::visible_tile_row_range(scroll_y, viewport_h, doc_h);
        let immediate_rows =
            Self::prioritized_tile_rows(visible_first_row, visible_last_row, scroll_y, viewport_h);

        if !self.display_list_complete {
            let (visible_y_start, visible_y_end) =
                Self::progressive_band_range(scroll_y, viewport_h, doc_h);
            let needs_full_list = match self.display_list_y_range {
                Some((built_y_start, built_y_end)) => {
                    visible_y_start < built_y_start || visible_y_end > built_y_end
                }
                None => true,
            };
            if needs_full_list {
                crate::debug_surf!(
                    "[render] repaint expanding visible display list to [{}..{})",
                    visible_y_start,
                    visible_y_end
                );
                self.hit_regions.clear();
                self.link_map.clear();
                for fc in &mut self.form_controls {
                    fc.seen = false;
                }
                self.walk_controls_visible(
                    root,
                    0,
                    0,
                    parent,
                    self.submit_cb,
                    self.submit_cb_ud,
                    visible_y_start,
                    visible_y_end,
                );
                self.display_list =
                    DisplayList::build_visible(root, visible_y_start, visible_y_end);
                self.display_list_complete = false;
                self.display_list_y_range = Some((visible_y_start, visible_y_end));
            }
        }

        for row in immediate_rows.iter().copied() {
            let tile_buf = self.rasterize_tile_dl(images, w, row, doc_h, clear_color);
            self.tile_cache.insert(row, tile_buf);
            self.create_tile_canvas(row, w, doc_h, parent);
        }

        for fc in &self.form_controls {
            if fc.control_id != 0 && fc.seen {
                ui::Control::from_id(fc.control_id).bring_to_front();
            }
        }

        !self.display_list_complete
    }

    // ─────────────────────────────────────────────────────────────────────
    // Scroll render (fast path)
    // ─────────────────────────────────────────────────────────────────────

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

        let render_y_start = (scroll_y - BUFFER_ZONE).max(0);
        let render_y_end = (scroll_y + viewport_h as i32 + BUFFER_ZONE).min(doc_h as i32);
        let first_row = render_y_start as u32 / TILE_HEIGHT;
        let last_row = if render_y_end > 0 {
            ((render_y_end - 1) as u32) / TILE_HEIGHT
        } else {
            0
        };
        let prioritized_rows =
            Self::prioritized_tile_rows(first_row, last_row, scroll_y, viewport_h);

        if !self.display_list_complete {
            let (band_y_start, band_y_end) =
                Self::progressive_band_range(scroll_y, viewport_h, doc_h);
            let needs_band_expand = match self.display_list_y_range {
                Some((built_y_start, built_y_end)) => {
                    band_y_start < built_y_start || band_y_end > built_y_end
                }
                None => true,
            };
            if needs_band_expand {
                crate::debug_surf!(
                    "[render] expanding visible display list for scroll range [{}..{})",
                    band_y_start,
                    band_y_end
                );
                self.hit_regions.clear();
                self.link_map.clear();
                for fc in &mut self.form_controls {
                    fc.seen = false;
                }
                self.walk_controls_visible(
                    root,
                    0,
                    0,
                    parent,
                    self.submit_cb,
                    self.submit_cb_ud,
                    band_y_start,
                    band_y_end,
                );
                self.display_list = DisplayList::build_visible(root, band_y_start, band_y_end);
                self.display_list_complete = false;
                self.display_list_y_range = Some((band_y_start, band_y_end));
                // Keep already rasterized tiles/canvases. Expanding the band only
                // adds more commands outside the previous range; it should not
                // destroy the current viewport and force a full repaint/jank spike.
            }
        }

        let mut rasterized = 0usize;
        let mut pending = false;
        for row in prioritized_rows {
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

        // Bring form controls in front of newly added tile canvases.
        if rasterized > 0 {
            for fc in &self.form_controls {
                if fc.control_id != 0 && fc.seen {
                    ui::Control::from_id(fc.control_id).bring_to_front();
                }
            }
        }

        // Evict distant tile canvases.
        let keep_first = first_row.saturating_sub(4);
        let keep_last = (last_row + 4).min(if doc_h > 0 {
            (doc_h - 1) / TILE_HEIGHT
        } else {
            0
        });
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
            let farthest_idx = self
                .tile_canvases
                .iter()
                .enumerate()
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
            buf.as_mut_ptr(),
            doc_w,
            TILE_HEIGHT,
            tile_y_start,
            tile_y_end,
        );

        buf
    }

    fn create_tile_canvas(&mut self, row: u32, doc_w: u32, doc_h: u32, parent: &ui::View) {
        let pixels = match self.tile_cache.get(row) {
            Some(px) => px,
            None => return,
        };

        let tile_y = (row * TILE_HEIGHT) as i32;
        let tile_h = TILE_HEIGHT
            .min(doc_h.saturating_sub(row * TILE_HEIGHT))
            .max(1);

        let c = ui::Canvas::new(doc_w, tile_h);
        c.set_position(0, tile_y);
        c.set_size(doc_w, tile_h);
        if let Some(cb) = self.link_cb {
            c.on_click_raw(cb, self.link_cb_ud);
            c.on_event_raw(ui::EVENT_MOUSE_MOVE, cb, self.link_cb_ud);
            c.on_event_raw(ui::EVENT_MOUSE_DOWN, cb, self.link_cb_ud);
            c.on_event_raw(ui::EVENT_MOUSE_UP, cb, self.link_cb_ud);
            #[cfg(not(feature = "host"))]
            c.on_event_raw(ui::EVENT_MOUSE_LEAVE, cb, self.link_cb_ud);
        }
        parent.add(&c);
        c.copy_pixels_from(pixels);

        self.tile_canvases.push(TileCanvas { row, canvas: c });
    }


}
