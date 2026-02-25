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

anyos_std::entry!(main);

use alloc::string::String;
use alloc::vec::Vec;
use alloc::vec;

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
    btn_back: ui_lib::Button,
    btn_forward: ui_lib::Button,
    btn_reload: ui_lib::Button,
    url_field: ui_lib::TextField,
    /// DevTools toggle button (right of URL field).
    btn_devtools: ui_lib::Button,
    /// Floating popup menu that appears below the DevTools button.
    devtools_menu: ui_lib::View,
    tab_bar_view: ui_lib::TabBar,
    content_view: ui_lib::View,
    status_label: ui_lib::Label,
    /// DevTools console panel (DOCK_BOTTOM, hidden when closed).
    devtools_panel: ui_lib::View,
    /// Label inside the console panel showing JS console output.
    devtools_label: ui_lib::Label,
    /// Whether the DevTools console is currently visible.
    devtools_open: bool,
    /// Whether the DevTools popup menu is currently visible.
    devtools_menu_visible: bool,
    tabs: Vec<tab::TabState>,
    active_tab: usize,
    cookies: http::CookieJar,
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

/// Start the 16 ms CSS animation tick timer (60 fps).
///
/// Each tick calls `WebView::tick(16)` on the active tab.  When the JS
/// runtime has active animations the webview relayouts automatically.
pub(crate) fn start_anim_timer() {
    let st = state();
    if st.anim_timer != 0 { return; }
    st.anim_timer = ui_lib::set_timer(16, || {
        let st = state();
        if st.tabs[st.active_tab].webview.tick(16) {
            // Animation is active — relayout was already done inside tick().
        }
        // Forward timer tick to JS setTimeout/setInterval/requestAnimationFrame.
        // (tick() handles this internally via JsRuntime::tick)
    });
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

    if !libsvg_client::init() {
        anyos_std::println!("[surf] WARN: libsvg.so not available — SVG images disabled");
    }

    // Optional startup URL from the process argument string.
    let mut args_buf = [0u8; 256];
    let raw_args = anyos_std::process::args(&mut args_buf);
    let arg_url = raw_args.trim();
    let start_url = if arg_url.is_empty() { None } else { Some(String::from(arg_url)) };

    // ── Window ──────────────────────────────────────────────────────────────
    let win = ui_lib::Window::new("Surf", -1, -1, 900, 700);

    // ── Toolbar (DOCK_TOP, 40 px) ────────────────────────────────────────────
    let toolbar = ui_lib::View::new();
    toolbar.set_dock(ui_lib::DOCK_TOP);
    toolbar.set_size(0, 40);
    toolbar.set_color(0xFF2A2A2C);
    win.add(&toolbar);

    let btn_back = ui_lib::Button::new("<");
    btn_back.set_position(8, 6);
    btn_back.set_size(32, 28);
    toolbar.add(&btn_back);

    let btn_forward = ui_lib::Button::new(">");
    btn_forward.set_position(42, 6);
    btn_forward.set_size(32, 28);
    toolbar.add(&btn_forward);

    let btn_reload = ui_lib::Button::new("R");
    btn_reload.set_position(76, 6);
    btn_reload.set_size(32, 28);
    toolbar.add(&btn_reload);

    // URL field — shortened by 84 px to make room for the DevTools button.
    let url_field = ui_lib::TextField::new();
    url_field.set_position(116, 6);
    url_field.set_size(666, 28);
    url_field.set_placeholder("Enter URL...");
    toolbar.add(&url_field);

    // DevTools button — right of URL field.
    let btn_devtools = ui_lib::Button::new("DevTools \u{25BC}");   // ▼
    btn_devtools.set_position(786, 6);
    btn_devtools.set_size(106, 28);
    toolbar.add(&btn_devtools);

    // ── DevTools popup menu (appears below toolbar, overlaid on content) ──────
    // The menu View is added to the window with an absolute position; it is
    // normally hidden and popped into view when the DevTools button is clicked.
    let devtools_menu = ui_lib::View::new();
    devtools_menu.set_size(180, 80);
    devtools_menu.set_color(0xFF3A3A3C);
    devtools_menu.set_visible(false);
    win.add(&devtools_menu);

    let menu_item_console = ui_lib::Label::new("  Show Console");
    menu_item_console.set_position(0, 0);
    menu_item_console.set_size(180, 38);
    menu_item_console.set_color(0xFF3A3A3C);
    menu_item_console.set_text_color(0xFFE5E5EA);
    menu_item_console.set_font_size(14);
    devtools_menu.add(&menu_item_console);

    let menu_item_clear = ui_lib::Label::new("  Clear Console");
    menu_item_clear.set_position(0, 40);
    menu_item_clear.set_size(180, 38);
    menu_item_clear.set_color(0xFF3A3A3C);
    menu_item_clear.set_text_color(0xFFE5E5EA);
    menu_item_clear.set_font_size(14);
    devtools_menu.add(&menu_item_clear);

    // ── Tab bar (DOCK_TOP, 30 px) ────────────────────────────────────────────
    let tab_bar_view = ui_lib::TabBar::new("New Tab");
    tab_bar_view.set_dock(ui_lib::DOCK_TOP);
    tab_bar_view.set_size(0, 30);
    win.add(&tab_bar_view);

    // ── DevTools console panel (DOCK_BOTTOM, initially height=0/hidden) ───────
    let devtools_panel = ui_lib::View::new();
    devtools_panel.set_dock(ui_lib::DOCK_BOTTOM);
    devtools_panel.set_size(0, 0);
    devtools_panel.set_color(0xFF1C1C1E);
    win.add(&devtools_panel);

    let devtools_label = ui_lib::Label::new("");
    devtools_label.set_dock(ui_lib::DOCK_FILL);
    devtools_label.set_color(0xFF1C1C1E);
    devtools_label.set_text_color(0xFF30D158);   // green console text
    devtools_label.set_font_size(12);
    devtools_label.set_padding(8, 8, 8, 8);
    devtools_panel.add(&devtools_label);

    // ── Status bar (DOCK_BOTTOM, 24 px) ─────────────────────────────────────
    let status_label = ui_lib::Label::new("Ready");
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

    unsafe {
        STATE = Some(AppState {
            win,
            toolbar,
            btn_back,
            btn_forward,
            btn_reload,
            url_field,
            btn_devtools,
            devtools_menu,
            tab_bar_view,
            content_view,
            status_label,
            devtools_panel,
            devtools_label,
            devtools_open: false,
            devtools_menu_visible: false,
            tabs: vec![initial_tab],
            active_tab: 0,
            cookies: http::CookieJar { cookies: Vec::new() },
            image_queue: Vec::new(),
            image_timer: 0,
            conn_pool: http::ConnPool::new(),
            ws_connections: Vec::new(),
            ws_poll_timer: 0,
            anim_timer: 0,
        });
    }

    // ── Button callbacks ─────────────────────────────────────────────────────
    let st = state();
    st.btn_back.on_click(|_| { tab::go_back(); });
    st.btn_forward.on_click(|_| { tab::go_forward(); });
    st.btn_reload.on_click(|_| { tab::reload(); });

    // DevTools button: show/hide the popup menu.
    btn_devtools.on_click(|_| {
        let st = state();
        let menu_visible = !st.devtools_menu_visible;
        st.devtools_menu_visible = menu_visible;
        if menu_visible {
            // Position the menu just below the DevTools button.
            // The button is at toolbar x=786, toolbar height=40, tabbar=30 → y=70.
            st.devtools_menu.set_position(720, 70);
        }
        st.devtools_menu.set_visible(menu_visible);
    });

    // Menu items.
    menu_item_console.on_click(|_| {
        let st = state();
        st.devtools_menu_visible = false;
        st.devtools_menu.set_visible(false);
        ui::toggle_devtools();
    });
    menu_item_clear.on_click(|_| {
        let st = state();
        st.devtools_menu_visible = false;
        st.devtools_menu.set_visible(false);
        ui::clear_devtools();
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

    // Close popup menu when window is clicked anywhere else.
    win.on_click(|_| {
        let st = state();
        if st.devtools_menu_visible {
            st.devtools_menu_visible = false;
            st.devtools_menu.set_visible(false);
        }
    });

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

    // Navigate to the initial URL if one was provided on the command line.
    if let Some(url) = start_url {
        let st = state();
        st.tabs[st.active_tab].url_text = url.clone();
        st.url_field.set_text(&url);
        tab::navigate(&url);
    }

    anyos_std::println!("[surf] entering event loop");
    ui_lib::run();
}
