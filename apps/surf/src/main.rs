// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Surf -- a web browser for anyOS.
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
const STATUS_H: i32 = 24;
const SCROLLBAR_W: u32 = 8;
const BTN_W: i32 = 32;
const BTN_H: i32 = 28;
const BTN_Y: i32 = 6;
const URL_X: i32 = 110;
const URL_H: u32 = 28;
const URL_Y: i32 = 6;

// Colors
const BG: u32 = 0xFF1E1E1E;
const TOOLBAR_BG: u32 = 0xFF2A2A2C;
const STATUS_BG: u32 = 0xFF252525;
const STATUS_TEXT: u32 = 0xFF969696;
const SEPARATOR: u32 = 0xFF3D3D3D;

// Menu item IDs
const MENU_OPEN_URL: u32 = 1;
const MENU_QUIT: u32 = 2;
const MENU_RELOAD: u32 = 10;
const MENU_ABOUT: u32 = 20;

// ---------------------------------------------------------------------------
// WinSurface trick for uisys DLL rendering on raw surface
// ---------------------------------------------------------------------------

/// Layout-compatible with the compositor's WinSurface struct.
/// When cast to u32 and >= 0x0100_0000, uisys treats it as a direct pixel
/// buffer surface rather than a kernel window ID.
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
// Browser state
// ---------------------------------------------------------------------------

struct BrowserState {
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
    needs_redraw: bool,
    // Page data
    page_dom: Option<dom::Dom>,
    page_styles: Vec<style::ComputedStyle>,
    page_layout: Option<layout::LayoutBox>,
    images: paint::ImageCache,
    hover_link: Option<String>,
}

impl BrowserState {
    fn new() -> Self {
        BrowserState {
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
            needs_redraw: true,
            page_dom: None,
            page_styles: Vec::new(),
            page_layout: None,
            images: paint::ImageCache { entries: Vec::new() },
            hover_link: None,
        }
    }

    fn can_go_back(&self) -> bool {
        self.history_pos > 0
    }

    fn can_go_forward(&self) -> bool {
        self.history_pos + 1 < self.history.len()
    }

    fn viewport_height(&self, win_h: u32) -> i32 {
        (win_h as i32 - TOOLBAR_H - STATUS_H).max(0)
    }

    fn clamp_scroll(&mut self, win_h: u32) {
        let vh = self.viewport_height(win_h);
        let max = (self.total_height - vh).max(0);
        if self.scroll_y < 0 { self.scroll_y = 0; }
        if self.scroll_y > max { self.scroll_y = max; }
    }
}

// ---------------------------------------------------------------------------
// Navigation
// ---------------------------------------------------------------------------

fn navigate(state: &mut BrowserState, url_str: &str, win_w: u32) {
    state.status_text = String::from("Loading...");
    state.needs_redraw = true;

    // Parse URL
    let url = match http::parse_url(url_str) {
        Ok(u) => u,
        Err(_) => {
            state.status_text = String::from("Invalid URL");
            return;
        }
    };

    // Fetch
    let response = match http::fetch(&url) {
        Ok(r) => r,
        Err(_) => {
            state.status_text = String::from("Failed to connect");
            return;
        }
    };

    if response.status < 200 || response.status >= 400 {
        state.status_text = String::from("HTTP error");
        return;
    }

    // Parse HTML body
    let body_str = core::str::from_utf8(&response.body).unwrap_or("");
    let dom = html::parse(body_str);

    // Extract <title>
    let title = dom.find_title().unwrap_or_else(|| String::from("Untitled"));

    // Collect <style> blocks and parse CSS
    let mut stylesheets = Vec::new();
    for (i, node) in dom.nodes.iter().enumerate() {
        if let dom::NodeType::Element { tag: dom::Tag::Style, .. } = &node.node_type {
            let css_text = dom.text_content(i);
            stylesheets.push(css::parse_stylesheet(&css_text));
        }
    }

    // Resolve styles
    let styles = style::resolve_styles(&dom, &stylesheets);

    // Layout
    let viewport_w = (win_w as i32 - SCROLLBAR_W as i32).max(100);
    let layout_root = layout::layout(&dom, &styles, viewport_w);
    let total_h = paint::total_height(&layout_root);

    // Collect and fetch images
    let mut images = paint::ImageCache { entries: Vec::new() };
    collect_and_fetch_images(&dom, &url, &mut images);

    // Update history
    let url_string = format_url(&url);
    if state.history.is_empty() || state.history_pos >= state.history.len()
        || state.history[state.history_pos] != url_string
    {
        // Trim forward history
        if state.history_pos + 1 < state.history.len() {
            state.history.truncate(state.history_pos + 1);
        }
        state.history.push(url_string.clone());
        state.history_pos = state.history.len() - 1;
    }

    // Update state
    state.current_url = Some(url);
    state.page_title = title;
    state.page_dom = Some(dom);
    state.page_styles = styles;
    state.page_layout = Some(layout_root);
    state.total_height = total_h;
    state.images = images;
    state.scroll_y = 0;
    state.hover_link = None;
    state.url_text = url_string;
    state.url_cursor = state.url_text.len();
    state.status_text = String::from("Done");
    state.needs_redraw = true;
}

fn collect_and_fetch_images(dom: &dom::Dom, base_url: &http::Url, cache: &mut paint::ImageCache) {
    for (i, node) in dom.nodes.iter().enumerate() {
        if let dom::NodeType::Element { tag: dom::Tag::Img, .. } = &node.node_type {
            if let Some(src) = dom.attr(i, "src") {
                let img_url = http::resolve_url(base_url, src);
                match http::fetch(&img_url) {
                    Ok(resp) => {
                        if let Some(info) = libimage_client::probe(&resp.body) {
                            let w = info.width as usize;
                            let h = info.height as usize;
                            let mut pixels = vec![0u32; w * h];
                            let mut scratch = vec![0u8; info.scratch_needed as usize];
                            if libimage_client::decode(&resp.body, &mut pixels, &mut scratch).is_ok() {
                                cache.entries.push(paint::ImageEntry {
                                    src: String::from(src),
                                    width: info.width,
                                    height: info.height,
                                    pixels,
                                });
                            }
                        }
                    }
                    Err(_) => {}
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
    if url.port != 80 {
        s.push(':');
        push_u32(&mut s, url.port as u32);
    }
    s.push_str(&url.path);
    s
}

fn push_u32(s: &mut String, val: u32) {
    if val >= 10 {
        push_u32(s, val / 10);
    }
    s.push((b'0' + (val % 10) as u8) as char);
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn render(
    client: &CompositorClient,
    win: &WindowHandle,
    state: &BrowserState,
) {
    let w = win.width;
    let h = win.height;
    let surface = win.surface();
    let pixel_count = (w * h) as usize;

    // Clear entire surface
    unsafe {
        let slice = core::slice::from_raw_parts_mut(surface, pixel_count);
        for px in slice.iter_mut() {
            *px = BG;
        }
    }

    let ws = WinSurface { pixels: surface, width: w, height: h };
    let wid = surface_id(&ws);

    // -- Toolbar background --
    fill_rect_raw(surface, w, 0, 0, w as i32, TOOLBAR_H, TOOLBAR_BG);

    // -- Back button --
    let back_style = if state.can_go_back() { ButtonStyle::Plain } else { ButtonStyle::Default };
    let back_state = if state.can_go_back() { ButtonState::Normal } else { ButtonState::Disabled };
    button(wid, 8, BTN_Y, BTN_W as u32, BTN_H as u32, "<", back_style, back_state);

    // -- Forward button --
    let fwd_style = if state.can_go_forward() { ButtonStyle::Plain } else { ButtonStyle::Default };
    let fwd_state = if state.can_go_forward() { ButtonState::Normal } else { ButtonState::Disabled };
    button(wid, 42, BTN_Y, BTN_W as u32, BTN_H as u32, ">", fwd_style, fwd_state);

    // -- Reload button --
    button(wid, 76, BTN_Y, BTN_W as u32, BTN_H as u32, "R", ButtonStyle::Plain, ButtonState::Normal);

    // -- URL bar --
    let url_w = (w as i32 - URL_X - 8).max(60) as u32;
    textfield(wid, URL_X, URL_Y, url_w, URL_H, &state.url_text, "Enter URL...", state.url_cursor as u32, state.url_focused);

    // -- Toolbar separator --
    fill_rect_raw(surface, w, 0, TOOLBAR_H - 1, w as i32, 1, SEPARATOR);

    // -- Content area: paint HTML --
    let viewport_h = state.viewport_height(h);
    if let Some(ref root) = state.page_layout {
        let content_w = (w as i32 - SCROLLBAR_W as i32).max(0) as u32;

        if content_w > 0 && viewport_h > 0 {
            // Create a sub-slice of the surface starting at the toolbar row.
            // paint::paint draws into this buffer treating row 0 as the top
            // of the content area.
            let offset = (TOOLBAR_H as u32 * w) as usize;
            let len = (viewport_h as u32 * w) as usize;
            let content_pixels = unsafe {
                core::slice::from_raw_parts_mut(surface.add(offset), len)
            };
            paint::paint(
                root,
                content_pixels,
                w,
                viewport_h as u32,
                state.scroll_y,
                &state.images,
            );
        }

        // -- Scrollbar --
        if state.total_height > viewport_h {
            let sb_x = w as i32 - SCROLLBAR_W as i32;
            scrollbar(
                wid,
                sb_x,
                TOOLBAR_H,
                SCROLLBAR_W,
                viewport_h as u32,
                state.total_height as u32,
                state.scroll_y as u32,
            );
        }
    }

    // -- Status bar --
    let status_y = (h as i32 - STATUS_H).max(TOOLBAR_H);
    fill_rect_raw(surface, w, 0, status_y, w as i32, STATUS_H, STATUS_BG);
    fill_rect_raw(surface, w, 0, status_y, w as i32, 1, SEPARATOR);
    label(wid, 8, status_y + 4, &state.status_text, STATUS_TEXT, FontSize::Small, TextAlign::Left);

    // Present
    client.present(win);
}

/// Fast raw pixel fill (no DLL call overhead).
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

fn handle_url_key(state: &mut BrowserState, key: u32, ch: u32) -> bool {
    match key {
        KEY_BACKSPACE => {
            if state.url_cursor > 0 {
                state.url_text.remove(state.url_cursor - 1);
                state.url_cursor -= 1;
                state.needs_redraw = true;
            }
        }
        KEY_DELETE => {
            if state.url_cursor < state.url_text.len() {
                state.url_text.remove(state.url_cursor);
                state.needs_redraw = true;
            }
        }
        KEY_LEFT => {
            if state.url_cursor > 0 {
                state.url_cursor -= 1;
                state.needs_redraw = true;
            }
        }
        KEY_RIGHT => {
            if state.url_cursor < state.url_text.len() {
                state.url_cursor += 1;
                state.needs_redraw = true;
            }
        }
        KEY_HOME => {
            state.url_cursor = 0;
            state.needs_redraw = true;
        }
        KEY_END => {
            state.url_cursor = state.url_text.len();
            state.needs_redraw = true;
        }
        KEY_ENTER => {
            return true; // signal to navigate
        }
        KEY_ESCAPE => {
            state.url_focused = false;
            state.needs_redraw = true;
        }
        _ => {
            if ch >= 0x20 && ch <= 0x7E {
                state.url_text.insert(state.url_cursor, ch as u8 as char);
                state.url_cursor += 1;
                state.needs_redraw = true;
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

fn main() {
    let client = match CompositorClient::init() {
        Some(c) => c,
        None => {
            anyos_std::println!("surf: failed to connect to compositor");
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

    // Center on screen
    let (scr_w, scr_h) = client.screen_size();
    let wx = (scr_w.saturating_sub(800)) / 2;
    let wy = (scr_h.saturating_sub(600)) / 2;
    client.move_window(&win, wx as i32, wy as i32);
    client.set_title(&win, "Surf");

    // Set up menu bar
    let mut mb = MenuBarBuilder::new()
        .menu("File")
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

    let mut state = BrowserState::new();

    // Check for URL argument
    let mut args_buf = [0u8; 256];
    let arg_url = anyos_std::process::args(&mut args_buf).trim();
    if !arg_url.is_empty() {
        state.url_text = String::from(arg_url);
        state.url_cursor = state.url_text.len();
        state.url_focused = false;
        let url_copy = state.url_text.clone();
        navigate(&mut state, &url_copy, win.width);
    }

    // Update window title
    update_title(&client, &win, &state);

    // Main event loop
    loop {
        // Poll events
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
                            // Re-layout with new width
                            if state.page_dom.is_some() {
                                relayout(&mut state, new_w);
                            }
                            state.clamp_scroll(new_h);
                            state.needs_redraw = true;
                        }
                    }
                }

                EVT_KEY_DOWN => {
                    let key = evt.arg1;
                    let ch = evt.arg2;

                    if state.url_focused {
                        if handle_url_key(&mut state, key, ch) {
                            // Enter pressed -- navigate
                            let url_copy = state.url_text.clone();
                            navigate(&mut state, &url_copy, win.width);
                            state.url_focused = false;
                            update_title(&client, &win, &state);
                        }
                    } else {
                        // Global shortcuts
                        match key {
                            KEY_ESCAPE => {
                                // Focus URL bar
                                state.url_focused = true;
                                state.url_cursor = state.url_text.len();
                                state.needs_redraw = true;
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
                        if button_hit_test(8, BTN_Y, BTN_W as u32, BTN_H as u32, mx, my) {
                            // Back
                            if state.can_go_back() {
                                state.history_pos -= 1;
                                let url = state.history[state.history_pos].clone();
                                navigate(&mut state, &url, win.width);
                                update_title(&client, &win, &state);
                            }
                        } else if button_hit_test(42, BTN_Y, BTN_W as u32, BTN_H as u32, mx, my) {
                            // Forward
                            if state.can_go_forward() {
                                state.history_pos += 1;
                                let url = state.history[state.history_pos].clone();
                                navigate(&mut state, &url, win.width);
                                update_title(&client, &win, &state);
                            }
                        } else if button_hit_test(76, BTN_Y, BTN_W as u32, BTN_H as u32, mx, my) {
                            // Reload
                            if let Some(ref url) = state.current_url {
                                let url_str = format_url(url);
                                navigate(&mut state, &url_str, win.width);
                                update_title(&client, &win, &state);
                            }
                        } else {
                            // URL bar click
                            let url_w = (win.width as i32 - URL_X - 8).max(60);
                            if mx >= URL_X && mx < URL_X + url_w && my >= URL_Y && my < URL_Y + URL_H as i32 {
                                state.url_focused = true;
                                // Estimate cursor position from click x
                                let click_offset = mx - URL_X - 4; // padding
                                let char_w = 7; // approximate char width
                                let pos = (click_offset / char_w).max(0) as usize;
                                state.url_cursor = pos.min(state.url_text.len());
                                state.needs_redraw = true;
                            } else {
                                state.url_focused = false;
                                state.needs_redraw = true;
                            }
                        }
                    } else if my >= (win.height as i32 - STATUS_H) {
                        // Status bar click -- no action
                        state.url_focused = false;
                        state.needs_redraw = true;
                    } else {
                        // Content area click
                        state.url_focused = false;
                        let content_y = my - TOOLBAR_H + state.scroll_y;
                        if let Some(ref root) = state.page_layout {
                            if let Some(link) = paint::hit_test(root, mx, content_y, 0) {
                                // Navigate to link
                                let resolved = if let Some(ref base) = state.current_url {
                                    format_url(&http::resolve_url(base, &link))
                                } else {
                                    link
                                };
                                navigate(&mut state, &resolved, win.width);
                                update_title(&client, &win, &state);
                            }
                        }
                        state.needs_redraw = true;
                    }
                }

                EVT_MOUSE_SCROLL => {
                    let dz = evt.arg1 as i32;
                    state.scroll_y += dz * 40;
                    state.clamp_scroll(win.height);
                    state.needs_redraw = true;
                }

                EVT_MOUSE_MOVE => {
                    let mx = evt.arg1 as i32;
                    let my = evt.arg2 as i32;

                    // Update hover link for status bar
                    if my > TOOLBAR_H && my < (win.height as i32 - STATUS_H) {
                        let content_y = my - TOOLBAR_H + state.scroll_y;
                        if let Some(ref root) = state.page_layout {
                            let link = paint::hit_test(root, mx, content_y, 0);
                            if link != state.hover_link {
                                state.hover_link = link.clone();
                                if let Some(ref url) = link {
                                    state.status_text = url.clone();
                                } else {
                                    state.status_text = String::from("Done");
                                }
                                state.needs_redraw = true;
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
                        MENU_OPEN_URL => {
                            state.url_focused = true;
                            state.url_text.clear();
                            state.url_cursor = 0;
                            state.needs_redraw = true;
                        }
                        MENU_RELOAD => {
                            if let Some(ref url) = state.current_url {
                                let url_str = format_url(url);
                                navigate(&mut state, &url_str, win.width);
                                update_title(&client, &win, &state);
                            }
                        }
                        MENU_ABOUT => {
                            state.status_text = String::from("Surf 1.0 - Web Browser for anyOS");
                            state.needs_redraw = true;
                        }
                        _ => {}
                    }
                }

                _ => {}
            }
        }

        // Repaint if needed
        if state.needs_redraw {
            render(&client, &win, &state);
            state.needs_redraw = false;
        }

        anyos_std::process::sleep(16); // ~60 Hz poll rate
    }
}

fn update_title(client: &CompositorClient, win: &WindowHandle, state: &BrowserState) {
    if state.page_title.is_empty() {
        client.set_title(win, "Surf");
    } else {
        let mut title = state.page_title.clone();
        title.push_str(" - Surf");
        client.set_title(win, &title);
    }
}

fn relayout(state: &mut BrowserState, win_w: u32) {
    if let Some(ref dom) = state.page_dom {
        let viewport_w = (win_w as i32 - SCROLLBAR_W as i32).max(100);
        let root = layout::layout(dom, &state.page_styles, viewport_w);
        state.total_height = paint::total_height(&root);
        state.page_layout = Some(root);
    }
}
