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
use libanyui_client as ui;
use ui::Widget;

// ═══════════════════════════════════════════════════════════
// Per-tab state
// ═══════════════════════════════════════════════════════════

/// Everything associated with one browser tab.
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
    /// Generation counter for the current navigation.
    /// Used to discard stale fetch results from the worker thread.
    pub(crate) nav_generation: u32,
}

impl TabState {
    /// Create a new, blank tab.
    pub(crate) fn new() -> Self {
        Self {
            webview: libwebview::WebView::new(900, 606),
            url_text: String::new(),
            current_url: None,
            page_title: String::new(),
            history: Vec::new(),
            history_pos: 0,
            status_text: String::from("Ready"),
            nav_generation: 0,
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
    pub(crate) fn tab_label(&self) -> &str {
        if !self.page_title.is_empty() {
            &self.page_title
        } else if !self.url_text.is_empty() {
            &self.url_text
        } else {
            "New Tab"
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
    anyos_std::println!("[surf] navigating to: {}", url_str);

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

    // Cancel any in-flight CSS/image work from the previous page.
    cancel_pending_resources();

    // Bump generation so stale resource results are discarded.
    let generation = crate::net_worker::new_generation();
    st.tabs[st.active_tab].nav_generation = generation;

    // Update UI to show loading state.
    let mut loading_msg = String::from("Loading: ");
    loading_msg.push_str(url_str);
    st.tabs[st.active_tab].status_text = loading_msg;
    crate::ui::update_status();

    // Clone cookies for the worker thread.
    let cookies = st.cookies.clone();

    // Submit to worker — returns immediately.
    crate::net_worker::submit(crate::net_worker::FetchRequest::Navigate {
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

    cancel_pending_resources();

    let generation = crate::net_worker::new_generation();
    st.tabs[st.active_tab].nav_generation = generation;

    st.tabs[st.active_tab].status_text = String::from("Submitting...");
    crate::ui::update_status();

    let cookies = st.cookies.clone();

    crate::net_worker::submit(crate::net_worker::FetchRequest::NavigatePost {
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

    cancel_pending_resources();

    st.tabs[tab_idx].status_text = String::from("Loading file...");
    crate::ui::update_status();

    // Read the file from disk.
    let body = match anyos_std::fs::read_to_vec(path) {
        Ok(data) => data,
        Err(_) => {
            let mut msg = String::from("File not found: ");
            msg.push_str(path);
            st.tabs[tab_idx].status_text = msg;
            crate::ui::update_status();
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

    // Clear previous page state.
    st.tabs[tab_idx].webview.clear_stylesheets();
    st.tabs[tab_idx].webview.set_url(&url_str);

    // Render the HTML.
    st.tabs[tab_idx].webview.set_html(&html);

    // Extract page title.
    let title = st.tabs[tab_idx].webview.get_title()
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

    // Update chrome UI.
    let url_for_field = st.tabs[tab_idx].url_text.clone();
    st.url_field.set_text(&url_for_field);
    crate::ui::update_title();
    crate::ui::update_status();
    crate::ui::update_tab_labels();

    anyos_std::println!("[surf] loaded local file: {}", path);
}

/// Cancel any stale CSS/image fetches from the old timer-based system
/// and clear AppState queues.
fn cancel_pending_resources() {
    let st = crate::state();
    if st.css_timer != 0 {
        ui::kill_timer(st.css_timer);
        st.css_timer = 0;
    }
    st.css_queue.clear();
    if st.image_timer != 0 {
        ui::kill_timer(st.image_timer);
        st.image_timer = 0;
    }
    st.image_queue.clear();
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
