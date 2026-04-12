// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! surf-host — anyOS Surf browser rendering on native Linux.
//!
//! Fetches a webpage via ureq (HTTP/HTTPS with TLS), renders it through
//! libwebview's HTML/CSS engine, and displays the result in a minifb window.
//!
//! Usage:
//!   surf-host <url>                                     Open in window
//!   surf-host <url> --screenshot out.png                Save screenshot and exit
//!   surf-host <url> --screenshot out.png --delay 2000   Wait 2s, then screenshot
//!   surf-host <url> --width 1280 --height 960           Custom viewport size
//!
//! In window mode:
//!   F5 = viewport screenshot   F6 = full-page screenshot
//!   Mouse click on link        = navigate
//!   Mouse click on form field  = focus field for typing
//!   Enter in focused field     = submit form (if inside a form)
//!   Escape = clear focus / quit (unfocused)

// Force-link libfont so its #[no_mangle] symbols are available to libfont_client.
extern crate libfont;

use std::io::Read;
use std::sync::{Arc, Mutex, mpsc};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use minifb::{Key, KeyRepeat, Window, WindowOptions, MouseMode, MouseButton};

// ── CLI args ─────────────────────────────────────────────────────────────────

struct Args {
    url: String,
    width: u32,
    height: u32,
    screenshot: Option<String>,
    fullpage: bool,
    delay_ms: u64,
    y_range: Option<(u32, u32)>, // (start, end) in pixels
}

fn parse_args() -> Args {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 || args[1].starts_with('-') {
        eprintln!("Usage: surf-host <url> [options]");
        eprintln!();
        eprintln!("Options:");
        eprintln!("  --screenshot <path.png>   Save screenshot and exit");
        eprintln!("  --fullpage                Capture entire page height (not just viewport)");
        eprintln!("  -y <start-end>            Capture Y range, e.g. -y 400-900");
        eprintln!("  --delay <ms>              Wait before screenshot (default: 0)");
        eprintln!("  --width <px>              Viewport width (default: 1024)");
        eprintln!("  --height <px>             Viewport height (default: 768)");
        eprintln!();
        eprintln!("In window mode: F5=screenshot, F6=full-page screenshot, Esc=quit.");
        std::process::exit(1);
    }

    let mut a = Args {
        url: args[1].clone(),
        width: 1024,
        height: 768,
        screenshot: None,
        fullpage: false,
        delay_ms: 0,
        y_range: None,
    };

    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--screenshot" | "-s" => {
                i += 1;
                a.screenshot = Some(args.get(i).cloned().unwrap_or_else(|| {
                    eprintln!("--screenshot requires a file path");
                    std::process::exit(1);
                }));
            }
            "--delay" | "-d" => {
                i += 1;
                a.delay_ms = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(0);
            }
            "--fullpage" | "-f" => {
                a.fullpage = true;
                i += 1;
                continue;
            }
            "-y" => {
                i += 1;
                if let Some(range_str) = args.get(i) {
                    a.y_range = parse_y_range(range_str);
                    if a.y_range.is_none() {
                        eprintln!("-y expects a range like 400-900 (in pixels)");
                        std::process::exit(1);
                    }
                }
            }
            "--width" | "-w" => {
                i += 1;
                a.width = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(1024);
            }
            "--height" | "-h" => {
                i += 1;
                a.height = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(768);
            }
            _ => {
                eprintln!("Unknown option: {}", args[i]);
                std::process::exit(1);
            }
        }
        i += 1;
    }
    a
}

// ── System Font Registration ──────────────────────────────────────────────────

/// Register system fonts as web-font aliases for CSS generic keywords and
/// common named fonts (Georgia, Arial, etc.) so they resolve to real glyphs.
fn register_system_fonts(wv: &mut libwebview::WebView) {
    // Helper: load a TTF/OTF file and register it under one or more names.
    let mut try_load = |path: &str, names: &[&str]| {
        if let Ok(data) = std::fs::read(path) {
            if let Some(font_id) = libfont_client::load_data(&data) {
                for &name in names {
                    wv.register_web_font(name, font_id);
                }
                return true;
            }
        }
        false
    };

    // Serif fonts: Georgia, Times New Roman, Times → NotoSerif / DejaVuSerif
    let serif_paths = [
        "/usr/share/fonts/truetype/noto/NotoSerif-Regular.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSerif-Regular.ttf",
    ];
    let serif_names = &["serif", "georgia", "times new roman", "times",
        "palatino", "palatino linotype", "book antiqua",
        "linux libertine o", "linux libertine", "charter"][..];
    for path in &serif_paths {
        if try_load(path, serif_names) { break; }
    }

    // Bold serif
    let serif_bold_paths = [
        "/usr/share/fonts/truetype/noto/NotoSerif-Bold.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSerif-Bold.ttf",
    ];
    for path in &serif_bold_paths {
        if try_load(path, &["serif-bold"]) { break; }
    }

    // Sans-serif: register aliases that might not match the default font_id=0
    // so that font-family:"Arial" etc. explicitly resolve.
    let sans_paths = [
        "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
    ];
    let sans_names = &["sans-serif", "arial", "helvetica", "helvetica neue",
        "verdana", "tahoma", "trebuchet ms", "system-ui",
        "-apple-system", "blinkmacsystemfont", "segoe ui",
        "roboto", "lato", "open sans", "source sans pro",
        "noto sans", "ubuntu", "cantarell", "fira sans",
        "droid sans", "liberation sans"][..];
    for path in &sans_paths {
        if try_load(path, sans_names) { break; }
    }

    // Monospace fonts
    let mono_paths = [
        "/usr/share/fonts/truetype/noto/NotoMono-Regular.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
    ];
    let mono_names = &["monospace", "courier new", "courier", "consolas",
        "monaco", "lucida console", "source code pro",
        "fira mono", "fira code", "ubuntu mono",
        "droid sans mono", "anonymous pro", "liberation mono"][..];
    for path in &mono_paths {
        if try_load(path, mono_names) { break; }
    }
}

// ── Navigation ────────────────────────────────────────────────────────────────

/// Load a URL: fetch HTML, load resources, run JS.  Returns (html, base_url).
/// This is the common pipeline used both at startup and during navigation.
fn load_page(wv: &mut libwebview::WebView, url: &str) -> PendingImages {
    eprintln!("[surf-host] loading: {}", url);
    let (html, base_url) = fetch_page(url);
    eprintln!("[surf-host] got {} bytes HTML", html.len());

    // Clear old page state including stylesheets so no styles bleed across pages.
    wv.clear();
    wv.clear_stylesheets();
    wv.set_url(&base_url);
    wv.set_html_no_js(&html);
    load_resources(wv, &base_url);         // CSS, fonts, SVGs (sync) + initial relayout
    let pending = start_image_loading(wv, &base_url);  // images (async, parallel threads)
    run_javascript(wv, &base_url);
    run_js_timers(wv, 5000);
    pending
}

fn debug_log_image_bounds(wv: &mut libwebview::WebView) {
    if std::env::var("SURF_DEBUG_HEISE").ok().as_deref() != Some("1") {
        return;
    }
    let Some(dom) = wv.dom() else { return; };
    eprintln!("[surf-host] debug module bounds begin");
    for (i, _) in dom.nodes.iter().enumerate() {
        let module_name = dom.attr(i, "data-module-name");
        let component = dom.attr(i, "data-component");
        let collapse_target = dom.attr(i, "data-collapse-target");
        let id_attr = dom.attr(i, "id");
        if module_name.is_none()
            && component.is_none()
            && collapse_target.is_none()
            && !matches!(id_attr, Some("HEI_D_Top" | "HEI_D_Right" | "HEI_M_Incontent-1" | "HEI_D_Stage" | "topnavimodule"))
        {
            continue;
        }
        let Some((x, y, w, h)) = wv.node_bounds(i) else { continue; };
        if y > 2200 || h <= 0 {
            continue;
        }
        eprintln!(
            "[surf-host]   module node={} y={} h={} x={} w={} module={:?} component={:?} id={:?} collapse={:?}",
            i,
            y,
            h,
            x,
            w,
            module_name,
            component,
            id_attr,
            collapse_target
        );
    }
    eprintln!("[surf-host] debug module bounds end");
    eprintln!("[surf-host] debug image bounds begin");
    let mut rows: Vec<(i32, usize, Option<&str>, Option<(i32, i32, i32, i32)>, String)> = Vec::new();
    for (i, _) in dom.nodes.iter().enumerate() {
        if !(dom.tag(i) == Some(libwebview::dom::Tag::Img) || dom.has_tag_name(i, "a-img")) {
            continue;
        }
        let src = dom.image_url(i).unwrap_or_default();
        let bounds = wv.node_bounds(i);
        let sort_y = bounds.map(|(_, y, _, _)| y).unwrap_or(i32::MAX);
        rows.push((sort_y, i, dom.raw_tag_name(i), bounds, src));
    }
    rows.sort_by_key(|(y, _, _, _, _)| *y);
    for (_, node_id, raw, bounds, src) in rows.into_iter().take(40) {
        let cache_info = wv.images.get_ref(&src).map(|entry| {
            let mut sample = String::new();
            for &idx in &[0usize, entry.pixels.len() / 2, entry.pixels.len().saturating_sub(1)] {
                if let Some(&px) = entry.pixels.get(idx) {
                    if !sample.is_empty() {
                        sample.push(',');
                    }
                    sample.push_str(&format!("{:08X}", px));
                }
            }
            format!(" cache={}x{} sample=[{}]", entry.width, entry.height, sample)
        }).unwrap_or_else(|| String::from(" cache=missing"));
        eprintln!(
            "[surf-host]   node={} raw={:?} bounds={:?} src={}{}",
            node_id, raw, bounds, src, cache_info
        );
    }
    eprintln!("[surf-host] debug image bounds end");

    fn dump_subtree(
        wv: &libwebview::WebView,
        dom: &libwebview::dom::Dom,
        node_id: usize,
        depth: usize,
        max_depth: usize,
    ) {
        if depth > max_depth {
            return;
        }
        let indent = "  ".repeat(depth);
        let bounds = wv.node_bounds(node_id);
        let tag = dom.raw_tag_name(node_id).unwrap_or("#text");
        let id_attr = dom.attr(node_id, "id").unwrap_or("");
        let class_attr = dom.attr(node_id, "class").unwrap_or("");
        let component = dom.attr(node_id, "data-component").unwrap_or("");
        let text = dom.text_content(node_id).replace('\n', " ");
        let text = text.trim();
        let text = if text.len() > 80 { &text[..80] } else { text };
        eprintln!(
            "[surf-host] {}node={} tag={} bounds={:?} id={:?} class={:?} component={:?} text={:?}",
            indent,
            node_id,
            tag,
            bounds,
            id_attr,
            class_attr,
            component,
            text
        );
        for &child_id in &dom.nodes[node_id].children {
            dump_subtree(wv, dom, child_id, depth + 1, max_depth);
        }
    }

    for (root, label, max_depth) in [
        (76usize, "topnavi", 5usize),
        (371usize, "header-scroll", 3usize),
        (421usize, "teaser-scroll", 3usize),
    ] {
        if root < dom.nodes.len() {
            eprintln!("[surf-host] debug subtree begin: {}", label);
            dump_subtree(wv, dom, root, 0, max_depth);
            eprintln!("[surf-host] debug subtree end: {}", label);
        }
    }

}

// ── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    let args = parse_args();
    let width = args.width;
    let height = args.height;

    // Initialize font engine
    libfont_client::init();

    // Create WebView
    let mut wv = libwebview::WebView::new(width, height);

    // Pre-register system font aliases so CSS font-family generic keywords and
    // common font names resolve to real fonts without @font-face declarations.
    register_system_fonts(&mut wv);

    // Load the initial page (CSS+fonts+SVGs sync, images async in parallel)
    let mut pending = load_page(&mut wv, &args.url);
    let mut current_url = args.url.clone();

    // For screenshot mode: wait for all images before capturing
    if args.screenshot.is_some() {
        for r in pending.drain() {
            wv.add_image(&r.src_attr, r.pixels, r.width, r.height);
        }
        wv.relayout();
        debug_log_image_bounds(&mut wv);
    }

    // Build initial framebuffer
    let mut framebuffer = vec![0xFFFFFFFFu32; (width * height) as usize];
    extract_pixels(&wv, &mut framebuffer, width as usize, height as usize, 0);

    // ── Screenshot-only mode ─────────────────────────────────────────────
    if let Some(ref path) = args.screenshot {
        if args.delay_ms > 0 {
            eprintln!("[surf-host] waiting {}ms before screenshot (running timers)...", args.delay_ms);
            // Run timers in steps during the wait period so setTimeout/setInterval
            // callbacks fire (e.g. boot sequences, animations).
            let step = 50u64;
            let mut waited = 0u64;
            while waited < args.delay_ms {
                if wv.has_timers() {
                    wv.run_timers(step);
                }
                wv.tick(step);
                waited += step;
                // Sleep a small amount to avoid 100% CPU
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            // Final relayout after timers
            wv.relayout();
            debug_log_image_bounds(&mut wv);
            // Print any console output from timer callbacks
            for line in wv.js_console() {
                eprintln!("[js:console:timer] {}", line);
            }
        }
        extract_pixels(&wv, &mut framebuffer, width as usize, height as usize, 0);
        if let Some((y_start, y_end)) = args.y_range {
            save_range_screenshot(&mut wv, width, y_start, y_end, path);
        } else if args.fullpage {
            save_fullpage_screenshot(&mut wv, width, path);
        } else {
            save_screenshot(&framebuffer, width, height, path);
        }
        eprintln!("[surf-host] screenshot saved: {}", path);
        return;
    }

    // ── Window mode ──────────────────────────────────────────────────────

    // Shared buffer for typed characters (from InputCallback running on UI thread).
    let typed_chars: Arc<Mutex<Vec<char>>> = Arc::new(Mutex::new(Vec::new()));
    let typed_chars_cb = typed_chars.clone();

    let mut window = Window::new(
        &format!("surf-host — {}", current_url),
        width as usize,
        height as usize,
        WindowOptions {
            resize: true,
            ..WindowOptions::default()
        },
    )
    .expect("Failed to create window");

    window.set_target_fps(30);

    // Register character input callback so we receive typed Unicode characters.
    {
        struct CharCollector(Arc<Mutex<Vec<char>>>);
        impl minifb::InputCallback for CharCollector {
            fn add_char(&mut self, uni_char: u32) {
                if let Some(c) = char::from_u32(uni_char) {
                    if !c.is_control() || c == '\n' || c == '\r' {
                        self.0.lock().unwrap().push(c);
                    }
                }
            }
        }
        window.set_input_callback(Box::new(CharCollector(typed_chars_cb)));
    }

    let mut scroll_y: i32 = 0;
    let mut needs_redraw = true;
    let mut screenshot_count: u32 = 0;
    let mut f5_was_pressed = false;
    let mut f6_was_pressed = false;

    // Mouse click tracking (detect rising/falling edge)
    let mut mouse_was_down = false;

    // Focused form control (control_id from libwebview)
    // We maintain the text content ourselves so we can echo it back.
    let mut focused_control: Option<(u32, String)> = None; // (control_id, current_text)

    // Navigation request (set by click handler, processed at top of loop)
    let mut navigate_to: Option<String> = None;

    eprintln!("[surf-host] window open. F5=screenshot, F6=full-page, click links, Esc=quit.");

    while window.is_open() {
        // ── Process navigation request ──────────────────────────────────
        if let Some(url) = navigate_to.take() {
            // Handle #anchor links — just scroll to the anchor position.
            if url.starts_with('#') {
                // TODO: resolve anchor position and set scroll_y
                eprintln!("[nav] anchor: {}", url);
            } else {
                let abs = resolve_url(&current_url, &url);
                eprintln!("[nav] navigating to: {}", abs);
                current_url = abs.clone();
                focused_control = None;
                pending = load_page(&mut wv, &abs);
                scroll_y = 0;
                window.set_title(&format!("surf-host — {}", abs));
                needs_redraw = true;
            }
        }

        // ── Escape key: unfocus or quit ─────────────────────────────────
        if window.is_key_pressed(Key::Escape, KeyRepeat::No) {
            if focused_control.is_some() {
                focused_control = None;
            } else {
                break;
            }
        }

        // ── Scroll ──────────────────────────────────────────────────────
        if let Some(scroll) = window.get_scroll_wheel() {
            let delta = -(scroll.1 as i32) * 40;
            scroll_y = (scroll_y + delta).max(0);
            needs_redraw = true;
        }

        // ── Keyboard navigation ─────────────────────────────────────────
        // Page Up / Page Down / Home / End
        let viewport_h = height as i32;
        if window.is_key_pressed(Key::PageDown, KeyRepeat::Yes) {
            scroll_y = (scroll_y + viewport_h - 40).max(0);
            needs_redraw = true;
        }
        if window.is_key_pressed(Key::PageUp, KeyRepeat::Yes) {
            scroll_y = (scroll_y - viewport_h + 40).max(0);
            needs_redraw = true;
        }
        if window.is_key_pressed(Key::Home, KeyRepeat::No) {
            scroll_y = 0;
            needs_redraw = true;
        }
        if window.is_key_pressed(Key::End, KeyRepeat::No) {
            scroll_y = (wv.total_height() - viewport_h).max(0);
            needs_redraw = true;
        }

        // ── Keyboard input for focused form control ─────────────────────
        if let Some((ctrl_id, ref mut text)) = focused_control {
            // Drain typed characters from callback.
            let new_chars: Vec<char> = {
                let mut guard = typed_chars.lock().unwrap();
                std::mem::take(&mut *guard)
            };
            let mut changed = !new_chars.is_empty();
            for c in new_chars {
                if c == '\r' || c == '\n' {
                    // Enter: submit the form that contains the focused control.
                    let node_id = wv.form_controls().iter()
                        .find(|fc| fc.control_id == ctrl_id)
                        .map(|fc| fc.node_id)
                        .unwrap_or(0);
                    if let Some((action, method, _enctype)) = wv.form_action_for_node(node_id) {
                        let data = wv.collect_form_data_for_node(node_id);
                        let query = form_encode(&data);
                        let nav_url = if method == "GET" {
                            let base = if action.is_empty() {
                                current_url.clone()
                            } else {
                                resolve_url(&current_url, &action)
                            };
                            if query.is_empty() { base } else { format!("{}?{}", base, query) }
                        } else {
                            resolve_url(&current_url, &action)
                        };
                        eprintln!("[enter] form submit → {}", nav_url);
                        navigate_to = Some(nav_url);
                    }
                } else {
                    text.push(c);
                }
            }

            // Backspace
            if window.is_key_pressed(Key::Backspace, KeyRepeat::Yes) {
                text.pop();
                changed = true;
            }

            if changed {
                wv.set_form_control_text(ctrl_id, text);
                needs_redraw = true;
            }
        } else {
            // Drain and discard typed chars when no field is focused.
            typed_chars.lock().unwrap().clear();
        }

        // ── Mouse click ─────────────────────────────────────────────────
        let mouse_down = window.get_mouse_down(MouseButton::Left);
        let clicked = !mouse_down && mouse_was_down; // released = click
        mouse_was_down = mouse_down;

        if clicked {
            if let Some((mx, my)) = window.get_mouse_pos(MouseMode::Discard) {
                let mx = mx as i32;
                let my = my as i32;

                // 1. Hit-test for text/textarea form control → focus
                if let Some(ctrl_id) = wv.hit_test_form_control_viewport(mx, my, scroll_y) {
                    let text = wv.get_form_control_text(ctrl_id);
                    focused_control = Some((ctrl_id, text));
                    needs_redraw = true;
                    eprintln!("[click] focused form control {}", ctrl_id);
                }
                // 2. Hit-test for submit button → collect form data and navigate
                else if let Some(node_id) = wv.hit_test_submit_viewport(mx, my, scroll_y) {
                    focused_control = None;
                    if let Some((action, method, _enctype)) = wv.form_action_for_node(node_id) {
                        let data = wv.collect_form_data_for_node(node_id);
                        let query = form_encode(&data);
                        let base = if action.is_empty() {
                            current_url.clone()
                        } else {
                            resolve_url(&current_url, &action)
                        };
                        let nav_url = if method == "GET" {
                            if query.is_empty() { base } else { format!("{}?{}", base, query) }
                        } else {
                            base
                        };
                        eprintln!("[click] submit → {}", nav_url);
                        navigate_to = Some(nav_url);
                    } else {
                        eprintln!("[click] submit (no form action)");
                    }
                }
                // 3. Hit-test for hyperlink → navigate
                else if let Some(href) = wv.hit_test_link_viewport(mx, my, scroll_y) {
                    let href = href.to_string();
                    focused_control = None;
                    navigate_to = Some(href);
                }
                // 4. Click on empty area → unfocus
                else {
                    focused_control = None;
                }
            }
        }

        // ── F5 = screenshot (current viewport) ─────────────────────────
        let f5_down = window.is_key_down(Key::F5);
        if f5_down && !f5_was_pressed {
            screenshot_count += 1;
            let path = format!("screenshot_{}.png", screenshot_count);
            save_screenshot(&framebuffer, width, height, &path);
            eprintln!("[surf-host] screenshot saved: {}", path);
        }
        f5_was_pressed = f5_down;

        // ── F6 = full-page screenshot ───────────────────────────────────
        let f6_down = window.is_key_down(Key::F6);
        if f6_down && !f6_was_pressed {
            screenshot_count += 1;
            let path = format!("screenshot_{}_full.png", screenshot_count);
            save_fullpage_screenshot(&mut wv, width, &path);
            eprintln!("[surf-host] full-page screenshot saved: {}", path);
        }
        f6_was_pressed = f6_down;

        // ── Window resize ───────────────────────────────────────────────
        let (win_w, win_h) = window.get_size();
        let win_w = win_w as u32;
        let win_h = win_h as u32;
        let cur_w = wv.viewport_width();
        if win_w != cur_w {
            wv.resize(win_w, win_h);
            framebuffer.resize((win_w * win_h) as usize, 0xFFFFFFFF);
            needs_redraw = true;
        }

        // ── Progressive image loading ───────────────────────────────────
        if !pending.is_done() {
            let results = pending.poll();
            if !results.is_empty() {
                for r in results {
                    wv.add_image(&r.src_attr, r.pixels, r.width, r.height);
                }
                wv.relayout();
                needs_redraw = true;
            }
        }

        // ── Render ──────────────────────────────────────────────────────
        if needs_redraw {
            let (fb_w, fb_h) = (win_w as usize, win_h as usize);
            framebuffer.fill(0xFFFFFFFF);
            extract_pixels(&wv, &mut framebuffer, fb_w, fb_h, scroll_y);
            // Draw form control text (typed content) on top of tiles.
            draw_form_control_texts(
                &mut framebuffer, &wv, fb_w, fb_h, scroll_y,
                focused_control.as_ref().map(|(id, _)| *id),
            );
            // Draw focus outline.
            if let Some((ctrl_id, _)) = focused_control {
                draw_focus_outline(&mut framebuffer, &wv, fb_w, fb_h, scroll_y, ctrl_id);
            }
            needs_redraw = false;
        }

        window
            .update_with_buffer(&framebuffer, win_w as usize, win_h as usize)
            .unwrap_or_default();
    }
}

// ── Focus outline ─────────────────────────────────────────────────────────────

/// Draw a blue 2-pixel outline around the focused form control.
fn draw_focus_outline(
    fb: &mut [u32],
    wv: &libwebview::WebView,
    fb_w: usize,
    fb_h: usize,
    scroll_y: i32,
    ctrl_id: u32,
) {
    // Find the form control position.
    for fc in wv.form_controls() {
        if fc.control_id != ctrl_id { continue; }
        let vx = fc.doc_x;
        let vy = fc.doc_y - scroll_y;
        let vw = fc.doc_w;
        let vh = fc.doc_h;
        let color = 0xFF0078D7u32; // Windows-style blue focus

        // Draw top, bottom, left, right edges (2px)
        for t in 0..2i32 {
            // Top
            for xi in (vx - t)..(vx + vw + t) {
                let yi = vy - t;
                if xi >= 0 && xi < fb_w as i32 && yi >= 0 && yi < fb_h as i32 {
                    fb[yi as usize * fb_w + xi as usize] = color;
                }
            }
            // Bottom
            for xi in (vx - t)..(vx + vw + t) {
                let yi = vy + vh + t;
                if xi >= 0 && xi < fb_w as i32 && yi >= 0 && yi < fb_h as i32 {
                    fb[yi as usize * fb_w + xi as usize] = color;
                }
            }
            // Left
            for yi in (vy - t)..(vy + vh + t) {
                let xi = vx - t;
                if xi >= 0 && xi < fb_w as i32 && yi >= 0 && yi < fb_h as i32 {
                    fb[yi as usize * fb_w + xi as usize] = color;
                }
            }
            // Right
            for yi in (vy - t)..(vy + vh + t) {
                let xi = vx + vw + t;
                if xi >= 0 && xi < fb_w as i32 && yi >= 0 && yi < fb_h as i32 {
                    fb[yi as usize * fb_w + xi as usize] = color;
                }
            }
        }
        break;
    }
}

// ── Form control text rendering ───────────────────────────────────────────────

/// Render the text content (and cursor) of each text form control into the framebuffer.
/// Also draws a cursor bar for the focused control.
fn draw_form_control_texts(
    fb: &mut [u32],
    wv: &libwebview::WebView,
    fb_w: usize,
    fb_h: usize,
    scroll_y: i32,
    focused_ctrl: Option<u32>,
) {
    for fc in wv.form_controls() {
        if fc.control_id == 0 { continue; }
        match fc.kind {
            libwebview::FormFieldKind::TextInput
            | libwebview::FormFieldKind::Password
            | libwebview::FormFieldKind::Textarea => {}
            _ => continue,
        }
        let text = wv.get_form_control_text(fc.control_id);
        if text.is_empty() { continue; }

        let vx = fc.doc_x;
        let vy = fc.doc_y - scroll_y;
        if vy + fc.doc_h < 0 || vy >= fb_h as i32 { continue; }

        // Draw text inside the box with 4px padding.
        let text_x = vx + 4;
        let text_y = vy + 4;
        let font_size: u16 = (fc.doc_h.saturating_sub(8).max(10) as u16).min(20);

        // Clip text to box width.
        let max_w = (fc.doc_w - 8).max(0) as u32;
        let display_text = clip_text_to_width(&text, font_size, max_w);

        if !display_text.is_empty() {
            libfont_client::draw_string_buf(
                fb.as_mut_ptr(), fb_w as u32, fb_h as u32,
                text_x, text_y, 0xFF000000,
                0, font_size, &display_text,
            );
        }

        // Draw cursor for focused field.
        if focused_ctrl == Some(fc.control_id) {
            let (text_w, _) = libfont_client::measure(0, font_size, &display_text);
            let cx = text_x + text_w as i32;
            let cy_top = vy + 3;
            let cy_bot = vy + fc.doc_h - 3;
            if cx >= 0 && cx < fb_w as i32 {
                for cy in cy_top.max(0)..cy_bot.min(fb_h as i32) {
                    fb[cy as usize * fb_w + cx as usize] = 0xFF000000;
                }
            }
        }
    }
}

/// Clip text so it fits within `max_width` pixels (right-clips to show cursor).
fn clip_text_to_width(text: &str, font_size: u16, max_width: u32) -> String {
    // Try full string first.
    let (w, _) = libfont_client::measure(0, font_size, text);
    if w <= max_width {
        return text.to_string();
    }
    // Show the end of the text (cursor area) by clipping from the left.
    let mut start = 0;
    let chars: Vec<char> = text.chars().collect();
    while start < chars.len() {
        let s: String = chars[start..].iter().collect();
        let (sw, _) = libfont_client::measure(0, font_size, &s);
        if sw <= max_width {
            return s;
        }
        start += 1;
    }
    String::new()
}

// ── Form helpers ──────────────────────────────────────────────────────────────

/// URL-encode form data as "key=value&key2=value2".
fn form_encode(data: &[(String, String)]) -> String {
    data.iter()
        .map(|(k, v)| format!("{}={}", url_encode(k), url_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9'
            | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => { out.push('%'); out.push_str(&format!("{:02X}", b)); }
        }
    }
    out
}

// ── Screenshot ───────────────────────────────────────────────────────────────

fn save_screenshot(fb: &[u32], width: u32, height: u32, path: &str) {
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    for (i, &argb) in fb.iter().enumerate() {
        let a = ((argb >> 24) & 0xFF) as u8;
        let r = ((argb >> 16) & 0xFF) as u8;
        let g = ((argb >> 8) & 0xFF) as u8;
        let b = (argb & 0xFF) as u8;
        rgba[i * 4] = r;
        rgba[i * 4 + 1] = g;
        rgba[i * 4 + 2] = b;
        rgba[i * 4 + 3] = a;
    }
    if let Some(img) = image::RgbaImage::from_raw(width, height, rgba) {
        if let Err(e) = img.save(path) {
            eprintln!("[surf-host] screenshot error: {}", e);
        }
    }
}

/// Parse a Y range like "400-900", "400-900px", "400px-900px".
fn parse_y_range(s: &str) -> Option<(u32, u32)> {
    let s = s.trim();
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 2 { return None; }
    let start: u32 = parts[0].trim().trim_end_matches("px").parse().ok()?;
    let end: u32 = parts[1].trim().trim_end_matches("px").parse().ok()?;
    if end <= start { return None; }
    Some((start, end))
}

/// Save a screenshot of a specific Y range of the document.
fn save_range_screenshot(wv: &mut libwebview::WebView, width: u32, y_start: u32, y_end: u32, path: &str) {
    let range_h = y_end - y_start;
    eprintln!("[surf-host] range: y={}..{} ({}px)", y_start, y_end, range_h);

    // Ensure tiles exist for the requested range
    let viewport_h = wv.viewport_height().max(256);
    let mut y = y_start as i32;
    while y < y_end as i32 {
        let mut pending = true;
        while pending {
            pending = wv.render_viewport_at(y);
        }
        y += viewport_h as i32;
    }

    // Render into a buffer sized to the range, scrolled to y_start
    let mut fb = vec![0xFFFFFFFFu32; (width * range_h) as usize];
    extract_pixels(wv, &mut fb, width as usize, range_h as usize, y_start as i32);
    save_screenshot(&fb, width, range_h, path);
}

/// Save a full-page screenshot by rendering the entire document height.
/// Scrolls through the entire page to ensure all tiles are rasterized.
fn save_fullpage_screenshot(wv: &mut libwebview::WebView, width: u32, path: &str) {
    let doc_h = wv.total_height().max(1) as u32;
    let viewport_h = wv.viewport_height();
    eprintln!("[surf-host] full-page: {}x{} (viewport {})", width, doc_h, viewport_h);

    // Render all tile rows by scrolling through the entire document.
    // render_viewport creates tiles incrementally (max 2 per call),
    // so we call it repeatedly until all tiles are generated.
    let step = viewport_h.max(256) as i32;
    let mut y = 0i32;
    while y < doc_h as i32 {
        // Simulate scrolling to this position to trigger tile creation
        let mut pending = true;
        while pending {
            pending = wv.render_viewport_at(y);
        }
        y += step;
    }

    let mut fb = vec![0xFFFFFFFFu32; (width * doc_h) as usize];
    extract_pixels(wv, &mut fb, width as usize, doc_h as usize, 0);
    save_screenshot(&fb, width, doc_h, path);
}

// ── Fetch helpers ────────────────────────────────────────────────────────────

/// Disk cache validity period (24 hours).
const CACHE_MAX_AGE_SECS: u64 = 24 * 3600;

fn disk_cache_dir() -> std::path::PathBuf {
    let base = std::env::var("XDG_CACHE_HOME").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        format!("{}/.cache", home)
    });
    std::path::PathBuf::from(base).join("surf-host")
}

fn url_cache_key(url: &str) -> String {
    let mut hasher = DefaultHasher::new();
    url.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn fetch_page(url: &str) -> (String, String) {
    if url.starts_with("file://") {
        let path = &url[7..];
        let html = std::fs::read_to_string(path).unwrap_or_else(|e| {
            format!("<h1>Error</h1><p>Failed to read {}: {}</p>", path, e)
        });
        (html, url.to_string())
    } else {
        let full_url = if url.starts_with("http://") || url.starts_with("https://") {
            url.to_string()
        } else {
            format!("https://{}", url)
        };
        match ureq::get(&full_url).call() {
            Ok(response) => {
                let final_url = response.get_url().to_string();
                let body = response.into_string().unwrap_or_default();
                (body, final_url)
            }
            Err(e) => {
                eprintln!("[surf-host] fetch error: {}", e);
                (format!("<h1>Error</h1><p>{}</p>", e), full_url)
            }
        }
    }
}

fn fetch_resource(url: &str) -> Option<Vec<u8>> {
    if url.starts_with("file://") {
        return std::fs::read(&url[7..]).ok();
    }

    // Check disk cache — fresh entries (< 24h) are served directly.
    let dir = disk_cache_dir();
    let key = url_cache_key(url);
    let data_path = dir.join(format!("{}.data", key));

    if let Ok(meta) = std::fs::metadata(&data_path) {
        if let Ok(modified) = meta.modified() {
            if let Ok(age) = modified.elapsed() {
                if age.as_secs() < CACHE_MAX_AGE_SECS {
                    if let Ok(data) = std::fs::read(&data_path) {
                        return Some(data);
                    }
                }
            }
        }
    }

    // Network fetch
    match ureq::get(url).call() {
        Ok(resp) => {
            let mut buf = Vec::new();
            resp.into_reader().read_to_end(&mut buf).ok()?;
            // Save to disk cache
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(&data_path, &buf);
            Some(buf)
        }
        Err(_) => {
            // Fall back to stale cache on network error
            std::fs::read(&data_path).ok()
        }
    }
}

pub fn resolve_url(base: &str, relative: &str) -> String {
    if relative.starts_with("http://") || relative.starts_with("https://") {
        return relative.to_string();
    }
    if relative.starts_with("//") {
        let scheme = if base.starts_with("https") { "https:" } else { "http:" };
        return format!("{}{}", scheme, relative);
    }
    if relative.starts_with('#') {
        // Fragment-only: same page
        if let Some(q) = base.find('#') {
            return format!("{}{}", &base[..q], relative);
        }
        return format!("{}{}", base, relative);
    }
    if relative.starts_with('/') {
        if base.starts_with("file://") {
            if let Ok(root) = std::env::var("SURF_WEB_ROOT") {
                let mut root = root.trim_end_matches('/').to_string();
                if !root.starts_with('/') {
                    root.insert(0, '/');
                }
                return format!("file://{}{}", root, relative);
            }
        }
        if let Some(idx) = base.find("://") {
            let rest = &base[idx + 3..];
            let host_end = rest.find('/').map(|i| idx + 3 + i).unwrap_or(base.len());
            return format!("{}{}", &base[..host_end], relative);
        }
        return format!("{}{}", base.trim_end_matches('/'), relative);
    }
    if relative.starts_with('?') {
        // Query-only: same path, new query
        let path_end = base.find('?').unwrap_or(base.len());
        return format!("{}{}", &base[..path_end], relative);
    }
    if let Some(last_slash) = base.rfind('/') {
        // Ensure we only go up to the directory part (after the host)
        let after_proto = if let Some(idx) = base.find("://") { idx + 3 } else { 0 };
        if last_slash > after_proto {
            format!("{}/{}", &base[..last_slash], relative)
        } else {
            format!("{}/{}", base.trim_end_matches('/'), relative)
        }
    } else {
        format!("{}/{}", base, relative)
    }
}

// ── Resource loading (DOM-based, identical to anyOS surf) ────────────────────

use libwebview::dom::{NodeType, Tag};

fn debug_dump_text_runs(bx: &libwebview::LayoutBox, depth: usize) {
    if let Some(text) = &bx.text {
        if !text.is_empty() {
            eprintln!(
                "[surf-host] text-run depth={} node={:?} x={} y={} w={} h={} font_id={} size={} text={:?}",
                depth,
                bx.node_id,
                bx.x,
                bx.y,
                bx.width,
                bx.height,
                bx.custom_font_id,
                bx.font_size,
                text
            );
        }
    }
    for child in &bx.children {
        debug_dump_text_runs(child, depth + 1);
    }
}

fn debug_dump_boxes(
    dom: &libwebview::dom::Dom,
    bx: &libwebview::LayoutBox,
    depth: usize,
    abs_x: i32,
    abs_y: i32,
) {
    let cur_abs_x = if bx.is_fixed { bx.x } else { abs_x + bx.x };
    let cur_abs_y = if bx.is_fixed { bx.y } else { abs_y + bx.y };
    if bx.bg_color != 0 || bx.node_id.is_some() {
        let (tag, class_attr) = if let Some(node_id) = bx.node_id {
            let node = dom.get(node_id);
            let tag = match &node.node_type {
                NodeType::Element { .. } => String::from("Element"),
                NodeType::Text(_) => String::from("#text"),
                _ => String::from("?"),
            };
            let class_attr = dom.attr(node_id, "class").unwrap_or("");
            (tag, class_attr.to_string())
        } else {
            (String::from("-"), String::new())
        };
        eprintln!(
            "[surf-host] box depth={} node={:?} tag={} class={:?} rel=({}, {}) abs=({}, {}) size=({}, {}) margin=({}, {}, {}, {}) bg=0x{:08x}",
            depth,
            bx.node_id,
            tag,
            class_attr,
            bx.x,
            bx.y,
            cur_abs_x,
            cur_abs_y,
            bx.width,
            bx.height,
            bx.margin.top,
            bx.margin.right,
            bx.margin.bottom,
            bx.margin.left,
            bx.bg_color
        );
    }
    for child in &bx.children {
        debug_dump_boxes(dom, child, depth + 1, cur_abs_x, cur_abs_y);
    }
}

fn debug_dump_pre_text(dom: &libwebview::dom::Dom) {
    for (node_id, node) in dom.nodes.iter().enumerate() {
        if matches!(node.node_type, NodeType::Element { tag: Tag::Pre, .. }) {
            eprintln!("[surf-host] pre node {}", node_id);
            for &child_id in &node.children {
                if let NodeType::Text(text) = &dom.get(child_id).node_type {
                    eprintln!("[surf-host] pre text child {} {:?}", child_id, text);
                }
            }
        }
    }
}

fn debug_dump_table_styles(wv: &libwebview::WebView, dom: &libwebview::dom::Dom) {
    for (node_id, _) in dom.nodes.iter().enumerate() {
        let Some(tag) = dom.tag(node_id) else { continue; };
        if !matches!(tag, Tag::Table | Tag::Td | Tag::Tr | Tag::Tbody) {
            continue;
        }
        let Some(style) = wv.resolved_style_ref(node_id) else { continue; };
        let bounds = wv.node_bounds(node_id);
        let tag_name = dom
            .raw_tag_name(node_id)
            .map(str::to_string)
            .unwrap_or_else(|| String::from("tag"));
        eprintln!(
            "[surf-host] table-style node={} tag={} bounds={:?} display={:?} width={:?} height={:?} padding=({}, {}, {}, {}) border_width={} border_sides=({}, {}, {}, {}) border_spacing=({}, {}) border_collapse={} margin=({}, {}, {}, {})",
            node_id,
            tag_name,
            bounds,
            style.display,
            style.width,
            style.height,
            style.padding_top,
            style.padding_right,
            style.padding_bottom,
            style.padding_left,
            style.border_width,
            style.border_top.width,
            style.border_right.width,
            style.border_bottom.width,
            style.border_left.width,
            style.border_spacing_x,
            style.border_spacing_y,
            style.border_collapse,
            style.margin_top,
            style.margin_right,
            style.margin_bottom,
            style.margin_left
        );
    }
}

fn debug_dump_named_styles(wv: &libwebview::WebView, dom: &libwebview::dom::Dom) {
    for (node_id, _) in dom.nodes.iter().enumerate() {
        let Some(id_attr) = dom.attr(node_id, "id") else { continue; };
        if !matches!(id_attr, "wrapper" | "div1" | "div2" | "reference" | "inner") {
            continue;
        }
        let Some(style) = wv.resolved_style_ref(node_id) else { continue; };
        let bg_image = match &style.background_image {
            libwebview::style::BackgroundImageVal::None => "none",
            libwebview::style::BackgroundImageVal::Url(_) => "url",
            libwebview::style::BackgroundImageVal::LinearGradient { .. } => "linear-gradient",
        };
        eprintln!(
            "[surf-host] named-style node={} id={} bounds={:?} top={:?}/{:?} left={:?}/{:?} right={:?}/{:?} bottom={:?}/{:?} padding=({}, {}, {}, {}) margin=({}, {}, {}, {}) bg_pos=({}, {}) bg_image={}",
            node_id,
            id_attr,
            wv.node_bounds(node_id),
            style.top,
            style.top_calc,
            style.left_offset,
            style.left_calc,
            style.right_offset,
            style.right_calc,
            style.bottom_offset,
            style.bottom_calc,
            style.padding_top,
            style.padding_right,
            style.padding_bottom,
            style.padding_left,
            style.margin_top,
            style.margin_right,
            style.margin_bottom,
            style.margin_left,
            style.background_position_x,
            style.background_position_y,
            bg_image,
        );
    }
}

/// Load all external resources by walking the parsed DOM tree.
/// This mirrors the logic in apps/surf/src/resources.rs.
fn load_resources(wv: &mut libwebview::WebView, base_url: &str) {
    // 1. Stylesheets: <link rel="stylesheet" href="...">
    let css_links = {
        let dom = match wv.dom() { Some(d) => d, None => return };
        let mut links = Vec::new();
        for (i, node) in dom.nodes.iter().enumerate() {
            if let NodeType::Element { tag: Tag::Link, .. } = &node.node_type {
                let rel = dom.attr(i, "rel").unwrap_or("");
                if !rel.eq_ignore_ascii_case("stylesheet") { continue; }
                if let Some(href) = dom.attr(i, "href") {
                    if !href.is_empty() {
                        links.push((resolve_url(base_url, href), String::from(href)));
                    }
                }
            }
        }
        links
    };

    for (css_url, _href) in &css_links {
        eprintln!("[surf-host] fetching CSS: {}", css_url);
        if let Some(css_body) = fetch_resource(css_url) {
            if let Ok(css_text) = String::from_utf8(css_body) {
                wv.add_stylesheet(&css_text);

                // @import URLs from this stylesheet
                let imports: Vec<String> = wv.last_stylesheet_imports()
                    .iter()
                    .map(|u| resolve_url(base_url, u))
                    .collect();
                for import_url in &imports {
                    eprintln!("[surf-host] fetching @import CSS: {}", import_url);
                    if let Some(import_body) = fetch_resource(import_url) {
                        if let Ok(import_text) = String::from_utf8(import_body) {
                            wv.add_stylesheet(&import_text);
                        }
                    }
                }

                // @font-face rules from this stylesheet
                let font_faces: Vec<_> = wv.last_stylesheet_font_faces()
                    .iter()
                    .map(|ff| (ff.family.clone(), ff.src_url.clone()))
                    .collect();
                for (family, src) in &font_faces {
                    if src.is_empty() { continue; }
                    let font_url = resolve_url(base_url, src);
                    eprintln!("[surf-host] fetching font: {}", font_url);
                    if let Some(font_data) = fetch_resource(&font_url) {
                        if let Some(font_id) = libfont_client::load_data(&font_data) {
                            wv.register_web_font(&family, font_id);
                        }
                    }
                }
            }
        }
    }

    if std::env::var_os("SURF_DEBUG_WEBFONTS").is_some() {
        eprintln!(
            "[surf-host] debug webfonts: Ahem={:?} ahem={:?}",
            wv.web_font_id("Ahem"),
            wv.web_font_id("ahem")
        );
    }

    // 2. @font-face from inline <style> blocks
    {
        let font_faces: Vec<_> = wv.all_font_faces()
            .iter()
            .map(|ff| (ff.family.clone(), ff.src_url.clone()))
            .collect();
        for (family, src) in &font_faces {
            if src.is_empty() { continue; }
            if wv.web_font_id(&family).is_some() { continue; } // already loaded
            let font_url = resolve_url(base_url, src);
            eprintln!("[surf-host] fetching inline font: {}", font_url);
            if let Some(font_data) = fetch_resource(&font_url) {
                if let Some(font_id) = libfont_client::load_data(&font_data) {
                    wv.register_web_font(&family, font_id);
                }
            }
        }
    }

    // 3. Images: loaded asynchronously via start_image_loading()

    // 4. Inline SVGs: <svg>...</svg> — rasterise via resvg and cache under __svg_N__
    let svg_nodes: Vec<(usize, String, Vec<(String, String)>)> = {
        let dom = match wv.dom() { Some(d) => d, None => { wv.relayout(); return; } };
        let mut svgs = Vec::new();
        for (i, node) in dom.nodes.iter().enumerate() {
            if let NodeType::Element { tag: Tag::Svg, attrs } = &node.node_type {
                // Inner SVG content is stored as a text child by the HTML parser.
                let mut inner = String::new();
                for &child_id in &node.children {
                    if let NodeType::Text(ref t) = dom.nodes[child_id].node_type {
                        inner = t.clone();
                        break;
                    }
                }
                if inner.is_empty() { continue; }
                let attr_list: Vec<(String, String)> = attrs.iter()
                    .map(|a| (a.name.clone(), a.value.clone()))
                    .collect();
                svgs.push((i, inner, attr_list));
            }
        }
        svgs
    };

    for (node_id, inner, attrs) in &svg_nodes {
        // Reconstruct full SVG markup from the stored inner content and attributes.
        let mut svg_markup = String::from("<svg");
        for (name, value) in attrs {
            svg_markup.push(' ');
            svg_markup.push_str(name);
            svg_markup.push_str("=\"");
            // Escape attribute value quotes.
            for ch in value.chars() {
                if ch == '"' { svg_markup.push_str("&quot;"); } else { svg_markup.push(ch); }
            }
            svg_markup.push('"');
        }
        // Ensure SVG namespace is present so resvg parses it correctly.
        if !attrs.iter().any(|(n, _)| n == "xmlns") {
            svg_markup.push_str(" xmlns=\"http://www.w3.org/2000/svg\"");
        }
        svg_markup.push('>');
        svg_markup.push_str(inner);
        svg_markup.push_str("</svg>");

        if let Some((pixels, w, h)) = decode_svg(svg_markup.as_bytes()) {
            // Key format: __svg_<node_id>__ — must match svg_inline_key() in layout/mod.rs
            let key = format!("__svg_{}__", node_id);
            eprintln!("[surf-host] rasterized inline SVG node={} {}x{}", node_id, w, h);
            wv.add_image(&key, pixels, w, h);
        }
    }

    // Re-layout with all resources loaded (images not yet available — placeholders used)
    wv.relayout();
    if std::env::var_os("SURF_DEBUG_LAYOUT_TEXT").is_some() {
        if let Some(root) = wv.layout_root_ref() {
            debug_dump_text_runs(root, 0);
        }
    }
    if std::env::var_os("SURF_DEBUG_LAYOUT_BOXES").is_some() {
        if let (Some(root), Some(dom)) = (wv.layout_root_ref(), wv.dom()) {
            debug_dump_boxes(dom, root, 0, 0, 0);
        }
    }
    if std::env::var_os("SURF_DEBUG_PRE_TEXT").is_some() {
        if let Some(dom) = wv.dom() {
            debug_dump_pre_text(dom);
        }
    }
    if std::env::var_os("SURF_DEBUG_TABLE_STYLES").is_some() {
        if let Some(dom) = wv.dom() {
            debug_dump_table_styles(wv, dom);
        }
    }
    if std::env::var_os("SURF_DEBUG_NAMED_STYLES").is_some() {
        if let Some(dom) = wv.dom() {
            debug_dump_named_styles(wv, dom);
        }
    }
}

// ── Parallel image loading ──────────────────────────────────────────────────

struct ImageLoadResult {
    src_attr: String,
    pixels: Vec<u32>,
    width: u32,
    height: u32,
}

struct PendingImages {
    receiver: mpsc::Receiver<ImageLoadResult>,
    done: bool,
}

impl PendingImages {
    fn empty() -> Self {
        let (_tx, rx) = mpsc::channel();
        PendingImages { receiver: rx, done: true }
    }

    /// Non-blocking: returns any images that have finished loading.
    fn poll(&mut self) -> Vec<ImageLoadResult> {
        let mut results = Vec::new();
        loop {
            match self.receiver.try_recv() {
                Ok(r) => results.push(r),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => { self.done = true; break; }
            }
        }
        results
    }

    /// Blocking: waits for all remaining images.
    fn drain(&mut self) -> Vec<ImageLoadResult> {
        let mut results = Vec::new();
        while let Ok(r) = self.receiver.recv() {
            results.push(r);
        }
        self.done = true;
        results
    }

    fn is_done(&self) -> bool {
        self.done
    }
}

/// Collect image URLs from DOM and spawn parallel fetch+decode threads.
/// Returns a `PendingImages` handle to poll or drain results.
fn start_image_loading(wv: &libwebview::WebView, base_url: &str) -> PendingImages {
    let img_infos = {
        let dom = match wv.dom() { Some(d) => d, None => return PendingImages::empty() };
        let mut infos = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for (i, node) in dom.nodes.iter().enumerate() {
            let is_image_like = matches!(
                &node.node_type,
                NodeType::Element { tag: Tag::Img, .. }
            ) || dom.has_tag_name(i, "a-img");
            if !is_image_like {
                continue;
            }

            if let Some(src) = dom.image_url(i) {
                if src.is_empty() || src.starts_with("data:") { continue; }
                if !seen.insert(src.to_string()) { continue; }
                let abs_url = resolve_url(base_url, &src);
                let bounds_hint = wv
                    .node_bounds(i)
                    .and_then(|(_, _, w, h)| {
                        if w > 0 && h > 0 {
                            Some((w as u32, h as u32))
                        } else {
                            None
                        }
                    });
                let target_w: Option<u32> = bounds_hint
                    .map(|(w, _)| w)
                    .or_else(|| dom.attr(i, "width").and_then(|s| s.trim().trim_end_matches("px").parse().ok()));
                let target_h: Option<u32> = bounds_hint
                    .map(|(_, h)| h)
                    .or_else(|| dom.attr(i, "height").and_then(|s| s.trim().trim_end_matches("px").parse().ok()));
                infos.push((abs_url, src, target_w, target_h));
            }
        }
        for (i, _) in dom.nodes.iter().enumerate() {
            let Some(style) = wv.resolved_style_ref(i) else { continue; };
            let bg_src = match &style.background_image {
                libwebview::style::BackgroundImageVal::Url(src) if !src.is_empty() => src,
                _ => continue,
            };
            if bg_src.starts_with("data:") {
                continue;
            }
            if !seen.insert(bg_src.clone()) {
                continue;
            }
            let abs_url = resolve_url(base_url, bg_src);
            infos.push((abs_url, bg_src.clone(), None, None));
        }
        infos
    };

    if img_infos.is_empty() {
        return PendingImages::empty();
    }

    eprintln!("[surf-host] loading {} images in parallel...", img_infos.len());
    let (tx, rx) = mpsc::channel();

    for (img_url, src_attr, tw, th) in img_infos {
        let tx = tx.clone();
        std::thread::spawn(move || {
            if let Some(img_data) = fetch_resource(&img_url) {
                if let Some((pixels, w, h)) = decode_image_scaled(&img_data, tw, th) {
                    eprintln!("[surf-host]   image ready {}x{}: {}", w, h, src_attr);
                    let _ = tx.send(ImageLoadResult {
                        src_attr,
                        pixels,
                        width: w,
                        height: h,
                    });
                }
            }
        });
    }
    drop(tx); // Close sender so receiver knows when all threads finish

    PendingImages { receiver: rx, done: false }
}

// ── JavaScript execution ────────────────────────────────────────────────────

/// Register a synchronous HTTP handler so that fetch()/XHR inside JS
/// can make real HTTP requests via ureq.
fn register_http_handler(wv: &mut libwebview::WebView) {
    use libjs::JsValue;
    use libjs::value::JsObject;
    use std::rc::Rc;
    use std::cell::RefCell;

    fn native_http_handler(_vm: &mut libjs::Vm, args: &[JsValue]) -> JsValue {
        let method = args.get(0).map(|v| v.to_js_string()).unwrap_or_default();
        let url = args.get(1).map(|v| v.to_js_string()).unwrap_or_default();
        let _headers = args.get(2).map(|v| v.to_js_string()).unwrap_or_default();
        let body = args.get(3).map(|v| v.to_js_string()).unwrap_or_default();

        if url.is_empty() {
            let mut obj = JsObject::new();
            obj.set(String::from("status"), JsValue::Number(0.0));
            obj.set(String::from("statusText"), JsValue::String(String::from("Empty URL")));
            obj.set(String::from("body"), JsValue::String(String::new()));
            return JsValue::Object(Rc::new(RefCell::new(obj)));
        }

        eprintln!("[js-http] {} {}", method, url);
        let result = if method == "POST" {
            ureq::post(&url)
                .set("Content-Type", "application/x-www-form-urlencoded")
                .send_string(&body)
        } else {
            ureq::get(&url).call()
        };

        match result {
            Ok(resp) => {
                let status = resp.status() as f64;
                let status_text = String::from(resp.status_text());
                let resp_body = resp.into_string().unwrap_or_default();
                let mut obj = JsObject::new();
                obj.set(String::from("status"), JsValue::Number(status));
                obj.set(String::from("statusText"), JsValue::String(status_text));
                obj.set(String::from("body"), JsValue::String(resp_body));
                JsValue::Object(Rc::new(RefCell::new(obj)))
            }
            Err(e) => {
                eprintln!("[js-http] error: {}", e);
                let mut obj = JsObject::new();
                obj.set(String::from("status"), JsValue::Number(0.0));
                obj.set(String::from("statusText"), JsValue::String(format!("{}", e)));
                obj.set(String::from("body"), JsValue::String(String::new()));
                JsValue::Object(Rc::new(RefCell::new(obj)))
            }
        }
    }

    let handler = libjs::vm::native_fn("__http_handler", native_http_handler);
    wv.js_runtime().engine().set_global("__http_handler", handler);
}

/// Collect all script entries from the DOM, fetch external scripts,
/// and execute them all in document order.
fn run_javascript(wv: &mut libwebview::WebView, base_url: &str) {
    // Register synchronous HTTP handler so fetch()/XHR work inside JS.
    register_http_handler(wv);

    // Collect script entries (inline + external) in document order.
    let entries = wv.script_entries();
    if entries.is_empty() {
        eprintln!("[js] no scripts found");
        return;
    }

    let mut scripts: Vec<String> = Vec::new();
    let mut external_count = 0u32;
    let mut inline_count = 0u32;

    for entry in &entries {
        match entry {
            libwebview::js::ScriptEntry::Inline { text, mode: _ } => {
                scripts.push(text.clone());
                inline_count += 1;
            }
            libwebview::js::ScriptEntry::External { src: src_url, mode: _ } => {
                let full_url = resolve_url(base_url, src_url);
                eprintln!("[js] fetching script: {}", full_url);
                if let Some(data) = fetch_resource(&full_url) {
                    if let Ok(text) = String::from_utf8(data) {
                        scripts.push(text);
                        external_count += 1;
                    } else {
                        eprintln!("[js]   not valid UTF-8, skipping");
                    }
                } else {
                    eprintln!("[js]   fetch failed, skipping");
                }
            }
        }
    }

    eprintln!("[js] {} scripts total ({} inline, {} external)",
        scripts.len(), inline_count, external_count);

    // Execute all scripts.
    wv.execute_js(&scripts);

    // Print console output.
    for line in wv.js_console() {
        eprintln!("[js:console] {}", line);
    }
}

/// Run JS timers for `total_ms` milliseconds in 50ms steps.
/// This lets setTimeout(fn, 0) and short-delay timers fire.
fn run_js_timers(wv: &mut libwebview::WebView, total_ms: u64) {
    if !wv.has_timers() { return; }
    eprintln!("[js] running timers for {}ms ({} timers pending)...", total_ms, wv.timer_count());
    let step = 50u64;
    let mut elapsed = 0u64;
    while elapsed < total_ms && wv.has_timers() {
        wv.run_timers(step);
        elapsed += step;
        // Print console output from timer callbacks
        for line in wv.js_console() {
            eprintln!("[js:console] {}", line);
        }
    }
    eprintln!("[js] timer loop done ({} ms elapsed, timers remaining: {})",
        elapsed, wv.has_timers());
}

// ── Image decoding ───────────────────────────────────────────────────────────

/// Maximum dimension for images without explicit target size (safety cap).
const MAX_DECODE_DIM: u32 = 2048;

/// Decode an image, optionally downscaling to target dimensions.
/// If target dimensions are specified and the source is >2x larger,
/// the image is resized before storing — saving significant memory.
fn decode_image_scaled(data: &[u8], target_w: Option<u32>, target_h: Option<u32>) -> Option<(Vec<u32>, u32, u32)> {
    if is_svg(data) {
        return decode_svg(data);
    }
    let img = image::load_from_memory(data).ok()?;
    let (orig_w, orig_h) = image::GenericImageView::dimensions(&img);

    let (final_w, final_h) = compute_decode_size(orig_w, orig_h, target_w, target_h);

    let rgba = if final_w != orig_w || final_h != orig_h {
        image::imageops::resize(
            &img.to_rgba8(), final_w, final_h,
            image::imageops::FilterType::Triangle,
        )
    } else {
        img.to_rgba8()
    };

    let (w, h) = image::GenericImageView::dimensions(&rgba);
    let pixels: Vec<u32> = rgba
        .pixels()
        .map(|p| {
            let [r, g, b, a] = p.0;
            ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
        })
        .collect();
    Some((pixels, w, h))
}

/// Compute decode target size.  Downscales if source is >2x the target.
/// Caps at MAX_DECODE_DIM when no target hint is available.
fn compute_decode_size(orig_w: u32, orig_h: u32, target_w: Option<u32>, target_h: Option<u32>) -> (u32, u32) {
    let (tw, th) = match (target_w, target_h) {
        (Some(w), Some(h)) if w > 0 && h > 0 => (w, h),
        (Some(w), None) if w > 0 && orig_w > 0 => {
            (w, (orig_h as u64 * w as u64 / orig_w as u64).max(1) as u32)
        }
        (None, Some(h)) if h > 0 && orig_h > 0 => {
            ((orig_w as u64 * h as u64 / orig_h as u64).max(1) as u32, h)
        }
        _ => {
            // No target hint — cap at MAX_DECODE_DIM
            if orig_w > MAX_DECODE_DIM || orig_h > MAX_DECODE_DIM {
                let scale = MAX_DECODE_DIM as f64 / orig_w.max(orig_h) as f64;
                return (
                    ((orig_w as f64 * scale) as u32).max(1),
                    ((orig_h as f64 * scale) as u32).max(1),
                );
            }
            return (orig_w, orig_h);
        }
    };

    // Only downscale if source is >2x larger (marginal savings not worth the blur)
    if orig_w <= tw * 2 && orig_h <= th * 2 {
        return (orig_w, orig_h);
    }

    (tw.max(1), th.max(1))
}

fn is_svg(data: &[u8]) -> bool {
    let header = &data[..data.len().min(256)];
    if let Ok(s) = std::str::from_utf8(header) {
        let trimmed = s.trim_start();
        trimmed.starts_with("<?xml") || trimmed.starts_with("<svg") || trimmed.contains("<svg")
    } else {
        false
    }
}

fn decode_svg(data: &[u8]) -> Option<(Vec<u32>, u32, u32)> {
    let tree = resvg::usvg::Tree::from_data(data, &resvg::usvg::Options::default()).ok()?;
    let size = tree.size();
    let w = size.width() as u32;
    let h = size.height() as u32;
    if w == 0 || h == 0 { return None; }
    let (rw, rh) = if w > 1024 || h > 1024 {
        let scale = 1024.0 / (w.max(h) as f32);
        ((w as f32 * scale) as u32, (h as f32 * scale) as u32)
    } else {
        (w, h)
    };
    let mut pixmap = resvg::tiny_skia::Pixmap::new(rw, rh)?;
    let transform = resvg::tiny_skia::Transform::from_scale(
        rw as f32 / size.width() as f32,
        rh as f32 / size.height() as f32,
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let pixels: Vec<u32> = pixmap
        .pixels()
        .iter()
        .map(|p| {
            ((p.alpha() as u32) << 24) | ((p.red() as u32) << 16) | ((p.green() as u32) << 8) | (p.blue() as u32)
        })
        .collect();
    Some((pixels, rw, rh))
}

// ── Pixel extraction ─────────────────────────────────────────────────────────

fn extract_pixels(
    _wv: &libwebview::WebView,
    fb: &mut [u32],
    width: usize,
    height: usize,
    scroll_y: i32,
) {
    // Tile canvases have positions set by the renderer (pos_y = row * 256).
    // We composite each canvas at its actual position, adjusted by scroll.
    for canvas_id in 1..500u32 {
        if let Some((pixels, cw, ch, _px, py)) = libanyui_client::host_get_canvas_pixels(canvas_id) {
            let cw = cw as usize;
            let ch = ch as usize;
            if cw == 0 || ch == 0 || pixels.len() < cw * ch { continue; }

            // Canvas position in document coordinates, adjusted by scroll
            let canvas_top = py - scroll_y;
            let canvas_bottom = canvas_top + ch as i32;

            // Skip if entirely outside viewport
            if canvas_bottom <= 0 || canvas_top >= height as i32 { continue; }

            let src_start = if canvas_top < 0 { (-canvas_top) as usize } else { 0 };
            let dst_start = if canvas_top > 0 { canvas_top as usize } else { 0 };

            for row in src_start..ch {
                let dst_y = dst_start + row - src_start;
                if dst_y >= height { break; }
                let copy_w = cw.min(width);
                let src_off = row * cw;
                let dst_off = dst_y * width;
                fb[dst_off..dst_off + copy_w].copy_from_slice(&pixels[src_off..src_off + copy_w]);
            }
        }
    }
}
