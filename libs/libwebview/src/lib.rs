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

#![cfg_attr(not(feature = "host"), no_std)]

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
    unsafe {
        core::arch::asm!("mov {}, rsp", out(reg) rsp);
    }
    rsp
}

/// Return current heap break position for debug tracing.
#[cfg(feature = "debug_surf")]
pub fn debug_heap_pos() -> u64 {
    // sbrk(0) returns current break without changing it.
    anyos_std::process::sbrk(0) as u64
}

pub mod css;
pub mod dom;
pub mod html;
pub mod js;
pub mod layout;
mod renderer;
pub mod style;

use alloc::string::String;
use alloc::vec::Vec;

use libanyui_client::{self as ui};

pub use layout::{FormFieldKind, LayoutBox};
pub use renderer::{FormControl, HitKind, ImageCache, ImageEntry};
use style::{Display, FloatVal, Position, TextAlignVal};

#[derive(Clone, Copy, PartialEq, Eq)]
enum MutationImpact {
    None,
    Paint,
    LayoutReuseStyles,
    LayoutRestyle,
}

struct IncrementalRelayoutPlan {
    parent_node: usize,
    target_nodes: Vec<usize>,
    rebuild_parent_children: bool,
}

/// A WebView renders HTML content inside a ScrollView using libanyui controls.
///
/// Uses viewport-based tile rendering: only the visible area (plus a buffer zone)
/// is drawn into the canvas.  On scroll, the tile is re-rendered from the cached
/// layout tree without a full CSS resolve or relayout.
/// Global web font map pointer — set before layout, read by the renderer.
/// Points to the current WebView's web_fonts Vec. Only valid during relayout.
static mut WEB_FONT_MAP: *const Vec<(String, u32)> = core::ptr::null();

/// Look up a web font ID by family name (called from renderer/layout).
/// `family` may be a single name or a comma-separated CSS font-family list
/// like `"Georgia, 'Times New Roman', serif"`.
/// Tries each name in order; returns the first registered match.
pub fn lookup_web_font(family: &str) -> Option<u32> {
    unsafe {
        if WEB_FONT_MAP.is_null() {
            return None;
        }
        let map = &*WEB_FONT_MAP;
        // Try the whole string first (fastest path for single-name entries).
        let lower = family.to_ascii_lowercase();
        if let Some(id) = map
            .iter()
            .find(|(f, _)| f.as_str() == lower.as_str())
            .map(|(_, id)| *id)
        {
            return Some(id);
        }
        // Parse comma-separated list and try each entry.
        for part in lower.split(',') {
            let name = part.trim().trim_matches('\'').trim_matches('"').trim();
            if name.is_empty() {
                continue;
            }
            if let Some(id) = map
                .iter()
                .find(|(f, _)| f.as_str() == name)
                .map(|(_, id)| *id)
            {
                return Some(id);
            }
        }
        None
    }
}

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
    /// Cached inline `<style>` blocks — parsed once in `set_html()`, reused on relayout.
    /// Invalidated only by `set_html()` (new page) or JS mutations that alter `<style>` tags.
    inline_sheets: Vec<css::Stylesheet>,
    /// Whether inline sheets need re-parsing (set by JS mutations, cleared after parse).
    inline_sheets_dirty: bool,
    /// Whether the keyframes collection needs rebuilding from sheets.
    keyframes_dirty: bool,
    /// Cached parsed inline `style="..."` declarations per node_id.
    /// Avoids re-parsing the same style attribute on every relayout.
    inline_style_cache: Vec<Option<Vec<css::Declaration>>>,
    pub images: ImageCache,
    viewport_width: i32,
    /// Viewport height in pixels (visible ScrollView area).
    viewport_height: u32,
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
    /// Cached layout tree for scroll re-renders (avoids full relayout on scroll).
    layout_root: Option<LayoutBox>,
    /// Scroll Y of the last rendered tile (for hysteresis / re-render threshold).
    last_render_scroll_y: i32,
    /// Cached body background color for scroll re-renders.
    bg_color_cached: u32,
    /// True while the renderer still has buffered/offscreen tiles to fill in.
    pending_tiles: bool,
    /// Web font mapping: (family_name_lowercase, font_id from libfont).
    web_fonts: Vec<(String, u32)>,
    /// Previous resolved styles — kept for CSS transition change detection.
    prev_styles: Vec<style::ComputedStyle>,
    /// Last fully resolved styles used for layout. Reused for safe local DOM
    /// mutations that do not require a global restyle pass.
    resolved_styles_cache: Vec<style::ComputedStyle>,
    /// Viewport width used when `resolved_styles_cache` was built.
    resolved_styles_viewport_width: i32,
    /// Viewport height used when `resolved_styles_cache` was built.
    resolved_styles_viewport_height: u32,
    /// Pseudo styles that match `resolved_styles_cache`.
    resolved_pseudo_styles: style::PseudoStyles,
    /// Pending animation/transition overrides to apply during the next relayout.
    /// Each entry is (node_id, declarations_to_overlay).
    anim_overrides: Vec<(usize, Vec<css::Declaration>)>,
    /// Per-node scroll offsets set via JS `element.scrollTop`/`scrollLeft`.
    /// Stored as (node_id, scroll_top, scroll_left).  Applied to LayoutBoxes
    /// after layout so overflow containers shift their children.
    scroll_offsets: Vec<(usize, i32, i32)>,
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

        let mut webview = Self {
            scroll_view,
            content_view,
            renderer: renderer::Renderer::new(),
            dom_val: None,
            default_sheet: css::parse_stylesheet(DEFAULT_CSS),
            external_sheets: Vec::new(),
            inline_sheets: Vec::new(),
            inline_sheets_dirty: true,
            keyframes_dirty: true,
            inline_style_cache: Vec::new(),
            images: ImageCache::new(),
            viewport_width: w as i32,
            viewport_height: h,
            total_height_val: 0,
            link_cb: None,
            link_cb_ud: 0,
            submit_cb: None,
            submit_cb_ud: 0,
            js_runtime: js::JsRuntime::new(),
            current_url: String::new(),
            keyframes: Vec::new(),
            layout_root: None,
            last_render_scroll_y: 0,
            bg_color_cached: 0xFFFFFFFF,
            pending_tiles: false,
            web_fonts: Vec::new(),
            prev_styles: Vec::new(),
            resolved_styles_cache: Vec::new(),
            resolved_styles_viewport_width: 0,
            resolved_styles_viewport_height: 0,
            resolved_pseudo_styles: style::PseudoStyles::empty(0),
            anim_overrides: Vec::new(),
            scroll_offsets: Vec::new(),
        };
        webview.js_runtime.set_viewport(w, h);
        webview
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
        self.add_parsed_stylesheet(css::parse_stylesheet(css_text));
    }

    /// Cache an already parsed external CSS stylesheet.
    pub fn add_parsed_stylesheet(&mut self, sheet: css::Stylesheet) {
        self.external_sheets.push(sheet);
        self.keyframes_dirty = true;
    }

    /// Return `@import` URLs from the most recently added external stylesheet.
    /// The caller should fetch these and add them as additional stylesheets.
    pub fn last_stylesheet_imports(&self) -> &[String] {
        if let Some(sheet) = self.external_sheets.last() {
            &sheet.imports
        } else {
            &[]
        }
    }

    /// Return `@font-face` rules from the most recently added external stylesheet.
    pub fn last_stylesheet_font_faces(&self) -> &[css::FontFaceRule] {
        if let Some(sheet) = self.external_sheets.last() {
            &sheet.font_faces
        } else {
            &[]
        }
    }

    /// Register a web font loaded from @font-face.
    /// `family` is the CSS font-family name (will be lowercased for matching).
    /// `font_id` is the ID returned by `libfont_client::load_data()`.
    pub fn register_web_font(&mut self, family: &str, font_id: u32) {
        let lower = family.to_ascii_lowercase();
        // Replace existing entry for the same family.
        if let Some(existing) = self.web_fonts.iter_mut().find(|(f, _)| f == &lower) {
            existing.1 = font_id;
        } else {
            self.web_fonts.push((lower, font_id));
        }
    }

    /// Look up a web font ID by family name. Returns None if not registered.
    pub fn web_font_id(&self, family: &str) -> Option<u32> {
        let lower = family.to_ascii_lowercase();
        self.web_fonts
            .iter()
            .find(|(f, _)| f == &lower)
            .map(|(_, id)| *id)
    }

    /// Return all `@font-face` rules across all stylesheets (inline + external + default).
    pub fn all_font_faces(&self) -> Vec<&css::FontFaceRule> {
        let mut result = Vec::new();
        for sheet in &self.external_sheets {
            for ff in &sheet.font_faces {
                result.push(ff);
            }
        }
        for sheet in &self.inline_sheets {
            for ff in &sheet.font_faces {
                result.push(ff);
            }
        }
        result
    }

    /// Clear all cached external and inline stylesheets.
    pub fn clear_stylesheets(&mut self) {
        self.external_sheets.clear();
        self.inline_sheets.clear();
        self.inline_sheets_dirty = true;
        self.keyframes_dirty = true;
    }

    /// Full cleanup for page navigation.
    /// Clears DOM, layout, images, renderer controls, stylesheets, web fonts,
    /// and resets the JS runtime so the new page starts with a clean slate.
    pub fn navigate_clear(&mut self) {
        // Clear rendered UI controls.
        self.renderer.clear_all();
        // Clear cached images.
        self.images.clear();
        // Clear DOM and layout tree.
        self.dom_val = None;
        self.layout_root = None;
        self.total_height_val = 0;
        self.last_render_scroll_y = 0;
        self.pending_tiles = false;
        self.content_view.set_size(self.viewport_width as u32, 1);
        // Clear all stylesheets (external + inline).
        self.external_sheets.clear();
        self.inline_sheets.clear();
        self.inline_sheets_dirty = true;
        self.keyframes_dirty = true;
        self.inline_style_cache.clear();
        self.resolved_styles_cache.clear();
        self.resolved_styles_viewport_width = 0;
        self.resolved_styles_viewport_height = 0;
        self.resolved_pseudo_styles = style::PseudoStyles::empty(0);
        // Clear web fonts from the previous page.
        self.web_fonts.clear();
        // Reset JS runtime (fresh engine, no timers/listeners/websockets).
        self.js_runtime.reset();
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

        // Clear per-node scroll offsets from previous page.
        self.scroll_offsets.clear();

        // Parse HTML → DOM.
        debug_surf!("[webview] html::parse start");
        let mut parsed_dom = html::parse(html_text);
        debug_surf!(
            "[webview] html::parse done: {} nodes",
            parsed_dom.nodes.len()
        );
        #[cfg(feature = "debug_surf")]
        anyos_std::println!(
            "[webview]   RSP=0x{:X} heap=0x{:X}",
            debug_rsp(),
            debug_heap_pos()
        );

        // NOTE: We do NOT replace "client-nojs" with "client-js" on the <html>
        // element.  Modern sites (Wikipedia, etc.) use client-nojs as a fallback
        // path designed for browsers without JS — which matches our capabilities
        // better than client-js, which expects full JS-controlled UI toggling.

        // New page — inline <style> blocks and style attribute cache need re-parsing.
        self.inline_sheets.clear();
        self.inline_sheets_dirty = true;
        self.inline_style_cache.clear();

        // Collect stylesheets and resolve + layout + render.
        self.do_layout_and_render(&parsed_dom, None);

        // Execute JavaScript <script> tags after initial render so that DOM
        // elements already exist for querySelector / getElementById calls.
        debug_surf!("[webview] JS execute_scripts start");
        let url = self.current_url.clone();
        self.js_runtime.execute_scripts(&parsed_dom, &url);
        debug_surf!(
            "[webview] JS execute_scripts done: {} console lines, {} mutations",
            self.js_runtime.console.len(),
            self.js_runtime.mutations.len()
        );

        // Apply DOM mutations recorded during JS execution (e.g. React/Vue renders)
        // and re-layout so the mutated content becomes visible.
        if !self.js_runtime.mutations.is_empty() {
            debug_surf!(
                "[webview] applying {} JS mutations after initial script run",
                self.js_runtime.mutations.len()
            );
            self.flush_pending_mutations(&mut parsed_dom);
        }

        // Store DOM for title queries etc.
        self.dom_val = Some(parsed_dom);
        debug_surf!("[webview] set_html complete");
    }

    /// Set HTML content and render — but skip JavaScript execution.
    ///
    /// Use this when the host wants to control JS execution timing
    /// (e.g. load external resources first, then run JS).
    /// After calling this, use [`script_entries`], [`execute_js`], etc.
    pub fn set_html_no_js(&mut self, html_text: &str) {
        debug_surf!("[webview] set_html_no_js: {} bytes input", html_text.len());

        // Parse HTML → DOM.
        let mut parsed_dom = html::parse(html_text);

        // NOTE: We do NOT replace "client-nojs" with "client-js" — see comment
        // in set_html() above for rationale.

        // New page — inline <style> blocks and style attribute cache need re-parsing.
        self.inline_sheets.clear();
        self.inline_sheets_dirty = true;
        self.inline_style_cache.clear();

        // Layout and render (no JS).
        self.do_layout_and_render(&parsed_dom, None);

        // Store DOM.
        self.dom_val = Some(parsed_dom);
    }

    /// Collect script entries from the current DOM in document order.
    ///
    /// Returns [`js::ScriptEntry::Inline`] for inline scripts and
    /// [`js::ScriptEntry::External`] for `<script src="...">` tags.
    /// The host should fetch external URLs and pass the resolved texts
    /// to [`execute_js`].
    pub fn script_entries(&self) -> Vec<js::ScriptEntry> {
        match &self.dom_val {
            Some(d) => js::JsRuntime::collect_script_entries(d),
            None => Vec::new(),
        }
    }

    /// Execute JavaScript scripts and apply DOM mutations.
    ///
    /// `scripts` should contain the text of each script to execute, in
    /// document order (resolved from [`script_entries`]).
    pub fn execute_js(&mut self, scripts: &[String]) {
        let mut dom = match self.dom_val.take() {
            Some(d) => d,
            None => return,
        };

        let url = self.current_url.clone();
        self.js_runtime.execute_script_sources(&dom, &url, scripts);

        // Apply DOM mutations and re-layout.
        if !self.js_runtime.mutations.is_empty() {
            self.flush_pending_mutations(&mut dom);
        }

        self.dom_val = Some(dom);
    }

    /// Run JS timers for `delta_ms` milliseconds and apply any resulting mutations.
    /// Returns `true` if any timer fired (and thus mutations may have occurred).
    pub fn run_timers(&mut self, delta_ms: u64) -> bool {
        let dom = match self.dom_val.take() {
            Some(d) => d,
            None => return false,
        };

        let fired = self.js_runtime.tick(&dom, delta_ms);

        // Apply mutations if timers fired.
        let mut dom = dom;
        if !self.js_runtime.mutations.is_empty() {
            self.flush_pending_mutations(&mut dom);
        }

        self.dom_val = Some(dom);
        fired > 0
    }

    /// Take pending HTTP requests from JavaScript (fetch/XHR).
    pub fn take_pending_http_requests(&mut self) -> Vec<js::PendingHttpRequest> {
        self.js_runtime.take_pending_http_requests()
    }

    /// Check if there are active JS timers.
    pub fn has_timers(&self) -> bool {
        !self.js_runtime.timers.is_empty()
    }

    /// Number of active JS timers.
    pub fn timer_count(&self) -> usize {
        self.js_runtime.timers.len()
    }

    /// Get the page title from the current DOM (if any).
    pub fn get_title(&self) -> Option<String> {
        self.dom_val.as_ref().and_then(|d| d.find_title())
    }

    /// Get the total document height in pixels.
    pub fn total_height(&self) -> i32 {
        self.total_height_val
    }

    /// Get the viewport height in pixels.
    pub fn viewport_height(&self) -> u32 {
        self.viewport_height
    }

    /// Get the viewport width in pixels.
    pub fn viewport_width(&self) -> u32 {
        self.viewport_width.max(0) as u32
    }

    /// Return the approximate document-space bounds for a DOM node from the
    /// cached layout tree.
    pub fn node_bounds(&self, node_id: usize) -> Option<(i32, i32, i32, i32)> {
        self.layout_root
            .as_ref()
            .and_then(|root| find_node_bounds(root, 0, 0, node_id))
    }

    /// Render tiles for the given scroll position (public wrapper).
    /// Returns `true` if there are pending tiles not yet rasterized.
    pub fn render_viewport_at(&mut self, scroll_y: i32) -> bool {
        self.render_viewport(scroll_y)
    }

    /// Resize the viewport and re-layout.
    pub fn resize(&mut self, w: u32, h: u32) {
        // Skip if dimensions haven't changed — avoids redundant relayouts.
        if self.viewport_width == w as i32 && self.viewport_height == h {
            return;
        }
        self.viewport_width = w as i32;
        self.viewport_height = h;
        self.js_runtime.set_viewport(w, h);
        self.scroll_view.set_size(w, h);

        // If we have a DOM, re-layout (invalidates cached layout tree).
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
                match self.flush_pending_mutations(&mut d) {
                    MutationImpact::LayoutReuseStyles | MutationImpact::LayoutRestyle => {}
                    MutationImpact::Paint | MutationImpact::None => {
                        self.dom_val = Some(d);
                        return;
                    }
                }
            } else if self.can_reuse_cached_styles_for_full_relayout(&d) {
                self.do_layout_and_render_with_cached_styles(&d, None);
            } else {
                self.do_layout_and_render(&d, None);
            }
            self.dom_val = Some(d);
        }
    }

    /// Repaint the current document from the cached layout tree without
    /// recomputing style or layout.
    ///
    /// This is the fast path for late image arrivals where geometry is already
    /// stable and only the visible tiles need fresh pixels.
    pub fn repaint_from_cached_layout(&mut self) {
        let root = match self.layout_root.as_ref() {
            Some(root) => root,
            None => return,
        };

        let doc_w = self.viewport_width.max(1) as u32;
        let doc_h = (self.total_height_val as u32).max(1);
        let scroll_y = self.scroll_view.get_state() as i32;
        let bg_color = if self.bg_color_cached != 0 {
            self.bg_color_cached
        } else {
            0xFFFFFFFF
        };

        self.renderer.clear();
        self.pending_tiles = self.renderer.render(
            root,
            &self.content_view,
            &self.images,
            doc_w,
            doc_h,
            self.viewport_height,
            scroll_y,
            bg_color,
            self.link_cb,
            self.link_cb_ud,
            self.submit_cb,
            self.submit_cb_ud,
        );
        self.last_render_scroll_y = scroll_y;
    }

    /// Advance CSS animations/transitions, JS timers, and scroll-based tile
    /// creation by `delta_ms` milliseconds.
    ///
    /// Returns `true` if any visual change occurred or pending tiles remain.
    pub fn tick(&mut self, delta_ms: u64) -> bool {
        let mut changed = false;

        // ── 1. Advance JS timers (setTimeout / setInterval / requestAnimationFrame). ──
        // Short-circuits internally when no timers exist (zero allocation).
        if !self.js_runtime.timers.is_empty() {
            let dom_opt = self.dom_val.take();
            if let Some(ref d) = dom_opt {
                self.js_runtime.tick(d, delta_ms);
            }
            self.dom_val = dom_opt;
        }

        // ── 2. CSS animations & transitions ──────────────────────────────────────
        if !self.js_runtime.active_animations.is_empty()
            || !self.js_runtime.active_transitions.is_empty()
        {
            let (any_active, overrides) = self
                .js_runtime
                .advance_animations(delta_ms, &self.keyframes);
            if !overrides.is_empty() {
                // Store overrides; they will be applied on top of computed
                // styles inside `do_layout_and_render`.
                self.anim_overrides = overrides;
                self.relayout();
                self.anim_overrides.clear();
                changed = true;
            }
            if any_active && !changed {
                // Even if no overrides this tick (e.g. in delay phase), keep
                // the animation loop alive.
                changed = true;
            }
        }

        // ── 3. Scroll-based tile management (compositor-driven). ─────────────────
        // Per-tile canvases are positioned in the content_view.  The compositor
        // handles smooth scrolling natively.  We only need to create tile
        // canvases for rows entering the pre-render zone (incrementally, max
        // 2 per tick to avoid blocking the event loop).
        //
        // When pending tiles remain, we signal changed=true so the anim timer
        // keeps running until all visible tiles are rasterized.  The per-tick
        // limit (MAX_TILES_PER_TICK) prevents blocking the event loop.
        if self.layout_root.is_some() {
            let scroll_y = self.scroll_view.get_state() as i32;
            let delta = (scroll_y - self.last_render_scroll_y).abs();
            if delta > 4 || self.pending_tiles {
                let pending = self.render_viewport(scroll_y);
                self.last_render_scroll_y = scroll_y;
                self.pending_tiles = pending;
                changed = true;
                if pending {
                    changed = true;
                }
            }
        }

        changed
    }

    /// Ensure tile canvases exist for the visible viewport range.
    ///
    /// Uses the fast scroll path: only creates canvases for rows not yet
    /// present.  Cache-miss tiles are rasterized incrementally (max 2 per
    /// call).  Returns `true` if there are still pending tiles.
    fn render_viewport(&mut self, scroll_y: i32) -> bool {
        // The display list is stored in the renderer — no layout_root needed
        // for scroll rendering.  We still pass root for API compatibility but
        // render_scroll ignores it (uses the display list instead).
        let root = match self.layout_root {
            Some(ref root) => root as *const LayoutBox,
            None => return false,
        };
        let doc_w = self.viewport_width as u32;
        let doc_h = (self.total_height_val as u32).max(1);

        // SAFETY: root points into self.layout_root which is not modified during render_scroll().
        unsafe {
            self.renderer.render_scroll(
                &*root,
                &self.content_view,
                &self.images,
                doc_w,
                doc_h,
                self.viewport_height,
                scroll_y,
                self.bg_color_cached,
                self.link_cb,
                self.link_cb_ud,
            )
        }
    }

    /// Clear all content (remove all controls, reset DOM).
    /// Used on full page navigation to destroy everything.
    pub fn clear(&mut self) {
        self.renderer.clear_all();
        self.images.clear();
        self.dom_val = None;
        self.layout_root = None;
        self.total_height_val = 0;
        self.last_render_scroll_y = 0;
        self.pending_tiles = false;
        self.content_view.set_size(self.viewport_width as u32, 1);
    }

    /// Access the current DOM (if set).
    pub fn dom(&self) -> Option<&dom::Dom> {
        self.dom_val.as_ref()
    }

    /// Look up the link URL for a control ID (used in click callbacks).
    ///
    /// If the control_id matches any tile canvas, performs a hit-test using
    /// the mouse position translated to document coordinates.
    pub fn link_url_for(&self, control_id: u32) -> Option<&str> {
        // Tile canvas click: translate mouse to document coords and hit-test.
        if let Some((mx, doc_y)) = self.renderer.tile_hit_coords(control_id) {
            return self.renderer.hit_test_link_at(mx, doc_y);
        }
        // Legacy: real control link_map lookup.
        self.renderer
            .link_map
            .iter()
            .find(|(id, _)| *id == control_id)
            .map(|(_, url)| url.as_str())
    }

    /// Check if a canvas click hit a submit button.  Returns the DOM node_id
    /// of the submit element, or None.
    pub fn canvas_submit_hit(&self, control_id: u32) -> Option<usize> {
        if let Some((mx, doc_y)) = self.renderer.tile_hit_coords(control_id) {
            return self.renderer.hit_test_submit_at(mx, doc_y);
        }
        None
    }

    // ── Viewport-coordinate hit-test helpers (for host/surf-host) ────────────

    /// Find the link URL at viewport position (vx, vy) given a scroll offset.
    /// Returns the URL string if a link was hit, else None.
    pub fn hit_test_link_viewport(&self, vx: i32, vy: i32, scroll_y: i32) -> Option<&str> {
        self.renderer.hit_test_link_at(vx, scroll_y + vy)
    }

    /// Find the submit-button DOM node_id at viewport position (vx, vy).
    pub fn hit_test_submit_viewport(&self, vx: i32, vy: i32, scroll_y: i32) -> Option<usize> {
        self.renderer.hit_test_submit_at(vx, scroll_y + vy)
    }

    /// Find the form control (TextInput / Textarea) at viewport position (vx, vy).
    /// Returns the control_id of the matching form control, or None.
    pub fn hit_test_form_control_viewport(&self, vx: i32, vy: i32, scroll_y: i32) -> Option<u32> {
        self.renderer.hit_test_form_at(vx, scroll_y + vy)
    }

    /// Check if a canvas click hit a form control (TextInput/Textarea).
    /// If so, focus the control and return true.
    pub fn focus_form_control_at_canvas(&self, canvas_ctrl_id: u32) -> bool {
        if let Some((mx, doc_y)) = self.renderer.tile_hit_coords(canvas_ctrl_id) {
            if let Some(fc_id) = self.renderer.hit_test_form_at(mx, doc_y) {
                ui::Control::from_id(fc_id).focus();
                return true;
            }
        }
        false
    }

    /// Read the text content of a form control by its control_id.
    pub fn get_form_control_text(&self, control_id: u32) -> String {
        let ctrl = ui::Control::from_id(control_id);
        let mut buf = [0u8; 4096];
        let len = ctrl.get_text(&mut buf);
        String::from(core::str::from_utf8(&buf[..len as usize]).unwrap_or(""))
    }

    /// Set the text content of a form control by its control_id.
    pub fn set_form_control_text(&self, control_id: u32, text: &str) {
        ui::Control::from_id(control_id).set_text(text);
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
        let dom = match self.dom_val.as_ref() {
            Some(d) => d,
            None => return Vec::new(),
        };

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
        let form_id = match form_node {
            Some(id) => id,
            None => return Vec::new(),
        };

        // Collect all form controls that are descendants of this form.
        let mut data = Vec::new();
        for fc in &self.renderer.form_controls {
            let mut is_child = false;
            let mut up = Some(fc.node_id);
            while let Some(id) = up {
                if id == form_id {
                    is_child = true;
                    break;
                }
                up = dom.get(id).parent;
            }
            if !is_child {
                continue;
            }

            let name = dom.attr(fc.node_id, "name").unwrap_or("");
            if name.is_empty() {
                continue;
            }

            match fc.kind {
                FormFieldKind::TextInput | FormFieldKind::Password => {
                    if fc.control_id == 0 {
                        continue;
                    }
                    let ctrl = ui::Control::from_id(fc.control_id);
                    let mut buf = [0u8; 2048];
                    let len = ctrl.get_text(&mut buf);
                    let val = core::str::from_utf8(&buf[..len as usize]).unwrap_or("");
                    data.push((String::from(name), String::from(val)));
                }
                FormFieldKind::Checkbox => {
                    if fc.control_id == 0 {
                        continue;
                    }
                    let ctrl = ui::Control::from_id(fc.control_id);
                    if ctrl.get_state() != 0 {
                        let val = dom.attr(fc.node_id, "value").unwrap_or("on");
                        data.push((String::from(name), String::from(val)));
                    }
                }
                FormFieldKind::Radio => {
                    if fc.control_id == 0 {
                        continue;
                    }
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
                    if fc.control_id == 0 {
                        continue;
                    }
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

    /// Extract scroll offset mutations from the pending mutation list and merge
    /// them into `self.scroll_offsets`.  Must be called *before* `apply_mutations`
    /// which consumes the mutation vec via `mem::take`.
    fn extract_scroll_offsets(&mut self) {
        for m in &self.js_runtime.mutations {
            match m {
                js::DomMutation::SetScrollTop { node_id, value } => {
                    if let Some(entry) = self
                        .scroll_offsets
                        .iter_mut()
                        .find(|(id, _, _)| *id == *node_id)
                    {
                        entry.1 = *value;
                    } else {
                        self.scroll_offsets.push((*node_id, *value, 0));
                    }
                }
                js::DomMutation::SetScrollLeft { node_id, value } => {
                    if let Some(entry) = self
                        .scroll_offsets
                        .iter_mut()
                        .find(|(id, _, _)| *id == *node_id)
                    {
                        entry.2 = *value;
                    } else {
                        self.scroll_offsets.push((*node_id, 0, *value));
                    }
                }
                _ => {}
            }
        }
    }

    /// Recursively apply stored scroll offsets to LayoutBoxes that match node IDs.
    fn apply_scroll_offsets_to_layout(offsets: &[(usize, i32, i32)], bx: &mut layout::LayoutBox) {
        if let Some(nid) = bx.node_id {
            if bx.overflow_hidden {
                if let Some(&(_, st, sl)) = offsets.iter().find(|(id, _, _)| *id == nid) {
                    bx.scroll_top = st;
                    bx.scroll_left = sl;
                }
            }
        }
        for child in &mut bx.children {
            Self::apply_scroll_offsets_to_layout(offsets, child);
        }
    }

    fn classify_pending_mutations(&self) -> MutationImpact {
        let mut impact = MutationImpact::None;
        for m in &self.js_runtime.mutations {
            match m {
                js::DomMutation::SetCookie { .. } => {}
                js::DomMutation::SetScrollTop { .. } | js::DomMutation::SetScrollLeft { .. } => {
                    if impact == MutationImpact::None {
                        impact = MutationImpact::Paint;
                    }
                }
                js::DomMutation::SetStyleProperty { property, .. } => {
                    if Self::style_property_is_paint_only(property) {
                        if impact == MutationImpact::None {
                            impact = MutationImpact::Paint;
                        }
                    } else if Self::style_property_can_reuse_cached_styles(property) {
                        impact = MutationImpact::LayoutReuseStyles;
                    } else {
                        return MutationImpact::LayoutRestyle;
                    }
                }
                js::DomMutation::SetAttribute { name, .. }
                | js::DomMutation::RemoveAttribute { name, .. } => {
                    impact = if Self::attribute_change_requires_style_recalc(name) {
                        MutationImpact::LayoutRestyle
                    } else {
                        MutationImpact::LayoutReuseStyles
                    };
                    if impact == MutationImpact::LayoutRestyle {
                        return impact;
                    }
                }
                js::DomMutation::RemoveNode { .. } => impact = MutationImpact::LayoutReuseStyles,
                _ => return MutationImpact::LayoutRestyle,
            }
        }
        impact
    }

    fn attribute_change_requires_style_recalc(name: &str) -> bool {
        matches!(
            name.trim().to_ascii_lowercase().as_str(),
            "class" | "id" | "style" | "hidden" | "align" | "type"
        )
    }

    fn mutations_dirty_inline_style_cache(mutations: &[js::DomMutation]) -> bool {
        mutations.iter().any(|m| match m {
            js::DomMutation::SetStyleProperty { .. } => true,
            js::DomMutation::SetAttribute { name, .. }
            | js::DomMutation::RemoveAttribute { name, .. } => {
                name.trim().eq_ignore_ascii_case("style")
            }
            _ => false,
        })
    }

    fn can_reuse_cached_styles_for_mutations(
        &self,
        dom: &dom::Dom,
        mutations: &[js::DomMutation],
    ) -> bool {
        if self.resolved_styles_cache.len() != dom.nodes.len()
            || self.resolved_pseudo_styles.before.len() != dom.nodes.len()
            || self.resolved_pseudo_styles.after.len() != dom.nodes.len()
        {
            return false;
        }
        mutations.iter().all(|m| match m {
            js::DomMutation::RemoveNode { .. } => true,
            js::DomMutation::SetStyleProperty { property, .. } => {
                Self::style_property_can_reuse_cached_styles(property)
            }
            js::DomMutation::SetAttribute { name, .. }
            | js::DomMutation::RemoveAttribute { name, .. } => {
                !Self::attribute_change_requires_style_recalc(name)
            }
            _ => false,
        })
    }

    fn style_property_can_reuse_cached_styles(property: &str) -> bool {
        matches!(
            property.trim().to_ascii_lowercase().as_str(),
            "display"
                | "width"
                | "height"
                | "min-width"
                | "max-width"
                | "min-height"
                | "max-height"
                | "margin"
                | "margin-top"
                | "margin-right"
                | "margin-bottom"
                | "margin-left"
                | "padding"
                | "padding-top"
                | "padding-right"
                | "padding-bottom"
                | "padding-left"
                | "border"
                | "border-top"
                | "border-right"
                | "border-bottom"
                | "border-left"
                | "border-width"
                | "border-style"
                | "border-radius"
                | "border-top-width"
                | "border-right-width"
                | "border-bottom-width"
                | "border-left-width"
                | "border-top-style"
                | "border-right-style"
                | "border-bottom-style"
                | "border-left-style"
                | "box-sizing"
                | "overflow"
                | "overflow-x"
                | "overflow-y"
                | "position"
                | "top"
                | "right"
                | "bottom"
                | "left"
                | "z-index"
                | "float"
                | "clear"
                | "flex"
                | "flex-grow"
                | "flex-shrink"
                | "flex-basis"
                | "flex-direction"
                | "flex-wrap"
                | "justify-content"
                | "align-items"
                | "align-self"
                | "align-content"
                | "order"
                | "gap"
                | "row-gap"
                | "column-gap"
                | "grid-template"
                | "grid-template-rows"
                | "grid-template-columns"
                | "grid-template-areas"
        )
    }

    fn style_property_is_paint_only(property: &str) -> bool {
        matches!(
            property.trim().to_ascii_lowercase().as_str(),
            "color"
                | "background"
                | "background-color"
                | "opacity"
                | "visibility"
                | "border-color"
                | "border-top-color"
                | "border-right-color"
                | "border-bottom-color"
                | "border-left-color"
                | "outline-color"
                | "text-decoration-color"
        )
    }

    fn parse_opacity_value(value: &str) -> Option<i32> {
        let s = value.trim();
        if s.is_empty() {
            return None;
        }
        if let Some(percent) = s.strip_suffix('%') {
            let p = percent.trim().parse::<i32>().ok()?;
            return Some(((p * 255) / 100).clamp(0, 255));
        }
        let f = s.parse::<f32>().ok()?;
        Some((f.clamp(0.0, 1.0) * 255.0) as i32)
    }

    fn apply_paint_only_mutation_to_layout(mutation: &js::DomMutation, bx: &mut LayoutBox) {
        let js::DomMutation::SetStyleProperty {
            node_id,
            property,
            value,
        } = mutation
        else {
            return;
        };

        let property = property.trim().to_ascii_lowercase();
        if bx.node_id == Some(*node_id as usize) {
            match property.as_str() {
                "color" => {
                    if let Some(c) = crate::css::try_parse_color_pub(value) {
                        bx.color = c;
                    }
                }
                "background" | "background-color" => {
                    if value.trim().eq_ignore_ascii_case("transparent") {
                        bx.bg_color = 0;
                    } else if let Some(c) = crate::css::try_parse_color_pub(value) {
                        bx.bg_color = c;
                    }
                }
                "opacity" => {
                    if let Some(opacity) = Self::parse_opacity_value(value) {
                        bx.opacity = opacity;
                    }
                }
                "visibility" => {
                    bx.visibility_hidden = matches!(
                        value.trim().to_ascii_lowercase().as_str(),
                        "hidden" | "collapse"
                    );
                }
                "border-color" => {
                    if let Some(c) = crate::css::try_parse_color_pub(value) {
                        bx.border_color = c;
                        bx.border_top_color = c;
                        bx.border_right_color = c;
                        bx.border_bottom_color = c;
                        bx.border_left_color = c;
                    }
                }
                "border-top-color" => {
                    if let Some(c) = crate::css::try_parse_color_pub(value) {
                        bx.border_top_color = c;
                    }
                }
                "border-right-color" => {
                    if let Some(c) = crate::css::try_parse_color_pub(value) {
                        bx.border_right_color = c;
                    }
                }
                "border-bottom-color" => {
                    if let Some(c) = crate::css::try_parse_color_pub(value) {
                        bx.border_bottom_color = c;
                    }
                }
                "border-left-color" => {
                    if let Some(c) = crate::css::try_parse_color_pub(value) {
                        bx.border_left_color = c;
                    }
                }
                "outline-color" => {
                    if let Some(c) = crate::css::try_parse_color_pub(value) {
                        bx.outline_color = c;
                    }
                }
                "text-decoration-color" => {
                    if let Some(c) = crate::css::try_parse_color_pub(value) {
                        bx.text_decoration_color = c;
                    }
                }
                _ => {}
            }
        }

        for child in &mut bx.children {
            Self::apply_paint_only_mutation_to_layout(mutation, child);
        }
    }

    fn apply_paint_only_mutations_to_layout(mutations: &[js::DomMutation], root: &mut LayoutBox) {
        for mutation in mutations {
            Self::apply_paint_only_mutation_to_layout(mutation, root);
        }
    }

    fn mutation_target_node_id(mutation: &js::DomMutation) -> Option<usize> {
        match mutation {
            js::DomMutation::SetAttribute { node_id, .. }
            | js::DomMutation::RemoveAttribute { node_id, .. }
            | js::DomMutation::SetTextContent { node_id, .. }
            | js::DomMutation::SetInnerHTML { node_id, .. }
            | js::DomMutation::SetStyleProperty { node_id, .. }
            | js::DomMutation::RemoveNode { node_id } => usize::try_from(*node_id).ok(),
            js::DomMutation::SetScrollTop { node_id, .. }
            | js::DomMutation::SetScrollLeft { node_id, .. } => Some(*node_id),
            _ => None,
        }
    }

    fn mutation_allows_incremental_layout(mutation: &js::DomMutation) -> bool {
        matches!(
            mutation,
            js::DomMutation::SetAttribute { .. }
                | js::DomMutation::RemoveAttribute { .. }
                | js::DomMutation::SetTextContent { .. }
                | js::DomMutation::SetStyleProperty { .. }
                | js::DomMutation::SetInnerHTML { .. }
                | js::DomMutation::RemoveNode { .. }
        )
    }

    fn mutation_requires_parent_rebuild(mutation: &js::DomMutation) -> bool {
        matches!(
            mutation,
            js::DomMutation::SetInnerHTML { .. } | js::DomMutation::RemoveNode { .. }
        )
    }

    fn node_supports_incremental_layout(style: &style::ComputedStyle) -> bool {
        style.float == FloatVal::None
            && !matches!(style.position, Position::Absolute | Position::Fixed)
            && matches!(
                style.display,
                Display::Block
                    | Display::FlowRoot
                    | Display::Flex
                    | Display::InlineFlex
                    | Display::Grid
                    | Display::InlineGrid
                    | Display::ListItem
                    | Display::TableRow
                    | Display::TableCell
            )
    }

    fn nearest_incremental_reflow_candidate(
        dom: &dom::Dom,
        styles: &[style::ComputedStyle],
        node_id: usize,
    ) -> Option<(usize, usize)> {
        if node_id >= dom.nodes.len() {
            return None;
        }
        let mut cur = Some(node_id);
        while let Some(id) = cur {
            if id < styles.len() && Self::node_supports_incremental_layout(&styles[id]) {
                let parent = dom.nodes.get(id).and_then(|n| n.parent)?;
                if Self::parent_supports_incremental_child_reflow(dom, styles, parent) {
                    return Some((id, parent));
                }
            }
            cur = dom.nodes.get(id).and_then(|n| n.parent);
        }
        None
    }

    fn build_incremental_relayout_plan(
        dom: &dom::Dom,
        styles: &[style::ComputedStyle],
        mutations: &[js::DomMutation],
    ) -> Option<IncrementalRelayoutPlan> {
        let mut single_target = None;
        let mut single_parent = None;
        let mut target_nodes = Vec::new();
        let mut rebuild_parent_children = false;

        for mutation in mutations {
            if !Self::mutation_allows_incremental_layout(mutation) {
                return None;
            }
            let node_id = Self::mutation_target_node_id(mutation)?;
            let (candidate, parent) =
                Self::nearest_incremental_reflow_candidate(dom, styles, node_id)?;
            match (single_target, single_parent) {
                (Some(existing_target), Some(existing_parent))
                    if existing_target != candidate || existing_parent != parent =>
                {
                    if existing_parent != parent {
                        return None;
                    }
                }
                (None, None) => {
                    single_target = Some(candidate);
                    single_parent = Some(parent);
                }
                _ => {}
            }
            if !target_nodes.contains(&candidate) {
                target_nodes.push(candidate);
            }
            if Self::mutation_requires_parent_rebuild(mutation) {
                rebuild_parent_children = true;
            }
        }

        Some(IncrementalRelayoutPlan {
            parent_node: single_parent?,
            target_nodes,
            rebuild_parent_children,
        })
    }

    fn parent_supports_incremental_child_reflow(
        dom: &dom::Dom,
        styles: &[style::ComputedStyle],
        parent_node: usize,
    ) -> bool {
        if parent_node >= styles.len() {
            return false;
        }
        let parent_style = &styles[parent_node];
        if !matches!(
            parent_style.display,
            Display::Block | Display::FlowRoot | Display::ListItem
        ) {
            return false;
        }
        dom.get(parent_node).children.iter().all(|&child| {
            if child >= styles.len() {
                return false;
            }
            let st = &styles[child];
            st.display == Display::None
                || (Self::node_supports_incremental_layout(st) && st.display != Display::Contents)
        })
    }

    fn reflow_target_child_in_parent(
        &mut self,
        dom: &dom::Dom,
        styles: &[style::ComputedStyle],
        pseudo_styles: &style::PseudoStyles,
        parent_box: &mut LayoutBox,
        parent_node: usize,
        target_node: usize,
    ) -> bool {
        if !Self::parent_supports_incremental_child_reflow(dom, styles, parent_node) {
            return false;
        }

        let Some(target_index) = parent_box
            .children
            .iter()
            .position(|child| child.node_id == Some(target_node))
        else {
            return false;
        };

        let content_width = parent_box.width
            - parent_box.padding.left
            - parent_box.padding.right
            - parent_box.border_left_width
            - parent_box.border_right_width;
        if content_width <= 0 {
            return false;
        }

        let old_child = &parent_box.children[target_index];
        let old_flow_extent = old_child.y + old_child.height + old_child.margin.bottom;
        let mut new_child = layout::block::build_block(
            dom,
            styles,
            pseudo_styles,
            target_node,
            content_width,
            &self.images,
            self.viewport_width,
            parent_box.height,
        );
        if !self.scroll_offsets.is_empty() {
            Self::apply_scroll_offsets_to_layout(&self.scroll_offsets, &mut new_child);
        }

        let parent_style = &styles[parent_node];
        let base_x = parent_box.border_left_width + parent_box.padding.left;
        new_child.x = base_x + new_child.margin.left;
        let total_child_w = new_child.width + new_child.margin.left + new_child.margin.right;
        if parent_style.text_align == TextAlignVal::Center {
            if total_child_w < content_width {
                new_child.x = base_x + (content_width - total_child_w) / 2;
            }
        } else if parent_style.text_align == TextAlignVal::Right {
            if total_child_w < content_width {
                new_child.x = base_x + content_width - total_child_w;
            }
        }

        new_child.y = if target_index == 0 {
            parent_box.border_top_width + parent_box.padding.top + new_child.margin.top
        } else {
            let prev = &parent_box.children[target_index - 1];
            let collapsed = core::cmp::max(prev.margin.bottom, new_child.margin.top);
            prev.y + prev.height + collapsed
        };

        let new_flow_extent = new_child.y + new_child.height + new_child.margin.bottom;
        let delta = new_flow_extent - old_flow_extent;
        parent_box.children[target_index] = new_child;

        if delta != 0 {
            for sibling in parent_box.children.iter_mut().skip(target_index + 1) {
                if sibling.is_fixed || sibling.is_out_of_flow {
                    continue;
                }
                sibling.y += delta;
            }
            parent_box.height = (parent_box.height + delta).max(1);
        }

        true
    }

    fn reflow_target_children_in_parent(
        &mut self,
        dom: &dom::Dom,
        styles: &[style::ComputedStyle],
        pseudo_styles: &style::PseudoStyles,
        parent_box: &mut LayoutBox,
        parent_node: usize,
        target_nodes: &[usize],
    ) -> bool {
        if target_nodes.is_empty()
            || !Self::parent_supports_incremental_child_reflow(dom, styles, parent_node)
        {
            return false;
        }

        let mut targets: Vec<(usize, usize)> = Vec::new();
        for &target_node in target_nodes {
            let Some(index) = parent_box
                .children
                .iter()
                .position(|child| child.node_id == Some(target_node))
            else {
                return false;
            };
            targets.push((index, target_node));
        }
        targets.sort_by_key(|(index, _)| *index);

        let mut changed = false;
        for (_, target_node) in targets {
            if self.reflow_target_child_in_parent(
                dom,
                styles,
                pseudo_styles,
                parent_box,
                parent_node,
                target_node,
            ) {
                changed = true;
            } else {
                return false;
            }
        }
        changed
    }

    fn relayout_child_box_in_parent(
        &mut self,
        dom: &dom::Dom,
        styles: &[style::ComputedStyle],
        pseudo_styles: &style::PseudoStyles,
        parent_box: &LayoutBox,
        parent_node: usize,
        child_node: usize,
        prev_sibling: Option<&LayoutBox>,
    ) -> Option<LayoutBox> {
        if child_node >= styles.len() {
            return None;
        }
        let child_style = &styles[child_node];
        if matches!(child_style.display, Display::None | Display::Contents) {
            return None;
        }

        let content_width = parent_box.width
            - parent_box.padding.left
            - parent_box.padding.right
            - parent_box.border_left_width
            - parent_box.border_right_width;
        if content_width <= 0 {
            return None;
        }

        let mut child = layout::block::build_block(
            dom,
            styles,
            pseudo_styles,
            child_node,
            content_width,
            &self.images,
            self.viewport_width,
            parent_box.height,
        );
        if !self.scroll_offsets.is_empty() {
            Self::apply_scroll_offsets_to_layout(&self.scroll_offsets, &mut child);
        }

        let parent_style = &styles[parent_node];
        let base_x = parent_box.border_left_width + parent_box.padding.left;
        child.x = base_x + child.margin.left;
        let total_child_w = child.width + child.margin.left + child.margin.right;
        if parent_style.text_align == TextAlignVal::Center {
            if total_child_w < content_width {
                child.x = base_x + (content_width - total_child_w) / 2;
            }
        } else if parent_style.text_align == TextAlignVal::Right {
            if total_child_w < content_width {
                child.x = base_x + content_width - total_child_w;
            }
        }

        child.y = if let Some(prev) = prev_sibling {
            let collapsed = core::cmp::max(prev.margin.bottom, child.margin.top);
            prev.y + prev.height + collapsed
        } else {
            parent_box.border_top_width + parent_box.padding.top + child.margin.top
        };

        Some(child)
    }

    fn rebuild_parent_children_in_place(
        &mut self,
        dom: &dom::Dom,
        styles: &[style::ComputedStyle],
        pseudo_styles: &style::PseudoStyles,
        parent_box: &mut LayoutBox,
        parent_node: usize,
    ) -> bool {
        if !Self::parent_supports_incremental_child_reflow(dom, styles, parent_node) {
            return false;
        }

        let old_children_bottom = parent_box
            .children
            .last()
            .map(|child| child.y + child.height + child.margin.bottom)
            .unwrap_or(parent_box.border_top_width + parent_box.padding.top);
        let min_tail = parent_box.border_bottom_width + parent_box.padding.bottom;
        let noncontent_tail = (parent_box.height - old_children_bottom).max(min_tail);

        let mut new_children = Vec::new();
        for &child_node in &dom.get(parent_node).children {
            let prev = new_children.last();
            if let Some(child) = self.relayout_child_box_in_parent(
                dom,
                styles,
                pseudo_styles,
                parent_box,
                parent_node,
                child_node,
                prev,
            ) {
                new_children.push(child);
            }
        }

        let new_children_bottom = new_children
            .last()
            .map(|child| child.y + child.height + child.margin.bottom)
            .unwrap_or(parent_box.border_top_width + parent_box.padding.top);
        parent_box.children = new_children;
        parent_box.height = (new_children_bottom + noncontent_tail).max(1);
        true
    }

    fn apply_incremental_layout_for_node_in_box(
        &mut self,
        dom: &dom::Dom,
        styles: &[style::ComputedStyle],
        pseudo_styles: &style::PseudoStyles,
        bx: &mut LayoutBox,
        target_node: usize,
    ) -> bool {
        let target_parent = dom.nodes.get(target_node).and_then(|n| n.parent);
        if let Some(parent_node) = target_parent {
            if bx.node_id == Some(parent_node)
                && self.reflow_target_child_in_parent(
                    dom,
                    styles,
                    pseudo_styles,
                    bx,
                    parent_node,
                    target_node,
                )
            {
                return true;
            }
        }

        for child in &mut bx.children {
            if self.apply_incremental_layout_for_node_in_box(
                dom,
                styles,
                pseudo_styles,
                child,
                target_node,
            ) {
                return true;
            }
        }
        false
    }

    fn apply_incremental_layout_for_targets_in_box(
        &mut self,
        dom: &dom::Dom,
        styles: &[style::ComputedStyle],
        pseudo_styles: &style::PseudoStyles,
        bx: &mut LayoutBox,
        parent_node: usize,
        target_nodes: &[usize],
        rebuild_parent_children: bool,
    ) -> bool {
        if bx.node_id == Some(parent_node) {
            if rebuild_parent_children
                && self.rebuild_parent_children_in_place(
                    dom,
                    styles,
                    pseudo_styles,
                    bx,
                    parent_node,
                )
            {
                return true;
            }
            if self.reflow_target_children_in_parent(
                dom,
                styles,
                pseudo_styles,
                bx,
                parent_node,
                target_nodes,
            ) {
                return true;
            }
        }

        for child in &mut bx.children {
            if self.apply_incremental_layout_for_targets_in_box(
                dom,
                styles,
                pseudo_styles,
                child,
                parent_node,
                target_nodes,
                rebuild_parent_children,
            ) {
                return true;
            }
        }
        false
    }

    fn apply_incremental_layout_for_node(
        &mut self,
        dom: &dom::Dom,
        styles: &[style::ComputedStyle],
        pseudo_styles: &style::PseudoStyles,
        target_node: usize,
    ) -> bool {
        let Some(mut root) = self.layout_root.take() else {
            return false;
        };

        if !self.apply_incremental_layout_for_node_in_box(
            dom,
            styles,
            pseudo_styles,
            &mut root,
            target_node,
        ) {
            self.layout_root = Some(root);
            return false;
        }

        layout::compute_subtree_bottom(&mut root);
        self.total_height_val = calc_total_height(&root);
        self.layout_root = Some(root);
        self.refresh_render_surface_for_layout(dom, styles);
        true
    }

    fn apply_incremental_relayout_plan(
        &mut self,
        dom: &dom::Dom,
        styles: &[style::ComputedStyle],
        pseudo_styles: &style::PseudoStyles,
        plan: &IncrementalRelayoutPlan,
    ) -> bool {
        let Some(mut root) = self.layout_root.take() else {
            return false;
        };

        if !self.apply_incremental_layout_for_targets_in_box(
            dom,
            styles,
            pseudo_styles,
            &mut root,
            plan.parent_node,
            &plan.target_nodes,
            plan.rebuild_parent_children,
        ) {
            self.layout_root = Some(root);
            return false;
        }

        layout::compute_subtree_bottom(&mut root);
        self.total_height_val = calc_total_height(&root);
        self.layout_root = Some(root);
        self.refresh_render_surface_for_layout(dom, styles);
        true
    }

    fn refresh_render_surface_for_layout(
        &mut self,
        dom: &dom::Dom,
        styles: &[style::ComputedStyle],
    ) {
        let body_id = dom.find_body().unwrap_or(0);
        let body_bg = styles.get(body_id).map(|s| s.background_color).unwrap_or(0);
        let bg_color = if body_bg != 0 { body_bg } else { 0xFFFFFFFF };
        self.content_view.set_color(bg_color);
        self.bg_color_cached = bg_color;
        self.content_view.set_size(
            self.viewport_width.max(1) as u32,
            (self.total_height_val as u32).max(1),
        );
        self.repaint_from_cached_layout();
    }

    fn resolved_style_cache_matches_dom(&self, dom: &dom::Dom) -> bool {
        self.resolved_styles_cache.len() == dom.nodes.len()
            && self.resolved_pseudo_styles.before.len() == dom.nodes.len()
            && self.resolved_pseudo_styles.after.len() == dom.nodes.len()
            && self.resolved_styles_viewport_width == self.viewport_width
            && self.resolved_styles_viewport_height == self.viewport_height
    }

    fn can_reuse_cached_styles_for_full_relayout(&self, dom: &dom::Dom) -> bool {
        !self.inline_sheets_dirty
            && !self.keyframes_dirty
            && self.anim_overrides.is_empty()
            && self.resolved_style_cache_matches_dom(dom)
    }

    fn update_resolved_style_cache(
        &mut self,
        styles: &[style::ComputedStyle],
        pseudo_styles: &style::PseudoStyles,
    ) {
        self.resolved_styles_cache.clear();
        self.resolved_styles_cache.extend_from_slice(styles);
        self.resolved_pseudo_styles.clone_from(pseudo_styles);
        self.resolved_styles_viewport_width = self.viewport_width;
        self.resolved_styles_viewport_height = self.viewport_height;
    }

    fn rebuild_keyframes_if_dirty(&mut self) {
        if !self.keyframes_dirty {
            return;
        }
        self.keyframes.clear();
        for kf in &self.default_sheet.keyframes {
            self.keyframes.push(kf.clone());
        }
        for sheet in &self.external_sheets {
            for kf in &sheet.keyframes {
                self.keyframes.push(kf.clone());
            }
        }
        for sheet in &self.inline_sheets {
            for kf in &sheet.keyframes {
                self.keyframes.push(kf.clone());
            }
        }
        self.keyframes_dirty = false;
    }

    fn apply_animation_overrides_to_styles(
        &self,
        d: &dom::Dom,
        styles: &mut [style::ComputedStyle],
    ) {
        if self.anim_overrides.is_empty() {
            return;
        }

        let root_fs = if !styles.is_empty() {
            styles[0].font_size
        } else {
            16
        };
        for (node_id, decls) in &self.anim_overrides {
            if *node_id < styles.len() {
                let parent_fs = {
                    let pid = d.nodes.get(*node_id).and_then(|n| n.parent).unwrap_or(0);
                    if pid < styles.len() {
                        styles[pid].font_size
                    } else {
                        root_fs
                    }
                };
                for decl in decls {
                    style::apply_declaration(&mut styles[*node_id], decl, parent_fs, root_fs);
                }
            }
        }
    }

    fn do_layout_and_render_with_cached_styles(
        &mut self,
        d: &dom::Dom,
        incremental_mutations: Option<&[js::DomMutation]>,
    ) {
        if !self.resolved_style_cache_matches_dom(d) {
            self.do_layout_and_render(d, incremental_mutations);
            return;
        }

        if let Some(mutations) = incremental_mutations {
            let has_style_mutations = mutations
                .iter()
                .any(|m| matches!(m, js::DomMutation::SetStyleProperty { .. }));

            if !has_style_mutations {
                if let Some(plan) =
                    Self::build_incremental_relayout_plan(d, &self.resolved_styles_cache, mutations)
                {
                    let styles = self.resolved_styles_cache.clone();
                    let pseudo_styles = self.resolved_pseudo_styles.clone();
                    if plan.target_nodes.len() == 1 {
                        let target_node = plan.target_nodes[0];
                        if self.apply_incremental_layout_for_node(
                            d,
                            &styles,
                            &pseudo_styles,
                            target_node,
                        ) {
                            debug_surf!(
                                "[webview] incremental relayout reused cached styles for subtree {}",
                                target_node
                            );
                            return;
                        }
                    } else if self.apply_incremental_relayout_plan(
                        d,
                        &styles,
                        &pseudo_styles,
                        &plan,
                    ) {
                        debug_surf!(
                            "[webview] incremental relayout reused cached styles for parent {} ({} children, rebuild={})",
                            plan.parent_node,
                            plan.target_nodes.len(),
                            plan.rebuild_parent_children
                        );
                        return;
                    }
                }
            } else {
                let mut styles = self.resolved_styles_cache.clone();
                let pseudo_styles = self.resolved_pseudo_styles.clone();
                Self::apply_style_mutations_to_cached_styles(d, &mut styles, mutations);
                if let Some(plan) = Self::build_incremental_relayout_plan(d, &styles, mutations) {
                    if plan.target_nodes.len() == 1 {
                        let target_node = plan.target_nodes[0];
                        if self.apply_incremental_layout_for_node(
                            d,
                            &styles,
                            &pseudo_styles,
                            target_node,
                        ) {
                            self.resolved_styles_cache = styles;
                            debug_surf!(
                                "[webview] incremental relayout reused cached styles for subtree {} after local style update",
                                target_node
                            );
                            return;
                        }
                    } else if self.apply_incremental_relayout_plan(
                        d,
                        &styles,
                        &pseudo_styles,
                        &plan,
                    ) {
                        self.resolved_styles_cache = styles;
                        debug_surf!(
                            "[webview] incremental relayout reused cached styles for parent {} after local style update ({} children, rebuild={})",
                            plan.parent_node,
                            plan.target_nodes.len(),
                            plan.rebuild_parent_children
                        );
                        return;
                    }
                }
            }
        }

        let mut styles = self.resolved_styles_cache.clone();
        if let Some(mutations) = incremental_mutations {
            Self::apply_style_mutations_to_cached_styles(d, &mut styles, mutations);
        }
        let mut pseudo_styles = self.resolved_pseudo_styles.clone();
        self.apply_animation_overrides_to_styles(d, &mut styles);

        self.layout_root = None;
        unsafe {
            WEB_FONT_MAP = &self.web_fonts as *const _;
        }
        let mut root = layout::layout(
            d,
            &styles,
            &mut pseudo_styles,
            self.viewport_width,
            self.viewport_height as i32,
            &self.images,
        );
        if !self.scroll_offsets.is_empty() {
            Self::apply_scroll_offsets_to_layout(&self.scroll_offsets, &mut root);
        }
        self.total_height_val = calc_total_height(&root);
        self.layout_root = Some(root);
        self.update_resolved_style_cache(&styles, &pseudo_styles);
        self.refresh_render_surface_for_layout(d, &styles);
    }

    fn apply_style_mutations_to_cached_styles(
        dom: &dom::Dom,
        styles: &mut [style::ComputedStyle],
        mutations: &[js::DomMutation],
    ) {
        let root_fs = styles.first().map(|s| s.font_size).unwrap_or(16);
        for mutation in mutations {
            let js::DomMutation::SetStyleProperty {
                node_id,
                property,
                value,
            } = mutation
            else {
                continue;
            };
            if !Self::style_property_can_reuse_cached_styles(property) {
                continue;
            }
            let Ok(node_id) = usize::try_from(*node_id) else {
                continue;
            };
            if node_id >= styles.len() {
                continue;
            }
            let parent_fs = dom.nodes[node_id]
                .parent
                .and_then(|pid| styles.get(pid))
                .map(|s| s.font_size)
                .unwrap_or(root_fs);
            let decls = crate::css::parse_inline_style(&alloc::format!("{}: {}", property, value));
            for decl in &decls {
                style::apply_declaration(&mut styles[node_id], decl, parent_fs, root_fs);
            }
        }
    }

    fn flush_pending_mutations(&mut self, dom: &mut dom::Dom) -> MutationImpact {
        let impact = self.classify_pending_mutations();
        let pending_mutations = if impact != MutationImpact::None {
            self.js_runtime.mutations.clone()
        } else {
            Vec::new()
        };
        if impact == MutationImpact::None {
            self.js_runtime.apply_mutations(dom);
            return MutationImpact::None;
        }

        self.extract_scroll_offsets();
        self.js_runtime.apply_mutations(dom);

        match impact {
            MutationImpact::LayoutReuseStyles => {
                if self.can_reuse_cached_styles_for_mutations(dom, &pending_mutations) {
                    self.do_layout_and_render_with_cached_styles(dom, Some(&pending_mutations));
                } else {
                    self.do_layout_and_render(dom, Some(&pending_mutations));
                }
            }
            MutationImpact::LayoutRestyle => {
                self.inline_sheets_dirty = true;
                if Self::mutations_dirty_inline_style_cache(&pending_mutations) {
                    self.inline_style_cache.clear();
                }
                self.do_layout_and_render(dom, Some(&pending_mutations));
            }
            MutationImpact::Paint => {
                if let Some(root) = self.layout_root.as_mut() {
                    Self::apply_scroll_offsets_to_layout(&self.scroll_offsets, root);
                    Self::apply_paint_only_mutations_to_layout(&pending_mutations, root);
                }
                self.repaint_from_cached_layout();
            }
            MutationImpact::None => {}
        }

        impact
    }

    /// Internal: collect stylesheets, resolve styles, layout, and render controls.
    fn do_layout_and_render(
        &mut self,
        d: &dom::Dom,
        incremental_mutations: Option<&[js::DomMutation]>,
    ) {
        debug_surf!(
            "[webview] do_layout_and_render: {} DOM nodes",
            d.nodes.len()
        );

        // ── Stylesheet pipeline — parse once, reuse on every relayout ────────────
        //
        // `self.default_sheet` is parsed once in `WebView::new()`.
        // `self.external_sheets` are parsed once each in `add_stylesheet()`.
        // Only inline `<style>` blocks are re-parsed here because they live in the
        // mutable DOM and may be altered by JS mutations; they are typically tiny.
        //
        // This eliminates the catastrophic O(images × CSS-bytes) re-parse cost
        // visible in logs as repeated 150 KB parses per image load.

        // Phase A: Parse inline <style> blocks — cached across relayouts.
        // Only re-parsed when dirty (new page via set_html, or JS mutations).
        if self.inline_sheets_dirty {
            self.inline_sheets.clear();
            let mut inline_count = 0u32;
            for (i, node) in d.nodes.iter().enumerate() {
                if let dom::NodeType::Element {
                    tag: dom::Tag::Style,
                    ..
                } = &node.node_type
                {
                    let css_text = d.text_content(i);
                    if !css_text.is_empty() {
                        debug_surf!(
                            "[webview] parse inline <style> #{}: {} bytes",
                            inline_count,
                            css_text.len()
                        );
                        self.inline_sheets.push(css::parse_stylesheet(&css_text));
                        inline_count += 1;
                    }
                }
            }
            self.inline_sheets_dirty = false;
            self.keyframes_dirty = true;
            debug_surf!("[webview] parsed {} inline <style> blocks", inline_count);
        }

        debug_surf!(
            "[webview] total stylesheets: {} (1 default + {} external + {} inline)",
            1 + self.external_sheets.len() + self.inline_sheets.len(),
            self.external_sheets.len(),
            self.inline_sheets.len()
        );

        // Phase B: Resolve styles using zero-copy references to pre-parsed sheets.
        let vw = self.viewport_width;
        let vh = if self.viewport_height > 0 {
            self.viewport_height as i32
        } else {
            self.viewport_width
        };
        debug_surf!("[webview] resolve_styles start ({} nodes)", d.nodes.len());
        let (mut styles, mut pseudo_styles) = {
            let mut all_sheets: Vec<&css::Stylesheet> =
                Vec::with_capacity(1 + self.external_sheets.len() + self.inline_sheets.len());
            all_sheets.push(&self.default_sheet);
            for sheet in &self.external_sheets {
                all_sheets.push(sheet);
            }
            for sheet in &self.inline_sheets {
                all_sheets.push(sheet);
            }
            style::resolve_styles(d, &all_sheets, vw, vh, &mut self.inline_style_cache)
        };
        debug_surf!("[webview] resolve_styles done: {} styles", styles.len());

        // Collect @keyframes blocks from all stylesheets so the animation
        // tick loop can look them up by name.  Only rebuild when sheets change.
        self.rebuild_keyframes_if_dirty();

        // Register new @keyframe animations for nodes that request them.
        self.js_runtime.start_animations(&styles);

        // Detect CSS property changes and start transitions.
        if !self.prev_styles.is_empty() {
            self.js_runtime
                .start_transitions(&self.prev_styles, &styles);
        }
        // Save a snapshot of resolved styles *before* animation overrides
        // so transition detection compares the base styles next time.
        self.prev_styles.clone_from(&styles);

        // Apply pending animation/transition overrides on top of the
        // resolved styles so layout uses the interpolated values.
        self.apply_animation_overrides_to_styles(d, &mut styles);

        self.update_resolved_style_cache(&styles, &pseudo_styles);

        #[cfg(feature = "debug_surf")]
        debug_surf!(
            "[webview]   RSP=0x{:X} heap=0x{:X}",
            debug_rsp(),
            debug_heap_pos()
        );

        let body_id = d.find_body().unwrap_or(0);
        if let Some(mutations) = incremental_mutations {
            if let Some(plan) = Self::build_incremental_relayout_plan(d, &styles, mutations) {
                if plan.target_nodes.len() == 1 {
                    let target_node = plan.target_nodes[0];
                    if self.apply_incremental_layout_for_node(
                        d,
                        &styles,
                        &pseudo_styles,
                        target_node,
                    ) {
                        debug_surf!(
                            "[webview] incremental relayout reused subtree {}",
                            target_node
                        );
                        return;
                    }
                } else if self.apply_incremental_relayout_plan(d, &styles, &pseudo_styles, &plan) {
                    debug_surf!(
                        "[webview] incremental relayout reused parent {} for {} children (rebuild={})",
                        plan.parent_node,
                        plan.target_nodes.len(),
                        plan.rebuild_parent_children
                    );
                    return;
                }
            }
        }

        // Drop old layout tree before allocating the new one — avoids holding
        // two full trees in memory simultaneously (can save several MB on complex pages).
        self.layout_root = None;

        // Layout.
        debug_surf!(
            "[webview] layout start (viewport_width={})",
            self.viewport_width
        );
        // Set web font map for renderer access.
        unsafe {
            WEB_FONT_MAP = &self.web_fonts as *const _;
        }
        let mut root = layout::layout(
            d,
            &styles,
            &mut pseudo_styles,
            self.viewport_width,
            self.viewport_height as i32,
            &self.images,
        );
        // Apply JS scroll offsets (element.scrollTop/scrollLeft) to layout boxes.
        if !self.scroll_offsets.is_empty() {
            Self::apply_scroll_offsets_to_layout(&self.scroll_offsets, &mut root);
        }
        self.total_height_val = calc_total_height(&root);
        #[cfg(feature = "debug_surf")]
        {
            let box_count = count_layout_boxes(&root);
            debug_surf!(
                "[webview] layout done: {} boxes, height={}",
                box_count,
                self.total_height_val
            );
            debug_surf!(
                "[webview]   RSP=0x{:X} heap=0x{:X}",
                debug_rsp(),
                debug_heap_pos()
            );
        }

        // Soft-clear: reset hit regions and mark form controls for GC.
        // Canvas and form controls persist across relayouts.
        self.renderer.clear();

        // Sync content view background to the body element's CSS background-color.
        let body_bg = styles.get(body_id).map(|s| s.background_color).unwrap_or(0);
        let bg_color = if body_bg != 0 { body_bg } else { 0xFFFFFFFF };
        self.content_view.set_color(bg_color);

        // Set content view height to document height.
        let doc_w = self.viewport_width as u32;
        let doc_h = (self.total_height_val as u32).max(1);
        self.content_view.set_size(doc_w, doc_h);

        // Cache body background for scroll re-renders.
        self.bg_color_cached = bg_color;

        // Render into canvas + update form controls.
        // Initial render starts at scroll_y=0.
        debug_surf!("[webview] renderer start");
        self.pending_tiles = self.renderer.render(
            &root,
            &self.content_view,
            &self.images,
            doc_w,
            doc_h,
            self.viewport_height,
            0, // scroll_y = 0 for initial render
            bg_color,
            self.link_cb,
            self.link_cb_ud,
            self.submit_cb,
            self.submit_cb_ud,
        );
        self.last_render_scroll_y = 0;
        debug_surf!(
            "[webview] renderer done: {} form_controls",
            self.renderer.control_count()
        );
        #[cfg(feature = "debug_surf")]
        debug_surf!(
            "[webview]   RSP=0x{:X} heap=0x{:X}",
            debug_rsp(),
            debug_heap_pos()
        );

        // Cache layout tree for scroll re-renders (no relayout needed on scroll).
        self.layout_root = Some(root);
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
        let fc = self
            .renderer
            .form_controls
            .iter()
            .find(|fc| fc.control_id == control_id)?;
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
        let dom = match self.dom_val.as_ref() {
            Some(d) => d,
            None => return Vec::new(),
        };
        let fc = match self
            .renderer
            .form_controls
            .iter()
            .find(|fc| fc.control_id == control_id)
        {
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

fn find_node_bounds(
    bx: &LayoutBox,
    offset_x: i32,
    offset_y: i32,
    target: usize,
) -> Option<(i32, i32, i32, i32)> {
    let abs_x = if bx.is_fixed { bx.x } else { offset_x + bx.x };
    let abs_y = if bx.is_fixed { bx.y } else { offset_y + bx.y };
    if bx.node_id == Some(target) {
        return Some((abs_x, abs_y, bx.width.max(0), bx.height.max(0)));
    }
    for child in &bx.children {
        if let Some(bounds) = find_node_bounds(child, abs_x, abs_y, target) {
            return Some(bounds);
        }
    }
    None
}

/// Calculate total document height from the root layout box.
/// Fixed-position boxes are excluded — they are viewport-anchored and do not
/// contribute to the scrollable document height.
fn calc_total_height(root: &LayoutBox) -> i32 {
    let bottom = root.y + root.height;
    let mut max = bottom;
    for child in &root.children {
        if child.is_fixed {
            continue;
        }
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
        if child.is_fixed {
            continue;
        }
        let ch = child_total_height(child, abs_y);
        if ch > max {
            max = ch;
        }
    }
    max
}

/// Browser default CSS (minimal reset + sensible defaults).
const DEFAULT_CSS: &str = "
head, script, style, link, meta, title { display: none; }
html, body { display: block; }
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
