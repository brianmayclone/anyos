use alloc::vec::Vec;
use crate::graphics::color::Color;
use crate::graphics::rect::Rect;
use crate::graphics::surface::Surface;

/// Window layer in the compositor
pub struct Layer {
    pub id: u32,
    pub surface: Surface,
    pub x: i32,
    pub y: i32,
    pub visible: bool,
    pub dirty: bool,
}

impl Layer {
    pub fn new(id: u32, x: i32, y: i32, width: u32, height: u32) -> Self {
        Layer {
            id,
            surface: Surface::new(width, height),
            x,
            y,
            visible: true,
            dirty: true,
        }
    }

    pub fn bounds(&self) -> Rect {
        Rect::new(self.x, self.y, self.surface.width, self.surface.height)
    }
}

/// Double-buffered compositor with z-ordered layers
pub struct Compositor {
    /// Back buffer (composited image)
    back_buffer: Surface,
    /// Framebuffer pointer and pitch
    framebuffer_addr: u32,
    framebuffer_pitch: u32,
    /// Screen dimensions
    pub width: u32,
    pub height: u32,
    /// Layers ordered back-to-front (index 0 = bottom)
    layers: Vec<Layer>,
    /// Next layer ID
    next_id: u32,
    /// Damage regions that need recompositing
    damage: Vec<Rect>,
    /// Mouse cursor state
    cursor_x: i32,
    cursor_y: i32,
    cursor_visible: bool,
    /// Hardware double-buffer: flush to HW back page, then flip
    hw_double_buffer: bool,
    /// Previous frame's damage regions (for HW double-buffer page sync)
    prev_damage: Vec<Rect>,
}

impl Compositor {
    pub fn new(width: u32, height: u32, framebuffer_addr: u32, framebuffer_pitch: u32) -> Self {
        Compositor {
            back_buffer: Surface::new_with_color(width, height, Color::MACOS_BG),
            framebuffer_addr,
            framebuffer_pitch,
            width,
            height,
            layers: Vec::new(),
            next_id: 1,
            damage: Vec::new(),
            cursor_x: (width / 2) as i32,
            cursor_y: (height / 2) as i32,
            hw_double_buffer: false,
            prev_damage: Vec::new(),
            cursor_visible: true,
        }
    }

    /// Create a new layer and return its ID
    pub fn create_layer(&mut self, x: i32, y: i32, width: u32, height: u32) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        let layer = Layer::new(id, x, y, width, height);
        self.damage.push(layer.bounds());
        self.layers.push(layer);
        id
    }

    /// Get a mutable reference to a layer's surface for drawing
    pub fn get_layer_surface(&mut self, id: u32) -> Option<&mut Surface> {
        for layer in self.layers.iter_mut() {
            if layer.id == id {
                layer.dirty = true;
                return Some(&mut layer.surface);
            }
        }
        None
    }

    /// Get a reference to a layer
    pub fn get_layer(&self, id: u32) -> Option<&Layer> {
        self.layers.iter().find(|l| l.id == id)
    }

    /// Get a mutable reference to a layer
    pub fn get_layer_mut(&mut self, id: u32) -> Option<&mut Layer> {
        self.layers.iter_mut().find(|l| l.id == id)
    }

    /// Move a layer to a new position
    pub fn move_layer(&mut self, id: u32, x: i32, y: i32) {
        if let Some(layer) = self.layers.iter_mut().find(|l| l.id == id) {
            // Damage old position
            self.damage.push(layer.bounds());
            layer.x = x;
            layer.y = y;
            layer.dirty = true;
            // Damage new position
            self.damage.push(layer.bounds());
        }
    }

    /// Raise a layer to the top of the z-order
    pub fn raise_layer(&mut self, id: u32) {
        if let Some(pos) = self.layers.iter().position(|l| l.id == id) {
            if pos < self.layers.len() - 1 {
                let layer = self.layers.remove(pos);
                self.damage.push(layer.bounds());
                self.layers.push(layer);
            }
        }
    }

    /// Remove a layer
    pub fn remove_layer(&mut self, id: u32) {
        if let Some(pos) = self.layers.iter().position(|l| l.id == id) {
            let layer = self.layers.remove(pos);
            self.damage.push(layer.bounds());
        }
    }

    /// Set layer visibility
    pub fn set_layer_visible(&mut self, id: u32, visible: bool) {
        if let Some(layer) = self.layers.iter_mut().find(|l| l.id == id) {
            if layer.visible != visible {
                layer.visible = visible;
                self.damage.push(layer.bounds());
            }
        }
    }

    /// Update cursor position
    pub fn move_cursor(&mut self, x: i32, y: i32) {
        // Damage old cursor area
        self.damage_cursor();
        self.cursor_x = x.max(0).min(self.width as i32 - 1);
        self.cursor_y = y.max(0).min(self.height as i32 - 1);
        // Damage new cursor area
        self.damage_cursor();
    }

    pub fn cursor_position(&self) -> (i32, i32) {
        (self.cursor_x, self.cursor_y)
    }

    fn damage_cursor(&mut self) {
        self.damage.push(Rect::new(
            self.cursor_x - 1,
            self.cursor_y - 1,
            16,
            20,
        ));
    }

    /// Mark entire screen as damaged (full recomposite)
    pub fn invalidate_all(&mut self) {
        self.damage.push(Rect::new(0, 0, self.width, self.height));
    }

    /// Mark a layer as dirty
    pub fn invalidate_layer(&mut self, id: u32) {
        if let Some(layer) = self.layers.iter_mut().find(|l| l.id == id) {
            layer.dirty = true;
            self.damage.push(layer.bounds());
        }
    }

    /// Find the topmost layer at a given screen coordinate
    pub fn layer_at(&self, x: i32, y: i32) -> Option<u32> {
        for layer in self.layers.iter().rev() {
            if layer.visible && layer.bounds().contains(x, y) {
                return Some(layer.id);
            }
        }
        None
    }

    /// Maximum individual damage rects before merging to single bounding box
    const MAX_DAMAGE_RECTS: usize = 16;

    /// Composite all layers into the back buffer and flush to the framebuffer.
    /// Uses individual damage regions to minimize work.
    pub fn compose(&mut self) {
        // Always add dirty layer bounds to damage list
        for i in 0..self.layers.len() {
            if self.layers[i].dirty {
                let bounds = self.layers[i].bounds();
                self.damage.push(bounds);
            }
        }

        if self.damage.is_empty() {
            // HW double-buffer: still sync prev_damage even with no new work
            if self.hw_double_buffer && !self.prev_damage.is_empty() {
                for rect in &self.prev_damage {
                    self.flush_region(*rect);
                }
                self.prev_damage.clear();
            }
            return;
        }

        // If too many damage rects, merge to single bounding rect
        if self.damage.len() > Self::MAX_DAMAGE_RECTS {
            let mut bounds = self.damage[0];
            for r in &self.damage[1..] {
                bounds = bounds.union(r);
            }
            self.damage.clear();
            self.damage.push(bounds);
        }

        // Clip all damage rects to screen and collect
        let screen_rect = Rect::new(0, 0, self.width, self.height);
        let clipped: Vec<Rect> = self.damage.iter()
            .filter_map(|r| r.intersection(&screen_rect))
            .collect();

        if clipped.is_empty() {
            self.damage.clear();
            // HW double-buffer: still need to sync prev_damage to current back page
            if self.hw_double_buffer && !self.prev_damage.is_empty() {
                for rect in &self.prev_damage {
                    self.flush_region(*rect);
                }
                self.prev_damage.clear();
            }
            return;
        }

        // HW double-buffer catch-up: the current back page was the front page last
        // frame, so it's missing the previous frame's updates. Flush them now from
        // the (already-correct) software back buffer.
        if self.hw_double_buffer {
            for rect in &self.prev_damage {
                self.flush_region(*rect);
            }
        }

        // Process each damage rect independently
        for damage_rect in &clipped {
            // Clear the damaged region with background color
            self.back_buffer.fill_rect(*damage_rect, Color::MACOS_BG);

            // Composite each visible layer that intersects this damage rect
            for layer_idx in 0..self.layers.len() {
                if !self.layers[layer_idx].visible {
                    continue;
                }
                let layer_bounds = self.layers[layer_idx].bounds();
                if let Some(intersection) = layer_bounds.intersection(damage_rect) {
                    // Blit only the clipped region for efficiency
                    let src_rect = Rect::new(
                        intersection.x - self.layers[layer_idx].x,
                        intersection.y - self.layers[layer_idx].y,
                        intersection.width,
                        intersection.height,
                    );
                    self.back_buffer.blit_rect(
                        &self.layers[layer_idx].surface,
                        src_rect,
                        intersection.x,
                        intersection.y,
                    );
                }
            }

            // Draw cursor if it intersects this damage rect
            if self.cursor_visible {
                let cursor_rect = Rect::new(self.cursor_x - 1, self.cursor_y - 1, 16, 20);
                if cursor_rect.intersects(damage_rect) {
                    self.draw_cursor();
                }
            }

            // Flush this damage rect to framebuffer
            self.flush_region(*damage_rect);
        }

        // Save current damage for next frame's back-page catch-up
        if self.hw_double_buffer {
            self.prev_damage = clipped;
        }

        // Clear damage
        self.damage.clear();
        for layer in &mut self.layers {
            layer.dirty = false;
        }

        // Hardware page flip if double-buffering is active
        if self.hw_double_buffer {
            crate::drivers::bochs_vga::flip();
            // Update framebuffer_addr to the new back page
            if let Some(back) = crate::drivers::bochs_vga::back_buffer_phys() {
                self.framebuffer_addr = back;
            }
        }
    }

    /// Enable hardware double-buffering (call after bochs_vga::init).
    pub fn enable_hw_double_buffer(&mut self) {
        if let Some(back) = crate::drivers::bochs_vga::back_buffer_phys() {
            self.framebuffer_addr = back;
            self.hw_double_buffer = true;
            // Do an initial full flush to the back page, then flip
            self.flush_all();
            crate::drivers::bochs_vga::flip();
            // Point to the new back page for future rendering
            if let Some(new_back) = crate::drivers::bochs_vga::back_buffer_phys() {
                self.framebuffer_addr = new_back;
            }
        }
    }

    /// Draw a simple arrow cursor
    fn draw_cursor(&mut self) {
        let cx = self.cursor_x;
        let cy = self.cursor_y;

        // Simple arrow cursor (12x18 pixels)
        static CURSOR: [u16; 18] = [
            0b1000000000000000,
            0b1100000000000000,
            0b1110000000000000,
            0b1111000000000000,
            0b1111100000000000,
            0b1111110000000000,
            0b1111111000000000,
            0b1111111100000000,
            0b1111111110000000,
            0b1111111111000000,
            0b1111111111100000,
            0b1111110000000000,
            0b1110011000000000,
            0b1100011000000000,
            0b1000001100000000,
            0b0000001100000000,
            0b0000000110000000,
            0b0000000000000000,
        ];

        // Draw cursor shadow (offset by 1)
        for (row, &bits) in CURSOR.iter().enumerate() {
            for col in 0..12 {
                if bits & (0x8000 >> col) != 0 {
                    self.back_buffer.put_pixel(
                        cx + col + 1,
                        cy + row as i32 + 1,
                        Color::with_alpha(128, 0, 0, 0),
                    );
                }
            }
        }

        // Draw cursor body (white with black outline)
        static CURSOR_OUTLINE: [u16; 18] = [
            0b1100000000000000,
            0b1010000000000000,
            0b1001000000000000,
            0b1000100000000000,
            0b1000010000000000,
            0b1000001000000000,
            0b1000000100000000,
            0b1000000010000000,
            0b1000000001000000,
            0b1000000000100000,
            0b1000000000010000,
            0b1000001110000000,
            0b1001000100000000,
            0b1010000100000000,
            0b1100000010000000,
            0b0000000010000000,
            0b0000000001100000,
            0b0000000000000000,
        ];

        for (row, &bits) in CURSOR.iter().enumerate() {
            let outline_bits = CURSOR_OUTLINE[row];
            for col in 0..12 {
                let mask = 0x8000u16 >> col;
                if outline_bits & mask != 0 {
                    self.back_buffer.put_pixel(cx + col, cy + row as i32, Color::BLACK);
                } else if bits & mask != 0 {
                    self.back_buffer.put_pixel(cx + col, cy + row as i32, Color::WHITE);
                }
            }
        }
    }

    /// Flush a region from the back buffer to the physical framebuffer.
    /// Uses u32 row copies — on little-endian x86, our ARGB u32 layout
    /// naturally produces BGRA byte order which is what VBE expects.
    fn flush_region(&self, region: Rect) {
        let fb = self.framebuffer_addr as *mut u32;
        let pitch_u32 = (self.framebuffer_pitch / 4) as usize;

        let x0 = region.x.max(0) as u32;
        let y0 = region.y.max(0) as u32;
        let x1 = (region.right() as u32).min(self.width);
        let y1 = (region.bottom() as u32).min(self.height);

        if x0 >= x1 || y0 >= y1 {
            return;
        }

        let row_len = (x1 - x0) as usize;

        for y in y0..y1 {
            let src_off = (y * self.width + x0) as usize;
            let dst_off = y as usize * pitch_u32 + x0 as usize;
            let src = &self.back_buffer.pixels[src_off..src_off + row_len];
            let dst = unsafe { core::slice::from_raw_parts_mut(fb.add(dst_off), row_len) };
            dst.copy_from_slice(src);
        }
    }

    /// Flush the entire back buffer to the framebuffer
    pub fn flush_all(&self) {
        self.flush_region(Rect::new(0, 0, self.width, self.height));
    }
}
