// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Surf — a tabbed web browser for anyOS.
//!
//! Renders HTML pages with CSS styling, fetched over HTTP/1.1.
//! Uses libanyui for the UI chrome (toolbar, tabs, status bar) and
//! libwebview for HTML content rendering via real UI controls.

#![no_std]
#![no_main]

use core::fmt;
use core::sync::atomic::{AtomicU32, Ordering};

static SURF_LOG_START_MS: AtomicU32 = AtomicU32::new(u32::MAX);

pub(crate) fn surf_log_times() -> (u32, u32) {
    let now = anyos_std::sys::uptime_ms();
    if SURF_LOG_START_MS.load(Ordering::Relaxed) == u32::MAX {
        let _ =
            SURF_LOG_START_MS.compare_exchange(u32::MAX, now, Ordering::Relaxed, Ordering::Relaxed);
    }
    let start = SURF_LOG_START_MS.load(Ordering::Relaxed);
    (now, now.wrapping_sub(start))
}

pub(crate) fn surf_log_print(args: fmt::Arguments<'_>) {
    let (now_ms, elapsed_ms) = surf_log_times();
    anyos_std::println!("[surf +{}ms @{}ms] {}", elapsed_ms, now_ms, args);
}

extern "C" fn deferred_kill_timer_cb(userdata: u64) {
    let timer_id = userdata as u32;
    if timer_id != 0 {
        ui_lib::kill_timer(timer_id);
    }
}

pub(crate) fn defer_kill_timer(timer_id: u32) {
    if timer_id != 0 {
        ui_lib::marshal_dispatch(deferred_kill_timer_cb, timer_id as u64);
    }
}

#[macro_export]
macro_rules! surf_log {
    ($($arg:tt)*) => {{
        $crate::surf_log_print(core::format_args!($($arg)*));
    }};
}

mod bookmarks;
mod callbacks;
mod config;
mod deflate {
    use alloc::vec::Vec;

    const MAX_DECOMPRESSED_BODY_BYTES: usize = 64 * 1024 * 1024;

    pub(crate) fn decompress_gzip(data: &[u8]) -> Option<Vec<u8>> {
        libzip::gzip::gzip_decompress_with_limit(data, MAX_DECOMPRESSED_BODY_BYTES).ok()
    }

    pub(crate) use libzip::inflate::inflate as decompress_deflate;
    pub(crate) use libzip::zlib::zlib_decompress as decompress_zlib;
}
mod devtools;
mod http;
mod js_worker;
mod net_worker;
mod resources;
mod settings;
mod tab;
mod tls;
mod ui;
mod ws;

anyos_std::entry!(main);

extern crate libfont_client;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use anyos_std::i18n;

use crate::tab::PageLoadPhase;
use libanyui_client as ui_lib;
use ui_lib::Widget;

#[derive(Clone, Copy)]
enum RenderSchedule {
    Immediate,
    Debounced,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RenderWork {
    None,
    Paint,
    Layout,
}

const IMAGE_RESULTS_PER_TAB_BATCH: usize = 16;
const MIN_IMAGE_RESULTS_PER_UI_POLL: usize = 4;
const IMAGE_BURST_MIN_GRACE_MS: u32 = 48;
const MAX_RESULTS_PER_UI_POLL: usize = 24;
const RESULT_PROCESS_BUDGET_MS: u32 = 10;
const SCROLL_INTERACTION_GRACE_MS: u32 = 160;
const MAX_DEFERRED_IMAGE_INFLIGHT: usize = 12;
const DEFERRED_IMAGE_BATCH_SIZE: usize = 6;
const VIEWPORT_DEFERRED_IMAGE_BATCH_SIZE: usize = 8;
const IMAGE_RESULT_BACKLOG_DEFER_THRESHOLD: usize = 8;
const MAX_DEFERRED_FONT_INFLIGHT: usize = 2;
const DEFERRED_FONT_BATCH_SIZE: usize = 2;
const MAX_BACKGROUND_RENDERS_PER_FLUSH: usize = 2;
const SCRIPT_PUMP_DELAY_MS: u32 = 16;
const VISUAL_IDLE_TICK_MS: u32 = 250;
const IDLE_TILE_PRERENDER_MIN_DELAY_MS: u32 = 32;
const JS_TIMER_ACTIVE_MIN_DELAY_MS: u32 = 16;
const JS_TIMER_IDLE_MIN_DELAY_MS: u32 = 250;
const JS_TIMER_QUIET_BACKOFF_AFTER: u16 = 8;
const JS_TIMER_QUIET_DEEP_BACKOFF_AFTER: u16 = 24;
const JS_TIMER_QUIET_BACKOFF_DELAY_MS: u32 = 1000;
const JS_TIMER_QUIET_DEEP_BACKOFF_DELAY_MS: u32 = 5000;
const JS_TIMER_CALLBACK_BUDGET: usize = 4;
const DEBUG_SKIP_BLOCKING_SLOT0: bool = false;
const RELAYOUT_FOLLOWUP_DELAY_MS: u32 = 16;
const IMAGE_REPAINT_BURST_DELAY_MS: u32 = 64;
const NET_POLL_INTERVAL_MS: u32 = 250;
const NET_STALE_WAIT_EMPTY_POLLS: u32 = 20;
const JS_WORKER_POLL_INTERVAL_MS: u32 = 50;
const DEBUG_SKIP_BLOCKING_SLOT2: bool = false;

fn debug_text_fingerprint(text: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for &b in text.as_bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn debug_text_prefix(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in text.chars().take(max_chars) {
        match ch {
            '\n' | '\r' | '\t' => out.push(' '),
            _ => out.push(ch),
        }
    }
    out
}

fn phase_name(phase: PageLoadPhase) -> &'static str {
    match phase {
        PageLoadPhase::Idle => "idle",
        PageLoadPhase::FetchingDocument => "fetching_document",
        PageLoadPhase::ParsingDocument => "parsing_document",
        PageLoadPhase::LoadingSubresources => "loading_subresources",
        PageLoadPhase::Interactive => "interactive",
        PageLoadPhase::Failed => "failed",
    }
}

fn log_tab_load_state(tab_index: usize, reason: &str) {
    let st = state();
    if tab_index >= st.tabs.len() {
        crate::surf_log!(
            "[surf] state: tab={} reason={} <out-of-range tabs={}>",
            tab_index,
            reason,
            st.tabs.len()
        );
        return;
    }
    let tab = &st.tabs[tab_index];
    crate::surf_log!(
        "[surf] state: tab={} reason={} phase={} loading={} pending_css={} pending_script={} pending_module={} deferred_fonts={} inflight_fonts={} deferred_images={} inflight_images={} ready_for_scripts={}",
        tab_index,
        reason,
        phase_name(tab.load_state.phase),
        tab.is_loading,
        tab.load_state.pending_stylesheet_count,
        tab.load_state.pending_script_count,
        tab.load_state.pending_module_count,
        tab.deferred_fonts.len(),
        tab.deferred_fonts_inflight,
        tab.deferred_images.len(),
        tab.deferred_images_inflight,
        tab.load_state.ready_for_script_execution()
    );
}

fn font_display_name(display: libwebview::css::FontDisplay) -> &'static str {
    match display {
        libwebview::css::FontDisplay::Auto => "auto",
        libwebview::css::FontDisplay::Block => "block",
        libwebview::css::FontDisplay::Swap => "swap",
        libwebview::css::FontDisplay::Fallback => "fallback",
        libwebview::css::FontDisplay::Optional => "optional",
    }
}

fn image_priority_name(priority: net_worker::ImagePriority) -> &'static str {
    match priority {
        net_worker::ImagePriority::Viewport => "viewport",
        net_worker::ImagePriority::Deferred => "deferred",
    }
}

fn script_mode_name(mode: libwebview::js::ScriptMode) -> &'static str {
    match mode {
        libwebview::js::ScriptMode::Blocking => "blocking",
        libwebview::js::ScriptMode::Defer => "defer",
        libwebview::js::ScriptMode::Async => "async",
        libwebview::js::ScriptMode::Module => "module",
    }
}

fn js_string_literal(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = core::fmt::Write::write_fmt(&mut out, format_args!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn module_import_wrapper(specifier: &str) -> String {
    anyos_std::format!("__import__({});", js_string_literal(specifier))
}

// ═══════════════════════════════════════════════════════════
// Debug helpers (feature-gated)
// ═══════════════════════════════════════════════════════════

/// Return current stack pointer for debug tracing.
#[cfg(feature = "debug_surf")]
#[inline(always)]
pub(crate) fn debug_rsp() -> u64 {
    let rsp: u64;
    unsafe {
        core::arch::asm!("mov {}, rsp", out(reg) rsp);
    }
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
    nav_group: ui_lib::View,
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
    /// DevTools window (Inspector / Console / Network / …).
    devtools: devtools::DevTools,
    tabs: Vec<tab::TabState>,
    active_tab: usize,
    cookies: http::CookieJar,
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
    /// Timer ID for cooperative script execution (0 = not running).
    script_pump_timer: u32,
    /// Timer ID for scheduling JS runtime timers onto the JS worker.
    js_runtime_timer: u32,
    /// Timer ID for draining JS worker results if marshal dispatch is delayed.
    js_worker_poll_timer: u32,
    /// Timer ID for delayed startup navigation (0 = not running).
    start_nav_timer: u32,
    /// Consecutive JS timer worker ticks that fired without visible/host work.
    js_timer_quiet_ticks: [u16; 16],
    /// Per-tab render work. Paint-only updates can reuse the cached layout tree,
    /// while layout updates require a full relayout before painting.
    render_dirty: [RenderWork; 16],
    /// True when the active tab scrolled and viewport tiles should be prepared
    /// from the animation timer instead of inside the scroll event callback.
    scroll_render_pending: bool,
    /// Last user scroll timestamp. Background work yields while this is hot.
    last_scroll_input_ms: u32,
    /// Last time visual-only animation work was allowed to run while idle.
    last_visual_tick_ms: u32,
    /// Last time Surf spent an idle frame pre-rendering offscreen tiles.
    last_idle_tile_prerender_ms: u32,
    /// Timer ID for the relayout debounce timer (0 = not running).
    relayout_timer: u32,
    /// Absolute uptime deadline for the pending relayout timer.
    relayout_due_ms: u32,
    /// Timer ID for debounced viewport resize handling (0 = not running).
    resize_timer: u32,
    /// A resize was requested while the active page was still in a heavy load phase.
    deferred_resize_pending: bool,
    /// Timer ID for theme refresh polling (0 = not running).
    theme_timer: u32,
    /// Last observed compositor theme state.
    last_theme_light: bool,
    /// User settings (homepage, etc.).
    config: config::SurfConfig,
    /// Bookmark store (hierarchical folders and bookmarks).
    bookmarks: config::BookmarkStore,
    /// Startup URL delayed until the first dock layout has produced a real
    /// content viewport.
    pending_start_url: Option<String>,
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
            let tab_conn_ids: Vec<u64> = st
                .ws_connections
                .iter()
                .filter(|c| c.tab_idx == tab_i)
                .map(|c| c.id)
                .collect();
            if tab_conn_ids.is_empty() {
                continue;
            }

            let runtime = st.tabs[tab_i].webview.js_runtime();
            let mut tab_conns: Vec<ws::WsConn> = Vec::new();
            let mut rest: Vec<ws::WsConn> = Vec::new();
            let all = core::mem::replace(&mut st.ws_connections, Vec::new());
            for c in all {
                if c.tab_idx == tab_i {
                    tab_conns.push(c);
                } else {
                    rest.push(c);
                }
            }
            let to_close = ws::poll_connections(&mut tab_conns, runtime);
            ws::remove_connections(&mut tab_conns, &to_close);
            for c in tab_conns {
                st.ws_connections.push(c);
            }
            for c in rest {
                st.ws_connections.push(c);
            }
        }

        if st.ws_connections.is_empty() {
            defer_kill_timer(st.ws_poll_timer);
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
    if st.anim_timer != 0 {
        return;
    }
    static mut IDLE_TICKS: u32 = 0;
    st.anim_timer = ui_lib::set_timer(16, || {
        let st = state();
        if st.scroll_render_pending {
            if scroll_interaction_hot() {
                let active_tab = st.active_tab;
                if active_tab < st.tabs.len() {
                    let scroll_y = st.tabs[active_tab].webview.scroll_view().get_state() as i32;
                    st.scroll_render_pending =
                        st.tabs[active_tab].webview.render_scroll_frame_at(scroll_y);
                }
                unsafe {
                    IDLE_TICKS = 0;
                }
                return;
            }
            st.scroll_render_pending = false;
            refresh_active_viewport_tiles();
            unsafe {
                IDLE_TICKS = 0;
            }
        }
        let net_results = drain_results_from_mailboxes();
        if !net_results.is_empty() {
            crate::surf_log!(
                "[surf] tick-drain: {} result(s) while anim timer active",
                net_results.len()
            );
            process_fetched_results(net_results);
        }
        if process_js_worker_results() {
            schedule_js_runtime_timer();
        }
        let active_tab = st.active_tab;
        let now_ms = anyos_std::sys::uptime_ms();
        let quiet_visual_idle = active_tab < st.tabs.len()
            && !st.tabs[active_tab].is_loading
            && !st.tabs[active_tab].js_worker_busy
            && active_tab < st.render_dirty.len()
            && st.render_dirty[active_tab] == RenderWork::None
            && !net_worker::has_pending_activity()
            && !net_worker::result_mailboxes_pending()
            && !scroll_interaction_hot();
        let mut changed = if active_tab >= st.tabs.len() {
            false
        } else if st.tabs[active_tab].js_worker_busy {
            if st.tabs[active_tab].webview.has_pending_tiles() && !scroll_interaction_hot() {
                let scroll_y = st.tabs[active_tab].webview.scroll_view().get_state() as i32;
                st.tabs[active_tab].webview.render_viewport_at(scroll_y)
            } else {
                false
            }
        } else if !st.tabs[active_tab].webview.has_visual_work() {
            false
        } else if quiet_visual_idle
            && st.last_visual_tick_ms != 0
            && now_ms.wrapping_sub(st.last_visual_tick_ms) < VISUAL_IDLE_TICK_MS
        {
            false
        } else {
            let delta_ms = if st.last_visual_tick_ms == 0 {
                16
            } else {
                now_ms.wrapping_sub(st.last_visual_tick_ms).max(1)
            };
            st.last_visual_tick_ms = now_ms;
            st.tabs[active_tab]
                .webview
                .tick_visual_only(delta_ms as u64)
        };
        if drain_js_navigation_for_tab(active_tab) {
            unsafe {
                IDLE_TICKS = 0;
            }
            return;
        }
        if !changed && quiet_visual_idle {
            let prerender_due = st.last_idle_tile_prerender_ms == 0
                || now_ms.wrapping_sub(st.last_idle_tile_prerender_ms)
                    >= IDLE_TILE_PRERENDER_MIN_DELAY_MS;
            if prerender_due {
                st.last_idle_tile_prerender_ms = now_ms;
                changed = st.tabs[active_tab].webview.prerender_idle_tiles();
            }
        }
        if changed {
            unsafe {
                IDLE_TICKS = 0;
            }
        } else {
            unsafe {
                IDLE_TICKS += 1;
            }
            // After ~300 ms of no work (20 ticks × 16ms), stop the timer.
            if unsafe { IDLE_TICKS } > 20 {
                unsafe {
                    IDLE_TICKS = 0;
                }
                if st.anim_timer != 0 {
                    defer_kill_timer(st.anim_timer);
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

pub(crate) fn mark_scroll_activity() {
    let st = state();
    st.scroll_render_pending = true;
    st.last_scroll_input_ms = anyos_std::sys::uptime_ms();
    st.last_idle_tile_prerender_ms = 0;
    ensure_anim_timer();
}

fn scroll_interaction_hot() -> bool {
    let st = state();
    st.last_scroll_input_ms != 0
        && anyos_std::sys::uptime_ms().wrapping_sub(st.last_scroll_input_ms)
            < SCROLL_INTERACTION_GRACE_MS
}

pub(crate) fn pump_deferred_images_for_tab(tab_index: usize) {
    if net_worker::result_mailbox_len_for_tab(tab_index) >= IMAGE_RESULT_BACKLOG_DEFER_THRESHOLD {
        return;
    }
    let allowance = {
        let st = state();
        if tab_index >= st.tabs.len() {
            return;
        }
        let tab = &st.tabs[tab_index];
        if !matches!(tab.load_state.phase, PageLoadPhase::Interactive) {
            return;
        }
        MAX_DEFERRED_IMAGE_INFLIGHT.saturating_sub(tab.deferred_images_inflight)
    };
    if allowance == 0 {
        return;
    }
    let batch = core::cmp::min(allowance, DEFERRED_IMAGE_BATCH_SIZE);
    let _ = resources::submit_deferred_images(tab_index, batch);
}

fn promote_viewport_deferred_images_for_tab(tab_index: usize) -> usize {
    if net_worker::result_mailbox_len_for_tab(tab_index) >= IMAGE_RESULT_BACKLOG_DEFER_THRESHOLD {
        return 0;
    }
    let allowance = {
        let st = state();
        if tab_index >= st.tabs.len() {
            return 0;
        }
        MAX_DEFERRED_IMAGE_INFLIGHT.saturating_sub(st.tabs[tab_index].deferred_images_inflight)
    };
    if allowance == 0 {
        return 0;
    }
    let batch = core::cmp::min(allowance, VIEWPORT_DEFERRED_IMAGE_BATCH_SIZE);
    resources::submit_viewport_deferred_images(tab_index, batch)
}

pub(crate) fn pump_deferred_fonts_for_tab(tab_index: usize) {
    let allowance = {
        let st = state();
        if tab_index >= st.tabs.len() {
            return;
        }
        let tab = &st.tabs[tab_index];
        if !matches!(tab.load_state.phase, PageLoadPhase::Interactive) {
            return;
        }
        MAX_DEFERRED_FONT_INFLIGHT.saturating_sub(tab.deferred_fonts_inflight)
    };
    if allowance == 0 {
        return;
    }
    let batch = core::cmp::min(allowance, DEFERRED_FONT_BATCH_SIZE);
    let _ = resources::submit_deferred_fonts(tab_index, batch);
}

pub(crate) fn pump_deferred_images_for_active_tab() {
    let tab_index = state().active_tab;
    pump_deferred_images_for_tab(tab_index);
}

pub(crate) fn refresh_active_viewport_tiles() {
    let st = state();
    let tab_index = st.active_tab;
    if tab_index >= st.tabs.len() {
        return;
    }
    if scroll_interaction_hot() {
        st.scroll_render_pending = true;
        ensure_anim_timer();
        return;
    }
    let scroll_y = st.tabs[tab_index].webview.scroll_view().get_state() as i32;
    // Grow the progressive layout budget *before* rendering so the freshly
    // revealed region is laid out and painted in the same pass.  This is the
    // mechanism that lets long pages (e.g. bild.de) finish loading past the
    // initial above-the-fold budget instead of stalling at partial height.
    if st.tabs[tab_index]
        .webview
        .deferred_layout_upgrade_needed(scroll_y)
    {
        st.tabs[tab_index].webview.upgrade_deferred_layout();
    }
    let pending = st.tabs[tab_index].webview.render_viewport_at(scroll_y);
    if pending {
        state().scroll_render_pending = true;
        ensure_anim_timer();
    }
}

fn apply_js_host_mutations(tab_index: usize) {
    let st = state();
    if tab_index >= st.tabs.len() {
        return;
    }
    let Some(url) = st.tabs[tab_index].current_url.clone() else {
        return;
    };
    let mutations = st.tabs[tab_index].webview.js_runtime().take_mutations();
    for mutation in mutations {
        match mutation {
            libwebview::js::DomMutation::SetCookie { value } => {
                st.cookies
                    .store_from_document_cookie(&value, &url.host, &url.path);
                let is_secure = url.scheme == "https";
                if let Some(cookie_hdr) = st.cookies.cookie_header(&url.host, &url.path, is_secure)
                {
                    st.tabs[tab_index]
                        .webview
                        .js_runtime()
                        .set_cookies(&cookie_hdr);
                }
            }
            libwebview::js::DomMutation::FormSubmit { form_node_id } => {
                if let Some((action, method, enctype)) = st.tabs[tab_index]
                    .webview
                    .form_action_for_node(form_node_id)
                {
                    let data = st.tabs[tab_index]
                        .webview
                        .collect_form_data_for_node(form_node_id);
                    crate::callbacks::submit_form_data(Some(&url), action, method, enctype, data);
                }
                return;
            }
            _ => {}
        }
    }
}

pub(crate) fn queue_iframe_snapshots_for_tab(tab_index: usize) -> usize {
    let base_url = {
        let st = state();
        if tab_index >= st.tabs.len() {
            return 0;
        }
        st.tabs[tab_index].current_url.clone()
    };
    let Some(base_url) = base_url else {
        return 0;
    };
    let queued = resources::queue_iframe_snapshots(&base_url, tab_index);
    if queued > 0 {
        crate::surf_log!(
            "[surf] queued iframe snapshots after DOM update: tab={} count={}",
            tab_index,
            queued
        );
    }
    queued
}

fn drain_js_navigation_for_tab(tab_index: usize) -> bool {
    let st = state();
    if tab_index >= st.tabs.len() {
        return false;
    }
    let Some(nav) = st.tabs[tab_index]
        .webview
        .take_pending_navigation_requests()
        .pop()
    else {
        return false;
    };
    let url = nav.url;
    let replace = nav.replace;
    let target = if let Some(ref base) = st.tabs[tab_index].current_url {
        let resolved = http::resolve_url(base, &url);
        ui::format_url(&resolved)
    } else {
        url
    };
    crate::surf_log!(
        "[surf-js] {} to {}",
        if replace { "replace" } else { "navigate" },
        target
    );
    if tab_index == st.active_tab {
        tab::navigate(&target);
        true
    } else {
        false
    }
}

fn mirror_new_js_console_lines(tab_index: usize) {
    const MAX_MIRRORED_CONSOLE_LINES_PER_DRAIN: usize = 24;
    let st = state();
    if tab_index >= st.tabs.len() {
        return;
    }
    let console_start = st.tabs[tab_index]
        .js_console_logged_len
        .min(st.tabs[tab_index].webview.js_console().len());
    let console = st.tabs[tab_index].webview.js_console();
    let new_lines = &console[console_start..];
    for line in new_lines.iter().take(MAX_MIRRORED_CONSOLE_LINES_PER_DRAIN) {
        crate::surf_log!("[surf-js] {}", line);
    }
    if new_lines.len() > MAX_MIRRORED_CONSOLE_LINES_PER_DRAIN {
        crate::surf_log!(
            "[surf-js] suppressed {} repeated console line(s)",
            new_lines.len() - MAX_MIRRORED_CONSOLE_LINES_PER_DRAIN
        );
    }
    st.tabs[tab_index].js_console_logged_len = console.len();
}

fn script_preview(script: &str) -> String {
    let mut preview = String::new();
    let mut truncated = false;
    for (idx, ch) in script.chars().enumerate() {
        if idx >= 96 {
            truncated = true;
            break;
        }
        let normalized = match ch {
            '\n' | '\r' | '\t' => ' ',
            _ => ch,
        };
        preview.push(normalized);
    }
    if truncated {
        preview.push_str("...");
    }
    preview
}

fn log_script_dump(slot: usize, label: &str, script_label: &str, script: &str) {
    if label != "blocking/defer" || slot != 2 {
        return;
    }
    crate::surf_log!(
        "[surf] slot-2 dump start: label={} source={} len={}",
        label,
        script_label,
        script.len()
    );
    let chars: Vec<char> = script.chars().collect();
    let mut start = 0usize;
    let mut chunk_idx = 0usize;
    while start < chars.len() && chunk_idx < 8 {
        let end = core::cmp::min(start + 200, chars.len());
        let mut chunk = String::new();
        for ch in &chars[start..end] {
            let normalized = match *ch {
                '\n' | '\r' | '\t' => ' ',
                _ => *ch,
            };
            chunk.push(normalized);
        }
        crate::surf_log!("[surf] slot-2 dump[{}]: {}", chunk_idx, chunk);
        start = end;
        chunk_idx += 1;
    }
    if start < chars.len() {
        crate::surf_log!(
            "[surf] slot-2 dump truncated: shown_chars={} total_chars={}",
            start,
            chars.len()
        );
    }
}

fn execute_script_slot(tab_index: usize, slot: usize, script: String, label: &str) {
    let script_label = {
        let st = state();
        if tab_index >= st.tabs.len() {
            return;
        }
        if !st.config.js_enabled {
            crate::surf_log!(
                "[surf] JS disabled in settings — skipping {} script [{}]",
                label,
                slot
            );
            return;
        }
        st.tabs[tab_index]
            .pending_script_labels
            .get(slot)
            .cloned()
            .unwrap_or_else(|| String::from("<unknown>"))
    };
    let preview = script_preview(&script);
    log_script_dump(slot, label, &script_label, &script);
    if DEBUG_SKIP_BLOCKING_SLOT0 && label == "blocking/defer" && slot == 0 {
        crate::surf_log!(
            "[surf] DEBUG skipping {} script [{}]: {} preview=\"{}\"",
            label,
            slot,
            script_label,
            preview
        );
        return;
    }
    if DEBUG_SKIP_BLOCKING_SLOT2 && label == "blocking/defer" && slot == 2 {
        crate::surf_log!(
            "[surf] DEBUG skipping {} script [{}]: {} preview=\"{}\"",
            label,
            slot,
            script_label,
            preview
        );
        return;
    }
    let generation;
    let js_state;
    {
        let st = state();
        if tab_index >= st.tabs.len() || st.tabs[tab_index].js_worker_busy {
            return;
        }
        generation = st.tabs[tab_index].load_state.generation;
        let Some(state) = st.tabs[tab_index].webview.take_js_execution_state() else {
            if slot < st.tabs[tab_index].pending_scripts.len()
                && st.tabs[tab_index].pending_scripts[slot].is_none()
            {
                st.tabs[tab_index].pending_scripts[slot] = Some(script);
            }
            schedule_script_pump_for_tab(tab_index);
            return;
        };
        st.tabs[tab_index].js_worker_busy = true;
        js_state = state;
    }
    crate::surf_log!(
        "[surf] scheduling {} script [{}] on JS worker: {} preview=\"{}\"",
        label,
        slot,
        script_label,
        preview
    );
    let request = js_worker::JsWorkerRequest {
        tab_index,
        job: js_worker::JsWorkerJob::Script {
            slot,
            label: String::from(label),
            script_label,
            script,
        },
        state: js_state,
        generation,
    };
    match js_worker::submit(request) {
        Ok(()) => ensure_js_worker_poll_timer(),
        Err(request) => {
            crate::surf_log!(
                "[surf] JS worker unavailable for {} script [{}]; running inline",
                label,
                slot
            );
            run_js_worker_request_inline(request);
        }
    }
}

fn finish_script_slot(result: js_worker::JsWorkerResult) {
    let (slot, label, script_label) = match result.kind {
        js_worker::JsWorkerResultKind::Script {
            slot,
            label,
            script_label,
        } => (slot, label, script_label),
        js_worker::JsWorkerResultKind::Timer { fired } => {
            finish_js_timer_job(
                result.tab_index,
                result.state,
                result.exec_ms,
                result.generation,
                fired,
            );
            return;
        }
    };
    let changed;
    {
        let st = state();
        if result.tab_index >= st.tabs.len() {
            return;
        }
        if !st.tabs[result.tab_index]
            .load_state
            .generation_matches(result.generation)
        {
            st.tabs[result.tab_index].js_worker_busy = false;
            crate::surf_log!(
                "[surf] discarding stale {} script [{}]: {}",
                label,
                slot,
                script_label
            );
            schedule_script_pump_for_tab(result.tab_index);
            return;
        }
        changed = st.tabs[result.tab_index]
            .webview
            .finish_js_execution_state(result.state);
        st.tabs[result.tab_index].js_worker_busy = false;
    }
    crate::surf_log!(
        "[surf] finished {} script [{}]: {} exec_ms={}",
        label,
        slot,
        script_label,
        result.exec_ms
    );
    mirror_new_js_console_lines(result.tab_index);
    apply_js_host_mutations(result.tab_index);
    connect_pending_ws(result.tab_index);
    if drain_js_navigation_for_tab(result.tab_index) {
        return;
    }
    if changed {
        let base_url = {
            let st = state();
            st.tabs
                .get(result.tab_index)
                .and_then(|tab| tab.current_url.clone())
        };
        if let Some(base_url) = base_url {
            let rasterized_svg = {
                let st = state();
                result.tab_index < st.tabs.len()
                    && resources::queue_inline_svgs(
                        &st.tabs[result.tab_index].webview,
                        &base_url,
                        result.tab_index,
                    )
            };
            if rasterized_svg {
                request_layout_refresh(result.tab_index);
            }
            {
                let st = state();
                if result.tab_index < st.tabs.len() {
                    if let Some(dom) = st.tabs[result.tab_index].webview.dom() {
                        resources::queue_images(dom, &base_url, result.tab_index, false);
                    }
                }
            }
            queue_iframe_snapshots_for_tab(result.tab_index);
            pump_deferred_images_for_tab(result.tab_index);
        }
        ensure_anim_timer();
    }
    finish_blocking_scripts_for_tab(result.tab_index);
    if any_script_work_pending() {
        schedule_script_pump();
    }
    {
        let st = state();
        if result.tab_index < st.tabs.len()
            && st.tabs[result.tab_index].webview.has_pending_js_work()
        {
            schedule_js_runtime_timer();
        }
    }
}

fn finish_js_timer_job(
    tab_index: usize,
    state_value: libwebview::JsExecutionState,
    exec_ms: u32,
    generation: u32,
    fired: usize,
) {
    let changed;
    {
        let st = state();
        if tab_index >= st.tabs.len() {
            return;
        }
        if !st.tabs[tab_index].load_state.generation_matches(generation) {
            st.tabs[tab_index].js_worker_busy = false;
            return;
        }
        changed = st.tabs[tab_index]
            .webview
            .finish_js_execution_state(state_value);
        st.tabs[tab_index].js_worker_busy = false;
    }
    if exec_ms >= 8 {
        crate::surf_log!(
            "[surf-perf] finished JS timer job: tab={} fired={} exec_ms={}",
            tab_index,
            fired,
            exec_ms
        );
    }
    mirror_new_js_console_lines(tab_index);
    apply_js_host_mutations(tab_index);
    connect_pending_ws(tab_index);
    if drain_js_navigation_for_tab(tab_index) {
        return;
    }
    {
        let st = state();
        if tab_index < st.js_timer_quiet_ticks.len() {
            if changed || fired == 0 {
                st.js_timer_quiet_ticks[tab_index] = 0;
            } else {
                st.js_timer_quiet_ticks[tab_index] =
                    st.js_timer_quiet_ticks[tab_index].saturating_add(1);
            }
        }
    }
    if changed {
        let base_url = {
            let st = state();
            st.tabs
                .get(tab_index)
                .and_then(|tab| tab.current_url.clone())
        };
        if let Some(base_url) = base_url {
            let rasterized_svg = {
                let st = state();
                tab_index < st.tabs.len()
                    && resources::queue_inline_svgs(
                        &st.tabs[tab_index].webview,
                        &base_url,
                        tab_index,
                    )
            };
            if rasterized_svg {
                request_layout_refresh(tab_index);
            }
            {
                let st = state();
                if tab_index < st.tabs.len() {
                    if let Some(dom) = st.tabs[tab_index].webview.dom() {
                        resources::queue_images(dom, &base_url, tab_index, false);
                    }
                }
            }
            queue_iframe_snapshots_for_tab(tab_index);
            pump_deferred_images_for_tab(tab_index);
        }
        ensure_anim_timer();
    }
    schedule_js_runtime_timer();
}

fn next_js_timer_tab() -> Option<(usize, u32)> {
    let st = state();
    let mut best: Option<(usize, u32)> = None;
    for (tab_index, tab) in st.tabs.iter().enumerate() {
        if tab.js_worker_busy {
            continue;
        }
        let Some(delay_ms) = tab.webview.next_js_task_delay_ms() else {
            continue;
        };
        let delay = delay_ms.min(u32::MAX as u64) as u32;
        match best {
            Some((_, best_delay)) if best_delay <= delay => {}
            _ => best = Some((tab_index, delay)),
        }
    }
    best
}

fn submit_js_timer_tick(tab_index: usize, elapsed_ms: u32) -> bool {
    let generation;
    let js_state;
    {
        let st = state();
        if tab_index >= st.tabs.len() || st.tabs[tab_index].js_worker_busy {
            return false;
        }
        generation = st.tabs[tab_index].load_state.generation;
        let Some(state_value) = st.tabs[tab_index].webview.take_js_execution_state() else {
            return false;
        };
        st.tabs[tab_index].js_worker_busy = true;
        js_state = state_value;
    }
    let request = js_worker::JsWorkerRequest {
        tab_index,
        job: js_worker::JsWorkerJob::Timer {
            delta_ms: elapsed_ms.max(1) as u64,
            callback_budget: JS_TIMER_CALLBACK_BUDGET,
        },
        state: js_state,
        generation,
    };
    match js_worker::submit(request) {
        Ok(()) => ensure_js_worker_poll_timer(),
        Err(request) => {
            crate::surf_log!("[surf] JS worker unavailable for timer job; running inline");
            run_js_worker_request_inline(request);
        }
    }
    true
}

fn schedule_js_runtime_timer() {
    let st = state();
    if st.js_runtime_timer != 0 {
        return;
    }
    let Some((tab_index, delay_ms)) = next_js_timer_tab() else {
        return;
    };
    let idle_tab = tab_index < st.tabs.len()
        && !st.tabs[tab_index].is_loading
        && !st.tabs[tab_index].js_worker_busy
        && tab_index < st.render_dirty.len()
        && st.render_dirty[tab_index] == RenderWork::None
        && !net_worker::has_pending_activity()
        && !net_worker::result_mailboxes_pending();
    let min_delay = if idle_tab {
        let quiet_ticks = if tab_index < st.js_timer_quiet_ticks.len() {
            st.js_timer_quiet_ticks[tab_index]
        } else {
            0
        };
        if quiet_ticks >= JS_TIMER_QUIET_DEEP_BACKOFF_AFTER {
            JS_TIMER_QUIET_DEEP_BACKOFF_DELAY_MS
        } else if quiet_ticks >= JS_TIMER_QUIET_BACKOFF_AFTER {
            JS_TIMER_QUIET_BACKOFF_DELAY_MS
        } else {
            JS_TIMER_IDLE_MIN_DELAY_MS
        }
    } else {
        JS_TIMER_ACTIVE_MIN_DELAY_MS
    };
    let delay_ms = delay_ms.max(min_delay);
    st.js_runtime_timer = ui_lib::set_timer(delay_ms, move || {
        {
            let st = state();
            let timer_id = st.js_runtime_timer;
            st.js_runtime_timer = 0;
            defer_kill_timer(timer_id);
        }
        let Some((tab_index, due_ms)) = next_js_timer_tab() else {
            return;
        };
        if !submit_js_timer_tick(tab_index, due_ms.max(delay_ms)) {
            schedule_js_runtime_timer();
        }
    });
}

pub(crate) fn handle_js_worker_results_ready() {
    let had_results = process_js_worker_results();
    if had_results {
        schedule_js_runtime_timer();
    }
    ensure_js_worker_poll_timer();
}

fn process_js_worker_results() -> bool {
    let results = js_worker::drain_results();
    if results.is_empty() {
        return false;
    }
    crate::surf_log!("[surf] drained {} JS worker result(s)", results.len());
    for result in results {
        finish_script_slot(result);
    }
    true
}

fn js_worker_work_pending() -> bool {
    let st = state();
    st.tabs.iter().any(|tab| tab.js_worker_busy) || js_worker::has_pending_activity()
}

fn start_js_worker_poll_timer() {
    let st = state();
    if st.js_worker_poll_timer != 0 {
        return;
    }
    st.js_worker_poll_timer = ui_lib::set_timer(JS_WORKER_POLL_INTERVAL_MS, || {
        let had_results = process_js_worker_results();
        if had_results {
            schedule_js_runtime_timer();
        }
        if js_worker_work_pending() {
            return;
        }
        let st = state();
        if st.js_worker_poll_timer != 0 {
            defer_kill_timer(st.js_worker_poll_timer);
            st.js_worker_poll_timer = 0;
        }
    });
}

fn ensure_js_worker_poll_timer() {
    if js_worker_work_pending() {
        start_js_worker_poll_timer();
    }
}

fn run_js_worker_request_inline(mut req: js_worker::JsWorkerRequest) {
    let start_ms = anyos_std::sys::uptime_ms();
    let kind = match req.job {
        js_worker::JsWorkerJob::Script {
            slot,
            label,
            script_label,
            script,
        } => {
            req.state.execute_script_source(script);
            js_worker::JsWorkerResultKind::Script {
                slot,
                label,
                script_label,
            }
        }
        js_worker::JsWorkerJob::Timer {
            delta_ms,
            callback_budget,
        } => {
            let fired = req.state.run_timers_with_budget(delta_ms, callback_budget);
            js_worker::JsWorkerResultKind::Timer { fired }
        }
    };
    let exec_ms = anyos_std::sys::uptime_ms().wrapping_sub(start_ms);
    finish_script_slot(js_worker::JsWorkerResult {
        tab_index: req.tab_index,
        kind,
        state: req.state,
        exec_ms,
        generation: req.generation,
    });
}

fn execute_buffered_async_scripts(tab_index: usize) {
    schedule_script_pump_for_tab(tab_index);
}

fn log_main_phase_elapsed(label: &str, start_ms: u32) {
    let elapsed_ms = anyos_std::sys::uptime_ms().wrapping_sub(start_ms);
    if elapsed_ms >= 8 {
        crate::surf_log!("[surf-perf] {} ui_ms={}", label, elapsed_ms);
    }
}

fn tab_has_pending_script_kind(tab_index: usize, async_scripts: bool) -> bool {
    let st = state();
    if tab_index >= st.tabs.len() {
        return false;
    }
    for slot in 0..st.tabs[tab_index].pending_scripts.len() {
        if st.tabs[tab_index].pending_scripts[slot].is_none() {
            continue;
        }
        let is_async = matches!(
            st.tabs[tab_index].pending_script_modes.get(slot),
            Some(libwebview::js::ScriptMode::Async)
        );
        if is_async == async_scripts {
            return true;
        }
    }
    false
}

fn take_next_pending_script(tab_index: usize, async_scripts: bool) -> Option<(usize, String)> {
    let st = state();
    if tab_index >= st.tabs.len() {
        return None;
    }
    for slot in 0..st.tabs[tab_index].pending_scripts.len() {
        let is_async = matches!(
            st.tabs[tab_index].pending_script_modes.get(slot),
            Some(libwebview::js::ScriptMode::Async)
        );
        if is_async != async_scripts {
            continue;
        }
        if let Some(text) = st.tabs[tab_index].pending_scripts[slot].take() {
            return Some((slot, text));
        }
    }
    None
}

fn script_work_pending_for_tab(tab_index: usize) -> bool {
    let st = state();
    if tab_index >= st.tabs.len() {
        return false;
    }
    if st.tabs[tab_index].js_worker_busy {
        return false;
    }
    if st.tabs[tab_index].load_state.ready_for_script_execution()
        && tab_has_pending_script_kind(tab_index, false)
    {
        return true;
    }
    matches!(
        st.tabs[tab_index].load_state.phase,
        PageLoadPhase::Interactive
    ) && tab_has_pending_script_kind(tab_index, true)
}

fn any_script_work_pending() -> bool {
    let st = state();
    for tab_index in 0..st.tabs.len() {
        if script_work_pending_for_tab(tab_index) {
            return true;
        }
    }
    false
}

fn schedule_script_pump_for_tab(tab_index: usize) {
    let st = state();
    if tab_index >= st.tabs.len() || st.script_pump_timer != 0 {
        return;
    }
    if !script_work_pending_for_tab(tab_index) {
        return;
    }
    st.script_pump_timer = ui_lib::set_timer(SCRIPT_PUMP_DELAY_MS, pump_script_tick);
}

fn schedule_script_pump() {
    let st = state();
    if st.script_pump_timer != 0 || !any_script_work_pending() {
        return;
    }
    st.script_pump_timer = ui_lib::set_timer(SCRIPT_PUMP_DELAY_MS, pump_script_tick);
}

fn next_script_tab() -> Option<usize> {
    let st = state();
    if st.active_tab < st.tabs.len() && script_work_pending_for_tab(st.active_tab) {
        return Some(st.active_tab);
    }
    for tab_index in 0..st.tabs.len() {
        if script_work_pending_for_tab(tab_index) {
            return Some(tab_index);
        }
    }
    None
}

fn finish_blocking_scripts_for_tab(tab_index: usize) {
    let st = state();
    if tab_index >= st.tabs.len()
        || st.tabs[tab_index].js_worker_busy
        || tab_has_pending_script_kind(tab_index, false)
    {
        return;
    }
    if !matches!(
        st.tabs[tab_index].load_state.phase,
        PageLoadPhase::Interactive
    ) {
        st.tabs[tab_index].load_state.mark_interactive();
        log_tab_load_state(tab_index, "mark_interactive_after_blocking_scripts");
        pump_deferred_fonts_for_tab(tab_index);
        pump_deferred_images_for_tab(tab_index);
        flush_deferred_resize_if_ready();
    }
    schedule_script_pump_for_tab(tab_index);
}

fn pump_script_tick() {
    {
        let st = state();
        let timer_id = st.script_pump_timer;
        st.script_pump_timer = 0;
        defer_kill_timer(timer_id);
    }

    if scroll_interaction_hot() {
        state().scroll_render_pending = true;
        ensure_anim_timer();
        schedule_script_pump();
        return;
    }

    if net_worker::result_mailboxes_pending() {
        let results = drain_results_from_mailboxes();
        if !results.is_empty() {
            crate::surf_log!(
                "[surf] script-pump yielding to {} queued network result(s)",
                results.len()
            );
            process_fetched_results(results);
        }
        if any_script_work_pending() {
            schedule_script_pump();
        }
        return;
    }

    if js_worker::has_pending_activity() {
        return;
    }

    let Some(tab_index) = next_script_tab() else {
        return;
    };

    let ran = {
        let st = state();
        if tab_index >= st.tabs.len() {
            false
        } else if st.tabs[tab_index].load_state.ready_for_script_execution()
            && tab_has_pending_script_kind(tab_index, false)
        {
            flush_pending_render_before_scripts(tab_index);
            if let Some((slot, script)) = take_next_pending_script(tab_index, false) {
                execute_script_slot(tab_index, slot, script, "blocking/defer");
                true
            } else {
                false
            }
        } else if matches!(
            st.tabs[tab_index].load_state.phase,
            PageLoadPhase::Interactive
        ) && tab_has_pending_script_kind(tab_index, true)
        {
            if let Some((slot, script)) = take_next_pending_script(tab_index, true) {
                crate::surf_log!("[surf] async script [{}] released after script gate", slot);
                execute_script_slot(tab_index, slot, script, "async");
                true
            } else {
                false
            }
        } else {
            false
        }
    };

    finish_blocking_scripts_for_tab(tab_index);

    if ran && any_script_work_pending() {
        schedule_script_pump();
    }
}

/// Resize the active tab's webview to match the content area.
fn resize_active_webview_now() {
    let st = state();
    let (w, h) = st.content_view.get_size();
    if w <= 0 || h <= 0 || st.active_tab >= st.tabs.len() {
        return;
    }
    st.tabs[st.active_tab].webview.resize(w, h);
}

fn active_tab_has_viewport_size_mismatch() -> bool {
    let st = state();
    if st.active_tab >= st.tabs.len() {
        return false;
    }
    let (w, h) = st.content_view.get_size();
    if w <= 0 || h <= 0 {
        return false;
    }
    let webview = &st.tabs[st.active_tab].webview;
    webview.viewport_width() != w || webview.viewport_height() != h
}

fn active_tab_allows_expensive_resize() -> bool {
    let st = state();
    if st.active_tab >= st.tabs.len() {
        return false;
    }
    let tab = &st.tabs[st.active_tab];
    !tab.is_loading && tab.load_state.phase != PageLoadPhase::LoadingSubresources
}

fn flush_deferred_resize_if_ready() {
    let should_apply = {
        let st = state();
        st.deferred_resize_pending && active_tab_allows_expensive_resize()
    };
    if !should_apply {
        return;
    }
    let st = state();
    st.deferred_resize_pending = false;
    crate::surf_log!("[surf] applying deferred resize after load phase");
    resize_active_webview_now();
    ensure_anim_timer();
}

/// Debounce expensive webview resize work during window drags.
fn schedule_active_webview_resize(delay_ms: u32) {
    let st = state();
    if st.resize_timer != 0 {
        ui_lib::kill_timer(st.resize_timer);
        st.resize_timer = 0;
    }
    st.resize_timer = ui_lib::set_timer(delay_ms, || {
        let st = state();
        let timer_id = st.resize_timer;
        st.resize_timer = 0;
        defer_kill_timer(timer_id);
        if !active_tab_allows_expensive_resize() && !active_tab_has_viewport_size_mismatch() {
            crate::surf_log!("[surf] deferring resize until active load settles");
            let st = state();
            st.deferred_resize_pending = true;
            schedule_active_webview_resize(250);
            return;
        }
        resize_active_webview_now();
        ensure_anim_timer();
    });
}

fn run_pending_start_navigation() {
    {
        let st = state();
        let timer_id = st.start_nav_timer;
        st.start_nav_timer = 0;
        defer_kill_timer(timer_id);
    }
    resize_active_webview_now();

    let start_url = {
        let st = state();
        st.pending_start_url.take()
    };
    if let Some(url) = start_url {
        let st = state();
        st.tabs[st.active_tab].url_text = url.clone();
        st.url_field.set_text(&url);
        crate::surf_log!("[surf] startup navigation after initial viewport resize");
        tab::navigate(&url);
    }
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
    if st.net_poll_timer != 0 {
        return;
    }
    static mut EMPTY_POLLS: u32 = 0;
    crate::surf_log!(
        "[surf] net-poll timer start: worker_pending={} tabs_waiting={}",
        net_worker::has_pending_activity(),
        st.tabs.iter().any(tab_waiting_on_network)
    );
    st.net_poll_timer = ui_lib::set_timer(NET_POLL_INTERVAL_MS, || {
        static mut STALE_WAIT_POLLS: u32 = 0;
        #[cfg(feature = "debug_surf")]
        let mailbox_counts = net_worker::mailbox_pending_counts();
        #[cfg(feature = "debug_surf")]
        crate::surf_log!(
            "[surf] net-poll tick: timer={} worker_pending={} mailbox_counts={:?}",
            state().net_poll_timer,
            net_worker::has_pending_activity(),
            mailbox_counts
        );
        if scroll_interaction_hot() {
            state().scroll_render_pending = true;
            ensure_anim_timer();
            return;
        }
        let results = drain_results_from_mailboxes();
        if results.is_empty() {
            let st = state();
            let tab_waiting_on_network = st.tabs.iter().any(tab_waiting_on_network);
            let worker_pending = net_worker::has_pending_activity();
            #[cfg(feature = "debug_surf")]
            crate::surf_log!(
                "[surf] net-poll empty: waiting_on_network={} worker_pending={} timer={}",
                tab_waiting_on_network,
                worker_pending,
                st.net_poll_timer
            );
            if worker_pending {
                unsafe {
                    EMPTY_POLLS = 0;
                    STALE_WAIT_POLLS = 0;
                }
                return;
            }
            if tab_waiting_on_network {
                unsafe {
                    STALE_WAIT_POLLS += 1;
                }
                if unsafe { STALE_WAIT_POLLS } <= NET_STALE_WAIT_EMPTY_POLLS {
                    unsafe {
                        EMPTY_POLLS = 0;
                    }
                    return;
                }
                crate::surf_log!(
                    "[surf] net-poll stale wait: no worker activity for {} empty poll(s), allowing timer to idle",
                    unsafe { STALE_WAIT_POLLS }
                );
            }

            unsafe {
                EMPTY_POLLS += 1;
            }
            if unsafe { EMPTY_POLLS } > 60 {
                unsafe {
                    EMPTY_POLLS = 0;
                    STALE_WAIT_POLLS = 0;
                }
                let st = state();
                if st.net_poll_timer != 0 {
                    crate::surf_log!("[surf] net-poll timer stop after idle");
                    defer_kill_timer(st.net_poll_timer);
                    st.net_poll_timer = 0;
                }
            }
            return;
        }
        unsafe {
            EMPTY_POLLS = 0;
            STALE_WAIT_POLLS = 0;
        }
        process_fetched_results(results);
    });
}

fn tab_waiting_on_network(tab: &tab::TabState) -> bool {
    tab.is_loading || !tab.load_state.ready_for_script_execution()
}

fn network_work_pending() -> bool {
    let st = state();
    st.tabs.iter().any(tab_waiting_on_network) || net_worker::has_pending_activity()
}

fn drain_results_from_mailboxes() -> Vec<net_worker::FetchResult> {
    let st = state();
    let mut all_results = Vec::new();
    for tab_index in 0..st.tabs.len() {
        let mut tab_results = net_worker::drain_results_for_tab(tab_index);
        if !tab_results.is_empty() {
            crate::surf_log!(
                "[surf] drained {} result(s) from tab mailbox {}",
                tab_results.len(),
                tab_index
            );
            all_results.append(&mut tab_results);
        }
    }
    all_results
}

/// Ensure the network poll timer is running. Called when new fetches are submitted.
pub(crate) fn ensure_net_poll_timer() {
    if !network_work_pending() {
        return;
    }
    start_net_poll_timer();
}

pub(crate) fn handle_net_worker_results_ready() {
    let results = drain_results_from_mailboxes();
    if !results.is_empty() {
        process_fetched_results(results);
    }
}

/// Dispatch completed fetch results to their handlers.
///
/// Image and font results usually set per-tab dirty flags instead of triggering
/// immediate relayouts. CSS is applied more strictly: Surf now waits for the
/// stylesheet chain to finish and flushes layout before running blocking scripts.
fn process_fetched_results(results: Vec<net_worker::FetchResult>) {
    let mut batches: Vec<(usize, Vec<net_worker::FetchResult>)> = Vec::new();
    let mut nav_count = 0usize;
    let mut css_count = 0usize;
    let mut image_count = 0usize;
    let mut iframe_count = 0usize;
    let mut font_count = 0usize;
    let mut script_count = 0usize;
    let mut module_count = 0usize;
    for result in results {
        match result {
            net_worker::FetchResult::NavDone { .. } | net_worker::FetchResult::NavError { .. } => {
                nav_count += 1;
            }
            net_worker::FetchResult::CssDone { .. } => css_count += 1,
            net_worker::FetchResult::ImageDone { .. } => image_count += 1,
            net_worker::FetchResult::IframeSnapshotDone { .. } => iframe_count += 1,
            net_worker::FetchResult::FontDone { .. } => font_count += 1,
            net_worker::FetchResult::ScriptDone { .. } => script_count += 1,
            net_worker::FetchResult::ModuleScriptDone { .. } => module_count += 1,
        }
        let tab_index = result.tab_index();
        if let Some((_, batch)) = batches.iter_mut().find(|(idx, _)| *idx == tab_index) {
            batch.push(result);
        } else {
            batches.push((tab_index, vec![result]));
        }
    }
    crate::surf_log!(
        "[surf] drain_results: total={} nav={} css={} image={} iframe={} font={} script={} module={}",
        nav_count + css_count + image_count + iframe_count + font_count + script_count + module_count,
        nav_count,
        css_count,
        image_count,
        iframe_count,
        font_count,
        script_count,
        module_count
    );

    for (tab_index, batch) in batches {
        let mut nav_results = Vec::new();
        let mut css_results = Vec::new();
        let mut script_results = Vec::new();
        let mut font_results = Vec::new();
        let mut images = Vec::new();
        for result in batch {
            match result {
                net_worker::FetchResult::NavDone { .. }
                | net_worker::FetchResult::NavError { .. } => nav_results.push(result),
                net_worker::FetchResult::CssDone { .. } => css_results.push(result),
                net_worker::FetchResult::ScriptDone { .. }
                | net_worker::FetchResult::ModuleScriptDone { .. } => script_results.push(result),
                net_worker::FetchResult::FontDone { .. } => font_results.push(result),
                net_worker::FetchResult::ImageDone { .. }
                | net_worker::FetchResult::IframeSnapshotDone { .. } => images.push(result),
            }
        }
        crate::surf_log!(
            "[surf] tab {} result batch: nav={} css={} script={} font={} image_or_iframe={}",
            tab_index,
            nav_results.len(),
            css_results.len(),
            script_results.len(),
            font_results.len(),
            images.len()
        );
        images.sort_by_key(|result| match result {
            net_worker::FetchResult::ImageDone { priority, .. } => match priority {
                net_worker::ImagePriority::Viewport => 0usize,
                net_worker::ImagePriority::Deferred => 1usize,
            },
            net_worker::FetchResult::IframeSnapshotDone { .. } => 0usize,
            _ => 0usize,
        });

        let deferred_images = if images.len() > IMAGE_RESULTS_PER_TAB_BATCH {
            images.split_off(IMAGE_RESULTS_PER_TAB_BATCH)
        } else {
            Vec::new()
        };

        if !deferred_images.is_empty() {
            crate::surf_log!(
                "[surf] deferring {} image result(s) for tab {} to keep UI responsive",
                deferred_images.len(),
                tab_index
            );
            net_worker::prepend_results_for_tab(tab_index, deferred_images);
        }

        let mut urgent = nav_results;
        urgent.extend(css_results);
        urgent.extend(font_results);
        urgent.extend(images);
        // Script execution can block the UI thread for a long time on heavy
        // pages. Apply visual resources first so their DevTools timing and
        // rendering are not stuck behind synchronous JS work.
        urgent.extend(script_results);

        let batch_start_ms = anyos_std::sys::uptime_ms();
        let mut processed = 0usize;
        let image_only_batch = urgent.iter().all(|result| {
            matches!(
                result,
                net_worker::FetchResult::ImageDone { .. }
                    | net_worker::FetchResult::IframeSnapshotDone { .. }
            )
        });
        while !urgent.is_empty() {
            let result = urgent.remove(0);
            process_single_fetch_result(result);
            processed += 1;
            let elapsed = anyos_std::sys::uptime_ms().wrapping_sub(batch_start_ms);
            let budget_exhausted =
                processed >= MAX_RESULTS_PER_UI_POLL || elapsed >= RESULT_PROCESS_BUDGET_MS;
            let keep_image_burst_moving = image_only_batch
                && processed < MIN_IMAGE_RESULTS_PER_UI_POLL
                && elapsed < IMAGE_BURST_MIN_GRACE_MS;
            if budget_exhausted && !keep_image_burst_moving {
                if !urgent.is_empty() {
                    crate::surf_log!(
                        "[surf] requeue {} result(s) for tab {} after ui budget processed={} elapsed={}ms",
                        urgent.len(),
                        tab_index,
                        processed,
                        elapsed
                    );
                    net_worker::prepend_results_for_tab(tab_index, urgent);
                    ensure_net_poll_timer();
                }
                break;
            }
        }
    }
}

fn record_resource_completion(
    url: &http::Url,
    size: u64,
    timing: Option<http::RequestTiming>,
    from_cache: bool,
) {
    if let Some(timing) = timing {
        if from_cache {
            devtools::record_request_cached_with_timing(&url.host, &url.path, size, timing);
        } else {
            devtools::record_request_done_with_timing(&url.host, &url.path, 200, size, timing);
        }
    } else {
        devtools::record_request_done(&url.host, &url.path, 200, size);
    }
}

fn process_single_fetch_result(result: net_worker::FetchResult) {
    match result {
        net_worker::FetchResult::NavDone {
            tab_index,
            response,
            url,
            cookies,
            generation,
        } => {
            devtools::record_request_done_with_timing(
                &url.host,
                &url.path,
                response.status as u32,
                response.body.len() as u64,
                response.timing,
            );
            let start_ms = anyos_std::sys::uptime_ms();
            handle_nav_done(tab_index, response, url, cookies, generation);
            log_main_phase_elapsed("handle_nav_done", start_ms);
        }
        net_worker::FetchResult::NavError {
            tab_index,
            error_msg,
            generation,
        } => {
            devtools::record_request_done_by_kind("html", 0, 0);
            let start_ms = anyos_std::sys::uptime_ms();
            handle_nav_error(tab_index, error_msg, generation);
            log_main_phase_elapsed("handle_nav_error", start_ms);
        }
        net_worker::FetchResult::CssDone {
            tab_index,
            href,
            url,
            body,
            headers,
            parsed,
            timing,
            from_cache,
            generation,
        } => {
            record_resource_completion(&url, body.len() as u64, timing, from_cache);
            let start_ms = anyos_std::sys::uptime_ms();
            handle_css_done(tab_index, href, body, headers, parsed, generation);
            log_main_phase_elapsed("handle_css_done", start_ms);
        }
        net_worker::FetchResult::ImageDone {
            tab_index,
            src,
            url,
            body,
            encoded_len,
            headers,
            decoded_raster,
            priority,
            from_deferred,
            timing,
            from_cache,
            generation,
        } => {
            record_resource_completion(&url, encoded_len as u64, timing, from_cache);
            let start_ms = anyos_std::sys::uptime_ms();
            let needs_layout = handle_image_done(
                tab_index,
                src,
                url,
                encoded_len,
                body,
                headers,
                decoded_raster,
                priority,
                from_deferred,
                generation,
            );
            log_main_phase_elapsed("handle_image_done", start_ms);
            if needs_layout {
                request_layout_refresh(tab_index);
            } else {
                request_image_refresh(tab_index);
            }
        }
        net_worker::FetchResult::IframeSnapshotDone {
            tab_index,
            node_id,
            src,
            url,
            body_len,
            pixels,
            render_width,
            render_height,
            stylesheet_count,
            script_count,
            render_ms,
            width: _requested_width,
            height: _requested_height,
            timing,
            from_cache,
            generation,
        } => {
            record_resource_completion(&url, body_len as u64, timing, from_cache);
            crate::surf_log!(
                "[surf] received IframeSnapshotDone: tab={} node={} src={} bytes={} css={} scripts={} pixels={} render_ms={} gen={}",
                tab_index,
                node_id,
                src,
                body_len,
                stylesheet_count,
                script_count,
                pixels.len(),
                render_ms,
                generation
            );
            let start_ms = anyos_std::sys::uptime_ms();
            let changed = resources::add_iframe_snapshot_pixels(
                tab_index,
                node_id,
                pixels,
                render_width,
                render_height,
                generation,
            );
            log_main_phase_elapsed("handle_iframe_snapshot_done", start_ms);
            if changed {
                request_image_refresh(tab_index);
            }
        }
        net_worker::FetchResult::FontDone {
            tab_index,
            family,
            weight,
            italic,
            url,
            body,
            display,
            timing,
            from_cache,
            generation,
        } => {
            record_resource_completion(&url, body.len() as u64, timing, from_cache);
            let start_ms = anyos_std::sys::uptime_ms();
            let needs_layout =
                handle_font_done(tab_index, family, weight, italic, body, display, generation);
            log_main_phase_elapsed("handle_font_done", start_ms);
            if needs_layout {
                request_render(tab_index, RenderWork::Layout, RenderSchedule::Debounced);
            }
        }
        net_worker::FetchResult::ScriptDone {
            tab_index,
            slot,
            url,
            body,
            headers,
            timing,
            from_cache,
            generation,
        } => {
            record_resource_completion(&url, body.len() as u64, timing, from_cache);
            crate::surf_log!(
                "[surf] received ScriptDone: tab={} slot={} bytes={} gen={}",
                tab_index,
                slot,
                body.len(),
                generation
            );
            let start_ms = anyos_std::sys::uptime_ms();
            handle_script_done(tab_index, slot, url, body, headers, generation);
            log_main_phase_elapsed("handle_script_done", start_ms);
        }
        net_worker::FetchResult::ModuleScriptDone {
            tab_index,
            specifier,
            url,
            body,
            headers,
            timing,
            from_cache,
            generation,
        } => {
            record_resource_completion(&url, body.len() as u64, timing, from_cache);
            crate::surf_log!(
                "[surf] received ModuleScriptDone: tab={} specifier={} bytes={} gen={}",
                tab_index,
                specifier,
                body.len(),
                generation
            );
            let start_ms = anyos_std::sys::uptime_ms();
            handle_module_script_done(tab_index, specifier, url, body, headers, generation);
            log_main_phase_elapsed("handle_module_script_done", start_ms);
        }
    }
}

fn render_delay_ms_for(tab_index: usize, work: RenderWork) -> u32 {
    let st = state();
    if tab_index >= st.tabs.len() {
        return 120;
    }
    let active = tab_index == st.active_tab;
    let phase = st.tabs[tab_index].load_state.phase;
    if active
        && work == RenderWork::Paint
        && (st.tabs[tab_index].deferred_images_inflight > 0
            || !st.tabs[tab_index].deferred_images.is_empty()
            || net_worker::result_mailboxes_pending())
    {
        return IMAGE_REPAINT_BURST_DELAY_MS;
    }
    match (active, work, phase) {
        (true, RenderWork::Paint, _) => 16,
        (true, RenderWork::Layout, PageLoadPhase::FetchingDocument)
        | (true, RenderWork::Layout, PageLoadPhase::ParsingDocument)
        | (true, RenderWork::Layout, PageLoadPhase::LoadingSubresources) => 24,
        (true, RenderWork::Layout, _) => 48,
        (_, RenderWork::Paint, _) => 80,
        _ => 120,
    }
}

fn schedule_render_flush(delay_ms: u32) {
    let st = state();
    let now = anyos_std::sys::uptime_ms();
    let new_due_ms = now.wrapping_add(delay_ms);

    if st.relayout_timer != 0 && st.relayout_due_ms != 0 {
        let current_remaining = st.relayout_due_ms.wrapping_sub(now);
        let new_remaining = new_due_ms.wrapping_sub(now);
        if current_remaining <= new_remaining {
            return;
        }
        ui_lib::kill_timer(st.relayout_timer);
        st.relayout_timer = 0;
    }
    st.relayout_timer = ui_lib::set_timer(delay_ms, flush_relayout);
    st.relayout_due_ms = new_due_ms;
}

/// Mark a tab as needing a relayout and start the debounce timer if not
/// already running.  The actual relayout happens in `flush_relayout()`.
fn request_render(tab_index: usize, work: RenderWork, schedule: RenderSchedule) {
    let st = state();
    if tab_index < st.render_dirty.len() {
        let current = st.render_dirty[tab_index];
        st.render_dirty[tab_index] = match (current, work) {
            (RenderWork::Layout, _) | (_, RenderWork::Layout) => RenderWork::Layout,
            (RenderWork::Paint, _) | (_, RenderWork::Paint) => RenderWork::Paint,
            _ => RenderWork::None,
        };
    }
    match schedule {
        RenderSchedule::Immediate => {
            flush_relayout_for_tab(tab_index);
        }
        RenderSchedule::Debounced => {
            let delay_ms = render_delay_ms_for(tab_index, work);
            schedule_render_flush(delay_ms);
        }
    }
    ensure_anim_timer();
}

pub(crate) fn request_layout_refresh(tab_index: usize) {
    request_render(tab_index, RenderWork::Layout, RenderSchedule::Debounced);
}

pub(crate) fn request_image_refresh(tab_index: usize) {
    // Image fetches often complete in long bursts on media-heavy pages. A
    // synchronous repaint per image makes the UI hitch continuously; mark the
    // tab paint-dirty and let the render timer coalesce many image arrivals
    // into one cached-layout repaint.
    request_render(tab_index, RenderWork::Paint, RenderSchedule::Debounced);
}

fn flush_relayout_for_tab(tab_idx: usize) {
    let st = state();
    if tab_idx >= st.tabs.len() || tab_idx >= st.render_dirty.len() {
        return;
    }
    let work = st.render_dirty[tab_idx];
    if work == RenderWork::None {
        return;
    }
    st.render_dirty[tab_idx] = RenderWork::None;
    let start_ms = anyos_std::sys::uptime_ms();
    match work {
        RenderWork::Layout => st.tabs[tab_idx].webview.relayout(),
        RenderWork::Paint => st.tabs[tab_idx].webview.repaint_from_cached_layout(),
        RenderWork::None => {}
    }
    if work == RenderWork::Layout && st.tabs[tab_idx].css_background_scan_pending {
        st.tabs[tab_idx].css_background_scan_pending = false;
        if let Some(base_url) = st.tabs[tab_idx].current_url.clone() {
            let queued = resources::queue_background_images(&base_url, tab_idx, 32);
            if queued > 0 {
                crate::surf_log!(
                    "[surf] queued CSS background images after layout: tab={} count={}",
                    tab_idx,
                    queued
                );
            }
        }
    }
    if work == RenderWork::Layout {
        if let Some(base_url) = st.tabs[tab_idx].current_url.clone() {
            let queued = resources::queue_iframe_snapshots(&base_url, tab_idx);
            if queued > 0 {
                crate::surf_log!(
                    "[surf] queued iframe snapshots after layout: tab={} count={}",
                    tab_idx,
                    queued
                );
            }
        }
        let _ = promote_viewport_deferred_images_for_tab(tab_idx);
    }
    let elapsed_ms = anyos_std::sys::uptime_ms().wrapping_sub(start_ms);
    crate::surf_log!(
        "[surf] render flush done: tab={} work={} elapsed={}ms",
        tab_idx,
        match work {
            RenderWork::Layout => "layout",
            RenderWork::Paint => "paint",
            RenderWork::None => "none",
        },
        elapsed_ms
    );
}

fn flush_pending_render_before_scripts(tab_index: usize) {
    let should_flush = {
        let st = state();
        tab_index < st.render_dirty.len() && st.render_dirty[tab_index] != RenderWork::None
    };
    if should_flush {
        crate::surf_log!(
            "[surf] flushing pending render before blocking scripts: tab={}",
            tab_index
        );
        flush_relayout_for_tab(tab_index);
    }
}

/// Debounce callback: perform one relayout per dirty tab, then clear flags
/// and stop the timer.
fn flush_relayout() {
    let st = state();
    let timer_id = st.relayout_timer;
    st.relayout_timer = 0;
    st.relayout_due_ms = 0;
    defer_kill_timer(timer_id);
    if scroll_interaction_hot() {
        schedule_render_flush(SCROLL_INTERACTION_GRACE_MS);
        return;
    }
    let mut any_dirty = false;
    let active_tab = st.active_tab;
    let mut background_renders = 0usize;

    if active_tab < st.render_dirty.len() && st.render_dirty[active_tab] != RenderWork::None {
        flush_relayout_for_tab(active_tab);
    }

    for tab_idx in 0..st.render_dirty.len() {
        if tab_idx == active_tab {
            continue;
        }
        if st.render_dirty[tab_idx] != RenderWork::None {
            if background_renders >= MAX_BACKGROUND_RENDERS_PER_FLUSH {
                any_dirty = true;
                continue;
            }
            flush_relayout_for_tab(tab_idx);
            background_renders += 1;
        }
    }

    // Check if new dirty flags were set during the relayouts above
    // (unlikely but possible if relayout triggers further resource loads).
    for &d in &st.render_dirty {
        if d != RenderWork::None {
            any_dirty = true;
            break;
        }
    }

    if any_dirty {
        schedule_render_flush(RELAYOUT_FOLLOWUP_DELAY_MS);
    }
}

/// Handle a completed navigation fetch: decode body, render HTML, update
/// history, queue external resources.
fn handle_nav_done(
    tab_index: usize,
    response: http::Response,
    original_url: http::Url,
    worker_cookies: http::CookieJar,
    generation: u32,
) {
    let st = state();
    let tab_idx = tab_index;
    if tab_idx >= st.tabs.len() {
        return;
    }

    // Discard stale result from a previous navigation.
    if !st.tabs[tab_idx].load_state.generation_matches(generation) {
        return;
    }

    // Merge cookies that the worker collected during the fetch.
    merge_cookies(worker_cookies);
    crate::surf_log!(
        "[surf] nav done: tab={} status={} bytes={} gen={}",
        tab_idx,
        response.status,
        response.body.len(),
        generation
    );

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
    if st.active_tab == tab_idx {
        ui::update_status();
    }

    // Decode response body (charset detection + Latin-1 transcoding).
    let body_text = resources::decode_http_body(&response.body, &response.headers);

    // Determine base URL (post-redirect URL takes precedence).
    let base_url = response.final_url.unwrap_or(original_url);
    let url_str = ui::format_url(&base_url);
    crate::surf_log!(
        "[surf] nav render start: tab={} url={} html_chars={}",
        tab_idx,
        url_str,
        body_text.len()
    );

    // Clear all state from the previous page (DOM, layout, images, JS, CSS).
    st.tabs[tab_idx].webview.navigate_clear();
    st.tabs[tab_idx].pending_scripts.clear();
    st.tabs[tab_idx].pending_script_modes.clear();
    st.tabs[tab_idx].pending_script_labels.clear();
    st.tabs[tab_idx].requested_module_urls.clear();
    st.tabs[tab_idx].deferred_fonts.clear();
    st.tabs[tab_idx].requested_font_urls.clear();
    st.tabs[tab_idx].deferred_fonts_inflight = 0;
    st.tabs[tab_idx].deferred_images.clear();
    st.tabs[tab_idx].requested_image_urls.clear();
    st.tabs[tab_idx].image_request_aliases.clear();
    st.tabs[tab_idx].requested_iframe_snapshots.clear();
    st.tabs[tab_idx].deferred_images_inflight = 0;
    st.tabs[tab_idx].favicon_pixels.clear();
    st.tabs[tab_idx].favicon_w = 0;
    st.tabs[tab_idx].favicon_h = 0;
    st.tabs[tab_idx].requested_favicon_url.clear();
    st.tabs[tab_idx].css_background_scan_pending = false;
    st.tabs[tab_idx].inline_svg_cache.clear();

    // Set URL and cookies on the JS runtime before rendering.
    st.tabs[tab_idx].webview.set_url(&url_str);
    let is_secure = base_url.scheme == "https";
    if let Some(cookie_hdr) = st
        .cookies
        .cookie_header(&base_url.host, &base_url.path, is_secure)
    {
        st.tabs[tab_idx]
            .webview
            .js_runtime()
            .set_cookies(&cookie_hdr);
    } else {
        st.tabs[tab_idx].webview.js_runtime().set_cookies("");
    }

    // Parse and store the HTML document (without JS). We intentionally defer
    // the first expensive full relayout until the stylesheet gate is ready so
    // large pages do not spend seconds rendering a mostly-unstyled version
    // only to immediately throw it away once CSS arrives.
    st.tabs[tab_idx].load_state.begin_parse();
    log_tab_load_state(tab_idx, "begin_parse");
    st.tabs[tab_idx].webview.set_html_dom_only(&body_text);
    crate::surf_log!(
        "[surf] html parsed: tab={} title_present={} dom_present={}",
        tab_idx,
        st.tabs[tab_idx].webview.get_title().is_some(),
        st.tabs[tab_idx].webview.dom().is_some()
    );
    if let Some(refresh_url) = st.tabs[tab_idx].webview.immediate_meta_refresh_url() {
        let resolved = http::resolve_url(&base_url, &refresh_url);
        let target = ui::format_url(&resolved);
        crate::surf_log!("[surf] meta refresh: tab={} to {}", tab_idx, target);
        if tab_idx == st.active_tab {
            tab::navigate(&target);
        }
        return;
    }

    // Extract page title.
    let title = st.tabs[tab_idx]
        .webview
        .get_title()
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
    if st.active_tab == tab_idx {
        let url_for_field = st.tabs[tab_idx].url_text.clone();
        st.url_field.set_text(&url_for_field);
        ui::update_title();
        ui::update_status();
    }
    ui::update_tab_labels();
    if st.active_tab == tab_idx {
        ui::update_devtools();
    }

    // Connect any WebSockets that JS requested during set_html().
    connect_pending_ws(tab_idx);

    let generation = st.tabs[tab_idx].load_state.generation;

    // Match surf-host more closely: queue stylesheets first, but do not let
    // hundreds of images block external scripts in the single network worker.
    let pending_stylesheet_count = if let Some(dom) = st.tabs[tab_idx].webview.dom() {
        resources::queue_favicon(dom, &base_url, tab_idx);
        let css_count = resources::queue_stylesheets(dom, &base_url, tab_idx);
        css_count
    } else {
        0
    };
    if resources::queue_inline_svgs(&st.tabs[tab_idx].webview, &base_url, tab_idx)
        && pending_stylesheet_count == 0
    {
        request_layout_refresh(tab_idx);
    }
    // Queue @font-face downloads from inline <style> blocks.
    resources::queue_font_faces(&st.tabs[tab_idx].webview, &base_url, tab_idx);

    // Collect script entries (inline + external) in document order.
    // External scripts are queued after stylesheets/fonts, but before images,
    // so app boot code is not delayed behind hundreds of media downloads.
    {
        let entries = st.tabs[tab_idx].webview.script_entries();
        let mut pending = Vec::with_capacity(entries.len());
        let mut modes = Vec::with_capacity(entries.len());
        let mut labels = Vec::with_capacity(entries.len());
        let mut ext_count = 0usize;
        let mut inline_count = 0usize;
        let mut async_count = 0usize;
        for (slot, entry) in entries.iter().enumerate() {
            match entry {
                libwebview::js::ScriptEntry::Inline { text, mode } => {
                    let label = String::from("<inline>");
                    if matches!(mode, libwebview::js::ScriptMode::Module) {
                        let specifier = anyos_std::format!("<inline-module-{}>", slot);
                        st.tabs[tab_idx]
                            .webview
                            .js_runtime()
                            .register_module_source(&specifier, text);
                        queue_module_dependencies(tab_idx, &base_url, text, generation);
                        pending.push(Some(module_import_wrapper(&specifier)));
                    } else {
                        pending.push(Some(text.clone()));
                    }
                    modes.push(mode.clone());
                    labels.push(label);
                    inline_count += 1;
                    if matches!(mode, libwebview::js::ScriptMode::Async) {
                        async_count += 1;
                    }
                }
                libwebview::js::ScriptEntry::External { src: src_url, mode } => {
                    let full_url = http::resolve_url(&base_url, src_url);
                    crate::surf_log!("[surf] queuing script fetch [{}]: {}", slot, src_url);
                    net_worker::submit(net_worker::FetchRequest::Script {
                        tab_index: tab_idx,
                        slot,
                        src: src_url.clone(),
                        url: full_url,
                        generation,
                    });
                    pending.push(None);
                    modes.push(mode.clone());
                    labels.push(src_url.clone());
                    if matches!(mode, libwebview::js::ScriptMode::Async) {
                        async_count += 1;
                    }
                    if !matches!(mode, libwebview::js::ScriptMode::Async) {
                        ext_count += 1;
                    }
                }
            }
        }
        st.tabs[tab_idx].pending_scripts = pending;
        st.tabs[tab_idx].pending_script_modes = modes;
        st.tabs[tab_idx].pending_script_labels = labels;
        st.tabs[tab_idx]
            .load_state
            .begin_subresource_load(pending_stylesheet_count, ext_count);
        crate::surf_log!(
            "[surf] subresources queued: tab={} stylesheets={} scripts_total={} inline_scripts={} external_blocking_or_defer={} async_scripts={}",
            tab_idx,
            pending_stylesheet_count,
            entries.len(),
            inline_count,
            ext_count,
            async_count
        );
        log_tab_load_state(tab_idx, "after_begin_subresource_load");
        if entries
            .iter()
            .any(|entry| matches!(entry, libwebview::js::ScriptEntry::External { .. }))
        {
            ensure_net_poll_timer();
        }
    }
    let modulepreload_count = queue_modulepreload_dependencies(tab_idx, &base_url, generation);
    if modulepreload_count > 0 {
        crate::surf_log!(
            "[surf] modulepreloads queued: tab={} count={}",
            tab_idx,
            modulepreload_count
        );
        log_tab_load_state(tab_idx, "after_modulepreload_queue");
    }

    if let Some(dom) = st.tabs[tab_idx].webview.dom() {
        let startup_critical_only =
            pending_stylesheet_count > 0 || st.tabs[tab_idx].load_state.pending_script_count > 0;
        resources::queue_images(dom, &base_url, tab_idx, startup_critical_only);
        let _ = promote_viewport_deferred_images_for_tab(tab_idx);
    }
    log_tab_load_state(tab_idx, "after_queue_images");

    if pending_stylesheet_count == 0 {
        request_render(tab_idx, RenderWork::Layout, RenderSchedule::Immediate);
    }

    // Run scripts only after the stylesheet chain has finished loading, and
    // flush any pending CSS layout before JS gets a chance to mutate the DOM.
    if st.tabs[tab_idx].load_state.ready_for_script_execution() {
        execute_pending_scripts(tab_idx);
    } else {
        flush_deferred_resize_if_ready();
    }

    // Restart animation/scroll tick timer (may have been stopped while idle).
    ensure_anim_timer();

    // Refresh DevTools panels — DOM is now populated.
    devtools::refresh_inspector();
    devtools::refresh_console();
}

/// Handle a navigation error: show the error message in the status bar.
fn handle_nav_error(tab_index: usize, error_msg: &'static str, generation: u32) {
    let st = state();
    let tab_idx = tab_index;
    if tab_idx >= st.tabs.len() || !st.tabs[tab_idx].load_state.generation_matches(generation) {
        return;
    }
    st.tabs[tab_idx].load_state.mark_failed();
    st.tabs[tab_idx].is_loading = false;
    st.tabs[tab_idx].status_text = String::from(error_msg);
    if st.active_tab == tab_idx {
        ui::update_status();
    }
    ui::update_tab_labels();
    flush_deferred_resize_if_ready();
}

/// Handle a completed CSS stylesheet fetch: apply the stylesheet.
fn handle_css_done(
    tab_index: usize,
    href: String,
    body: Vec<u8>,
    headers: String,
    parsed: Option<net_worker::DecodedCss>,
    generation: u32,
) {
    let st = state();
    if tab_index >= st.tabs.len() {
        return;
    }
    if !st.tabs[tab_index].load_state.generation_matches(generation) {
        return;
    }

    crate::surf_log!(
        "[surf] css done: tab={} href={} bytes={} gen={} pending_css_before={}",
        tab_index,
        href,
        body.len(),
        generation,
        st.tabs[tab_index].load_state.pending_stylesheet_count
    );

    st.tabs[tab_index].load_state.on_stylesheet_finished();
    log_tab_load_state(tab_index, "after_css_finished");

    if body.is_empty() {
        crate::surf_log!("[surf] skipped empty/failed CSS: {}", href);
        if st.tabs[tab_index].load_state.pending_stylesheet_count == 0 {
            st.tabs[tab_index].css_background_scan_pending = true;
            request_render(tab_index, RenderWork::Layout, RenderSchedule::Debounced);
        }
        if st.tabs[tab_index].load_state.ready_for_script_execution() {
            flush_pending_render_before_scripts(tab_index);
            execute_pending_scripts(tab_index);
        }
        return;
    }

    let css_text = resources::decode_http_body(&body, &headers);
    crate::surf_log!(
        "[surf] css fingerprint: href={} chars={} fnv1a={:016x} prefix={:?}",
        href,
        css_text.len(),
        debug_text_fingerprint(&css_text),
        debug_text_prefix(&css_text, 120)
    );

    if let Some(parsed) = parsed {
        st.tabs[tab_index]
            .webview
            .add_parsed_stylesheet(parsed.sheet);
    } else {
        st.tabs[tab_index].webview.add_stylesheet(&css_text);
    }
    crate::surf_log!("[surf] applied CSS: {}", href);

    // Process @import URLs from the newly added stylesheet.
    if let Ok(base_url) = crate::http::parse_url(&href) {
        let imports: Vec<String> = st.tabs[tab_index]
            .webview
            .last_stylesheet_imports()
            .to_vec();
        let had_imports = !imports.is_empty();
        if had_imports {
            crate::surf_log!(
                "[surf] css imports discovered: tab={} href={} count={}",
                tab_index,
                href,
                imports.len()
            );
        }
        st.tabs[tab_index]
            .load_state
            .on_stylesheets_added(imports.len());
        for import_url in imports {
            let resolved = crate::http::resolve_url(&base_url, &import_url);
            crate::net_worker::submit(crate::net_worker::FetchRequest::Css {
                tab_index,
                href: String::from(import_url.as_str()),
                url: resolved,
                generation,
            });
        }
        if had_imports {
            ensure_net_poll_timer();
            log_tab_load_state(tab_index, "after_css_imports_added");
        }

        // Process @font-face rules — queue font downloads.
        let font_faces: Vec<(String, String, u32, bool, libwebview::css::FontDisplay)> = st.tabs
            [tab_index]
            .webview
            .last_stylesheet_font_faces()
            .iter()
            .map(|ff| {
                (
                    ff.family.clone(),
                    ff.src_url.clone(),
                    ff.weight,
                    ff.italic,
                    ff.display,
                )
            })
            .collect();
        resources::queue_font_face_batch(tab_index, generation, &base_url, &font_faces);
    }

    if st.tabs[tab_index].load_state.pending_stylesheet_count == 0 {
        st.tabs[tab_index].css_background_scan_pending = true;
        let schedule = {
            let active = tab_index == st.active_tab;
            if active {
                RenderSchedule::Immediate
            } else {
                RenderSchedule::Debounced
            }
        };
        request_render(tab_index, RenderWork::Layout, schedule);
    }

    if st.tabs[tab_index].load_state.ready_for_script_execution() {
        flush_pending_render_before_scripts(tab_index);
        execute_pending_scripts(tab_index);
    }
}

/// Handle a completed web font fetch: load TTF data into libfont.
///
/// Returns `true` if the font was loaded and a relayout is needed.
fn handle_font_done(
    tab_index: usize,
    family: String,
    weight: u32,
    italic: bool,
    body: Vec<u8>,
    display: libwebview::css::FontDisplay,
    generation: u32,
) -> bool {
    let st = state();
    if tab_index >= st.tabs.len() {
        return false;
    }
    if !st.tabs[tab_index].load_state.generation_matches(generation) {
        return false;
    }
    crate::surf_log!(
        "[surf] font done: tab={} family='{}' bytes={} display={} gen={}",
        tab_index,
        family,
        body.len(),
        font_display_name(display),
        generation
    );

    // Try loading the font data (supports TTF/sfnt and WOFF2 TrueType outlines).
    if let Some(font_id) = resources::load_valid_web_font_data(&family, &body) {
        st.tabs[tab_index]
            .webview
            .register_web_font_with_style(&family, weight, italic, font_id);
        if matches!(
            display,
            libwebview::css::FontDisplay::Swap
                | libwebview::css::FontDisplay::Fallback
                | libwebview::css::FontDisplay::Optional
        ) && st.tabs[tab_index].deferred_fonts_inflight > 0
        {
            st.tabs[tab_index].deferred_fonts_inflight -= 1;
        }
        crate::surf_log!("[surf] loaded web font '{}' -> id {}", family, font_id);
        log_tab_load_state(tab_index, "after_font_loaded");
        if matches!(
            st.tabs[tab_index].load_state.phase,
            PageLoadPhase::Interactive
        ) {
            pump_deferred_fonts_for_tab(tab_index);
        }
        true
    } else {
        if matches!(
            display,
            libwebview::css::FontDisplay::Swap
                | libwebview::css::FontDisplay::Fallback
                | libwebview::css::FontDisplay::Optional
        ) && st.tabs[tab_index].deferred_fonts_inflight > 0
        {
            st.tabs[tab_index].deferred_fonts_inflight -= 1;
        }
        crate::surf_log!("[surf] failed to load web font '{}'", family);
        log_tab_load_state(tab_index, "after_font_failed");
        if matches!(
            st.tabs[tab_index].load_state.phase,
            PageLoadPhase::Interactive
        ) {
            pump_deferred_fonts_for_tab(tab_index);
        }
        false
    }
}

/// Handle a completed image fetch: decode SVG or raster and add to cache.
///
/// Returns `true` if the image decode can affect geometry and therefore needs
/// a relayout instead of a paint-only refresh.
fn handle_image_done(
    tab_index: usize,
    src: String,
    url: http::Url,
    encoded_len: usize,
    _body: Vec<u8>,
    headers: String,
    decoded_raster: Option<net_worker::DecodedRaster>,
    priority: net_worker::ImagePriority,
    from_deferred: bool,
    generation: u32,
) -> bool {
    let st = state();
    if tab_index >= st.tabs.len() {
        return false;
    }
    if !st.tabs[tab_index].load_state.generation_matches(generation) {
        return false;
    }
    crate::surf_log!(
        "[surf] image done: tab={} src={} bytes={} priority={} gen={}",
        tab_index,
        src,
        encoded_len,
        image_priority_name(priority),
        generation
    );

    if resources::is_favicon_src(&src) {
        if let Some(decoded_raster) = decoded_raster {
            if let Some((pixels, w, h)) = resources::scale_icon_pixels(
                &decoded_raster.pixels,
                decoded_raster.width,
                decoded_raster.height,
            ) {
                st.tabs[tab_index].favicon_pixels = pixels;
                st.tabs[tab_index].favicon_w = w;
                st.tabs[tab_index].favicon_h = h;
                st.tabs[tab_index].requested_favicon_url = ui::format_url(&url);
                ui::update_tab_labels();
                crate::surf_log!(
                    "[surf] favicon loaded: tab={} url={} size={}x{}",
                    tab_index,
                    st.tabs[tab_index].requested_favicon_url,
                    w,
                    h
                );
            }
        } else if !src.ends_with("/favicon.ico") {
            let _ = resources::queue_favicon_fallback(tab_index);
        }
        return false;
    }

    let _ = headers;
    let mut needs_layout = false;
    if let Some(decoded_raster) = decoded_raster {
        if let Some(black_ratio_ppm) = decoded_raster.suspicious_black_ppm {
            crate::surf_log!(
                "[surf] WARN suspicious worker image decode: src={} format={} {}x{} black_ratio_ppm={} tab={}",
                src,
                libimage_client::format_name(decoded_raster.format),
                decoded_raster.width,
                decoded_raster.height,
                black_ratio_ppm,
                tab_index
            );
        }
        let request_key = resources::image_request_key_for_url(&url);
        let mut image_srcs = Vec::new();
        image_srcs.push(src.clone());
        for alias in &st.tabs[tab_index].image_request_aliases {
            if alias.request_key == request_key
                && alias.src != src
                && !image_srcs.iter().any(|existing| existing == &alias.src)
            {
                image_srcs.push(alias.src.clone());
            }
        }
        let alias_count = image_srcs.len().saturating_sub(1);
        let width = decoded_raster.width;
        let height = decoded_raster.height;
        let mut pixels = decoded_raster.pixels;
        let last_idx = image_srcs.len().saturating_sub(1);
        for (idx, image_src) in image_srcs.into_iter().enumerate() {
            let pixels_for_src = if idx == last_idx {
                core::mem::take(&mut pixels)
            } else {
                pixels.clone()
            };
            needs_layout |= st.tabs[tab_index].webview.add_image_and_get_layout_effect(
                &image_src,
                pixels_for_src,
                width,
                height,
            );
        }
        if alias_count > 0 {
            crate::surf_log!(
                "[surf] image aliases applied: tab={} key={} aliases={}",
                tab_index,
                request_key,
                alias_count
            );
        }
    } else {
        crate::surf_log!(
            "[surf] image unavailable after worker decode: tab={} src={} bytes={}",
            tab_index,
            src,
            encoded_len
        );
    }
    if from_deferred && st.tabs[tab_index].deferred_images_inflight > 0 {
        st.tabs[tab_index].deferred_images_inflight -= 1;
    }
    if matches!(
        st.tabs[tab_index].load_state.phase,
        PageLoadPhase::Interactive
    ) {
        pump_deferred_images_for_tab(tab_index);
    }
    log_tab_load_state(tab_index, "after_image_done");
    needs_layout
}

/// Handle a completed external script fetch.
///
/// Places the fetched text into the tab's `pending_scripts` slot.
/// When all slots are filled, executes all scripts in document order.
fn module_url_key(url: &http::Url) -> String {
    ui::format_url(url)
}

fn register_module_source_aliases(
    tab_index: usize,
    specifier: &str,
    url: &http::Url,
    source: &str,
) {
    let st = state();
    if tab_index >= st.tabs.len() || source.is_empty() {
        return;
    }
    let absolute = module_url_key(url);
    st.tabs[tab_index]
        .webview
        .js_runtime()
        .register_module_source(specifier, source);
    st.tabs[tab_index]
        .webview
        .js_runtime()
        .register_module_source(&absolute, source);
    for alias in module_source_aliases(specifier, url) {
        st.tabs[tab_index]
            .webview
            .js_runtime()
            .register_module_source(&alias, source);
    }
}

fn module_source_aliases(specifier: &str, url: &http::Url) -> Vec<String> {
    let mut aliases = Vec::new();
    push_unique_module_alias(&mut aliases, specifier);
    push_unique_module_alias(&mut aliases, &module_url_key(url));
    push_unique_module_alias(&mut aliases, &url.path);
    push_unique_module_alias(&mut aliases, url.path.trim_start_matches('/'));

    if let Some(file_start) = url.path.rfind('/') {
        let file = &url.path[file_start + 1..];
        push_unique_module_alias(&mut aliases, &anyos_std::format!("./{}", file));
        if url.path.contains("/chunks/") {
            push_unique_module_alias(&mut aliases, &anyos_std::format!("../chunks/{}", file));
            push_unique_module_alias(&mut aliases, &anyos_std::format!("./chunks/{}", file));
        }
        if url.path.contains("/entries/") {
            push_unique_module_alias(&mut aliases, &anyos_std::format!("../entries/{}", file));
            push_unique_module_alias(&mut aliases, &anyos_std::format!("./entries/{}", file));
        }
    }

    aliases
}

fn push_unique_module_alias(aliases: &mut Vec<String>, alias: &str) {
    if alias.is_empty() || aliases.iter().any(|existing| existing == alias) {
        return;
    }
    aliases.push(String::from(alias));
}

fn queue_modulepreload_dependencies(
    tab_index: usize,
    page_url: &http::Url,
    generation: u32,
) -> usize {
    let links = state()
        .tabs
        .get(tab_index)
        .and_then(|tab| tab.webview.dom())
        .map(libwebview::js::extract_modulepreload_links_from_dom)
        .unwrap_or_default();
    if links.is_empty() {
        return 0;
    }

    let st = state();
    if tab_index >= st.tabs.len() {
        return 0;
    }

    let mut queued = 0usize;
    for specifier in links {
        let url = http::resolve_url(page_url, &specifier);
        let key = module_url_key(&url);
        if st.tabs[tab_index]
            .requested_module_urls
            .iter()
            .any(|existing| existing == &key)
        {
            continue;
        }
        st.tabs[tab_index].requested_module_urls.push(key);
        crate::surf_log!(
            "[surf] queuing modulepreload fetch: tab={} specifier={}",
            tab_index,
            specifier
        );
        net_worker::submit(net_worker::FetchRequest::ModuleScript {
            tab_index,
            specifier,
            url,
            generation,
        });
        queued += 1;
    }

    if queued > 0 {
        st.tabs[tab_index].load_state.on_module_added(queued);
        ensure_net_poll_timer();
    }
    queued
}

fn queue_module_dependencies(
    tab_index: usize,
    referrer_url: &http::Url,
    source: &str,
    generation: u32,
) -> usize {
    let page_url = state()
        .tabs
        .get(tab_index)
        .and_then(|tab| tab.current_url.as_ref())
        .map(module_url_key)
        .unwrap_or_default();
    let current_page_id = state()
        .tabs
        .get(tab_index)
        .and_then(|tab| tab.webview.dom())
        .and_then(libwebview::js::extract_vike_page_id_from_dom);
    let specs = libwebview::js::extract_module_specifiers_for_page_with_page_id(
        source,
        &page_url,
        current_page_id.as_deref(),
    );
    if specs.is_empty() {
        return 0;
    }

    let st = state();
    if tab_index >= st.tabs.len() {
        return 0;
    }

    let mut queued = 0usize;
    for specifier in specs {
        let url = http::resolve_url(referrer_url, &specifier);
        let key = module_url_key(&url);
        if st.tabs[tab_index]
            .requested_module_urls
            .iter()
            .any(|existing| existing == &key)
        {
            continue;
        }
        st.tabs[tab_index].requested_module_urls.push(key);
        crate::surf_log!(
            "[surf] queuing module fetch: tab={} specifier={} referrer={}://{}{}",
            tab_index,
            specifier,
            referrer_url.scheme,
            referrer_url.host,
            referrer_url.path
        );
        net_worker::submit(net_worker::FetchRequest::ModuleScript {
            tab_index,
            specifier,
            url,
            generation,
        });
        queued += 1;
    }

    if queued > 0 {
        st.tabs[tab_index].load_state.on_module_added(queued);
        ensure_net_poll_timer();
    }
    queued
}

fn handle_module_script_done(
    tab_index: usize,
    specifier: String,
    url: http::Url,
    body: Vec<u8>,
    headers: String,
    generation: u32,
) {
    let st = state();
    if tab_index >= st.tabs.len() {
        return;
    }
    if !st.tabs[tab_index].load_state.generation_matches(generation) {
        return;
    }
    let text = resources::decode_http_body(&body, &headers);
    if !text.is_empty() {
        register_module_source_aliases(tab_index, &specifier, &url, &text);
        queue_module_dependencies(tab_index, &url, &text, generation);
    }
    st.tabs[tab_index].load_state.on_module_finished();
    log_tab_load_state(tab_index, "after_module_script_done");
    if st.tabs[tab_index].load_state.ready_for_script_execution() {
        execute_pending_scripts(tab_index);
    }
}

fn handle_script_done(
    tab_index: usize,
    slot: usize,
    url: http::Url,
    body: Vec<u8>,
    headers: String,
    generation: u32,
) {
    let st = state();
    if tab_index >= st.tabs.len() {
        crate::surf_log!(
            "[surf] dropping ScriptDone: tab {} out of range (tabs={})",
            tab_index,
            st.tabs.len()
        );
        return;
    }
    if !st.tabs[tab_index].load_state.generation_matches(generation) {
        crate::surf_log!(
            "[surf] dropping ScriptDone: stale generation tab={} slot={} got={} expected={}",
            tab_index,
            slot,
            generation,
            st.tabs[tab_index].load_state.generation
        );
        return;
    }
    if slot >= st.tabs[tab_index].pending_scripts.len() {
        crate::surf_log!(
            "[surf] dropping ScriptDone: slot {} out of range (pending_len={})",
            slot,
            st.tabs[tab_index].pending_scripts.len()
        );
        return;
    }

    let text = resources::decode_http_body(&body, &headers);
    let label = st.tabs[tab_index]
        .pending_script_labels
        .get(slot)
        .cloned()
        .unwrap_or_else(|| String::from("<unknown>"));
    if !text.is_empty() {
        register_module_source_aliases(tab_index, &label, &url, &text);
        queue_module_dependencies(tab_index, &url, &text, generation);
    }
    crate::surf_log!(
        "[surf] script [{}] fetched: {} bytes label={} pending_before={}",
        slot,
        text.len(),
        label,
        st.tabs[tab_index].load_state.pending_script_count
    );
    let mode = st.tabs[tab_index]
        .pending_script_modes
        .get(slot)
        .cloned()
        .unwrap_or(libwebview::js::ScriptMode::Blocking);
    if matches!(mode, libwebview::js::ScriptMode::Module) {
        st.tabs[tab_index].pending_scripts[slot] =
            Some(module_import_wrapper(&module_url_key(&url)));
    } else {
        st.tabs[tab_index].pending_scripts[slot] = Some(text);
    }

    let mode = st.tabs[tab_index]
        .pending_script_modes
        .get(slot)
        .cloned()
        .unwrap_or(libwebview::js::ScriptMode::Blocking);
    if !matches!(mode, libwebview::js::ScriptMode::Async) {
        st.tabs[tab_index].load_state.on_script_finished();
    }

    crate::surf_log!(
        "[surf] script [{}] stored: pending_after={} stylesheets_pending={} mode={}",
        slot,
        st.tabs[tab_index].load_state.pending_script_count,
        st.tabs[tab_index].load_state.pending_stylesheet_count,
        script_mode_name(mode.clone())
    );
    log_tab_load_state(tab_index, "after_script_done");

    if matches!(mode, libwebview::js::ScriptMode::Async) {
        crate::surf_log!(
            "[surf] async script [{}] buffered until blocking/defer gate opens",
            slot
        );
    }

    // Execute when the blocking stylesheet/script gate is open.
    if st.tabs[tab_index].load_state.ready_for_script_execution() {
        execute_pending_scripts(tab_index);
    }
}

/// Schedule pending scripts for a tab in document order.
///
/// Called when all external script fetches have completed (or immediately
/// if the page has only inline scripts). The actual execution is pumped in
/// small UI-timer slices so heavy pages do not block image/font completion
/// accounting and painting for many seconds.
fn execute_pending_scripts(tab_index: usize) {
    let st = state();
    if tab_index >= st.tabs.len() {
        return;
    }
    if !st.tabs[tab_index].load_state.ready_for_script_execution() {
        log_tab_load_state(tab_index, "execute_pending_scripts_blocked");
        return;
    }

    if !tab_has_pending_script_kind(tab_index, false) {
        st.tabs[tab_index].load_state.mark_interactive();
        log_tab_load_state(tab_index, "mark_interactive_no_blocking_scripts");
        execute_buffered_async_scripts(tab_index);
        pump_deferred_fonts_for_tab(tab_index);
        pump_deferred_images_for_tab(tab_index);
        flush_deferred_resize_if_ready();
        return;
    }

    crate::surf_log!(
        "[surf] execute_pending_scripts: tab={} pending_slots={} scheduled",
        tab_index,
        st.tabs[tab_index].pending_scripts.len()
    );
    schedule_script_pump_for_tab(tab_index);
}

/// Merge cookies returned by the worker thread into the main cookie jar.
///
/// Worker-side cookies take precedence (they represent the most recent
/// Set-Cookie headers from the server).
fn merge_cookies(worker_jar: http::CookieJar) {
    let st = state();
    for cookie in worker_jar.cookies {
        // Replace existing cookie with same name+domain+path, or add new.
        let existing =
            st.cookies.cookies.iter_mut().find(|c| {
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
    crate::surf_log!("[surf] starting...");

    if !ui_lib::init() {
        crate::surf_log!("[surf] ERROR: failed to init libanyui");
        return;
    }
    i18n::init();

    if !libsvg_client::init() {
        crate::surf_log!("[surf] WARN: libsvg.so not available — SVG images disabled");
    }

    // Load user settings and bookmarks from disk.
    let (surf_config, surf_bookmarks) = config::load();

    // Optional startup URL from the process argument string.
    let mut args_buf = [0u8; 256];
    let raw_args = anyos_std::process::args(&mut args_buf);
    let arg_url = raw_args.trim();
    let pending_start_url = if arg_url.is_empty() {
        if surf_config.homepage.is_empty() {
            None
        } else {
            Some(surf_config.homepage.clone())
        }
    } else {
        Some(String::from(arg_url))
    };

    // ── Window ──────────────────────────────────────────────────────────────
    let win = ui_lib::Window::new(i18n::t("Surf"), -1, -1, 900, 700);
    let tc = ui_lib::theme::colors();

    // ── Toolbar (DOCK_TOP, 40 px) ────────────────────────────────────────────
    let toolbar = ui_lib::View::new();
    toolbar.set_dock(ui_lib::DOCK_TOP);
    toolbar.set_size(0, 40);
    toolbar.set_color(tc.toolbar_bg);
    toolbar.set_padding(6, 6, 6, 6);
    win.add(&toolbar);

    // Navigation buttons (DOCK_LEFT).
    let nav_group = ui_lib::View::new();
    nav_group.set_dock(ui_lib::DOCK_LEFT);
    nav_group.set_size(104, 28);
    nav_group.set_color(tc.toolbar_bg);
    toolbar.add(&nav_group);

    let btn_back = ui_lib::IconButton::new("");
    btn_back.set_position(0, 0);
    btn_back.set_size(32, 28);
    btn_back.set_system_icon(
        "chevron-left",
        ui_lib::IconType::Outline,
        tc.text_secondary,
        20,
    );
    nav_group.add(&btn_back);

    let btn_forward = ui_lib::IconButton::new("");
    btn_forward.set_position(34, 0);
    btn_forward.set_size(32, 28);
    btn_forward.set_system_icon(
        "chevron-right",
        ui_lib::IconType::Outline,
        tc.text_secondary,
        20,
    );
    nav_group.add(&btn_forward);

    let btn_reload = ui_lib::IconButton::new("");
    btn_reload.set_position(68, 0);
    btn_reload.set_size(32, 28);
    btn_reload.set_system_icon("refresh", ui_lib::IconType::Outline, tc.text_secondary, 20);
    nav_group.add(&btn_reload);

    // Hamburger menu button (DOCK_RIGHT).
    let btn_menu = ui_lib::IconButton::new("");
    btn_menu.set_dock(ui_lib::DOCK_RIGHT);
    btn_menu.set_size(36, 28);
    btn_menu.set_system_icon("menu-2", ui_lib::IconType::Outline, tc.text_secondary, 20);
    toolbar.add(&btn_menu);

    // URL field (DOCK_FILL — takes all remaining width).
    let url_field = ui_lib::TextField::new();
    url_field.set_dock(ui_lib::DOCK_FILL);
    if surf_config.homepage.is_empty() {
        url_field.set_placeholder(i18n::t("Enter URL..."));
    } else {
        url_field.set_placeholder(&surf_config.homepage);
    }
    toolbar.add(&url_field);

    // Loading progress bar (DOCK_TOP, 3px, below toolbar).
    let url_progress = ui_lib::ProgressBar::new(0);
    url_progress.set_dock(ui_lib::DOCK_TOP);
    url_progress.set_size(0, 3);
    url_progress.set_color(tc.accent);
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

    // ── DevTools window (Inspector / Console / Network / …) ────────────────
    let devtools = devtools::build();

    // ── Status bar (DOCK_BOTTOM, 24 px) ─────────────────────────────────────
    let status_label = ui_lib::Label::new(i18n::t("Ready"));
    status_label.set_dock(ui_lib::DOCK_BOTTOM);
    status_label.set_size(0, 24);
    status_label.set_color(tc.toolbar_bg);
    status_label.set_text_color(tc.text_secondary);
    status_label.set_font_size(12);
    status_label.set_padding(8, 4, 0, 0);
    win.add(&status_label);

    // ── Content area (DOCK_FILL) ─────────────────────────────────────────────
    let content_view = ui_lib::View::new();
    content_view.set_dock(ui_lib::DOCK_FILL);
    content_view.set_color(tc.window_bg);
    win.add(&content_view);

    // ── Initial tab ──────────────────────────────────────────────────────────
    let mut initial_tab = tab::TabState::new();
    initial_tab
        .webview
        .set_link_callback(callbacks::on_link_click, 0);
    initial_tab
        .webview
        .set_submit_callback(callbacks::on_form_submit, 0);
    content_view.add(initial_tab.webview.scroll_view());
    initial_tab
        .webview
        .scroll_view()
        .set_dock(ui_lib::DOCK_FILL);
    initial_tab.webview.scroll_view().on_scroll(|_| {
        mark_scroll_activity();
    });

    unsafe {
        STATE = Some(AppState {
            win,
            toolbar,
            nav_group,
            btn_back,
            btn_forward,
            btn_reload,
            btn_menu,
            url_field,
            url_progress,
            tab_bar_view,
            content_view,
            status_label,
            devtools,
            tabs: vec![initial_tab],
            active_tab: 0,
            cookies: http::CookieJar {
                cookies: Vec::new(),
            },
            conn_pool: http::ConnPool::new(),
            ws_connections: Vec::new(),
            ws_poll_timer: 0,
            anim_timer: 0,
            net_poll_timer: 0,
            script_pump_timer: 0,
            js_runtime_timer: 0,
            js_worker_poll_timer: 0,
            start_nav_timer: 0,
            js_timer_quiet_ticks: [0; 16],
            render_dirty: [RenderWork::None; 16],
            scroll_render_pending: false,
            last_scroll_input_ms: 0,
            last_visual_tick_ms: 0,
            last_idle_tile_prerender_ms: 0,
            relayout_timer: 0,
            relayout_due_ms: 0,
            resize_timer: 0,
            deferred_resize_pending: false,
            theme_timer: 0,
            last_theme_light: ui_lib::theme::is_light(),
            config: surf_config,
            bookmarks: surf_bookmarks,
            pending_start_url,
        });
    }

    ui::apply_theme();
    ui::ensure_theme_timer();

    // Initialize background network worker queues.
    net_worker::init();
    js_worker::init();

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
            2 => {
                let st = state();
                ui::close_tab(st.active_tab);
            }
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
    st.btn_back.on_click(|_| {
        tab::go_back();
    });
    st.btn_forward.on_click(|_| {
        tab::go_forward();
    });
    st.btn_reload.on_click(|_| {
        tab::reload();
    });

    // Hamburger menu button: open popup on left-click.
    st.btn_menu.on_click(|_| {
        let st = state();
        st.btn_menu.open_popup();
    });

    // Hamburger menu item handler.
    hamburger_menu.on_item_click(|e| {
        match e.index {
            0 => ui::add_tab(), // New Tab
            1 => {
                let st = state();
                ui::close_tab(st.active_tab);
            } // Close Tab
            // 2 = separator
            3 => {} // History (TODO)
            4 => {} // Downloads (TODO)
            5 => bookmarks::open_bookmark_manager(),
            // 6 = separator
            7 => {} // Zoom In (TODO)
            8 => {} // Zoom Out (TODO)
            9 => {} // Reset Zoom (TODO)
            // 10 = separator
            11 => settings::open_settings(),
            12 => ui::toggle_devtools(), // Developer Tools
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

    win.on_close(|_| {
        ui_lib::quit();
    });

    // ── DevTools wiring ─────────────────────────────────────────────────────
    {
        let st = state();
        // Inspector: refresh styles when the user picks a node in the tree.
        st.devtools.dom_tree.on_selection_changed(|_| {
            devtools::show_selected_node_styles();
        });
        // Console: Enter in the input field evaluates the expression.
        st.devtools.console_input.on_submit(|_| {
            devtools::eval_console_input();
        });
        // Tab bar: switch which panel is visible AND toggle picker mode.
        // libanyui exposes only one on_change per control, so the panel
        // switching has to live inside the same callback.
        st.devtools.tab_bar.on_active_changed(|e| {
            devtools::switch_panel(e.index);
        });
        st.devtools.win.on_close(|_| {
            let st = state();
            st.devtools.open = false;
            st.devtools.win.set_visible(false);
        });

        // Network panel: clear / pause / search / kind filters.
        st.devtools.net_clear_btn.on_click(|_| {
            devtools::clear_network();
        });
        st.devtools.net_pause_btn.on_click(|_| {
            devtools::toggle_pause();
        });
        st.devtools.net_search.on_text_changed(|_| {
            devtools::refresh_network();
        });
        let kinds = st.devtools.net_filter_btns.len();
        for i in 0..kinds {
            let id = st.devtools.net_filter_btns[i].0.clone();
            st.devtools.net_filter_btns[i].1.on_click(move |_| {
                devtools::set_kind_filter(&id);
            });
        }
    }

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
        } else if ctrl && key == b'D' as u32 {
            // Ctrl+D — quick-add current page to bookmarks.
            bookmarks::add_current_page();
        } else if ctrl && key == b'B' as u32 {
            // Ctrl+B — open bookmark manager.
            bookmarks::open_bookmark_manager();
        }
    });

    win.on_click(|_| {});

    // Viewport resize: re-layout the active tab's webview.
    win.on_resize(|_| {
        schedule_active_webview_resize(50);
    });

    // Start the CSS animation tick timer.
    start_anim_timer();

    // One-shot timer: after the first layout pass, resize the WebView to the
    // actual content_view dimensions (dock sizes aren't computed until run()),
    // then start the initial navigation so CSS media queries and early JS see
    // the same viewport that surf-host starts with.
    schedule_active_webview_resize(50);
    state().start_nav_timer = ui_lib::set_timer(75, run_pending_start_navigation);

    crate::surf_log!("[surf] entering event loop");
    ui_lib::run();
    anyos_std::process::exit(0);
}
