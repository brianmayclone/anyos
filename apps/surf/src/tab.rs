// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Per-tab state and navigation logic for the Surf browser.
//!
//! `TabState` holds everything associated with a single browser tab:
//! the `WebView`, URL/history, and page title.  The navigation functions
//! (`navigate`, `navigate_post`, `go_back`, `go_forward`, `reload`) are
//! also defined here because they operate primarily on per-tab data.

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
}

impl TabState {
    /// Create a new, blank tab.
    pub(crate) fn new() -> Self {
        Self {
            webview: libwebview::WebView::new(800, 600),
            url_text: String::new(),
            current_url: None,
            page_title: String::new(),
            history: Vec::new(),
            history_pos: 0,
            status_text: String::from("Ready"),
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
/// Fetches the page over HTTP/HTTPS, renders it, discovers external resources,
/// and updates the history/UI.  Errors are surfaced in the status bar.
pub(crate) fn navigate(url_str: &str) {
    let st = crate::state();
    anyos_std::println!("[surf] navigating to: {}", url_str);

    let url = match crate::http::parse_url(url_str) {
        Ok(u) => u,
        Err(_) => {
            st.tabs[st.active_tab].status_text = String::from("Invalid URL");
            crate::ui::update_status();
            return;
        }
    };

    let mut loading_msg = String::from("Loading: ");
    loading_msg.push_str(url_str);
    st.tabs[st.active_tab].status_text = loading_msg;
    crate::ui::update_status();

    let response = match crate::http::fetch(&url, &mut st.cookies, &mut st.conn_pool) {
        Ok(r) => r,
        Err(e) => {
            let msg = match e {
                crate::http::FetchError::InvalidUrl => "Invalid URL",
                crate::http::FetchError::DnsFailure => "DNS lookup failed",
                crate::http::FetchError::ConnectFailure => "Connection failed",
                crate::http::FetchError::SendFailure => "Send failed",
                crate::http::FetchError::NoResponse => "No response",
                crate::http::FetchError::TooManyRedirects => "Too many redirects",
                crate::http::FetchError::TlsHandshakeFailed => "TLS handshake failed",
            };
            st.tabs[st.active_tab].status_text = String::from(msg);
            crate::ui::update_status();
            return;
        }
    };

    if response.status < 200 || response.status >= 400 {
        st.tabs[st.active_tab].status_text = String::from("HTTP error");
        crate::ui::update_status();
        return;
    }

    // Use the final (post-redirect) URL as the base for relative references.
    let base_url = response.final_url.unwrap_or_else(|| crate::http::clone_url(&url));

    let body_text = crate::resources::decode_http_body(&response.body, &response.headers);
    anyos_std::println!("[surf] received {} bytes, parsing HTML...", body_text.len());

    st.tabs[st.active_tab].status_text = String::from("Rendering page...");
    crate::ui::update_status();

    // Pass URL + cookies to the JS runtime before rendering so that
    // window.location and document.cookie are correct during script execution.
    st.tabs[st.active_tab].webview.clear_stylesheets();
    let url_string_for_js = crate::ui::format_url(&base_url);
    st.tabs[st.active_tab].webview.set_url(&url_string_for_js);
    let is_secure = base_url.scheme == "https";
    let cookie_hdr = st.cookies
        .cookie_header(&base_url.host, &base_url.path, is_secure)
        .unwrap_or_default();
    st.tabs[st.active_tab].webview.js_runtime().set_cookies(&cookie_hdr);

    st.tabs[st.active_tab].webview.set_html(&body_text);
    anyos_std::println!("[surf] render complete");

    for line in st.tabs[st.active_tab].webview.js_console() {
        anyos_std::println!("[js] {}", line);
    }

    // Update history and tab metadata.
    let title = st.tabs[st.active_tab]
        .webview
        .get_title()
        .unwrap_or_else(|| String::from("Untitled"));
    let url_string = crate::ui::format_url(&base_url);
    {
        let tab = &mut st.tabs[st.active_tab];
        if tab.history.is_empty()
            || tab.history_pos >= tab.history.len()
            || tab.history[tab.history_pos] != url_string
        {
            if tab.history_pos + 1 < tab.history.len() {
                tab.history.truncate(tab.history_pos + 1);
            }
            tab.history.push(url_string.clone());
            tab.history_pos = tab.history.len() - 1;
        }
        tab.page_title = title;
        tab.url_text = url_string;
        tab.status_text = String::from("Done");
    }

    // Refresh DevTools console with any output from this page's scripts.
    crate::ui::update_devtools();

    // Connect any WebSockets that JS requested during set_html.
    let tab_idx = st.active_tab;
    crate::connect_pending_ws(tab_idx);

    // Update the URL bar and chrome immediately so the user sees the page as
    // loaded before we block on external CSS / image downloads.
    let st = crate::state();
    st.url_field.set_text(&st.tabs[st.active_tab].url_text.clone());
    crate::ui::update_title();
    crate::ui::update_status();
    crate::ui::update_tab_labels();

    // Parse the body again for resource discovery (stylesheets, images).
    // A second parse is cheaper than complicating set_html to return DOM info.
    let dom_for_resources = libwebview::html::parse(&body_text);
    anyos_std::println!("[surf] DOM: {} nodes", dom_for_resources.nodes.len());

    // Cancel any stale CSS / image fetches left over from the previous page.
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

    crate::resources::queue_stylesheets(&dom_for_resources, &base_url, tab_idx);
    crate::resources::queue_images(&dom_for_resources, &base_url, tab_idx);

    st.tabs[st.active_tab].current_url = Some(base_url);
}

/// Navigate the active tab using a form POST request.
///
/// Behaves like `navigate` but sends `body` (URL-encoded form data) as the
/// HTTP POST body instead of using GET parameters.
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

    st.tabs[st.active_tab].status_text = String::from("Submitting...");
    crate::ui::update_status();

    let response = match crate::http::fetch_post(&url, body, &mut st.cookies, &mut st.conn_pool) {
        Ok(r) => r,
        Err(_) => {
            st.tabs[st.active_tab].status_text = String::from("Submit failed");
            crate::ui::update_status();
            return;
        }
    };

    if response.status < 200 || response.status >= 400 {
        st.tabs[st.active_tab].status_text = String::from("HTTP error");
        crate::ui::update_status();
        return;
    }

    let base_url = response.final_url.unwrap_or_else(|| crate::http::clone_url(&url));
    let body_text = crate::resources::decode_http_body(&response.body, &response.headers);

    let tab = &mut st.tabs[st.active_tab];
    tab.webview.clear_stylesheets();
    let post_url_str = crate::ui::format_url(&base_url);
    tab.webview.set_url(&post_url_str);
    let post_is_secure = base_url.scheme == "https";
    let post_cookie_hdr = st.cookies
        .cookie_header(&base_url.host, &base_url.path, post_is_secure)
        .unwrap_or_default();
    tab.webview.js_runtime().set_cookies(&post_cookie_hdr);
    tab.webview.set_html(&body_text);

    for line in tab.webview.js_console() {
        anyos_std::println!("[js] {}", line);
    }

    let title = tab.webview.get_title().unwrap_or_else(|| String::from("Untitled"));
    let url_string = crate::ui::format_url(&base_url);

    if tab.history.is_empty()
        || tab.history_pos >= tab.history.len()
        || tab.history[tab.history_pos] != url_string
    {
        if tab.history_pos + 1 < tab.history.len() {
            tab.history.truncate(tab.history_pos + 1);
        }
        tab.history.push(url_string.clone());
        tab.history_pos = tab.history.len() - 1;
    }
    tab.page_title = title;
    tab.url_text = url_string;
    tab.status_text = String::from("Done");

    // Refresh DevTools console with any output from this page's scripts.
    crate::ui::update_devtools();

    // Connect any WebSockets that JS requested during set_html.
    let tab_idx = st.active_tab;
    crate::connect_pending_ws(tab_idx);

    // Update chrome immediately before blocking on CSS/image downloads.
    let st = crate::state();
    st.url_field.set_text(&st.tabs[st.active_tab].url_text.clone());
    crate::ui::update_title();
    crate::ui::update_status();
    crate::ui::update_tab_labels();

    let dom_for_resources = libwebview::html::parse(&body_text);

    // Cancel any stale CSS / image fetches left over from the previous page.
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

    crate::resources::queue_stylesheets(&dom_for_resources, &base_url, tab_idx);
    crate::resources::queue_images(&dom_for_resources, &base_url, tab_idx);

    st.tabs[st.active_tab].current_url = Some(base_url);
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
