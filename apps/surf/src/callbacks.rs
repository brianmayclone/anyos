// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! libanyui C-ABI callbacks and form-encoding utilities for the Surf browser.
//!
//! These callbacks are registered with `WebView::set_link_callback` and
//! `WebView::set_submit_callback` and are invoked by the UI toolkit when the
//! user interacts with rendered page controls.

use alloc::string::String;
use alloc::vec::Vec;

// ═══════════════════════════════════════════════════════════
// Link click callback
// ═══════════════════════════════════════════════════════════

/// Called by libanyui when the user clicks on the page canvas or a rendered control.
///
/// Resolves the link URL relative to the page's base URL and navigates to it.
/// Also handles canvas-based submit button hits (since the canvas only has one callback).
pub(crate) extern "C" fn on_link_click(ctrl_id: u32, _event_type: u32, _userdata: u64) {
    let st = crate::state();
    let tab = &mut st.tabs[st.active_tab];
    let control_node = tab.webview.node_id_for_control(ctrl_id);

    match _event_type {
        libanyui_client::EVENT_MOUSE_MOVE => {
            let hovered = control_node.or_else(|| tab.webview.hit_test_node_canvas(ctrl_id));
            tab.webview.set_hovered_node(hovered);
            return;
        }
        libanyui_client::EVENT_MOUSE_DOWN => {
            let active = control_node.or_else(|| tab.webview.hit_test_node_canvas(ctrl_id));
            tab.webview.set_active_node(active);
            tab.webview.set_hovered_node(active);
            return;
        }
        libanyui_client::EVENT_MOUSE_ENTER => {
            let hovered = control_node.or_else(|| tab.webview.hit_test_node_canvas(ctrl_id));
            tab.webview.set_hovered_node(hovered);
            return;
        }
        libanyui_client::EVENT_MOUSE_UP => {
            let hovered = control_node.or_else(|| tab.webview.hit_test_node_canvas(ctrl_id));
            tab.webview.set_active_node(None);
            tab.webview.set_hovered_node(hovered);
            return;
        }
        libanyui_client::EVENT_FOCUS => {
            let focused = control_node;
            tab.webview.set_focused_node(focused, true);
            return;
        }
        libanyui_client::EVENT_BLUR => {
            tab.webview.set_focused_node(None, false);
            return;
        }
        libanyui_client::EVENT_MOUSE_LEAVE => {
            tab.webview.set_hovered_node(None);
            tab.webview.set_active_node(None);
            return;
        }
        _ => {}
    }

    if _event_type != libanyui_client::EVENT_CLICK {
        return;
    }

    // DevTools element-picker mode: route the click to the inspector instead
    // of the page's normal link/submit handling.
    if st.devtools.picker_active {
        if let Some(node_id) = tab.webview.hit_test_node_canvas(ctrl_id) {
            crate::devtools::select_dom_node(node_id);
        }
        return;
    }

    let tab_index = st.active_tab;
    let clicked_submit_node = st.tabs[tab_index]
        .webview
        .submit_node_for_control(ctrl_id);
    if !st.tabs[tab_index].webview.dispatch_click_for_control(ctrl_id) {
        process_dom_event_side_effects(tab_index);
        return;
    }
    if process_dom_event_side_effects(tab_index) {
        return;
    }
    let tab = &mut st.tabs[tab_index];

    // Try link hit first.
    if let Some(link_url) = tab.webview.link_url_for(ctrl_id) {
        let resolved = if link_url.starts_with("file://") {
            String::from(link_url)
        } else if let Some(ref base) = tab.current_url {
            let resolved_url = crate::http::resolve_url(base, link_url);
            crate::ui::format_url(&resolved_url)
        } else {
            String::from(link_url)
        };
        crate::tab::navigate(&resolved);
        return;
    }

    // Try submit button hit (canvas-based submit regions).
    if let Some(node_id) = clicked_submit_node {
        if !tab.webview.dispatch_submit_for_node(node_id) {
            process_dom_event_side_effects(tab_index);
            return;
        }
        if process_dom_event_side_effects(tab_index) {
            return;
        }
        let tab = &st.tabs[tab_index];
        if let Some((action, method, enctype)) = tab.webview.form_action_for_node(node_id) {
            let data = tab.webview.collect_form_data_for_node(node_id);
            submit_form_data(tab.current_url.as_ref(), action, method, enctype, data);
        }
        return;
    }

    // Try reset button hit — resets all controls in the parent <form>.
    if tab.webview.is_reset_button(ctrl_id) {
        tab.webview.reset_form(ctrl_id);
        return;
    }

    // Try file input hit — open a file dialog (HTML §4.10.5.1.18).
    if let Some(node_id) = tab.webview.canvas_file_input_hit(ctrl_id) {
        if let Some(path) = libanyui_client::FileDialog::open_file() {
            tab.webview.set_file_input_value(node_id, &path);
        }
        return;
    }

    if tab.webview.toggle_checkbox_for_canvas(ctrl_id) {
        return;
    }

    if tab.webview.advance_select_for_canvas(ctrl_id) {
        return;
    }

    if tab.webview.toggle_radio_for_canvas(ctrl_id) {
        return;
    }

    if tab.webview.set_range_for_canvas(ctrl_id) {
        return;
    }

    if tab.webview.set_color_for_canvas(ctrl_id) {
        return;
    }

    // Try color input hit — open a simple color entry.
    // anyOS doesn't have a native color picker dialog, so we allow editing
    // the hex value directly in the text field (already a TextField control).
    // Nothing special to do here — just focus the text field.

    // Try focusing a form control (text field / textarea) at the click position.
    // Also handles <label for="id"> clicks (implicit and explicit association).
    tab.webview.focus_form_control_at_canvas(ctrl_id);
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
    let tab_index = st.active_tab;

    if _event_type == libanyui_client::EVENT_SUBMIT
        && !st.tabs[tab_index].webview.dispatch_enter_for_control(ctrl_id)
    {
        process_dom_event_side_effects(tab_index);
        return;
    }

    if !st.tabs[tab_index].webview.dispatch_submit_for_control(ctrl_id) {
        process_dom_event_side_effects(tab_index);
        return;
    }
    if process_dom_event_side_effects(tab_index) {
        return;
    }

    let tab = &st.tabs[tab_index];

    // Works for both submit-button clicks and Enter key in text fields.
    let (action, method, enctype) = match tab.webview.form_action_for(ctrl_id) {
        Some(ame) => ame,
        None => return,
    };

    // Collect form data.
    let data = tab.webview.collect_form_data(ctrl_id);

    submit_form_data(tab.current_url.as_ref(), action, method, enctype, data);
}

pub(crate) fn submit_form_data(
    current_url: Option<&crate::http::Url>,
    action: String,
    method: String,
    enctype: String,
    data: Vec<(String, String)>,
) {
    // Resolve the action URL relative to the current page.
    let resolved_action = if let Some(base) = current_url {
        let action_url = crate::http::resolve_url(base, &action);
        crate::ui::format_url(&action_url)
    } else {
        action
    };

    if method == "POST" {
        if enctype == "multipart/form-data" {
            // Encode as multipart/form-data (RFC 7578).
            let boundary = "----anyOSFormBoundary7MA4YWxkTrZu0gW";
            let mut body = String::new();
            for (name, value) in &data {
                body.push_str("--");
                body.push_str(boundary);
                body.push_str("\r\n");
                body.push_str("Content-Disposition: form-data; name=\"");
                body.push_str(name);
                body.push_str("\"\r\n\r\n");
                body.push_str(value);
                body.push_str("\r\n");
            }
            body.push_str("--");
            body.push_str(boundary);
            body.push_str("--\r\n");
            crate::tab::navigate_post(&resolved_action, &body);
        } else {
            // Default: application/x-www-form-urlencoded.
            let mut encoded = String::new();
            for (i, (name, value)) in data.iter().enumerate() {
                if i > 0 {
                    encoded.push('&');
                }
                url_encode_into(&mut encoded, name);
                encoded.push('=');
                url_encode_into(&mut encoded, value);
            }
            crate::tab::navigate_post(&resolved_action, &encoded);
        }
    } else {
        // GET: append form data as a query string.
        let mut encoded = String::new();
        for (i, (name, value)) in data.iter().enumerate() {
            if i > 0 {
                encoded.push('&');
            }
            url_encode_into(&mut encoded, name);
            encoded.push('=');
            url_encode_into(&mut encoded, value);
        }
        let mut url = resolved_action;
        if !encoded.is_empty() {
            url.push(if url.contains('?') { '&' } else { '?' });
            url.push_str(&encoded);
        }
        crate::tab::navigate(&url);
    }
}

fn process_dom_event_side_effects(tab_index: usize) -> bool {
    crate::apply_js_host_mutations(tab_index);
    crate::connect_pending_ws(tab_index);
    crate::drain_js_navigation_for_tab(tab_index)
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
                out.push(if hi < 10 {
                    (b'0' + hi) as char
                } else {
                    (b'A' + hi - 10) as char
                });
                out.push(if lo < 10 {
                    (b'0' + lo) as char
                } else {
                    (b'A' + lo - 10) as char
                });
            }
        }
    }
}
