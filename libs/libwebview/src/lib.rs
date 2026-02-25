//! libwebview — HTML rendering library for anyOS.
//!
//! Renders HTML content using real libanyui controls (Labels, Views,
//! ImageViews, TextFields, etc.) positioned by a CSS layout engine.
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

pub use renderer::{ImageCache, ImageEntry, FormControl};
pub use layout::{LayoutBox, FormFieldKind};

/// A WebView renders HTML content inside a ScrollView using libanyui controls.
pub struct WebView {
    scroll_view: ui::ScrollView,
    content_view: ui::View,
    renderer: renderer::Renderer,
    dom_val: Option<dom::Dom>,
    /// External CSS text (added via add_stylesheet), re-parsed on each render.
    external_css: Vec<String>,
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
            external_css: Vec::new(),
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

    /// Add an external CSS stylesheet (as text). Applied on next `set_html()` or `relayout()`.
    pub fn add_stylesheet(&mut self, css_text: &str) {
        self.external_css.push(String::from(css_text));
    }

    /// Clear all added stylesheets.
    pub fn clear_stylesheets(&mut self) {
        self.external_css.clear();
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
        let parsed_dom = html::parse(html_text);
        debug_surf!("[webview] html::parse done: {} nodes", parsed_dom.nodes.len());
        #[cfg(feature = "debug_surf")]
        anyos_std::println!("[webview]   RSP=0x{:X} heap=0x{:X}", debug_rsp(), debug_heap_pos());

        // Collect stylesheets and resolve + layout + render.
        self.do_layout_and_render(&parsed_dom);

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
        if let Some(d) = self.dom_val.take() {
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
    pub fn clear(&mut self) {
        self.renderer.clear();
        self.dom_val = None;
        self.total_height_val = 0;
        self.content_view.set_size(self.viewport_width as u32, 1);
    }

    /// Access the current DOM (if set).
    pub fn dom(&self) -> Option<&dom::Dom> {
        self.dom_val.as_ref()
    }

    /// Look up the link URL for a control ID (used in click callbacks).
    pub fn link_url_for(&self, control_id: u32) -> Option<&str> {
        self.renderer.link_map.iter()
            .find(|(id, _)| *id == control_id)
            .map(|(_, url)| url.as_str())
    }

    /// Internal: collect stylesheets, resolve styles, layout, and render controls.
    fn do_layout_and_render(&mut self, d: &dom::Dom) {
        debug_surf!("[webview] do_layout_and_render: {} DOM nodes", d.nodes.len());

        // Collect all stylesheets.
        let mut all_sheets: Vec<css::Stylesheet> = Vec::new();

        // Browser default stylesheet.
        all_sheets.push(css::parse_stylesheet(DEFAULT_CSS));

        // External stylesheets (added via add_stylesheet).
        for (idx, css_text) in self.external_css.iter().enumerate() {
            debug_surf!("[webview] parse external stylesheet #{}: {} bytes", idx, css_text.len());
            all_sheets.push(css::parse_stylesheet(css_text));
        }

        // Inline <style> elements from DOM.
        let mut inline_count = 0u32;
        for (i, node) in d.nodes.iter().enumerate() {
            if let dom::NodeType::Element { tag: dom::Tag::Style, .. } = &node.node_type {
                let css_text = d.text_content(i);
                if !css_text.is_empty() {
                    debug_surf!("[webview] parse inline <style> #{}: {} bytes", inline_count, css_text.len());
                    all_sheets.push(css::parse_stylesheet(&css_text));
                    inline_count += 1;
                }
            }
        }

        debug_surf!("[webview] total stylesheets: {} (1 default + {} external + {} inline)",
            all_sheets.len(), self.external_css.len(), inline_count);
        #[cfg(feature = "debug_surf")]
        {
            let total_rules: usize = all_sheets.iter().map(|s| s.rules.len()).sum();
            debug_surf!("[webview] total CSS rules: {}", total_rules);
            debug_surf!("[webview]   RSP=0x{:X} heap=0x{:X}", debug_rsp(), debug_heap_pos());
        }

        // Collect all @keyframes from all stylesheets for use by the animation tick.
        self.keyframes.clear();
        for sheet in &all_sheets {
            for kf in &sheet.keyframes {
                // Only keep the last definition for each name (CSS spec behaviour).
                self.keyframes.retain(|k: &css::KeyframeSet| k.name != kf.name);
                self.keyframes.push(css::KeyframeSet {
                    name: kf.name.clone(),
                    stops: kf.stops.iter().map(|s| css::KeyframeStop {
                        offset: s.offset,
                        declarations: s.declarations.clone(),
                    }).collect(),
                });
            }
        }

        // Resolve styles (pass viewport dimensions for @media queries).
        debug_surf!("[webview] resolve_styles start ({} nodes)", d.nodes.len());
        let vh = self.total_height_val.max(self.viewport_width); // approximate viewport height
        let styles = style::resolve_styles(d, &all_sheets, self.viewport_width, vh);
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

        // Clear old controls.
        self.renderer.clear();

        // Set content view height to document height.
        let content_h = (self.total_height_val as u32).max(1);
        self.content_view.set_size(self.viewport_width as u32, content_h);

        // Render new controls.
        debug_surf!("[webview] renderer start");
        self.renderer.render(
            &root,
            &self.content_view,
            &self.images,
            self.link_cb,
            self.link_cb_ud,
            self.submit_cb,
            self.submit_cb_ud,
        );
        debug_surf!("[webview] renderer done: {} controls", self.renderer.control_count());
        #[cfg(feature = "debug_surf")]
        debug_surf!("[webview]   RSP=0x{:X} heap=0x{:X}", debug_rsp(), debug_heap_pos());

        // Execute JavaScript <script> tags.
        debug_surf!("[webview] JS execute_scripts start");
        self.js_runtime.execute_scripts(d, &self.current_url);
        debug_surf!("[webview] JS execute_scripts done: {} console lines, {} mutations",
            self.js_runtime.console.len(), self.js_runtime.mutations.len());
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

    /// Check if a control ID belongs to a submit button.
    pub fn is_submit_button(&self, control_id: u32) -> bool {
        self.renderer.form_controls.iter().any(|fc| {
            fc.control_id == control_id
                && matches!(fc.kind, FormFieldKind::Submit | FormFieldKind::ButtonEl)
        })
    }

    /// Find the form action URL for a submit button click.
    /// Walks up the DOM from the button to find the parent `<form>` and its action attribute.
    pub fn form_action_for(&self, control_id: u32) -> Option<(String, String)> {
        let dom = self.dom_val.as_ref()?;
        let fc = self.renderer.form_controls.iter().find(|fc| fc.control_id == control_id)?;
        // Walk up to find parent <form>.
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
    /// Reads current values from the libanyui TextFields/Checkboxes.
    pub fn collect_form_data(&self, control_id: u32) -> Vec<(String, String)> {
        let dom = match self.dom_val.as_ref() { Some(d) => d, None => return Vec::new() };

        // Find the parent <form> node.
        let fc = match self.renderer.form_controls.iter().find(|fc| fc.control_id == control_id) {
            Some(f) => f,
            None => return Vec::new(),
        };
        let mut form_node = None;
        let mut cur = Some(fc.node_id);
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
            // Check if this control is a descendant of form_id.
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
                    let ctrl = ui::Control::from_id(fc.control_id);
                    let mut buf = [0u8; 2048];
                    let len = ctrl.get_text(&mut buf);
                    let val = core::str::from_utf8(&buf[..len as usize]).unwrap_or("");
                    data.push((String::from(name), String::from(val)));
                }
                FormFieldKind::Checkbox => {
                    let ctrl = ui::Control::from_id(fc.control_id);
                    if ctrl.get_state() != 0 {
                        let val = dom.attr(fc.node_id, "value").unwrap_or("on");
                        data.push((String::from(name), String::from(val)));
                    }
                }
                FormFieldKind::Radio => {
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
fn calc_total_height(root: &LayoutBox) -> i32 {
    let bottom = root.y + root.height;
    let mut max = bottom;
    for child in &root.children {
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
