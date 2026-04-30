// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Per-tab state and navigation logic for the Surf browser.
//!
//! `TabState` holds everything associated with a single browser tab:
//! the `WebView`, URL/history, and page title.  The navigation functions
//! (`navigate`, `navigate_post`, `go_back`, `go_forward`, `reload`) submit
//! fetch requests to the background network worker and return immediately,
//! keeping the UI thread responsive.

use alloc::string::String;
use alloc::vec::Vec;

const SYSTEM_FONT_REGULAR: u32 = 0;
const SYSTEM_FONT_MONO: u32 = 4;

fn register_builtin_web_fonts(wv: &mut libwebview::WebView) {
    for family in [
        "sans-serif",
        "arial",
        "helvetica",
        "helvetica neue",
        "verdana",
        "tahoma",
        "trebuchet ms",
        "system-ui",
        "-apple-system",
        "blinkmacsystemfont",
        "segoe ui",
        "roboto",
        "lato",
        "open sans",
        "source sans pro",
        "noto sans",
        "ubuntu",
        "cantarell",
        "fira sans",
        "droid sans",
        "liberation sans",
        "serif",
        "georgia",
        "times new roman",
        "times",
        "palatino",
        "palatino linotype",
        "book antiqua",
        "linux libertine o",
        "linux libertine",
        "charter",
    ] {
        wv.register_web_font(family, SYSTEM_FONT_REGULAR);
    }

    for family in [
        "monospace",
        "courier new",
        "courier",
        "consolas",
        "monaco",
        "lucida console",
        "source code pro",
        "fira mono",
        "fira code",
        "ubuntu mono",
        "droid sans mono",
        "anonymous pro",
        "liberation mono",
    ] {
        wv.register_web_font(family, SYSTEM_FONT_MONO);
    }
}

// ═══════════════════════════════════════════════════════════
// Per-tab state
// ═══════════════════════════════════════════════════════════

/// Everything associated with one browser tab.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PageLoadPhase {
    Idle,
    FetchingDocument,
    ParsingDocument,
    LoadingSubresources,
    Interactive,
    Failed,
}

pub(crate) struct PageLoadState {
    /// Generation counter for the current navigation.
    pub(crate) generation: u32,
    /// Coarse-grained page lifecycle phase for the active document.
    pub(crate) phase: PageLoadPhase,
    /// Number of external script fetches still outstanding.
    pub(crate) pending_script_count: usize,
    /// Number of external stylesheet fetches still outstanding.
    pub(crate) pending_stylesheet_count: usize,
    /// Number of ES module chunk fetches still outstanding.
    pub(crate) pending_module_count: usize,
}

#[derive(Clone)]
pub(crate) struct DeferredImageRequest {
    pub(crate) node_id: usize,
    pub(crate) dom_index: usize,
    pub(crate) src: String,
    pub(crate) url: crate::http::Url,
    pub(crate) priority: crate::net_worker::ImagePriority,
    pub(crate) width: Option<i32>,
    pub(crate) height: Option<i32>,
    pub(crate) lazy_requested: bool,
    pub(crate) high_priority: bool,
    pub(crate) generation: u32,
}

#[derive(Clone)]
pub(crate) struct DeferredFontRequest {
    pub(crate) family: String,
    pub(crate) url: crate::http::Url,
    pub(crate) display: libwebview::css::FontDisplay,
    pub(crate) generation: u32,
}

impl PageLoadState {
    fn new() -> Self {
        Self {
            generation: 0,
            phase: PageLoadPhase::Idle,
            pending_script_count: 0,
            pending_stylesheet_count: 0,
            pending_module_count: 0,
        }
    }

    pub(crate) fn begin_navigation(&mut self, generation: u32) {
        self.generation = generation;
        self.phase = PageLoadPhase::FetchingDocument;
        self.pending_script_count = 0;
        self.pending_stylesheet_count = 0;
        self.pending_module_count = 0;
    }

    pub(crate) fn begin_parse(&mut self) {
        self.phase = PageLoadPhase::ParsingDocument;
    }

    pub(crate) fn begin_subresource_load(&mut self, stylesheets: usize, scripts: usize) {
        self.phase = if stylesheets == 0 && scripts == 0 {
            PageLoadPhase::Interactive
        } else {
            PageLoadPhase::LoadingSubresources
        };
        self.pending_stylesheet_count = stylesheets;
        self.pending_script_count = scripts;
        self.pending_module_count = 0;
    }

    pub(crate) fn on_stylesheets_added(&mut self, count: usize) {
        self.pending_stylesheet_count += count;
        if count > 0 {
            self.phase = PageLoadPhase::LoadingSubresources;
        }
    }

    pub(crate) fn on_stylesheet_finished(&mut self) {
        if self.pending_stylesheet_count > 0 {
            self.pending_stylesheet_count -= 1;
        }
    }

    pub(crate) fn on_script_finished(&mut self) {
        if self.pending_script_count > 0 {
            self.pending_script_count -= 1;
        }
    }

    pub(crate) fn on_module_added(&mut self, count: usize) {
        self.pending_module_count += count;
        if count > 0 {
            self.phase = PageLoadPhase::LoadingSubresources;
        }
    }

    pub(crate) fn on_module_finished(&mut self) {
        if self.pending_module_count > 0 {
            self.pending_module_count -= 1;
        }
    }

    pub(crate) fn mark_interactive(&mut self) {
        self.phase = PageLoadPhase::Interactive;
        self.pending_script_count = 0;
        self.pending_module_count = 0;
    }

    pub(crate) fn mark_failed(&mut self) {
        self.phase = PageLoadPhase::Failed;
        self.pending_script_count = 0;
        self.pending_stylesheet_count = 0;
        self.pending_module_count = 0;
    }

    pub(crate) fn generation_matches(&self, generation: u32) -> bool {
        self.generation == generation
    }

    pub(crate) fn ready_for_script_execution(&self) -> bool {
        self.pending_script_count == 0
            && self.pending_stylesheet_count == 0
            && self.pending_module_count == 0
    }
}

pub(crate) struct TabState {
    /// HTML rendering widget for this tab.
    pub(crate) webview: libwebview::WebView,
    /// Displayed URL text (may differ from `current_url` during redirects).
    pub(crate) url_text: String,
    /// Parsed URL of the currently loaded page.
    pub(crate) current_url: Option<crate::http::Url>,
    /// `<title>` extracted from the last loaded page.
    pub(crate) page_title: String,
    /// Navigation history stack (fully-qualified URL strings).
    pub(crate) history: Vec<String>,
    /// Current position within `history` (0 = oldest entry).
    pub(crate) history_pos: usize,
    /// Short status string shown in the status bar.
    pub(crate) status_text: String,
    /// Whether this tab is currently loading (navigation in progress).
    pub(crate) is_loading: bool,
    /// Structured loader state for the current page.
    pub(crate) load_state: PageLoadState,
    /// Pending script slots — one entry per `<script>` tag in document order.
    /// `Some(text)` = fetched but not executed yet, `None` = either still
    /// pending externally or already executed.
    pub(crate) pending_scripts: Vec<Option<String>>,
    /// Execution mode for each pending script slot.
    pub(crate) pending_script_modes: Vec<libwebview::js::ScriptMode>,
    /// Human-readable script label per slot for logging/debugging.
    pub(crate) pending_script_labels: Vec<String>,
    /// Absolute module chunk URLs already queued for this navigation.
    pub(crate) requested_module_urls: Vec<String>,
    /// Web fonts intentionally deferred until after first paint / interactive.
    pub(crate) deferred_fonts: Vec<DeferredFontRequest>,
    /// Absolute font URLs already queued for this navigation.
    pub(crate) requested_font_urls: Vec<String>,
    /// Number of deferred font fetches currently in flight.
    pub(crate) deferred_fonts_inflight: usize,
    /// Deferred image requests that are intentionally not submitted yet.
    pub(crate) deferred_images: Vec<DeferredImageRequest>,
    /// Number of deferred image requests currently in flight.
    pub(crate) deferred_images_inflight: usize,
    /// External CSS has completed and background images should be discovered
    /// after the next layout refreshes the computed-style cache.
    pub(crate) css_background_scan_pending: bool,
    /// Number of JS console lines already mirrored to Surf's system log.
    pub(crate) js_console_logged_len: usize,
    /// True while DOM + JS runtime are detached and executing on the JS worker.
    pub(crate) js_worker_busy: bool,
}

impl TabState {
    /// Create a new, blank tab.
    pub(crate) fn new() -> Self {
        let mut webview = libwebview::WebView::new(900, 606);
        register_builtin_web_fonts(&mut webview);
        Self {
            webview,
            url_text: String::new(),
            current_url: None,
            page_title: String::new(),
            history: Vec::new(),
            history_pos: 0,
            status_text: String::from("Ready"),
            is_loading: false,
            load_state: PageLoadState::new(),
            pending_scripts: Vec::new(),
            pending_script_modes: Vec::new(),
            pending_script_labels: Vec::new(),
            requested_module_urls: Vec::new(),
            deferred_fonts: Vec::new(),
            requested_font_urls: Vec::new(),
            deferred_fonts_inflight: 0,
            deferred_images: Vec::new(),
            deferred_images_inflight: 0,
            css_background_scan_pending: false,
            js_console_logged_len: 0,
            js_worker_busy: false,
        }
    }

    /// Returns `true` when the history has an earlier entry to go back to.
    pub(crate) fn can_go_back(&self) -> bool {
        self.history_pos > 0
    }

    /// Returns `true` when the history has a later entry to go forward to.
    pub(crate) fn can_go_forward(&self) -> bool {
        self.history_pos + 1 < self.history.len()
    }

    /// Short text used as the tab-bar label for this tab.
    /// Shows a spinner prefix while loading.
    pub(crate) fn tab_label(&self) -> String {
        let base = if !self.page_title.is_empty() {
            &self.page_title
        } else if !self.url_text.is_empty() {
            &self.url_text
        } else {
            "New Tab"
        };
        if self.is_loading {
            let mut s = String::from("\u{27F3} "); // ⟳
            s.push_str(base);
            s
        } else {
            String::from(base)
        }
    }
}

// ═══════════════════════════════════════════════════════════
// Navigation
// ═══════════════════════════════════════════════════════════

/// Navigate the active tab to `url_str` using a GET request.
///
/// For `file://` URLs the file is read from the local filesystem and
/// rendered directly without going through the network worker.
/// For `http://` and `https://` URLs the fetch is submitted to the
/// background network worker and returns immediately.
pub(crate) fn navigate(url_str: &str) {
    let st = crate::state();
    crate::surf_log!("[surf] navigating to: {}", url_str);
    crate::resize_active_webview_now();

    // Handle file:// URLs locally — no network needed.
    if url_str.starts_with("file://") {
        navigate_file(&url_str[7..]);
        return;
    }

    let url = match crate::http::parse_url(url_str) {
        Ok(u) => u,
        Err(_) => {
            st.tabs[st.active_tab].status_text = String::from("Invalid URL");
            crate::ui::update_status();
            return;
        }
    };

    // Bump generation so stale resource results are discarded.
    let generation = crate::net_worker::new_generation();
    st.tabs[st.active_tab]
        .load_state
        .begin_navigation(generation);
    st.tabs[st.active_tab].js_console_logged_len = 0;
    st.tabs[st.active_tab].js_worker_busy = false;
    crate::devtools::reset_for_navigation();

    // Update UI to show loading state.
    st.tabs[st.active_tab].is_loading = true;
    let mut loading_msg = String::from("Loading: ");
    loading_msg.push_str(url_str);
    st.tabs[st.active_tab].status_text = loading_msg;
    crate::ui::update_status();
    crate::ui::update_tab_labels();

    // Clone cookies for the worker thread.
    let cookies = st.cookies.clone();

    // Submit to worker — returns immediately.
    crate::net_worker::submit(crate::net_worker::FetchRequest::Navigate {
        tab_index: st.active_tab,
        url,
        cookies,
        generation,
    });
    crate::ensure_net_poll_timer();
}

/// Navigate the active tab using a form POST request.
///
/// Submits the fetch to the background network worker and returns
/// immediately, just like `navigate()`.
pub(crate) fn navigate_post(url_str: &str, body: &str) {
    let st = crate::state();

    let url = match crate::http::parse_url(url_str) {
        Ok(u) => u,
        Err(_) => {
            st.tabs[st.active_tab].status_text = String::from("Invalid URL");
            crate::ui::update_status();
            return;
        }
    };

    let generation = crate::net_worker::new_generation();
    st.tabs[st.active_tab]
        .load_state
        .begin_navigation(generation);
    st.tabs[st.active_tab].js_console_logged_len = 0;
    st.tabs[st.active_tab].js_worker_busy = false;
    crate::devtools::reset_for_navigation();

    st.tabs[st.active_tab].is_loading = true;
    st.tabs[st.active_tab].status_text = String::from("Submitting...");
    crate::ui::update_status();
    crate::ui::update_tab_labels();

    let cookies = st.cookies.clone();

    crate::net_worker::submit(crate::net_worker::FetchRequest::NavigatePost {
        tab_index: st.active_tab,
        url,
        body: String::from(body),
        cookies,
        generation,
    });
    crate::ensure_net_poll_timer();
}

/// Navigate to a local file on the filesystem.
///
/// Reads the file at `path` (the part after `file://`) and renders it
/// directly in the active tab.  No network worker is involved.
fn navigate_file(path: &str) {
    let st = crate::state();
    let tab_idx = st.active_tab;

    let generation = crate::net_worker::new_generation();
    st.tabs[tab_idx].load_state.begin_navigation(generation);
    st.tabs[tab_idx].is_loading = true;

    st.tabs[tab_idx].status_text = String::from("Loading file...");
    crate::ui::update_status();
    crate::ui::update_tab_labels();

    // Read the file from disk.
    let body = match anyos_std::fs::read_to_vec(path) {
        Ok(data) => data,
        Err(_) => {
            let mut msg = String::from("File not found: ");
            msg.push_str(path);
            st.tabs[tab_idx].load_state.mark_failed();
            st.tabs[tab_idx].is_loading = false;
            st.tabs[tab_idx].status_text = msg;
            crate::ui::update_status();
            crate::ui::update_tab_labels();
            return;
        }
    };

    // Convert body to string (UTF-8 or Latin-1 fallback).
    let html = crate::resources::decode_http_body(&body, "");

    // Build a file:// URL for display and history.
    let mut url_str = String::from("file://");
    url_str.push_str(path);

    // Build a pseudo-Url for base URL resolution (relative links).
    let base_url = crate::http::Url {
        scheme: String::from("file"),
        host: String::new(),
        port: 0,
        path: String::from(path),
    };

    // Clear all state from the previous page (DOM, layout, images, JS, CSS).
    st.tabs[tab_idx].webview.navigate_clear();
    st.tabs[tab_idx].webview.set_url(&url_str);

    // Render the HTML.
    st.tabs[tab_idx].load_state.begin_parse();
    st.tabs[tab_idx].webview.set_html(&html);

    // Extract page title.
    let title = st.tabs[tab_idx]
        .webview
        .get_title()
        .unwrap_or_else(String::new);

    // Update history.
    let at_same = if !st.tabs[tab_idx].history.is_empty()
        && st.tabs[tab_idx].history_pos < st.tabs[tab_idx].history.len()
    {
        st.tabs[tab_idx].history[st.tabs[tab_idx].history_pos] == url_str
    } else {
        false
    };
    if !at_same {
        if !st.tabs[tab_idx].history.is_empty() {
            let pos = st.tabs[tab_idx].history_pos;
            st.tabs[tab_idx].history.truncate(pos + 1);
        }
        st.tabs[tab_idx].history.push(url_str.clone());
        st.tabs[tab_idx].history_pos = st.tabs[tab_idx].history.len() - 1;
    }

    st.tabs[tab_idx].page_title = title;
    st.tabs[tab_idx].url_text = url_str;
    st.tabs[tab_idx].current_url = Some(base_url);
    st.tabs[tab_idx].status_text = String::from("Done");
    st.tabs[tab_idx].is_loading = false;
    st.tabs[tab_idx].load_state.mark_interactive();

    // Update chrome UI.
    let url_for_field = st.tabs[tab_idx].url_text.clone();
    st.url_field.set_text(&url_for_field);
    crate::ui::update_title();
    crate::ui::update_status();
    crate::ui::update_tab_labels();

    crate::surf_log!("[surf] loaded local file: {}", path);
}

/// Navigate the active tab one step back in its history.
pub(crate) fn go_back() {
    let st = crate::state();
    let tab = &st.tabs[st.active_tab];
    if tab.can_go_back() {
        let new_pos = tab.history_pos - 1;
        let url = tab.history[new_pos].clone();
        st.tabs[st.active_tab].history_pos = new_pos;
        navigate(&url);
    }
}

/// Navigate the active tab one step forward in its history.
pub(crate) fn go_forward() {
    let st = crate::state();
    let tab = &st.tabs[st.active_tab];
    if tab.can_go_forward() {
        let new_pos = tab.history_pos + 1;
        let url = tab.history[new_pos].clone();
        st.tabs[st.active_tab].history_pos = new_pos;
        navigate(&url);
    }
}

/// Reload the current page in the active tab.
pub(crate) fn reload() {
    let st = crate::state();
    let url = st.tabs[st.active_tab].url_text.clone();
    if !url.is_empty() {
        navigate(&url);
    }
}
