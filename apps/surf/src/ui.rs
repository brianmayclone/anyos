// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! UI chrome helpers and tab management for the Surf browser.
//!
//! Contains the stateless helper functions that update window/tab-bar chrome
//! (`update_title`, `update_status`, `update_tab_labels`) as well as URL
//! formatting utilities and the tab lifecycle functions (`add_tab`,
//! `close_tab`, `switch_tab`).

use alloc::string::String;
use libanyui_client as ui;
use ui::Widget;

// ═══════════════════════════════════════════════════════════
// URL helpers
// ═══════════════════════════════════════════════════════════

/// Format a parsed `Url` back into a canonical string (`scheme://host[:port]/path`).
///
/// Non-standard ports (≠ 80 for http, ≠ 443 for https) are included explicitly.
pub(crate) fn format_url(url: &crate::http::Url) -> String {
    let mut s = String::new();
    s.push_str(&url.scheme);
    s.push_str("://");
    s.push_str(&url.host);
    if (url.scheme == "http" && url.port != 80) || (url.scheme == "https" && url.port != 443) {
        s.push(':');
        push_u32(&mut s, url.port as u32);
    }
    s.push_str(&url.path);
    s
}

/// Append the decimal representation of `val` to `s` without heap allocation.
pub(crate) fn push_u32(s: &mut String, val: u32) {
    if val >= 10 {
        push_u32(s, val / 10);
    }
    s.push((b'0' + (val % 10) as u8) as char);
}

// ═══════════════════════════════════════════════════════════
// Window / chrome updates
// ═══════════════════════════════════════════════════════════

/// Update the window title to reflect the active tab's page title.
pub(crate) fn update_title() {
    let st = crate::state();
    let tab = &st.tabs[st.active_tab];
    if tab.page_title.is_empty() {
        st.win.set_title("Surf");
    } else {
        let mut title = String::from("Surf - ");
        title.push_str(&tab.page_title);
        st.win.set_title(&title);
    }
}

/// Update the status bar text from the active tab's `status_text`.
pub(crate) fn update_status() {
    let st = crate::state();
    let text = st.tabs[st.active_tab].status_text.clone();
    st.status_label.set_text(&text);
}

/// Rebuild the tab-bar labels from all open tabs and highlight the active one.
pub(crate) fn update_tab_labels() {
    let st = crate::state();
    let mut labels = String::new();
    for (i, tab) in st.tabs.iter().enumerate() {
        if i > 0 {
            labels.push('|');
        }
        let label = tab.tab_label();
        // Truncate overly long tab labels to keep the tab bar readable.
        if label.len() > 20 {
            labels.push_str(&label[..20]);
            labels.push_str("...");
        } else {
            labels.push_str(label);
        }
    }
    st.tab_bar_view.set_text(&labels);
    st.tab_bar_view.set_state(st.active_tab as u32);
}

// ═══════════════════════════════════════════════════════════
// Tab lifecycle
// ═══════════════════════════════════════════════════════════

/// Open a new blank tab and make it the active tab.
pub(crate) fn add_tab() {
    let st = crate::state();
    let mut tab = crate::tab::TabState::new();
    tab.webview.set_link_callback(crate::callbacks::on_link_click, 0);
    tab.webview.set_submit_callback(crate::callbacks::on_form_submit, 0);
    st.content_view.add(tab.webview.scroll_view());
    tab.webview.scroll_view().set_dock(ui::DOCK_FILL);
    tab.webview.scroll_view().on_scroll(|_| { crate::ensure_anim_timer(); });

    // Hide all existing tabs while we add the new one.
    for t in &st.tabs {
        t.webview.scroll_view().set_visible(false);
    }

    st.tabs.push(tab);
    st.active_tab = st.tabs.len() - 1;
    st.url_field.set_text("");
    update_title();
    update_tab_labels();
}

/// Close the tab at `idx`.
///
/// Quits the application when the last tab is closed.
pub(crate) fn close_tab(idx: usize) {
    let st = crate::state();
    if st.tabs.len() <= 1 {
        ui::quit();
        return;
    }
    st.tabs[idx].webview.scroll_view().remove();
    st.tabs.remove(idx);
    if st.active_tab >= st.tabs.len() {
        st.active_tab = st.tabs.len() - 1;
    }
    switch_tab(st.active_tab);
}

// ═══════════════════════════════════════════════════════════
// DevTools console panel
// ═══════════════════════════════════════════════════════════

/// Toggle the DevTools console panel open/closed.
pub(crate) fn toggle_devtools() {
    let st = crate::state();
    st.devtools_open = !st.devtools_open;
    let (label, height) = if st.devtools_open {
        ("DevTools \u{25B2}", 200u32)   // ▲
    } else {
        ("DevTools \u{25BC}", 0u32)     // ▼
    };
    st.btn_devtools.set_text(label);
    st.devtools_panel.set_size(0, height);
    if st.devtools_open {
        update_devtools();
    }
}

/// Clear the DevTools console output.
pub(crate) fn clear_devtools() {
    let st = crate::state();
    st.devtools_label.set_text("");
}

/// Refresh the DevTools console panel with the active tab's JS console output.
pub(crate) fn update_devtools() {
    let st = crate::state();
    if !st.devtools_open { return; }
    let lines = st.tabs[st.active_tab].webview.js_console();
    let mut text = String::new();
    for (i, line) in lines.iter().enumerate() {
        if i > 0 { text.push('\n'); }
        text.push_str(line);
        // Limit output to last 200 lines to avoid unbounded growth.
        if i >= 199 { break; }
    }
    st.devtools_label.set_text(&text);
}

/// Make the tab at `idx` the active (visible) tab.
pub(crate) fn switch_tab(idx: usize) {
    let st = crate::state();
    if idx >= st.tabs.len() {
        return;
    }
    st.tabs[st.active_tab].webview.scroll_view().set_visible(false);
    st.active_tab = idx;
    st.tabs[st.active_tab].webview.scroll_view().set_visible(true);
    let url_text = st.tabs[st.active_tab].url_text.clone();
    st.url_field.set_text(&url_text);
    update_title();
    update_status();
    update_tab_labels();
}
