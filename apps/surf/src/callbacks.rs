// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! libanyui C-ABI callbacks and form-encoding utilities for the Surf browser.
//!
//! These callbacks are registered with `WebView::set_link_callback` and
//! `WebView::set_submit_callback` and are invoked by the UI toolkit when the
//! user interacts with rendered page controls.

use alloc::string::String;

// ═══════════════════════════════════════════════════════════
// Link click callback
// ═══════════════════════════════════════════════════════════

/// Called by libanyui when the user clicks a rendered hyperlink.
///
/// Resolves the link URL relative to the page's base URL and navigates to it.
pub(crate) extern "C" fn on_link_click(ctrl_id: u32, _event_type: u32, _userdata: u64) {
    let st = crate::state();
    let tab = &st.tabs[st.active_tab];
    if let Some(link_url) = tab.webview.link_url_for(ctrl_id) {
        // file:// URLs are already absolute — pass through directly.
        let resolved = if link_url.starts_with("file://") {
            String::from(link_url)
        } else if let Some(ref base) = tab.current_url {
            let resolved_url = crate::http::resolve_url(base, link_url);
            crate::ui::format_url(&resolved_url)
        } else {
            String::from(link_url)
        };
        crate::tab::navigate(&resolved);
    }
}

// ═══════════════════════════════════════════════════════════
// Form submit callback
// ═══════════════════════════════════════════════════════════

/// Called by libanyui when the user clicks a form submit button.
///
/// Collects form fields, URL-encodes them, resolves the action URL, and
/// navigates with either GET (query string) or POST (request body).
pub(crate) extern "C" fn on_form_submit(ctrl_id: u32, _event_type: u32, _userdata: u64) {
    let st = crate::state();
    let tab = &st.tabs[st.active_tab];

    if !tab.webview.is_submit_button(ctrl_id) {
        return;
    }

    let (action, method) = match tab.webview.form_action_for(ctrl_id) {
        Some(am) => am,
        None => return,
    };

    // Collect and URL-encode form data.
    let data = tab.webview.collect_form_data(ctrl_id);
    let mut encoded = String::new();
    for (i, (name, value)) in data.iter().enumerate() {
        if i > 0 {
            encoded.push('&');
        }
        url_encode_into(&mut encoded, name);
        encoded.push('=');
        url_encode_into(&mut encoded, value);
    }

    // Resolve the action URL relative to the current page.
    let resolved_action = if let Some(ref base) = tab.current_url {
        let action_url = crate::http::resolve_url(base, &action);
        crate::ui::format_url(&action_url)
    } else {
        action
    };

    if method == "POST" {
        crate::tab::navigate_post(&resolved_action, &encoded);
    } else {
        // GET: append form data as a query string.
        let mut url = resolved_action;
        if !encoded.is_empty() {
            url.push(if url.contains('?') { '&' } else { '?' });
            url.push_str(&encoded);
        }
        crate::tab::navigate(&url);
    }
}

// ═══════════════════════════════════════════════════════════
// URL encoding
// ═══════════════════════════════════════════════════════════

/// Percent-encode `s` and append the result to `out`.
///
/// Follows RFC 3986 unreserved characters (A-Z, a-z, 0-9, `-`, `_`, `.`, `~`).
/// Spaces are encoded as `+` for `application/x-www-form-urlencoded`.
pub(crate) fn url_encode_into(out: &mut String, s: &str) {
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => {
                out.push('%');
                let hi = b >> 4;
                let lo = b & 0xF;
                out.push(if hi < 10 { (b'0' + hi) as char } else { (b'A' + hi - 10) as char });
                out.push(if lo < 10 { (b'0' + lo) as char } else { (b'A' + lo - 10) as char });
            }
        }
    }
}
