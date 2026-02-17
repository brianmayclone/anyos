// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Surf -- a tabbed web browser for anyOS.
//!
//! Renders HTML pages with CSS styling, fetched over HTTP/1.1.
//! Uses libcompositor_client for window management and uisys_client
//! for toolbar widgets (text fields, buttons, scrollbar).

#![no_std]
#![no_main]

mod dom;
mod html;
mod css;
mod style;
mod layout;
mod paint;
mod http;
mod deflate;

anyos_std::entry!(main);

use alloc::string::String;
use alloc::vec::Vec;
use alloc::vec;
use libcompositor_client::*;
use uisys_client::*;

// ---------------------------------------------------------------------------
// Layout constants
// ---------------------------------------------------------------------------

const TOOLBAR_H: i32 = 40;
const TAB_BAR_H: i32 = 30;
const CHROME_H: i32 = TOOLBAR_H + TAB_BAR_H;
const STATUS_H: i32 = 24;
const SCROLLBAR_W: u32 = 8;
const BTN_W: i32 = 32;
const BTN_H: i32 = 28;
const BTN_Y: i32 = 6;
const URL_X: i32 = 110;
const URL_H: u32 = 28;
const URL_Y: i32 = 6;

const TAB_MAX_W: i32 = 180;
const TAB_MIN_W: i32 = 60;
const TAB_NEW_BTN_W: i32 = 28;

// Colors
const BG: u32 = 0xFF1E1E1E;
const TOOLBAR_BG: u32 = 0xFF2A2A2C;
const STATUS_BG: u32 = 0xFF252525;
const STATUS_TEXT: u32 = 0xFF969696;
const SEPARATOR: u32 = 0xFF3D3D3D;
const TAB_BG: u32 = 0xFF333335;
const TAB_ACTIVE_BG: u32 = 0xFF1E1E1E;
const TAB_TEXT: u32 = 0xFFA0A0A0;
const TAB_ACTIVE_TEXT: u32 = 0xFFE6E6E6;
const TAB_CLOSE_TEXT: u32 = 0xFF808080;
const TAB_BAR_BG: u32 = 0xFF252527;

// Menu item IDs
const MENU_OPEN_URL: u32 = 1;
const MENU_QUIT: u32 = 2;
const MENU_RELOAD: u32 = 10;
const MENU_NEW_TAB: u32 = 11;
const MENU_CLOSE_TAB: u32 = 12;
const MENU_ABOUT: u32 = 20;

// ---------------------------------------------------------------------------
// WinSurface trick for uisys DLL rendering on raw surface
// ---------------------------------------------------------------------------

#[repr(C)]
struct WinSurface {
    pixels: *mut u32,
    width: u32,
    height: u32,
}

fn surface_id(ws: &WinSurface) -> u32 {
    ws as *const WinSurface as u32
}

// ---------------------------------------------------------------------------
// Per-tab state
// ---------------------------------------------------------------------------

struct TabState {
    url_text: String,
    url_cursor: usize,
    url_focused: bool,
    current_url: Option<http::Url>,
    page_title: String,
    scroll_y: i32,
    total_height: i32,
    history: Vec<String>,
    history_pos: usize,
    status_text: String,
    page_dom: Option<dom::Dom>,
    page_styles: Vec<style::ComputedStyle>,
    page_layout: Option<layout::LayoutBox>,
    images: paint::ImageCache,
    hover_link: Option<String>,
    // Full-page off-screen pixel buffer (rendered once, blitted on scroll)
    page_pixels: Vec<u32>,
    page_pixels_w: u32,
    page_pixels_h: u32,
}

impl TabState {
    fn new() -> Self {
        TabState {
            url_text: String::new(),
            url_cursor: 0,
            url_focused: true,
            current_url: None,
            page_title: String::new(),
            scroll_y: 0,
            total_height: 0,
            history: Vec::new(),
            history_pos: 0,
            status_text: String::from("Ready"),
            page_dom: None,
            page_styles: Vec::new(),
            page_layout: None,
            images: paint::ImageCache { entries: Vec::new() },
            hover_link: None,
            page_pixels: Vec::new(),
            page_pixels_w: 0,
            page_pixels_h: 0,
        }
    }

    fn tab_label(&self) -> &str {
        if !self.page_title.is_empty() {
            &self.page_title
        } else if !self.url_text.is_empty() {
            &self.url_text
        } else {
            "New Tab"
        }
    }

    fn can_go_back(&self) -> bool {
        self.history_pos > 0
    }

    fn can_go_forward(&self) -> bool {
        self.history_pos + 1 < self.history.len()
    }

    fn viewport_height(&self, win_h: u32) -> i32 {
        (win_h as i32 - CHROME_H - STATUS_H).max(0)
    }

    fn clamp_scroll(&mut self, win_h: u32) {
        let vh = self.viewport_height(win_h);
        let max = (self.total_height - vh).max(0);
        if self.scroll_y < 0 { self.scroll_y = 0; }
        if self.scroll_y > max { self.scroll_y = max; }
    }
}

// ---------------------------------------------------------------------------
// Browser state (all tabs)
// ---------------------------------------------------------------------------

struct Browser {
    tabs: Vec<TabState>,
    active_tab: usize,
    cookies: http::CookieJar,
    needs_redraw: bool,
}

impl Browser {
    fn new() -> Self {
        Browser {
            tabs: vec![TabState::new()],
            active_tab: 0,
            cookies: http::CookieJar::new(),
            needs_redraw: true,
        }
    }

    fn tab(&self) -> &TabState {
        &self.tabs[self.active_tab]
    }

    fn tab_mut(&mut self) -> &mut TabState {
        &mut self.tabs[self.active_tab]
    }

    fn add_tab(&mut self) {
        self.tabs.push(TabState::new());
        self.active_tab = self.tabs.len() - 1;
        self.needs_redraw = true;
    }

    fn close_tab(&mut self, idx: usize) {
        if self.tabs.len() <= 1 { return; } // keep at least one tab
        self.tabs.remove(idx);
        if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len() - 1;
        } else if self.active_tab > idx {
            self.active_tab -= 1;
        }
        self.needs_redraw = true;
    }

    fn switch_tab(&mut self, idx: usize) {
        if idx < self.tabs.len() && idx != self.active_tab {
            self.active_tab = idx;
            self.needs_redraw = true;
        }
    }
}

// ---------------------------------------------------------------------------
// Toolbar icons
// ---------------------------------------------------------------------------

struct NavIcons {
    back: Option<ControlIcon>,
    forward: Option<ControlIcon>,
    reload: Option<ControlIcon>,
    secure: Option<ControlIcon>,
    insecure: Option<ControlIcon>,
}

impl NavIcons {
    fn load() -> Self {
        let sz = 16;
        NavIcons {
            back: load_control_icon("left", sz),
            forward: load_control_icon("right", sz),
            reload: load_control_icon("refresh", sz),
            secure: load_control_icon("secure", sz),
            insecure: load_control_icon("insecure", sz),
        }
    }
}

// ---------------------------------------------------------------------------
// Navigation
// ---------------------------------------------------------------------------

fn navigate(browser: &mut Browser, url_str: &str, win_w: u32) {
    anyos_std::println!("[surf] navigate: {}", url_str);

    let tab = browser.tab_mut();
    tab.status_text = String::from("Loading...");
    browser.needs_redraw = true;

    let url = match http::parse_url(url_str) {
        Ok(u) => {
            anyos_std::println!("[surf] parsed URL: {}://{}:{}{}", u.scheme, u.host, u.port, u.path);
            u
        }
        Err(_) => {
            anyos_std::println!("[surf] ERROR: invalid URL");
            browser.tab_mut().status_text = String::from("Invalid URL");
            return;
        }
    };

    let response = match http::fetch(&url, &mut browser.cookies) {
        Ok(r) => {
            anyos_std::println!("[surf] fetch OK: status={}, body={} bytes", r.status, r.body.len());
            r
        }
        Err(e) => {
            let msg = match e {
                http::FetchError::InvalidUrl => "Invalid URL",
                http::FetchError::DnsFailure => "DNS lookup failed",
                http::FetchError::ConnectFailure => "Connection failed",
                http::FetchError::SendFailure => "Send failed",
                http::FetchError::NoResponse => "No response",
                http::FetchError::TooManyRedirects => "Too many redirects",
                http::FetchError::HttpsNotSupported => "HTTPS not supported",
            };
            anyos_std::println!("[surf] ERROR: fetch failed: {}", msg);
            browser.tab_mut().status_text = String::from(msg);
            return;
        }
    };

    if response.status < 200 || response.status >= 400 {
        anyos_std::println!("[surf] ERROR: HTTP status {}", response.status);
        browser.tab_mut().status_text = String::from("HTTP error");
        return;
    }

    // Use lossy UTF-8 conversion — many pages use ISO-8859-1 or have stray bytes
    let body_text = String::from_utf8_lossy(&response.body).into_owned();
    anyos_std::println!("[surf] body as text: {} chars", body_text.len());

    anyos_std::println!("[surf] parsing HTML...");
    let dom = html::parse(&body_text);
    anyos_std::println!("[surf] DOM: {} nodes", dom.nodes.len());

    let title = dom.find_title().unwrap_or_else(|| String::from("Untitled"));
    anyos_std::println!("[surf] title: {}", title);

    let mut stylesheets = Vec::new();
    for (i, node) in dom.nodes.iter().enumerate() {
        if let dom::NodeType::Element { tag: dom::Tag::Style, .. } = &node.node_type {
            let css_text = dom.text_content(i);
            anyos_std::println!("[surf] found <style> block: {} chars", css_text.len());
            stylesheets.push(css::parse_stylesheet(&css_text));
        }
    }
    anyos_std::println!("[surf] {} stylesheets parsed", stylesheets.len());

    anyos_std::println!("[surf] resolving styles...");
    let styles = style::resolve_styles(&dom, &stylesheets);
    anyos_std::println!("[surf] styles resolved for {} nodes", styles.len());

    let viewport_w = (win_w as i32 - SCROLLBAR_W as i32).max(100);
    anyos_std::println!("[surf] layout: viewport_w={}", viewport_w);
    let layout_root = layout::layout(&dom, &styles, viewport_w);
    let total_h = paint::total_height(&layout_root);
    anyos_std::println!("[surf] layout done: total_height={}", total_h);

    let mut images = paint::ImageCache { entries: Vec::new() };
    collect_and_fetch_images(&dom, &url, &mut images, &mut browser.cookies);
    anyos_std::println!("[surf] images: {} loaded", images.entries.len());

    // Pre-render full page into off-screen buffer
    let page_buf_w = win_w;
    let page_buf_h = (total_h as u32).max(1);
    anyos_std::println!("[surf] allocating page buffer: {}x{} ({} KB)",
        page_buf_w, page_buf_h,
        (page_buf_w as usize * page_buf_h as usize * 4) / 1024);
    let mut page_pixels = vec![0u32; (page_buf_w as usize) * (page_buf_h as usize)];
    paint::paint(&layout_root, &mut page_pixels, page_buf_w, page_buf_h, 0, &images);
    anyos_std::println!("[surf] page rendered to off-screen buffer");

    let url_string = format_url(&url);
    let tab = browser.tab_mut();

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
    tab.page_dom = Some(dom);
    tab.page_styles = styles;
    tab.page_layout = Some(layout_root);
    tab.total_height = total_h;
    tab.images = images;
    tab.page_pixels = page_pixels;
    tab.page_pixels_w = page_buf_w;
    tab.page_pixels_h = page_buf_h;
    tab.scroll_y = 0;
    tab.hover_link = None;
    tab.url_text = url_string;
    tab.url_cursor = tab.url_text.len();
    tab.status_text = String::from("Done");
    browser.needs_redraw = true;
    anyos_std::println!("[surf] navigate complete, needs_redraw=true");
}

fn collect_and_fetch_images(
    dom: &dom::Dom,
    base_url: &http::Url,
    cache: &mut paint::ImageCache,
    cookies: &mut http::CookieJar,
) {
    for (_i, node) in dom.nodes.iter().enumerate() {
        if let dom::NodeType::Element { tag: dom::Tag::Img, .. } = &node.node_type {
            if let Some(src) = dom.attr(_i, "src") {
                anyos_std::println!("[surf] fetching image: {}", src);
                let img_url = http::resolve_url(base_url, src);
                match http::fetch(&img_url, cookies) {
                    Ok(resp) => {
                        if let Some(info) = libimage_client::probe(&resp.body) {
                            anyos_std::println!("[surf] image {}x{}, scratch={}", info.width, info.height, info.scratch_needed);
                            let w = info.width as usize;
                            let h = info.height as usize;
                            let mut pixels = vec![0u32; w * h];
                            let mut scratch = vec![0u8; info.scratch_needed as usize];
                            if libimage_client::decode(&resp.body, &mut pixels, &mut scratch).is_ok() {
                                anyos_std::println!("[surf] image decoded OK: {}", src);
                                cache.entries.push(paint::ImageEntry {
                                    src: String::from(src),
                                    width: info.width,
                                    height: info.height,
                                    pixels,
                                });
                            } else {
                                anyos_std::println!("[surf] image decode FAILED: {}", src);
                            }
                        } else {
                            anyos_std::println!("[surf] image probe failed (unknown format): {} ({} bytes)", src, resp.body.len());
                        }
                    }
                    Err(_) => {
                        anyos_std::println!("[surf] image fetch failed: {}", src);
                    }
                }
            }
        }
    }
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
// Rendering
// ---------------------------------------------------------------------------

/// Alpha-blend icon pixels onto a raw surface at (ix, iy).
fn blit_icon_alpha_raw(surface: *mut u32, stride: u32, ix: i32, iy: i32, pixels: &[u32], iw: u32, ih: u32) {
    for row in 0..ih as i32 {
        let py = iy + row;
        if py < 0 { continue; }
        for col in 0..iw as i32 {
            let px_x = ix + col;
            if px_x < 0 { continue; }
            let src_idx = (row as u32 * iw + col as u32) as usize;
            if src_idx >= pixels.len() { break; }
            let src = pixels[src_idx];
            let a = (src >> 24) & 0xFF;
            if a == 0 { continue; } // fully transparent — skip
            let dst_idx = (py as u32 * stride + px_x as u32) as usize;
            unsafe {
                let dst = *surface.add(dst_idx);
                if a >= 255 {
                    *surface.add(dst_idx) = src; // fully opaque
                } else {
                    let inv = 255 - a;
                    let r = (((src >> 16) & 0xFF) * a + ((dst >> 16) & 0xFF) * inv) / 255;
                    let g = (((src >> 8) & 0xFF) * a + ((dst >> 8) & 0xFF) * inv) / 255;
                    let b = ((src & 0xFF) * a + (dst & 0xFF) * inv) / 255;
                    *surface.add(dst_idx) = 0xFF000000 | (r << 16) | (g << 8) | b;
                }
            }
        }
    }
}

/// Tint icon pixels: replace RGB with tint color, preserve alpha. If `dimmed`, reduce alpha.
fn tint_icon(pixels: &[u32], tint: u32, dimmed: bool) -> Vec<u32> {
    let tr = (tint >> 16) & 0xFF;
    let tg = (tint >> 8) & 0xFF;
    let tb = tint & 0xFF;
    let mut out = vec![0u32; pixels.len()];
    for i in 0..pixels.len() {
        let a = (pixels[i] >> 24) & 0xFF;
        if a == 0 { continue; }
        let a = if dimmed { a / 3 } else { a };
        out[i] = (a << 24) | (tr << 16) | (tg << 8) | tb;
    }
    out
}

/// Render an icon centered inside a button area, tinted white. If `dimmed`, reduces alpha.
fn render_btn_icon(surface: *mut u32, stride: u32, bx: i32, by: i32, bw: i32, bh: i32, icon: &Option<ControlIcon>, dimmed: bool) {
    if let Some(ref ic) = icon {
        let ix = bx + (bw - ic.width as i32) / 2;
        let iy = by + (bh - ic.height as i32) / 2;
        let tinted = tint_icon(&ic.pixels, 0xFFFFFFFF, dimmed);
        blit_icon_alpha_raw(surface, stride, ix, iy, &tinted, ic.width, ic.height);
    }
}

fn render(client: &CompositorClient, win: &WindowHandle, browser: &Browser, icons: &NavIcons) {
    let w = win.width;
    let h = win.height;
    let surface = win.surface();
    let pixel_count = (w * h) as usize;

    unsafe {
        let slice = core::slice::from_raw_parts_mut(surface, pixel_count);
        for px in slice.iter_mut() { *px = BG; }
    }

    let ws = WinSurface { pixels: surface, width: w, height: h };
    let wid = surface_id(&ws);
    let tab = browser.tab();

    // -- Toolbar background --
    fill_rect_raw(surface, w, 0, 0, w as i32, TOOLBAR_H, TOOLBAR_BG);

    // -- Back button --
    let back_style = if tab.can_go_back() { ButtonStyle::Plain } else { ButtonStyle::Default };
    let back_state = if tab.can_go_back() { ButtonState::Normal } else { ButtonState::Disabled };
    button(wid, 8, BTN_Y, BTN_W as u32, BTN_H as u32, "", back_style, back_state);
    render_btn_icon(surface, w, 8, BTN_Y, BTN_W, BTN_H, &icons.back, !tab.can_go_back());

    // -- Forward button --
    let fwd_style = if tab.can_go_forward() { ButtonStyle::Plain } else { ButtonStyle::Default };
    let fwd_state = if tab.can_go_forward() { ButtonState::Normal } else { ButtonState::Disabled };
    button(wid, 42, BTN_Y, BTN_W as u32, BTN_H as u32, "", fwd_style, fwd_state);
    render_btn_icon(surface, w, 42, BTN_Y, BTN_W, BTN_H, &icons.forward, !tab.can_go_forward());

    // -- Reload button --
    button(wid, 76, BTN_Y, BTN_W as u32, BTN_H as u32, "", ButtonStyle::Plain, ButtonState::Normal);
    render_btn_icon(surface, w, 76, BTN_Y, BTN_W, BTN_H, &icons.reload, false);

    // -- URL bar --
    let url_w = (w as i32 - URL_X - 8).max(60) as u32;
    textfield(wid, URL_X, URL_Y, url_w, URL_H, &tab.url_text, "Enter URL...", tab.url_cursor as u32, tab.url_focused);

    // -- Toolbar separator --
    fill_rect_raw(surface, w, 0, TOOLBAR_H - 1, w as i32, 1, SEPARATOR);

    // -- Tab bar --
    fill_rect_raw(surface, w, 0, TOOLBAR_H, w as i32, TAB_BAR_H, TAB_BAR_BG);
    render_tab_bar(surface, wid, w, browser);
    fill_rect_raw(surface, w, 0, CHROME_H - 1, w as i32, 1, SEPARATOR);

    // -- Content area: blit from pre-rendered full-page buffer --
    let viewport_h = tab.viewport_height(h);
    if tab.page_pixels_w > 0 && tab.page_pixels_h > 0 && viewport_h > 0 {
        let offset = (CHROME_H as u32 * w) as usize;
        let src_y = tab.scroll_y.max(0) as u32;
        let blit_w = tab.page_pixels_w.min(w) as usize;

        for dy in 0..viewport_h as u32 {
            let src_row = src_y + dy;
            if src_row >= tab.page_pixels_h { break; }
            let src_off = (src_row as usize) * (tab.page_pixels_w as usize);
            let dst_off = offset + (dy as usize) * (w as usize);
            unsafe {
                core::ptr::copy_nonoverlapping(
                    tab.page_pixels.as_ptr().add(src_off),
                    surface.add(dst_off),
                    blit_w,
                );
            }
        }

        // -- Scrollbar --
        if tab.total_height > viewport_h {
            let sb_x = w as i32 - SCROLLBAR_W as i32;
            scrollbar(wid, sb_x, CHROME_H, SCROLLBAR_W, viewport_h as u32, tab.total_height as u32, tab.scroll_y as u32);
        }
    }

    // -- Status bar --
    let status_y = (h as i32 - STATUS_H).max(CHROME_H);
    fill_rect_raw(surface, w, 0, status_y, w as i32, STATUS_H, STATUS_BG);
    fill_rect_raw(surface, w, 0, status_y, w as i32, 1, SEPARATOR);
    label(wid, 8, status_y + 4, &tab.status_text, STATUS_TEXT, FontSize::Small, TextAlign::Left);

    client.present(win);
}

fn render_tab_bar(surface: *mut u32, wid: u32, win_w: u32, browser: &Browser) {
    let tab_count = browser.tabs.len();
    let available_w = win_w as i32 - TAB_NEW_BTN_W - 8; // leave room for + button
    let tab_w = (available_w / tab_count.max(1) as i32).clamp(TAB_MIN_W, TAB_MAX_W);
    let ty = TOOLBAR_H;

    for (i, tab) in browser.tabs.iter().enumerate() {
        let tx = i as i32 * tab_w;
        if tx + tab_w > available_w { break; }

        let is_active = i == browser.active_tab;
        let bg = if is_active { TAB_ACTIVE_BG } else { TAB_BG };
        let text_color = if is_active { TAB_ACTIVE_TEXT } else { TAB_TEXT };

        // Tab background
        fill_rect_raw(surface, win_w, tx, ty + 2, tab_w - 1, TAB_BAR_H - 3, bg);

        // Tab label (truncated)
        let label_text = tab.tab_label();
        let max_chars = ((tab_w - 24) / 7).max(1) as usize; // ~7px per char
        let display = if label_text.len() > max_chars {
            &label_text[..max_chars]
        } else {
            label_text
        };
        label(wid, tx + 8, ty + 8, display, text_color, FontSize::Small, TextAlign::Left);

        // Close button "x" on each tab (except if only 1 tab)
        if tab_count > 1 {
            label(wid, tx + tab_w - 16, ty + 8, "x", TAB_CLOSE_TEXT, FontSize::Small, TextAlign::Left);
        }

        // Separator between tabs
        if i + 1 < tab_count {
            fill_rect_raw(surface, win_w, tx + tab_w - 1, ty + 4, 1, TAB_BAR_H - 8, SEPARATOR);
        }
    }

    // "+" button for new tab
    let plus_x = (tab_count as i32 * tab_w).min(available_w);
    button(wid, plus_x + 4, ty + 2, TAB_NEW_BTN_W as u32 - 4, (TAB_BAR_H - 4) as u32, "+", ButtonStyle::Plain, ButtonState::Normal);
}

fn fill_rect_raw(surface: *mut u32, stride: u32, x: i32, y: i32, w: i32, h: i32, color: u32) {
    if w <= 0 || h <= 0 { return; }
    let stride = stride as usize;
    for row in 0..h {
        let py = (y + row) as usize;
        for col in 0..w {
            let px = (x + col) as usize;
            let idx = py * stride + px;
            unsafe { *surface.add(idx) = color; }
        }
    }
}

// ---------------------------------------------------------------------------
// Event handling
// ---------------------------------------------------------------------------

fn handle_url_key(tab: &mut TabState, key: u32, ch: u32) -> bool {
    match key {
        KEY_BACKSPACE => {
            if tab.url_cursor > 0 {
                tab.url_text.remove(tab.url_cursor - 1);
                tab.url_cursor -= 1;
            }
        }
        KEY_DELETE => {
            if tab.url_cursor < tab.url_text.len() {
                tab.url_text.remove(tab.url_cursor);
            }
        }
        KEY_LEFT => {
            if tab.url_cursor > 0 { tab.url_cursor -= 1; }
        }
        KEY_RIGHT => {
            if tab.url_cursor < tab.url_text.len() { tab.url_cursor += 1; }
        }
        KEY_HOME => { tab.url_cursor = 0; }
        KEY_END => { tab.url_cursor = tab.url_text.len(); }
        KEY_ENTER => { return true; }
        KEY_ESCAPE => { tab.url_focused = false; }
        _ => {
            if ch >= 0x20 && ch <= 0x7E {
                tab.url_text.insert(tab.url_cursor, ch as u8 as char);
                tab.url_cursor += 1;
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

fn main() {
    anyos_std::println!("[surf] starting Surf browser v1.0");

    let client = match CompositorClient::init() {
        Some(c) => c,
        None => {
            anyos_std::println!("[surf] failed to connect to compositor");
            return;
        }
    };

    let mut win = match client.create_window(800, 600, 0) {
        Some(w) => w,
        None => {
            anyos_std::println!("surf: failed to create window");
            return;
        }
    };

    let (scr_w, scr_h) = client.screen_size();
    let wx = (scr_w.saturating_sub(800)) / 2;
    let wy = (scr_h.saturating_sub(600)) / 2;
    client.move_window(&win, wx as i32, wy as i32);
    client.set_title(&win, "Surf");

    let mut mb = MenuBarBuilder::new()
        .menu("File")
            .item(MENU_NEW_TAB, "New Tab", 0)
            .item(MENU_CLOSE_TAB, "Close Tab", 0)
            .separator()
            .item(MENU_OPEN_URL, "Open URL", 0)
            .separator()
            .item(MENU_QUIT, "Quit", 0)
        .end_menu()
        .menu("View")
            .item(MENU_RELOAD, "Reload", 0)
        .end_menu()
        .menu("Help")
            .item(MENU_ABOUT, "About Surf", 0)
        .end_menu();
    let menu_data = mb.build();
    client.set_menu(&win, menu_data);

    let mut browser = Browser::new();
    let icons = NavIcons::load();
    anyos_std::println!("[surf] icons loaded: back={} fwd={} reload={} secure={} insecure={}",
        icons.back.is_some(), icons.forward.is_some(), icons.reload.is_some(),
        icons.secure.is_some(), icons.insecure.is_some());

    // Check for URL argument
    let mut args_buf = [0u8; 256];
    let arg_url = anyos_std::process::args(&mut args_buf).trim();
    anyos_std::println!("[surf] args: '{}'", arg_url);
    if !arg_url.is_empty() {
        browser.tab_mut().url_text = String::from(arg_url);
        browser.tab_mut().url_cursor = browser.tab().url_text.len();
        browser.tab_mut().url_focused = false;
        let url_copy = browser.tab().url_text.clone();
        navigate(&mut browser, &url_copy, win.width);
    }

    update_title(&client, &win, &browser);

    // Main event loop
    loop {
        while let Some(evt) = client.poll_event(&win) {
            match evt.event_type {
                EVT_WINDOW_CLOSE => {
                    client.destroy_window(&win);
                    return;
                }

                EVT_RESIZE => {
                    let new_w = evt.arg1;
                    let new_h = evt.arg2;
                    if new_w != win.width || new_h != win.height {
                        if client.resize_window(&mut win, new_w, new_h) {
                            if browser.tab().page_dom.is_some() {
                                relayout(&mut browser, new_w);
                            }
                            browser.tab_mut().clamp_scroll(new_h);
                            browser.needs_redraw = true;
                        }
                    }
                }

                EVT_KEY_DOWN => {
                    let key = evt.arg1;
                    let ch = evt.arg2;
                    let modifiers = evt.arg3;
                    let ctrl = modifiers & 2 != 0; // bit 1 = ctrl

                    // Ctrl+T = new tab
                    if ctrl && (ch == b't' as u32 || ch == b'T' as u32) {
                        browser.add_tab();
                        update_title(&client, &win, &browser);
                        continue;
                    }

                    // Ctrl+W = close tab
                    if ctrl && (ch == b'w' as u32 || ch == b'W' as u32) {
                        if browser.tabs.len() > 1 {
                            let idx = browser.active_tab;
                            browser.close_tab(idx);
                            update_title(&client, &win, &browser);
                        } else {
                            client.destroy_window(&win);
                            return;
                        }
                        continue;
                    }

                    // Ctrl+L = focus URL bar
                    if ctrl && (ch == b'l' as u32 || ch == b'L' as u32) {
                        browser.tab_mut().url_focused = true;
                        browser.tab_mut().url_cursor = browser.tab().url_text.len();
                        browser.needs_redraw = true;
                        continue;
                    }

                    if browser.tab().url_focused {
                        let should_navigate = handle_url_key(browser.tab_mut(), key, ch);
                        browser.needs_redraw = true;
                        if should_navigate {
                            let url_copy = browser.tab().url_text.clone();
                            navigate(&mut browser, &url_copy, win.width);
                            browser.tab_mut().url_focused = false;
                            update_title(&client, &win, &browser);
                        }
                    } else {
                        match key {
                            KEY_ESCAPE => {
                                browser.tab_mut().url_focused = true;
                                browser.tab_mut().url_cursor = browser.tab().url_text.len();
                                browser.needs_redraw = true;
                            }
                            _ => {}
                        }
                    }
                }

                EVT_MOUSE_DOWN => {
                    let mx = evt.arg1 as i32;
                    let my = evt.arg2 as i32;

                    if my < TOOLBAR_H {
                        // Toolbar click
                        handle_toolbar_click(&mut browser, &client, &win, mx, my);
                    } else if my < CHROME_H {
                        // Tab bar click
                        handle_tab_bar_click(&mut browser, &client, &win, mx, my);
                    } else if my >= (win.height as i32 - STATUS_H) {
                        browser.tab_mut().url_focused = false;
                        browser.needs_redraw = true;
                    } else {
                        // Content area click
                        browser.tab_mut().url_focused = false;
                        let content_y = my - CHROME_H + browser.tab().scroll_y;
                        if let Some(ref root) = browser.tab().page_layout {
                            if let Some(link) = paint::hit_test(root, mx, content_y, 0) {
                                let resolved = if let Some(ref base) = browser.tab().current_url {
                                    format_url(&http::resolve_url(base, &link))
                                } else {
                                    link
                                };
                                navigate(&mut browser, &resolved, win.width);
                                update_title(&client, &win, &browser);
                            }
                        }
                        browser.needs_redraw = true;
                    }
                }

                EVT_MOUSE_SCROLL => {
                    let dz = evt.arg1 as i32;
                    browser.tab_mut().scroll_y += dz * 40;
                    browser.tab_mut().clamp_scroll(win.height);
                    browser.needs_redraw = true;
                }

                EVT_MOUSE_MOVE => {
                    let mx = evt.arg1 as i32;
                    let my = evt.arg2 as i32;

                    if my > CHROME_H && my < (win.height as i32 - STATUS_H) {
                        let content_y = my - CHROME_H + browser.tab().scroll_y;
                        if let Some(ref root) = browser.tab().page_layout {
                            let link = paint::hit_test(root, mx, content_y, 0);
                            if link != browser.tab().hover_link {
                                let new_status = if let Some(ref url) = link {
                                    url.clone()
                                } else {
                                    String::from("Done")
                                };
                                browser.tab_mut().hover_link = link;
                                browser.tab_mut().status_text = new_status;
                                browser.needs_redraw = true;
                            }
                        }
                    }
                }

                EVT_MENU_ITEM => {
                    let item_id = evt.arg1;
                    match item_id {
                        MENU_QUIT => {
                            client.destroy_window(&win);
                            return;
                        }
                        MENU_NEW_TAB => {
                            browser.add_tab();
                            update_title(&client, &win, &browser);
                        }
                        MENU_CLOSE_TAB => {
                            if browser.tabs.len() > 1 {
                                let idx = browser.active_tab;
                                browser.close_tab(idx);
                                update_title(&client, &win, &browser);
                            }
                        }
                        MENU_OPEN_URL => {
                            browser.tab_mut().url_focused = true;
                            browser.tab_mut().url_text.clear();
                            browser.tab_mut().url_cursor = 0;
                            browser.needs_redraw = true;
                        }
                        MENU_RELOAD => {
                            if let Some(ref url) = browser.tab().current_url {
                                let url_str = format_url(url);
                                navigate(&mut browser, &url_str, win.width);
                                update_title(&client, &win, &browser);
                            }
                        }
                        MENU_ABOUT => {
                            browser.tab_mut().status_text = String::from("Surf 1.0 - Web Browser for anyOS");
                            browser.needs_redraw = true;
                        }
                        _ => {}
                    }
                }

                _ => {}
            }
        }

        if browser.needs_redraw {
            render(&client, &win, &browser, &icons);
            browser.needs_redraw = false;
        }

        anyos_std::process::sleep(16);
    }
}

fn handle_toolbar_click(browser: &mut Browser, client: &CompositorClient, win: &WindowHandle, mx: i32, my: i32) {
    if button_hit_test(8, BTN_Y, BTN_W as u32, BTN_H as u32, mx, my) {
        // Back
        if browser.tab().can_go_back() {
            let new_pos = browser.tab().history_pos - 1;
            browser.tab_mut().history_pos = new_pos;
            let url = browser.tab().history[new_pos].clone();
            navigate(browser, &url, win.width);
            update_title(client, win, browser);
        }
    } else if button_hit_test(42, BTN_Y, BTN_W as u32, BTN_H as u32, mx, my) {
        // Forward
        if browser.tab().can_go_forward() {
            let new_pos = browser.tab().history_pos + 1;
            browser.tab_mut().history_pos = new_pos;
            let url = browser.tab().history[new_pos].clone();
            navigate(browser, &url, win.width);
            update_title(client, win, browser);
        }
    } else if button_hit_test(76, BTN_Y, BTN_W as u32, BTN_H as u32, mx, my) {
        // Reload
        if let Some(ref url) = browser.tab().current_url {
            let url_str = format_url(url);
            navigate(browser, &url_str, win.width);
            update_title(client, win, browser);
        }
    } else {
        // URL bar click
        let url_w = (win.width as i32 - URL_X - 8).max(60);
        if mx >= URL_X && mx < URL_X + url_w && my >= URL_Y && my < URL_Y + URL_H as i32 {
            browser.tab_mut().url_focused = true;
            let click_offset = mx - URL_X - 4;
            let char_w = 7;
            let pos = (click_offset / char_w).max(0) as usize;
            browser.tab_mut().url_cursor = pos.min(browser.tab().url_text.len());
        } else {
            browser.tab_mut().url_focused = false;
        }
        browser.needs_redraw = true;
    }
}

fn handle_tab_bar_click(browser: &mut Browser, client: &CompositorClient, win: &WindowHandle, mx: i32, my: i32) {
    let tab_count = browser.tabs.len();
    let available_w = win.width as i32 - TAB_NEW_BTN_W - 8;
    let tab_w = (available_w / tab_count.max(1) as i32).clamp(TAB_MIN_W, TAB_MAX_W);

    // Check "+" button
    let plus_x = (tab_count as i32 * tab_w).min(available_w);
    if mx >= plus_x && mx < plus_x + TAB_NEW_BTN_W {
        browser.add_tab();
        update_title(client, win, browser);
        return;
    }

    // Check tab clicks
    let tab_idx = mx / tab_w;
    if tab_idx >= 0 && (tab_idx as usize) < tab_count {
        let idx = tab_idx as usize;
        let tab_start = idx as i32 * tab_w;

        // Close button area (last 16px of tab)
        if tab_count > 1 && mx >= tab_start + tab_w - 16 {
            browser.close_tab(idx);
            update_title(client, win, browser);
        } else {
            browser.switch_tab(idx);
            update_title(client, win, browser);
        }
    }
}

fn update_title(client: &CompositorClient, win: &WindowHandle, browser: &Browser) {
    let tab = browser.tab();
    if tab.page_title.is_empty() {
        client.set_title(win, "Surf");
    } else {
        let mut title = tab.page_title.clone();
        title.push_str(" - Surf");
        client.set_title(win, &title);
    }
}

fn relayout(browser: &mut Browser, win_w: u32) {
    let tab = browser.tab_mut();
    if let Some(ref dom) = tab.page_dom {
        let viewport_w = (win_w as i32 - SCROLLBAR_W as i32).max(100);
        let root = layout::layout(dom, &tab.page_styles, viewport_w);
        tab.total_height = paint::total_height(&root);

        // Re-render full page into off-screen buffer
        let page_buf_w = win_w;
        let page_buf_h = (tab.total_height as u32).max(1);
        tab.page_pixels = vec![0u32; (page_buf_w as usize) * (page_buf_h as usize)];
        paint::paint(&root, &mut tab.page_pixels, page_buf_w, page_buf_h, 0, &tab.images);
        tab.page_pixels_w = page_buf_w;
        tab.page_pixels_h = page_buf_h;

        tab.page_layout = Some(root);
    }
}
