// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Surf — a tabbed web browser for anyOS.
//!
//! Renders HTML pages with CSS styling, fetched over HTTP/1.1.
//! Uses libanyui for the UI chrome (toolbar, tabs, status bar) and
//! libwebview for HTML content rendering via real UI controls.

#![no_std]
#![no_main]

mod http;
mod deflate;
mod tls;
mod tab;
mod resources;
mod ui;
mod callbacks;
mod ws;
mod net_worker;

anyos_std::entry!(main);

extern crate libfont_client;

use alloc::string::String;
use alloc::vec::Vec;
use alloc::vec;
use anyos_std::i18n;

use libanyui_client as ui_lib;
use ui_lib::Widget;

// ═══════════════════════════════════════════════════════════
// Debug helpers (feature-gated)
// ═══════════════════════════════════════════════════════════

/// Return current stack pointer for debug tracing.
#[cfg(feature = "debug_surf")]
#[inline(always)]
pub(crate) fn debug_rsp() -> u64 {
    let rsp: u64;
    unsafe { core::arch::asm!("mov {}, rsp", out(reg) rsp); }
    rsp
}

/// Return current heap break position for debug tracing.
#[cfg(feature = "debug_surf")]
pub(crate) fn debug_heap() -> u64 {
    anyos_std::process::sbrk(0) as u64
}

// ═══════════════════════════════════════════════════════════
// Global application state
// ═══════════════════════════════════════════════════════════

struct AppState {
    win: ui_lib::Window,
    toolbar: ui_lib::View,
    btn_back: ui_lib::IconButton,
    btn_forward: ui_lib::IconButton,
    btn_reload: ui_lib::IconButton,
    btn_menu: ui_lib::IconButton,
    url_field: ui_lib::TextField,
    /// Loading progress bar below the toolbar.
    url_progress: ui_lib::ProgressBar,
    tab_bar_view: ui_lib::TabBar,
    content_view: ui_lib::View,
    status_label: ui_lib::Label,
    /// DevTools console window (separate from main browser window).
    devtools_win: ui_lib::Window,
    /// Label inside the DevTools window showing JS console output.
    devtools_label: ui_lib::Label,
    /// Whether the DevTools window is currently visible.
    devtools_open: bool,
    tabs: Vec<tab::TabState>,
    active_tab: usize,
    cookies: http::CookieJar,
    /// Pending CSS fetch queue: (tab_index, href_attr, resolved_url).
    css_queue: Vec<(usize, String, http::Url)>,
    /// Timer ID for the async CSS fetch loop (0 = not running).
    css_timer: u32,
    /// Pending image fetch queue: (tab_index, img_src_attr, resolved_url).
    image_queue: Vec<(usize, String, http::Url)>,
    /// Timer ID for the async image fetch loop (0 = not running).
    image_timer: u32,
    /// HTTP connection pool for reusing TCP/TLS connections.
    conn_pool: http::ConnPool,
    /// All live WebSocket connections across all tabs.
    ws_connections: Vec<ws::WsConn>,
    /// Timer ID for the WebSocket poll loop (0 = not running).
    ws_poll_timer: u32,
    /// Timer ID for the CSS animation tick (0 = not running).
    anim_timer: u32,
    /// Timer ID for the network poll loop (0 = not running).
    net_poll_timer: u32,
    /// Per-tab dirty flags: set when CSS/images arrive, cleared after relayout.
    relayout_dirty: [bool; 16],
    /// Timer ID for the relayout debounce timer (0 = not running).
    relayout_timer: u32,
}

static mut STATE: Option<AppState> = None;

/// Return a mutable reference to the global `AppState`.
///
/// # Panics
/// Panics if called before `STATE` is initialised in `main`.
pub(crate) fn state() -> &'static mut AppState {
    unsafe { STATE.as_mut().unwrap() }
}

// ═══════════════════════════════════════════════════════════
// WebSocket integration helpers
// ═══════════════════════════════════════════════════════════

/// Drain the pending-connect queue for `tab_idx` and open the TCP connections.
///
/// Called after each `set_html` invocation so that WebSocket constructors
/// executed by page scripts are immediately connected.
pub(crate) fn connect_pending_ws(tab_idx: usize) {
    let st = state();
    let connects = st.tabs[tab_idx].webview.js_runtime().take_ws_connects();
    if connects.is_empty() {
        return;
    }
    for req in connects {
        // Borrow-split: we need both `ws_connections` and the tab's runtime.
        let runtime = st.tabs[tab_idx].webview.js_runtime();
        ws::handle_connect(req, &mut st.ws_connections, runtime, &st.cookies, tab_idx);
    }
    ws_start_poll_timer();
}

/// Start the WebSocket poll timer if it is not already running.
///
/// The timer fires every 50 ms, handles outbound sends/closes, and polls all
/// connections for incoming frames, routing each message to the JS runtime of
/// the tab that owns the connection.
fn ws_start_poll_timer() {
    let st = state();
    if st.ws_poll_timer != 0 {
        return;
    }
    st.ws_poll_timer = ui_lib::set_timer(50, || {
        let st = state();

        // Outbound: flush sends and closes from every tab's runtime.
        for tab_i in 0..st.tabs.len() {
            let sends = st.tabs[tab_i].webview.js_runtime().take_ws_sends();
            ws::handle_sends(sends, &mut st.ws_connections);

            let closes = st.tabs[tab_i].webview.js_runtime().take_ws_closes();
            let to_remove = ws::handle_closes(
                closes,
                &mut st.ws_connections,
                st.tabs[tab_i].webview.js_runtime(),
            );
            ws::remove_connections(&mut st.ws_connections, &to_remove);
        }

        // Inbound: poll each connection and deliver to the owning tab's runtime.
        for tab_i in 0..st.tabs.len() {
            let tab_conn_ids: Vec<u64> = st.ws_connections
                .iter()
                .filter(|c| c.tab_idx == tab_i)
                .map(|c| c.id)
                .collect();
            if tab_conn_ids.is_empty() { continue; }

            let runtime = st.tabs[tab_i].webview.js_runtime();
            let mut tab_conns: Vec<ws::WsConn> = Vec::new();
            let mut rest: Vec<ws::WsConn> = Vec::new();
            let all = core::mem::replace(&mut st.ws_connections, Vec::new());
            for c in all {
                if c.tab_idx == tab_i { tab_conns.push(c); } else { rest.push(c); }
            }
            let to_close = ws::poll_connections(&mut tab_conns, runtime);
            ws::remove_connections(&mut tab_conns, &to_close);
            for c in tab_conns { st.ws_connections.push(c); }
            for c in rest { st.ws_connections.push(c); }
        }

        if st.ws_connections.is_empty() {
            ui_lib::kill_timer(st.ws_poll_timer);
            st.ws_poll_timer = 0;
        }
    });
}

// ═══════════════════════════════════════════════════════════
// CSS animation tick
// ═══════════════════════════════════════════════════════════

/// Start the 16 ms animation / scroll tick timer.
///
/// Each tick calls `WebView::tick(16)` on the active tab.  The timer
/// automatically kills itself after a period of inactivity (no animations,
/// no JS timers, no pending tile creation) and is restarted by
/// `ensure_anim_timer()` when new work arrives (page load, scroll, etc.).
pub(crate) fn start_anim_timer() {
    let st = state();
    if st.anim_timer != 0 { return; }
    static mut IDLE_TICKS: u32 = 0;
    st.anim_timer = ui_lib::set_timer(16, || {
        let st = state();
        let changed = st.tabs[st.active_tab].webview.tick(16);
        if changed {
            unsafe { IDLE_TICKS = 0; }
        } else {
            unsafe { IDLE_TICKS += 1; }
            // After ~300 ms of no work (20 ticks × 16ms), stop the timer.
            if unsafe { IDLE_TICKS } > 20 {
                unsafe { IDLE_TICKS = 0; }
                if st.anim_timer != 0 {
                    ui_lib::kill_timer(st.anim_timer);
                    st.anim_timer = 0;
                }
            }
        }
    });
}

/// Ensure the animation timer is running (restart if stopped).
///
/// Call this when new work arrives: page navigation, scroll events,
/// new CSS/image resources, etc.
pub(crate) fn ensure_anim_timer() {
    start_anim_timer();
}

// ═══════════════════════════════════════════════════════════
// Network worker result processing
// ═══════════════════════════════════════════════════════════

/// Start the network poll timer if not already running.
///
/// Fires every 50 ms, drains completed fetch results from the worker thread.
/// Auto-stops after 60 consecutive empty polls (~3 seconds idle).
/// Restarted by `ensure_net_poll_timer()` when new fetches are submitted.
fn start_net_poll_timer() {
    let st = state();
    if st.net_poll_timer != 0 { return; }
    static mut EMPTY_POLLS: u32 = 0;
    st.net_poll_timer = ui_lib::set_timer(50, || {
        let results = net_worker::drain_results();
        if results.is_empty() {
            unsafe { EMPTY_POLLS += 1; }
            if unsafe { EMPTY_POLLS } > 60 {
                unsafe { EMPTY_POLLS = 0; }
                let st = state();
                if st.net_poll_timer != 0 {
                    ui_lib::kill_timer(st.net_poll_timer);
                    st.net_poll_timer = 0;
                }
            }
            return;
        }
        unsafe { EMPTY_POLLS = 0; }
        process_fetched_results(results);
    });
}

/// Ensure the network poll timer is running. Called when new fetches are submitted.
pub(crate) fn ensure_net_poll_timer() {
    start_net_poll_timer();
}

/// Dispatch completed fetch results to their handlers.
///
/// CSS and image results set per-tab dirty flags instead of triggering
/// immediate relayouts.  A separate debounce timer (`flush_relayout`)
/// coalesces all pending relayouts into one pass every 300 ms.
fn process_fetched_results(results: Vec<net_worker::FetchResult>) {
    for result in results {
        match result {
            net_worker::FetchResult::NavDone { response, url, cookies, generation } => {
                handle_nav_done(response, url, cookies, generation);
            }
            net_worker::FetchResult::NavError { error_msg, generation } => {
                handle_nav_error(error_msg, generation);
            }
            net_worker::FetchResult::CssDone { tab_index, href, body, headers, generation } => {
                if handle_css_done(tab_index, href, body, headers, generation) {
                    mark_relayout_dirty(tab_index);
                }
            }
            net_worker::FetchResult::ImageDone { tab_index, src, body, headers, generation } => {
                if handle_image_done(tab_index, src, body, headers, generation) {
                    mark_relayout_dirty(tab_index);
                }
            }
            net_worker::FetchResult::FontDone { tab_index, family, body, generation } => {
                if handle_font_done(tab_index, family, body, generation) {
                    mark_relayout_dirty(tab_index);
                }
            }
            net_worker::FetchResult::ScriptDone { tab_index, slot, body, headers, generation } => {
                handle_script_done(tab_index, slot, body, headers, generation);
            }
        }
    }
}

/// Mark a tab as needing a relayout and start the debounce timer if not
/// already running.  The actual relayout happens in `flush_relayout()`.
fn mark_relayout_dirty(tab_index: usize) {
    let st = state();
    if tab_index < st.relayout_dirty.len() {
        st.relayout_dirty[tab_index] = true;
    }
    // Start the debounce timer if not already running.
    if st.relayout_timer == 0 {
        st.relayout_timer = ui_lib::set_timer(300, flush_relayout);
    }
    // Ensure the anim timer is running so new tiles are created after relayout.
    ensure_anim_timer();
}

/// Debounce callback: perform one relayout per dirty tab, then clear flags
/// and stop the timer.
fn flush_relayout() {
    let st = state();
    let mut any_dirty = false;

    for tab_idx in 0..st.relayout_dirty.len() {
        if st.relayout_dirty[tab_idx] {
            st.relayout_dirty[tab_idx] = false;
            if tab_idx < st.tabs.len() {
                st.tabs[tab_idx].webview.relayout();
            }
        }
    }

    // Check if new dirty flags were set during the relayouts above
    // (unlikely but possible if relayout triggers further resource loads).
    for &d in &st.relayout_dirty {
        if d { any_dirty = true; break; }
    }

    if !any_dirty {
        // All clean — kill the debounce timer.
        if st.relayout_timer != 0 {
            ui_lib::kill_timer(st.relayout_timer);
            st.relayout_timer = 0;
        }
    }
}

/// Handle a completed navigation fetch: decode body, render HTML, update
/// history, queue external resources.
fn handle_nav_done(
    response: http::Response,
    original_url: http::Url,
    worker_cookies: http::CookieJar,
    generation: u32,
) {
    let st = state();
    let tab_idx = st.active_tab;

    // Discard stale result from a previous navigation.
    if st.tabs[tab_idx].nav_generation != generation {
        return;
    }

    // Merge cookies that the worker collected during the fetch.
    merge_cookies(worker_cookies);

    // HTTP error check.
    if response.status < 200 || response.status >= 400 {
        let mut msg = String::from("HTTP error ");
        ui::push_u32(&mut msg, response.status as u32);
        st.tabs[tab_idx].is_loading = false;
        st.tabs[tab_idx].status_text = msg;
        ui::update_status();
        ui::update_tab_labels();
        return;
    }

    st.tabs[tab_idx].status_text = String::from("Rendering...");
    ui::update_status();

    // Decode response body (charset detection + Latin-1 transcoding).
    let body_text = resources::decode_http_body(&response.body, &response.headers);

    // Determine base URL (post-redirect URL takes precedence).
    let base_url = response.final_url.unwrap_or(original_url);
    let url_str = ui::format_url(&base_url);

    // Clear all state from the previous page (DOM, layout, images, JS, CSS).
    st.tabs[tab_idx].webview.navigate_clear();

    // Set URL and cookies on the JS runtime before rendering.
    st.tabs[tab_idx].webview.set_url(&url_str);
    let is_secure = base_url.scheme == "https";
    if let Some(cookie_hdr) = st.cookies.cookie_header(&base_url.host, &base_url.path, is_secure) {
        st.tabs[tab_idx].webview.js_runtime().set_cookies(&cookie_hdr);
    } else {
        st.tabs[tab_idx].webview.js_runtime().set_cookies("");
    }

    // Parse and render the HTML document (without JS — scripts are executed
    // after external scripts have been fetched, matching the surf-host flow).
    st.tabs[tab_idx].webview.set_html_no_js(&body_text);

    // Collect script entries (inline + external) in document order.
    // Inline scripts are stored immediately; external scripts are queued
    // for async fetch via the net_worker.  Once all external fetches
    // complete, scripts are executed in document order via execute_js().
    let generation = st.tabs[tab_idx].nav_generation;
    {
        let entries = st.tabs[tab_idx].webview.script_entries();
        let mut pending = Vec::with_capacity(entries.len());
        let mut ext_count = 0usize;
        for (slot, entry) in entries.iter().enumerate() {
            match entry {
                libwebview::js::ScriptEntry::Inline(text) => {
                    pending.push(Some(text.clone()));
                }
                libwebview::js::ScriptEntry::External(src_url) => {
                    let full_url = http::resolve_url(&base_url, src_url);
                    anyos_std::println!("[surf] queuing script fetch [{}]: {}", slot, src_url);
                    net_worker::submit(net_worker::FetchRequest::Script {
                        tab_index: tab_idx,
                        slot,
                        src: src_url.clone(),
                        url: full_url,
                        generation,
                    });
                    pending.push(None); // placeholder — filled when fetch completes
                    ext_count += 1;
                }
            }
        }
        st.tabs[tab_idx].pending_scripts = pending;
        st.tabs[tab_idx].pending_script_count = ext_count;

        // If there are no external scripts, execute immediately.
        if ext_count == 0 && !entries.is_empty() {
            execute_pending_scripts(tab_idx);
        }
    }

    // Extract page title.
    let title = st.tabs[tab_idx].webview.get_title()
        .unwrap_or_else(String::new);

    // Update navigation history — only push if URL differs from current position.
    let at_same = if !st.tabs[tab_idx].history.is_empty()
        && st.tabs[tab_idx].history_pos < st.tabs[tab_idx].history.len()
    {
        st.tabs[tab_idx].history[st.tabs[tab_idx].history_pos] == url_str
    } else {
        false
    };

    if !at_same {
        // Truncate any forward history.
        if !st.tabs[tab_idx].history.is_empty() {
            let pos = st.tabs[tab_idx].history_pos;
            st.tabs[tab_idx].history.truncate(pos + 1);
        }
        st.tabs[tab_idx].history.push(url_str.clone());
        st.tabs[tab_idx].history_pos = st.tabs[tab_idx].history.len() - 1;
    }

    st.tabs[tab_idx].page_title = title;
    st.tabs[tab_idx].url_text = url_str;
    st.tabs[tab_idx].current_url = Some(base_url.clone());
    st.tabs[tab_idx].status_text = String::from("Done");
    st.tabs[tab_idx].is_loading = false;

    // Update chrome UI.
    let url_for_field = st.tabs[tab_idx].url_text.clone();
    st.url_field.set_text(&url_for_field);
    ui::update_title();
    ui::update_status();
    ui::update_tab_labels();
    ui::update_devtools();

    // Connect any WebSockets that JS requested during set_html().
    connect_pending_ws(tab_idx);

    // Queue external CSS, images, and web fonts for async fetch via the worker thread.
    // Queue external CSS, images, and web fonts.
    if let Some(dom) = st.tabs[tab_idx].webview.dom() {
        resources::queue_stylesheets(dom, &base_url, tab_idx);
        resources::queue_images(dom, &base_url, tab_idx);
        resources::queue_inline_svgs(dom, tab_idx);
    }
    // Queue @font-face downloads from inline <style> blocks.
    resources::queue_font_faces(&st.tabs[tab_idx].webview, &base_url, tab_idx);

    // Restart animation/scroll tick timer (may have been stopped while idle).
    ensure_anim_timer();
}

/// Handle a navigation error: show the error message in the status bar.
fn handle_nav_error(error_msg: &'static str, generation: u32) {
    let st = state();
    let tab_idx = st.active_tab;
    if st.tabs[tab_idx].nav_generation != generation {
        return;
    }
    st.tabs[tab_idx].is_loading = false;
    st.tabs[tab_idx].status_text = String::from(error_msg);
    ui::update_status();
    ui::update_tab_labels();
}

/// Handle a completed CSS stylesheet fetch: apply the stylesheet.
///
/// Returns `true` if the stylesheet was applied and a relayout is needed.
/// The caller batches relayouts to avoid redundant work.
fn handle_css_done(
    tab_index: usize,
    href: String,
    body: Vec<u8>,
    headers: String,
    generation: u32,
) -> bool {
    let st = state();
    if tab_index >= st.tabs.len() { return false; }
    if st.tabs[tab_index].nav_generation != generation { return false; }

    let css_text = resources::decode_http_body(&body, &headers);
    st.tabs[tab_index].webview.add_stylesheet(&css_text);
    anyos_std::println!("[surf] applied CSS: {}", href);

    // Process @import URLs from the newly added stylesheet.
    if let Ok(base_url) = crate::http::parse_url(&href) {
        let imports: Vec<String> = st.tabs[tab_index].webview.last_stylesheet_imports().to_vec();
        for import_url in imports {
            let resolved = crate::http::resolve_url(&base_url, &import_url);
            crate::net_worker::submit(crate::net_worker::FetchRequest::Css {
                tab_index,
                href: String::from(import_url.as_str()),
                url: resolved,
                generation,
            });
        }

        // Process @font-face rules — queue font downloads.
        let font_faces: Vec<(String, String)> = st.tabs[tab_index].webview
            .last_stylesheet_font_faces()
            .iter()
            .map(|ff| (ff.family.clone(), ff.src_url.clone()))
            .collect();
        for (family, src_url) in font_faces {
            let resolved = crate::http::resolve_url(&base_url, &src_url);
            crate::net_worker::submit(crate::net_worker::FetchRequest::Font {
                tab_index,
                family,
                url: resolved,
                generation,
            });
        }
    }

    true
}

/// Handle a completed web font fetch: load TTF data into libfont.
///
/// Returns `true` if the font was loaded and a relayout is needed.
fn handle_font_done(
    tab_index: usize,
    family: String,
    body: Vec<u8>,
    generation: u32,
) -> bool {
    let st = state();
    if tab_index >= st.tabs.len() { return false; }
    if st.tabs[tab_index].nav_generation != generation { return false; }

    // Try loading the font data (supports TTF/OTF and WOFF2 via Brotli decompression).
    if let Some(font_id) = libfont_client::load_data(&body) {
        st.tabs[tab_index].webview.register_web_font(&family, font_id);
        anyos_std::println!("[surf] loaded web font '{}' -> id {}", family, font_id);
        true
    } else {
        anyos_std::println!("[surf] failed to load web font '{}'", family);
        false
    }
}

/// Handle a completed image fetch: decode SVG or raster and add to cache.
///
/// Returns `true` if the image was decoded and a relayout is needed.
/// The caller batches relayouts to avoid redundant work.
fn handle_image_done(
    tab_index: usize,
    src: String,
    body: Vec<u8>,
    headers: String,
    generation: u32,
) -> bool {
    let st = state();
    if tab_index >= st.tabs.len() { return false; }
    if st.tabs[tab_index].nav_generation != generation { return false; }

    if resources::is_svg(&src, &headers) {
        resources::decode_svg_no_relayout(&body, &src, tab_index);
    } else {
        resources::decode_raster_no_relayout(&body, &src, tab_index);
    }
    true
}

/// Handle a completed external script fetch.
///
/// Places the fetched text into the tab's `pending_scripts` slot.
/// When all slots are filled, executes all scripts in document order.
fn handle_script_done(
    tab_index: usize,
    slot: usize,
    body: Vec<u8>,
    headers: String,
    generation: u32,
) {
    let st = state();
    if tab_index >= st.tabs.len() { return; }
    if st.tabs[tab_index].nav_generation != generation { return; }
    if slot >= st.tabs[tab_index].pending_scripts.len() { return; }

    let text = resources::decode_http_body(&body, &headers);
    anyos_std::println!("[surf] script [{}] fetched: {} bytes", slot, text.len());
    st.tabs[tab_index].pending_scripts[slot] = Some(text);

    if st.tabs[tab_index].pending_script_count > 0 {
        st.tabs[tab_index].pending_script_count -= 1;
    }

    // All external scripts fetched — execute everything in document order.
    if st.tabs[tab_index].pending_script_count == 0 {
        execute_pending_scripts(tab_index);
    }
}

/// Execute all pending scripts for a tab in document order.
///
/// Called when all external script fetches have completed (or immediately
/// if the page has only inline scripts).
fn execute_pending_scripts(tab_index: usize) {
    let st = state();
    if tab_index >= st.tabs.len() { return; }

    let scripts: Vec<String> = st.tabs[tab_index].pending_scripts
        .iter()
        .filter_map(|s| s.clone())
        .collect();
    // Clear pending state.
    st.tabs[tab_index].pending_scripts.clear();
    st.tabs[tab_index].pending_script_count = 0;

    if scripts.is_empty() { return; }

    anyos_std::println!("[surf] executing {} scripts in document order", scripts.len());
    st.tabs[tab_index].webview.execute_js(&scripts);

    // Flush JS console output.
    for line in st.tabs[tab_index].webview.js_console() {
        anyos_std::println!("[surf-js] {}", line);
    }

    // JS may have requested WebSocket connections.
    connect_pending_ws(tab_index);

    // Relayout to reflect DOM mutations from JS.
    mark_relayout_dirty(tab_index);
}

/// Merge cookies returned by the worker thread into the main cookie jar.
///
/// Worker-side cookies take precedence (they represent the most recent
/// Set-Cookie headers from the server).
fn merge_cookies(worker_jar: http::CookieJar) {
    let st = state();
    for cookie in worker_jar.cookies {
        // Replace existing cookie with same name+domain+path, or add new.
        let existing = st.cookies.cookies.iter_mut().find(|c| {
            c.name == cookie.name && c.domain == cookie.domain && c.path == cookie.path
        });
        if let Some(existing) = existing {
            existing.value = cookie.value;
            existing.secure = cookie.secure;
            existing.http_only = cookie.http_only;
        } else {
            st.cookies.cookies.push(cookie);
        }
    }
}

// ═══════════════════════════════════════════════════════════
// Entry point
// ═══════════════════════════════════════════════════════════

fn main() {
    anyos_std::println!("[surf] starting...");

    if !ui_lib::init() {
        anyos_std::println!("[surf] ERROR: failed to init libanyui");
        return;
    }
    i18n::init();

    if !libsvg_client::init() {
        anyos_std::println!("[surf] WARN: libsvg.so not available — SVG images disabled");
    }

    // Optional startup URL from the process argument string.
    let mut args_buf = [0u8; 256];
    let raw_args = anyos_std::process::args(&mut args_buf);
    let arg_url = raw_args.trim();
    let start_url = if arg_url.is_empty() { None } else { Some(String::from(arg_url)) };

    // ── Window ──────────────────────────────────────────────────────────────
    let win = ui_lib::Window::new(i18n::t("Surf"), -1, -1, 900, 700);

    // ── Toolbar (DOCK_TOP, 40 px) ────────────────────────────────────────────
    let toolbar = ui_lib::View::new();
    toolbar.set_dock(ui_lib::DOCK_TOP);
    toolbar.set_size(0, 40);
    toolbar.set_color(0xFF2A2A2C);
    toolbar.set_padding(6, 6, 6, 6);
    win.add(&toolbar);

    // Navigation buttons (DOCK_LEFT).
    let nav_group = ui_lib::View::new();
    nav_group.set_dock(ui_lib::DOCK_LEFT);
    nav_group.set_size(104, 28);
    nav_group.set_color(0xFF2A2A2C);
    toolbar.add(&nav_group);

    let btn_back = ui_lib::IconButton::new("");
    btn_back.set_position(0, 0);
    btn_back.set_size(32, 28);
    btn_back.set_system_icon("chevron-left", ui_lib::IconType::Outline, 0xFFCCCCCC, 20);
    nav_group.add(&btn_back);

    let btn_forward = ui_lib::IconButton::new("");
    btn_forward.set_position(34, 0);
    btn_forward.set_size(32, 28);
    btn_forward.set_system_icon("chevron-right", ui_lib::IconType::Outline, 0xFFCCCCCC, 20);
    nav_group.add(&btn_forward);

    let btn_reload = ui_lib::IconButton::new("");
    btn_reload.set_position(68, 0);
    btn_reload.set_size(32, 28);
    btn_reload.set_system_icon("refresh", ui_lib::IconType::Outline, 0xFFCCCCCC, 20);
    nav_group.add(&btn_reload);

    // Hamburger menu button (DOCK_RIGHT).
    let btn_menu = ui_lib::IconButton::new("");
    btn_menu.set_dock(ui_lib::DOCK_RIGHT);
    btn_menu.set_size(36, 28);
    btn_menu.set_system_icon("menu-2", ui_lib::IconType::Outline, 0xFFCCCCCC, 20);
    toolbar.add(&btn_menu);

    // URL field (DOCK_FILL — takes all remaining width).
    let url_field = ui_lib::TextField::new();
    url_field.set_dock(ui_lib::DOCK_FILL);
    url_field.set_placeholder(i18n::t("Enter URL..."));
    toolbar.add(&url_field);

    // Loading progress bar (DOCK_TOP, 3px, below toolbar).
    let url_progress = ui_lib::ProgressBar::new(0);
    url_progress.set_dock(ui_lib::DOCK_TOP);
    url_progress.set_size(0, 3);
    url_progress.set_color(0xFF0A84FF);  // blue accent
    url_progress.set_visible(false);
    win.add(&url_progress);

    // Hamburger context menu.
    let menu_items = anyos_std::format!(
        "{}|{}|-|{}|{}|{}|-|{}|{}|{}|-|{}|{}|-|{}",
        i18n::t("New Tab"),
        i18n::t("Close Tab"),
        i18n::t("History"),
        i18n::t("Downloads"),
        i18n::t("Bookmarks"),
        i18n::t("Zoom In"),
        i18n::t("Zoom Out"),
        i18n::t("Reset Zoom"),
        i18n::t("Settings"),
        i18n::t("Developer Tools"),
        i18n::t("About Surf"),
    );
    let hamburger_menu = ui_lib::ContextMenu::new(&menu_items);
    btn_menu.set_context_menu(&hamburger_menu);

    // ── Tab bar (DOCK_TOP, 30 px) ────────────────────────────────────────────
    let tab_bar_view = ui_lib::TabBar::new(i18n::t("New Tab"));
    tab_bar_view.set_dock(ui_lib::DOCK_TOP);
    tab_bar_view.set_size(0, 30);
    win.add(&tab_bar_view);

    // ── DevTools window (separate window, initially hidden) ─────────────────
    // Create off-screen (x=9999) so it doesn't flash on startup.
    // toggle_devtools() will move it to a visible position when opened.
    let devtools_win = ui_lib::Window::new(i18n::t("DevTools - Console"), 9999, 9999, 700, 400);
    devtools_win.set_visible(false);

    let devtools_label = ui_lib::Label::new("");
    devtools_label.set_dock(ui_lib::DOCK_FILL);
    devtools_label.set_color(0xFF1C1C1E);
    devtools_label.set_text_color(0xFF30D158);   // green console text
    devtools_label.set_font_size(12);
    devtools_label.set_padding(8, 8, 8, 8);
    devtools_win.add(&devtools_label);

    // ── Status bar (DOCK_BOTTOM, 24 px) ─────────────────────────────────────
    let status_label = ui_lib::Label::new(i18n::t("Ready"));
    status_label.set_dock(ui_lib::DOCK_BOTTOM);
    status_label.set_size(0, 24);
    status_label.set_color(0xFF252525);
    status_label.set_text_color(0xFF969696);
    status_label.set_font_size(12);
    status_label.set_padding(8, 4, 0, 0);
    win.add(&status_label);

    // ── Content area (DOCK_FILL) ─────────────────────────────────────────────
    let content_view = ui_lib::View::new();
    content_view.set_dock(ui_lib::DOCK_FILL);
    content_view.set_color(0xFFFFFFFF);
    win.add(&content_view);

    // ── Initial tab ──────────────────────────────────────────────────────────
    let mut initial_tab = tab::TabState::new();
    initial_tab.webview.set_link_callback(callbacks::on_link_click, 0);
    initial_tab.webview.set_submit_callback(callbacks::on_form_submit, 0);
    content_view.add(initial_tab.webview.scroll_view());
    initial_tab.webview.scroll_view().set_dock(ui_lib::DOCK_FILL);
    initial_tab.webview.scroll_view().on_scroll(|_| { ensure_anim_timer(); });

    unsafe {
        STATE = Some(AppState {
            win,
            toolbar,
            btn_back,
            btn_forward,
            btn_reload,
            btn_menu,
            url_field,
            url_progress,
            tab_bar_view,
            content_view,
            status_label,
            devtools_win,
            devtools_label,
            devtools_open: false,
            tabs: vec![initial_tab],
            active_tab: 0,
            cookies: http::CookieJar { cookies: Vec::new() },
            css_queue: Vec::new(),
            css_timer: 0,
            image_queue: Vec::new(),
            image_timer: 0,
            conn_pool: http::ConnPool::new(),
            ws_connections: Vec::new(),
            ws_poll_timer: 0,
            anim_timer: 0,
            net_poll_timer: 0,
            relayout_dirty: [false; 16],
            relayout_timer: 0,
        });
    }

    // Initialize background network worker queues.
    net_worker::init();

    // ── Menu bar ──
    let mut mb = ui_lib::MenuBarBuilder::new()
        .menu(i18n::t("File"))
            .item(1, i18n::t("New Tab"), 0)
            .item(2, i18n::t("Close Tab"), 0)
            .separator()
            .item(3, i18n::t("Quit"), 0)
        .end_menu()
        .menu(i18n::t("Edit"))
            .item(10, i18n::t("Cut"), 0)
            .item(11, i18n::t("Copy"), 0)
            .item(12, i18n::t("Paste"), 0)
        .end_menu()
        .menu(i18n::t("View"))
            .item(20, i18n::t("Reload"), 0)
            .separator()
            .item(21, i18n::t("DevTools Console"), 0)
        .end_menu()
        .menu(i18n::t("Navigate"))
            .item(30, i18n::t("Back"), 0)
            .item(31, i18n::t("Forward"), 0)
        .end_menu();
    let menu_data = mb.build();
    let menu = ui_lib::MenuBar::set(win.id(), menu_data);
    menu.on_item(|e| {
        match e.item_id {
            1 => ui::add_tab(),
            2 => { let st = state(); ui::close_tab(st.active_tab); }
            3 => ui_lib::quit(),
            10 | 11 | 12 => {} // handled by focused control
            20 => tab::reload(),
            21 => ui::toggle_devtools(),
            30 => tab::go_back(),
            31 => tab::go_forward(),
            _ => {}
        }
    });

    // ── Button callbacks ─────────────────────────────────────────────────────
    let st = state();
    st.btn_back.on_click(|_| { tab::go_back(); });
    st.btn_forward.on_click(|_| { tab::go_forward(); });
    st.btn_reload.on_click(|_| { tab::reload(); });

    // Hamburger menu button: open popup on left-click.
    st.btn_menu.on_click(|_| {
        let st = state();
        st.btn_menu.open_popup();
    });

    // Hamburger menu item handler.
    hamburger_menu.on_item_click(|e| {
        match e.index {
            0 => ui::add_tab(),                                      // New Tab
            1 => { let st = state(); ui::close_tab(st.active_tab); } // Close Tab
            // 2 = separator
            3 => {}  // History (TODO)
            4 => {}  // Downloads (TODO)
            5 => {}  // Bookmarks (TODO)
            // 6 = separator
            7 => {}  // Zoom In (TODO)
            8 => {}  // Zoom Out (TODO)
            9 => {}  // Reset Zoom (TODO)
            // 10 = separator
            11 => {} // Settings (TODO)
            12 => ui::toggle_devtools(),                              // Developer Tools
            // 13 = separator
            14 => {} // About Surf (TODO)
            _ => {}
        }
    });

    // URL field: navigate on Enter.
    st.url_field.on_submit(|e| {
        let st = state();
        let mut buf = [0u8; 2048];
        let len = ui_lib::Control::from_id(e.id).get_text(&mut buf);
        if len > 0 {
            if let Ok(url_str) = core::str::from_utf8(&buf[..len as usize]) {
                let url = String::from(url_str);
                st.tabs[st.active_tab].url_text = url.clone();
                tab::navigate(&url);
            }
        }
    });

    // Tab bar: switch tabs when the active segment changes.
    tab_bar_view.on_active_changed(|e| {
        ui::switch_tab(e.index as usize);
    });

    win.on_close(|_| { ui_lib::quit(); });

    // Keyboard shortcuts.
    win.on_key_down(|e| {
        let mods = e.modifiers;
        let key = e.keycode;
        let ctrl = mods & 2 != 0;
        let shift = mods & 1 != 0;

        if ctrl && key == b'T' as u32 {
            ui::add_tab();
        } else if ctrl && key == b'W' as u32 {
            let st = state();
            ui::close_tab(st.active_tab);
        } else if ctrl && key == b'L' as u32 {
            let st = state();
            st.url_field.focus();
        } else if ctrl && key == b'R' as u32 {
            tab::reload();
        } else if ctrl && shift && key == b'J' as u32 {
            // Ctrl+Shift+J — toggle DevTools console (Chrome shortcut).
            ui::toggle_devtools();
        } else if ctrl && shift && key == b'I' as u32 {
            // Ctrl+Shift+I — also toggle DevTools (Chrome/Firefox shortcut).
            ui::toggle_devtools();
        }
    });

    win.on_click(|_| {});

    // Viewport resize: re-layout the active tab's webview.
    win.on_resize(|_| {
        let st = state();
        let (w, h) = st.content_view.get_size();
        if w > 0 && h > 0 {
            let t = &mut st.tabs[st.active_tab];
            t.webview.resize(w, h);
        }
    });

    // Start the CSS animation tick timer.
    start_anim_timer();

    // Start the network worker poll timer (10 ms).
    start_net_poll_timer();

    // Navigate to the initial URL if one was provided on the command line.
    if let Some(url) = start_url {
        let st = state();
        st.tabs[st.active_tab].url_text = url.clone();
        st.url_field.set_text(&url);
        tab::navigate(&url);
    }

    // One-shot timer: after the first layout pass, resize the WebView to the
    // actual content_view dimensions (dock sizes aren't computed until run()).
    static mut RESIZE_TIMER: u32 = 0;
    unsafe {
        RESIZE_TIMER = ui_lib::set_timer(50, || {
            let st = state();
            let (w, h) = st.content_view.get_size();
            if w > 0 && h > 0 {
                let t = &mut st.tabs[st.active_tab];
                t.webview.resize(w, h);
            }
            // Kill after first fire — this is a one-shot timer.
            if unsafe { RESIZE_TIMER } != 0 {
                ui_lib::kill_timer(unsafe { RESIZE_TIMER });
                RESIZE_TIMER = 0;
            }
        });
    }

    anyos_std::println!("[surf] entering event loop");
    ui_lib::run();
    anyos_std::process::exit(0);
}
