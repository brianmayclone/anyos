impl Renderer {
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
