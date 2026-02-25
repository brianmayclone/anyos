// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Surf -- a tabbed web browser for anyOS.
//!
//! Renders HTML pages with CSS styling, fetched over HTTP/1.1.
//! Uses libanyui for the UI chrome (toolbar, tabs, status bar) and
//! libwebview for HTML content rendering via real UI controls.

#![no_std]
#![no_main]

mod http;
mod deflate;
mod tls;

anyos_std::entry!(main);

use alloc::string::String;
use alloc::vec::Vec;
use alloc::vec;

use libanyui_client as ui;
use ui::Widget;

// ---------------------------------------------------------------------------
// Per-tab state
// ---------------------------------------------------------------------------

struct TabState {
    webview: libwebview::WebView,
    url_text: String,
    current_url: Option<http::Url>,
    page_title: String,
    history: Vec<String>,
    history_pos: usize,
    status_text: String,
}

impl TabState {
    fn new() -> Self {
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

    fn can_go_back(&self) -> bool { self.history_pos > 0 }
    fn can_go_forward(&self) -> bool { self.history_pos + 1 < self.history.len() }

    fn tab_label(&self) -> &str {
        if !self.page_title.is_empty() {
            &self.page_title
        } else if !self.url_text.is_empty() {
            &self.url_text
        } else {
            "New Tab"
        }
    }
}

// ---------------------------------------------------------------------------
// Global application state
// ---------------------------------------------------------------------------

struct AppState {
    win: ui::Window,
    toolbar: ui::View,
    btn_back: ui::Button,
    btn_forward: ui::Button,
    btn_reload: ui::Button,
    url_field: ui::TextField,
    tab_bar_view: ui::TabBar,
    content_view: ui::View,
    status_label: ui::Label,
    tabs: Vec<TabState>,
    active_tab: usize,
    cookies: http::CookieJar,
}

static mut STATE: Option<AppState> = None;

fn state() -> &'static mut AppState {
    unsafe { STATE.as_mut().unwrap() }
}

// ---------------------------------------------------------------------------
// Navigation
// ---------------------------------------------------------------------------

fn navigate(url_str: &str) {
    let st = state();
    anyos_std::println!("[surf] navigating to: {}", url_str);

    let url = match http::parse_url(url_str) {
        Ok(u) => u,
        Err(_) => {
            st.tabs[st.active_tab].status_text = String::from("Invalid URL");
            update_status();
            return;
        }
    };

    st.tabs[st.active_tab].status_text = String::from("Loading...");
    update_status();

    let response = match http::fetch(&url, &mut st.cookies) {
        Ok(r) => r,
        Err(e) => {
            let msg = match e {
                http::FetchError::InvalidUrl => "Invalid URL",
                http::FetchError::DnsFailure => "DNS lookup failed",
                http::FetchError::ConnectFailure => "Connection failed",
                http::FetchError::SendFailure => "Send failed",
                http::FetchError::NoResponse => "No response",
                http::FetchError::TooManyRedirects => "Too many redirects",
                http::FetchError::TlsHandshakeFailed => "TLS handshake failed",
            };
            st.tabs[st.active_tab].status_text = String::from(msg);
            update_status();
            return;
        }
    };

    if response.status < 200 || response.status >= 400 {
        st.tabs[st.active_tab].status_text = String::from("HTTP error");
        update_status();
        return;
    }

    let body_text = String::from_utf8_lossy(&response.body).into_owned();

    // Collect and fetch images from HTML.
    let dom_for_images = libwebview::html::parse(&body_text);
    let tab = &mut st.tabs[st.active_tab];
    collect_and_fetch_images(&dom_for_images, &url, &mut tab.webview, &mut st.cookies);

    // Set HTML content — this parses, lays out, and renders controls.
    let tab = &mut st.tabs[st.active_tab];
    tab.webview.set_html(&body_text);

    // Extract title.
    let title = tab.webview.get_title().unwrap_or_else(|| String::from("Untitled"));

    // Update URL history.
    let url_string = format_url(&url);
    if tab.history.is_empty() || tab.history_pos >= tab.history.len()
        || tab.history[tab.history_pos] != url_string
    {
        if tab.history_pos + 1 < tab.history.len() {
            tab.history.truncate(tab.history_pos + 1);
        }
        tab.history.push(url_string.clone());
        tab.history_pos = tab.history.len() - 1;
    }

    tab.current_url = Some(url);
    tab.page_title = title;
    tab.url_text = url_string.clone();
    tab.status_text = String::from("Done");

    // Update UI.
    let st = state();
    st.url_field.set_text(&st.tabs[st.active_tab].url_text);
    update_title();
    update_status();
    update_tab_labels();
}

fn navigate_post(url_str: &str, body: &str) {
    let st = state();
    let url = match http::parse_url(url_str) {
        Ok(u) => u,
        Err(_) => {
            st.tabs[st.active_tab].status_text = String::from("Invalid URL");
            update_status();
            return;
        }
    };

    st.tabs[st.active_tab].status_text = String::from("Submitting...");
    update_status();

    let response = match http::fetch_post(&url, body, &mut st.cookies) {
        Ok(r) => r,
        Err(_) => {
            st.tabs[st.active_tab].status_text = String::from("Submit failed");
            update_status();
            return;
        }
    };

    if response.status < 200 || response.status >= 400 {
        st.tabs[st.active_tab].status_text = String::from("HTTP error");
        update_status();
        return;
    }

    let body_text = String::from_utf8_lossy(&response.body).into_owned();

    let dom_for_images = libwebview::html::parse(&body_text);
    let tab = &mut st.tabs[st.active_tab];
    collect_and_fetch_images(&dom_for_images, &url, &mut tab.webview, &mut st.cookies);

    let tab = &mut st.tabs[st.active_tab];
    tab.webview.set_html(&body_text);
    let title = tab.webview.get_title().unwrap_or_else(|| String::from("Untitled"));

    let url_string = format_url(&url);
    if tab.history.is_empty() || tab.history_pos >= tab.history.len()
        || tab.history[tab.history_pos] != url_string
    {
        if tab.history_pos + 1 < tab.history.len() {
            tab.history.truncate(tab.history_pos + 1);
        }
        tab.history.push(url_string.clone());
        tab.history_pos = tab.history.len() - 1;
    }

    tab.current_url = Some(url);
    tab.page_title = title;
    tab.url_text = url_string.clone();
    tab.status_text = String::from("Done");

    let st = state();
    st.url_field.set_text(&st.tabs[st.active_tab].url_text);
    update_title();
    update_status();
    update_tab_labels();
}

fn go_back() {
    let st = state();
    let tab = &st.tabs[st.active_tab];
    if tab.can_go_back() {
        let new_pos = tab.history_pos - 1;
        let url = tab.history[new_pos].clone();
        st.tabs[st.active_tab].history_pos = new_pos;
        navigate(&url);
    }
}

fn go_forward() {
    let st = state();
    let tab = &st.tabs[st.active_tab];
    if tab.can_go_forward() {
        let new_pos = tab.history_pos + 1;
        let url = tab.history[new_pos].clone();
        st.tabs[st.active_tab].history_pos = new_pos;
        navigate(&url);
    }
}

fn reload() {
    let st = state();
    let url = st.tabs[st.active_tab].url_text.clone();
    if !url.is_empty() {
        navigate(&url);
    }
}

// ---------------------------------------------------------------------------
// Image collection
// ---------------------------------------------------------------------------

fn collect_and_fetch_images(
    dom: &libwebview::dom::Dom,
    base_url: &http::Url,
    webview: &mut libwebview::WebView,
    cookies: &mut http::CookieJar,
) {
    for (i, node) in dom.nodes.iter().enumerate() {
        if let libwebview::dom::NodeType::Element { tag: libwebview::dom::Tag::Img, .. } = &node.node_type {
            if let Some(src) = dom.attr(i, "src") {
                let img_url = http::resolve_url(base_url, src);
                match http::fetch(&img_url, cookies) {
                    Ok(resp) => {
                        if let Some(info) = libimage_client::probe(&resp.body) {
                            let w = info.width as usize;
                            let h = info.height as usize;
                            let mut pixels = vec![0u32; w * h];
                            let mut scratch = vec![0u8; info.scratch_needed as usize];
                            if libimage_client::decode(&resp.body, &mut pixels, &mut scratch).is_ok() {
                                webview.add_image(src, pixels, info.width, info.height);
                            }
                        }
                    }
                    Err(_) => {}
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tab management
// ---------------------------------------------------------------------------

fn add_tab() {
    let st = state();
    let mut tab = TabState::new();
    // Set up callbacks for the new webview.
    tab.webview.set_link_callback(on_link_click, 0);
    tab.webview.set_submit_callback(on_form_submit, 0);
    // Add the scroll view to our content area.
    st.content_view.add(tab.webview.scroll_view());
    tab.webview.scroll_view().set_dock(ui::DOCK_FILL);
    // Hide all existing tabs' scroll views.
    for t in &st.tabs {
        t.webview.scroll_view().set_visible(false);
    }
    st.tabs.push(tab);
    st.active_tab = st.tabs.len() - 1;
    st.url_field.set_text("");
    update_title();
    update_tab_labels();
}

fn close_tab(idx: usize) {
    let st = state();
    if st.tabs.len() <= 1 {
        ui::quit();
        return;
    }
    // Remove the scroll view from the content area.
    st.tabs[idx].webview.scroll_view().remove();
    st.tabs.remove(idx);
    if st.active_tab >= st.tabs.len() {
        st.active_tab = st.tabs.len() - 1;
    }
    switch_tab(st.active_tab);
}

fn switch_tab(idx: usize) {
    let st = state();
    if idx >= st.tabs.len() { return; }
    // Hide old tab's scroll view.
    st.tabs[st.active_tab].webview.scroll_view().set_visible(false);
    st.active_tab = idx;
    // Show new tab's scroll view.
    st.tabs[st.active_tab].webview.scroll_view().set_visible(true);
    // Update URL bar and title.
    st.url_field.set_text(&st.tabs[st.active_tab].url_text);
    update_title();
    update_status();
    update_tab_labels();
}

// ---------------------------------------------------------------------------
// UI helpers
// ---------------------------------------------------------------------------

fn update_title() {
    let st = state();
    let tab = &st.tabs[st.active_tab];
    if tab.page_title.is_empty() {
        st.win.set_title("Surf");
    } else {
        let mut title = String::from("Surf - ");
        title.push_str(&tab.page_title);
        st.win.set_title(&title);
    }
}

fn update_status() {
    let st = state();
    let tab = &st.tabs[st.active_tab];
    st.status_label.set_text(&tab.status_text);
}

fn update_tab_labels() {
    // Build tab labels string for tab bar. Pipe-separated labels.
    let st = state();
    let mut labels = String::new();
    for (i, tab) in st.tabs.iter().enumerate() {
        if i > 0 { labels.push('|'); }
        let label = tab.tab_label();
        // Truncate long labels.
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

fn format_url(url: &http::Url) -> String {
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

fn push_u32(s: &mut String, val: u32) {
    if val >= 10 { push_u32(s, val / 10); }
    s.push((b'0' + (val % 10) as u8) as char);
}

// ---------------------------------------------------------------------------
// Callbacks
// ---------------------------------------------------------------------------

extern "C" fn on_link_click(ctrl_id: u32, _event_type: u32, _userdata: u64) {
    let st = state();
    let tab = &st.tabs[st.active_tab];
    if let Some(link_url) = tab.webview.link_url_for(ctrl_id) {
        let resolved = if let Some(ref base) = tab.current_url {
            let resolved_url = http::resolve_url(base, link_url);
            format_url(&resolved_url)
        } else {
            String::from(link_url)
        };
        navigate(&resolved);
    }
}

extern "C" fn on_form_submit(ctrl_id: u32, _event_type: u32, _userdata: u64) {
    let st = state();
    let tab = &st.tabs[st.active_tab];

    if !tab.webview.is_submit_button(ctrl_id) {
        return;
    }

    // Get form action and method.
    let (action, method) = match tab.webview.form_action_for(ctrl_id) {
        Some(am) => am,
        None => return,
    };

    // Collect form data.
    let data = tab.webview.collect_form_data(ctrl_id);

    // URL-encode the form data.
    let mut encoded = String::new();
    for (i, (name, value)) in data.iter().enumerate() {
        if i > 0 { encoded.push('&'); }
        url_encode_into(&mut encoded, name);
        encoded.push('=');
        url_encode_into(&mut encoded, value);
    }

    // Resolve action URL relative to current page.
    let resolved_action = if let Some(ref base) = tab.current_url {
        let action_url = http::resolve_url(base, &action);
        format_url(&action_url)
    } else {
        action
    };

    if method == "POST" {
        navigate_post(&resolved_action, &encoded);
    } else {
        // GET: append query string to URL.
        let mut url = resolved_action;
        if !encoded.is_empty() {
            url.push(if url.contains('?') { '&' } else { '?' });
            url.push_str(&encoded);
        }
        navigate(&url);
    }
}

fn url_encode_into(out: &mut String, s: &str) {
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

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    anyos_std::println!("[surf] starting...");

    if !ui::init() {
        anyos_std::println!("[surf] ERROR: failed to init libanyui");
        return;
    }

    // Check if URL argument was provided.
    let mut args_buf = [0u8; 256];
    let raw_args = anyos_std::process::args(&mut args_buf);
    let arg_url = raw_args.trim();
    let start_url = if arg_url.is_empty() { None } else { Some(String::from(arg_url)) };

    // Create window.
    let win = ui::Window::new("Surf", -1, -1, 900, 700);

    // -- Toolbar (DOCK_TOP, 40px) --
    let toolbar = ui::View::new();
    toolbar.set_dock(ui::DOCK_TOP);
    toolbar.set_size(0, 40);
    toolbar.set_color(0xFF2A2A2C);
    win.add(&toolbar);

    let btn_back = ui::Button::new("<");
    btn_back.set_position(8, 6);
    btn_back.set_size(32, 28);
    toolbar.add(&btn_back);

    let btn_forward = ui::Button::new(">");
    btn_forward.set_position(42, 6);
    btn_forward.set_size(32, 28);
    toolbar.add(&btn_forward);

    let btn_reload = ui::Button::new("R");
    btn_reload.set_position(76, 6);
    btn_reload.set_size(32, 28);
    toolbar.add(&btn_reload);

    let url_field = ui::TextField::new();
    url_field.set_position(116, 6);
    url_field.set_size(750, 28);
    url_field.set_placeholder("Enter URL...");
    toolbar.add(&url_field);

    // -- Tab bar (DOCK_TOP, 30px) — using TabBar control --
    let tab_bar_view = ui::TabBar::new("New Tab");
    tab_bar_view.set_dock(ui::DOCK_TOP);
    tab_bar_view.set_size(0, 30);
    win.add(&tab_bar_view);

    // -- Status bar (DOCK_BOTTOM, 24px) --
    let status_label = ui::Label::new("Ready");
    status_label.set_dock(ui::DOCK_BOTTOM);
    status_label.set_size(0, 24);
    status_label.set_color(0xFF252525);
    status_label.set_text_color(0xFF969696);
    status_label.set_font_size(12);
    status_label.set_padding(8, 4, 0, 0);
    win.add(&status_label);

    // -- Content area (DOCK_FILL) --
    let content_view = ui::View::new();
    content_view.set_dock(ui::DOCK_FILL);
    content_view.set_color(0xFFFFFFFF);
    win.add(&content_view);

    // Create initial tab.
    let mut initial_tab = TabState::new();
    initial_tab.webview.set_link_callback(on_link_click, 0);
    initial_tab.webview.set_submit_callback(on_form_submit, 0);
    content_view.add(initial_tab.webview.scroll_view());
    initial_tab.webview.scroll_view().set_dock(ui::DOCK_FILL);

    unsafe {
        STATE = Some(AppState {
            win,
            toolbar,
            btn_back,
            btn_forward,
            btn_reload,
            url_field,
            tab_bar_view,
            content_view,
            status_label,
            tabs: vec![initial_tab],
            active_tab: 0,
            cookies: http::CookieJar { cookies: Vec::new() },
        });
    }

    // Set up button callbacks.
    let st = state();
    st.btn_back.on_click(|_| { go_back(); });
    st.btn_forward.on_click(|_| { go_forward(); });
    st.btn_reload.on_click(|_| { reload(); });

    // URL field: navigate on Enter.
    st.url_field.on_submit(|e| {
        let st = state();
        let mut buf = [0u8; 2048];
        let len = ui::Control::from_id(e.id).get_text(&mut buf);
        if len > 0 {
            if let Ok(url_str) = core::str::from_utf8(&buf[..len as usize]) {
                let url = String::from(url_str);
                st.tabs[st.active_tab].url_text = url.clone();
                navigate(&url);
            }
        }
    });

    // Tab bar: switch tabs on selection change.
    tab_bar_view.on_active_changed(|e| {
        switch_tab(e.index as usize);
    });

    // Window keyboard shortcuts.
    win.on_key_down(|e| {
        let mods = e.modifiers;
        let key = e.keycode;
        let ctrl = mods & 2 != 0;

        if ctrl && key == b'T' as u32 {
            add_tab();
        } else if ctrl && key == b'W' as u32 {
            let st = state();
            close_tab(st.active_tab);
        } else if ctrl && key == b'L' as u32 {
            let st = state();
            st.url_field.focus();
        } else if ctrl && key == b'R' as u32 {
            reload();
        }
    });

    // Window resize: update webview.
    win.on_resize(|_| {
        let st = state();
        let (w, h) = st.content_view.get_size();
        if w > 0 && h > 0 {
            let tab = &mut st.tabs[st.active_tab];
            tab.webview.resize(w, h);
        }
    });

    // Navigate to initial URL if provided.
    if let Some(url) = start_url {
        let st = state();
        st.tabs[st.active_tab].url_text = url.clone();
        st.url_field.set_text(&url);
        navigate(&url);
    }

    anyos_std::println!("[surf] entering event loop");
    ui::run();
}
