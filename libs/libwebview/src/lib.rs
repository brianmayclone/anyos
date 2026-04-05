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
        if WEB_FONT_MAP.is_null() { return None; }
        let map = &*WEB_FONT_MAP;
        // Try the whole string first (fastest path for single-name entries).
        let lower = family.to_ascii_lowercase();
        if let Some(id) = map.iter().find(|(f, _)| f.as_str() == lower.as_str()).map(|(_, id)| *id) {
            return Some(id);
        }
        // Parse comma-separated list and try each entry.
        for part in lower.split(',') {
            let name = part.trim().trim_matches('\'').trim_matches('"').trim();
            if name.is_empty() { continue; }
            if let Some(id) = map.iter().find(|(f, _)| f.as_str() == name).map(|(_, id)| *id) {
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
    inline_style_cache: Vec<(usize, Vec<css::Declaration>)>,
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
    /// Web font mapping: (family_name_lowercase, font_id from libfont).
    web_fonts: Vec<(String, u32)>,
    /// Previous resolved styles — kept for CSS transition change detection.
    prev_styles: Vec<style::ComputedStyle>,
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

        Self {
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
            web_fonts: Vec::new(),
            prev_styles: Vec::new(),
            anim_overrides: Vec::new(),
            scroll_offsets: Vec::new(),
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
        self.web_fonts.iter().find(|(f, _)| f == &lower).map(|(_, id)| *id)
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
        self.content_view.set_size(self.viewport_width as u32, 1);
        // Clear all stylesheets (external + inline).
        self.external_sheets.clear();
        self.inline_sheets.clear();
        self.inline_sheets_dirty = true;
        self.keyframes_dirty = true;
        self.inline_style_cache.clear();
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
        debug_surf!("[webview] html::parse done: {} nodes", parsed_dom.nodes.len());
        #[cfg(feature = "debug_surf")]
        anyos_std::println!("[webview]   RSP=0x{:X} heap=0x{:X}", debug_rsp(), debug_heap_pos());

        // NOTE: We do NOT replace "client-nojs" with "client-js" on the <html>
        // element.  Modern sites (Wikipedia, etc.) use client-nojs as a fallback
        // path designed for browsers without JS — which matches our capabilities
        // better than client-js, which expects full JS-controlled UI toggling.

        // New page — inline <style> blocks and style attribute cache need re-parsing.
        self.inline_sheets.clear();
        self.inline_sheets_dirty = true;
        self.inline_style_cache.clear();

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
            self.extract_scroll_offsets();
            self.js_runtime.apply_mutations(&mut parsed_dom);
            self.inline_sheets_dirty = true; // JS may have altered <style> tags
            self.inline_style_cache.clear(); // JS may have altered style="..." attrs
            self.do_layout_and_render(&parsed_dom);
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
        self.do_layout_and_render(&parsed_dom);

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
            let n = self.js_runtime.mutations.len();
            debug_surf!("[webview] applying {} JS mutations + relayout", n);
            self.extract_scroll_offsets();
            self.js_runtime.apply_mutations(&mut dom);
            self.inline_sheets_dirty = true;
            self.inline_style_cache.clear();
            self.do_layout_and_render(&dom);
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
            self.extract_scroll_offsets();
            self.js_runtime.apply_mutations(&mut dom);
            self.inline_sheets_dirty = true;
            self.inline_style_cache.clear();
            self.do_layout_and_render(&dom);
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
                self.extract_scroll_offsets();
                self.js_runtime.apply_mutations(&mut d);
                // JS may have modified <style> tags or style="..." attributes.
                self.inline_sheets_dirty = true;
                self.inline_style_cache.clear();
            }
            self.do_layout_and_render(&d);
            self.dom_val = Some(d);
        }
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
            let (any_active, overrides) =
                self.js_runtime.advance_animations(delta_ms, &self.keyframes);
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
            if delta > 4 {
                let pending = self.render_viewport(scroll_y);
                self.last_render_scroll_y = scroll_y;
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
        self.renderer.link_map.iter()
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

    /// Extract scroll offset mutations from the pending mutation list and merge
    /// them into `self.scroll_offsets`.  Must be called *before* `apply_mutations`
    /// which consumes the mutation vec via `mem::take`.
    fn extract_scroll_offsets(&mut self) {
        for m in &self.js_runtime.mutations {
            match m {
                js::DomMutation::SetScrollTop { node_id, value } => {
                    if let Some(entry) = self.scroll_offsets.iter_mut().find(|(id, _, _)| *id == *node_id) {
                        entry.1 = *value;
                    } else {
                        self.scroll_offsets.push((*node_id, *value, 0));
                    }
                }
                js::DomMutation::SetScrollLeft { node_id, value } => {
                    if let Some(entry) = self.scroll_offsets.iter_mut().find(|(id, _, _)| *id == *node_id) {
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

        // Phase A: Parse inline <style> blocks — cached across relayouts.
        // Only re-parsed when dirty (new page via set_html, or JS mutations).
        if self.inline_sheets_dirty {
            self.inline_sheets.clear();
            let mut inline_count = 0u32;
            for (i, node) in d.nodes.iter().enumerate() {
                if let dom::NodeType::Element { tag: dom::Tag::Style, .. } = &node.node_type {
                    let css_text = d.text_content(i);
                    if !css_text.is_empty() {
                        debug_surf!("[webview] parse inline <style> #{}: {} bytes", inline_count, css_text.len());
                        self.inline_sheets.push(css::parse_stylesheet(&css_text));
                        inline_count += 1;
                    }
                }
            }
            self.inline_sheets_dirty = false;
            self.keyframes_dirty = true;
            debug_surf!("[webview] parsed {} inline <style> blocks", inline_count);
        }

        debug_surf!("[webview] total stylesheets: {} (1 default + {} external + {} inline)",
            1 + self.external_sheets.len() + self.inline_sheets.len(),
            self.external_sheets.len(), self.inline_sheets.len());

        // Phase B: Resolve styles using zero-copy references to pre-parsed sheets.
        let vw = self.viewport_width;
        let vh = if self.viewport_height > 0 { self.viewport_height as i32 } else { self.viewport_width };
        debug_surf!("[webview] resolve_styles start ({} nodes)", d.nodes.len());
        let (mut styles, mut pseudo_styles) = {
            let mut all_sheets: Vec<&css::Stylesheet> = Vec::with_capacity(
                1 + self.external_sheets.len() + self.inline_sheets.len()
            );
            all_sheets.push(&self.default_sheet);
            for sheet in &self.external_sheets { all_sheets.push(sheet); }
            for sheet in &self.inline_sheets { all_sheets.push(sheet); }
            style::resolve_styles(d, &all_sheets, vw, vh, &mut self.inline_style_cache)
        };
        debug_surf!("[webview] resolve_styles done: {} styles", styles.len());

        // Collect @keyframes blocks from all stylesheets so the animation
        // tick loop can look them up by name.  Only rebuild when sheets change.
        if self.keyframes_dirty {
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

        // Register new @keyframe animations for nodes that request them.
        self.js_runtime.start_animations(&styles);

        // Detect CSS property changes and start transitions.
        if !self.prev_styles.is_empty() {
            self.js_runtime.start_transitions(&self.prev_styles, &styles);
        }
        // Save a snapshot of resolved styles *before* animation overrides
        // so transition detection compares the base styles next time.
        self.prev_styles = styles.clone();

        // Apply pending animation/transition overrides on top of the
        // resolved styles so layout uses the interpolated values.
        if !self.anim_overrides.is_empty() {
            let root_fs = if !styles.is_empty() { styles[0].font_size } else { 16 };
            for (node_id, decls) in &self.anim_overrides {
                if *node_id < styles.len() {
                    let parent_fs = {
                        let pid = d.nodes.get(*node_id).and_then(|n| n.parent).unwrap_or(0);
                        if pid < styles.len() { styles[pid].font_size } else { root_fs }
                    };
                    for decl in decls {
                        style::apply_declaration(&mut styles[*node_id], decl, parent_fs, root_fs);
                    }
                }
            }
        }

        #[cfg(feature = "debug_surf")]
        debug_surf!("[webview]   RSP=0x{:X} heap=0x{:X}", debug_rsp(), debug_heap_pos());

        // Drop old layout tree before allocating the new one — avoids holding
        // two full trees in memory simultaneously (can save several MB on complex pages).
        self.layout_root = None;

        // Layout.
        debug_surf!("[webview] layout start (viewport_width={})", self.viewport_width);
        // Set web font map for renderer access.
        unsafe { WEB_FONT_MAP = &self.web_fonts as *const _; }
        let mut root = layout::layout(d, &styles, &mut pseudo_styles, self.viewport_width, &self.images);
        // Apply JS scroll offsets (element.scrollTop/scrollLeft) to layout boxes.
        if !self.scroll_offsets.is_empty() {
            Self::apply_scroll_offsets_to_layout(&self.scroll_offsets, &mut root);
        }
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

        // Cache body background for scroll re-renders.
        self.bg_color_cached = bg_color;

        // Render into canvas + update form controls.
        // Initial render starts at scroll_y=0.
        debug_surf!("[webview] renderer start");
        self.renderer.render(
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
        debug_surf!("[webview] renderer done: {} form_controls", self.renderer.control_count());
        #[cfg(feature = "debug_surf")]
        debug_surf!("[webview]   RSP=0x{:X} heap=0x{:X}", debug_rsp(), debug_heap_pos());

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
