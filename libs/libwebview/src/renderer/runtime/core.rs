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
            controls_walk_pending: false,
            headless: false,
        }
    }

    /// Switch this renderer into headless mode (see `Renderer::headless`).
    pub fn set_headless(&mut self) {
        self.headless = true;
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
            if tc.active && tc.canvas.id() == ctrl_id {
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

    pub fn tile_canvas_ids(&self) -> Vec<u32> {
        self.tile_canvases
            .iter()
            .filter(|tc| tc.active)
            .map(|tc| tc.canvas.id())
            .collect()
    }

    /// Soft clear: reset hit regions, invalidate tile cache, destroy canvases.
    pub fn clear(&mut self) {
        self.hit_regions.clear();
        self.link_map.clear();
        self.tile_cache.invalidate_all();
        self.deactivate_all_tile_canvases();
        for fc in &mut self.form_controls {
            fc.seen = false;
        }
        self.display_list.clear();
        self.display_list_complete = true;
        self.display_list_y_range = None;
        self.controls_walk_pending = false;
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
        self.controls_walk_pending = false;
    }
}
