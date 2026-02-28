//! Canvas-based renderer: draws static content (text, backgrounds, borders,
//! images) into a single Canvas pixel buffer.  Only truly interactive form
//! controls (TextField, Checkbox, etc.) are real libanyui controls — created
//! once and updated in-place on relayouts (never destroyed).

use alloc::string::String;
use alloc::vec::Vec;

use libanyui_client::{self as ui, Widget};

use crate::layout::{LayoutBox, FormFieldKind};
use crate::style::TextDeco;

// ═══════════════════════════════════════════════════════════════════════════
// Image cache (unchanged)
// ═══════════════════════════════════════════════════════════════════════════

/// Image cache entry (decoded pixel data).
pub struct ImageEntry {
    pub src: String,
    pub pixels: Vec<u32>,
    pub width: u32,
    pub height: u32,
}

/// Cache of decoded images, looked up by URL.
pub struct ImageCache {
    pub entries: Vec<ImageEntry>,
}

impl ImageCache {
    pub fn new() -> Self {
        ImageCache { entries: Vec::new() }
    }

    pub fn get(&self, src: &str) -> Option<&ImageEntry> {
        self.entries.iter().find(|e| e.src == src)
    }

    pub fn add(&mut self, src: String, pixels: Vec<u32>, width: u32, height: u32) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.src == src) {
            entry.pixels = pixels;
            entry.width = width;
            entry.height = height;
            return;
        }
        self.entries.push(ImageEntry { src, pixels, width, height });
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Hit regions (for link/submit click handling on the canvas)
// ═══════════════════════════════════════════════════════════════════════════

/// A clickable region on the canvas.
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
// Renderer
// ═══════════════════════════════════════════════════════════════════════════

/// Canvas-based renderer that draws static content into a pixel buffer and
/// manages persistent form controls.
///
/// Uses viewport-based tile rendering: only the visible viewport plus a buffer
/// zone is rendered into the pixel buffer.  On scroll, the tile is re-rendered
/// from the cached layout tree without a full relayout.
pub(crate) struct Renderer {
    /// The single Canvas for all static content.
    canvas: Option<ui::Canvas>,
    /// Current canvas dimensions.
    canvas_w: u32,
    canvas_h: u32,
    /// Clickable regions (links, submit buttons) for hit-testing.
    pub hit_regions: Vec<HitRegion>,
    /// Persistent form controls — only destroyed on full page navigation.
    pub form_controls: Vec<FormControl>,
    /// Compatibility: control_id → link URL (for submit button Labels).
    pub link_map: Vec<(u32, String)>,
    /// Current tile origin Y in document coordinates.
    render_y: i32,
}

impl Renderer {
    pub fn new() -> Self {
        Self {
            canvas: None,
            canvas_w: 0,
            canvas_h: 0,
            hit_regions: Vec::new(),
            form_controls: Vec::new(),
            link_map: Vec::new(),
            render_y: 0,
        }
    }

    /// Return the current tile origin Y (document coordinates).
    pub fn render_y(&self) -> i32 {
        self.render_y
    }

    /// Return the canvas control ID (for identifying canvas clicks).
    pub fn canvas_id(&self) -> Option<u32> {
        self.canvas.as_ref().map(|c| c.id())
    }

    /// Return a reference to the canvas (for mouse position queries).
    pub fn canvas_ref(&self) -> Option<&ui::Canvas> {
        self.canvas.as_ref()
    }

    /// Return the number of form controls currently tracked.
    pub fn control_count(&self) -> usize {
        self.form_controls.len()
    }

    /// Soft clear: reset hit regions and link map but keep canvas + form controls.
    /// Called on each relayout.
    pub fn clear(&mut self) {
        self.hit_regions.clear();
        self.link_map.clear();
        // Mark all form controls as unseen for GC.
        for fc in &mut self.form_controls {
            fc.seen = false;
        }
    }

    /// Hard clear: destroy everything including canvas and form controls.
    /// Called on full page navigation (new URL).
    pub fn clear_all(&mut self) {
        for fc in &self.form_controls {
            if fc.control_id != 0 {
                ui::Control::from_id(fc.control_id).remove();
            }
        }
        self.form_controls.clear();
        if let Some(ref c) = self.canvas {
            ui::Control::from_id(c.id()).remove();
        }
        self.canvas = None;
        self.canvas_w = 0;
        self.canvas_h = 0;
        self.hit_regions.clear();
        self.link_map.clear();
    }

    /// Hit-test the canvas at the given coordinates for a link URL.
    pub fn hit_test_link(&self, x: i32, y: i32) -> Option<&str> {
        for region in &self.hit_regions {
            if x >= region.x && x < region.x + region.w
                && y >= region.y && y < region.y + region.h
            {
                if let HitKind::Link(ref url) = region.kind {
                    return Some(url.as_str());
                }
            }
        }
        None
    }

    /// Hit-test the canvas at the given coordinates for a submit button.
    pub fn hit_test_submit(&self, x: i32, y: i32) -> Option<usize> {
        for region in &self.hit_regions {
            if x >= region.x && x < region.x + region.w
                && y >= region.y && y < region.y + region.h
            {
                if let HitKind::Submit(node_id) = region.kind {
                    return Some(node_id);
                }
            }
        }
        None
    }

    /// Render the layout tree into the canvas using viewport-based tiling.
    ///
    /// Only the visible viewport region (plus a buffer zone above/below) is
    /// allocated and drawn.  This keeps memory usage proportional to the
    /// viewport size (~6 MiB) instead of the full document height.
    ///
    /// - `root`: root LayoutBox from the layout engine.
    /// - `parent`: the content_view to add canvas and form controls into.
    /// - `images`: decoded image cache.
    /// - `doc_w`, `doc_h`: document dimensions.
    /// - `viewport_h`: visible viewport height in pixels.
    /// - `scroll_y`: current vertical scroll offset.
    /// - `bg_color`: body background color for the canvas clear.
    /// - `link_cb`, `link_cb_ud`: C ABI callback for canvas clicks.
    /// - `submit_cb`, `submit_cb_ud`: C ABI callback for form submit controls.
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
        crate::debug_surf!("[render] tile render start ({}x{}, vp_h={}, scroll_y={})",
            doc_w, doc_h, viewport_h, scroll_y);

        let w = doc_w.max(1);

        // ── Compute visible tile bounds ──
        const BUFFER_ZONE: i32 = 500;
        let render_y_start = (scroll_y - BUFFER_ZONE).max(0);
        let render_y_end = (scroll_y + viewport_h as i32 + BUFFER_ZONE).min(doc_h as i32);
        let tile_h = ((render_y_end - render_y_start) as u32).max(1);

        // Ensure canvas exists and has correct size, positioned at tile origin.
        self.ensure_canvas(parent, w, tile_h, link_cb, link_cb_ud);
        if let Some(ref canvas) = self.canvas {
            canvas.set_position(0, render_y_start);
        }
        self.render_y = render_y_start;

        // Allocate a LOCAL pixel buffer for the visible tile only.
        let pixel_count = (w as usize) * (tile_h as usize);
        let clear_color = if bg_color != 0 { bg_color } else { 0xFFFFFFFF };
        let mut local_buf: Vec<u32> = Vec::with_capacity(pixel_count);
        local_buf.resize(pixel_count, clear_color);
        let buf = local_buf.as_mut_ptr();

        // Walk layout tree — draws only visible boxes, culls the rest.
        self.walk_canvas(
            root, buf, w, tile_h, images,
            0, 0,
            render_y_start, render_y_end, scroll_y,
            parent, submit_cb, submit_cb_ud,
        );

        // Transfer the rendered tile to the canvas (single IPC call).
        if let Some(ref canvas) = self.canvas {
            canvas.copy_pixels_from(&local_buf);
        }

        // GC: remove form controls that were not seen in this render pass.
        self.form_controls.retain(|fc| {
            if !fc.seen && fc.control_id != 0 {
                ui::Control::from_id(fc.control_id).remove();
                false
            } else {
                fc.seen || fc.control_id == 0
            }
        });

        crate::debug_surf!("[render] tile render done: {}x{} at y={}, {} hit_regions, {} form_controls",
            w, tile_h, render_y_start, self.hit_regions.len(), self.form_controls.len());
    }

    /// Ensure the canvas exists and has the correct size.
    fn ensure_canvas(
        &mut self,
        parent: &ui::View,
        w: u32,
        h: u32,
        link_cb: Option<ui::Callback>,
        link_cb_ud: u64,
    ) -> &ui::Canvas {
        // Minimum height of 1 pixel.
        let h = h.max(1);
        let w = w.max(1);

        if self.canvas.is_none() {
            let c = ui::Canvas::new(w, h);
            c.set_position(0, 0);
            c.set_size(w, h);
            parent.add(&c);
            // Register link click callback on the canvas.
            if let Some(cb) = link_cb {
                c.on_click_raw(cb, link_cb_ud);
            }
            self.canvas = Some(c);
            self.canvas_w = w;
            self.canvas_h = h;
        } else if w != self.canvas_w || h != self.canvas_h {
            let c = self.canvas.as_ref().unwrap();
            c.set_size(w, h);
            self.canvas_w = w;
            self.canvas_h = h;
        }

        self.canvas.as_ref().unwrap()
    }

    /// Walk the layout tree and draw visible boxes into the tile pixel buffer.
    ///
    /// Boxes outside `[render_y_start, render_y_end)` are culled (no pixel
    /// drawing) but form controls are always processed so that ScrollView can
    /// clip them independently.
    fn walk_canvas(
        &mut self,
        bx: &LayoutBox,
        buf: *mut u32,
        stride: u32,
        buf_h: u32,
        images: &ImageCache,
        offset_x: i32,
        offset_y: i32,
        render_y_start: i32,
        render_y_end: i32,
        scroll_y: i32,
        parent: &ui::View,
        submit_cb: Option<ui::Callback>,
        submit_cb_ud: u64,
    ) {
        let (abs_x, abs_y) = if bx.is_fixed {
            (bx.x, bx.y)
        } else {
            (offset_x + bx.x, offset_y + bx.y)
        };

        // Skip invisible boxes.
        if bx.visibility_hidden {
            return;
        }

        // Determine if this box is within the visible tile.
        // Fixed-position boxes are always drawn (they're viewport-anchored).
        let in_tile = bx.is_fixed
            || (abs_y + bx.height > render_y_start && abs_y < render_y_end);

        // Translate Y to tile-local coordinates for pixel drawing.
        let draw_y = if bx.is_fixed {
            // Fixed elements are viewport-relative — map to tile position.
            bx.y + (scroll_y - render_y_start)
        } else {
            abs_y - render_y_start
        };

        // ── Pixel drawing (only for visible boxes) ──
        if in_tile {
            // Background.
            let has_bg = bx.bg_color != 0 && bx.bg_color != 0x00000000;
            if has_bg {
                fill_rect_buf(buf, stride, buf_h, abs_x, draw_y, bx.width, bx.height, bx.bg_color);
            }

            // Border (4 edges).
            let has_border = bx.border_width > 0 && bx.border_color != 0 && bx.border_color != 0x00000000;
            if has_border {
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
                // Still need to process children / form fields below, so no return.
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
                    let needs_underline = bx.text_decoration == TextDeco::Underline
                        || bx.link_url.is_some();
                    if needs_underline {
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

                    // Register link hit region (tile-local coordinates for click matching).
                    if let Some(ref url) = bx.link_url {
                        self.hit_regions.push(HitRegion {
                            x: abs_x, y: draw_y,
                            w: bx.width, h: bx.height,
                            kind: HitKind::Link(url.clone()),
                        });
                    }
                }
            }

            // Image — blit directly into canvas buffer.
            if let Some(ref src) = bx.image_src {
                if let Some(entry) = images.get(src) {
                    let dw = bx.image_width.unwrap_or(bx.width);
                    let dh = bx.image_height.unwrap_or(bx.height);
                    blit_image_buf(
                        buf, stride, buf_h,
                        abs_x, draw_y, dw, dh,
                        &entry.pixels, entry.width, entry.height,
                    );
                }
            }
        }

        // ── Form controls (always processed — ScrollView clips them) ──
        // Real libanyui controls use absolute document coordinates (abs_x, abs_y).
        // Submit/button pixel drawing uses tile-local draw_y and is gated by in_tile.
        if let Some(kind) = bx.form_field {
            self.emit_form_control(kind, bx, abs_x, abs_y, draw_y, in_tile, buf, stride, buf_h, parent, submit_cb, submit_cb_ud);
        }

        // Recurse into children.
        for child in &bx.children {
            self.walk_canvas(
                child, buf, stride, buf_h, images,
                abs_x, abs_y,
                render_y_start, render_y_end, scroll_y,
                parent, submit_cb, submit_cb_ud,
            );
        }
    }

    /// Create or update a persistent form control.
    ///
    /// - `x`, `y`: absolute document coordinates (for real libanyui controls).
    /// - `draw_y`: tile-local Y coordinate (for pixel buffer drawing).
    /// - `in_tile`: whether this box is within the visible tile.
    fn emit_form_control(
        &mut self,
        kind: FormFieldKind,
        bx: &LayoutBox,
        x: i32,
        y: i32,
        draw_y: i32,
        in_tile: bool,
        buf: *mut u32,
        stride: u32,
        buf_h: u32,
        parent: &ui::View,
        submit_cb: Option<ui::Callback>,
        submit_cb_ud: u64,
    ) {
        let node_id = bx.node_id.unwrap_or(0);

        match kind {
            FormFieldKind::TextInput | FormFieldKind::Password => {
                // Look up existing control by node_id.
                if let Some(fc) = self.form_controls.iter_mut().find(|fc| fc.node_id == node_id && fc.kind == kind) {
                    // Update position/size/color.
                    let ctrl = ui::Control::from_id(fc.control_id);
                    ctrl.set_position(x, y);
                    ctrl.set_size(bx.width as u32, bx.height as u32);
                    let bg = if bx.bg_color != 0 { bx.bg_color } else { 0xFFFFFFFF };
                    let fg = if bx.color != 0 { bx.color } else { 0xFF000000 };
                    ctrl.set_color(bg);
                    ctrl.set_text_color(fg);
                    fc.seen = true;
                } else {
                    // Create new control.
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
                // Draw button appearance into canvas (only if visible in tile).
                if in_tile {
                    let label_text = if let Some(ref t) = bx.text { t.as_str() } else { "Submit" };

                    // Default web button bg + border if no CSS styling.
                    if bx.bg_color == 0 && bx.border_width == 0 {
                        fill_rect_buf(buf, stride, buf_h, x, draw_y, bx.width, bx.height, 0xFFE0E0E0);
                        // 1px border.
                        fill_rect_buf(buf, stride, buf_h, x, draw_y, bx.width, 1, 0xFF808080);
                        fill_rect_buf(buf, stride, buf_h, x, draw_y + bx.height - 1, bx.width, 1, 0xFF808080);
                        fill_rect_buf(buf, stride, buf_h, x, draw_y + 1, 1, (bx.height - 2).max(0), 0xFF808080);
                        fill_rect_buf(buf, stride, buf_h, x + bx.width - 1, draw_y + 1, 1, (bx.height - 2).max(0), 0xFF808080);
                    }

                    // Center text in button.
                    let font_size = bx.font_size.max(1) as u16;
                    let text_color = if bx.color != 0 { bx.color } else { 0xFF000000 };
                    let (tw, _th) = libfont_client::measure(0, font_size, label_text);
                    let tx = x + (bx.width - tw as i32) / 2;
                    let ty = draw_y + (bx.height - font_size as i32) / 2;
                    libfont_client::draw_string_buf(
                        buf, stride, buf_h,
                        tx, ty, text_color, 0, font_size,
                        label_text,
                    );

                    // Register submit hit region (tile-local coords for canvas click matching).
                    self.hit_regions.push(HitRegion {
                        x, y: draw_y, w: bx.width, h: bx.height,
                        kind: HitKind::Submit(node_id),
                    });
                }
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
                // No visible control, but register for form data collection.
                if !self.form_controls.iter().any(|fc| fc.node_id == node_id && fc.kind == kind) {
                    self.form_controls.push(FormControl {
                        control_id: 0, node_id, kind,
                        name: String::new(), seen: true,
                    });
                } else {
                    // Mark as seen.
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
            // Alpha-blend if color is semi-transparent.
            let alpha = (color >> 24) & 0xFF;
            if alpha >= 255 {
                // Opaque fast path.
                for i in 0..cw {
                    *ptr.add(i) = color;
                }
            } else if alpha > 0 {
                // Alpha blend.
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

    // Clip destination to buffer bounds.
    let x0 = dx.max(0);
    let y0 = dy.max(0);
    let x1 = (dx + dw).min(s);
    let y1 = (dy + dh).min(bh);
    if x0 >= x1 || y0 >= y1 { return; }

    // Nearest-neighbor scaling.
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
