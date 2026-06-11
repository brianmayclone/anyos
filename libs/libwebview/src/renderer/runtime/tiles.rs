impl Renderer {
    fn park_inactive_tile_canvas(tc: &mut TileCanvas) {
        tc.canvas.set_position(0, -1_000_000);
        tc.active = false;
    }

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

    fn active_tile_canvas_count(&self) -> usize {
        self.tile_canvases.iter().filter(|tc| tc.active).count()
    }

    fn deactivate_distant_tile_canvases(&mut self, keep_first: u32, keep_last: u32) {
        for tc in &mut self.tile_canvases {
            if tc.active && (tc.row < keep_first || tc.row > keep_last) {
                Self::park_inactive_tile_canvas(tc);
            }
        }
    }

    pub(crate) fn invalidate_y_range(&mut self, y: i32, h: i32) {
        let y0 = y.max(0) as u32;
        let y1 = (y.saturating_add(h.max(1))).max(1) as u32;
        let first_row = y0 / TILE_HEIGHT;
        let last_row = y1.saturating_sub(1) / TILE_HEIGHT;
        for row in first_row..=last_row {
            self.tile_cache.invalidate_row(row);
            for tc in &mut self.tile_canvases {
                if tc.active && tc.row == row {
                    Self::park_inactive_tile_canvas(tc);
                }
            }
        }
    }

    pub(crate) fn copy_viewport_pixels(
        &self,
        fb: &mut [u32],
        width: usize,
        height: usize,
        scroll_y: i32,
        bg_color: u32,
    ) {
        let clear_color = if bg_color != 0 { bg_color } else { 0xFFFFFFFF };
        let pixel_count = width.saturating_mul(height).min(fb.len());
        for px in fb.iter_mut().take(pixel_count) {
            *px = clear_color;
        }
        if width == 0 || height == 0 || self.doc_w == 0 || fb.len() < width {
            return;
        }

        let doc_w = self.doc_w as usize;
        let copy_w = doc_w.min(width);
        if copy_w == 0 {
            return;
        }

        let first_row = scroll_y.max(0) as u32 / TILE_HEIGHT;
        let viewport_bottom = scroll_y.saturating_add(height as i32).max(0);
        let last_row = if viewport_bottom > 0 {
            ((viewport_bottom - 1) as u32) / TILE_HEIGHT
        } else {
            first_row
        };

        for row in first_row..=last_row {
            let Some(tile) = self.tile_cache.get(row) else {
                continue;
            };
            let tile_top = (row * TILE_HEIGHT) as i32 - scroll_y;
            let src_start = if tile_top < 0 {
                (-tile_top) as usize
            } else {
                0
            };
            if src_start >= TILE_HEIGHT as usize {
                continue;
            }
            let dst_start = if tile_top > 0 {
                tile_top as usize
            } else {
                0
            };
            if dst_start >= height {
                continue;
            }

            let rows = (TILE_HEIGHT as usize - src_start).min(height - dst_start);
            for y in 0..rows {
                let src_off = (src_start + y).saturating_mul(doc_w);
                let dst_off = (dst_start + y).saturating_mul(width);
                if src_off.saturating_add(copy_w) > tile.len()
                    || dst_off.saturating_add(copy_w) > fb.len()
                {
                    break;
                }
                fb[dst_off..dst_off + copy_w].copy_from_slice(&tile[src_off..src_off + copy_w]);
            }
        }
    }

    fn deactivate_farthest_active_tile_canvas(&mut self, scroll_y: i32, viewport_h: u32) -> bool {
        let vp_center_row = ((scroll_y + viewport_h as i32 / 2).max(0) as u32) / TILE_HEIGHT;
        let farthest_idx = self
            .tile_canvases
            .iter()
            .enumerate()
            .filter(|(_, tc)| tc.active)
            .max_by_key(|(_, tc)| {
                if tc.row > vp_center_row {
                    tc.row - vp_center_row
                } else {
                    vp_center_row - tc.row
                }
            })
            .map(|(i, _)| i);
        let Some(idx) = farthest_idx else {
            return false;
        };
        Self::park_inactive_tile_canvas(&mut self.tile_canvases[idx]);
        true
    }

    fn create_tile_canvas(&mut self, row: u32, doc_w: u32, doc_h: u32, parent: &ui::View) -> bool {
        if self.headless {
            // Headless rendering keeps pixels in the tile cache only; the
            // caller reads them back via copy_viewport_pixels().
            self.tile_cache.touch(row);
            return self.tile_cache.get(row).is_some();
        }
        self.tile_cache.touch(row);
        let pixels = match self.tile_cache.get(row) {
            Some(px) => px,
            None => return false,
        };

        let tile_y = (row * TILE_HEIGHT) as i32;
        let tile_h = TILE_HEIGHT
            .min(doc_h.saturating_sub(row * TILE_HEIGHT))
            .max(1);

        if let Some(idx) = self
            .tile_canvases
            .iter()
            .position(|tc| tc.active && tc.row == row)
        {
            let tc = &mut self.tile_canvases[idx];
            if tc.w != doc_w || tc.h != tile_h {
                tc.canvas.set_size(doc_w, tile_h);
                tc.w = doc_w;
                tc.h = tile_h;
            }
            tc.canvas.set_position(0, tile_y);
            tc.canvas.copy_pixels_from(pixels);
            return true;
        }

        if let Some(idx) = self.tile_canvases.iter().position(|tc| !tc.active) {
            let tc = &mut self.tile_canvases[idx];
            tc.row = row;
            if tc.w != doc_w || tc.h != tile_h {
                tc.canvas.set_size(doc_w, tile_h);
                tc.w = doc_w;
                tc.h = tile_h;
            }
            tc.canvas.set_position(0, tile_y);
            tc.canvas.copy_pixels_from(pixels);
            tc.active = true;
            return true;
        }

        if self.tile_canvases.len() >= MAX_TILE_CANVASES {
            return false;
        }

        let c = ui::Canvas::new(doc_w, tile_h);
        c.set_position(0, tile_y);
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

        self.tile_canvases.push(TileCanvas {
            row,
            canvas: c,
            active: true,
            w: doc_w,
            h: tile_h,
        });
        true
    }
}
