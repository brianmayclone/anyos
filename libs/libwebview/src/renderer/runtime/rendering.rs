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
        for tc in self.tile_canvases.drain(..) {
            ui::Control::from_id(tc.canvas.id()).remove();
        }

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
        scrolling: bool,
        _link_cb: Option<ui::Callback>,
        _link_cb_ud: u64,
    ) -> bool {
        let w = doc_w.max(1);
        let clear_color = if bg_color != 0 { bg_color } else { 0xFFFFFFFF };

        self.doc_w = w;
        self.doc_h = doc_h;
        self.last_scroll_y = scroll_y;

        let buffer_zone = if scrolling { 0 } else { BUFFER_ZONE };
        let render_y_start = (scroll_y - buffer_zone).max(0);
        let render_y_end = (scroll_y + viewport_h as i32 + buffer_zone).min(doc_h as i32);
        let first_row = render_y_start as u32 / TILE_HEIGHT;
        let last_row = if render_y_end > 0 {
            ((render_y_end - 1) as u32) / TILE_HEIGHT
        } else {
            0
        };
        let prioritized_rows =
            Self::prioritized_tile_rows(first_row, last_row, scroll_y, viewport_h);

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
            if needs_band_expand {
                crate::debug_surf!(
                    "[render] expanding visible display list for scroll range [{}..{})",
                    band_y_start,
                    band_y_end
                );
                self.hit_regions.clear();
                self.link_map.clear();
                if !scrolling {
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
                }
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
        let max_tiles = if scrolling {
            MAX_TILES_PER_SCROLL_TICK
        } else {
            MAX_TILES_PER_IDLE_TICK
        };
        for row in prioritized_rows {
            if self.tile_canvases.iter().any(|tc| tc.row == row) {
                continue;
            }

            if self.tile_cache.get(row).is_none() {
                if rasterized >= max_tiles {
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

}
