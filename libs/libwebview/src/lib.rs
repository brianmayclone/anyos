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

pub mod dom;
pub mod html;
pub mod css;
pub mod style;
pub mod layout;
mod renderer;

use alloc::string::String;
use alloc::vec::Vec;

use libanyui_client::{self as ui};

pub use renderer::{ImageCache, ImageEntry};
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
        // Parse HTML → DOM.
        let parsed_dom = html::parse(html_text);

        // Collect stylesheets and resolve + layout + render.
        self.do_layout_and_render(&parsed_dom);

        // Store DOM for title queries etc.
        self.dom_val = Some(parsed_dom);
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
        // Collect all stylesheets.
        let mut all_sheets: Vec<css::Stylesheet> = Vec::new();

        // Browser default stylesheet.
        all_sheets.push(css::parse_stylesheet(DEFAULT_CSS));

        // External stylesheets (added via add_stylesheet).
        for css_text in &self.external_css {
            all_sheets.push(css::parse_stylesheet(css_text));
        }

        // Inline <style> elements from DOM.
        for (i, node) in d.nodes.iter().enumerate() {
            if let dom::NodeType::Element { tag: dom::Tag::Style, .. } = &node.node_type {
                let css_text = d.text_content(i);
                if !css_text.is_empty() {
                    all_sheets.push(css::parse_stylesheet(&css_text));
                }
            }
        }

        // Resolve styles.
        let styles = style::resolve_styles(d, &all_sheets);

        // Layout.
        let root = layout::layout(d, &styles, self.viewport_width);
        self.total_height_val = calc_total_height(&root);

        // Clear old controls.
        self.renderer.clear();

        // Set content view height to document height.
        let content_h = (self.total_height_val as u32).max(1);
        self.content_view.set_size(self.viewport_width as u32, content_h);

        // Render new controls.
        self.renderer.render(
            &root,
            &self.content_view,
            &self.images,
            self.link_cb,
            self.link_cb_ud,
        );
    }
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
