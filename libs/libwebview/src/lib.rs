//! libwebview — HTML rendering library for anyOS.
//!
//! Renders HTML content into a single Canvas pixel buffer for static content
//! (text, backgrounds, borders, images) and uses persistent libanyui controls
//! only for interactive form elements (TextField, Checkbox, etc.).
//!
//! # Usage
//! ```rust,ignore
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

use alloc::format;
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

#[derive(Clone, Copy)]
struct PendingSmoothScroll {
    node_id: usize,
    start_top: i32,
    start_left: i32,
    target_top: i32,
    target_left: i32,
    elapsed_ms: u32,
    duration_ms: u32,
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
const SYNTHETIC_AHEM_FONT_ID: u32 = u32::MAX - 1;
const SYNTHETIC_CONDENSED_FONT_ID: u32 = u32::MAX - 2;
const SYNTHETIC_NARROW_FONT_ID: u32 = u32::MAX - 3;
const SYNTHETIC_EXTRA_CONDENSED_FONT_ID: u32 = u32::MAX - 4;
const JS_TIMER_CALLBACK_BUDGET: usize = 4;
const JS_QUIET_TIMER_TICKS_BEFORE_THROTTLE: u32 = 30;
const JS_QUIET_TIMER_THROTTLE_MS: u64 = 250;

fn font_family_contains_ahem(family: &str) -> bool {
    family
        .to_ascii_lowercase()
        .split(',')
        .any(|part| part.trim().trim_matches('\'').trim_matches('"').trim() == "ahem")
}

fn synthetic_condensed_font_for_family(family: &str) -> Option<u32> {
    let lower = family.to_ascii_lowercase();
    for part in lower.split(',') {
        let name = part.trim().trim_matches('\'').trim_matches('"').trim();
        if name.is_empty() {
            continue;
        }
        if name.contains("extra-condensed")
            || name.contains("extracondensed")
            || name.contains("extracond")
            || name.contains("xnarrow")
            || name.contains("x-narrow")
        {
            return Some(SYNTHETIC_EXTRA_CONDENSED_FONT_ID);
        }
        if name.contains("narrow") {
            return Some(SYNTHETIC_NARROW_FONT_ID);
        }
        if name.contains("condensed") || name == "sans-serif-condensed" {
            return Some(SYNTHETIC_CONDENSED_FONT_ID);
        }
    }
    None
}

/// Look up a web font ID by family name (called from renderer/layout).
/// `family` may be a single name or a comma-separated CSS font-family list
/// like `"Georgia, 'Times New Roman', serif"`.
/// Tries each name in order; returns the first registered match.
pub fn lookup_web_font(family: &str) -> Option<u32> {
    unsafe {
        if font_family_contains_ahem(family) {
            return Some(SYNTHETIC_AHEM_FONT_ID);
        }
        if WEB_FONT_MAP.is_null() {
            return synthetic_condensed_font_for_family(family);
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
            // If a page asks for a narrow/condensed family that we could not
            // load (for example unsupported WOFF/CFF data), synthesize that
            // width before falling through to a later generic `sans-serif`.
            // This keeps flex/intrinsic sizing close to the author's intent.
            if let Some(id) = synthetic_condensed_font_for_family(name) {
                return Some(id);
            }
        }
        synthetic_condensed_font_for_family(family)
    }
}

pub fn is_ahem_font_id(font_id: u32) -> bool {
    if font_id == SYNTHETIC_AHEM_FONT_ID {
        return true;
    }
    if font_id == 0 {
        return false;
    }
    unsafe {
        if WEB_FONT_MAP.is_null() {
            return false;
        }
        let map = &*WEB_FONT_MAP;
        map.iter().any(|(family, id)| {
            *id == font_id && family.trim_matches('\'').trim_matches('"') == "ahem"
        })
    }
}

pub fn is_synthetic_condensed_font_id(font_id: u32) -> bool {
    matches!(
        font_id,
        SYNTHETIC_CONDENSED_FONT_ID | SYNTHETIC_NARROW_FONT_ID | SYNTHETIC_EXTRA_CONDENSED_FONT_ID
    )
}

pub fn synthetic_font_width_scale_percent(font_id: u32) -> i32 {
    match font_id {
        SYNTHETIC_EXTRA_CONDENSED_FONT_ID => 62,
        SYNTHETIC_NARROW_FONT_ID => 72,
        SYNTHETIC_CONDENSED_FONT_ID => 76,
        _ => 100,
    }
}

fn resolve_root_background_color(dom: &dom::Dom, styles: &[style::ComputedStyle]) -> u32 {
    let body_id = dom.find_body().unwrap_or(0);
    let body_bg = styles.get(body_id).map(|s| s.background_color).unwrap_or(0);
    if body_bg != 0 {
        return body_bg;
    }

    let html_bg = dom
        .find_html()
        .and_then(|html_id| styles.get(html_id))
        .map(|s| s.background_color)
        .unwrap_or(0);
    if html_bg != 0 {
        html_bg
    } else {
        0xFFFFFFFF
    }
}

fn normalize_document_height(content_height: i32, viewport_height: u32) -> i32 {
    content_height.max(viewport_height.max(1) as i32)
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
    /// Prepared flattened CSS rule set plus selector index.
    prepared_stylesheets: Option<style::PreparedStylesheets>,
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
    /// True when the current layout tree only covers an initial viewport budget
    /// and should be upgraded to a full layout once the user scrolls near the
    /// current bottom edge.
    deferred_full_layout_pending: bool,
    /// Current progressive layout budget in pixels for staged first render.
    deferred_layout_budget_px: i32,
    /// Current progressive style budget in node count for staged first render.
    deferred_style_node_budget: usize,
    /// Last expansion time for the staged first render budgets.
    deferred_budget_last_expand_ms: u64,
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
    /// Pending smooth-scroll animations for overflow containers.
    smooth_scrolls: Vec<PendingSmoothScroll>,
    /// True when the page was loaded via `set_html_dom_only()` and the first
    /// real render after external CSS arrives should prefer correctness over
    /// aggressive progressive culling.
    dom_only_initial_render_pending: bool,
    /// Dynamic selector state for pseudo-classes such as :hover and :focus.
    selector_state: style::SelectorState,
    /// Consecutive ticks where JS timers fired without producing visible or
    /// host-visible work. Used to throttle noisy analytics/heartbeat timers.
    js_quiet_timer_ticks: u32,
    /// Real elapsed time accumulated while quiet timers are throttled.
    js_timer_throttle_accum_ms: u64,
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
            prepared_stylesheets: None,
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
            deferred_full_layout_pending: false,
            deferred_layout_budget_px: 0,
            deferred_style_node_budget: 0,
            deferred_budget_last_expand_ms: 0,
            web_fonts: Vec::new(),
            prev_styles: Vec::new(),
            resolved_styles_cache: Vec::new(),
            resolved_styles_viewport_width: 0,
            resolved_styles_viewport_height: 0,
            resolved_pseudo_styles: style::PseudoStyles::empty(0),
            anim_overrides: Vec::new(),
            scroll_offsets: Vec::new(),
            smooth_scrolls: Vec::new(),
            dom_only_initial_render_pending: false,
            selector_state: style::SelectorState::default(),
            js_quiet_timer_ticks: 0,
            js_timer_throttle_accum_ms: 0,
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
        self.prepared_stylesheets = None;
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
        self.prepared_stylesheets = None;
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
        self.clear_deferred_layout_state();
        self.selector_state = style::SelectorState::default();
        self.content_view.set_size(self.viewport_width as u32, 1);
        // Clear all stylesheets (external + inline).
        self.external_sheets.clear();
        self.inline_sheets.clear();
        self.inline_sheets_dirty = true;
        self.keyframes_dirty = true;
        self.inline_style_cache.clear();
        self.prepared_stylesheets = None;
        self.resolved_styles_cache.clear();
        self.resolved_styles_viewport_width = 0;
        self.resolved_styles_viewport_height = 0;
        self.resolved_pseudo_styles = style::PseudoStyles::empty(0);
        // Clear web fonts from the previous page.
        self.web_fonts.clear();
        // Reset JS runtime (fresh engine, no timers/listeners/websockets).
        self.js_runtime.reset();
        self.js_quiet_timer_ticks = 0;
        self.js_timer_throttle_accum_ms = 0;
    }

    fn prime_inline_stylesheets_from_dom(&mut self, dom: &dom::Dom) {
        self.inline_sheets.clear();
        let mut inline_count = 0u32;
        for (i, node) in dom.nodes.iter().enumerate() {
            if let dom::NodeType::Element {
                tag: dom::Tag::Style,
                ..
            } = &node.node_type
            {
                let css_text = dom.text_content(i);
                if !css_text.is_empty() {
                    self.inline_sheets.push(css::parse_stylesheet(&css_text));
                    inline_count += 1;
                }
            }
        }
        self.inline_sheets_dirty = false;
        self.keyframes_dirty = true;
        self.prepared_stylesheets = None;
        debug_surf!(
            "[webview] primed {} inline <style> blocks without initial relayout",
            inline_count
        );
    }

    /// Add a decoded image to the cache. Will be displayed on next render.
    pub fn add_image(&mut self, src: &str, pixels: Vec<u32>, w: u32, h: u32) {
        self.images.add(String::from(src), pixels, w, h);
    }

    pub fn has_decoded_image(&self, src: &str) -> bool {
        self.images.has_pixels_for(src)
    }

    /// Returns true when loading `src` can change geometry and therefore needs
    /// a full relayout instead of a paint-only refresh.
    ///
    /// This is primarily needed for `<img>` elements without explicit width or
    /// height attributes, where the intrinsic image size becomes known only
    /// after decoding.
    pub fn image_requires_layout_refresh(&self, src: &str) -> bool {
        let Some(dom) = self.dom_val.as_ref() else {
            return false;
        };
        for (node_id, node) in dom.nodes.iter().enumerate() {
            let is_image_like = matches!(
                &node.node_type,
                dom::NodeType::Element {
                    tag: dom::Tag::Img,
                    ..
                }
            ) || dom.has_tag_name(node_id, "a-img");
            if !is_image_like {
                continue;
            }
            let Some(node_src) = dom.image_url(node_id) else {
                continue;
            };
            if node_src != src {
                continue;
            }
            let has_width = dom.attr(node_id, "width").is_some();
            let has_height = dom.attr(node_id, "height").is_some();
            if !(has_width && has_height) {
                return true;
            }
        }
        false
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
        self.smooth_scrolls.clear();

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
        self.prepared_stylesheets = None;

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
        self.prepared_stylesheets = None;
        self.dom_only_initial_render_pending = false;

        // Layout and render (no JS).
        self.do_layout_and_render(&parsed_dom, None);

        // Store DOM.
        self.dom_val = Some(parsed_dom);
    }

    /// Parse HTML and store the DOM without performing the initial layout pass.
    ///
    /// This is useful for hosts that want to discover subresources first and
    /// only perform the first expensive relayout once the stylesheet chain is
    /// complete, avoiding a large unstyled render followed by a second full
    /// render moments later.
    pub fn set_html_dom_only(&mut self, html_text: &str) {
        debug_surf!(
            "[webview] set_html_dom_only: {} bytes input",
            html_text.len()
        );

        let parsed_dom = html::parse(html_text);

        self.inline_sheets.clear();
        self.inline_sheets_dirty = true;
        self.inline_style_cache.clear();
        self.layout_root = None;
        self.total_height_val = 0;
        self.last_render_scroll_y = 0;
        self.pending_tiles = false;
        self.clear_deferred_layout_state();
        self.content_view
            .set_size(self.viewport_width.max(1) as u32, 1);
        self.dom_only_initial_render_pending = true;

        self.prime_inline_stylesheets_from_dom(&parsed_dom);
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
    pub fn execute_js(&mut self, scripts: &[String]) -> bool {
        let mut dom = match self.dom_val.take() {
            Some(d) => d,
            None => return false,
        };

        let url = self.current_url.clone();
        self.js_runtime.execute_script_sources(&dom, &url, scripts);

        // Apply DOM mutations and re-layout.
        let mut changed = false;
        if !self.js_runtime.mutations.is_empty() {
            self.flush_pending_mutations(&mut dom);
            changed = true;
        }

        self.dom_val = Some(dom);
        changed
    }

    /// Evaluate a JavaScript expression from Developer Tools and append the
    /// prompt/result to the page console buffer. Returns true when DOM changes
    /// require a refresh.
    pub fn eval_js_for_devtools(&mut self, source: &str) -> bool {
        let mut dom = match self.dom_val.take() {
            Some(d) => d,
            None => return false,
        };

        self.js_runtime
            .push_console_line(alloc::format!("> {}", source));
        let result = self.js_runtime.eval_with_dom(source, &dom);
        self.js_runtime
            .console
            .push(alloc::format!("< {}", result.to_js_string()));

        let mut changed = false;
        if !self.js_runtime.mutations.is_empty() {
            self.flush_pending_mutations(&mut dom);
            changed = true;
        }

        self.dom_val = Some(dom);
        changed
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

    /// Take pending JavaScript-initiated page navigations.
    pub fn take_pending_navigation_requests(&mut self) -> Vec<js::PendingNavigationRequest> {
        self.js_runtime.take_pending_navigation_requests()
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

    pub fn layout_root_ref(&self) -> Option<&LayoutBox> {
        self.layout_root.as_ref()
    }

    /// Return the last fully resolved computed style for a DOM node.
    pub fn resolved_style_ref(&self, node_id: usize) -> Option<&style::ComputedStyle> {
        self.resolved_styles_cache.get(node_id)
    }

    /// Build a Developer Tools inspector report for a DOM node.
    ///
    /// This is intentionally assembled in libwebview rather than Surf so the
    /// inspector sees the same selector matching, cached computed styles, and
    /// layout data the renderer used.
    pub fn devtools_inspector_report(&self, node_id: usize) -> Option<String> {
        let dom = self.dom_val.as_ref()?;
        if node_id >= dom.nodes.len() {
            return None;
        }
        Some(build_devtools_inspector_report(self, dom, node_id))
    }

    pub fn tile_canvas_ids(&self) -> Vec<u32> {
        self.renderer.tile_canvas_ids()
    }

    /// Render tiles for the given scroll position (public wrapper).
    /// Returns `true` if there are pending tiles not yet rasterized.
    pub fn render_viewport_at(&mut self, scroll_y: i32) -> bool {
        let pending = self.render_viewport(scroll_y, false);
        self.last_render_scroll_y = scroll_y;
        self.pending_tiles = pending;
        pending
    }

    /// Render only immediately visible viewport tiles with a tiny per-frame
    /// budget. Used while the user is actively scrolling.
    pub fn render_scroll_frame_at(&mut self, scroll_y: i32) -> bool {
        let pending = self.render_viewport(scroll_y, true);
        self.last_render_scroll_y = scroll_y;
        self.pending_tiles = pending;
        pending
    }

    pub fn deferred_layout_upgrade_needed(&self, scroll_y: i32) -> bool {
        self.should_upgrade_deferred_layout(scroll_y)
    }

    /// Resize the viewport and re-layout.
    pub fn resize(&mut self, w: u32, h: u32) {
        // Skip if dimensions haven't changed — avoids redundant relayouts.
        if self.viewport_width == w as i32 && self.viewport_height == h {
            return;
        }
        self.viewport_width = w as i32;
        self.viewport_height = h;
        self.prepared_stylesheets = None;
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

    fn layout_budget_for_document(&self, dom: &dom::Dom) -> Option<i32> {
        if self.deferred_full_layout_pending && self.deferred_layout_budget_px > 0 {
            return Some(self.deferred_layout_budget_px);
        }
        let _ = dom;
        None
    }

    fn style_budget_for_document(&self, dom: &dom::Dom) -> Option<usize> {
        if self.deferred_full_layout_pending && self.deferred_style_node_budget > 0 {
            return Some(self.deferred_style_node_budget.min(dom.nodes.len()));
        }
        None
    }

    fn ensure_initial_progressive_budget(&mut self, dom: &dom::Dom) {
        let large_doc = dom.nodes.len() > 1800;
        let initial_large_document_layout =
            self.layout_root.is_none() && self.total_height_val == 0 && large_doc;
        if !initial_large_document_layout {
            return;
        }
        if self.deferred_layout_budget_px > 0 || self.deferred_style_node_budget > 0 {
            return;
        }

        let viewport_h = self.viewport_height.max(1) as i32;
        let budget_multiplier = if dom.nodes.len() > 7000 {
            4
        } else if dom.nodes.len() > 4000 {
            4
        } else {
            3
        };
        self.deferred_layout_budget_px = (viewport_h * budget_multiplier).max(4096);
        self.deferred_style_node_budget = if dom.nodes.len() > 7000 {
            4096
        } else if dom.nodes.len() > 4000 {
            3072
        } else {
            2048
        }
        .min(dom.nodes.len());
        self.deferred_budget_last_expand_ms = anyos_std::sys::uptime_ms() as u64;
        self.deferred_full_layout_pending = true;
        debug_surf!(
            "[webview] using progressive first-render budget: nodes={} style_budget={} layout_budget={}px",
            dom.nodes.len(),
            self.deferred_style_node_budget,
            self.deferred_layout_budget_px
        );
    }

    fn clear_deferred_layout_state(&mut self) {
        self.deferred_full_layout_pending = false;
        self.deferred_layout_budget_px = 0;
        self.deferred_style_node_budget = 0;
        self.deferred_budget_last_expand_ms = 0;
    }

    fn should_upgrade_deferred_layout(&self, scroll_y: i32) -> bool {
        if !self.deferred_full_layout_pending {
            return false;
        }
        let viewport_h = self.viewport_height.max(1) as i32;
        let upgrade_threshold = (viewport_h * 2).max(1024);
        scroll_y + upgrade_threshold >= self.total_height_val
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

        self.pending_tiles = self.renderer.repaint(
            root,
            &self.content_view,
            &self.images,
            doc_w,
            doc_h,
            self.viewport_height,
            scroll_y,
            bg_color,
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
            let quiet_throttled =
                self.js_quiet_timer_ticks >= JS_QUIET_TIMER_TICKS_BEFORE_THROTTLE;
            let mut timer_delta_ms = delta_ms;
            let mut run_js_timers = true;
            if quiet_throttled {
                self.js_timer_throttle_accum_ms += delta_ms;
                if self.js_timer_throttle_accum_ms < JS_QUIET_TIMER_THROTTLE_MS {
                    run_js_timers = false;
                    changed = true;
                } else {
                    timer_delta_ms = self.js_timer_throttle_accum_ms;
                    self.js_timer_throttle_accum_ms = 0;
                }
            }

            if run_js_timers {
                let mut dom = match self.dom_val.take() {
                    Some(d) => d,
                    None => return false,
                };
                let pending_http_before = self.js_runtime.pending_http_requests.len();
                let pending_nav_before = self.js_runtime.pending_navigation_requests.len();
                let pending_ws_before = self.js_runtime.pending_ws_connects.len()
                    + self.js_runtime.pending_ws_sends.len()
                    + self.js_runtime.pending_ws_closes.len();
                let fired =
                    self.js_runtime
                        .tick_with_budget(&dom, timer_delta_ms, JS_TIMER_CALLBACK_BUDGET);
                let produced_host_work =
                    self.js_runtime.pending_http_requests.len() != pending_http_before
                        || self.js_runtime.pending_navigation_requests.len() != pending_nav_before
                        || (self.js_runtime.pending_ws_connects.len()
                            + self.js_runtime.pending_ws_sends.len()
                            + self.js_runtime.pending_ws_closes.len())
                            != pending_ws_before;
                if !self.js_runtime.mutations.is_empty() {
                    self.flush_pending_mutations(&mut dom);
                    self.relayout();
                    changed = true;
                    self.js_quiet_timer_ticks = 0;
                    self.js_timer_throttle_accum_ms = 0;
                } else if produced_host_work {
                    changed = true;
                    self.js_quiet_timer_ticks = 0;
                    self.js_timer_throttle_accum_ms = 0;
                } else if fired > 0 {
                    self.js_quiet_timer_ticks = self.js_quiet_timer_ticks.saturating_add(1);
                    // Keep the tick loop alive for timer-driven async work, but
                    // once callbacks repeatedly produce no visible/host work they
                    // are throttled above instead of running at 60Hz forever.
                    changed = true;
                }
                self.dom_val = Some(dom);
            }
        } else {
            self.js_quiet_timer_ticks = 0;
            self.js_timer_throttle_accum_ms = 0;
        }

        // ── 2. CSS animations & transitions ──────────────────────────────────────
        if !self.js_runtime.active_animations.is_empty()
            || !self.js_runtime.active_transitions.is_empty()
            || !self.js_runtime.active_style_animations.is_empty()
        {
            if !self.js_runtime.active_style_animations.is_empty() {
                let style_anim_mutations = self.js_runtime.advance_style_animations(delta_ms);
                if style_anim_mutations > 0 {
                    let mut dom = match self.dom_val.take() {
                        Some(d) => d,
                        None => return changed,
                    };
                    self.flush_pending_mutations(&mut dom);
                    self.dom_val = Some(dom);
                    changed = true;
                }
            }

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

        // ── 2.5. Smooth scrolling for overflow containers ───────────────────────
        if self.advance_smooth_scrolls(delta_ms) {
            changed = true;
        }

        // ── 3. Scroll-based tile management (compositor-driven). ─────────────────
        // Per-tile canvases are positioned in the content_view.  The compositor
        // handles smooth scrolling natively.  We only need to create tile
        // canvases for rows entering the pre-render zone (incrementally, max
        // 2 per tick to avoid blocking the event loop).
        //
        // When pending tiles remain, we signal changed=true so the anim timer
        // keeps running until all visible tiles are rasterized.  The per-tick
        // limit prevents blocking the event loop.
        if self.layout_root.is_some() {
            let scroll_y = self.scroll_view.get_state() as i32;
            let delta = (scroll_y - self.last_render_scroll_y).abs();
            if delta > 4 || self.pending_tiles {
                let pending = self.render_viewport(scroll_y, false);
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
    fn render_viewport(&mut self, scroll_y: i32, scrolling: bool) -> bool {
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
                scrolling,
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
        self.clear_deferred_layout_state();
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

    /// Check if a canvas click hit a reset button.  Returns the DOM node_id
    /// of the reset element, or None.
    pub fn canvas_reset_hit(&self, control_id: u32) -> Option<usize> {
        if let Some((mx, doc_y)) = self.renderer.tile_hit_coords(control_id) {
            return self.renderer.hit_test_reset_at(mx, doc_y);
        }
        None
    }

    /// Check if a control is a reset button (canvas or legacy).
    pub fn is_reset_button(&self, control_id: u32) -> bool {
        self.canvas_reset_hit(control_id).is_some()
    }

    /// Check if a canvas click hit a file input. Returns the DOM node_id.
    pub fn canvas_file_input_hit(&self, control_id: u32) -> Option<usize> {
        if let Some((mx, doc_y)) = self.renderer.tile_hit_coords(control_id) {
            return self.renderer.hit_test_file_input_at(mx, doc_y);
        }
        None
    }

    /// Check if a canvas click hit a color input. Returns the DOM node_id.
    pub fn canvas_color_input_hit(&self, control_id: u32) -> Option<usize> {
        if let Some((mx, doc_y)) = self.renderer.tile_hit_coords(control_id) {
            return self.renderer.hit_test_color_input_at(mx, doc_y);
        }
        None
    }

    pub fn canvas_checkbox_hit(&self, control_id: u32) -> Option<usize> {
        if let Some((mx, doc_y)) = self.renderer.tile_hit_coords(control_id) {
            return self.renderer.hit_test_checkbox_at(mx, doc_y);
        }
        None
    }

    pub fn canvas_select_hit(&self, control_id: u32) -> Option<usize> {
        if let Some((mx, doc_y)) = self.renderer.tile_hit_coords(control_id) {
            return self.renderer.hit_test_select_at(mx, doc_y);
        }
        None
    }

    pub fn canvas_radio_hit(&self, control_id: u32) -> Option<usize> {
        if let Some((mx, doc_y)) = self.renderer.tile_hit_coords(control_id) {
            return self.renderer.hit_test_radio_at(mx, doc_y);
        }
        None
    }

    pub fn canvas_range_hit(&self, control_id: u32) -> Option<usize> {
        if let Some((mx, doc_y)) = self.renderer.tile_hit_coords(control_id) {
            return self.renderer.hit_test_range_at(mx, doc_y);
        }
        None
    }

    fn select_option_nodes(dom: &dom::Dom, select_node: usize) -> Vec<usize> {
        let mut out = Vec::new();
        let children = dom.get(select_node).children.clone();
        for cid in children {
            if dom.tag(cid) == Some(dom::Tag::Option) {
                out.push(cid);
            } else if dom.tag(cid) == Some(dom::Tag::Optgroup) {
                let group_children = dom.get(cid).children.clone();
                for gcid in group_children {
                    if dom.tag(gcid) == Some(dom::Tag::Option) {
                        out.push(gcid);
                    }
                }
            }
        }
        out
    }

    pub fn advance_select_for_canvas(&mut self, control_id: u32) -> bool {
        let Some(node_id) = self.canvas_select_hit(control_id) else {
            return false;
        };
        {
            let Some(dom) = self.dom_val.as_mut() else {
                return false;
            };
            let option_nodes = Self::select_option_nodes(dom, node_id);
            if option_nodes.is_empty() {
                return false;
            }

            let mut enabled_nodes = Vec::new();
            let mut selected_enabled_idx = None;
            for option_id in option_nodes.iter().copied() {
                if let dom::NodeType::Element { ref mut attrs, .. } =
                    dom.get_mut(option_id).node_type
                {
                    if attrs
                        .iter()
                        .all(|a| a.name != "data-webview-default-selected")
                    {
                        attrs.push(dom::Attr {
                            name: String::from("data-webview-default-selected"),
                            value: if attrs.iter().any(|a| a.name == "selected") {
                                String::from("1")
                            } else {
                                String::from("0")
                            },
                        });
                    }
                }
                if dom.attr(option_id, "disabled").is_some() {
                    continue;
                }
                if dom.attr(option_id, "selected").is_some() {
                    selected_enabled_idx = Some(enabled_nodes.len());
                }
                enabled_nodes.push(option_id);
            }
            if enabled_nodes.is_empty() {
                return false;
            }

            let next_idx = selected_enabled_idx
                .map(|idx| (idx + 1) % enabled_nodes.len())
                .unwrap_or(0);
            let next_node = enabled_nodes[next_idx];

            for option_id in option_nodes {
                if let dom::NodeType::Element { ref mut attrs, .. } =
                    dom.get_mut(option_id).node_type
                {
                    if option_id == next_node {
                        if attrs.iter().all(|a| a.name != "selected") {
                            attrs.push(dom::Attr {
                                name: String::from("selected"),
                                value: String::new(),
                            });
                        }
                    } else if let Some(pos) = attrs.iter().position(|a| a.name == "selected") {
                        attrs.remove(pos);
                    }
                }
            }
        }
        self.relayout();
        true
    }

    pub fn toggle_checkbox_for_canvas(&mut self, control_id: u32) -> bool {
        let Some(node_id) = self.canvas_checkbox_hit(control_id) else {
            return false;
        };
        {
            let Some(dom) = self.dom_val.as_mut() else {
                return false;
            };
            if let dom::NodeType::Element { ref mut attrs, .. } = dom.get_mut(node_id).node_type {
                if attrs
                    .iter()
                    .all(|a| a.name != "data-webview-default-checked")
                {
                    attrs.push(dom::Attr {
                        name: String::from("data-webview-default-checked"),
                        value: if attrs.iter().any(|a| a.name == "checked") {
                            String::from("1")
                        } else {
                            String::from("0")
                        },
                    });
                }
                if let Some(pos) = attrs.iter().position(|a| a.name == "checked") {
                    attrs.remove(pos);
                } else {
                    attrs.push(dom::Attr {
                        name: String::from("checked"),
                        value: String::new(),
                    });
                }
            }
        }
        self.relayout();
        true
    }

    pub fn toggle_radio_for_canvas(&mut self, control_id: u32) -> bool {
        let Some(node_id) = self.canvas_radio_hit(control_id) else {
            return false;
        };
        {
            let Some(dom) = self.dom_val.as_mut() else {
                return false;
            };

            let radio_name = String::from(dom.attr(node_id, "name").unwrap_or(""));
            let form_node = Self::find_form_for_node_in_dom(dom, node_id);

            for other_id in 0..dom.nodes.len() {
                if dom.tag(other_id) != Some(dom::Tag::Input) || other_id == node_id {
                    continue;
                }
                if !dom
                    .attr(other_id, "type")
                    .map(|t| t.eq_ignore_ascii_case("radio"))
                    .unwrap_or(false)
                {
                    continue;
                }
                let other_name = dom.attr(other_id, "name").unwrap_or("");
                if other_name != radio_name {
                    continue;
                }
                let same_form = {
                    Self::find_form_for_node_in_dom(dom, other_id) == form_node
                };
                if !same_form {
                    continue;
                }
                if let dom::NodeType::Element { ref mut attrs, .. } =
                    dom.get_mut(other_id).node_type
                {
                    if attrs
                        .iter()
                        .all(|a| a.name != "data-webview-default-checked")
                    {
                        attrs.push(dom::Attr {
                            name: String::from("data-webview-default-checked"),
                            value: if attrs.iter().any(|a| a.name == "checked") {
                                String::from("1")
                            } else {
                                String::from("0")
                            },
                        });
                    }
                    if let Some(pos) = attrs.iter().position(|a| a.name == "checked") {
                        attrs.remove(pos);
                    }
                }
            }

            if let dom::NodeType::Element { ref mut attrs, .. } = dom.get_mut(node_id).node_type {
                if attrs
                    .iter()
                    .all(|a| a.name != "data-webview-default-checked")
                {
                    attrs.push(dom::Attr {
                        name: String::from("data-webview-default-checked"),
                        value: if attrs.iter().any(|a| a.name == "checked") {
                            String::from("1")
                        } else {
                            String::from("0")
                        },
                    });
                }
                if attrs.iter().all(|a| a.name != "checked") {
                    attrs.push(dom::Attr {
                        name: String::from("checked"),
                        value: String::new(),
                    });
                }
            }
        }

        self.relayout();
        true
    }

    pub fn set_range_for_canvas(&mut self, control_id: u32) -> bool {
        let Some(node_id) = self.canvas_range_hit(control_id) else {
            return false;
        };
        let Some((mx, _doc_y)) = self.renderer.tile_hit_coords(control_id) else {
            return false;
        };
        let Some(fc) = self
            .renderer
            .form_controls
            .iter()
            .find(|fc| fc.node_id == node_id && fc.kind == FormFieldKind::Range)
        else {
            return false;
        };
        let rel_x = (mx - fc.doc_x).clamp(0, fc.doc_w.max(1));
        let pct = (rel_x as f32 / fc.doc_w.max(1) as f32).clamp(0.0, 1.0);

        {
            let Some(dom) = self.dom_val.as_mut() else {
                return false;
            };
            let min_f: f32 = dom
                .attr(node_id, "min")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0);
            let max_f: f32 = dom
                .attr(node_id, "max")
                .and_then(|s| s.parse().ok())
                .unwrap_or(100.0);
            let default_value = String::from(dom.attr(node_id, "value").unwrap_or(""));
            let value = min_f + (max_f - min_f).max(0.0) * pct;
            let nearest_int = if value >= 0.0 {
                (value + 0.5) as i32
            } else {
                (value - 0.5) as i32
            };
            let diff = value - nearest_int as f32;
            let value_str = if diff <= 0.000_001 && diff >= -0.000_001 {
                alloc::format!("{}", nearest_int)
            } else {
                alloc::format!("{}", value)
            };
            if let dom::NodeType::Element { ref mut attrs, .. } = dom.get_mut(node_id).node_type {
                if attrs.iter().all(|a| a.name != "data-webview-default-value") {
                    attrs.push(dom::Attr {
                        name: String::from("data-webview-default-value"),
                        value: default_value,
                    });
                }
                if let Some(attr) = attrs.iter_mut().find(|a| a.name == "value") {
                    attr.value = value_str;
                } else {
                    attrs.push(dom::Attr {
                        name: String::from("value"),
                        value: value_str,
                    });
                }
            }
        }
        self.relayout();
        true
    }

    pub fn set_color_for_canvas(&mut self, control_id: u32) -> bool {
        let Some(node_id) = self.canvas_color_input_hit(control_id) else {
            return false;
        };
        let Some((mx, doc_y)) = self.renderer.tile_hit_coords(control_id) else {
            return false;
        };
        let Some(fc) = self
            .renderer
            .form_controls
            .iter()
            .find(|fc| fc.node_id == node_id && fc.kind == FormFieldKind::Color)
        else {
            return false;
        };
        let rel_x = (mx - fc.doc_x).clamp(0, fc.doc_w.max(1) - 1);
        let rel_y = (doc_y - fc.doc_y).clamp(0, fc.doc_h.max(1) - 1);
        let width = fc.doc_w.max(1) as u32;
        let height = fc.doc_h.max(1) as u32;

        // Map click position onto a simple HSV picker:
        // horizontal = hue, vertical = value, saturation fixed high.
        let hue = rel_x as u32 * 360 / width;
        let value = 255u32.saturating_sub(rel_y as u32 * 155 / height);
        let saturation = 220u32;
        let color = hsv_to_rgb_u32(hue.min(359), saturation, value.max(100));
        let color_hex = color_to_hex(color);

        if let Some(dom) = self.dom_val.as_mut() {
            let default_value = String::from(dom.attr(node_id, "value").unwrap_or("#000000"));
            if let dom::NodeType::Element { ref mut attrs, .. } = dom.get_mut(node_id).node_type {
                if attrs.iter().all(|a| a.name != "data-webview-default-value") {
                    attrs.push(dom::Attr {
                        name: String::from("data-webview-default-value"),
                        value: default_value,
                    });
                }
            }
        }
        self.set_color_input_value(node_id, &color_hex);
        true
    }

    /// Set the selected file path for a file input control.
    /// Updates the DOM value attribute and the control's display text.
    pub fn set_file_input_value(&mut self, node_id: usize, file_path: &str) {
        // Extract just the filename from the path.
        let filename = file_path.rsplit('/').next().unwrap_or(file_path);
        // Update the DOM attribute.
        if let Some(dom) = self.dom_val.as_mut() {
            if let dom::NodeType::Element { ref mut attrs, .. } = dom.get_mut(node_id).node_type {
                if let Some(attr) = attrs.iter_mut().find(|a| a.name == "value") {
                    attr.value = String::from(filename);
                } else {
                    attrs.push(dom::Attr {
                        name: String::from("value"),
                        value: String::from(filename),
                    });
                }
                // Store full path in data-filepath for form submission.
                if let Some(attr) = attrs.iter_mut().find(|a| a.name == "data-filepath") {
                    attr.value = String::from(file_path);
                } else {
                    attrs.push(dom::Attr {
                        name: String::from("data-filepath"),
                        value: String::from(file_path),
                    });
                }
            }
        }
    }

    /// Set the color value for a color input control.
    pub fn set_color_input_value(&mut self, node_id: usize, color_hex: &str) {
        if let Some(dom) = self.dom_val.as_mut() {
            if let dom::NodeType::Element { ref mut attrs, .. } = dom.get_mut(node_id).node_type {
                if let Some(attr) = attrs.iter_mut().find(|a| a.name == "value") {
                    attr.value = String::from(color_hex);
                } else {
                    attrs.push(dom::Attr {
                        name: String::from("value"),
                        value: String::from(color_hex),
                    });
                }
            }
        }
        // Update the native control's text and background color.
        if let Some(fc) = self
            .renderer
            .form_controls
            .iter()
            .find(|fc| fc.node_id == node_id)
        {
            if fc.control_id != 0 {
                let ctrl = ui::Control::from_id(fc.control_id);
                ctrl.set_text(color_hex);
                let color = renderer::parse_color_value(color_hex);
                ctrl.set_color(color);
            }
        }
        self.relayout();
    }

    /// Reset all form controls in the form containing the given reset button.
    /// Restores each control to its initial/default value (HTML §4.10.22.3).
    pub fn reset_form(&mut self, control_id: u32) {
        let node_id = match self.canvas_reset_hit(control_id) {
            Some(n) => n,
            None => return,
        };
        let dom = match self.dom_val.as_mut() {
            Some(d) => d,
            None => return,
        };

        let form_id = match Self::find_form_for_node_in_dom(dom, node_id) {
            Some(id) => id,
            None => return,
        };

        // Reset all form controls that are descendants of this form.
        for fc in &self.renderer.form_controls {
            if Self::find_form_for_node_in_dom(dom, fc.node_id) != Some(form_id) {
                continue;
            }

            match fc.kind {
                FormFieldKind::TextInput | FormFieldKind::Password => {
                    if fc.control_id == 0 {
                        continue;
                    }
                    let default_val = dom.attr(fc.node_id, "value").unwrap_or("");
                    ui::Control::from_id(fc.control_id).set_text(default_val);
                }
                FormFieldKind::Checkbox => {
                    let checked = dom
                        .attr(fc.node_id, "data-webview-default-checked")
                        .map(|s| s == "1")
                        .unwrap_or_else(|| dom.attr(fc.node_id, "checked").is_some());
                    if let dom::NodeType::Element { ref mut attrs, .. } =
                        dom.get_mut(fc.node_id).node_type
                    {
                        if checked {
                            if attrs.iter().all(|a| a.name != "checked") {
                                attrs.push(dom::Attr {
                                    name: String::from("checked"),
                                    value: String::new(),
                                });
                            }
                        } else if let Some(pos) = attrs.iter().position(|a| a.name == "checked") {
                            attrs.remove(pos);
                        }
                    }
                    if fc.control_id == 0 {
                        continue;
                    }
                    ui::Control::from_id(fc.control_id).set_state(if checked { 1 } else { 0 });
                }
                FormFieldKind::Radio => {
                    let checked = dom
                        .attr(fc.node_id, "data-webview-default-checked")
                        .map(|s| s == "1")
                        .unwrap_or_else(|| dom.attr(fc.node_id, "checked").is_some());
                    if let dom::NodeType::Element { ref mut attrs, .. } =
                        dom.get_mut(fc.node_id).node_type
                    {
                        if checked {
                            if attrs.iter().all(|a| a.name != "checked") {
                                attrs.push(dom::Attr {
                                    name: String::from("checked"),
                                    value: String::new(),
                                });
                            }
                        } else if let Some(pos) = attrs.iter().position(|a| a.name == "checked") {
                            attrs.remove(pos);
                        }
                    }
                    if fc.control_id == 0 {
                        continue;
                    }
                    ui::Control::from_id(fc.control_id).set_state(if checked { 1 } else { 0 });
                }
                FormFieldKind::Textarea => {
                    if fc.control_id == 0 {
                        continue;
                    }
                    let default_val = dom.text_content(fc.node_id);
                    ui::Control::from_id(fc.control_id).set_text(default_val.trim());
                }
                FormFieldKind::Select => {
                    if fc.control_id == 0 {
                        let option_nodes = Self::select_option_nodes(dom, fc.node_id);
                        for option_id in option_nodes {
                            if let dom::NodeType::Element { ref mut attrs, .. } =
                                dom.get_mut(option_id).node_type
                            {
                                let selected = attrs
                                    .iter()
                                    .find(|a| a.name == "data-webview-default-selected")
                                    .map(|a| a.value.as_str() == "1")
                                    .unwrap_or_else(|| attrs.iter().any(|a| a.name == "selected"));
                                if selected {
                                    if attrs.iter().all(|a| a.name != "selected") {
                                        attrs.push(dom::Attr {
                                            name: String::from("selected"),
                                            value: String::new(),
                                        });
                                    }
                                } else if let Some(pos) =
                                    attrs.iter().position(|a| a.name == "selected")
                                {
                                    attrs.remove(pos);
                                }
                            }
                        }
                        continue;
                    }
                    // Reset to the initially-selected option (first with `selected` attr).
                    let mut sel_idx: u32 = 0;
                    let mut idx: u32 = 0;
                    let children = &dom.get(fc.node_id).children;
                    for &cid in children {
                        if dom.tag(cid) == Some(dom::Tag::Option) {
                            if dom.attr(cid, "selected").is_some() {
                                sel_idx = idx;
                                break;
                            }
                            idx += 1;
                        }
                    }
                    ui::Control::from_id(fc.control_id).set_state(sel_idx);
                }
                FormFieldKind::Range => {
                    if fc.control_id == 0 {
                        if let dom::NodeType::Element { ref mut attrs, .. } =
                            dom.get_mut(fc.node_id).node_type
                        {
                            let default_val = attrs
                                .iter()
                                .find(|a| a.name == "data-webview-default-value")
                                .map(|a| a.value.clone())
                                .unwrap_or_else(|| {
                                    attrs
                                        .iter()
                                        .find(|a| a.name == "value")
                                        .map(|a| a.value.clone())
                                        .unwrap_or_default()
                                });
                            if let Some(attr) = attrs.iter_mut().find(|a| a.name == "value") {
                                attr.value = default_val;
                            } else {
                                attrs.push(dom::Attr {
                                    name: String::from("value"),
                                    value: default_val,
                                });
                            }
                        }
                        continue;
                    }
                    let min_f: f32 = dom
                        .attr(fc.node_id, "min")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0.0);
                    let max_f: f32 = dom
                        .attr(fc.node_id, "max")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(100.0);
                    let val_f: f32 = dom
                        .attr(fc.node_id, "value")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(50.0);
                    let pct = if max_f > min_f {
                        ((val_f - min_f) / (max_f - min_f) * 100.0) as u32
                    } else {
                        50
                    };
                    ui::Control::from_id(fc.control_id).set_state(pct.min(100));
                }
                FormFieldKind::Number => {
                    if fc.control_id == 0 {
                        continue;
                    }
                    let default_val = dom.attr(fc.node_id, "value").unwrap_or("");
                    ui::Control::from_id(fc.control_id).set_text(default_val);
                }
                FormFieldKind::Date
                | FormFieldKind::Month
                | FormFieldKind::Week
                | FormFieldKind::Time
                | FormFieldKind::DatetimeLocal => {
                    if fc.control_id == 0 {
                        continue;
                    }
                    // Parse the default value attribute and pack into state.
                    let default_val = dom.attr(fc.node_id, "value").unwrap_or("");
                    let packed = parse_value_to_packed(default_val, fc.kind);
                    ui::Control::from_id(fc.control_id).set_state(packed);
                }
                FormFieldKind::Color => {
                    if fc.control_id == 0 {
                        if let dom::NodeType::Element { ref mut attrs, .. } =
                            dom.get_mut(fc.node_id).node_type
                        {
                            let default_val = attrs
                                .iter()
                                .find(|a| a.name == "data-webview-default-value")
                                .map(|a| a.value.clone())
                                .unwrap_or_else(|| {
                                    attrs
                                        .iter()
                                        .find(|a| a.name == "value")
                                        .map(|a| a.value.clone())
                                        .unwrap_or_else(|| String::from("#000000"))
                                });
                            if let Some(attr) = attrs.iter_mut().find(|a| a.name == "value") {
                                attr.value = default_val;
                            } else {
                                attrs.push(dom::Attr {
                                    name: String::from("value"),
                                    value: default_val,
                                });
                            }
                        }
                        continue;
                    }
                    let default_val = dom.attr(fc.node_id, "value").unwrap_or("#000000");
                    let color = renderer::parse_color_value(default_val);
                    ui::Control::from_id(fc.control_id).set_state(color);
                }
                _ => {}
            }
        }

        let _ = dom;
        self.relayout();
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

    /// Find the topmost DOM node at viewport position `(vx, vy)`.
    pub fn hit_test_node_viewport(&self, vx: i32, vy: i32, scroll_y: i32) -> Option<usize> {
        self.hit_test_node_document(vx, scroll_y + vy)
    }

    /// Find the topmost DOM node hit on a tile canvas control.
    pub fn hit_test_node_canvas(&self, canvas_ctrl_id: u32) -> Option<usize> {
        let (mx, doc_y) = self.renderer.tile_hit_coords(canvas_ctrl_id)?;
        self.hit_test_node_document(mx, doc_y)
    }

    /// Dispatch a DOM click for a rendered control/canvas hit.
    ///
    /// Returns `true` when the browser default action may continue.
    pub fn dispatch_click_for_control(&mut self, control_id: u32) -> bool {
        let node_id = self
            .node_id_for_control(control_id)
            .or_else(|| self.hit_test_node_canvas(control_id));
        let Some(node_id) = node_id else {
            return true;
        };

        let data = js::EventData::Mouse {
            client_x: 0.0,
            client_y: 0.0,
            page_x: 0.0,
            page_y: 0.0,
            screen_x: 0.0,
            screen_y: 0.0,
            offset_x: 0.0,
            offset_y: 0.0,
            button: 0,
            buttons: 0,
            ctrl_key: false,
            shift_key: false,
            alt_key: false,
            meta_key: false,
        };
        self.dispatch_dom_event_and_apply(node_id, "click", &data)
    }

    /// Dispatch the keyboard events produced by pressing Enter in a native
    /// form control. Returns `true` when the default submit action may run.
    pub fn dispatch_enter_for_control(&mut self, control_id: u32) -> bool {
        let Some(node_id) = self.node_id_for_control(control_id) else {
            return true;
        };
        let data = js::EventData::Keyboard {
            key: String::from("Enter"),
            code: String::from("Enter"),
            key_code: 13,
            which: 13,
            char_code: 13,
            ctrl_key: false,
            shift_key: false,
            alt_key: false,
            meta_key: false,
            repeat: false,
            is_composing: false,
        };
        let keydown_allowed = self.dispatch_dom_event_and_apply(node_id, "keydown", &data);
        self.dispatch_dom_event_and_apply(node_id, "keypress", &data);
        self.dispatch_dom_event_and_apply(node_id, "keyup", &data);
        keydown_allowed
    }

    /// Dispatch a cancelable `submit` event for the form containing `node_id`.
    pub fn dispatch_submit_for_node(&mut self, node_id: usize) -> bool {
        let Some(form_id) = self.find_form_for_node(node_id) else {
            return true;
        };
        self.dispatch_dom_event_and_apply(form_id, "submit", &js::EventData::None)
    }

    /// Dispatch a cancelable `submit` event for the form containing a control.
    pub fn dispatch_submit_for_control(&mut self, control_id: u32) -> bool {
        let node_id = self
            .canvas_submit_hit(control_id)
            .or_else(|| self.node_id_for_control(control_id));
        let Some(node_id) = node_id else {
            return true;
        };
        self.dispatch_submit_for_node(node_id)
    }

    fn dispatch_dom_event_and_apply(
        &mut self,
        node_id: usize,
        event_name: &str,
        data: &js::EventData,
    ) -> bool {
        let mut dom = match self.dom_val.take() {
            Some(d) => d,
            None => return true,
        };
        self.sync_native_form_controls_into_dom(&mut dom);
        let default_allowed = self.js_runtime.dispatch_event(&dom, node_id, event_name, data);
        if !self.js_runtime.mutations.is_empty() {
            self.flush_pending_mutations(&mut dom);
            self.dom_val = Some(dom);
            self.relayout();
        } else {
            self.dom_val = Some(dom);
        }
        default_allowed
    }

    fn sync_native_form_controls_into_dom(&self, dom: &mut dom::Dom) {
        for fc in &self.renderer.form_controls {
            if fc.control_id == 0 || fc.node_id >= dom.nodes.len() {
                continue;
            }

            match fc.kind {
                FormFieldKind::TextInput
                | FormFieldKind::Password
                | FormFieldKind::Number
                | FormFieldKind::Textarea => {
                    let ctrl = ui::Control::from_id(fc.control_id);
                    let mut buf = [0u8; 8192];
                    let len = ctrl.get_text(&mut buf);
                    if let Ok(value) = core::str::from_utf8(&buf[..len as usize]) {
                        dom.set_attr(fc.node_id, "value", value);
                    }
                }
                FormFieldKind::Checkbox | FormFieldKind::Radio => {
                    let ctrl = ui::Control::from_id(fc.control_id);
                    if ctrl.get_state() != 0 {
                        dom.set_attr(fc.node_id, "checked", "checked");
                    } else {
                        dom.remove_attr(fc.node_id, "checked");
                    }
                }
                FormFieldKind::Range => {
                    let ctrl = ui::Control::from_id(fc.control_id);
                    let pct = ctrl.get_state() as f32 / 100.0;
                    let min_f: f32 = dom
                        .attr(fc.node_id, "min")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0.0);
                    let max_f: f32 = dom
                        .attr(fc.node_id, "max")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(100.0);
                    let raw = min_f + pct * (max_f - min_f);
                    let value = format_range_value(raw.min(max_f).max(min_f));
                    dom.set_attr(fc.node_id, "value", &value);
                }
                FormFieldKind::Color => {
                    let ctrl = ui::Control::from_id(fc.control_id);
                    let argb = ctrl.get_state();
                    let r = (argb >> 16) & 0xFF;
                    let g = (argb >> 8) & 0xFF;
                    let b = argb & 0xFF;
                    let mut hex = String::from("#");
                    let hex_digit = |n: u32| -> char {
                        if n < 10 {
                            (b'0' + n as u8) as char
                        } else {
                            (b'a' + (n - 10) as u8) as char
                        }
                    };
                    hex.push(hex_digit(r >> 4));
                    hex.push(hex_digit(r & 0xF));
                    hex.push(hex_digit(g >> 4));
                    hex.push(hex_digit(g & 0xF));
                    hex.push(hex_digit(b >> 4));
                    hex.push(hex_digit(b & 0xF));
                    dom.set_attr(fc.node_id, "value", &hex);
                }
                _ => {}
            }
        }
    }

    /// Check if a canvas click hit a form control (TextInput/Textarea).
    /// If so, focus the control and return true.
    /// Also handles `<label for="id">` clicks — focuses the associated control.
    pub fn focus_form_control_at_canvas(&mut self, canvas_ctrl_id: u32) -> bool {
        if let Some((mx, doc_y)) = self.renderer.tile_hit_coords(canvas_ctrl_id) {
            // Direct form control hit.
            if let Some(fc_id) = self.renderer.hit_test_form_at(mx, doc_y) {
                ui::Control::from_id(fc_id).focus();
                if let Some(node_id) = self
                    .renderer
                    .form_controls
                    .iter()
                    .find(|fc| fc.control_id == fc_id)
                    .map(|fc| fc.node_id)
                {
                    self.set_focused_node(Some(node_id), true);
                }
                return true;
            }

            // Label-for-control: check if we hit a <label for="..."> and focus
            // the referenced form control (HTML §4.10.4).
            if let Some(dom) = self.dom_val.as_ref() {
                if let Some(hit_node) = self.hit_test_node_document(mx, doc_y) {
                    // Walk up from hit node to find an ancestor <label>.
                    let mut cur = Some(hit_node);
                    while let Some(nid) = cur {
                        if dom.tag(nid) == Some(dom::Tag::Label) {
                            if let Some(for_id) = dom.attr(nid, "for") {
                                // Find the form control with matching id attribute.
                                if let Some(target_node) = self.find_node_by_id(for_id) {
                                    if let Some(fc) = self
                                        .renderer
                                        .form_controls
                                        .iter()
                                        .find(|fc| fc.node_id == target_node)
                                    {
                                        if fc.control_id != 0 {
                                            ui::Control::from_id(fc.control_id).focus();
                                            self.set_focused_node(Some(target_node), true);
                                            return true;
                                        }
                                    }
                                }
                            } else {
                                // Implicit label association: <label> wrapping a control.
                                // Find the first form control descendant.
                                if let Some(fc) = self.find_first_control_in_label(nid) {
                                    if fc.control_id != 0 {
                                        ui::Control::from_id(fc.control_id).focus();
                                        self.set_focused_node(Some(fc.node_id), true);
                                        return true;
                                    }
                                }
                            }
                            break;
                        }
                        cur = dom.get(nid).parent;
                    }
                }
            }
        }
        false
    }

    /// Find a DOM node by its `id` attribute.
    fn find_node_by_id(&self, id: &str) -> Option<usize> {
        let dom = self.dom_val.as_ref()?;
        for i in 0..dom.nodes.len() {
            if dom.attr(i, "id") == Some(id) {
                return Some(i);
            }
        }
        None
    }

    /// Find the first form control that is a descendant of a <label> node.
    /// Used for implicit label association (HTML §4.10.4).
    fn find_first_control_in_label(&self, label_node: usize) -> Option<&renderer::FormControl> {
        let dom = self.dom_val.as_ref()?;
        // BFS through label's children.
        let mut stack = dom.get(label_node).children.clone();
        while let Some(nid) = stack.pop() {
            if let Some(fc) = self
                .renderer
                .form_controls
                .iter()
                .find(|fc| fc.node_id == nid)
            {
                return Some(fc);
            }
            let children = &dom.get(nid).children;
            for &child in children.iter().rev() {
                stack.push(child);
            }
        }
        None
    }

    pub fn set_hovered_node(&mut self, node_id: Option<usize>) {
        self.selector_state.hovered_node = node_id;
    }

    pub fn set_active_node(&mut self, node_id: Option<usize>) {
        self.selector_state.active_node = node_id;
    }

    pub fn set_focused_node(&mut self, node_id: Option<usize>, focus_visible: bool) {
        self.selector_state.focused_node = node_id;
        self.selector_state.focus_visible_node = if focus_visible { node_id } else { None };
    }

    pub fn clear_selector_state(&mut self) {
        self.selector_state = style::SelectorState::default();
    }

    fn hit_test_node_document(&self, doc_x: i32, doc_y: i32) -> Option<usize> {
        let root = self.layout_root.as_ref()?;
        Self::hit_test_layout_box(root, 0, 0, doc_x, doc_y)
    }

    fn hit_test_layout_box(
        bx: &LayoutBox,
        offset_x: i32,
        offset_y: i32,
        doc_x: i32,
        doc_y: i32,
    ) -> Option<usize> {
        let abs_x = if bx.is_fixed { bx.x } else { offset_x + bx.x };
        let abs_y = if bx.is_fixed { bx.y } else { offset_y + bx.y };

        for child in bx.children.iter().rev() {
            if let Some(node_id) = Self::hit_test_layout_box(child, abs_x, abs_y, doc_x, doc_y) {
                return Some(node_id);
            }
        }

        if doc_x >= abs_x && doc_x < abs_x + bx.width && doc_y >= abs_y && doc_y < abs_y + bx.height
        {
            return bx.node_id;
        }

        None
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

    pub fn node_id_for_control(&self, control_id: u32) -> Option<usize> {
        self.renderer
            .form_controls
            .iter()
            .find(|fc| fc.control_id == control_id)
            .map(|fc| fc.node_id)
    }

    /// Find the form action URL, method, and enctype for a submit button identified by DOM node_id.
    /// Returns (action, method, enctype).
    pub fn form_action_for_node(&self, node_id: usize) -> Option<(String, String, String)> {
        let dom = self.dom_val.as_ref()?;
        let id = self.find_form_for_node(node_id)?;
        let action = dom.attr(id, "action").unwrap_or("");
        let method = dom.attr(id, "method").unwrap_or("GET");
        let enctype = dom
            .attr(id, "enctype")
            .unwrap_or("application/x-www-form-urlencoded");
        return Some((
            String::from(action),
            method.to_ascii_uppercase(),
            String::from(enctype),
        ));
    }

    fn find_form_for_node(&self, node_id: usize) -> Option<usize> {
        let dom = self.dom_val.as_ref()?;
        Self::find_form_for_node_in_dom(dom, node_id)
    }

    fn find_form_for_node_in_dom(dom: &dom::Dom, node_id: usize) -> Option<usize> {
        if let Some(form_attr) = dom.attr(node_id, "form") {
            for id in 0..dom.nodes.len() {
                if dom.tag(id) == Some(dom::Tag::Form) && dom.attr(id, "id") == Some(form_attr) {
                    return Some(id);
                }
            }
        }

        let mut cur = Some(node_id);
        while let Some(id) = cur {
            if dom.tag(id) == Some(dom::Tag::Form) {
                return Some(id);
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

        let form_id = match Self::find_form_for_node_in_dom(dom, node_id) {
            Some(id) => id,
            None => return Vec::new(),
        };

        // Collect all form controls that are descendants of this form.
        let mut data = Vec::new();
        for fc in &self.renderer.form_controls {
            if Self::find_form_for_node_in_dom(dom, fc.node_id) != Some(form_id) {
                continue;
            }

            let name = dom.attr(fc.node_id, "name").unwrap_or("");
            if name.is_empty() {
                continue;
            }

            match fc.kind {
                FormFieldKind::TextInput | FormFieldKind::Password | FormFieldKind::Number => {
                    if fc.control_id == 0 {
                        continue;
                    }
                    let ctrl = ui::Control::from_id(fc.control_id);
                    let mut buf = [0u8; 2048];
                    let len = ctrl.get_text(&mut buf);
                    let val = core::str::from_utf8(&buf[..len as usize]).unwrap_or("");
                    data.push((String::from(name), String::from(val)));
                }
                FormFieldKind::Date | FormFieldKind::Month | FormFieldKind::Week => {
                    if fc.control_id == 0 {
                        continue;
                    }
                    // DatePicker stores packed u32: unpack to ISO date string.
                    let ctrl = ui::Control::from_id(fc.control_id);
                    let packed = ctrl.get_state();
                    let val = format_packed_date(packed, fc.kind);
                    data.push((String::from(name), val));
                }
                FormFieldKind::Time => {
                    if fc.control_id == 0 {
                        continue;
                    }
                    let ctrl = ui::Control::from_id(fc.control_id);
                    let packed = ctrl.get_state();
                    let val = format_packed_time(packed);
                    data.push((String::from(name), val));
                }
                FormFieldKind::DatetimeLocal => {
                    if fc.control_id == 0 {
                        continue;
                    }
                    let ctrl = ui::Control::from_id(fc.control_id);
                    let packed = ctrl.get_state();
                    let date_part = format_packed_date(packed, FormFieldKind::Date);
                    let time_part = format_packed_time(packed);
                    let mut val = date_part;
                    val.push('T');
                    val.push_str(&time_part);
                    data.push((String::from(name), val));
                }
                FormFieldKind::Color => {
                    if fc.control_id == 0 {
                        let val = dom.attr(fc.node_id, "value").unwrap_or("#000000");
                        data.push((String::from(name), String::from(val)));
                        continue;
                    }
                    // ColorWell stores the color as u32 ARGB via get_state().
                    let ctrl = ui::Control::from_id(fc.control_id);
                    let argb = ctrl.get_state();
                    let r = (argb >> 16) & 0xFF;
                    let g = (argb >> 8) & 0xFF;
                    let b = argb & 0xFF;
                    let mut hex = String::from("#");
                    let hex_digit = |n: u32| -> char {
                        if n < 10 {
                            (b'0' + n as u8) as char
                        } else {
                            (b'a' + (n - 10) as u8) as char
                        }
                    };
                    hex.push(hex_digit(r >> 4));
                    hex.push(hex_digit(r & 0xF));
                    hex.push(hex_digit(g >> 4));
                    hex.push(hex_digit(g & 0xF));
                    hex.push(hex_digit(b >> 4));
                    hex.push(hex_digit(b & 0xF));
                    data.push((String::from(name), hex));
                }
                FormFieldKind::Checkbox => {
                    if fc.control_id == 0 {
                        let checked = dom.attr(fc.node_id, "checked").is_some();
                        if checked {
                            let val = dom.attr(fc.node_id, "value").unwrap_or("on");
                            data.push((String::from(name), String::from(val)));
                        }
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
                        let checked = dom.attr(fc.node_id, "checked").is_some();
                        if checked {
                            let val = dom.attr(fc.node_id, "value").unwrap_or("");
                            data.push((String::from(name), String::from(val)));
                        }
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
                FormFieldKind::Select => {
                    if fc.control_id == 0 {
                        let option_nodes = Self::select_option_nodes(dom, fc.node_id);
                        let mut selected_value = None;
                        for option_id in option_nodes {
                            if dom.attr(option_id, "selected").is_some() {
                                let txt = dom.text_content(option_id);
                                let val = dom.attr(option_id, "value").unwrap_or(txt.trim());
                                selected_value = Some(String::from(val));
                                break;
                            }
                        }
                        let val = selected_value.unwrap_or_default();
                        data.push((String::from(name), val));
                        continue;
                    }
                    // Get selected index from the native DropDown widget.
                    let ctrl = ui::Control::from_id(fc.control_id);
                    let sel_idx = ctrl.get_state() as usize;
                    let val = self.select_option_value(dom, fc.node_id, sel_idx);
                    data.push((String::from(name), val));
                }
                FormFieldKind::Range => {
                    if fc.control_id == 0 {
                        let val = dom.attr(fc.node_id, "value").unwrap_or("50");
                        data.push((String::from(name), String::from(val)));
                        continue;
                    }
                    // Slider state is 0..100. Map back to min..max range.
                    let ctrl = ui::Control::from_id(fc.control_id);
                    let pct = ctrl.get_state() as f32 / 100.0;
                    let min_f: f32 = dom
                        .attr(fc.node_id, "min")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0.0);
                    let max_f: f32 = dom
                        .attr(fc.node_id, "max")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(100.0);
                    let step_f: f32 = dom
                        .attr(fc.node_id, "step")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(1.0);
                    let raw = min_f + pct * (max_f - min_f);
                    let snapped = if step_f > 0.0 {
                        let steps = (raw - min_f) / step_f;
                        let rounded_steps = if steps >= 0.0 {
                            (steps + 0.5) as i32
                        } else {
                            (steps - 0.5) as i32
                        } as f32;
                        (rounded_steps * step_f + min_f).min(max_f).max(min_f)
                    } else {
                        raw
                    };
                    let val = format_range_value(snapped);
                    data.push((String::from(name), val));
                }
                _ => {}
            }
        }
        data
    }

    /// Look up the value of the nth `<option>` within a `<select>` node.
    /// Walks the DOM children (including `<optgroup>` wrappers).
    fn select_option_value(&self, dom: &dom::Dom, select_node: usize, idx: usize) -> String {
        let mut count = 0usize;
        let children = &dom.get(select_node).children;
        for &cid in children {
            if dom.tag(cid) == Some(dom::Tag::Optgroup) {
                // Optgroup label entry.
                if count == idx {
                    return String::new(); // optgroup separator has no value
                }
                count += 1;
                let group_children = &dom.get(cid).children;
                for &gcid in group_children {
                    if dom.tag(gcid) == Some(dom::Tag::Option) {
                        if count == idx {
                            let txt = dom.text_content(gcid);
                            let val = dom.attr(gcid, "value").unwrap_or(txt.trim());
                            return String::from(val);
                        }
                        count += 1;
                    }
                }
            } else if dom.tag(cid) == Some(dom::Tag::Option) {
                if count == idx {
                    let txt = dom.text_content(cid);
                    let val = dom.attr(cid, "value").unwrap_or(txt.trim());
                    return String::from(val);
                }
                count += 1;
            }
        }
        String::new()
    }

    fn current_scroll_offsets_for_node(&self, node_id: usize) -> (i32, i32) {
        self.scroll_offsets
            .iter()
            .find(|(id, _, _)| *id == node_id)
            .map(|(_, top, left)| (*top, *left))
            .unwrap_or((0, 0))
    }

    fn node_uses_smooth_scroll(&self, node_id: usize) -> bool {
        self.resolved_styles_cache
            .get(node_id)
            .map(|style| style.scroll_behavior == style::ScrollBehaviorVal::Smooth)
            .unwrap_or(false)
    }

    fn set_scroll_offsets_for_node(&mut self, node_id: usize, top: i32, left: i32) {
        if let Some(entry) = self
            .scroll_offsets
            .iter_mut()
            .find(|(id, _, _)| *id == node_id)
        {
            entry.1 = top.max(0);
            entry.2 = left.max(0);
        } else {
            self.scroll_offsets.push((node_id, top.max(0), left.max(0)));
        }
    }

    fn cancel_smooth_scroll(&mut self, node_id: usize) {
        self.smooth_scrolls
            .retain(|scroll| scroll.node_id != node_id);
    }

    fn start_or_update_smooth_scroll(&mut self, node_id: usize, target_top: i32, target_left: i32) {
        let (start_top, start_left) = self.current_scroll_offsets_for_node(node_id);
        if let Some(existing) = self
            .smooth_scrolls
            .iter_mut()
            .find(|scroll| scroll.node_id == node_id)
        {
            existing.start_top = start_top;
            existing.start_left = start_left;
            existing.target_top = target_top.max(0);
            existing.target_left = target_left.max(0);
            existing.elapsed_ms = 0;
            return;
        }
        self.smooth_scrolls.push(PendingSmoothScroll {
            node_id,
            start_top,
            start_left,
            target_top: target_top.max(0),
            target_left: target_left.max(0),
            elapsed_ms: 0,
            duration_ms: 220,
        });
    }

    /// Extract scroll offset mutations from the pending mutation list and merge
    /// them into `self.scroll_offsets`, optionally scheduling smooth scrolling.
    /// Must be called *before* `apply_mutations` which consumes the mutation vec.
    fn extract_scroll_offsets(&mut self) {
        #[derive(Clone, Copy)]
        struct ScrollRequest {
            node_id: usize,
            top: Option<i32>,
            left: Option<i32>,
            smooth: Option<bool>,
        }

        let mut requests: Vec<ScrollRequest> = Vec::new();
        for m in &self.js_runtime.mutations {
            match m {
                js::DomMutation::SetScrollTop {
                    node_id,
                    value,
                    smooth,
                } => {
                    if let Some(req) = requests.iter_mut().find(|req| req.node_id == *node_id) {
                        req.top = Some(*value);
                        if smooth.is_some() {
                            req.smooth = *smooth;
                        }
                    } else {
                        requests.push(ScrollRequest {
                            node_id: *node_id,
                            top: Some(*value),
                            left: None,
                            smooth: *smooth,
                        });
                    }
                }
                js::DomMutation::SetScrollLeft {
                    node_id,
                    value,
                    smooth,
                } => {
                    if let Some(req) = requests.iter_mut().find(|req| req.node_id == *node_id) {
                        req.left = Some(*value);
                        if smooth.is_some() {
                            req.smooth = *smooth;
                        }
                    } else {
                        requests.push(ScrollRequest {
                            node_id: *node_id,
                            top: None,
                            left: Some(*value),
                            smooth: *smooth,
                        });
                    }
                }
                _ => {}
            }
        }

        for req in requests {
            let (current_top, current_left) = self.current_scroll_offsets_for_node(req.node_id);
            let target_top = req.top.unwrap_or(current_top).max(0);
            let target_left = req.left.unwrap_or(current_left).max(0);
            let use_smooth = req
                .smooth
                .unwrap_or_else(|| self.node_uses_smooth_scroll(req.node_id));
            if use_smooth && (target_top != current_top || target_left != current_left) {
                self.start_or_update_smooth_scroll(req.node_id, target_top, target_left);
            } else {
                self.cancel_smooth_scroll(req.node_id);
                self.set_scroll_offsets_for_node(req.node_id, target_top, target_left);
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

    fn advance_smooth_scrolls(&mut self, delta_ms: u64) -> bool {
        if self.smooth_scrolls.is_empty() {
            return false;
        }

        let step_ms = delta_ms.min(u32::MAX as u64) as u32;
        let mut changed = false;
        let mut updates: Vec<(usize, i32, i32)> = Vec::new();
        for idx in (0..self.smooth_scrolls.len()).rev() {
            let scroll = &mut self.smooth_scrolls[idx];
            scroll.elapsed_ms = scroll.elapsed_ms.saturating_add(step_ms);
            let duration = scroll.duration_ms.max(1);
            let t =
                (scroll.elapsed_ms.min(duration) as i32 * 10000 / duration as i32).clamp(0, 10000);
            let eased = style::apply_timing(style::TimingFunction::EaseInOut, t);
            let top = scroll.start_top
                + (((scroll.target_top - scroll.start_top) as i64 * eased as i64) / 10000) as i32;
            let left = scroll.start_left
                + (((scroll.target_left - scroll.start_left) as i64 * eased as i64) / 10000) as i32;
            updates.push((scroll.node_id, top, left));
            changed = true;
            if scroll.elapsed_ms >= duration {
                updates.push((scroll.node_id, scroll.target_top, scroll.target_left));
                self.smooth_scrolls.swap_remove(idx);
            }
        }

        if changed {
            for (node_id, top, left) in updates {
                self.set_scroll_offsets_for_node(node_id, top, left);
            }
            if let Some(root) = self.layout_root.as_mut() {
                Self::apply_scroll_offsets_to_layout(&self.scroll_offsets, root);
            }
            self.repaint_from_cached_layout();
        }
        changed
    }

    fn classify_pending_mutations(&self) -> MutationImpact {
        let mut impact = MutationImpact::None;
        for m in &self.js_runtime.mutations {
            match m {
                js::DomMutation::SetCookie { .. }
                | js::DomMutation::FormSubmit { .. }
                | js::DomMutation::FormReset { .. } => {}
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
        self.total_height_val =
            normalize_document_height(calc_total_height(&root), self.viewport_height);
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
        self.total_height_val =
            normalize_document_height(calc_total_height(&root), self.viewport_height);
        self.layout_root = Some(root);
        self.refresh_render_surface_for_layout(dom, styles);
        true
    }

    fn refresh_render_surface_for_layout(
        &mut self,
        dom: &dom::Dom,
        styles: &[style::ComputedStyle],
    ) {
        let bg_color = resolve_root_background_color(dom, styles);
        self.content_view.set_color(bg_color);
        self.bg_color_cached = bg_color;
        let doc_w = self.viewport_width.max(1) as u32;
        let doc_h = (self.total_height_val as u32).max(1);
        self.content_view.set_size(doc_w, doc_h);

        let Some(root) = self.layout_root.as_ref() else {
            return;
        };

        #[cfg(feature = "host")]
        if std::env::var_os("SURF_DEBUG_RENDER_REFRESH").is_some() {
            eprintln!(
                "[libwebview] refresh render surface: doc={}x{} root_h={}",
                doc_w,
                doc_h,
                root.height
            );
        }

        // Cached-style relayouts can change geometry, so a paint-only refresh
        // would reuse a stale display list and stale tile commands. Rebuild the
        // render surface from the new layout tree instead.
        self.renderer.clear();
        self.pending_tiles = self.renderer.render(
            root,
            &self.content_view,
            &self.images,
            doc_w,
            doc_h,
            self.viewport_height,
            0,
            bg_color,
            self.link_cb,
            self.link_cb_ud,
            self.submit_cb,
            self.submit_cb_ud,
            false,
        );
        self.last_render_scroll_y = 0;
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
                let parent_style = d
                    .nodes
                    .get(*node_id)
                    .and_then(|n| n.parent)
                    .and_then(|pid| styles.get(pid))
                    .cloned();
                let parent_fs = {
                    let pid = d.nodes.get(*node_id).and_then(|n| n.parent).unwrap_or(0);
                    if pid < styles.len() {
                        styles[pid].font_size
                    } else {
                        root_fs
                    }
                };
                for decl in decls {
                    style::apply_declaration(
                        &mut styles[*node_id],
                        decl,
                        parent_style.as_ref(),
                        parent_fs,
                        root_fs,
                    );
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
        let layout_budget = self.layout_budget_for_document(d);
        let mut root = layout::layout_with_budget(
            d,
            &styles,
            &mut pseudo_styles,
            self.viewport_width,
            self.viewport_height as i32,
            &self.images,
            layout_budget,
            None,
        );
        if !self.scroll_offsets.is_empty() {
            Self::apply_scroll_offsets_to_layout(&self.scroll_offsets, &mut root);
        }
        self.total_height_val =
            normalize_document_height(calc_total_height(&root), self.viewport_height);
        self.layout_root = Some(root);
        self.deferred_full_layout_pending = layout_budget.is_some();
        if !self.deferred_full_layout_pending {
            self.clear_deferred_layout_state();
        }
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
            let parent_style = dom.nodes[node_id]
                .parent
                .and_then(|pid| styles.get(pid))
                .cloned();
            let decls = crate::css::parse_inline_style(&alloc::format!("{}: {}", property, value));
            for decl in &decls {
                style::apply_declaration(
                    &mut styles[node_id],
                    decl,
                    parent_style.as_ref(),
                    parent_fs,
                    root_fs,
                );
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
                self.prepared_stylesheets = None;
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
        let dom_only_first_render = self.dom_only_initial_render_pending
            && self.layout_root.is_none()
            && self.total_height_val == 0;
        self.ensure_initial_progressive_budget(d);

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
            self.prepared_stylesheets = None;
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
        let style_start_ms = anyos_std::sys::uptime_ms();
        if self.prepared_stylesheets.is_none() {
            let mut all_sheets: Vec<&css::Stylesheet> =
                Vec::with_capacity(1 + self.external_sheets.len() + self.inline_sheets.len());
            all_sheets.push(&self.default_sheet);
            for sheet in &self.external_sheets {
                all_sheets.push(sheet);
            }
            for sheet in &self.inline_sheets {
                all_sheets.push(sheet);
            }
            self.prepared_stylesheets =
                Some(style::PreparedStylesheets::prepare(&all_sheets, vw, vh));
        }
        let prepared = self.prepared_stylesheets.as_ref().unwrap();
        let style_budget = self.style_budget_for_document(d);
        let (mut styles, mut pseudo_styles) = if let Some(node_budget) = style_budget {
            style::resolve_styles_prepared_budgeted_with_state(
                d,
                prepared,
                vw,
                vh,
                &mut self.inline_style_cache,
                node_budget,
                &self.selector_state,
            )
        } else {
            style::resolve_styles_prepared_with_state(
                d,
                prepared,
                vw,
                vh,
                &mut self.inline_style_cache,
                &self.selector_state,
            )
        };
        let _style_elapsed_ms = anyos_std::sys::uptime_ms().wrapping_sub(style_start_ms);
        debug_surf!(
            "[webview] resolve_styles done: {} styles elapsed={}ms style_budget={}",
            styles.len(),
            _style_elapsed_ms,
            style_budget.unwrap_or(0)
        );

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

        // Keep the computed styles that were actually resolved, even for a
        // budgeted first pass. The cache is intentionally smaller than the DOM
        // in that state, so it will not be reused as a full-layout cache, but
        // callers such as image discovery and host debugging can still inspect
        // above-the-fold nodes.
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
        let layout_budget = self.layout_budget_for_document(d);
        let layout_start_ms = anyos_std::sys::uptime_ms();
        let mut root = layout::layout_with_budget(
            d,
            &styles,
            &mut pseudo_styles,
            self.viewport_width,
            self.viewport_height as i32,
            &self.images,
            layout_budget,
            style_budget,
        );
        // Apply JS scroll offsets (element.scrollTop/scrollLeft) to layout boxes.
        if !self.scroll_offsets.is_empty() {
            Self::apply_scroll_offsets_to_layout(&self.scroll_offsets, &mut root);
        }
        self.total_height_val =
            normalize_document_height(calc_total_height(&root), self.viewport_height);
        self.deferred_full_layout_pending = layout_budget.is_some() || style_budget.is_some();
        if !self.deferred_full_layout_pending {
            self.clear_deferred_layout_state();
        }
        let _layout_elapsed_ms = anyos_std::sys::uptime_ms().wrapping_sub(layout_start_ms);
        #[cfg(feature = "debug_surf")]
        {
            let box_count = count_layout_boxes(&root);
            debug_surf!(
                "[webview] layout done: {} boxes, height={} elapsed={}ms",
                box_count,
                self.total_height_val,
                _layout_elapsed_ms
            );
            debug_surf!(
                "[webview] deferred budgets: pending={} style_budget={} layout_budget={}px",
                self.deferred_full_layout_pending,
                self.deferred_style_node_budget,
                self.deferred_layout_budget_px
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

        // Sync content view background to the propagated root/html background.
        let bg_color = resolve_root_background_color(d, &styles);
        self.content_view.set_color(bg_color);

        // Set content view height to document height.
        let doc_w = self.viewport_width as u32;
        let doc_h = (self.total_height_val as u32).max(1);
        self.content_view.set_size(doc_w, doc_h);

        // Cache the resolved root background for scroll re-renders.
        self.bg_color_cached = bg_color;

        // Render into canvas + update form controls.
        // Initial render starts at scroll_y=0.
        debug_surf!("[webview] renderer start");
        let render_start_ms = anyos_std::sys::uptime_ms();
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
            !dom_only_first_render,
        );
        let _render_elapsed_ms = anyos_std::sys::uptime_ms().wrapping_sub(render_start_ms);
        debug_surf!(
            "[webview] renderer done: pending_tiles={} elapsed={}ms",
            self.pending_tiles,
            _render_elapsed_ms
        );
        self.last_render_scroll_y = 0;
        self.dom_only_initial_render_pending = false;
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

    /// Drain pending JS-initiated form submissions.
    /// Returns a list of form node IDs that called `form.submit()`.
    pub fn drain_form_submits(&mut self) -> Vec<usize> {
        let mut submits = Vec::new();
        self.js_runtime.mutations.retain(|m| {
            if let js::DomMutation::FormSubmit { form_node_id } = m {
                submits.push(*form_node_id);
                false // remove from mutation list
            } else {
                true
            }
        });
        submits
    }

    /// Drain pending JS-initiated form resets.
    /// Returns a list of form node IDs that called `form.reset()`.
    pub fn drain_form_resets(&mut self) -> Vec<usize> {
        let mut resets = Vec::new();
        self.js_runtime.mutations.retain(|m| {
            if let js::DomMutation::FormReset { form_node_id } = m {
                resets.push(*form_node_id);
                false
            } else {
                true
            }
        });
        resets
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

    /// Find the form action URL, method, and enctype for a submit button click.
    /// Returns (action, method, enctype).
    /// Handles both real controls and canvas-based submit hit regions.
    pub fn form_action_for(&self, control_id: u32) -> Option<(String, String, String)> {
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
                let enctype = dom
                    .attr(id, "enctype")
                    .unwrap_or("application/x-www-form-urlencoded");
                return Some((
                    String::from(action),
                    method.to_ascii_uppercase(),
                    String::from(enctype),
                ));
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

/// Format a range slider value as a decimal string.
/// Pack a date/time ISO string into the DateTimePicker u32 format.
fn pack_datetime(year: u32, month: u32, day: u32, hour: u32, minute: u32) -> u32 {
    (minute & 0x3F)
        | ((hour & 0x1F) << 6)
        | ((day & 0x1F) << 11)
        | ((month & 0x0F) << 16)
        | ((year & 0xFFF) << 20)
}

/// Parse a value attribute string into a packed DateTimePicker u32.
fn parse_value_to_packed(val: &str, kind: FormFieldKind) -> u32 {
    match kind {
        FormFieldKind::Date => {
            // "YYYY-MM-DD"
            let b = val.as_bytes();
            if b.len() >= 10 && b[4] == b'-' && b[7] == b'-' {
                let y = parse_decimal(&b[0..4]);
                let m = parse_decimal(&b[5..7]);
                let d = parse_decimal(&b[8..10]);
                pack_datetime(y, m, d, 0, 0)
            } else {
                0
            }
        }
        FormFieldKind::Time => {
            // "HH:MM"
            let b = val.as_bytes();
            if b.len() >= 5 && b[2] == b':' {
                let h = parse_decimal(&b[0..2]);
                let mi = parse_decimal(&b[3..5]);
                pack_datetime(0, 0, 0, h, mi)
            } else {
                0
            }
        }
        FormFieldKind::DatetimeLocal => {
            // "YYYY-MM-DDThh:mm"
            let parts: Vec<&str> = val.split('T').collect();
            if parts.len() >= 2 {
                let db = parts[0].as_bytes();
                let tb = parts[1].as_bytes();
                if db.len() >= 10 && tb.len() >= 5 {
                    let y = parse_decimal(&db[0..4]);
                    let mo = parse_decimal(&db[5..7]);
                    let d = parse_decimal(&db[8..10]);
                    let h = parse_decimal(&tb[0..2]);
                    let mi = parse_decimal(&tb[3..5]);
                    pack_datetime(y, mo, d, h, mi)
                } else {
                    0
                }
            } else {
                0
            }
        }
        FormFieldKind::Month => {
            // "YYYY-MM"
            let b = val.as_bytes();
            if b.len() >= 7 && b[4] == b'-' {
                let y = parse_decimal(&b[0..4]);
                let m = parse_decimal(&b[5..7]);
                pack_datetime(y, m, 1, 0, 0)
            } else {
                0
            }
        }
        FormFieldKind::Week => {
            // "YYYY-Www"
            let b = val.as_bytes();
            if b.len() >= 8 && b[4] == b'-' && b[5] == b'W' {
                let y = parse_decimal(&b[0..4]);
                let w = parse_decimal(&b[6..8]);
                pack_datetime(y, 1, w, 0, 0) // store week as day
            } else {
                0
            }
        }
        _ => 0,
    }
}

fn parse_decimal(b: &[u8]) -> u32 {
    let mut n: u32 = 0;
    for &c in b {
        if c >= b'0' && c <= b'9' {
            n = n * 10 + (c - b'0') as u32;
        }
    }
    n
}

/// Unpack DatePicker/DateTimePicker state and format as ISO date string.
/// Bit layout: minute(0-5), hour(6-10), day(11-15), month(16-19), year(20-31).
fn unpack_datetime(packed: u32) -> (u32, u32, u32, u32, u32) {
    let minute = packed & 0x3F;
    let hour = (packed >> 6) & 0x1F;
    let day = (packed >> 11) & 0x1F;
    let month = (packed >> 16) & 0x0F;
    let year = (packed >> 20) & 0xFFF;
    (year, month, day, hour, minute)
}

fn format_packed_date(packed: u32, kind: FormFieldKind) -> String {
    let (year, month, day, _, _) = unpack_datetime(packed);
    let mut s = String::new();
    // YYYY
    format_u32_padded(&mut s, year, 4);
    s.push('-');
    format_u32_padded(&mut s, month, 2);
    match kind {
        FormFieldKind::Month => {} // YYYY-MM
        FormFieldKind::Week => {
            // YYYY-Www — simplified: use day as week number.
            s.push_str("-W");
            format_u32_padded(&mut s, day.max(1), 2);
        }
        _ => {
            // YYYY-MM-DD
            s.push('-');
            format_u32_padded(&mut s, day, 2);
        }
    }
    s
}

fn format_packed_time(packed: u32) -> String {
    let (_, _, _, hour, minute) = unpack_datetime(packed);
    let mut s = String::new();
    format_u32_padded(&mut s, hour, 2);
    s.push(':');
    format_u32_padded(&mut s, minute, 2);
    s
}

/// Append a u32 as a zero-padded decimal with exactly `width` digits.
fn format_u32_padded(s: &mut String, n: u32, width: usize) {
    let mut buf = [b'0'; 8];
    let mut v = n;
    let mut i = width;
    while i > 0 {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    for &b in &buf[..width] {
        s.push(b as char);
    }
}

fn format_range_value(v: f32) -> String {
    let mut s = String::new();
    if v < 0.0 {
        s.push('-');
    }
    let abs_v = if v < 0.0 { -v } else { v };
    let i = abs_v as u32;
    let frac = ((abs_v - i as f32) * 100.0 + 0.5) as u32;
    format_u32_into(&mut s, i);
    if frac > 0 {
        s.push('.');
        if frac < 10 {
            s.push('0');
        }
        format_u32_into(&mut s, frac);
        // Strip trailing zero.
        if s.ends_with('0') {
            s.pop();
        }
    }
    s
}

fn color_to_hex(color: u32) -> String {
    let r = (color >> 16) & 0xFF;
    let g = (color >> 8) & 0xFF;
    let b = color & 0xFF;
    let mut hex = String::from("#");
    let hex_digit = |n: u32| -> char {
        if n < 10 {
            (b'0' + n as u8) as char
        } else {
            (b'a' + (n - 10) as u8) as char
        }
    };
    hex.push(hex_digit(r >> 4));
    hex.push(hex_digit(r & 0xF));
    hex.push(hex_digit(g >> 4));
    hex.push(hex_digit(g & 0xF));
    hex.push(hex_digit(b >> 4));
    hex.push(hex_digit(b & 0xF));
    hex
}

fn hsv_to_rgb_u32(h_deg: u32, s: u32, v: u32) -> u32 {
    let h = h_deg % 360;
    let region = h / 60;
    let remainder = (h % 60) * 255 / 60;
    let p = v.saturating_mul(255 - s) / 255;
    let q = v.saturating_mul(255 - (s.saturating_mul(remainder) / 255)) / 255;
    let t = v.saturating_mul(255 - (s.saturating_mul(255 - remainder) / 255)) / 255;
    let (r, g, b) = match region {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    0xFF000000 | (r << 16) | (g << 8) | b
}

/// Append a u32 as decimal text.
fn format_u32_into(s: &mut String, mut n: u32) {
    if n == 0 {
        s.push('0');
        return;
    }
    let start = s.len();
    while n > 0 {
        s.push((b'0' + (n % 10) as u8) as char);
        n /= 10;
    }
    // Reverse the digits we just pushed.
    let bytes = unsafe { s.as_bytes_mut() };
    bytes[start..].reverse();
}

/// Append an i32 as decimal text.
fn format_i32_into(s: &mut String, n: i32) {
    if n < 0 {
        s.push('-');
        format_u32_into(s, (-(n as i64)) as u32);
    } else {
        format_u32_into(s, n as u32);
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

fn build_devtools_inspector_report(webview: &WebView, dom: &dom::Dom, node_id: usize) -> String {
    let mut out = String::new();
    let node = &dom.nodes[node_id];

    out.push_str("DOM\n");
    out.push_str("----------------------------------------\n");
    append_devtools_dom_path(dom, node_id, &mut out);
    append_devtools_node_syntax(node, &mut out);

    if let Some((x, y, w, h)) = webview.node_bounds(node_id) {
        out.push_str("\nBox Model\n");
        out.push_str("----------------------------------------\n");
        out.push_str(&format!("  document: x={} y={} size={}x{}\n", x, y, w, h));
    }

    if let Some(style) = webview.resolved_style_ref(node_id) {
        out.push_str("\nComputed Style\n");
        out.push_str("----------------------------------------\n");
        out.push_str(&format!("  display:          {:?}\n", style.display));
        out.push_str(&format!("  position:         {}\n", devtools_position_name(style.position)));
        out.push_str(&format!("  color:            {}\n", devtools_css_color(style.color)));
        out.push_str(&format!(
            "  background-color: {}\n",
            devtools_css_color(style.background_color)
        ));
        out.push_str(&format!(
            "  font-family:      {}\n",
            style.font_family.as_deref().unwrap_or("(default)")
        ));
        out.push_str(&format!("  font-size:        {}px\n", style.font_size));
        out.push_str(&format!("  line-height:      {}px\n", style.line_height));
        out.push_str(&format!("  width / height:   {:?} / {:?}\n", style.width, style.height));
        out.push_str(&format!(
            "  margin:           {} {} {} {}\n",
            style.margin_top, style.margin_right, style.margin_bottom, style.margin_left
        ));
        out.push_str(&format!(
            "  border-width:     {} {} {} {}\n",
            style.border_top.width,
            style.border_right.width,
            style.border_bottom.width,
            style.border_left.width
        ));
        out.push_str(&format!(
            "  padding:          {} {} {} {}\n",
            style.padding_top, style.padding_right, style.padding_bottom, style.padding_left
        ));
        out.push_str(&format!(
            "  overflow-x / y:   {:?} / {:?}\n",
            style.overflow_x, style.overflow_y
        ));
        out.push_str(&format!("  z-index:          {}\n", style.z_index));
        out.push_str(&format!(
            "  transform:        translate({}px, {}px) scale({}, {}) rotate({})\n",
            style.transform_tx,
            style.transform_ty,
            devtools_css_num(style.transform_sx),
            devtools_css_num(style.transform_sy),
            devtools_css_num(style.transform_rotate)
        ));
    }

    out.push_str("\nMatched Rules\n");
    out.push_str("----------------------------------------\n");
    out.push_str("  Regel-/Cascade-Details werden hier aufgebaut.\n");
    out
}

fn append_devtools_dom_path(dom: &dom::Dom, mut node_id: usize, out: &mut String) {
    let mut chain: Vec<String> = Vec::new();
    loop {
        if node_id >= dom.nodes.len() {
            break;
        }
        let node = &dom.nodes[node_id];
        match &node.node_type {
            dom::NodeType::Element { tag, attrs } => {
                let mut s = String::from(tag.tag_name());
                if let Some(id) = devtools_attr_value(attrs, "id") {
                    s.push('#');
                    s.push_str(id);
                }
                if let Some(classes) = devtools_attr_value(attrs, "class") {
                    for class in classes.split_whitespace().take(2) {
                        s.push('.');
                        s.push_str(class);
                    }
                }
                chain.push(s);
            }
            dom::NodeType::Text(_) => chain.push(String::from("#text")),
        }
        match node.parent {
            Some(parent) => node_id = parent,
            None => break,
        }
    }
    chain.reverse();
    out.push_str("  ");
    for (i, item) in chain.iter().enumerate() {
        if i > 0 {
            out.push_str(" > ");
        }
        out.push_str(item);
    }
    out.push('\n');
}

fn append_devtools_node_syntax(node: &dom::DomNode, out: &mut String) {
    match &node.node_type {
        dom::NodeType::Element { tag, attrs } => {
            out.push_str("  <");
            out.push_str(tag.tag_name());
            for attr in attrs {
                out.push('\n');
                out.push_str("    ");
                out.push_str(&attr.name);
                out.push_str("=\"");
                devtools_push_escaped_preview(out, &attr.value, 160);
                out.push('"');
            }
            out.push_str("\n  >\n");
        }
        dom::NodeType::Text(text) => {
            out.push_str("  \"");
            devtools_push_escaped_preview(out, text.trim(), 300);
            out.push_str("\"\n");
        }
    }
}

fn devtools_attr_value<'a>(attrs: &'a [dom::Attr], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|a| a.name.eq_ignore_ascii_case(name))
        .map(|a| a.value.as_str())
}

fn devtools_push_escaped_preview(out: &mut String, value: &str, max_chars: usize) {
    for c in value.chars().take(max_chars) {
        match c {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '"' => out.push_str("\\\""),
            _ => out.push(c),
        }
    }
    if value.chars().count() > max_chars {
        out.push_str("...");
    }
}

fn devtools_css_color(c: u32) -> String {
    format!(
        "#{:02X}{:02X}{:02X} / {}",
        (c >> 16) & 0xFF,
        (c >> 8) & 0xFF,
        c & 0xFF,
        (c >> 24) & 0xFF
    )
}

fn devtools_css_num(v: i32) -> String {
    if v % 100 == 0 {
        format!("{}", v / 100)
    } else {
        format!("{}.{:02}", v / 100, (v.abs() % 100))
    }
}

fn devtools_position_name(v: style::Position) -> &'static str {
    match v {
        style::Position::Static => "static",
        style::Position::Relative => "relative",
        style::Position::Absolute => "absolute",
        style::Position::Fixed => "fixed",
        style::Position::Sticky => "sticky",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find_node_by_id(dom: &dom::Dom, id_value: &str) -> usize {
        dom.nodes
            .iter()
            .position(|node| {
                matches!(
                    &node.node_type,
                    dom::NodeType::Element { attrs, .. }
                        if attrs.iter().any(|a| a.name == "id" && a.value == id_value)
                )
            })
            .expect("node id")
    }

    #[test]
    fn js_inserted_elements_are_restyled_against_external_css() {
        let mut wv = WebView::new(800, 600);
        wv.set_url("https://example.test/");
        wv.set_html_no_js("<html><body><div id=\"root\"></div></body></html>");
        wv.add_stylesheet(
            r#"
            .text-white { color: #fff; }
            .surface { background-color: #020617; }
            .hero { font-size: 48px; display: flex; }
            "#,
        );
        wv.relayout();

        assert!(wv.execute_js(&[String::from(
            r#"
            const el = document.createElement('h1');
            el.id = 'hero';
            el.className = 'text-white surface hero';
            el.textContent = 'CoreVM';
            document.getElementById('root').appendChild(el);
            "#,
        )]));

        let hero = {
            let dom = wv.dom().expect("dom");
            find_node_by_id(dom, "hero")
        };
        let style = wv.resolved_style_ref(hero).expect("computed style");

        assert_eq!(style.color, 0xFFFFFFFF);
        assert_eq!(style.background_color, 0xFF020617);
        assert_eq!(style.font_size, 48);
        assert!(matches!(style.display, Display::Flex));
    }

    #[test]
    fn controls_can_be_associated_with_form_attribute() {
        let mut wv = WebView::new(800, 600);
        wv.set_url("https://example.test/");
        wv.set_html_no_js(
            r#"
            <html><body>
              <form id="search" action="/search"></form>
              <input id="q" name="q" form="search" value="test">
              <button id="go" form="search">Search</button>
            </body></html>
            "#,
        );

        let dom = wv.dom().expect("dom");
        let form = find_node_by_id(dom, "search");
        let input = find_node_by_id(dom, "q");
        let button = find_node_by_id(dom, "go");

        assert_eq!(WebView::find_form_for_node_in_dom(dom, input), Some(form));
        assert_eq!(WebView::find_form_for_node_in_dom(dom, button), Some(form));
        assert_eq!(
            wv.form_action_for_node(button),
            Some((
                String::from("/search"),
                String::from("GET"),
                String::from("application/x-www-form-urlencoded"),
            ))
        );
    }
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
pre { margin: 0; }
blockquote { margin: 16px 0; padding-left: 16px; border-left: 4px solid #ddd; }
hr { margin: 16px 0; border: none; border-top: 1px solid #ccc; }
td, th { padding: 4px 8px; }
img { max-width: 100%; }
strong, b { font-weight: bold; }
em, i { font-style: italic; }
";
