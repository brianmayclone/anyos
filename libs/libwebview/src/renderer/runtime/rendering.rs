impl Renderer {
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
        self.deactivate_all_tile_canvases();

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

        let fast_initial_display_list =
            doc_h > viewport_h.saturating_mul(3) || root.subtree_bottom > viewport_h as i32 * 3;
        let (initial_visible_y_start, initial_visible_y_end) =
            Self::progressive_band_range(scroll_y, viewport_h, doc_h);

        // 3.5 For very tall documents, build only the visible display list first.
        // This makes the first styled paint arrive quickly; the full list is
        // materialized later on demand when the user scrolls outside the
        // initial viewport band.
        if fast_initial_display_list && allow_progressive_display_list {
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
                let (build_y_start, build_y_end) =
                    self.expand_display_list_band(root, doc_h, visible_y_start, visible_y_end);
                self.rebuild_controls_for_range(root, parent, build_y_start, build_y_end);
            } else if self.controls_walk_pending {
                if let Some((built_y_start, built_y_end)) = self.display_list_y_range {
                    self.rebuild_controls_for_range(root, parent, built_y_start, built_y_end);
                }
            }
        } else if self.controls_walk_pending {
            self.rebuild_controls_for_range(root, parent, 0, doc_h.max(1) as i32);
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

    /// Refresh draw commands and tile pixels for a document-space Y range.
    ///
    /// Paint-only JS mutations do not change geometry, so throwing away every
    /// visible tile is unnecessary. Rebuild the display-list band that covers
    /// the changed node(s), invalidate only the overlapping tile rows, and let
    /// the normal viewport tile pump rasterize the rows that are actually
    /// needed on screen.
    pub fn refresh_paint_y_range(&mut self, root: &LayoutBox, y: i32, h: i32) {
        let raw_y0 = y.max(0);
        let raw_y1 = y.saturating_add(h.max(1)).max(raw_y0 + 1);
        let y0 = ((raw_y0 as u32 / TILE_HEIGHT) * TILE_HEIGHT) as i32;
        let mut y1 = ((((raw_y1 as u32).saturating_add(TILE_HEIGHT - 1)) / TILE_HEIGHT)
            * TILE_HEIGHT) as i32;
        if self.doc_h > 0 {
            y1 = y1.min(self.doc_h as i32);
        }
        if y1 <= y0 {
            return;
        }

        if self.display_list_complete {
            // The full list stays authoritative — rebuild it in place so the
            // band bookkeeping (and every offscreen tile outside the mutated
            // rows) survives paint-only JS mutations.
            self.display_list = DisplayList::build(root);
        } else {
            let (built_y_start, built_y_end) = self.display_list_y_range.unwrap_or((y0, y1));
            let build_y_start = built_y_start.min(y0);
            let build_y_end = built_y_end.max(y1);
            self.display_list = DisplayList::build_visible(root, build_y_start, build_y_end);
            self.display_list_y_range = Some((build_y_start, build_y_end));
            if self.doc_h > 0 {
                self.mark_display_list_complete_if_covering(self.doc_h);
            }
        }
        self.invalidate_y_range(y0, y1 - y0);
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
        scrolling: bool,
        _link_cb: Option<ui::Callback>,
        _link_cb_ud: u64,
    ) -> bool {
        let w = doc_w.max(1);
        let clear_color = if bg_color != 0 { bg_color } else { 0xFFFFFFFF };
        let previous_scroll_y = self.last_scroll_y;

        self.doc_w = w;
        self.doc_h = doc_h;
        self.last_scroll_y = scroll_y;

        let (prefetch_above, prefetch_below) = if scrolling {
            let delta = scroll_y - previous_scroll_y;
            let directional = (TILE_HEIGHT as i32 * 2).max(512);
            let trailing = (TILE_HEIGHT as i32 / 2).max(128);
            if delta > 0 {
                (trailing, directional)
            } else if delta < 0 {
                (directional, trailing)
            } else {
                (TILE_HEIGHT as i32, TILE_HEIGHT as i32)
            }
        } else {
            (BUFFER_ZONE, BUFFER_ZONE)
        };
        let render_y_start = (scroll_y - prefetch_above).max(0);
        let render_y_end = (scroll_y + viewport_h as i32 + prefetch_below).min(doc_h as i32);
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
        let mut pending = false;

        if !self.display_list_complete {
            let (band_y_start, band_y_end) = if scrolling {
                let vp = viewport_h.max(1) as i32;
                let start = (scroll_y - vp).max(0);
                let end = (scroll_y + vp * 2).min(doc_h as i32).max(start + 1);
                (start, end)
            } else {
                Self::progressive_band_range(scroll_y, viewport_h, doc_h)
            };
            let needs_band_expand = match self.display_list_y_range {
                Some((built_y_start, built_y_end)) => {
                    band_y_start < built_y_start || band_y_end > built_y_end
                }
                None => true,
            };
            let defer_display_list_expand = needs_band_expand && scrolling;
            let visible_tile_missing = defer_display_list_expand
                && (visible_first_row..=visible_last_row).any(|row| {
                    !self
                        .tile_canvases
                        .iter()
                        .any(|tc| tc.active && tc.row == row)
                        && self.tile_cache.get(row).is_none()
                });
            if defer_display_list_expand && !visible_tile_missing {
                // Building a visible display list still walks a large part of
                // the layout tree. Doing that inside a scroll-input frame is
                // the kind of synchronous work users feel immediately. If all
                // visible tiles are already backed by pixels, leave offscreen
                // expansion for the next scroll/idle opportunity. This is not
                // visible work and must not keep the 16 ms timer alive forever.
            }
            if needs_band_expand && (!defer_display_list_expand || visible_tile_missing) {
                let (target_y_start, target_y_end) = if visible_tile_missing {
                    let start = (scroll_y - TILE_HEIGHT as i32).max(0);
                    let end = (scroll_y + viewport_h as i32 + TILE_HEIGHT as i32).min(doc_h as i32);
                    (start, end.max(start + 1))
                } else {
                    (band_y_start, band_y_end)
                };
                crate::debug_surf!(
                    "[render] expanding visible display list for scroll range [{}..{})",
                    target_y_start,
                    target_y_end
                );
                // The band grows as a union of old and new coverage; tiles
                // rasterized inside the old band stay valid, only the newly
                // covered rows are invalidated. This is what keeps already
                // rendered content (e.g. the top of the page) alive when the
                // user scrolls back up.
                let (build_y_start, build_y_end) =
                    self.expand_display_list_band(root, doc_h, target_y_start, target_y_end);
                if scrolling {
                    // Walking controls/hit regions over the band is sync work
                    // a scroll frame cannot afford; the next idle/repaint pass
                    // rebuilds them for the expanded band.
                    self.controls_walk_pending = true;
                } else {
                    self.rebuild_controls_for_range(root, parent, build_y_start, build_y_end);
                }
            }
        }

        let mut rasterized = 0usize;
        let mut created_canvases = 0usize;
        let visible_row_budget = visible_last_row.saturating_sub(visible_first_row) as usize + 1;
        let max_tiles = if scrolling {
            MAX_TILES_PER_SCROLL_TICK.max(visible_row_budget)
        } else {
            MAX_TILES_PER_IDLE_TICK
        };
        let max_canvas_creates = if scrolling {
            MAX_TILE_CANVAS_CREATES_PER_SCROLL_TICK.max(visible_row_budget)
        } else {
            MAX_TILE_CANVAS_CREATES_PER_IDLE_TICK
        };

        let keep_margin_rows = 16;
        let keep_first = first_row.saturating_sub(keep_margin_rows);
        let keep_last = (last_row + keep_margin_rows).min(if doc_h > 0 {
            (doc_h - 1) / TILE_HEIGHT
        } else {
            0
        });
        self.deactivate_distant_tile_canvases(keep_first, keep_last);

        for row in prioritized_rows {
            let row_is_visible = row >= visible_first_row && row <= visible_last_row;
            if self
                .tile_canvases
                .iter()
                .any(|tc| tc.active && tc.row == row)
            {
                self.tile_cache.touch(row);
                continue;
            }

            if self.tile_cache.get(row).is_none() {
                if !self.row_fully_covered(row, doc_h) {
                    if row_is_visible {
                        pending = true;
                    }
                    continue;
                }
                if rasterized >= max_tiles {
                    if !scrolling || row_is_visible {
                        pending = true;
                    }
                    continue;
                }
                let tile_buf = self.rasterize_tile_dl(images, w, row, doc_h, clear_color);
                self.tile_cache.insert(row, tile_buf);
                rasterized += 1;
            }

            if created_canvases >= max_canvas_creates {
                if !scrolling || row_is_visible {
                    pending = true;
                }
                continue;
            }
            if self.tile_canvases.len() >= MAX_TILE_CANVASES
                && !self.tile_canvases.iter().any(|tc| !tc.active)
            {
                self.deactivate_farthest_active_tile_canvas(scroll_y, viewport_h);
            }
            if self.create_tile_canvas(row, w, doc_h, parent) {
                created_canvases += 1;
            } else {
                if !scrolling || row_is_visible {
                    pending = true;
                }
            }
        }

        // Bring form controls in front of newly added tile canvases.
        if created_canvases > 0 {
            for fc in &self.form_controls {
                if fc.control_id != 0 && fc.seen {
                    ui::Control::from_id(fc.control_id).bring_to_front();
                }
            }
        }

        while self.active_tile_canvas_count() > MAX_TILE_CANVASES {
            if !self.deactivate_farthest_active_tile_canvas(scroll_y, viewport_h) {
                break;
            }
        }

        pending
    }

    /// Opportunistically prepare offscreen rows after the page has gone idle.
    ///
    /// The normal scroll path keeps only a small safety band around the
    /// viewport so input stays responsive. This path uses spare idle frames to
    /// rasterize a wider band into the tile cache and materialize canvases only
    /// for the nearer rows. That gives future scrolls something ready to show
    /// without turning idle time into a long synchronous full-page render.
    pub fn render_idle_prerender(
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
        if doc_h == 0 || viewport_h == 0 {
            return false;
        }

        let w = doc_w.max(1);
        let clear_color = if bg_color != 0 { bg_color } else { 0xFFFFFFFF };
        let vp = viewport_h.max(1) as i32;
        let doc_h_i32 = doc_h as i32;
        let max_row = (doc_h - 1) / TILE_HEIGHT;

        self.doc_w = w;
        self.doc_h = doc_h;
        self.last_scroll_y = scroll_y;

        let render_y_start = (scroll_y - vp * IDLE_PRERENDER_VIEWPORTS_BEFORE).max(0);
        let render_y_end = (scroll_y + vp * (IDLE_PRERENDER_VIEWPORTS_AFTER + 1))
            .min(doc_h_i32)
            .max(render_y_start + 1);
        let first_row = render_y_start as u32 / TILE_HEIGHT;
        let last_row = if render_y_end > 0 {
            (((render_y_end - 1) as u32) / TILE_HEIGHT).min(max_row)
        } else {
            first_row
        };

        let canvas_y_start = (scroll_y - vp * IDLE_PRERENDER_CANVAS_VIEWPORTS_BEFORE).max(0);
        let canvas_y_end = (scroll_y + vp * (IDLE_PRERENDER_CANVAS_VIEWPORTS_AFTER + 1))
            .min(doc_h_i32)
            .max(canvas_y_start + 1);
        let canvas_first_row = canvas_y_start as u32 / TILE_HEIGHT;
        let canvas_last_row = if canvas_y_end > 0 {
            (((canvas_y_end - 1) as u32) / TILE_HEIGHT).min(max_row)
        } else {
            canvas_first_row
        };

        if !self.display_list_complete {
            let needs_band_expand = match self.display_list_y_range {
                Some((built_y_start, built_y_end)) => {
                    render_y_start < built_y_start || render_y_end > built_y_end
                }
                None => true,
            };
            if needs_band_expand {
                crate::debug_surf!(
                    "[render] idle display-list prewarm range [{}..{})",
                    render_y_start,
                    render_y_end
                );
                let (build_y_start, build_y_end) =
                    self.expand_display_list_band(root, doc_h, render_y_start, render_y_end);
                self.rebuild_controls_for_range(root, parent, build_y_start, build_y_end);
            } else if self.controls_walk_pending {
                if let Some((built_y_start, built_y_end)) = self.display_list_y_range {
                    self.rebuild_controls_for_range(root, parent, built_y_start, built_y_end);
                }
            }
        } else if self.controls_walk_pending {
            self.rebuild_controls_for_range(root, parent, 0, doc_h_i32);
        }

        let keep_first = first_row.saturating_sub(32);
        let keep_last = (last_row + 32).min(max_row);
        self.deactivate_distant_tile_canvases(keep_first, keep_last);

        let prioritized_rows =
            Self::prioritized_tile_rows(first_row, last_row, scroll_y, viewport_h);
        let mut rasterized = 0usize;
        let mut created_canvases = 0usize;
        let mut pending = false;
        let mut did_work = false;

        for row in prioritized_rows {
            if !self.row_fully_covered(row, doc_h) {
                pending = true;
                continue;
            }

            if self.tile_cache.get(row).is_none() {
                if rasterized >= MAX_TILES_PER_IDLE_PRERENDER_TICK {
                    pending = true;
                    continue;
                }
                let tile_buf = self.rasterize_tile_dl(images, w, row, doc_h, clear_color);
                self.tile_cache.insert(row, tile_buf);
                rasterized += 1;
                did_work = true;
            } else {
                self.tile_cache.touch(row);
            }

            if row < canvas_first_row || row > canvas_last_row {
                continue;
            }
            if self
                .tile_canvases
                .iter()
                .any(|tc| tc.active && tc.row == row)
            {
                continue;
            }
            if created_canvases >= MAX_TILE_CANVAS_CREATES_PER_IDLE_PRERENDER_TICK {
                pending = true;
                continue;
            }
            if self.tile_canvases.len() >= MAX_TILE_CANVASES
                && !self.tile_canvases.iter().any(|tc| !tc.active)
            {
                self.deactivate_farthest_active_tile_canvas(scroll_y, viewport_h);
            }
            if self.create_tile_canvas(row, w, doc_h, parent) {
                created_canvases += 1;
                did_work = true;
            } else {
                pending = true;
            }
        }

        if created_canvases > 0 {
            for fc in &self.form_controls {
                if fc.control_id != 0 && fc.seen {
                    ui::Control::from_id(fc.control_id).bring_to_front();
                }
            }
        }

        while self.active_tile_canvas_count() > MAX_TILE_CANVASES {
            if !self.deactivate_farthest_active_tile_canvas(scroll_y, viewport_h) {
                break;
            }
        }

        pending || did_work
    }

    // ─────────────────────────────────────────────────────────────────────
    // Internal helpers
    // ─────────────────────────────────────────────────────────────────────

    /// True when the display list covers tile `row` completely, i.e. a tile
    /// rasterized from it cannot contain culled holes at the band edges.
    fn row_fully_covered(&self, row: u32, doc_h: u32) -> bool {
        if self.display_list_complete {
            return true;
        }
        let Some((built_y_start, built_y_end)) = self.display_list_y_range else {
            return false;
        };
        let row_y_start = (row * TILE_HEIGHT) as i32;
        let row_y_end = ((row as u64 + 1) * TILE_HEIGHT as u64).min(doc_h.max(1) as u64) as i32;
        row_y_start >= built_y_start && row_y_end <= built_y_end
    }

    /// Flip `display_list_complete` once the built band covers the document.
    fn mark_display_list_complete_if_covering(&mut self, doc_h: u32) {
        if self.display_list_complete {
            return;
        }
        if let Some((built_y_start, built_y_end)) = self.display_list_y_range {
            if built_y_start <= 0 && built_y_end >= doc_h.max(1) as i32 {
                self.display_list_complete = true;
            }
        }
    }

    /// Rebuild hit regions, link map and form controls for a document Y range.
    fn rebuild_controls_for_range(
        &mut self,
        root: &LayoutBox,
        parent: &ui::View,
        y_start: i32,
        y_end: i32,
    ) {
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
            y_start,
            y_end,
        );
        self.controls_walk_pending = false;
    }

    /// Expand the progressive display-list band to the union of the current
    /// band and `[target_y_start, target_y_end)`, then invalidate only the
    /// newly covered tile rows. Tiles rasterized inside the previous band were
    /// produced from identical draw commands and stay valid.
    fn expand_display_list_band(
        &mut self,
        root: &LayoutBox,
        doc_h: u32,
        target_y_start: i32,
        target_y_end: i32,
    ) -> (i32, i32) {
        let doc_h_i32 = doc_h.max(1) as i32;
        let old_range = self.display_list_y_range;
        let (old_y_start, old_y_end) = old_range.unwrap_or((target_y_start, target_y_start));
        let build_y_start = old_y_start.min(target_y_start).max(0);
        let build_y_end = old_y_end
            .max(target_y_end)
            .min(doc_h_i32)
            .max(build_y_start + 1);
        self.display_list = DisplayList::build_visible(root, build_y_start, build_y_end);
        self.display_list_y_range = Some((build_y_start, build_y_end));
        self.mark_display_list_complete_if_covering(doc_h);
        match old_range {
            None => self.invalidate_y_range(build_y_start, build_y_end - build_y_start),
            Some((prev_y_start, prev_y_end)) => {
                if build_y_start < prev_y_start {
                    self.invalidate_y_range(build_y_start, prev_y_start - build_y_start);
                }
                if build_y_end > prev_y_end {
                    self.invalidate_y_range(prev_y_end, build_y_end - prev_y_end);
                }
            }
        }
        (build_y_start, build_y_end)
    }
}
