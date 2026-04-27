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

use eframe::egui;
use minifb::{Key, KeyRepeat, MouseButton, MouseMode, Window, WindowOptions};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::sync::{mpsc, Arc, Mutex};

const SURF_HOST_USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124 Safari/537.36";

// ── CLI args ─────────────────────────────────────────────────────────────────

struct Args {
    url: String,
    width: u32,
    height: u32,
    screenshot: Option<String>,
    fullpage: bool,
    delay_ms: u64,
    y_range: Option<(u32, u32)>, // (start, end) in pixels
    minifb: bool,
    js_enabled: bool,
    remote_listen: Option<String>,
}

fn parse_args() -> Args {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-?") {
        eprintln!("Usage: surf-host [url] [options]");
        eprintln!();
        eprintln!("Options:");
        eprintln!("  --screenshot <path.png>   Save screenshot and exit");
        eprintln!("  --fullpage                Capture entire page height (not just viewport)");
        eprintln!("  -y <start-end>            Capture Y range, e.g. -y 400-900");
        eprintln!("  --delay <ms>              Wait before screenshot (default: 0)");
        eprintln!("  --width <px>              Viewport width (default: 1024)");
        eprintln!("  --height <px>             Viewport height (default: 768)");
        eprintln!("  --minifb                  Use the legacy minifb window instead of egui");
        eprintln!("  --no-js                   Disable JavaScript execution");
        eprintln!("  --remote-listen <addr>    Listen for text commands (default: 127.0.0.1:8787)");
        eprintln!();
        eprintln!("Remote commands: open <url>, reload, scroll <y>, screenshot <path>, fullpage <path>, status");
        std::process::exit(1);
    }

    let mut url = String::from("about:blank");
    let mut i = 1;
    if args.get(1).is_some_and(|arg| !arg.starts_with('-')) {
        url = args[1].clone();
        i = 2;
    }

    let mut a = Args {
        url,
        width: 1024,
        height: 768,
        screenshot: None,
        fullpage: false,
        delay_ms: 0,
        y_range: None,
        minifb: false,
        js_enabled: true,
        remote_listen: Some(String::from("127.0.0.1:8787")),
    };

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
            "--minifb" => {
                a.minifb = true;
                i += 1;
                continue;
            }
            "--no-js" => {
                a.js_enabled = false;
                i += 1;
                continue;
            }
            "--no-remote" => {
                a.remote_listen = None;
                i += 1;
                continue;
            }
            "--remote-listen" => {
                i += 1;
                a.remote_listen = Some(args.get(i).cloned().unwrap_or_else(|| {
                    eprintln!("--remote-listen requires an address, e.g. 127.0.0.1:8787");
                    std::process::exit(1);
                }));
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
    let serif_names = &[
        "serif",
        "georgia",
        "times new roman",
        "times",
        "palatino",
        "palatino linotype",
        "book antiqua",
        "linux libertine o",
        "linux libertine",
        "charter",
    ][..];
    for path in &serif_paths {
        if try_load(path, serif_names) {
            break;
        }
    }

    // Bold serif
    let serif_bold_paths = [
        "/usr/share/fonts/truetype/noto/NotoSerif-Bold.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSerif-Bold.ttf",
    ];
    for path in &serif_bold_paths {
        if try_load(path, &["serif-bold"]) {
            break;
        }
    }

    // Sans-serif: register aliases that might not match the default font_id=0
    // so that font-family:"Arial" etc. explicitly resolve.
    let sans_paths = [
        "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
    ];
    let sans_names = &[
        "sans-serif",
        "arial",
        "helvetica",
        "helvetica neue",
        "verdana",
        "tahoma",
        "trebuchet ms",
        "system-ui",
        "-apple-system",
        "blinkmacsystemfont",
        "segoe ui",
        "roboto",
        "lato",
        "open sans",
        "source sans pro",
        "noto sans",
        "ubuntu",
        "cantarell",
        "fira sans",
        "droid sans",
        "liberation sans",
    ][..];
    for path in &sans_paths {
        if try_load(path, sans_names) {
            break;
        }
    }

    // Monospace fonts
    let mono_paths = [
        "/usr/share/fonts/truetype/noto/NotoMono-Regular.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
    ];
    let mono_names = &[
        "monospace",
        "courier new",
        "courier",
        "consolas",
        "monaco",
        "lucida console",
        "source code pro",
        "fira mono",
        "fira code",
        "ubuntu mono",
        "droid sans mono",
        "anonymous pro",
        "liberation mono",
    ][..];
    for path in &mono_paths {
        if try_load(path, mono_names) {
            break;
        }
    }
}

// ── Navigation ────────────────────────────────────────────────────────────────

/// Load a URL: fetch HTML, load resources, run JS.  Returns (html, base_url).
/// This is the common pipeline used both at startup and during navigation.
fn load_page(wv: &mut libwebview::WebView, url: &str, js_enabled: bool) -> PendingImages {
    load_page_inner(wv, url, js_enabled, 0)
}

fn load_page_inner(
    wv: &mut libwebview::WebView,
    url: &str,
    js_enabled: bool,
    redirect_depth: u8,
) -> PendingImages {
    eprintln!("[surf-host] loading: {}", url);
    let (html, base_url) = fetch_page(url);
    eprintln!("[surf-host] got {} bytes HTML", html.len());

    // Clear old page state including stylesheets so no styles bleed across pages.
    wv.clear();
    wv.clear_stylesheets();
    wv.set_url(&base_url);
    wv.set_html_no_js(&html);
    load_resources(wv, &base_url); // CSS, fonts, SVGs (sync) + initial relayout
    let pending = start_image_loading(wv, &base_url); // images (async, parallel threads)
    if js_enabled {
        run_javascript(wv, &base_url);
        run_js_timers(wv, 5000);
        if redirect_depth < 3 {
            if let Some(nav) = wv.take_pending_navigation_requests().pop() {
                let abs = resolve_url(&base_url, &nav.url);
                eprintln!(
                    "[js-nav] {} to {}",
                    if nav.replace { "replace" } else { "navigate" },
                    abs
                );
                return load_page_inner(wv, &abs, js_enabled, redirect_depth + 1);
            }
        }
    }
    pending
}

fn debug_log_image_bounds(wv: &mut libwebview::WebView) {
    if std::env::var("SURF_DEBUG_HEISE").ok().as_deref() != Some("1") {
        return;
    }
    let Some(dom) = wv.dom() else {
        return;
    };
    eprintln!("[surf-host] debug module bounds begin");
    for (i, _) in dom.nodes.iter().enumerate() {
        let module_name = dom.attr(i, "data-module-name");
        let component = dom.attr(i, "data-component");
        let collapse_target = dom.attr(i, "data-collapse-target");
        let id_attr = dom.attr(i, "id");
        if module_name.is_none()
            && component.is_none()
            && collapse_target.is_none()
            && !matches!(
                id_attr,
                Some(
                    "HEI_D_Top"
                        | "HEI_D_Right"
                        | "HEI_M_Incontent-1"
                        | "HEI_D_Stage"
                        | "topnavimodule"
                )
            )
        {
            continue;
        }
        let Some((x, y, w, h)) = wv.node_bounds(i) else {
            continue;
        };
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
    let mut rows: Vec<(
        i32,
        usize,
        Option<&str>,
        Option<(i32, i32, i32, i32)>,
        String,
    )> = Vec::new();
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
        let style_info = wv
            .resolved_style_ref(node_id)
            .map(|style| {
                format!(
                    " style=mt:{} mr:{} mb:{} ml:{} pt:{} pr:{} pb:{} pl:{} w:{:?} h:{:?}",
                    style.margin_top,
                    style.margin_right,
                    style.margin_bottom,
                    style.margin_left,
                    style.padding_top,
                    style.padding_right,
                    style.padding_bottom,
                    style.padding_left,
                    style.width,
                    style.height,
                )
            })
            .unwrap_or_default();
        let cache_info = wv
            .images
            .get_ref(&src)
            .map(|entry| {
                let mut sample = String::new();
                for &idx in &[
                    0usize,
                    entry.pixels.len() / 2,
                    entry.pixels.len().saturating_sub(1),
                ] {
                    if let Some(&px) = entry.pixels.get(idx) {
                        if !sample.is_empty() {
                            sample.push(',');
                        }
                        sample.push_str(&format!("{:08X}", px));
                    }
                }
                format!(
                    " cache={}x{} sample=[{}]",
                    entry.width, entry.height, sample
                )
            })
            .unwrap_or_else(|| String::from(" cache=missing"));
        eprintln!(
            "[surf-host]   node={} raw={:?} bounds={:?} src={}{}{}",
            node_id, raw, bounds, src, cache_info, style_info
        );
    }
    eprintln!("[surf-host] debug image bounds end");

    eprintln!("[surf-host] debug inline svg bounds begin");
    for (node_id, node) in dom.nodes.iter().enumerate() {
        if !matches!(
            &node.node_type,
            libwebview::dom::NodeType::Element {
                tag: libwebview::dom::Tag::Svg,
                ..
            }
        ) {
            continue;
        }
        let key = format!("__svg_{}__", node_id);
        let cache_info = wv
            .images
            .get_ref(&key)
            .map(|entry| {
                let mid = (entry.pixels.len() / 2).min(entry.pixels.len().saturating_sub(1));
                let last = entry.pixels.len().saturating_sub(1);
                let mut sample = String::new();
                for idx in [0usize, mid, last] {
                    if let Some(&px) = entry.pixels.get(idx) {
                        if !sample.is_empty() {
                            sample.push(',');
                        }
                        sample.push_str(&format!("{:08X}", px));
                    }
                }
                format!(
                    " cache={}x{} sample=[{}]",
                    entry.width, entry.height, sample
                )
            })
            .unwrap_or_else(|| String::from(" cache=missing"));
        let class_attr = dom.attr(node_id, "class").unwrap_or("");
        eprintln!(
            "[surf-host]   svg node={} bounds={:?} class={:?}{}",
            node_id,
            wv.node_bounds(node_id),
            class_attr,
            cache_info
        );
    }
    eprintln!("[surf-host] debug inline svg bounds end");

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
        let display = wv
            .resolved_style_ref(node_id)
            .map(|style| format!("{:?}", style.display))
            .unwrap_or_else(|| String::from("?"));
        let text = dom.text_content(node_id).replace('\n', " ");
        let text = text.trim();
        let text = if text.len() > 80 { &text[..80] } else { text };
        eprintln!(
            "[surf-host] {}node={} tag={} display={} bounds={:?} id={:?} class={:?} component={:?} text={:?}",
            indent, node_id, tag, display, bounds, id_attr, class_attr, component, text
        );
        for &child_id in &dom.nodes[node_id].children {
            dump_subtree(wv, dom, child_id, depth + 1, max_depth);
        }
    }

    for (root, label, max_depth) in [
        (76usize, "topnavi", 5usize),
        (361usize, "page-header", 5usize),
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

// ── egui host shell ─────────────────────────────────────────────────────────

enum RemoteCommand {
    Open(String),
    Reload,
    Scroll(i32),
    Screenshot(String),
    FullPage(String),
    Status,
}

struct RemoteRequest {
    command: RemoteCommand,
    reply: TcpStream,
}

struct BrowserHostApp {
    wv: libwebview::WebView,
    pending: PendingImages,
    current_url: String,
    url_input: String,
    framebuffer: Vec<u32>,
    scroll_y: i32,
    texture: Option<egui::TextureHandle>,
    focused_control: Option<(u32, String)>,
    remote_rx: Option<mpsc::Receiver<RemoteRequest>>,
    js_enabled: bool,
    screenshot_count: u32,
    status: String,
    needs_redraw: bool,
}

impl BrowserHostApp {
    fn new(
        wv: libwebview::WebView,
        pending: PendingImages,
        current_url: String,
        remote_rx: Option<mpsc::Receiver<RemoteRequest>>,
        js_enabled: bool,
    ) -> Self {
        let width = wv.viewport_width().max(1) as u32;
        let height = wv.viewport_height().max(1);
        Self {
            wv,
            pending,
            current_url: current_url.clone(),
            url_input: current_url,
            framebuffer: vec![0xFFFFFFFFu32; (width * height) as usize],
            scroll_y: 0,
            texture: None,
            focused_control: None,
            remote_rx,
            js_enabled,
            screenshot_count: 0,
            status: String::from("ready"),
            needs_redraw: true,
        }
    }

    fn navigate(&mut self, url: &str) {
        let abs = if self.current_url == "about:blank" {
            resolve_url("https://example.com/", url)
        } else {
            resolve_url(&self.current_url, url)
        };
        self.status = format!("loading {}", abs);
        self.current_url = abs.clone();
        self.url_input = abs.clone();
        self.focused_control = None;
        self.pending = load_page(&mut self.wv, &abs, self.js_enabled);
        self.scroll_y = 0;
        self.needs_redraw = true;
        self.status = format!("loaded {}", abs);
    }

    fn reload(&mut self) {
        let url = self.current_url.clone();
        self.navigate(&url);
    }

    fn clamp_scroll(&mut self) {
        let max_scroll = (self.wv.total_height() - self.wv.viewport_height() as i32).max(0);
        self.scroll_y = self.scroll_y.clamp(0, max_scroll);
    }

    fn process_remote(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.remote_rx.take() else {
            return;
        };
        while let Ok(mut req) = rx.try_recv() {
            let response = match req.command {
                RemoteCommand::Open(url) => {
                    self.navigate(&url);
                    format!("OK open {}\n", self.current_url)
                }
                RemoteCommand::Reload => {
                    self.reload();
                    format!("OK reload {}\n", self.current_url)
                }
                RemoteCommand::Scroll(y) => {
                    self.scroll_y = y;
                    self.clamp_scroll();
                    self.needs_redraw = true;
                    format!("OK scroll {}\n", self.scroll_y)
                }
                RemoteCommand::Screenshot(path) => {
                    self.render_framebuffer();
                    save_screenshot(
                        &self.framebuffer,
                        self.wv.viewport_width().max(1) as u32,
                        self.wv.viewport_height().max(1),
                        &path,
                    );
                    format!("OK screenshot {}\n", path)
                }
                RemoteCommand::FullPage(path) => {
                    let width = self.wv.viewport_width().max(1) as u32;
                    save_fullpage_screenshot(&mut self.wv, width, &path);
                    self.needs_redraw = true;
                    format!("OK fullpage {}\n", path)
                }
                RemoteCommand::Status => {
                    format!(
                        "OK url={} scroll={} viewport={}x{} doc_h={}\n",
                        self.current_url,
                        self.scroll_y,
                        self.wv.viewport_width(),
                        self.wv.viewport_height(),
                        self.wv.total_height()
                    )
                }
            };
            let _ = std::io::Write::write_all(&mut req.reply, response.as_bytes());
            ctx.request_repaint();
        }
        self.remote_rx = Some(rx);
    }

    fn poll_page_work(&mut self) {
        if !self.pending.is_done() {
            let results = self.pending.poll();
            if !results.is_empty() {
                for r in results {
                    self.wv.add_image(&r.src_attr, r.pixels, r.width, r.height);
                }
                self.wv.relayout();
                self.needs_redraw = true;
            }
        }
        if self.wv.has_timers() {
            self.wv.run_timers(16);
            if let Some(nav) = self.wv.take_pending_navigation_requests().pop() {
                let abs = resolve_url(&self.current_url, &nav.url);
                eprintln!(
                    "[js-nav] {} to {}",
                    if nav.replace { "replace" } else { "navigate" },
                    abs
                );
                self.navigate(&abs);
                return;
            }
            self.wv.tick(16);
            self.wv.relayout();
            self.needs_redraw = true;
            for line in self.wv.js_console() {
                eprintln!("[js:console:egui] {}", line);
            }
        }
    }

    fn render_framebuffer(&mut self) {
        self.clamp_scroll();
        let width = self.wv.viewport_width().max(1) as usize;
        let height = self.wv.viewport_height().max(1) as usize;
        if self.framebuffer.len() != width * height {
            self.framebuffer.resize(width * height, 0xFFFFFFFF);
        }
        self.framebuffer.fill(0xFFFFFFFF);
        extract_pixels(
            &self.wv,
            &mut self.framebuffer,
            width,
            height,
            self.scroll_y,
        );
        draw_form_control_texts(
            &mut self.framebuffer,
            &self.wv,
            width,
            height,
            self.scroll_y,
            self.focused_control.as_ref().map(|(id, _)| *id),
        );
        if let Some((ctrl_id, _)) = self.focused_control {
            draw_focus_outline(
                &mut self.framebuffer,
                &self.wv,
                width,
                height,
                self.scroll_y,
                ctrl_id,
            );
        }
        self.needs_redraw = false;
    }

    fn color_image(&self) -> egui::ColorImage {
        let width = self.wv.viewport_width().max(1) as usize;
        let height = self.wv.viewport_height().max(1) as usize;
        let mut rgba = Vec::with_capacity(width * height * 4);
        for &argb in &self.framebuffer {
            rgba.push(((argb >> 16) & 0xFF) as u8);
            rgba.push(((argb >> 8) & 0xFF) as u8);
            rgba.push((argb & 0xFF) as u8);
            rgba.push(((argb >> 24) & 0xFF) as u8);
        }
        egui::ColorImage::from_rgba_unmultiplied([width, height], &rgba)
    }

    fn update_texture(&mut self, ctx: &egui::Context) {
        if self.needs_redraw {
            self.render_framebuffer();
            let image = self.color_image();
            let options = egui::TextureOptions::NEAREST;
            if let Some(texture) = &mut self.texture {
                texture.set(image, options);
            } else {
                self.texture = Some(ctx.load_texture("surf-webview", image, options));
            }
        }
    }

    fn handle_browser_click(&mut self, pos: egui::Pos2, origin: egui::Pos2) {
        let mx = (pos.x - origin.x).round() as i32;
        let my = (pos.y - origin.y).round() as i32;
        if let Some(ctrl_id) = self
            .wv
            .hit_test_form_control_viewport(mx, my, self.scroll_y)
        {
            let text = self.wv.get_form_control_text(ctrl_id);
            self.focused_control = Some((ctrl_id, text));
            self.needs_redraw = true;
        } else if let Some(node_id) = self.wv.hit_test_submit_viewport(mx, my, self.scroll_y) {
            self.focused_control = None;
            if let Some((action, method, _enctype)) = self.wv.form_action_for_node(node_id) {
                let data = self.wv.collect_form_data_for_node(node_id);
                let query = form_encode(&data);
                let base = if action.is_empty() {
                    self.current_url.clone()
                } else {
                    resolve_url(&self.current_url, &action)
                };
                let nav_url = if method == "GET" && !query.is_empty() {
                    format!("{}?{}", base, query)
                } else {
                    base
                };
                self.navigate(&nav_url);
            }
        } else if let Some(href) = self.wv.hit_test_link_viewport(mx, my, self.scroll_y) {
            let href = href.to_string();
            self.focused_control = None;
            self.navigate(&href);
        } else {
            self.focused_control = None;
            self.needs_redraw = true;
        }
    }
}

impl eframe::App for BrowserHostApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.process_remote(ctx);
        self.poll_page_work();

        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Reload").clicked() {
                    self.reload();
                }
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.url_input)
                        .desired_width(f32::INFINITY)
                        .hint_text("URL"),
                );
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let url = self.url_input.clone();
                    self.navigate(&url);
                }
                if ui.button("Go").clicked() {
                    let url = self.url_input.clone();
                    self.navigate(&url);
                }
                if ui.button("Shot").clicked() {
                    self.screenshot_count += 1;
                    let path = format!("screenshot_{}.png", self.screenshot_count);
                    self.render_framebuffer();
                    save_screenshot(
                        &self.framebuffer,
                        self.wv.viewport_width().max(1) as u32,
                        self.wv.viewport_height().max(1),
                        &path,
                    );
                    self.status = format!("saved {}", path);
                }
            });
        });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.status);
                ui.separator();
                ui.label(format!(
                    "{}x{} scroll {} / {}",
                    self.wv.viewport_width(),
                    self.wv.viewport_height(),
                    self.scroll_y,
                    self.wv.total_height()
                ));
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            let available = ui.available_size();
            let new_w = available.x.max(1.0).round() as u32;
            let new_h = available.y.max(1.0).round() as u32;
            if new_w != self.wv.viewport_width().max(1) as u32
                || new_h != self.wv.viewport_height().max(1)
            {
                self.wv.resize(new_w, new_h);
                self.framebuffer
                    .resize((new_w * new_h) as usize, 0xFFFFFFFF);
                self.needs_redraw = true;
            }

            let scroll_delta = ui.input(|i| i.raw_scroll_delta.y);
            if scroll_delta.abs() > 0.0 && ui.rect_contains_pointer(ui.max_rect()) {
                self.scroll_y = (self.scroll_y - scroll_delta.round() as i32).max(0);
                self.clamp_scroll();
                self.needs_redraw = true;
            }

            self.update_texture(ctx);
            if let Some(texture) = &self.texture {
                let size = egui::vec2(
                    self.wv.viewport_width().max(1) as f32,
                    self.wv.viewport_height().max(1) as f32,
                );
                let response = ui.add(egui::Image::new(texture).fit_to_exact_size(size));
                if response.clicked() {
                    if let Some(pos) = response.interact_pointer_pos() {
                        self.handle_browser_click(pos, response.rect.min);
                    }
                }
            }
        });

        if self.pending.is_done() && !self.wv.has_timers() {
            ctx.request_repaint_after(std::time::Duration::from_millis(33));
        } else {
            ctx.request_repaint();
        }
    }
}

fn start_remote_listener(addr: &str) -> Option<mpsc::Receiver<RemoteRequest>> {
    let listener = match TcpListener::bind(addr) {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!("[surf-host] remote listen failed on {}: {}", addr, e);
            return None;
        }
    };
    eprintln!("[surf-host] remote control listening on {}", addr);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                continue;
            };
            let mut line = String::new();
            let _ = std::io::Read::read_to_string(&mut stream, &mut line);
            match parse_remote_command(line.trim()) {
                Some(command) => {
                    if tx
                        .send(RemoteRequest {
                            command,
                            reply: stream,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                None => {
                    let _ = std::io::Write::write_all(&mut stream, b"ERR unknown command\n");
                }
            }
        }
    });
    Some(rx)
}

fn parse_remote_command(line: &str) -> Option<RemoteCommand> {
    let (cmd, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
    let rest = rest.trim();
    match cmd {
        "open" | "navigate" if !rest.is_empty() => Some(RemoteCommand::Open(rest.to_string())),
        "reload" => Some(RemoteCommand::Reload),
        "scroll" => rest.parse().ok().map(RemoteCommand::Scroll),
        "screenshot" if !rest.is_empty() => Some(RemoteCommand::Screenshot(rest.to_string())),
        "fullpage" if !rest.is_empty() => Some(RemoteCommand::FullPage(rest.to_string())),
        "status" => Some(RemoteCommand::Status),
        _ => None,
    }
}

fn run_egui_browser(
    wv: libwebview::WebView,
    pending: PendingImages,
    current_url: String,
    width: u32,
    height: u32,
    remote_listen: Option<String>,
    js_enabled: bool,
) {
    let remote_rx = remote_listen.as_deref().and_then(start_remote_listener);
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([width as f32, height as f32])
            .with_title(format!("surf-host — {}", current_url)),
        renderer: eframe::Renderer::Glow,
        hardware_acceleration: eframe::HardwareAcceleration::Off,
        ..Default::default()
    };
    let app = BrowserHostApp::new(wv, pending, current_url, remote_rx, js_enabled);
    let _ = eframe::run_native(
        "surf-host",
        native_options,
        Box::new(move |_cc| Box::new(app)),
    );
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
    let mut pending = load_page(&mut wv, &args.url, args.js_enabled);
    let mut current_url = args.url.clone();

    // For screenshot mode: wait for all images before capturing
    if args.screenshot.is_some() {
        let mut results = pending.drain();
        results.sort_by(|a, b| b.node_id.cmp(&a.node_id));
        wv.relayout();
        let debug_heise = std::env::var("SURF_DEBUG_HEISE").ok().as_deref() == Some("1");
        let mut added_images = 0usize;
        let mut skipped_images = 0usize;
        for r in results {
            let priority_y = wv
                .node_bounds(r.node_id)
                .map(|(_, y, _, _)| y)
                .unwrap_or(r.priority_y);
            if priority_y != i32::MAX && priority_y > (height as i32).saturating_add(512) {
                skipped_images += 1;
                continue;
            }
            if debug_heise && added_images < 8 {
                eprintln!(
                    "[surf-host] screenshot add image y={} {}x{} src={}",
                    priority_y, r.width, r.height, r.src_attr
                );
            }
            wv.add_image(&r.src_attr, r.pixels, r.width, r.height);
            added_images += 1;
        }
        if debug_heise {
            eprintln!(
                "[surf-host] screenshot image add summary: added={} skipped={}",
                added_images, skipped_images
            );
        }
        wv.relayout();
        let mut pending_tiles = true;
        while pending_tiles {
            pending_tiles = wv.render_viewport_at(0);
        }
        debug_log_image_bounds(&mut wv);
    }

    // Build initial framebuffer
    let mut framebuffer = vec![0xFFFFFFFFu32; (width * height) as usize];
    extract_pixels(&wv, &mut framebuffer, width as usize, height as usize, 0);

    // ── Screenshot-only mode ─────────────────────────────────────────────
    if let Some(ref path) = args.screenshot {
        if args.delay_ms > 0 {
            eprintln!(
                "[surf-host] waiting {}ms before screenshot (running timers)...",
                args.delay_ms
            );
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
            let mut pending_tiles = true;
            while pending_tiles {
                pending_tiles = wv.render_viewport_at(0);
            }
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

    if !args.minifb {
        run_egui_browser(
            wv,
            pending,
            current_url,
            width,
            height,
            args.remote_listen.clone(),
            args.js_enabled,
        );
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
                pending = load_page(&mut wv, &abs, args.js_enabled);
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
                    let node_id = wv
                        .form_controls()
                        .iter()
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
                            if query.is_empty() {
                                base
                            } else {
                                format!("{}?{}", base, query)
                            }
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
                            if query.is_empty() {
                                base
                            } else {
                                format!("{}?{}", base, query)
                            }
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
                &mut framebuffer,
                &wv,
                fb_w,
                fb_h,
                scroll_y,
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
        if fc.control_id != ctrl_id {
            continue;
        }
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
        if fc.control_id == 0 {
            continue;
        }
        match fc.kind {
            libwebview::FormFieldKind::TextInput
            | libwebview::FormFieldKind::Password
            | libwebview::FormFieldKind::Textarea => {}
            _ => continue,
        }
        let text = wv.get_form_control_text(fc.control_id);
        if text.is_empty() {
            continue;
        }

        let vx = fc.doc_x;
        let vy = fc.doc_y - scroll_y;
        if vy + fc.doc_h < 0 || vy >= fb_h as i32 {
            continue;
        }

        // Draw text inside the box with 4px padding.
        let text_x = vx + 4;
        let text_y = vy + 4;
        let font_size: u16 = (fc.doc_h.saturating_sub(8).max(10) as u16).min(20);

        // Clip text to box width.
        let max_w = (fc.doc_w - 8).max(0) as u32;
        let display_text = clip_text_to_width(&text, font_size, max_w);

        if !display_text.is_empty() {
            libfont_client::draw_string_buf(
                fb.as_mut_ptr(),
                fb_w as u32,
                fb_h as u32,
                text_x,
                text_y,
                0xFF000000,
                0,
                font_size,
                &display_text,
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
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => {
                out.push('%');
                out.push_str(&format!("{:02X}", b));
            }
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
    if parts.len() != 2 {
        return None;
    }
    let start: u32 = parts[0].trim().trim_end_matches("px").parse().ok()?;
    let end: u32 = parts[1].trim().trim_end_matches("px").parse().ok()?;
    if end <= start {
        return None;
    }
    Some((start, end))
}

/// Save a screenshot of a specific Y range of the document.
fn save_range_screenshot(
    wv: &mut libwebview::WebView,
    width: u32,
    y_start: u32,
    y_end: u32,
    path: &str,
) {
    let range_h = y_end - y_start;
    eprintln!(
        "[surf-host] range: y={}..{} ({}px)",
        y_start, y_end, range_h
    );

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
    extract_pixels(
        wv,
        &mut fb,
        width as usize,
        range_h as usize,
        y_start as i32,
    );
    save_screenshot(&fb, width, range_h, path);
}

/// Save a full-page screenshot by rendering the entire document height.
/// Scrolls through the entire page to ensure all tiles are rasterized.
fn save_fullpage_screenshot(wv: &mut libwebview::WebView, width: u32, path: &str) {
    let doc_h = wv.total_height().max(1) as u32;
    let viewport_h = wv.viewport_height();
    eprintln!(
        "[surf-host] full-page: {}x{} (viewport {})",
        width, doc_h, viewport_h
    );

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
    if url == "about:blank" {
        (
            String::from("<!doctype html><title>Surf Host</title><style>body{font:16px sans-serif;padding:32px;color:#202124} input{font:inherit;padding:8px;width:24em}</style><h1>Surf Host</h1><p>Enter a URL in the toolbar to start browsing.</p>"),
            String::from("about:blank"),
        )
    } else if url.starts_with("file://") {
        let path = &url[7..];
        let html = std::fs::read_to_string(path)
            .unwrap_or_else(|e| format!("<h1>Error</h1><p>Failed to read {}: {}</p>", path, e));
        (html, url.to_string())
    } else {
        let full_url = if url.starts_with("http://") || url.starts_with("https://") {
            url.to_string()
        } else {
            format!("https://{}", url)
        };
        let dir = disk_cache_dir();
        let key = url_cache_key(&format!("{}\n{}", full_url, SURF_HOST_USER_AGENT));
        let data_path = dir.join(format!("{}.html", key));
        let url_path = dir.join(format!("{}.url", key));

        match ureq::get(&full_url)
            .set("User-Agent", SURF_HOST_USER_AGENT)
            .call()
        {
            Ok(response) => {
                let final_url = response.get_url().to_string();
                let content_type = response.header("Content-Type").map(str::to_string);
                let mut bytes = Vec::new();
                let _ = response.into_reader().read_to_end(&mut bytes);
                let body = decode_html_bytes(&bytes, content_type.as_deref());
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::fs::write(&data_path, &bytes);
                let _ = std::fs::write(&url_path, final_url.as_bytes());
                (body, final_url)
            }
            Err(e) => {
                eprintln!("[surf-host] fetch error: {}", e);
                if let Ok(bytes) = std::fs::read(&data_path) {
                    let final_url =
                        std::fs::read_to_string(&url_path).unwrap_or_else(|_| full_url.clone());
                    return (decode_html_bytes(&bytes, None), final_url);
                }
                (format!("<h1>Error</h1><p>{}</p>", e), full_url)
            }
        }
    }
}

fn decode_html_bytes(bytes: &[u8], content_type: Option<&str>) -> String {
    let charset = content_type
        .and_then(extract_charset_from_content_type)
        .or_else(|| extract_charset_from_meta(bytes));

    match charset.as_deref() {
        Some("windows-1252") | Some("cp1252") | Some("iso-8859-1") | Some("latin1")
        | Some("latin-1") => decode_windows_1252(bytes),
        _ => match String::from_utf8(bytes.to_vec()) {
            Ok(text) => text,
            Err(_) => decode_windows_1252(bytes),
        },
    }
}

fn extract_charset_from_content_type(content_type: &str) -> Option<String> {
    for part in content_type.split(';') {
        let part = part.trim();
        if let Some((name, value)) = part.split_once('=') {
            if name.trim().eq_ignore_ascii_case("charset") {
                return Some(normalize_charset(value));
            }
        }
    }
    None
}

fn extract_charset_from_meta(bytes: &[u8]) -> Option<String> {
    let head_len = bytes.len().min(4096);
    let head = String::from_utf8_lossy(&bytes[..head_len]).to_ascii_lowercase();
    let idx = head.find("charset")?;
    let after = &head[idx + "charset".len()..];
    let after = after.trim_start();
    let after = after.strip_prefix('=').unwrap_or(after).trim_start();
    let after = after
        .strip_prefix('"')
        .or_else(|| after.strip_prefix('\''))
        .unwrap_or(after);
    let mut end = 0usize;
    for ch in after.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            end += ch.len_utf8();
        } else {
            break;
        }
    }
    if end == 0 {
        None
    } else {
        Some(normalize_charset(&after[..end]))
    }
}

fn normalize_charset(value: &str) -> String {
    value
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_ascii_lowercase()
}

fn decode_windows_1252(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        let ch = match b {
            0x80 => '\u{20AC}',
            0x82 => '\u{201A}',
            0x83 => '\u{0192}',
            0x84 => '\u{201E}',
            0x85 => '\u{2026}',
            0x86 => '\u{2020}',
            0x87 => '\u{2021}',
            0x88 => '\u{02C6}',
            0x89 => '\u{2030}',
            0x8A => '\u{0160}',
            0x8B => '\u{2039}',
            0x8C => '\u{0152}',
            0x8E => '\u{017D}',
            0x91 => '\u{2018}',
            0x92 => '\u{2019}',
            0x93 => '\u{201C}',
            0x94 => '\u{201D}',
            0x95 => '\u{2022}',
            0x96 => '\u{2013}',
            0x97 => '\u{2014}',
            0x98 => '\u{02DC}',
            0x99 => '\u{2122}',
            0x9A => '\u{0161}',
            0x9B => '\u{203A}',
            0x9C => '\u{0153}',
            0x9E => '\u{017E}',
            0x9F => '\u{0178}',
            _ => b as char,
        };
        out.push(ch);
    }
    out
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
    match ureq::get(url).set("User-Agent", SURF_HOST_USER_AGENT).call() {
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
        let scheme = if base.starts_with("https") {
            "https:"
        } else {
            "http:"
        };
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
        let after_proto = if let Some(idx) = base.find("://") {
            idx + 3
        } else {
            0
        };
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

use libwebview::dom::{Dom, NodeType, Tag};

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
                NodeType::Element { tag, .. } => String::from(tag.tag_name()),
                NodeType::Text(_) => String::from("#text"),
            };
            let class_attr = dom.attr(node_id, "class").unwrap_or("");
            (tag, class_attr.to_string())
        } else {
            (String::from("-"), String::new())
        };
        let text_align = match bx.text_align {
            libwebview::style::TextAlignVal::Left => "left",
            libwebview::style::TextAlignVal::Center => "center",
            libwebview::style::TextAlignVal::Right => "right",
            libwebview::style::TextAlignVal::Justify => "justify",
        };
        eprintln!(
            "[surf-host] box depth={} node={:?} tag={} class={:?} rel=({}, {}) abs=({}, {}) size=({}, {}) margin=({}, {}, {}, {}) text_align={} bg=0x{:08x}",
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
            text_align,
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

fn debug_dump_dom_elements(dom: &libwebview::dom::Dom) {
    for (node_id, node) in dom.nodes.iter().enumerate() {
        let NodeType::Element { tag, .. } = &node.node_type else {
            continue;
        };
        eprintln!(
            "[surf-host] dom node={} parent={:?} tag={} id={:?} class={:?} children={}",
            node_id,
            node.parent,
            tag.tag_name(),
            dom.attr(node_id, "id").unwrap_or(""),
            dom.attr(node_id, "class").unwrap_or(""),
            node.children.len()
        );
    }
}

fn debug_dump_interesting_styles(wv: &libwebview::WebView, dom: &libwebview::dom::Dom) {
    fn position_name(position: libwebview::style::Position) -> &'static str {
        match position {
            libwebview::style::Position::Static => "static",
            libwebview::style::Position::Relative => "relative",
            libwebview::style::Position::Absolute => "absolute",
            libwebview::style::Position::Fixed => "fixed",
            libwebview::style::Position::Sticky => "sticky",
        }
    }

    fn visibility_name(visibility: libwebview::style::Visibility) -> &'static str {
        match visibility {
            libwebview::style::Visibility::Visible => "visible",
            libwebview::style::Visibility::Hidden => "hidden",
            libwebview::style::Visibility::Collapse => "collapse",
        }
    }
    fn flex_direction_name(value: libwebview::style::FlexDirection) -> &'static str {
        match value {
            libwebview::style::FlexDirection::Row => "row",
            libwebview::style::FlexDirection::RowReverse => "row-reverse",
            libwebview::style::FlexDirection::Column => "column",
            libwebview::style::FlexDirection::ColumnReverse => "column-reverse",
        }
    }
    fn justify_content_name(value: libwebview::style::JustifyContent) -> &'static str {
        match value {
            libwebview::style::JustifyContent::FlexStart => "flex-start",
            libwebview::style::JustifyContent::FlexEnd => "flex-end",
            libwebview::style::JustifyContent::Center => "center",
            libwebview::style::JustifyContent::SpaceBetween => "space-between",
            libwebview::style::JustifyContent::SpaceAround => "space-around",
            libwebview::style::JustifyContent::SpaceEvenly => "space-evenly",
        }
    }
    fn align_items_name(value: libwebview::style::AlignItems) -> &'static str {
        match value {
            libwebview::style::AlignItems::FlexStart => "flex-start",
            libwebview::style::AlignItems::FlexEnd => "flex-end",
            libwebview::style::AlignItems::Center => "center",
            libwebview::style::AlignItems::Stretch => "stretch",
            libwebview::style::AlignItems::Baseline => "baseline",
        }
    }
    fn align_content_name(value: libwebview::style::AlignContent) -> &'static str {
        match value {
            libwebview::style::AlignContent::FlexStart => "flex-start",
            libwebview::style::AlignContent::FlexEnd => "flex-end",
            libwebview::style::AlignContent::Center => "center",
            libwebview::style::AlignContent::SpaceBetween => "space-between",
            libwebview::style::AlignContent::SpaceAround => "space-around",
            libwebview::style::AlignContent::SpaceEvenly => "space-evenly",
            libwebview::style::AlignContent::Stretch => "stretch",
        }
    }

    const INTERESTING_CLASSES: &[&str] = &[
        "skip-link",
        "page-content",
        "main-content",
        "block",
        "block__layout-wrapper",
        "layout",
        "stage-teaser",
        "teaser__image",
        "page-footer",
        "nav-list--main",
        "nav_btn--type-main",
        "nav_btn__text",
        "A7sPV",
        "KWUYAe",
        "oMByyf",
        "UbbAWe",
        "SDkEP",
        "RNNXgb",
        "tbsYnb",
        "JZzhke",
        "a4bIc",
        "gLFyf",
        "fM33ce",
        "BKRPef",
        "WC2Die",
        "plR5qb",
        "L3eUgb",
        "LLD4me",
        "k1zIA",
        "rSk4se",
        "LS8OJ",
        "yr19Zb",
        "ikrT4e",
        "om7nvf",
        "A8SBwf",
    ];

    for (node_id, _) in dom.nodes.iter().enumerate() {
        let Some(tag) = dom.tag(node_id) else {
            continue;
        };
        let id_attr = dom.attr(node_id, "id").unwrap_or("");
        let class_attr = dom.attr(node_id, "class").unwrap_or("");
        let is_interesting_id = matches!(
            id_attr,
            "app" | "main" | "superbannerWrapper" | "skyWrapper" | "billboardWrapper"
        );
        let is_main_nav_item = tag == Tag::Li
            && dom
                .get(node_id)
                .parent
                .and_then(|parent| dom.attr(parent, "class"))
                .map(|class_attr| {
                    class_attr
                        .split_ascii_whitespace()
                        .any(|class| class == "nav-list--main")
                })
                .unwrap_or(false);
        let is_interesting_class = class_attr
            .split_ascii_whitespace()
            .any(|class| INTERESTING_CLASSES.contains(&class));
        if !is_interesting_id && !is_interesting_class && !is_main_nav_item {
            continue;
        }
        let Some(style) = wv.resolved_style_ref(node_id) else {
            continue;
        };
        let shadow_info = style
            .box_shadows
            .first()
            .map(|s| {
                format!(
                    " first_shadow=(x:{} y:{} blur:{} spread:{} color=0x{:08x} inset:{})",
                    s.offset_x, s.offset_y, s.blur, s.spread, s.color, s.inset
                )
            })
            .unwrap_or_else(String::new);
        eprintln!(
            "[surf-host] interesting-style node={} tag={} id={:?} class={:?} bounds={:?} display={:?} position={} visibility={} overflow=({:?},{:?}) flex=({},{},{:?}/{:?}) flexdir={:?} justify={:?} align={:?} align_content={:?} width={:?} width_pct={:?} width_calc={:?} height={:?} height_pct={:?} height_calc={:?} min=({:?},{:?}) max=({:?},{:?}) margin=({:?},{:?},{:?},{:?}) margin_auto=({},{},{},{}) padding=({},{},{},{}) grid_rows={} grid_cols={} border_w=({},{},{},{}) border_c=({:#010x},{:#010x},{:#010x},{:#010x}) radius=({},{},{},{}) z={} opacity={:.3} shadows={}{}",
            node_id,
            tag.tag_name(),
            id_attr,
            class_attr,
            wv.node_bounds(node_id),
            style.display,
            position_name(style.position),
            visibility_name(style.visibility),
            style.overflow_x,
            style.overflow_y,
            style.flex_grow,
            style.flex_shrink,
            style.flex_basis,
            style.flex_basis_pct,
            flex_direction_name(style.flex_direction),
            justify_content_name(style.justify_content),
            align_items_name(style.align_items),
            align_content_name(style.align_content),
            style.width,
            style.width_pct,
            style.width_calc,
            style.height,
            style.height_pct,
            style.height_calc,
            style.min_width,
            style.min_height,
            style.max_width,
            style.max_height,
            style.margin_top,
            style.margin_right,
            style.margin_bottom,
            style.margin_left,
            style.margin_top_auto,
            style.margin_right_auto,
            style.margin_bottom_auto,
            style.margin_left_auto,
            style.padding_top,
            style.padding_right,
            style.padding_bottom,
            style.padding_left,
            style.grid_template_rows.len(),
            style.grid_template_columns.len(),
            style.border_width,
            style.border_width,
            style.border_width,
            style.border_width,
            style.border_color,
            style.border_color,
            style.border_color,
            style.border_color,
            style.border_top_left_radius,
            style.border_top_right_radius,
            style.border_bottom_right_radius,
            style.border_bottom_left_radius,
            style.z_index,
            style.opacity,
            style.box_shadows.len(),
            shadow_info
        );
    }
}

fn debug_dump_table_styles(wv: &libwebview::WebView, dom: &libwebview::dom::Dom) {
    for (node_id, _) in dom.nodes.iter().enumerate() {
        let Some(tag) = dom.tag(node_id) else {
            continue;
        };
        if !matches!(tag, Tag::Table | Tag::Td | Tag::Tr | Tag::Tbody) {
            continue;
        }
        let Some(style) = wv.resolved_style_ref(node_id) else {
            continue;
        };
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
        let Some(id_attr) = dom.attr(node_id, "id") else {
            continue;
        };
        if !matches!(id_attr, "wrapper" | "div1" | "div2" | "reference" | "inner") {
            continue;
        }
        let Some(style) = wv.resolved_style_ref(node_id) else {
            continue;
        };
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
        let dom = match wv.dom() {
            Some(d) => d,
            None => return,
        };
        let mut links = Vec::new();
        for (i, node) in dom.nodes.iter().enumerate() {
            if let NodeType::Element { tag: Tag::Link, .. } = &node.node_type {
                let rel = dom.attr(i, "rel").unwrap_or("");
                if !rel.eq_ignore_ascii_case("stylesheet") {
                    continue;
                }
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
                let sheet = libwebview::css::parse_stylesheet(&css_text);
                if std::env::var("SURF_DEBUG_HEISE").ok().as_deref() == Some("1") {
                    eprintln!(
                        "[surf-host] parsed CSS: rules={} media={} layers={} imports={}",
                        sheet.rules.len(),
                        sheet.media_rules.len(),
                        sheet.layer_order.len(),
                        sheet.imports.len()
                    );
                    fn selector_has_class(
                        sel: &libwebview::css::Selector,
                        class_name: &str,
                    ) -> bool {
                        match sel {
                            libwebview::css::Selector::Simple(simple)
                            | libwebview::css::Selector::Descendant(_, simple)
                            | libwebview::css::Selector::Child(_, simple)
                            | libwebview::css::Selector::AdjacentSibling(_, simple)
                            | libwebview::css::Selector::GeneralSibling(_, simple) => {
                                simple.classes.iter().any(|cls| cls == class_name)
                            }
                            libwebview::css::Selector::Universal => false,
                        }
                    }
                    let matching_media = sheet
                        .media_rules
                        .iter()
                        .filter(|mr| {
                            libwebview::css::evaluate_media_query(&mr.query, 1365, 200)
                                && mr.rules.iter().any(|rule| {
                                    rule.selectors.iter().any(|sel| {
                                        selector_has_class(sel, "xl:inline")
                                            || selector_has_class(sel, "md:hidden")
                                            || selector_has_class(sel, r"xl\:inline")
                                            || selector_has_class(sel, r"md\:hidden")
                                    })
                                })
                        })
                        .count();
                    eprintln!(
                        "[surf-host] matching responsive media buckets={}",
                        matching_media
                    );
                }
                wv.add_parsed_stylesheet(sheet);

                // @import URLs from this stylesheet
                let imports: Vec<String> = wv
                    .last_stylesheet_imports()
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
                let font_faces: Vec<_> = wv
                    .last_stylesheet_font_faces()
                    .iter()
                    .map(|ff| (ff.family.clone(), ff.src_url.clone()))
                    .collect();
                for (family, src) in &font_faces {
                    if src.is_empty() {
                        continue;
                    }
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
        let font_faces: Vec<_> = wv
            .all_font_faces()
            .iter()
            .map(|ff| (ff.family.clone(), ff.src_url.clone()))
            .collect();
        for (family, src) in &font_faces {
            if src.is_empty() {
                continue;
            }
            if wv.web_font_id(&family).is_some() {
                continue;
            } // already loaded
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
        let dom = match wv.dom() {
            Some(d) => d,
            None => {
                wv.relayout();
                return;
            }
        };
        let mut svgs = Vec::new();
        for (i, node) in dom.nodes.iter().enumerate() {
            if let NodeType::Element {
                tag: Tag::Svg,
                attrs,
            } = &node.node_type
            {
                let Some(inner) = inline_svg_inner_markup(dom, i) else {
                    continue;
                };
                let attr_list: Vec<(String, String)> = attrs
                    .iter()
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
            svg_markup.push_str(canonical_svg_attr_name(name));
            svg_markup.push_str("=\"");
            // Escape attribute value quotes.
            for ch in value.chars() {
                if ch == '"' {
                    svg_markup.push_str("&quot;");
                } else {
                    svg_markup.push(ch);
                }
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
        let svg_markup = expand_external_svg_uses(&svg_markup, base_url);
        let svg_markup = apply_svg_inherited_color(
            svg_markup,
            wv.resolved_style_ref(*node_id).map(|style| style.color),
        );
        if std::env::var("SURF_DEBUG_INLINE_SVG_NODE")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            == Some(*node_id)
        {
            eprintln!(
                "[surf-host] inline SVG node={} attrs={:?} markup={}",
                node_id, attrs, svg_markup
            );
        }

        if let Some((pixels, w, h)) = decode_svg(svg_markup.as_bytes()) {
            // Key format: __svg_<node_id>__ — must match svg_inline_key() in layout/mod.rs
            let key = format!("__svg_{}__", node_id);
            eprintln!(
                "[surf-host] rasterized inline SVG node={} {}x{}",
                node_id, w, h
            );
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
    if std::env::var_os("SURF_DEBUG_DOM_ELEMENTS").is_some() {
        if let Some(dom) = wv.dom() {
            debug_dump_dom_elements(dom);
        }
    }
    if std::env::var_os("SURF_DEBUG_INTERESTING_STYLES").is_some() {
        if let Some(dom) = wv.dom() {
            debug_dump_interesting_styles(wv, dom);
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
    node_id: usize,
    priority_y: i32,
}

struct PendingImages {
    receiver: mpsc::Receiver<ImageLoadResult>,
    done: bool,
}

impl PendingImages {
    fn empty() -> Self {
        let (_tx, rx) = mpsc::channel();
        PendingImages {
            receiver: rx,
            done: true,
        }
    }

    /// Non-blocking: returns any images that have finished loading.
    fn poll(&mut self) -> Vec<ImageLoadResult> {
        let mut results = Vec::new();
        loop {
            match self.receiver.try_recv() {
                Ok(r) => results.push(r),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.done = true;
                    break;
                }
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
        let dom = match wv.dom() {
            Some(d) => d,
            None => return PendingImages::empty(),
        };
        let mut infos = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for (i, node) in dom.nodes.iter().enumerate() {
            let is_image_like = matches!(&node.node_type, NodeType::Element { tag: Tag::Img, .. })
                || dom.has_tag_name(i, "a-img");
            if !is_image_like {
                continue;
            }

            if let Some(src) = dom.image_url(i) {
                if src.is_empty() || src.starts_with("data:") {
                    continue;
                }
                if !seen.insert(src.to_string()) {
                    continue;
                }
                let abs_url = resolve_url(base_url, &src);
                let bounds = wv.node_bounds(i);
                let priority_y = bounds.map(|(_, y, _, _)| y).unwrap_or(i32::MAX);
                let bounds_hint = bounds.and_then(|(_, _, w, h)| {
                    if w > 0 && h > 0 {
                        Some((w as u32, h as u32))
                    } else {
                        None
                    }
                });
                let target_w: Option<u32> = bounds_hint.map(|(w, _)| w).or_else(|| {
                    dom.attr(i, "width")
                        .and_then(|s| s.trim().trim_end_matches("px").parse().ok())
                });
                let target_h: Option<u32> = bounds_hint.map(|(_, h)| h).or_else(|| {
                    dom.attr(i, "height")
                        .and_then(|s| s.trim().trim_end_matches("px").parse().ok())
                });
                infos.push((abs_url, src, target_w, target_h, i, priority_y));
            }
        }
        for (i, _) in dom.nodes.iter().enumerate() {
            let Some(style) = wv.resolved_style_ref(i) else {
                continue;
            };
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
            let priority_y = wv.node_bounds(i).map(|(_, y, _, _)| y).unwrap_or(i32::MAX);
            infos.push((abs_url, bg_src.clone(), None, None, i, priority_y));
        }
        infos
    };

    if img_infos.is_empty() {
        return PendingImages::empty();
    }

    eprintln!(
        "[surf-host] loading {} images in parallel...",
        img_infos.len()
    );
    let (tx, rx) = mpsc::channel();

    for (img_url, src_attr, tw, th, node_id, priority_y) in img_infos {
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
                        node_id,
                        priority_y,
                    });
                } else if std::env::var_os("SURF_DEBUG_IMAGES").is_some() && priority_y < 1500 {
                    eprintln!(
                        "[surf-host]   image decode failed node={} y={} bytes={} src={}",
                        node_id,
                        priority_y,
                        img_data.len(),
                        src_attr
                    );
                }
            } else if std::env::var_os("SURF_DEBUG_IMAGES").is_some() && priority_y < 1500 {
                eprintln!(
                    "[surf-host]   image fetch failed node={} y={} src={}",
                    node_id, priority_y, src_attr
                );
            }
        });
    }
    drop(tx); // Close sender so receiver knows when all threads finish

    PendingImages {
        receiver: rx,
        done: false,
    }
}

// ── JavaScript execution ────────────────────────────────────────────────────

/// Register a synchronous HTTP handler so that fetch()/XHR inside JS
/// can make real HTTP requests via ureq.
fn register_http_handler(wv: &mut libwebview::WebView) {
    use libjs::value::JsObject;
    use libjs::JsValue;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn native_http_handler(_vm: &mut libjs::Vm, args: &[JsValue]) -> JsValue {
        let method = args.get(0).map(|v| v.to_js_string()).unwrap_or_default();
        let url = args.get(1).map(|v| v.to_js_string()).unwrap_or_default();
        let _headers = args.get(2).map(|v| v.to_js_string()).unwrap_or_default();
        let body = args.get(3).map(|v| v.to_js_string()).unwrap_or_default();

        if url.is_empty() {
            let mut obj = JsObject::new();
            obj.set(String::from("status"), JsValue::Number(0.0));
            obj.set(
                String::from("statusText"),
                JsValue::String(String::from("Empty URL")),
            );
            obj.set(String::from("body"), JsValue::String(String::new()));
            return JsValue::Object(Rc::new(RefCell::new(obj)));
        }

        eprintln!("[js-http] {} {}", method, url);
        let result = if method == "POST" {
            ureq::post(&url)
                .set("User-Agent", SURF_HOST_USER_AGENT)
                .set("Content-Type", "application/x-www-form-urlencoded")
                .send_string(&body)
        } else {
            ureq::get(&url).set("User-Agent", SURF_HOST_USER_AGENT).call()
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
                obj.set(
                    String::from("statusText"),
                    JsValue::String(format!("{}", e)),
                );
                obj.set(String::from("body"), JsValue::String(String::new()));
                JsValue::Object(Rc::new(RefCell::new(obj)))
            }
        }
    }

    let handler = libjs::vm::native_fn("__http_handler", native_http_handler);
    wv.js_runtime()
        .engine()
        .set_global("__http_handler", handler);
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
            libwebview::js::ScriptEntry::External {
                src: src_url,
                mode: _,
            } => {
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

    eprintln!(
        "[js] {} scripts total ({} inline, {} external)",
        scripts.len(),
        inline_count,
        external_count
    );

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
    if !wv.has_timers() {
        return;
    }
    eprintln!(
        "[js] running timers for {}ms ({} timers pending)...",
        total_ms,
        wv.timer_count()
    );
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
    eprintln!(
        "[js] timer loop done ({} ms elapsed, timers remaining: {})",
        elapsed,
        wv.has_timers()
    );
}

// ── Image decoding ───────────────────────────────────────────────────────────

/// Maximum dimension for images without explicit target size (safety cap).
const MAX_DECODE_DIM: u32 = 2048;

/// Decode an image, optionally downscaling to target dimensions.
/// If target dimensions are specified and the source is >2x larger,
/// the image is resized before storing — saving significant memory.
fn decode_image_scaled(
    data: &[u8],
    target_w: Option<u32>,
    target_h: Option<u32>,
) -> Option<(Vec<u32>, u32, u32)> {
    if is_svg(data) {
        return decode_svg(data);
    }
    let img = match image::load_from_memory(data) {
        Ok(img) => img,
        Err(_) => return decode_image_scaled_libimage(data, target_w, target_h),
    };
    let (orig_w, orig_h) = image::GenericImageView::dimensions(&img);

    let (final_w, final_h) = compute_decode_size(orig_w, orig_h, target_w, target_h);

    let rgba = if final_w != orig_w || final_h != orig_h {
        image::imageops::resize(
            &img.to_rgba8(),
            final_w,
            final_h,
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

fn decode_image_scaled_libimage(
    data: &[u8],
    target_w: Option<u32>,
    target_h: Option<u32>,
) -> Option<(Vec<u32>, u32, u32)> {
    let info = libimage::jpeg::probe(data)
        .or_else(|| libimage::png::probe(data))
        .or_else(|| libimage::webp::probe(data))?;
    let pixel_count = (info.width as usize).checked_mul(info.height as usize)?;
    let mut pixels = vec![0u32; pixel_count];
    let mut scratch = vec![0u8; info.scratch_needed as usize];

    let rc = match info.format {
        libimage::types::FMT_JPEG => libimage::jpeg::decode(data, &mut pixels, &mut scratch),
        libimage::types::FMT_PNG => libimage::png::decode(data, &mut pixels, &mut scratch),
        libimage::types::FMT_WEBP => libimage::webp::decode(data, &mut pixels, &mut scratch),
        _ => return None,
    };
    if rc != libimage::types::ERR_OK {
        return None;
    }

    let (final_w, final_h) = compute_decode_size(info.width, info.height, target_w, target_h);
    if final_w == info.width && final_h == info.height {
        return Some((pixels, info.width, info.height));
    }
    Some((
        resize_argb_nearest(&pixels, info.width, info.height, final_w, final_h),
        final_w,
        final_h,
    ))
}

fn resize_argb_nearest(src: &[u32], src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> Vec<u32> {
    let mut dst = vec![0u32; (dst_w as usize).saturating_mul(dst_h as usize)];
    if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 {
        return dst;
    }
    for y in 0..dst_h {
        let sy = ((y as u64) * (src_h as u64) / (dst_h as u64)).min((src_h - 1) as u64) as usize;
        for x in 0..dst_w {
            let sx =
                ((x as u64) * (src_w as u64) / (dst_w as u64)).min((src_w - 1) as u64) as usize;
            dst[y as usize * dst_w as usize + x as usize] = src[sy * src_w as usize + sx];
        }
    }
    dst
}

/// Compute decode target size.  Downscales if source is >2x the target.
/// Caps at MAX_DECODE_DIM when no target hint is available.
fn compute_decode_size(
    orig_w: u32,
    orig_h: u32,
    target_w: Option<u32>,
    target_h: Option<u32>,
) -> (u32, u32) {
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
    let fallback_w = size.width() as u32;
    let fallback_h = size.height() as u32;
    let (w, h) = svg_intrinsic_raster_size(data).unwrap_or((fallback_w, fallback_h));
    if w == 0 || h == 0 {
        return None;
    }
    let (rw, rh) = if w > 1024 || h > 1024 {
        let scale = 1024.0 / (w.max(h) as f32);
        ((w as f32 * scale) as u32, (h as f32 * scale) as u32)
    } else {
        (w, h)
    };
    let mut pixmap = resvg::tiny_skia::Pixmap::new(rw, rh)?;
    if let Some(bg) = parse_svg_root_background(data) {
        let color = resvg::tiny_skia::Color::from_rgba8(
            ((bg >> 16) & 0xFF) as u8,
            ((bg >> 8) & 0xFF) as u8,
            (bg & 0xFF) as u8,
            ((bg >> 24) & 0xFF) as u8,
        );
        pixmap.fill(color);
    }
    let transform = resvg::tiny_skia::Transform::from_scale(
        rw as f32 / size.width() as f32,
        rh as f32 / size.height() as f32,
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let pixels: Vec<u32> = pixmap
        .pixels()
        .iter()
        .map(|p| {
            ((p.alpha() as u32) << 24)
                | ((p.red() as u32) << 16)
                | ((p.green() as u32) << 8)
                | (p.blue() as u32)
        })
        .collect();
    Some((pixels, rw, rh))
}

fn apply_svg_inherited_color(svg: String, color: Option<u32>) -> String {
    let Some(color) = color else {
        return svg;
    };
    let rgb = color & 0x00FF_FFFF;
    let hex = format!("#{:06x}", rgb);
    let mut out = svg.replace("currentColor", &hex).replace("currentcolor", &hex);
    out = inject_svg_root_color(&out, &hex);
    inject_default_svg_fill(&out, &hex)
}

fn inline_svg_inner_markup(dom: &Dom, node_id: usize) -> Option<String> {
    let node = dom.nodes.get(node_id)?;

    for &child_id in &node.children {
        if let NodeType::Text(ref text) = dom.nodes[child_id].node_type {
            if !text.trim().is_empty() {
                return Some(text.clone());
            }
        }
    }

    let mut out = String::new();
    for &child_id in &node.children {
        serialize_svg_dom_node(dom, child_id, &mut out);
    }
    (!out.trim().is_empty()).then_some(out)
}

fn serialize_svg_dom_node(dom: &Dom, node_id: usize, out: &mut String) {
    let Some(node) = dom.nodes.get(node_id) else {
        return;
    };
    match &node.node_type {
        NodeType::Text(text) => escape_xml_text_into(text, out),
        NodeType::Element { tag, attrs } => {
            let name = dom
                .raw_tag_name(node_id)
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| tag.tag_name());
            out.push('<');
            out.push_str(name);
            for attr in attrs {
                if attr.name == "\0" {
                    continue;
                }
                out.push(' ');
                out.push_str(canonical_svg_attr_name(&attr.name));
                out.push_str("=\"");
                out.push_str(&escape_xml_attr(&attr.value));
                out.push('"');
            }
            out.push('>');
            for &child_id in &node.children {
                serialize_svg_dom_node(dom, child_id, out);
            }
            out.push_str("</");
            out.push_str(name);
            out.push('>');
        }
    }
}

fn escape_xml_text_into(value: &str, out: &mut String) {
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
}

fn inject_svg_root_color(svg: &str, hex: &str) -> String {
    let Some(svg_start) = svg.find("<svg") else {
        return String::from(svg);
    };
    let Some(tag_end_rel) = svg[svg_start..].find('>') else {
        return String::from(svg);
    };
    let tag_end = svg_start + tag_end_rel;
    let tag = &svg[svg_start..tag_end];
    if tag.contains(" style=") || tag.contains(" color=") || tag.contains("color:") {
        return String::from(svg);
    }
    let mut out = String::with_capacity(svg.len() + hex.len() + 22);
    out.push_str(&svg[..tag_end]);
    out.push_str(" style=\"color:");
    out.push_str(hex);
    out.push_str("\"");
    out.push_str(&svg[tag_end..]);
    out
}

fn inject_default_svg_fill(svg: &str, hex: &str) -> String {
    let mut out = String::with_capacity(svg.len() + 32);
    let mut cursor = 0usize;
    while let Some(rel_start) = svg[cursor..].find('<') {
        let start = cursor + rel_start;
        out.push_str(&svg[cursor..start]);
        let Some(rel_end) = svg[start..].find('>') else {
            out.push_str(&svg[start..]);
            return out;
        };
        let end = start + rel_end;
        let tag = &svg[start..end];
        if svg_tag_allows_default_fill(tag)
            && !tag.contains(" fill=")
            && !tag.contains("fill:")
            && !tag.contains(" stroke=")
        {
            let trimmed_len = tag.trim_end().len();
            if tag[..trimmed_len].ends_with('/') {
                out.push_str(&tag[..trimmed_len - 1]);
                out.push_str(" fill=\"");
                out.push_str(hex);
                out.push('"');
                out.push_str(&tag[trimmed_len - 1..]);
            } else {
                out.push_str(tag);
                out.push_str(" fill=\"");
                out.push_str(hex);
                out.push('"');
            }
        } else {
            out.push_str(tag);
        }
        out.push('>');
        cursor = end + 1;
    }
    out.push_str(&svg[cursor..]);
    out
}

fn svg_tag_allows_default_fill(tag: &str) -> bool {
    let name = svg_tag_name(tag);
    matches!(
        name.as_deref(),
        Some("path" | "rect" | "circle" | "ellipse" | "polygon" | "polyline")
    )
}

fn expand_external_svg_uses(svg: &str, base_url: &str) -> String {
    let mut out = String::with_capacity(svg.len());
    let mut cursor = 0usize;

    while let Some(rel_start) = svg[cursor..].find("<use") {
        let start = cursor + rel_start;
        out.push_str(&svg[cursor..start]);
        let Some(tag_end_rel) = svg[start..].find('>') else {
            out.push_str(&svg[start..]);
            return out;
        };
        let tag_end = start + tag_end_rel + 1;
        let use_tag = &svg[start..tag_end];

        let href = extract_attr_value(use_tag, "href")
            .or_else(|| extract_attr_value(use_tag, "xlink:href"));
        let replacement = href.and_then(|href| expand_external_svg_use(href, base_url));

        if let Some(replacement) = replacement {
            out.push_str(&replacement);
        } else {
            out.push_str(use_tag);
        }

        cursor = tag_end;
        if !use_tag.trim_end().ends_with("/>") {
            if let Some(close_rel) = svg[cursor..].find("</use>") {
                cursor += close_rel + "</use>".len();
            }
        }
    }

    out.push_str(&svg[cursor..]);
    out
}

fn expand_external_svg_use(href: &str, base_url: &str) -> Option<String> {
    let href = href.trim();
    if href.is_empty() || href.starts_with('#') || href.starts_with("data:") {
        return None;
    }
    let hash = href.rfind('#')?;
    let sprite_url = resolve_url(base_url, &href[..hash]);
    let symbol_id = &href[hash + 1..];
    if symbol_id.is_empty() {
        return None;
    }
    let sprite = fetch_resource(&sprite_url)?;
    let sprite = String::from_utf8(sprite).ok()?;
    extract_svg_fragment_by_id(&sprite, symbol_id)
}

fn extract_svg_fragment_by_id(sprite: &str, id: &str) -> Option<String> {
    let id_patterns = [
        format!("id=\"{}\"", id),
        format!("id='{}'", id),
        format!("id={}", id),
    ];
    let id_pos = id_patterns
        .iter()
        .filter_map(|needle| sprite.find(needle))
        .min()?;
    let tag_start = sprite[..id_pos].rfind('<')?;
    if sprite[tag_start..].starts_with("</") {
        return None;
    }
    let tag_end = tag_start + sprite[tag_start..].find('>')?;
    let open_tag = &sprite[tag_start..=tag_end];
    let tag_name = svg_tag_name(open_tag)?;
    let view_box = extract_attr_value(open_tag, "viewBox")
        .or_else(|| extract_attr_value(open_tag, "viewbox"));

    if open_tag.trim_end().ends_with("/>") {
        return Some(open_tag.to_string());
    }

    let close = format!("</{}>", tag_name);
    let inner_start = tag_end + 1;
    let inner_rel_end = sprite[inner_start..].find(&close)?;
    let inner = &sprite[inner_start..inner_start + inner_rel_end];

    if tag_name.eq_ignore_ascii_case("symbol") {
        let mut nested = String::from("<svg");
        if let Some(vb) = view_box {
            nested.push_str(" viewBox=\"");
            nested.push_str(&escape_xml_attr(vb));
            nested.push('"');
        }
        nested.push_str(" width=\"100%\" height=\"100%\" preserveAspectRatio=\"xMidYMid meet\">");
        nested.push_str(inner);
        nested.push_str("</svg>");
        Some(nested)
    } else {
        let mut group = String::from("<g>");
        group.push_str(inner);
        group.push_str("</g>");
        Some(group)
    }
}

fn svg_tag_name(open_tag: &str) -> Option<&str> {
    let body = open_tag.strip_prefix('<')?.trim_start();
    let end = body
        .find(|c: char| c.is_ascii_whitespace() || c == '>' || c == '/')
        .unwrap_or(body.len());
    (end > 0).then_some(&body[..end])
}

fn canonical_svg_attr_name(name: &str) -> &str {
    match name {
        "attributename" => "attributeName",
        "attributetype" => "attributeType",
        "basefrequency" => "baseFrequency",
        "calcmode" => "calcMode",
        "clippathunits" => "clipPathUnits",
        "diffuseconstant" => "diffuseConstant",
        "edgemode" => "edgeMode",
        "filterunits" => "filterUnits",
        "gradienttransform" => "gradientTransform",
        "gradientunits" => "gradientUnits",
        "kernelmatrix" => "kernelMatrix",
        "kernelunitlength" => "kernelUnitLength",
        "keypoints" => "keyPoints",
        "keysplines" => "keySplines",
        "keytimes" => "keyTimes",
        "lengthadjust" => "lengthAdjust",
        "limitingconeangle" => "limitingConeAngle",
        "markerheight" => "markerHeight",
        "markerunits" => "markerUnits",
        "markerwidth" => "markerWidth",
        "maskcontentunits" => "maskContentUnits",
        "maskunits" => "maskUnits",
        "numoctaves" => "numOctaves",
        "pathlength" => "pathLength",
        "patterncontentunits" => "patternContentUnits",
        "patterntransform" => "patternTransform",
        "patternunits" => "patternUnits",
        "pointsatx" => "pointsAtX",
        "pointsaty" => "pointsAtY",
        "pointsatz" => "pointsAtZ",
        "preservealpha" => "preserveAlpha",
        "preserveaspectratio" => "preserveAspectRatio",
        "primitiveunits" => "primitiveUnits",
        "refx" => "refX",
        "refy" => "refY",
        "repeatcount" => "repeatCount",
        "repeatdur" => "repeatDur",
        "requiredextensions" => "requiredExtensions",
        "requiredfeatures" => "requiredFeatures",
        "specularconstant" => "specularConstant",
        "specularexponent" => "specularExponent",
        "spreadmethod" => "spreadMethod",
        "startoffset" => "startOffset",
        "stddeviation" => "stdDeviation",
        "surfacescale" => "surfaceScale",
        "systemlanguage" => "systemLanguage",
        "tablevalues" => "tableValues",
        "targetx" => "targetX",
        "targety" => "targetY",
        "textlength" => "textLength",
        "viewbox" => "viewBox",
        "viewtarget" => "viewTarget",
        "xchannelselector" => "xChannelSelector",
        "ychannelselector" => "yChannelSelector",
        _ => name,
    }
}

fn escape_xml_attr(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("&quot;"),
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

fn parse_svg_root_background(data: &[u8]) -> Option<u32> {
    let text = std::str::from_utf8(data).ok()?;
    let svg_start = text.find("<svg")?;
    let after = &text[svg_start..];
    let tag_end = after.find('>')?;
    let svg_tag = &after[..tag_end];
    let style_attr = extract_attr_value(svg_tag, "style")?;
    parse_background_style(style_attr)
}

fn svg_intrinsic_raster_size(data: &[u8]) -> Option<(u32, u32)> {
    let text = std::str::from_utf8(data).ok()?;
    let svg_start = text.find("<svg")?;
    let after = &text[svg_start..];
    let tag_end = after.find('>')?;
    let svg_tag = &after[..tag_end];

    let width = extract_attr_value(svg_tag, "width").and_then(parse_svg_length_px);
    let height = extract_attr_value(svg_tag, "height").and_then(parse_svg_length_px);
    let viewbox = extract_attr_value(svg_tag, "viewBox")
        .or_else(|| extract_attr_value(svg_tag, "viewbox"))
        .and_then(parse_viewbox_size);
    let ratio = viewbox.map(|(w, h)| w as f32 / h as f32);

    let (w, h) = match (width, height, viewbox, ratio) {
        (Some(w), Some(h), _, _) => (w, h),
        (Some(w), None, _, Some(r)) if r > 0.0 => (w, ((w as f32) / r).round() as u32),
        (None, Some(h), _, Some(r)) if r > 0.0 => (((h as f32) * r).round() as u32, h),
        (Some(w), None, _, None) => (w, 150),
        (None, Some(h), _, None) => (300, h),
        (None, None, Some((vw, vh)), _) => (vw, vh),
        _ => return None,
    };

    Some((w.clamp(1, 4096), h.clamp(1, 4096)))
}

fn parse_svg_length_px(value: &str) -> Option<u32> {
    let value = value.trim();
    if value.is_empty() || value.ends_with('%') {
        return None;
    }
    let value = value.strip_suffix("px").unwrap_or(value).trim();
    let int_part = value.split('.').next()?.trim();
    let parsed = int_part.parse::<u32>().ok()?;
    (parsed > 0).then_some(parsed)
}

fn parse_viewbox_size(value: &str) -> Option<(u32, u32)> {
    let mut nums = [0f32; 4];
    let mut count = 0usize;
    for part in value
        .split(|c: char| c.is_ascii_whitespace() || c == ',')
        .filter(|s| !s.is_empty())
    {
        if count >= 4 {
            break;
        }
        nums[count] = part.parse::<f32>().ok()?;
        count += 1;
    }
    if count == 4 && nums[2] > 0.0 && nums[3] > 0.0 {
        Some((nums[2].round() as u32, nums[3].round() as u32))
    } else {
        None
    }
}

fn extract_attr_value<'a>(tag_text: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!("{name}=");
    let start = tag_text.find(&needle)? + needle.len();
    let rest = &tag_text[start..];
    let quote = rest.as_bytes().first().copied()?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let rest = &rest[1..];
    let end = rest.find(quote as char)?;
    Some(&rest[..end])
}

fn parse_background_style(style_attr: &str) -> Option<u32> {
    for decl in style_attr.split(';') {
        let mut parts = decl.splitn(2, ':');
        let name = parts.next()?.trim().to_ascii_lowercase();
        let value = parts.next()?.trim();
        if name == "background" || name == "background-color" {
            if let Some(color) = libwebview::css::try_parse_color_pub(value) {
                return Some(color);
            }
            if let Some(color) = libwebview::css::named_color_pub(&value.to_ascii_lowercase()) {
                return Some(color);
            }
        }
    }
    None
}

// ── Pixel extraction ─────────────────────────────────────────────────────────

fn extract_pixels(
    wv: &libwebview::WebView,
    fb: &mut [u32],
    width: usize,
    height: usize,
    scroll_y: i32,
) {
    // Tile canvases have positions set by the renderer (pos_y = row * 256).
    // We composite each canvas at its actual position, adjusted by scroll.
    for canvas_id in wv.tile_canvas_ids() {
        if let Some((pixels, cw, ch, _px, py)) = libanyui_client::host_get_canvas_pixels(canvas_id)
        {
            let cw = cw as usize;
            let ch = ch as usize;
            if cw == 0 || ch == 0 || pixels.len() < cw * ch {
                continue;
            }

            // Canvas position in document coordinates, adjusted by scroll
            let canvas_top = py - scroll_y;
            let canvas_bottom = canvas_top + ch as i32;

            // Skip if entirely outside viewport
            if canvas_bottom <= 0 || canvas_top >= height as i32 {
                continue;
            }

            let src_start = if canvas_top < 0 {
                (-canvas_top) as usize
            } else {
                0
            };
            let dst_start = if canvas_top > 0 {
                canvas_top as usize
            } else {
                0
            };

            for row in src_start..ch {
                let dst_y = dst_start + row - src_start;
                if dst_y >= height {
                    break;
                }
                let copy_w = cw.min(width);
                let src_off = row * cw;
                let dst_off = dst_y * width;
                fb[dst_off..dst_off + copy_w].copy_from_slice(&pixels[src_off..src_off + copy_w]);
            }
        }
    }
}
