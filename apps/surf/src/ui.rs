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

    // Update the URL progress bar visibility.
    let loading = st.tabs[st.active_tab].is_loading;
    st.url_progress.set_visible(loading);
    if loading {
        // Indeterminate style: show full bar animating.
        st.url_progress.set_state(100);
    }
}

/// Rebuild the tab-bar labels from all open tabs and highlight the active one.
pub(crate) fn update_tab_labels() {
    let st = crate::state();
    let mut labels = String::new();
    for (i, tab) in st.tabs.iter().enumerate() {
        if i > 0 {
            labels.push('|');
        }
        labels.push_str(&tab_bar_label(tab));
    }
    st.tab_bar_view.set_text(&labels);
    for (i, tab) in st.tabs.iter().enumerate() {
        if !tab.favicon_pixels.is_empty() && tab.favicon_w > 0 && tab.favicon_h > 0 {
            st.tab_bar_view.set_tab_icon(
                i as u32,
                &tab.favicon_pixels,
                tab.favicon_w,
                tab.favicon_h,
            );
        } else {
            st.tab_bar_view.clear_tab_icon(i as u32);
        }
    }
    st.tab_bar_view.set_state(st.active_tab as u32);
}

fn tab_bar_label(tab: &crate::tab::TabState) -> String {
    const MAX_CHARS: usize = 24;

    let raw = tab.tab_label();
    let mut out = String::new();
    let mut chars = 0usize;
    let mut truncated = false;
    let mut last_space = false;

    for ch in raw.chars() {
        if chars >= MAX_CHARS {
            truncated = true;
            break;
        }
        let mapped = match ch {
            '|' => '-',
            '\n' | '\r' | '\t' => ' ',
            c if c.is_control() => ' ',
            c => c,
        };
        if mapped == ' ' {
            if last_space {
                continue;
            }
            last_space = true;
        } else {
            last_space = false;
        }
        out.push(mapped);
        chars += 1;
    }

    if truncated {
        out.push_str("...");
    }
    out
}

/// Apply the current theme palette to Surf's chrome controls.
pub(crate) fn apply_theme() {
    let st = crate::state();
    let tc = ui::theme::colors();

    st.toolbar.set_color(tc.toolbar_bg);
    st.nav_group.set_color(tc.toolbar_bg);
    st.btn_back
        .set_system_icon("chevron-left", ui::IconType::Outline, tc.text_secondary, 20);
    st.btn_forward.set_system_icon(
        "chevron-right",
        ui::IconType::Outline,
        tc.text_secondary,
        20,
    );
    st.btn_reload
        .set_system_icon("refresh", ui::IconType::Outline, tc.text_secondary, 20);
    st.btn_menu
        .set_system_icon("menu-2", ui::IconType::Outline, tc.text_secondary, 20);
    st.url_progress.set_color(tc.accent);
    st.status_label.set_color(tc.toolbar_bg);
    st.status_label.set_text_color(tc.text_secondary);
    st.content_view.set_color(tc.window_bg);
    st.devtools.console_output.set_color(tc.input_bg);
    st.devtools.console_output.set_text_color(tc.success);
}

/// Poll for compositor theme changes and re-apply Surf chrome colors.
pub(crate) fn ensure_theme_timer() {
    let st = crate::state();
    if st.theme_timer != 0 {
        return;
    }

    st.theme_timer = ui::set_timer(500, || {
        let st = crate::state();
        let is_light = ui::theme::is_light();
        if st.last_theme_light != is_light {
            st.last_theme_light = is_light;
            apply_theme();
            update_tab_labels();
            update_status();
            update_devtools();
        }
    });
}

// ═══════════════════════════════════════════════════════════
// Tab lifecycle
// ═══════════════════════════════════════════════════════════

/// Open a new blank tab and make it the active tab.
pub(crate) fn add_tab() {
    let st = crate::state();
    let mut tab = crate::tab::TabState::new();
    tab.webview
        .set_link_callback(crate::callbacks::on_link_click, 0);
    tab.webview
        .set_submit_callback(crate::callbacks::on_form_submit, 0);
    st.content_view.add(tab.webview.scroll_view());
    tab.webview.scroll_view().set_dock(ui::DOCK_FILL);
    tab.webview.scroll_view().on_scroll(|_| {
        crate::mark_scroll_activity();
    });

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
    crate::net_worker::handle_closed_tab(idx);
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

/// Toggle the DevTools window open/closed.
pub(crate) fn toggle_devtools() {
    crate::devtools::toggle();
}

/// Clear the DevTools console output (legacy entry; the new devtools panel
/// repopulates from `js_console()` on every refresh).
pub(crate) fn clear_devtools() {
    let st = crate::state();
    st.devtools.console_output.set_text("");
}

/// Refresh all DevTools panels.
pub(crate) fn update_devtools() {
    crate::devtools::refresh_all();
}

/// Make the tab at `idx` the active (visible) tab.
pub(crate) fn switch_tab(idx: usize) {
    let st = crate::state();
    if idx >= st.tabs.len() {
        return;
    }
    st.tabs[st.active_tab]
        .webview
        .scroll_view()
        .set_visible(false);
    st.active_tab = idx;
    st.tabs[st.active_tab]
        .webview
        .scroll_view()
        .set_visible(true);
    let url_text = st.tabs[st.active_tab].url_text.clone();
    st.url_field.set_text(&url_text);
    update_title();
    update_status();
    update_tab_labels();
}
