//! Renderer: maps a LayoutBox tree to libanyui controls.
//!
//! Walks the layout tree produced by the layout engine and creates
//! real libanyui controls (Labels, Views, ImageViews, etc.) for each
//! visible element, positioned according to CSS layout calculations.

use alloc::string::String;
use alloc::vec::Vec;

use libanyui_client::{self as ui, Widget};

use crate::layout::{LayoutBox, BoxType, FormFieldKind};
use crate::style::TextDeco;

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
        // Replace if already exists.
        if let Some(entry) = self.entries.iter_mut().find(|e| e.src == src) {
            entry.pixels = pixels;
            entry.width = width;
            entry.height = height;
            return;
        }
        self.entries.push(ImageEntry { src, pixels, width, height });
    }
}

/// Information about a rendered form control.
pub struct FormControl {
    /// The libanyui control ID.
    pub control_id: u32,
    /// The DOM node ID of the form element.
    pub node_id: usize,
    /// The form field kind.
    pub kind: FormFieldKind,
    /// The input name attribute (for form submission).
    pub name: String,
}

/// Tracks all controls created for a page, allowing bulk cleanup.
pub(crate) struct Renderer {
    /// Control IDs of all controls created for the current page.
    controls: Vec<u32>,
    /// Mapping from control ID → link URL for click handling.
    pub link_map: Vec<(u32, String)>,
    /// Mapping of form controls for data collection.
    pub form_controls: Vec<FormControl>,
}

impl Renderer {
    pub fn new() -> Self {
        Self { controls: Vec::new(), link_map: Vec::new(), form_controls: Vec::new() }
    }

    /// Return the number of controls currently tracked.
    pub fn control_count(&self) -> usize {
        self.controls.len()
    }

    /// Remove all previously created controls from the UI tree.
    pub fn clear(&mut self) {
        for &id in &self.controls {
            ui::Control::from_id(id).remove();
        }
        self.controls.clear();
        self.link_map.clear();
        self.form_controls.clear();
    }

    /// Walk the layout tree and create libanyui controls inside `parent`.
    ///
    /// `link_callback_id` is the event callback used for link clicks
    /// (registered by the WebView owner).
    pub fn render(
        &mut self,
        root: &LayoutBox,
        parent: &ui::View,
        images: &ImageCache,
        link_cb: Option<ui::Callback>,
        link_cb_ud: u64,
        submit_cb: Option<ui::Callback>,
        submit_cb_ud: u64,
    ) {
        crate::debug_surf!("[render] render start");
        #[cfg(feature = "debug_surf")]
        crate::debug_surf!("[render]   RSP=0x{:X} heap=0x{:X}", crate::debug_rsp(), crate::debug_heap_pos());
        self.walk(root, parent, images, 0, 0, link_cb, link_cb_ud, submit_cb, submit_cb_ud);
        crate::debug_surf!("[render] render done: {} controls created", self.controls.len());
        #[cfg(feature = "debug_surf")]
        crate::debug_surf!("[render]   RSP=0x{:X} heap=0x{:X}", crate::debug_rsp(), crate::debug_heap_pos());
    }

    fn walk(
        &mut self,
        bx: &LayoutBox,
        parent: &ui::View,
        images: &ImageCache,
        offset_x: i32,
        offset_y: i32,
        link_cb: Option<ui::Callback>,
        link_cb_ud: u64,
        submit_cb: Option<ui::Callback>,
        submit_cb_ud: u64,
    ) {
        // position:fixed boxes use viewport-absolute coordinates; ignore parent offsets.
        let (abs_x, abs_y) = if bx.is_fixed {
            (bx.x, bx.y)
        } else {
            (offset_x + bx.x, offset_y + bx.y)
        };

        // Skip invisible boxes.
        if bx.visibility_hidden {
            return;
        }

        // Background/border box — create View(s) for visible bg and/or border.
        let has_bg = bx.bg_color != 0 && bx.bg_color != 0x00000000;
        let has_border = bx.border_width > 0 && bx.border_color != 0 && bx.border_color != 0x00000000;
        if has_border {
            // Outer view = border color, full box size.
            let outer = ui::View::new();
            outer.set_position(abs_x, abs_y);
            outer.set_size(bx.width as u32, bx.height as u32);
            outer.set_color(bx.border_color);
            parent.add(&outer);
            self.controls.push(outer.id());
            // Inner view = background color, inset by border width.
            let bw = bx.border_width;
            let inner_w = (bx.width - bw * 2).max(0) as u32;
            let inner_h = (bx.height - bw * 2).max(0) as u32;
            if inner_w > 0 && inner_h > 0 {
                let inner = ui::View::new();
                inner.set_position(abs_x + bw, abs_y + bw);
                inner.set_size(inner_w, inner_h);
                let bg = if has_bg { bx.bg_color } else { 0xFFFFFFFF };
                inner.set_color(bg);
                parent.add(&inner);
                self.controls.push(inner.id());
            }
        } else if has_bg {
            let view = ui::View::new();
            view.set_position(abs_x, abs_y);
            view.set_size(bx.width as u32, bx.height as u32);
            view.set_color(bx.bg_color);
            parent.add(&view);
            self.controls.push(view.id());
        }

        // Horizontal rule — thin gray line.
        if bx.is_hr {
            let hr = ui::View::new();
            hr.set_position(abs_x, abs_y);
            hr.set_size(bx.width as u32, 1);
            hr.set_color(0xFF999999);
            parent.add(&hr);
            self.controls.push(hr.id());
            return;
        }

        // List marker (bullet/number).
        if let Some(ref marker) = bx.list_marker {
            let lbl = ui::Label::new(marker);
            lbl.set_position(abs_x - 20, abs_y);
            lbl.set_font_size(bx.font_size as u32);
            lbl.set_text_color(bx.color);
            lbl.set_size(20, bx.font_size as u32 + 4);
            parent.add(&lbl);
            self.controls.push(lbl.id());
        }

        // Text fragment.
        if let Some(ref text) = bx.text {
            if !text.is_empty() {
                let lbl = ui::Label::new(text);
                lbl.set_position(abs_x, abs_y);
                lbl.set_size(bx.width as u32, bx.height as u32);
                lbl.set_font_size(bx.font_size as u32);
                lbl.set_text_color(bx.color);
                if bx.bold {
                    lbl.set_font(1); // font_id 1 = bold
                }
                parent.add(&lbl);
                self.controls.push(lbl.id());

                // Underline for links or text-decoration.
                let needs_underline = bx.text_decoration == TextDeco::Underline
                    || bx.link_url.is_some();
                if needs_underline {
                    let underline = ui::View::new();
                    underline.set_position(abs_x, abs_y + bx.height - 1);
                    underline.set_size(bx.width as u32, 1);
                    let ul_color = if bx.link_url.is_some() { bx.color } else { bx.color };
                    underline.set_color(ul_color);
                    parent.add(&underline);
                    self.controls.push(underline.id());
                }

                // Line-through decoration.
                if bx.text_decoration == TextDeco::LineThrough {
                    let strike = ui::View::new();
                    strike.set_position(abs_x, abs_y + bx.height / 2);
                    strike.set_size(bx.width as u32, 1);
                    strike.set_color(bx.color);
                    parent.add(&strike);
                    self.controls.push(strike.id());
                }

                // Link click handler.
                if let Some(ref url) = bx.link_url {
                    self.link_map.push((lbl.id(), url.clone()));
                    if let Some(cb) = link_cb {
                        lbl.on_click_raw(cb, link_cb_ud);
                    }
                }
            }
        }

        // Image — only render if image data is available (avoid black rectangles).
        if let Some(ref src) = bx.image_src {
            if let Some(entry) = images.get(src) {
                let iw = bx.image_width.unwrap_or(bx.width) as u32;
                let ih = bx.image_height.unwrap_or(bx.height) as u32;
                let img = ui::ImageView::new(iw, ih);
                img.set_position(abs_x, abs_y);
                img.set_size(iw, ih);
                img.set_pixels(&entry.pixels, entry.width, entry.height);
                parent.add(&img);
                self.controls.push(img.id());
            }
        }

        // Form fields.
        if let Some(kind) = bx.form_field {
            self.emit_form_control(kind, bx, abs_x, abs_y, parent, submit_cb, submit_cb_ud);
        }

        // Recurse into children.
        for child in &bx.children {
            self.walk(child, parent, images, abs_x, abs_y, link_cb, link_cb_ud, submit_cb, submit_cb_ud);
        }
    }

    fn emit_form_control(
        &mut self,
        kind: FormFieldKind,
        bx: &LayoutBox,
        x: i32,
        y: i32,
        parent: &ui::View,
        submit_cb: Option<ui::Callback>,
        submit_cb_ud: u64,
    ) {
        match kind {
            FormFieldKind::TextInput => {
                let tf = ui::TextField::new();
                tf.set_position(x, y);
                tf.set_size(bx.width as u32, bx.height as u32);
                // Web-style appearance: white background, dark text.
                tf.set_color(0xFFFFFFFF);
                tf.set_text_color(0xFF000000);
                if let Some(ref ph) = bx.form_placeholder {
                    tf.set_placeholder(ph);
                }
                if let Some(ref val) = bx.form_value {
                    tf.set_text(val);
                }
                parent.add(&tf);
                let id = tf.id();
                self.controls.push(id);
                self.register_form_control(id, bx, kind);
            }
            FormFieldKind::Password => {
                let tf = ui::TextField::new();
                tf.set_password_mode(true);
                tf.set_position(x, y);
                tf.set_size(bx.width as u32, bx.height as u32);
                // Web-style appearance: white background, dark text.
                tf.set_color(0xFFFFFFFF);
                tf.set_text_color(0xFF000000);
                if let Some(ref ph) = bx.form_placeholder {
                    tf.set_placeholder(ph);
                }
                if let Some(ref val) = bx.form_value {
                    tf.set_text(val);
                }
                parent.add(&tf);
                let id = tf.id();
                self.controls.push(id);
                self.register_form_control(id, bx, kind);
            }
            FormFieldKind::Submit | FormFieldKind::ButtonEl => {
                let label = if let Some(ref t) = bx.text { t.as_str() } else { "Submit" };
                let btn = ui::Button::new(label);
                btn.set_position(x, y);
                btn.set_size(bx.width as u32, bx.height as u32);
                parent.add(&btn);
                let id = btn.id();
                self.controls.push(id);
                self.register_form_control(id, bx, kind);
                // Wire submit callback.
                if let Some(cb) = submit_cb {
                    btn.on_click_raw(cb, submit_cb_ud);
                }
            }
            FormFieldKind::Checkbox => {
                let cb = ui::Checkbox::new("");
                cb.set_position(x, y);
                cb.set_size(bx.width as u32, bx.height as u32);
                parent.add(&cb);
                let id = cb.id();
                self.controls.push(id);
                self.register_form_control(id, bx, kind);
            }
            FormFieldKind::Radio => {
                let rb = ui::RadioButton::new("");
                rb.set_position(x, y);
                rb.set_size(bx.width as u32, bx.height as u32);
                parent.add(&rb);
                let id = rb.id();
                self.controls.push(id);
                self.register_form_control(id, bx, kind);
            }
            FormFieldKind::Textarea => {
                let ta = ui::TextArea::new();
                ta.set_position(x, y);
                ta.set_size(bx.width as u32, bx.height as u32);
                ta.set_color(0xFFFFFFFF);
                ta.set_text_color(0xFF000000);
                parent.add(&ta);
                let id = ta.id();
                self.controls.push(id);
                self.register_form_control(id, bx, kind);
            }
            FormFieldKind::Hidden => {
                // No visible control, but register for form data collection.
                let node_id = bx.node_id.unwrap_or(0);
                self.form_controls.push(FormControl {
                    control_id: 0, // no UI control
                    node_id,
                    kind,
                    name: String::new(),
                });
            }
        }
    }

    fn register_form_control(&mut self, control_id: u32, bx: &LayoutBox, kind: FormFieldKind) {
        let node_id = bx.node_id.unwrap_or(0);
        // Get the name from form_value metadata or leave empty.
        // The actual name is read from the DOM at form submission time.
        self.form_controls.push(FormControl {
            control_id,
            node_id,
            kind,
            name: String::new(), // populated at form collection time
        });
    }
}
