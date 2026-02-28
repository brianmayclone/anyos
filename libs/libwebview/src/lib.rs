//! libwebview — HTML rendering library for anyOS.
//!
//! Renders HTML content into a single Canvas pixel buffer for static content
//! (text, backgrounds, borders, images) and uses persistent libanyui controls
//! only for interactive form elements (TextField, Checkbox, etc.).
//!
//! # Usage
//! ```rust
//! use libwebview::WebView;
//!
//! let mut wv = WebView::new(800, 600);
//! parent_view.add(&wv.scroll_view());
//! wv.scroll_view().set_dock(libanyui_client::DOCK_FILL);
//! wv.set_html("<h1>Hello World</h1><p>This is rendered with real controls.</p>");
//! ```

#![no_std]

extern crate alloc;

// ═══════════════════════════════════════════════════════════
// Debug logging macro — enabled by `debug_surf` feature flag
// ═══════════════════════════════════════════════════════════

/// Debug logging macro for the Surf browser pipeline.
/// Compiles to a no-op when the `debug_surf` feature is not enabled.
#[cfg(feature = "debug_surf")]
#[macro_export]
macro_rules! debug_surf {
    ($($arg:tt)*) => {
        anyos_std::println!($($arg)*);
    };
}

#[cfg(not(feature = "debug_surf"))]
#[macro_export]
macro_rules! debug_surf {
    ($($arg:tt)*) => {};
}

/// Return current stack pointer (approximate) for debug tracing.
#[cfg(feature = "debug_surf")]
#[inline(always)]
pub fn debug_rsp() -> u64 {
    let rsp: u64;
    unsafe { core::arch::asm!("mov {}, rsp", out(reg) rsp); }
    rsp
}

/// Return current heap break position for debug tracing.
#[cfg(feature = "debug_surf")]
pub fn debug_heap_pos() -> u64 {
    // sbrk(0) returns current break without changing it.
    anyos_std::process::sbrk(0) as u64
}

pub mod dom;
pub mod html;
pub mod css;
pub mod style;
pub mod layout;
pub mod js;
mod renderer;

use alloc::string::String;
use alloc::vec::Vec;

use libanyui_client::{self as ui};

pub use renderer::{ImageCache, ImageEntry, FormControl, HitKind};
pub use layout::{LayoutBox, FormFieldKind};

/// A WebView renders HTML content inside a ScrollView using libanyui controls.
pub struct WebView {
    scroll_view: ui::ScrollView,
    content_view: ui::View,
    renderer: renderer::Renderer,
    dom_val: Option<dom::Dom>,
    /// Browser default stylesheet — parsed once in `new()`, reused on every relayout.
    default_sheet: css::Stylesheet,
    /// Pre-parsed external stylesheets — parsed once in `add_stylesheet()` and cached.
    /// Eliminates the need to re-parse up to several hundred KB of CSS on every image load.
    external_sheets: Vec<css::Stylesheet>,
    pub images: ImageCache,
    viewport_width: i32,
    total_height_val: i32,
    link_cb: Option<ui::Callback>,
    link_cb_ud: u64,
    /// Form submit callback (called when a submit button is clicked).
    submit_cb: Option<ui::Callback>,
    submit_cb_ud: u64,
    /// JavaScript runtime for executing <script> tags.
    js_runtime: js::JsRuntime,
    /// Current page URL — exposed as `window.location` inside JS.
    current_url: String,
    /// All @keyframes blocks from the last parsed stylesheets (for animation tick).
    keyframes: Vec<css::KeyframeSet>,
}

impl WebView {
    /// Create a new WebView with the given initial dimensions.
    pub fn new(w: u32, h: u32) -> Self {
        // Initialize the font renderer (idempotent — safe to call multiple times).
        libfont_client::init();

        let scroll_view = ui::ScrollView::new();
        scroll_view.set_size(w, h);

        let content_view = ui::View::new();
        content_view.set_size(w, h);
        content_view.set_color(0xFFFFFFFF); // white background
        scroll_view.add(&content_view);

        Self {
            scroll_view,
            content_view,
            renderer: renderer::Renderer::new(),
            dom_val: None,
            default_sheet: css::parse_stylesheet(DEFAULT_CSS),
            external_sheets: Vec::new(),
            images: ImageCache::new(),
            viewport_width: w as i32,
            total_height_val: 0,
            link_cb: None,
            link_cb_ud: 0,
            submit_cb: None,
            submit_cb_ud: 0,
            js_runtime: js::JsRuntime::new(),
            current_url: String::new(),
            keyframes: Vec::new(),
        }
    }

    /// Returns the ScrollView container (add this to your window).
    pub fn scroll_view(&self) -> &ui::ScrollView {
        &self.scroll_view
    }

    /// Returns the content View (all rendered controls are children of this).
    pub fn content_view(&self) -> &ui::View {
        &self.content_view
    }

    /// Set the raw link-click callback (extern "C" function pointer).
    /// The callback will be called with the control ID of the clicked label.
    pub fn set_link_callback(&mut self, cb: ui::Callback, userdata: u64) {
        self.link_cb = Some(cb);
        self.link_cb_ud = userdata;
    }

    /// Set the form-submit callback (extern "C" function pointer).
    /// The callback will be called with the control ID of the clicked submit button.
    pub fn set_submit_callback(&mut self, cb: ui::Callback, userdata: u64) {
        self.submit_cb = Some(cb);
        self.submit_cb_ud = userdata;
    }

    /// Set the current page URL.  Must be called before `set_html()` so that
    /// the JS environment has the correct `window.location` / `document.location`
    /// values when scripts run.
    pub fn set_url(&mut self, url: &str) {
        self.current_url = String::from(url);
    }

    /// Parse and cache an external CSS stylesheet.
    ///
    /// Parsing happens exactly once here.  Subsequent calls to `relayout()` reuse
    /// the pre-parsed form, which is orders of magnitude faster than re-parsing
    /// hundreds of kilobytes of CSS text on every image or resource load.
    pub fn add_stylesheet(&mut self, css_text: &str) {
        self.external_sheets.push(css::parse_stylesheet(css_text));
    }

    /// Clear all cached external stylesheets.
    pub fn clear_stylesheets(&mut self) {
        self.external_sheets.clear();
    }

    /// Add a decoded image to the cache. Will be displayed on next render.
    pub fn add_image(&mut self, src: &str, pixels: Vec<u32>, w: u32, h: u32) {
        self.images.add(String::from(src), pixels, w, h);
    }

    /// Set HTML content and render it.
    pub fn set_html(&mut self, html_text: &str) {
        debug_surf!("[webview] set_html: {} bytes input", html_text.len());
        #[cfg(feature = "debug_surf")]
        {
            let rsp0 = debug_rsp();
            let heap0 = debug_heap_pos();
            anyos_std::println!("[webview] set_html: RSP=0x{:X} heap=0x{:X}", rsp0, heap0);
        }

        // Parse HTML → DOM.
        debug_surf!("[webview] html::parse start");
        let mut parsed_dom = html::parse(html_text);
        debug_surf!("[webview] html::parse done: {} nodes", parsed_dom.nodes.len());
        #[cfg(feature = "debug_surf")]
        anyos_std::println!("[webview]   RSP=0x{:X} heap=0x{:X}", debug_rsp(), debug_heap_pos());

        // Collect stylesheets and resolve + layout + render.
        self.do_layout_and_render(&parsed_dom);

        // Execute JavaScript <script> tags after initial render so that DOM
        // elements already exist for querySelector / getElementById calls.
        debug_surf!("[webview] JS execute_scripts start");
        let url = self.current_url.clone();
        self.js_runtime.execute_scripts(&parsed_dom, &url);
        debug_surf!("[webview] JS execute_scripts done: {} console lines, {} mutations",
            self.js_runtime.console.len(), self.js_runtime.mutations.len());

        // Apply DOM mutations recorded during JS execution (e.g. React/Vue renders)
        // and re-layout so the mutated content becomes visible.
        if !self.js_runtime.mutations.is_empty() {
            debug_surf!("[webview] applying {} JS mutations + relayout", self.js_runtime.mutations.len());
            self.js_runtime.apply_mutations(&mut parsed_dom);
            self.do_layout_and_render(&parsed_dom);
        }

        // Store DOM for title queries etc.
        self.dom_val = Some(parsed_dom);
        debug_surf!("[webview] set_html complete");
    }

    /// Get the page title from the current DOM (if any).
    pub fn get_title(&self) -> Option<String> {
        self.dom_val.as_ref().and_then(|d| d.find_title())
    }

    /// Get the total document height in pixels.
    pub fn total_height(&self) -> i32 {
        self.total_height_val
    }

    /// Resize the viewport and re-layout.
    pub fn resize(&mut self, w: u32, h: u32) {
        self.viewport_width = w as i32;
        self.scroll_view.set_size(w, h);

        // If we have a DOM, re-layout.
        if self.dom_val.is_some() {
            self.relayout();
        }
    }

    /// Re-run layout and rendering with current DOM/stylesheets.
    pub fn relayout(&mut self) {
        // Need to temporarily take the DOM to avoid borrow conflict.
        if let Some(mut d) = self.dom_val.take() {
            // Apply any pending JS mutations before re-rendering.
            if !self.js_runtime.mutations.is_empty() {
                self.js_runtime.apply_mutations(&mut d);
            }
            self.do_layout_and_render(&d);
            self.dom_val = Some(d);
        }
    }

    /// Advance CSS animations/transitions and JS timers by `delta_ms` milliseconds.
    ///
    /// Returns `true` if any animation changed the document (relayout was performed).
    /// Call at ~60 fps when any page may have running animations.
    pub fn tick(&mut self, delta_ms: u64) -> bool {
        // ── 1. Advance JS timers (setTimeout / setInterval / requestAnimationFrame). ──
        if let Some(ref d) = self.dom_val.as_ref().map(|_| ()) {
            let _ = d; // borrow trick — we need to pass the dom
        }
        // We can't borrow dom_val and js_runtime simultaneously, so take dom temporarily.
        let dom_opt = self.dom_val.take();
        if let Some(ref d) = dom_opt {
            self.js_runtime.tick(d, delta_ms);
        }
        self.dom_val = dom_opt;

        // ── 2. Advance CSS animations. ──────────────────────────────────────────────
        // We pass keyframes by reference (they are stored in WebView).
        // advance_animations returns (any_active, overrides).
        // If there are no active animations, skip the expensive relayout.
        if self.js_runtime.active_animations.is_empty()
            && self.js_runtime.active_transitions.is_empty()
        {
            return false;
        }

        let (any_active, _overrides) =
            self.js_runtime.advance_animations(delta_ms, &self.keyframes);

        if any_active {
            // Re-layout with current overrides applied.
            // For simplicity we do a full relayout; a future optimisation could
            // apply only the overridden node styles.
            self.relayout();
            return true;
        }
        false
    }

    /// Clear all content (remove all controls, reset DOM).
    /// Used on full page navigation to destroy everything.
    pub fn clear(&mut self) {
        self.renderer.clear_all();
        self.dom_val = None;
        self.total_height_val = 0;
        self.content_view.set_size(self.viewport_width as u32, 1);
    }

    /// Access the current DOM (if set).
    pub fn dom(&self) -> Option<&dom::Dom> {
        self.dom_val.as_ref()
    }

    /// Look up the link URL for a control ID (used in click callbacks).
    ///
    /// If the control_id matches the canvas, performs a hit-test using the
    /// last mouse position to find the clicked link.
    pub fn link_url_for(&self, control_id: u32) -> Option<&str> {
        // Canvas click: hit-test at last mouse position.
        if let Some(canvas_id) = self.renderer.canvas_id() {
            if control_id == canvas_id {
                if let Some(ref canvas) = self.renderer.canvas_ref() {
                    let (mx, my, _) = canvas.get_mouse();
                    return self.renderer.hit_test_link(mx, my);
                }
            }
        }
        // Legacy: real control link_map lookup.
        self.renderer.link_map.iter()
            .find(|(id, _)| *id == control_id)
            .map(|(_, url)| url.as_str())
    }

    /// Check if a canvas click hit a submit button.  Returns the DOM node_id
    /// of the submit element, or None.
    pub fn canvas_submit_hit(&self, control_id: u32) -> Option<usize> {
        if let Some(canvas_id) = self.renderer.canvas_id() {
            if control_id == canvas_id {
                if let Some(ref canvas) = self.renderer.canvas_ref() {
                    let (mx, my, _) = canvas.get_mouse();
                    return self.renderer.hit_test_submit(mx, my);
                }
            }
        }
        None
    }

    /// Find the form action URL for a submit button identified by DOM node_id.
    /// Used for canvas-based submit hit regions.
    pub fn form_action_for_node(&self, node_id: usize) -> Option<(String, String)> {
        let dom = self.dom_val.as_ref()?;
        let mut cur = Some(node_id);
        while let Some(id) = cur {
            if dom.tag(id) == Some(dom::Tag::Form) {
                let action = dom.attr(id, "action").unwrap_or("");
                let method = dom.attr(id, "method").unwrap_or("GET");
                return Some((String::from(action), method.to_ascii_uppercase()));
            }
            cur = dom.get(id).parent;
        }
        None
    }

    /// Collect form data for a form containing the given DOM node_id.
    /// Used for canvas-based submit hit regions.
    pub fn collect_form_data_for_node(&self, node_id: usize) -> Vec<(String, String)> {
        let dom = match self.dom_val.as_ref() { Some(d) => d, None => return Vec::new() };

        // Find the parent <form> node.
        let mut form_node = None;
        let mut cur = Some(node_id);
        while let Some(id) = cur {
            if dom.tag(id) == Some(dom::Tag::Form) {
                form_node = Some(id);
                break;
            }
            cur = dom.get(id).parent;
        }
        let form_id = match form_node { Some(id) => id, None => return Vec::new() };

        // Collect all form controls that are descendants of this form.
        let mut data = Vec::new();
        for fc in &self.renderer.form_controls {
            let mut is_child = false;
            let mut up = Some(fc.node_id);
            while let Some(id) = up {
                if id == form_id { is_child = true; break; }
                up = dom.get(id).parent;
            }
            if !is_child { continue; }

            let name = dom.attr(fc.node_id, "name").unwrap_or("");
            if name.is_empty() { continue; }

            match fc.kind {
                FormFieldKind::TextInput | FormFieldKind::Password => {
                    if fc.control_id == 0 { continue; }
                    let ctrl = ui::Control::from_id(fc.control_id);
                    let mut buf = [0u8; 2048];
                    let len = ctrl.get_text(&mut buf);
                    let val = core::str::from_utf8(&buf[..len as usize]).unwrap_or("");
                    data.push((String::from(name), String::from(val)));
                }
                FormFieldKind::Checkbox => {
                    if fc.control_id == 0 { continue; }
                    let ctrl = ui::Control::from_id(fc.control_id);
                    if ctrl.get_state() != 0 {
                        let val = dom.attr(fc.node_id, "value").unwrap_or("on");
                        data.push((String::from(name), String::from(val)));
                    }
                }
                FormFieldKind::Radio => {
                    if fc.control_id == 0 { continue; }
                    let ctrl = ui::Control::from_id(fc.control_id);
                    if ctrl.get_state() != 0 {
                        let val = dom.attr(fc.node_id, "value").unwrap_or("");
                        data.push((String::from(name), String::from(val)));
                    }
                }
                FormFieldKind::Hidden => {
                    let val = dom.attr(fc.node_id, "value").unwrap_or("");
                    data.push((String::from(name), String::from(val)));
                }
                FormFieldKind::Textarea => {
                    if fc.control_id == 0 { continue; }
                    let ctrl = ui::Control::from_id(fc.control_id);
                    let mut buf = [0u8; 8192];
                    let len = ctrl.get_text(&mut buf);
                    let val = core::str::from_utf8(&buf[..len as usize]).unwrap_or("");
                    data.push((String::from(name), String::from(val)));
                }
                _ => {}
            }
        }
        data
    }

    /// Internal: collect stylesheets, resolve styles, layout, and render controls.
    fn do_layout_and_render(&mut self, d: &dom::Dom) {
        debug_surf!("[webview] do_layout_and_render: {} DOM nodes", d.nodes.len());

        // ── Stylesheet pipeline — parse once, reuse on every relayout ────────────
        //
        // `self.default_sheet` is parsed once in `WebView::new()`.
        // `self.external_sheets` are parsed once each in `add_stylesheet()`.
        // Only inline `<style>` blocks are re-parsed here because they live in the
        // mutable DOM and may be altered by JS mutations; they are typically tiny.
        //
        // This eliminates the catastrophic O(images × CSS-bytes) re-parse cost
        // visible in logs as repeated 150 KB parses per image load.

        // Phase A: Parse inline <style> blocks (small, DOM-dependent).
        let mut inline_sheets: Vec<css::Stylesheet> = Vec::new();
        let mut inline_count = 0u32;
        for (i, node) in d.nodes.iter().enumerate() {
            if let dom::NodeType::Element { tag: dom::Tag::Style, .. } = &node.node_type {
                let css_text = d.text_content(i);
                if !css_text.is_empty() {
                    debug_surf!("[webview] parse inline <style> #{}: {} bytes", inline_count, css_text.len());
                    inline_sheets.push(css::parse_stylesheet(&css_text));
                    inline_count += 1;
                }
            }
        }

        debug_surf!("[webview] total stylesheets: {} (1 default + {} external + {} inline)",
            1 + self.external_sheets.len() + inline_count as usize,
            self.external_sheets.len(), inline_count);
        #[cfg(feature = "debug_surf")]
        {
            let ext_rules: usize = self.external_sheets.iter().map(|s| s.rules.len()).sum();
            let inline_rules: usize = inline_sheets.iter().map(|s| s.rules.len()).sum();
            let total_rules = self.default_sheet.rules.len() + ext_rules + inline_rules;
            debug_surf!("[webview] total CSS rules: {}", total_rules);
            debug_surf!("[webview]   RSP=0x{:X} heap=0x{:X}", debug_rsp(), debug_heap_pos());
        }

        // Phase B: Collect @keyframes BEFORE building the borrowed `all_sheets` slice.
        // This avoids a borrow conflict: we need `&mut self.keyframes` here, but once
        // `all_sheets` holds `&self.default_sheet` / `&self.external_sheets` the borrow
        // checker considers those fields frozen for the lifetime of `all_sheets`.
        self.keyframes.clear();
        for kf in &self.default_sheet.keyframes {
            self.keyframes.retain(|k: &css::KeyframeSet| k.name != kf.name);
            self.keyframes.push(kf.clone());
        }
        for sheet in &self.external_sheets {
            for kf in &sheet.keyframes {
                self.keyframes.retain(|k: &css::KeyframeSet| k.name != kf.name);
                self.keyframes.push(kf.clone());
            }
        }
        for sheet in &inline_sheets {
            for kf in &sheet.keyframes {
                self.keyframes.retain(|k: &css::KeyframeSet| k.name != kf.name);
                self.keyframes.push(kf.clone());
            }
        }

        // Phase C: Resolve styles using zero-copy references to pre-parsed sheets.
        // `all_sheets` is scoped tightly: the borrows on `self.default_sheet` and
        // `self.external_sheets` are released as soon as `resolve_styles` returns,
        // allowing the subsequent mutable `self.xxx` calls to proceed freely.
        let vw = self.viewport_width;
        let vh = self.total_height_val.max(self.viewport_width);
        debug_surf!("[webview] resolve_styles start ({} nodes)", d.nodes.len());
        let styles = {
            let mut all_sheets: Vec<&css::Stylesheet> = Vec::with_capacity(
                1 + self.external_sheets.len() + inline_sheets.len()
            );
            all_sheets.push(&self.default_sheet);
            for sheet in &self.external_sheets { all_sheets.push(sheet); }
            for sheet in &inline_sheets { all_sheets.push(sheet); }
            style::resolve_styles(d, &all_sheets, vw, vh)
            // `all_sheets` (and its borrows) are dropped here.
        };
        debug_surf!("[webview] resolve_styles done: {} styles", styles.len());

        // Register new @keyframe animations for nodes that request them.
        self.js_runtime.start_animations(&styles);
        #[cfg(feature = "debug_surf")]
        debug_surf!("[webview]   RSP=0x{:X} heap=0x{:X}", debug_rsp(), debug_heap_pos());

        // Layout.
        debug_surf!("[webview] layout start (viewport_width={})", self.viewport_width);
        let root = layout::layout(d, &styles, self.viewport_width, &self.images);
        self.total_height_val = calc_total_height(&root);
        #[cfg(feature = "debug_surf")]
        {
            let box_count = count_layout_boxes(&root);
            debug_surf!("[webview] layout done: {} boxes, height={}", box_count, self.total_height_val);
            debug_surf!("[webview]   RSP=0x{:X} heap=0x{:X}", debug_rsp(), debug_heap_pos());
        }

        // Soft-clear: reset hit regions and mark form controls for GC.
        // Canvas and form controls persist across relayouts.
        self.renderer.clear();

        // Sync content view background to the body element's CSS background-color.
        let body_id = d.find_body().unwrap_or(0);
        let body_bg = styles.get(body_id).map(|s| s.background_color).unwrap_or(0);
        let bg_color = if body_bg != 0 { body_bg } else { 0xFFFFFFFF };
        self.content_view.set_color(bg_color);

        // Set content view height to document height.
        let doc_w = self.viewport_width as u32;
        let doc_h = (self.total_height_val as u32).max(1);
        self.content_view.set_size(doc_w, doc_h);

        // Render into canvas + update form controls.
        debug_surf!("[webview] renderer start");
        self.renderer.render(
            &root,
            &self.content_view,
            &self.images,
            doc_w,
            doc_h,
            bg_color,
            self.link_cb,
            self.link_cb_ud,
            self.submit_cb,
            self.submit_cb_ud,
        );
        debug_surf!("[webview] renderer done: {} form_controls", self.renderer.control_count());
        #[cfg(feature = "debug_surf")]
        debug_surf!("[webview]   RSP=0x{:X} heap=0x{:X}", debug_rsp(), debug_heap_pos());
    }

    /// Access the JS runtime (e.g. for evaluating additional scripts or reading console).
    pub fn js_runtime(&mut self) -> &mut js::JsRuntime {
        &mut self.js_runtime
    }

    /// Get console output from JavaScript execution.
    pub fn js_console(&self) -> &[String] {
        self.js_runtime.get_console()
    }

    /// Get all rendered form controls (for form submission).
    pub fn form_controls(&self) -> &[FormControl] {
        &self.renderer.form_controls
    }

    /// Check if a control ID belongs to a submit button (real control or canvas hit).
    pub fn is_submit_button(&self, control_id: u32) -> bool {
        // Canvas hit-test for submit regions.
        if self.canvas_submit_hit(control_id).is_some() {
            return true;
        }
        // Legacy: real control lookup.
        self.renderer.form_controls.iter().any(|fc| {
            fc.control_id == control_id
                && matches!(fc.kind, FormFieldKind::Submit | FormFieldKind::ButtonEl)
        })
    }

    /// Find the form action URL for a submit button click.
    /// Handles both real controls and canvas-based submit hit regions.
    pub fn form_action_for(&self, control_id: u32) -> Option<(String, String)> {
        // Canvas hit-test for submit regions.
        if let Some(node_id) = self.canvas_submit_hit(control_id) {
            return self.form_action_for_node(node_id);
        }
        // Legacy: real control lookup.
        let dom = self.dom_val.as_ref()?;
        let fc = self.renderer.form_controls.iter().find(|fc| fc.control_id == control_id)?;
        let mut cur = Some(fc.node_id);
        while let Some(id) = cur {
            if dom.tag(id) == Some(dom::Tag::Form) {
                let action = dom.attr(id, "action").unwrap_or("");
                let method = dom.attr(id, "method").unwrap_or("GET");
                return Some((String::from(action), method.to_ascii_uppercase()));
            }
            cur = dom.get(id).parent;
        }
        None
    }

    /// Collect form data (name=value pairs) for the form containing `control_id`.
    /// Handles both real controls and canvas-based submit hit regions.
    pub fn collect_form_data(&self, control_id: u32) -> Vec<(String, String)> {
        // Canvas hit-test for submit regions.
        if let Some(node_id) = self.canvas_submit_hit(control_id) {
            return self.collect_form_data_for_node(node_id);
        }
        // Legacy: real control lookup.
        let dom = match self.dom_val.as_ref() { Some(d) => d, None => return Vec::new() };
        let fc = match self.renderer.form_controls.iter().find(|fc| fc.control_id == control_id) {
            Some(f) => f,
            None => return Vec::new(),
        };
        self.collect_form_data_for_node(fc.node_id)
    }
}

/// Count total layout boxes in the tree (debug only).
#[cfg(feature = "debug_surf")]
fn count_layout_boxes(root: &LayoutBox) -> usize {
    let mut count = 1usize;
    for child in &root.children {
        count += count_layout_boxes(child);
    }
    count
}

/// Calculate total document height from the root layout box.
/// Fixed-position boxes are excluded — they are viewport-anchored and do not
/// contribute to the scrollable document height.
fn calc_total_height(root: &LayoutBox) -> i32 {
    let bottom = root.y + root.height;
    let mut max = bottom;
    for child in &root.children {
        if child.is_fixed { continue; }
        let ch = child_total_height(child, root.y);
        if ch > max {
            max = ch;
        }
    }
    max
}

fn child_total_height(bx: &LayoutBox, parent_y: i32) -> i32 {
    let abs_y = parent_y + bx.y;
    let bottom = abs_y + bx.height;
    let mut max = bottom;
    for child in &bx.children {
        if child.is_fixed { continue; }
        let ch = child_total_height(child, abs_y);
        if ch > max {
            max = ch;
        }
    }
    max
}

/// Browser default CSS (minimal reset + sensible defaults).
const DEFAULT_CSS: &str = "
body { margin: 8px; font-size: 16px; color: #000; }
h1 { font-size: 32px; font-weight: bold; margin: 21px 0; }
h2 { font-size: 24px; font-weight: bold; margin: 19px 0; }
h3 { font-size: 19px; font-weight: bold; margin: 18px 0; }
h4 { font-size: 16px; font-weight: bold; margin: 21px 0; }
h5 { font-size: 13px; font-weight: bold; margin: 22px 0; }
h6 { font-size: 11px; font-weight: bold; margin: 24px 0; }
p { margin: 16px 0; }
ul, ol { margin: 16px 0; padding-left: 40px; }
li { margin: 4px 0; }
a { color: #0066cc; text-decoration: underline; }
pre, code { font-family: monospace; }
pre { margin: 16px 0; padding: 8px; background: #f5f5f5; }
blockquote { margin: 16px 0; padding-left: 16px; border-left: 4px solid #ddd; }
hr { margin: 16px 0; border: none; border-top: 1px solid #ccc; }
table { border-collapse: collapse; }
td, th { padding: 4px 8px; }
img { max-width: 100%; }
strong, b { font-weight: bold; }
em, i { font-style: italic; }
";
