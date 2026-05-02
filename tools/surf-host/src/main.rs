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
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

const SURF_HOST_USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124 Safari/537.36 Surf/1.0";
const HOST_SYNC_WEB_FONT_LIMIT: usize = 6;
const HOST_VIEWPORT_RENDER_PASS_LIMIT: usize = 128;
const HOST_SCREENSHOT_TIMER_CALLBACK_BUDGET: usize = 64;

// ── CLI args ─────────────────────────────────────────────────────────────────

struct Args {
    url: String,
    width: u32,
    height: u32,
    screenshot: Option<String>,
    fullpage: bool,
    bottom: bool,
    click: Option<(i32, i32)>,
    eval_sources: Vec<String>,
    delay_ms: u64,
    y_range: Option<(u32, u32)>, // (start, end) in pixels
    minifb: bool,
    js_enabled: bool,
    load_web_fonts: bool,
    remote_listen: Option<String>,
    image_backend: ImageBackend,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImageBackend {
    Host,
    Anyos,
}

#[derive(Clone, Debug)]
struct HostCookie {
    name: String,
    value: String,
    domain: String,
    path: String,
    secure: bool,
}

#[derive(Default)]
struct HostCookieJar {
    cookies: Vec<HostCookie>,
}

struct HostUrlParts {
    scheme: String,
    host: String,
    path: String,
}

impl HostCookieJar {
    fn store_from_document_cookie(&mut self, cookie: &str, request_url: &str) {
        let Some(parts) = parse_host_url(request_url) else {
            return;
        };
        self.store(cookie, &parts.host, &parts.path);
    }

    fn store(&mut self, header: &str, request_host: &str, request_path: &str) {
        let mut parts = header.split(';');
        let Some(first) = parts.next() else {
            return;
        };
        let Some((name, value)) = first.split_once('=') else {
            return;
        };
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        if std::env::var_os("SURF_HOST_DEBUG_COOKIES").is_some() {
            eprintln!(
                "[surf-host] document.cookie set {}={} ({} bytes)",
                name,
                value.trim(),
                value.trim().len()
            );
        }
        let mut domain = request_host.to_ascii_lowercase();
        let mut path = default_cookie_path(request_path);
        let mut secure = false;

        for attr in parts {
            let attr = attr.trim();
            if attr.eq_ignore_ascii_case("secure") {
                secure = true;
                continue;
            }
            if let Some((key, val)) = attr.split_once('=') {
                let key = key.trim();
                let val = val.trim().trim_matches('"');
                if key.eq_ignore_ascii_case("domain") {
                    domain = val.trim_start_matches('.').to_ascii_lowercase();
                } else if key.eq_ignore_ascii_case("path") && val.starts_with('/') {
                    path = val.to_string();
                }
            }
        }

        if let Some(existing) = self
            .cookies
            .iter_mut()
            .find(|c| c.name == name && c.domain == domain && c.path == path)
        {
            existing.value = value.trim().to_string();
            existing.secure = secure;
            return;
        }

        self.cookies.push(HostCookie {
            name: name.to_string(),
            value: value.trim().to_string(),
            domain,
            path,
            secure,
        });
    }

    fn cookie_header_for_url(&self, url: &str) -> Option<String> {
        let parts = parse_host_url(url)?;
        let is_secure = parts.scheme == "https";
        let mut out = String::new();
        for cookie in &self.cookies {
            if cookie.secure && !is_secure {
                continue;
            }
            if !domain_matches(&parts.host, &cookie.domain) {
                continue;
            }
            if !parts.path.starts_with(&cookie.path) {
                continue;
            }
            if !out.is_empty() {
                out.push_str("; ");
            }
            out.push_str(&cookie.name);
            out.push('=');
            out.push_str(&cookie.value);
        }
        if out.is_empty() {
            None
        } else {
            Some(out)
        }
    }
}

fn parse_host_url(url: &str) -> Option<HostUrlParts> {
    let (scheme, rest) = url.split_once("://")?;
    let host_end = rest
        .find(|ch| ch == '/' || ch == '?' || ch == '#')
        .unwrap_or(rest.len());
    let host_port = &rest[..host_end];
    let host = host_port
        .split('@')
        .last()?
        .split(':')
        .next()?
        .to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    let tail = &rest[host_end..];
    let path = if tail.starts_with('/') {
        let end = tail.find(|ch| ch == '?' || ch == '#').unwrap_or(tail.len());
        tail[..end].to_string()
    } else {
        String::from("/")
    };
    Some(HostUrlParts {
        scheme: scheme.to_ascii_lowercase(),
        host,
        path,
    })
}

fn default_cookie_path(request_path: &str) -> String {
    if !request_path.starts_with('/') {
        return String::from("/");
    }
    match request_path.rfind('/') {
        Some(0) | None => String::from("/"),
        Some(idx) => request_path[..idx].to_string(),
    }
}

fn domain_matches(host: &str, domain: &str) -> bool {
    host == domain || host.ends_with(&format!(".{}", domain))
}

fn parse_args() -> Args {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-?") {
        eprintln!("Usage: surf-host [url] [options]");
        eprintln!();
        eprintln!("Options:");
        eprintln!("  --screenshot <path.png>   Save screenshot and exit");
        eprintln!("  --fullpage                Capture entire page height (not just viewport)");
        eprintln!(
            "  --bottom                  Capture the bottom viewport after laying out the page"
        );
        eprintln!("  -y <start-end>            Capture Y range, e.g. -y 400-900");
        eprintln!("  --delay <ms>              Wait before screenshot (default: 0)");
        eprintln!("  --width <px>              Viewport width (default: 1024)");
        eprintln!("  --height <px>             Viewport height (default: 768)");
        eprintln!("  --minifb                  Use the legacy minifb window instead of egui");
        eprintln!("  --no-js                   Disable JavaScript execution");
        eprintln!("  --no-web-fonts            Skip @font-face downloads");
        eprintln!("  --anyos-image-path        Decode images/SVGs through libimage/libsvg only");
        eprintln!("  --libimage-only           Alias for --anyos-image-path");
        eprintln!("  --remote-listen <addr>    Listen for text commands (default: 127.0.0.1:8787)");
        eprintln!("  --eval <js>               Evaluate JavaScript before screenshot capture");
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
        bottom: false,
        click: None,
        eval_sources: Vec::new(),
        delay_ms: 0,
        y_range: None,
        minifb: false,
        js_enabled: true,
        load_web_fonts: true,
        remote_listen: Some(String::from("127.0.0.1:8787")),
        image_backend: ImageBackend::Host,
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
            "--bottom" => {
                a.bottom = true;
                i += 1;
                continue;
            }
            "--click" => {
                i += 1;
                if let Some(spec) = args.get(i) {
                    a.click = parse_click_point(spec);
                    if a.click.is_none() {
                        eprintln!("--click expects x,y, e.g. --click 660,482");
                        std::process::exit(1);
                    }
                }
            }
            "--eval" => {
                i += 1;
                if let Some(source) = args.get(i) {
                    a.eval_sources.push(source.clone());
                } else {
                    eprintln!("--eval requires a JavaScript source string");
                    std::process::exit(1);
                }
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
            "--no-web-fonts" => {
                a.load_web_fonts = false;
                i += 1;
                continue;
            }
            "--anyos-image-path" | "--libimage-only" => {
                a.image_backend = ImageBackend::Anyos;
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

fn parse_click_point(s: &str) -> Option<(i32, i32)> {
    let (x, y) = s.split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
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

fn render_viewport_bounded(wv: &mut libwebview::WebView, scroll_y: i32, context: &str) {
    for pass in 0..HOST_VIEWPORT_RENDER_PASS_LIMIT {
        if !wv.render_viewport_at(scroll_y) {
            return;
        }
        if pass + 1 == HOST_VIEWPORT_RENDER_PASS_LIMIT {
            eprintln!(
                "[surf-host] viewport render still pending after {} pass(es) during {}; saving partial frame",
                HOST_VIEWPORT_RENDER_PASS_LIMIT,
                context
            );
        }
    }
}

fn load_valid_web_font_data(family: &str, data: &[u8]) -> Option<u32> {
    let Some(font_id) = libfont_client::load_data(data) else {
        return None;
    };
    if web_font_is_renderable(font_id) {
        return Some(font_id);
    }
    libfont_client::unload(font_id);
    eprintln!(
        "[surf-host] rejected web font '{}' -> id {}: render validation failed",
        family, font_id
    );
    None
}

fn web_font_is_renderable(font_id: u32) -> bool {
    let (w, h) = libfont_client::measure(font_id, 24, "Ag");
    if w < 8 || h < 8 || w > 200 || h > 100 {
        return false;
    }

    let mut pixels = vec![0u32; 96 * 48];
    libfont_client::draw_string_buf(
        pixels.as_mut_ptr(),
        96,
        48,
        4,
        4,
        0xFFFF_FFFF,
        font_id,
        24,
        "Ag",
    );
    pixels
        .iter()
        .any(|px| ((*px >> 24) & 0xFF) != 0 || (*px & 0x00FF_FFFF) != 0)
}

// ── Navigation ────────────────────────────────────────────────────────────────

/// Load a URL: fetch HTML, load resources, run JS.  Returns (html, base_url).
/// This is the common pipeline used both at startup and during navigation.
fn load_page(
    wv: &mut libwebview::WebView,
    url: &str,
    js_enabled: bool,
    image_backend: ImageBackend,
    load_web_fonts: bool,
    cookies: &mut HostCookieJar,
) -> PendingImages {
    load_page_inner(
        wv,
        url,
        js_enabled,
        image_backend,
        load_web_fonts,
        cookies,
        0,
    )
}

fn load_page_inner(
    wv: &mut libwebview::WebView,
    url: &str,
    js_enabled: bool,
    image_backend: ImageBackend,
    load_web_fonts: bool,
    cookies: &mut HostCookieJar,
    redirect_depth: u8,
) -> PendingImages {
    eprintln!("[surf-host] loading: {}", url);
    eprintln!("[surf-host] image backend: {:?}", image_backend);
    let (html, base_url) = fetch_page(url, cookies);
    eprintln!("[surf-host] got {} bytes HTML", html.len());

    // Clear old page state including stylesheets so no styles bleed across pages.
    wv.clear();
    wv.clear_stylesheets();
    wv.set_url(&base_url);
    if let Some(cookie_hdr) = cookies.cookie_header_for_url(&base_url) {
        wv.js_runtime().set_cookies(&cookie_hdr);
    }
    wv.set_html_no_js(&html);
    if redirect_depth < 3 {
        if let Some(refresh_url) = wv.immediate_meta_refresh_url() {
            let abs = resolve_url(&base_url, &refresh_url);
            eprintln!("[meta-refresh] to {}", abs);
            return load_page_inner(
                wv,
                &abs,
                js_enabled,
                image_backend,
                load_web_fonts,
                cookies,
                redirect_depth + 1,
            );
        }
    }
    load_resources(wv, &base_url, image_backend, load_web_fonts); // CSS, fonts, SVGs (sync) + initial relayout
    let mut pending = if js_enabled {
        PendingImages::empty()
    } else {
        start_image_loading(wv, &base_url, image_backend) // images (async, parallel threads)
    };
    if js_enabled {
        if run_javascript(wv, &base_url, cookies) {
            wv.relayout();
        }
        run_js_debug_probes(wv);
        if wv.has_timers() {
            if std::env::var_os("SURF_HOST_SKIP_INITIAL_TIMERS").is_some() {
                eprintln!(
                    "[js] skipping initial timer drain ({} timer(s) pending; SURF_HOST_SKIP_INITIAL_TIMERS set)",
                    wv.timer_count()
                );
            } else {
                let timer_ms = std::env::var("SURF_HOST_INITIAL_TIMER_MS")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or_else(|| {
                        if std::env::var_os("SURF_HOST_RUN_INITIAL_TIMERS").is_some() {
                            5000
                        } else {
                            500
                        }
                    });
                run_js_timers(wv, &base_url, cookies, timer_ms);
            }
        }
        if rasterize_inline_svgs(wv, &base_url, image_backend) {
            wv.relayout();
        }
        let mut loaded_frames = HashSet::new();
        if load_iframe_snapshots(
            wv,
            &base_url,
            js_enabled,
            image_backend,
            load_web_fonts,
            cookies,
            &mut loaded_frames,
        ) {
            wv.relayout();
        }
        if std::env::var_os("SURF_DEBUG_DOM_ELEMENTS_AFTER_JS").is_some() {
            if let Some(dom) = wv.dom() {
                debug_dump_dom_elements(dom);
            }
        }
        if std::env::var_os("SURF_DEBUG_INTERESTING_STYLES_AFTER_JS").is_some() {
            if let Some(dom) = wv.dom() {
                debug_dump_interesting_styles(wv, dom);
            }
        }
        if redirect_depth < 3 {
            if let Some(nav) = wv.take_pending_navigation_requests().pop() {
                let abs = resolve_url(&base_url, &nav.url);
                eprintln!(
                    "[js-nav] {} to {}",
                    if nav.replace { "replace" } else { "navigate" },
                    abs
                );
                return load_page_inner(
                    wv,
                    &abs,
                    js_enabled,
                    image_backend,
                    load_web_fonts,
                    cookies,
                    redirect_depth + 1,
                );
            }
        }
        pending = start_image_loading(wv, &base_url, image_backend);
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

fn load_iframe_snapshots(
    wv: &mut libwebview::WebView,
    base_url: &str,
    js_enabled: bool,
    image_backend: ImageBackend,
    load_web_fonts: bool,
    cookies: &mut HostCookieJar,
    loaded_frames: &mut HashSet<String>,
) -> bool {
    let frames = {
        let Some(dom) = wv.dom() else {
            return false;
        };
        let mut frames = Vec::new();
        for (node_id, node) in dom.nodes.iter().enumerate() {
            if !matches!(
                node.node_type,
                NodeType::Element {
                    tag: Tag::Iframe,
                    ..
                }
            ) {
                continue;
            }
            let Some(src) = dom.attr(node_id, "src") else {
                continue;
            };
            if src.is_empty() || src.starts_with("about:") || src.starts_with("javascript:") {
                continue;
            }
            let url = resolve_url(base_url, src);
            let (mut w, mut h) = wv
                .node_bounds(node_id)
                .map(|(_, _, w, h)| (w, h))
                .unwrap_or((300, 150));
            let (style_w, style_h) = iframe_style_dimensions(dom.attr(node_id, "style"));
            if w <= 300 {
                if let Some(style_w) = style_w {
                    w = style_w;
                }
            }
            if h <= 150 {
                if let Some(style_h) = style_h {
                    h = style_h;
                }
            }
            if w <= 0 {
                w = dom
                    .attr(node_id, "width")
                    .and_then(parse_pxish_i32)
                    .unwrap_or(300);
            }
            if h <= 0 {
                h = dom
                    .attr(node_id, "height")
                    .and_then(parse_pxish_i32)
                    .unwrap_or(150);
            }
            frames.push((
                node_id,
                url,
                w.max(1).min(1920) as u32,
                h.max(1).min(1200) as u32,
            ));
        }
        frames
    };

    let mut changed = false;
    for (node_id, url, width, height) in frames {
        let cache_key = format!("{}:{}", node_id, url);
        if !loaded_frames.insert(cache_key) {
            continue;
        }
        let image_key = libwebview::iframe_snapshot_key(node_id);
        if wv.has_decoded_image(&image_key) {
            continue;
        }
        eprintln!(
            "[surf-host] loading iframe snapshot: node={} {}x{} {}",
            node_id, width, height, url
        );
        if let Some((pixels, w, h)) = render_iframe_snapshot(
            &url,
            width,
            height,
            js_enabled,
            image_backend,
            load_web_fonts,
            cookies,
        ) {
            wv.add_image(&image_key, pixels, w, h);
            changed = true;
        }
    }
    changed
}

fn iframe_style_dimensions(style: Option<&str>) -> (Option<i32>, Option<i32>) {
    let Some(style) = style else {
        return (None, None);
    };
    let mut width = None;
    let mut height = None;
    for decl in style.split(';') {
        let Some((name, value)) = decl.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        let parsed = parse_pxish_i32(value.trim());
        match name.as_str() {
            "width" => width = parsed.or(width),
            "height" => height = parsed.or(height),
            _ => {}
        }
    }
    (width, height)
}

fn parse_pxish_i32(value: &str) -> Option<i32> {
    let value = value.trim();
    if value.is_empty() || value.ends_with('%') || value.eq_ignore_ascii_case("auto") {
        return None;
    }
    let value = value.strip_suffix("px").unwrap_or(value).trim();
    let mut end = 0usize;
    let mut seen_digit = false;
    for (idx, ch) in value.char_indices() {
        if ch.is_ascii_digit() {
            seen_digit = true;
            end = idx + ch.len_utf8();
            continue;
        }
        if ch == '.' {
            end = idx;
            break;
        }
        if idx == 0 && (ch == '+' || ch == '-') {
            end = ch.len_utf8();
            continue;
        }
        break;
    }
    if !seen_digit {
        return None;
    }
    value[..end].parse::<i32>().ok()
}

fn render_iframe_snapshot(
    url: &str,
    width: u32,
    height: u32,
    js_enabled: bool,
    image_backend: ImageBackend,
    load_web_fonts: bool,
    cookies: &mut HostCookieJar,
) -> Option<(Vec<u32>, u32, u32)> {
    let width = width.max(1);
    let height = height.max(1);
    let (html, base_url) = fetch_page(url, cookies);
    let mut frame = libwebview::WebView::new(width, height);
    frame.set_url(&base_url);
    if let Some(cookie_hdr) = cookies.cookie_header_for_url(&base_url) {
        frame.js_runtime().set_cookies(&cookie_hdr);
    }
    frame.set_html_no_js(&html);
    load_resources(&mut frame, &base_url, image_backend, load_web_fonts);
    let run_snapshot_js = js_enabled && std::env::var_os("SURF_HOST_IFRAME_SNAPSHOT_JS").is_some();
    if run_snapshot_js {
        if run_javascript(&mut frame, &base_url, cookies) {
            frame.relayout();
        }
        if frame.has_timers() {
            run_js_timers(&mut frame, &base_url, cookies, 250);
            frame.relayout();
        }
    }
    let mut pending = start_image_loading(&frame, &base_url, image_backend);
    let deadline = Instant::now() + Duration::from_millis(1500);
    let mut images = Vec::new();
    while !pending.is_done() && Instant::now() < deadline {
        images.extend(pending.poll());
        std::thread::sleep(Duration::from_millis(10));
    }
    images.extend(pending.poll());
    images.sort_by(|a, b| b.node_id.cmp(&a.node_id));
    for image in images {
        frame.add_image(&image.src_attr, image.pixels, image.width, image.height);
    }
    frame.relayout();
    let mut pending_tiles = true;
    let mut passes = 0usize;
    while pending_tiles && passes < HOST_VIEWPORT_RENDER_PASS_LIMIT {
        pending_tiles = frame.render_viewport_at(0);
        passes += 1;
    }
    let mut pixels = vec![0xFFFFFFFFu32; (width * height) as usize];
    extract_pixels(&frame, &mut pixels, width as usize, height as usize, 0);
    Some((pixels, width, height))
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
    cookies: HostCookieJar,
    current_url: String,
    url_input: String,
    framebuffer: Vec<u32>,
    scroll_y: i32,
    texture: Option<egui::TextureHandle>,
    focused_control: Option<(u32, String)>,
    remote_rx: Option<mpsc::Receiver<RemoteRequest>>,
    js_enabled: bool,
    load_web_fonts: bool,
    image_backend: ImageBackend,
    loaded_frames: HashSet<String>,
    screenshot_count: u32,
    status: String,
    needs_redraw: bool,
    devtools_open: bool,
    devtools_tab: DevToolsTab,
    devtools_selected_node: Option<usize>,
    devtools_console_input: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DevToolsTab {
    Inspector,
    Console,
    Network,
}

impl BrowserHostApp {
    fn new(
        wv: libwebview::WebView,
        pending: PendingImages,
        cookies: HostCookieJar,
        current_url: String,
        remote_rx: Option<mpsc::Receiver<RemoteRequest>>,
        js_enabled: bool,
        load_web_fonts: bool,
        image_backend: ImageBackend,
    ) -> Self {
        let width = wv.viewport_width().max(1) as u32;
        let height = wv.viewport_height().max(1);
        Self {
            wv,
            pending,
            cookies,
            current_url: current_url.clone(),
            url_input: current_url,
            framebuffer: vec![0xFFFFFFFFu32; (width * height) as usize],
            scroll_y: 0,
            texture: None,
            focused_control: None,
            remote_rx,
            js_enabled,
            load_web_fonts,
            image_backend,
            loaded_frames: HashSet::new(),
            screenshot_count: 0,
            status: String::from("ready"),
            needs_redraw: true,
            devtools_open: false,
            devtools_tab: DevToolsTab::Inspector,
            devtools_selected_node: None,
            devtools_console_input: String::new(),
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
        self.loaded_frames.clear();
        self.pending = load_page(
            &mut self.wv,
            &abs,
            self.js_enabled,
            self.image_backend,
            self.load_web_fonts,
            &mut self.cookies,
        );
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
            if self.drain_js_side_effects() {
                return;
            }
        }
        if self.wv.tick_visual_only(16) {
            if load_iframe_snapshots(
                &mut self.wv,
                &self.current_url,
                self.js_enabled,
                self.image_backend,
                self.load_web_fonts,
                &mut self.cookies,
                &mut self.loaded_frames,
            ) {
                self.wv.relayout();
            }
            self.wv.relayout();
            self.needs_redraw = true;
            for line in self.wv.js_console() {
                eprintln!("[js:console:egui] {}", line);
            }
        }
    }

    fn drain_js_side_effects(&mut self) -> bool {
        if let Some(abs) =
            apply_host_js_mutations(&mut self.wv, &self.current_url, &mut self.cookies)
        {
            eprintln!("[js-nav] form submit to {}", abs);
            self.navigate(&abs);
            return true;
        }
        if let Some(nav) = self.wv.take_pending_navigation_requests().pop() {
            let abs = resolve_url(&self.current_url, &nav.url);
            eprintln!(
                "[js-nav] {} to {}",
                if nav.replace { "replace" } else { "navigate" },
                abs
            );
            self.navigate(&abs);
            return true;
        }
        false
    }

    fn submit_form_node(&mut self, node_id: usize) {
        if !self.wv.dispatch_submit_for_node(node_id) {
            let _ = self.drain_js_side_effects();
            return;
        }
        if self.drain_js_side_effects() {
            return;
        }
        let Some((action, method, _enctype)) = self.wv.form_action_for_node(node_id) else {
            return;
        };
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

    fn submit_form_control(&mut self, ctrl_id: u32) {
        let node_id = self
            .wv
            .form_controls()
            .iter()
            .find(|fc| fc.control_id == ctrl_id)
            .map(|fc| fc.node_id);
        if let Some(node_id) = node_id {
            self.submit_form_node(node_id);
        }
    }

    fn handle_page_keyboard(&mut self, ctx: &egui::Context) {
        let Some((ctrl_id, mut text)) = self.focused_control.take() else {
            return;
        };
        let events = ctx.input(|i| i.events.clone());
        let mut changed = false;
        let mut submit = false;
        for event in events {
            match event {
                egui::Event::Text(s) => {
                    for ch in s.chars() {
                        if ch != '\r' && ch != '\n' {
                            text.push(ch);
                            changed = true;
                        }
                    }
                }
                egui::Event::Key {
                    key: egui::Key::Backspace,
                    pressed: true,
                    ..
                } => {
                    text.pop();
                    changed = true;
                }
                egui::Event::Key {
                    key: egui::Key::Enter,
                    pressed: true,
                    ..
                } => {
                    submit = true;
                }
                _ => {}
            }
        }
        if changed {
            self.wv.set_form_control_text(ctrl_id, &text);
            self.needs_redraw = true;
        }
        if submit {
            if self.wv.dispatch_enter_for_control(ctrl_id) {
                self.submit_form_control(ctrl_id);
            } else {
                let _ = self.drain_js_side_effects();
            }
        } else {
            self.focused_control = Some((ctrl_id, text));
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
            self.submit_form_node(node_id);
        } else if let Some(href) = self.wv.hit_test_link_viewport(mx, my, self.scroll_y) {
            let href = href.to_string();
            self.focused_control = None;
            self.navigate(&href);
        } else {
            self.focused_control = None;
            self.needs_redraw = true;
        }
    }

    fn render_devtools(&mut self, ctx: &egui::Context) {
        if !self.devtools_open {
            return;
        }

        egui::SidePanel::right("surf_host_devtools")
            .resizable(true)
            .default_width(430.0)
            .min_width(300.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Developer Tools");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("×").clicked() {
                            self.devtools_open = false;
                        }
                    });
                });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.selectable_value(
                        &mut self.devtools_tab,
                        DevToolsTab::Inspector,
                        "Inspector",
                    );
                    ui.selectable_value(&mut self.devtools_tab, DevToolsTab::Console, "Konsole");
                    ui.selectable_value(&mut self.devtools_tab, DevToolsTab::Network, "Netzwerk");
                });
                ui.separator();

                match self.devtools_tab {
                    DevToolsTab::Inspector => self.render_devtools_inspector(ui),
                    DevToolsTab::Console => self.render_devtools_console(ui),
                    DevToolsTab::Network => self.render_devtools_network(ui),
                }
            });
    }

    fn render_devtools_inspector(&mut self, ui: &mut egui::Ui) {
        ui.columns(2, |cols| {
            egui::ScrollArea::vertical()
                .id_source("surf_host_dom_tree")
                .show(&mut cols[0], |ui| {
                    if let Some(dom) = self.wv.dom() {
                        if dom.nodes.is_empty() {
                            ui.label("DOM leer");
                        } else {
                            render_dom_node_tree(ui, dom, 0, &mut self.devtools_selected_node);
                        }
                    } else {
                        ui.label("Keine Seite geladen");
                    }
                });

            egui::ScrollArea::vertical()
                .id_source("surf_host_style_report")
                .show(&mut cols[1], |ui| {
                    if let Some(node_id) = self.devtools_selected_node {
                        if let Some(report) = self.wv.devtools_inspector_report(node_id) {
                            ui.add(
                                egui::TextEdit::multiline(&mut report.clone())
                                    .font(egui::TextStyle::Monospace)
                                    .desired_width(f32::INFINITY)
                                    .desired_rows(28),
                            );
                        } else {
                            ui.label("Kein Report fuer diesen Node");
                        }
                    } else {
                        ui.label("Element im DOM-Baum auswaehlen.");
                    }
                });
        });
    }

    fn render_devtools_console(&mut self, ui: &mut egui::Ui) {
        let mut console = self.wv.js_console().join("\n");
        egui::ScrollArea::vertical()
            .id_source("surf_host_console_output")
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut console)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(f32::INFINITY)
                        .desired_rows(24)
                        .interactive(false),
                );
            });
        ui.separator();
        ui.horizontal(|ui| {
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.devtools_console_input)
                    .hint_text("JavaScript auswerten")
                    .desired_width(f32::INFINITY),
            );
            let enter = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button("Run").clicked() || enter {
                let source = self.devtools_console_input.trim().to_string();
                if !source.is_empty() {
                    if self.wv.eval_js_for_devtools(&source) {
                        self.needs_redraw = true;
                    }
                    if let Some(abs) =
                        apply_host_js_mutations(&mut self.wv, &self.current_url, &mut self.cookies)
                    {
                        self.navigate(&abs);
                    }
                    self.devtools_console_input.clear();
                }
            }
        });
    }

    fn render_devtools_network(&mut self, ui: &mut egui::Ui) {
        ui.label("Host-Netzwerkpanel: Request-Erfassung wird als naechstes mit Surf geteilt.");
        ui.separator();
        ui.monospace(format!(
            "URL: {}\nViewport: {}x{}\nDokumenthoehe: {}\nScroll: {}",
            self.current_url,
            self.wv.viewport_width(),
            self.wv.viewport_height(),
            self.wv.total_height(),
            self.scroll_y
        ));
    }
}

impl eframe::App for BrowserHostApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.process_remote(ctx);
        self.poll_page_work();
        self.handle_page_keyboard(ctx);

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
                if ui.button("DevTools").clicked() {
                    self.devtools_open = !self.devtools_open;
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
                if response.hovered() {
                    if let Some(pos) = response.hover_pos() {
                        let mx = (pos.x - response.rect.min.x).round() as i32;
                        let my = (pos.y - response.rect.min.y).round() as i32;
                        if self.wv.handle_mouse_move_at_viewport(mx, my, self.scroll_y) {
                            self.needs_redraw = true;
                        }
                    }
                } else if self.wv.set_hovered_node(None) {
                    self.needs_redraw = true;
                }
                if response.clicked() {
                    if let Some(pos) = response.interact_pointer_pos() {
                        self.handle_browser_click(pos, response.rect.min);
                    }
                }
            }
        });

        self.render_devtools(ctx);

        if self.pending.is_done() && !self.wv.has_timers() && !self.wv.has_visual_work() {
            ctx.request_repaint_after(std::time::Duration::from_millis(33));
        } else {
            ctx.request_repaint();
        }
    }
}

fn render_dom_node_tree(
    ui: &mut egui::Ui,
    dom: &libwebview::dom::Dom,
    node_id: usize,
    selected: &mut Option<usize>,
) {
    let Some(node) = dom.nodes.get(node_id) else {
        return;
    };
    let label = dom_node_label(dom, node_id);
    let is_selected = *selected == Some(node_id);
    match &node.node_type {
        libwebview::dom::NodeType::Element { .. } if !node.children.is_empty() => {
            egui::CollapsingHeader::new(label)
                .id_source(("dom", node_id))
                .default_open(node_id == 0 || is_selected)
                .show(ui, |ui| {
                    if ui.selectable_label(is_selected, "select").clicked() {
                        *selected = Some(node_id);
                    }
                    for &child in &node.children {
                        render_dom_node_tree(ui, dom, child, selected);
                    }
                });
        }
        _ => {
            if ui.selectable_label(is_selected, label).clicked() {
                *selected = Some(node_id);
            }
        }
    }
}

fn dom_node_label(dom: &libwebview::dom::Dom, node_id: usize) -> String {
    let Some(node) = dom.nodes.get(node_id) else {
        return format!("#{}", node_id);
    };
    match &node.node_type {
        libwebview::dom::NodeType::Element { tag, .. } => {
            let mut s = String::from(tag.tag_name().to_ascii_lowercase());
            if let Some(id) = dom.attr(node_id, "id") {
                if !id.is_empty() {
                    s.push('#');
                    s.push_str(id);
                }
            }
            if let Some(class) = dom.attr(node_id, "class") {
                for cls in class.split_whitespace().take(3) {
                    s.push('.');
                    s.push_str(cls);
                }
            }
            s
        }
        libwebview::dom::NodeType::Text(text) => {
            let preview: String = text.trim().chars().take(32).collect();
            if preview.is_empty() {
                String::from("#text")
            } else {
                format!("#text {:?}", preview)
            }
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
    cookies: HostCookieJar,
    current_url: String,
    width: u32,
    height: u32,
    remote_listen: Option<String>,
    js_enabled: bool,
    load_web_fonts: bool,
    image_backend: ImageBackend,
) {
    let remote_rx = remote_listen.as_deref().and_then(start_remote_listener);
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([width as f32, height as f32])
            .with_title(format!("surf-host — {}", current_url)),
        renderer: eframe::Renderer::Wgpu,
        hardware_acceleration: eframe::HardwareAcceleration::Preferred,
        ..Default::default()
    };
    let app = BrowserHostApp::new(
        wv,
        pending,
        cookies,
        current_url,
        remote_rx,
        js_enabled,
        load_web_fonts,
        image_backend,
    );
    if let Err(err) = eframe::run_native(
        "surf-host",
        native_options,
        Box::new(move |_cc| Box::new(app)),
    ) {
        eprintln!("[surf-host] egui startup failed: {}", err);
        eprintln!("[surf-host] rerun with --minifb to use the CPU fallback shell");
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
    let mut cookies = HostCookieJar::default();
    let mut pending = load_page(
        &mut wv,
        &args.url,
        args.js_enabled,
        args.image_backend,
        args.load_web_fonts,
        &mut cookies,
    );
    let mut current_url = args.url.clone();

    // For screenshot mode: wait for all images before capturing
    if args.screenshot.is_some() {
        let mut results = pending.drain();
        results.sort_by(|a, b| b.node_id.cmp(&a.node_id));
        wv.relayout();
        let capture_window = if args.fullpage {
            None
        } else if let Some((y_start, y_end)) = args.y_range {
            Some((y_start as i32, y_end as i32))
        } else if args.bottom {
            let doc_h = wv.total_height().max(1);
            let start = (doc_h - height as i32).max(0);
            Some((start, doc_h))
        } else {
            Some((0, (height as i32).saturating_add(512)))
        };
        let debug_heise = std::env::var("SURF_DEBUG_HEISE").ok().as_deref() == Some("1");
        let mut added_images = 0usize;
        let mut skipped_images = 0usize;
        for r in results {
            let priority_y = wv
                .node_bounds(r.node_id)
                .map(|(_, y, _, _)| y)
                .unwrap_or(r.priority_y);
            if let Some((y_start, y_end)) = capture_window {
                let preload_top = y_start.saturating_sub(768);
                let preload_bottom = y_end.saturating_add(768);
                if priority_y != i32::MAX
                    && (priority_y < preload_top || priority_y > preload_bottom)
                {
                    skipped_images += 1;
                    continue;
                }
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
        render_viewport_bounded(&mut wv, 0, "initial frame");
        debug_log_image_bounds(&mut wv);
    }

    // Build initial framebuffer
    let mut framebuffer = vec![0xFFFFFFFFu32; (width * height) as usize];
    extract_pixels(&wv, &mut framebuffer, width as usize, height as usize, 0);

    // ── Screenshot-only mode ─────────────────────────────────────────────
    if let Some(ref path) = args.screenshot {
        if let Some((x, y)) = args.click {
            let mut hit_info = String::from("<none>");
            let hit_node_id = wv.hit_test_node_viewport(x, y, 0);
            if let Some(node_id) = hit_node_id {
                if let Some(dom) = wv.dom() {
                    let tag = dom
                        .tag(node_id)
                        .map(|tag| tag.tag_name())
                        .unwrap_or("<text>");
                    let class = dom.attr(node_id, "class").unwrap_or("");
                    hit_info = format!("node={} tag={} class={}", node_id, tag, class);
                }
            }
            let click_listener_count = wv
                .js_runtime()
                .event_listeners
                .iter()
                .filter(|listener| listener.event == "click")
                .count();
            let hit_click_listener_count = hit_node_id
                .map(|node_id| {
                    wv.js_runtime()
                        .event_listeners
                        .iter()
                        .filter(|listener| listener.node_id == node_id && listener.event == "click")
                        .count()
                })
                .unwrap_or(0);
            let allowed = wv.dispatch_click_at_viewport(x, y, 0);
            eprintln!(
                "[surf-host] scripted click x={} y={} hit={} click_listeners={} hit_click_listeners={} default_allowed={}",
                x, y, hit_info, click_listener_count, hit_click_listener_count, allowed
            );
            wv.run_timers(250);
            wv.tick(250);
            wv.relayout();
            render_viewport_bounded(&mut wv, 0, "after scripted click");
        }
        for source in &args.eval_sources {
            let changed = wv.eval_js_for_devtools(source);
            eprintln!("[surf-host] eval changed_dom={} source={}", changed, source);
            if let Some(abs) = apply_host_js_mutations(&mut wv, &current_url, &mut cookies) {
                eprintln!("[surf-host] eval navigation: {}", abs);
                current_url = abs.clone();
                pending = load_page(
                    &mut wv,
                    &abs,
                    args.js_enabled,
                    args.image_backend,
                    args.load_web_fonts,
                    &mut cookies,
                );
                for r in pending.drain() {
                    wv.add_image(&r.src_attr, r.pixels, r.width, r.height);
                }
                wv.relayout();
                render_viewport_bounded(&mut wv, 0, "after eval navigation");
                continue;
            }
            if changed {
                wv.relayout();
                render_viewport_bounded(&mut wv, 0, "after eval");
            }
        }
        if args.delay_ms > 0 {
            eprintln!(
                "[surf-host] waiting {}ms before screenshot (running timers)...",
                args.delay_ms
            );
            // Run timers in steps during the wait period so setTimeout/setInterval
            // callbacks fire (e.g. boot sequences, animations).
            let run_screenshot_timers =
                std::env::var_os("SURF_HOST_SKIP_SCREENSHOT_TIMERS").is_none();
            let step = 50u64;
            let mut waited = 0u64;
            while waited < args.delay_ms {
                if run_screenshot_timers && wv.has_timers() {
                    wv.run_timers_with_budget(step, HOST_SCREENSHOT_TIMER_CALLBACK_BUDGET);
                    if let Some(abs) = apply_host_js_mutations(&mut wv, &current_url, &mut cookies)
                    {
                        eprintln!("[surf-host] timer navigation: {}", abs);
                        current_url = abs.clone();
                        pending = load_page(
                            &mut wv,
                            &abs,
                            args.js_enabled,
                            args.image_backend,
                            args.load_web_fonts,
                            &mut cookies,
                        );
                        for r in pending.drain() {
                            wv.add_image(&r.src_attr, r.pixels, r.width, r.height);
                        }
                        wv.relayout();
                    }
                }
                wv.tick_visual_only(step);
                waited += step;
                // Sleep a small amount to avoid 100% CPU
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            // Final relayout after timers
            wv.relayout();
            if std::env::var_os("SURF_DEBUG_LAYOUT_BOXES_AFTER_WAIT").is_some() {
                if let (Some(root), Some(dom)) = (wv.layout_root_ref(), wv.dom()) {
                    debug_dump_boxes(dom, root, 0, 0, 0);
                }
            }
            if std::env::var_os("SURF_DEBUG_LAYOUT_TEXT_AFTER_WAIT").is_some() {
                if let Some(root) = wv.layout_root_ref() {
                    debug_dump_text_runs(root, 0);
                }
            }
            render_viewport_bounded(&mut wv, 0, "screenshot delay");
            debug_log_image_bounds(&mut wv);
            // Print a bounded console sample from timer callbacks. Pages with
            // 10ms consent/ad polling loops can otherwise spend more time
            // logging than rendering in screenshot tests.
            let console = wv.js_console();
            let mut printed = 0usize;
            let mut suppressed = 0usize;
            let mut last_line: Option<&str> = None;
            for line in console {
                if last_line == Some(line.as_str()) {
                    suppressed += 1;
                    continue;
                }
                if printed < 64 {
                    eprintln!("[js:console:timer] {}", line);
                    printed += 1;
                } else {
                    suppressed += 1;
                }
                last_line = Some(line);
            }
            if suppressed > 0 {
                eprintln!(
                    "[js:console:timer] suppressed {} repeated/excess line(s)",
                    suppressed
                );
            }
        }
        extract_pixels(&wv, &mut framebuffer, width as usize, height as usize, 0);
        if let Some((y_start, y_end)) = args.y_range {
            save_range_screenshot(&mut wv, width, y_start, y_end, path);
        } else if args.fullpage {
            save_fullpage_screenshot(&mut wv, width, path);
        } else if args.bottom {
            let doc_h = wv.total_height().max(1);
            let scroll_y = (doc_h - height as i32).max(0);
            eprintln!(
                "[surf-host] bottom: doc_h={} viewport={} scroll_y={}",
                doc_h, height, scroll_y
            );
            render_viewport_bounded(&mut wv, scroll_y, "bottom screenshot");
            extract_pixels(
                &wv,
                &mut framebuffer,
                width as usize,
                height as usize,
                scroll_y,
            );
            save_screenshot(&framebuffer, width, height, path);
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
            cookies,
            current_url,
            width,
            height,
            args.remote_listen.clone(),
            args.js_enabled,
            args.load_web_fonts,
            args.image_backend,
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
    let mut last_mouse_pos: Option<(i32, i32)> = None;

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
                pending = load_page(
                    &mut wv,
                    &abs,
                    args.js_enabled,
                    args.image_backend,
                    args.load_web_fonts,
                    &mut cookies,
                );
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
            let mut submit_requested = window.is_key_pressed(Key::Enter, KeyRepeat::No);
            for c in new_chars {
                if c == '\r' || c == '\n' {
                    submit_requested = true;
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

            if submit_requested {
                let node_id = wv
                    .form_controls()
                    .iter()
                    .find(|fc| fc.control_id == ctrl_id)
                    .map(|fc| fc.node_id)
                    .unwrap_or(0);
                if let Some(nav_url) =
                    submit_form_node_host(&mut wv, &current_url, &mut cookies, node_id)
                {
                    eprintln!("[enter] form submit → {}", nav_url);
                    navigate_to = Some(nav_url);
                }
            }
        } else {
            // Drain and discard typed chars when no field is focused.
            typed_chars.lock().unwrap().clear();
        }

        // ── Mouse click ─────────────────────────────────────────────────
        let mouse_pos_now = window
            .get_mouse_pos(MouseMode::Discard)
            .map(|(mx, my)| (mx as i32, my as i32));
        if mouse_pos_now != last_mouse_pos {
            last_mouse_pos = mouse_pos_now;
            if let Some((mx, my)) = mouse_pos_now {
                if wv.handle_mouse_move_at_viewport(mx, my, scroll_y) {
                    needs_redraw = true;
                }
            } else if wv.set_hovered_node(None) {
                needs_redraw = true;
            }
        }

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
                    if let Some(nav_url) =
                        submit_form_node_host(&mut wv, &current_url, &mut cookies, node_id)
                    {
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
fn submit_form_node_host(
    wv: &mut libwebview::WebView,
    current_url: &str,
    cookies: &mut HostCookieJar,
    node_id: usize,
) -> Option<String> {
    if !wv.dispatch_submit_for_node(node_id) {
        if let Some(nav_url) = apply_host_js_mutations(wv, current_url, cookies) {
            return Some(nav_url);
        }
        return wv
            .take_pending_navigation_requests()
            .pop()
            .map(|nav| resolve_url(current_url, &nav.url));
    }

    if let Some(nav_url) = apply_host_js_mutations(wv, current_url, cookies) {
        return Some(nav_url);
    }
    if let Some(nav) = wv.take_pending_navigation_requests().pop() {
        return Some(resolve_url(current_url, &nav.url));
    }

    submit_form_node_host_no_event(wv, current_url, node_id)
}

fn submit_form_node_host_no_event(
    wv: &mut libwebview::WebView,
    current_url: &str,
    node_id: usize,
) -> Option<String> {
    let (action, method, _enctype) = wv.form_action_for_node(node_id)?;
    let data = wv.collect_form_data_for_node(node_id);
    let query = form_encode(&data);
    let base = if action.is_empty() {
        current_url.to_string()
    } else {
        resolve_url(current_url, &action)
    };
    if method == "GET" && !query.is_empty() {
        Some(format!("{}?{}", base, query))
    } else {
        Some(base)
    }
}

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

fn fetch_page(url: &str, cookies: &mut HostCookieJar) -> (String, String) {
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

        let mut request = ureq::get(&full_url).set("User-Agent", SURF_HOST_USER_AGENT);
        if let Some(cookie_hdr) = cookies.cookie_header_for_url(&full_url) {
            eprintln!("[surf-host] sending cookies: {} bytes", cookie_hdr.len());
            request = request.set("Cookie", &cookie_hdr);
        }

        match request.call() {
            Ok(response) => {
                let final_url = response.get_url().to_string();
                let content_type = response.header("Content-Type").map(str::to_string);
                if let Some(parts) = parse_host_url(&final_url) {
                    for set_cookie in response.all("set-cookie") {
                        cookies.store(set_cookie, &parts.host, &parts.path);
                    }
                }
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
    if url.starts_with("data:") {
        return decode_data_uri(url);
    }

    let user_agent = if url.starts_with("https://fonts.googleapis.com/")
        || url.starts_with("http://fonts.googleapis.com/")
    {
        "curl/8.0"
    } else {
        SURF_HOST_USER_AGENT
    };

    // Check disk cache — fresh entries (< 24h) are served directly.
    let dir = disk_cache_dir();
    let key = url_cache_key(&format!("{}\n{}", url, user_agent));
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
    match ureq::get(url)
        .set("User-Agent", user_agent)
        .set("Accept", "*/*")
        .timeout(std::time::Duration::from_secs(8))
        .call()
    {
        Ok(resp) => {
            let mut buf = Vec::new();
            resp.into_reader().read_to_end(&mut buf).ok()?;
            // Save to disk cache
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(&data_path, &buf);
            Some(buf)
        }
        Err(err) => {
            eprintln!("[surf-host] fetch_resource failed: {} ({})", url, err);
            // Fall back to stale cache on network error
            std::fs::read(&data_path).ok()
        }
    }
}

pub fn resolve_url(base: &str, relative: &str) -> String {
    if relative.trim().is_empty() {
        if let Some(hash) = base.find('#') {
            return base[..hash].to_string();
        }
        return base.to_string();
    }
    if relative.starts_with("data:") {
        return relative.to_string();
    }
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
        let joined = if last_slash > after_proto {
            format!("{}/{}", &base[..last_slash], relative)
        } else {
            format!("{}/{}", base.trim_end_matches('/'), relative)
        };
        normalize_http_path(&joined)
    } else {
        format!("{}/{}", base, relative)
    }
}

fn normalize_http_path(url: &str) -> String {
    let Some(scheme_idx) = url.find("://") else {
        return url.to_string();
    };
    let path_start = url[scheme_idx + 3..]
        .find('/')
        .map(|i| scheme_idx + 3 + i)
        .unwrap_or(url.len());
    if path_start >= url.len() {
        return url.to_string();
    }
    let (origin, rest) = url.split_at(path_start);
    let (path_part, suffix) = match rest.find(['?', '#']) {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let mut segments: Vec<&str> = Vec::new();
    for seg in path_part.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            _ => segments.push(seg),
        }
    }
    let mut out = String::from(origin);
    out.push('/');
    out.push_str(&segments.join("/"));
    if path_part.ends_with('/') && !out.ends_with('/') {
        out.push('/');
    }
    out.push_str(suffix);
    out
}

fn resolve_css_resource_urls(css: &str, css_url: &str) -> String {
    let bytes = css.as_bytes();
    let mut out = String::with_capacity(css.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if !rest_starts_with_ci(bytes, i, b"url(") {
            out.push(bytes[i] as char);
            i += 1;
            continue;
        }

        out.push_str("url(");
        i += 4;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            out.push(bytes[i] as char);
            i += 1;
        }

        let quote = match bytes.get(i).copied() {
            Some(b'"') | Some(b'\'') => {
                let q = bytes[i];
                out.push(q as char);
                i += 1;
                Some(q)
            }
            _ => None,
        };

        let url_start = i;
        while i < bytes.len() {
            if let Some(q) = quote {
                if bytes[i] == q {
                    break;
                }
            } else if bytes[i] == b')' || bytes[i].is_ascii_whitespace() {
                break;
            }
            i += 1;
        }
        let raw_url = &css[url_start..i];
        out.push_str(&resolve_css_url(css_url, raw_url));

        if let Some(q) = quote {
            if i < bytes.len() && bytes[i] == q {
                out.push(q as char);
                i += 1;
            }
        }
        while i < bytes.len() && bytes[i] != b')' {
            out.push(bytes[i] as char);
            i += 1;
        }
        if i < bytes.len() {
            out.push(')');
            i += 1;
        }
    }
    out
}

fn resolve_css_url(css_url: &str, raw_url: &str) -> String {
    let trimmed = raw_url.trim();
    if trimmed.is_empty()
        || trimmed.starts_with('#')
        || trimmed.starts_with("data:")
        || trimmed.starts_with("blob:")
        || trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.starts_with("//")
    {
        return String::from(raw_url);
    }
    resolve_url(css_url, trimmed)
}

fn rest_starts_with_ci(haystack: &[u8], pos: usize, needle: &[u8]) -> bool {
    haystack
        .get(pos..pos.saturating_add(needle.len()))
        .is_some_and(|slice| slice.eq_ignore_ascii_case(needle))
}

fn base64_decode(input: &[u8]) -> Vec<u8> {
    fn val(b: u8) -> Option<u8> {
        match b {
            b'A'..=b'Z' => Some(b - b'A'),
            b'a'..=b'z' => Some(26 + b - b'a'),
            b'0'..=b'9' => Some(52 + b - b'0'),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }

    let mut out = Vec::new();
    let mut buf = 0u32;
    let mut bits = 0u32;
    for &b in input {
        if b == b'=' {
            break;
        }
        if matches!(b, b' ' | b'\n' | b'\r' | b'\t') {
            continue;
        }
        let Some(v) = val(b) else {
            continue;
        };
        buf = (buf << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xff) as u8);
        }
    }
    out
}

fn percent_decode_data_uri_payload(payload: &str) -> Vec<u8> {
    fn val(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(10 + b - b'a'),
            b'A'..=b'F' => Some(10 + b - b'A'),
            _ => None,
        }
    }

    let bytes = payload.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (val(bytes[i + 1]), val(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

fn decode_data_uri(src: &str) -> Option<Vec<u8>> {
    let rest = src.strip_prefix("data:")?;
    let comma = rest.find(',')?;
    let meta = &rest[..comma];
    let payload = &rest[comma + 1..];
    let bytes = if meta.contains(";base64") {
        base64_decode(payload.as_bytes())
    } else {
        percent_decode_data_uri_payload(payload)
    };
    (!bytes.is_empty()).then_some(bytes)
}

fn decode_font_data_uri(src: &str) -> Option<Vec<u8>> {
    let rest = src.strip_prefix("data:")?;
    let comma = rest.find(',')?;
    let meta = &rest[..comma];
    let mime = meta.split(';').next().unwrap_or("").trim();
    let is_font = mime.starts_with("font/")
        || mime.starts_with("application/font")
        || mime.starts_with("application/x-font")
        || mime.eq_ignore_ascii_case("application/octet-stream");
    if !is_font {
        return None;
    }
    decode_data_uri(src)
}

// ── Resource loading (DOM-based, identical to anyOS surf) ────────────────────

use libwebview::dom::{Dom, NodeType, Tag};

fn debug_dump_text_runs(bx: &libwebview::LayoutBox, depth: usize) {
    if let Some(text) = &bx.text {
        if !text.is_empty() {
            eprintln!(
                "[surf-host] text-run depth={} node={:?} x={} y={} w={} h={} font_id={} size={} color=0x{:08x} text={:?}",
                depth,
                bx.node_id,
                bx.x,
                bx.y,
                bx.width,
                bx.height,
                bx.custom_font_id,
                bx.font_size,
                bx.color,
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
            "[surf-host] box depth={} node={:?} tag={} class={:?} rel=({}, {}) abs=({}, {}) size=({}, {}) margin=({}, {}, {}, {}) text_align={} color=0x{:08x} bg=0x{:08x}",
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
            bx.color,
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
    fn background_clip_name(value: libwebview::style::BackgroundClipVal) -> &'static str {
        match value {
            libwebview::style::BackgroundClipVal::BorderBox => "border-box",
            libwebview::style::BackgroundClipVal::PaddingBox => "padding-box",
            libwebview::style::BackgroundClipVal::ContentBox => "content-box",
            libwebview::style::BackgroundClipVal::Text => "text",
        }
    }
    fn background_image_name(value: &libwebview::style::BackgroundImageVal) -> &'static str {
        match value {
            libwebview::style::BackgroundImageVal::None => "none",
            libwebview::style::BackgroundImageVal::Url(_) => "url",
            libwebview::style::BackgroundImageVal::LinearGradient { .. } => "linear-gradient",
            libwebview::style::BackgroundImageVal::RadialGradient { .. } => "radial-gradient",
            libwebview::style::BackgroundImageVal::ConicGradient { .. } => "conic-gradient",
        }
    }
    fn has_ancestor_with_class(dom: &libwebview::dom::Dom, node_id: usize, needle: &str) -> bool {
        let mut cur = dom.get(node_id).parent;
        while let Some(parent) = cur {
            if dom
                .attr(parent, "class")
                .map(|classes| classes.split_ascii_whitespace().any(|class| class == needle))
                .unwrap_or(false)
            {
                return true;
            }
            cur = dom.get(parent).parent;
        }
        false
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
        "xL0qi",
        "iydNQb",
        "mTurwe",
        "bvUkz",
        "u4Uk3c",
        "lTxWLe",
        "L3eUgb",
        "LLD4me",
        "k1zIA",
        "rSk4se",
        "LS8OJ",
        "yr19Zb",
        "ikrT4e",
        "om7nvf",
        "A8SBwf",
        "collection-module",
        "module-content",
        "unit-wrapper",
        "unit-copy-wrapper",
        "unit-image-wrapper",
        "unit-image",
        "headline",
        "subhead",
        "Header-Navigation",
        "Header-Navigation-List",
        "Header-Navigation-First-Level",
        "Header-Navigation-All",
        "Header-Navigation-Icon",
        "ho-scroll-container-teaser-list",
        "scroll-container",
        "start-tests-button",
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
        let is_body = tag == Tag::Body;
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
        let is_google_ai_button_part = has_ancestor_with_class(dom, node_id, "plR5qb");
        if !is_body
            && !is_interesting_id
            && !is_interesting_class
            && !is_main_nav_item
            && !is_google_ai_button_part
        {
            continue;
        }
        if tag == Tag::Style && is_google_ai_button_part {
            let css_text = dom.text_content(node_id);
            let preview: String = css_text.chars().take(1200).collect();
            eprintln!(
                "[surf-host] interesting-style-css node={} parent={:?} preview={:?}",
                node_id,
                dom.get(node_id).parent,
                preview
            );
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
            "[surf-host] interesting-style node={} tag={} id={:?} class={:?} bounds={:?} display={:?} position={} visibility={} overflow=({:?},{:?}) font_family={:?} bg={:#010x} bg_image={:?} bg_clip={:?} mask={:?} flex=({},{},{:?}/{:?}) flexdir={:?} justify={:?} align={:?} align_content={:?} width={:?} width_pct={:?} width_calc={:?} height={:?} height_pct={:?} height_calc={:?} inset=({:?}/{:?},{:?}/{:?},{:?}/{:?},{:?}/{:?}) transform=(tx:{} tx_pct:{} ty:{} ty_pct:{} sx:{} sy:{} rot:{}) min=({:?},{:?}) max=({:?},{:?}) margin=({:?},{:?},{:?},{:?}) margin_auto=({},{},{},{}) padding=({},{},{},{}) grid_rows={} grid_cols={} border_w=({},{},{},{}) border_c=({:#010x},{:#010x},{:#010x},{:#010x}) radius=({},{},{},{}) z={} opacity={:.3} shadows={}{}",
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
            style.font_family,
            style.background_color,
            background_image_name(&style.background_image),
            background_clip_name(style.background_clip),
            background_image_name(&style.mask_image),
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
            style.top,
            style.top_calc,
            style.right_offset,
            style.right_calc,
            style.bottom_offset,
            style.bottom_calc,
            style.left_offset,
            style.left_calc,
            style.transform_tx,
            style.transform_tx_pct,
            style.transform_ty,
            style.transform_ty_pct,
            style.transform_sx,
            style.transform_sy,
            style.transform_rotate,
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
            libwebview::style::BackgroundImageVal::RadialGradient { .. } => "radial-gradient",
            libwebview::style::BackgroundImageVal::ConicGradient { .. } => "conic-gradient",
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
fn load_resources(
    wv: &mut libwebview::WebView,
    base_url: &str,
    image_backend: ImageBackend,
    load_web_fonts: bool,
) {
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
    let mut fetched_font_urls: Vec<String> = Vec::new();
    let mut fetched_font_keys: Vec<(String, u32, bool)> = Vec::new();
    let mut sync_font_attempts = 0usize;
    let mut sync_font_limit_logged = false;

    for (css_url, _href) in &css_links {
        eprintln!("[surf-host] fetching CSS: {}", css_url);
        if let Some(css_body) = fetch_resource(css_url) {
            if let Ok(css_text) = String::from_utf8(css_body) {
                let css_text = resolve_css_resource_urls(&css_text, css_url);
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
                    .map(|u| resolve_url(css_url, u))
                    .collect();
                for import_url in &imports {
                    eprintln!("[surf-host] fetching @import CSS: {}", import_url);
                    if let Some(import_body) = fetch_resource(import_url) {
                        if let Ok(import_text) = String::from_utf8(import_body) {
                            let import_text = resolve_css_resource_urls(&import_text, import_url);
                            wv.add_stylesheet(&import_text);
                        }
                    }
                }

                if load_web_fonts {
                    // @font-face rules from this stylesheet
                    let font_faces: Vec<_> = wv
                        .last_stylesheet_font_faces()
                        .iter()
                        .map(|ff| (ff.family.clone(), ff.src_url.clone(), ff.weight, ff.italic))
                        .collect();
                    for (family, src, weight, italic) in &font_faces {
                        if src.is_empty() {
                            continue;
                        }
                        if wv.has_web_font_style(family, *weight, *italic)
                            || fetched_font_keys
                                .iter()
                                .any(|(f, w, i)| f == family && *w == *weight && *i == *italic)
                        {
                            continue;
                        }
                        if src.starts_with("data:") {
                            if let Some(font_data) = decode_font_data_uri(src) {
                                if let Some(font_id) = load_valid_web_font_data(&family, &font_data)
                                {
                                    eprintln!(
                                        "[surf-host] loaded data font: {} ({} bytes)",
                                        family,
                                        font_data.len()
                                    );
                                    wv.register_web_font_with_style(
                                        &family, *weight, *italic, font_id,
                                    );
                                    fetched_font_keys.push((family.clone(), *weight, *italic));
                                }
                            }
                            continue;
                        }
                        if sync_font_attempts >= HOST_SYNC_WEB_FONT_LIMIT {
                            if !sync_font_limit_logged {
                                eprintln!(
                                    "[surf-host] skipping remaining web fonts after {} synchronous fetches",
                                    HOST_SYNC_WEB_FONT_LIMIT
                                );
                                sync_font_limit_logged = true;
                            }
                            continue;
                        }
                        let font_url = resolve_url(css_url, src);
                        if fetched_font_urls.iter().any(|url| url == &font_url) {
                            continue;
                        }
                        fetched_font_urls.push(font_url.clone());
                        fetched_font_keys.push((family.clone(), *weight, *italic));
                        sync_font_attempts += 1;
                        eprintln!("[surf-host] fetching font: {}", font_url);
                        if let Some(font_data) = fetch_resource(&font_url) {
                            if let Some(font_id) = load_valid_web_font_data(&family, &font_data) {
                                wv.register_web_font_with_style(&family, *weight, *italic, font_id);
                            }
                        }
                    }
                }
            }
        }
    }

    if std::env::var_os("SURF_DEBUG_WEBFONTS").is_some() {
        eprintln!(
            "[surf-host] debug webfonts: Ahem={:?} ahem={:?} Gotham={:?} GothamCond={:?} GothamXNarrow={:?} sans={:?}",
            wv.web_font_id("Ahem"),
            wv.web_font_id("ahem"),
            wv.web_font_id("Gotham"),
            wv.web_font_id("Gotham Cond"),
            wv.web_font_id("Gotham XNarrow"),
            wv.web_font_id("sans-serif")
        );
    }

    // 2. @font-face from inline <style> blocks
    if load_web_fonts {
        let font_faces: Vec<_> = wv
            .all_font_faces()
            .iter()
            .map(|ff| (ff.family.clone(), ff.src_url.clone(), ff.weight, ff.italic))
            .collect();
        for (family, src, weight, italic) in &font_faces {
            if src.is_empty() {
                continue;
            }
            if wv.has_web_font_style(&family, *weight, *italic) {
                continue;
            } // already loaded
            if fetched_font_keys
                .iter()
                .any(|(f, w, i)| f == family && *w == *weight && *i == *italic)
            {
                continue;
            }
            if src.starts_with("data:") {
                if let Some(font_data) = decode_font_data_uri(src) {
                    if let Some(font_id) = load_valid_web_font_data(&family, &font_data) {
                        eprintln!(
                            "[surf-host] loaded inline data font: {} ({} bytes)",
                            family,
                            font_data.len()
                        );
                        wv.register_web_font_with_style(&family, *weight, *italic, font_id);
                        fetched_font_keys.push((family.clone(), *weight, *italic));
                    }
                }
                continue;
            }
            if sync_font_attempts >= HOST_SYNC_WEB_FONT_LIMIT {
                if !sync_font_limit_logged {
                    eprintln!(
                        "[surf-host] skipping remaining web fonts after {} synchronous fetches",
                        HOST_SYNC_WEB_FONT_LIMIT
                    );
                    sync_font_limit_logged = true;
                }
                continue;
            }
            let font_url = resolve_url(base_url, src);
            if fetched_font_urls.iter().any(|url| url == &font_url) {
                continue;
            }
            fetched_font_urls.push(font_url.clone());
            fetched_font_keys.push((family.clone(), *weight, *italic));
            sync_font_attempts += 1;
            eprintln!("[surf-host] fetching inline font: {}", font_url);
            if let Some(font_data) = fetch_resource(&font_url) {
                if let Some(font_id) = load_valid_web_font_data(&family, &font_data) {
                    wv.register_web_font_with_style(&family, *weight, *italic, font_id);
                }
            }
        }
    }

    // 3. Images: loaded asynchronously via start_image_loading()

    // 4. Inline SVGs: <svg>...</svg> — rasterise via resvg and cache under __svg_N__
    rasterize_inline_svgs(wv, base_url, image_backend);

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

fn rasterize_inline_svgs(
    wv: &mut libwebview::WebView,
    base_url: &str,
    image_backend: ImageBackend,
) -> bool {
    let svg_nodes: Vec<(usize, String, Vec<(String, String)>)> = {
        let dom = match wv.dom() {
            Some(d) => d,
            None => {
                return false;
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

    let mut rasterized_any = false;
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

        if let Some((pixels, w, h)) = decode_svg(svg_markup.as_bytes(), image_backend) {
            // Key format: __svg_<node_id>__ — must match svg_inline_key() in layout/mod.rs
            let key = format!("__svg_{}__", node_id);
            eprintln!(
                "[surf-host] rasterized inline SVG node={} {}x{}",
                node_id, w, h
            );
            wv.add_image(&key, pixels, w, h);
            rasterized_any = true;
        }
    }
    rasterized_any
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
fn start_image_loading(
    wv: &libwebview::WebView,
    base_url: &str,
    image_backend: ImageBackend,
) -> PendingImages {
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
                let has_attr_w = dom
                    .attr(i, "width")
                    .and_then(parse_positive_px_attr)
                    .is_some();
                let has_attr_h = dom
                    .attr(i, "height")
                    .and_then(parse_positive_px_attr)
                    .is_some();
                let (css_w, css_h) = explicit_image_decode_hints(wv, i, bounds);
                let attr_bounds = bounds.and_then(|(_, _, w, h)| {
                    if w > 0 && h > 0 && (has_attr_w || has_attr_h) {
                        Some((w as u32, h as u32))
                    } else {
                        None
                    }
                });
                let target_w = css_w.or_else(|| attr_bounds.map(|(w, _)| w));
                let target_h = css_h.or_else(|| attr_bounds.map(|(_, h)| h));
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
                if let Some((pixels, w, h)) = decode_image_scaled(&img_data, tw, th, image_backend)
                {
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

fn parse_positive_px_attr(value: &str) -> Option<u32> {
    let trimmed = value.trim().trim_end_matches("px").trim();
    let parsed = trimmed.parse::<u32>().ok()?;
    (parsed > 0).then_some(parsed)
}

fn explicit_image_decode_hints(
    wv: &libwebview::WebView,
    node_id: usize,
    bounds: Option<(i32, i32, i32, i32)>,
) -> (Option<u32>, Option<u32>) {
    let Some((_, _, box_w, box_h)) = bounds else {
        return (None, None);
    };
    let Some(style) = wv.resolved_style_ref(node_id) else {
        return (None, None);
    };
    let width = if style.width.is_some() && box_w > 0 {
        Some(box_w as u32)
    } else {
        None
    };
    let height = if style.height.is_some() && box_h > 0 {
        Some(box_h as u32)
    } else {
        None
    };
    (width, height)
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
            ureq::get(&url)
                .set("User-Agent", SURF_HOST_USER_AGENT)
                .call()
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
fn apply_host_js_mutations(
    wv: &mut libwebview::WebView,
    base_url: &str,
    cookies: &mut HostCookieJar,
) -> Option<String> {
    let mutations = wv.js_runtime().take_mutations();
    let mut nav_url = None;
    for mutation in mutations {
        match mutation {
            libwebview::js::DomMutation::SetCookie { value } => {
                cookies.store_from_document_cookie(&value, base_url);
                if let Some(cookie_hdr) = cookies.cookie_header_for_url(base_url) {
                    wv.js_runtime().set_cookies(&cookie_hdr);
                }
            }
            libwebview::js::DomMutation::FormSubmit { form_node_id } => {
                nav_url = submit_form_node_host_no_event(wv, base_url, form_node_id);
            }
            _ => {}
        }
    }
    nav_url
}

fn prefetch_module_sources(
    wv: &mut libwebview::WebView,
    page_url: &str,
    referrer_url: &str,
    source: &str,
    seen: &mut HashSet<String>,
) {
    let current_page_id = wv
        .dom()
        .and_then(libwebview::js::extract_vike_page_id_from_dom);
    for specifier in libwebview::js::extract_module_specifiers_for_page_with_page_id(
        source,
        page_url,
        current_page_id.as_deref(),
    ) {
        let full_url = resolve_url(referrer_url, &specifier);
        if !seen.insert(full_url.clone()) {
            continue;
        }
        eprintln!("[js] fetching module: {} -> {}", specifier, full_url);
        let Some(data) = fetch_resource(&full_url) else {
            eprintln!("[js]   module fetch failed");
            continue;
        };
        let Ok(text) = String::from_utf8(data) else {
            eprintln!("[js]   module not valid UTF-8, skipping");
            continue;
        };
        register_module_source_aliases(wv, &specifier, &full_url, &text);
        prefetch_module_sources(wv, page_url, &full_url, &text, seen);
    }
}

fn prefetch_modulepreload_sources(
    wv: &mut libwebview::WebView,
    page_url: &str,
    seen: &mut HashSet<String>,
) {
    let links = wv
        .dom()
        .map(libwebview::js::extract_modulepreload_links_from_dom)
        .unwrap_or_default();
    for href in links {
        let full_url = resolve_url(page_url, &href);
        if !seen.insert(full_url.clone()) {
            continue;
        }
        eprintln!("[js] fetching modulepreload: {} -> {}", href, full_url);
        let Some(data) = fetch_resource(&full_url) else {
            eprintln!("[js]   modulepreload fetch failed");
            continue;
        };
        let Ok(text) = String::from_utf8(data) else {
            eprintln!("[js]   modulepreload not valid UTF-8, skipping");
            continue;
        };
        register_module_source_aliases(wv, &href, &full_url, &text);
        prefetch_module_sources(wv, page_url, &full_url, &text, seen);
    }
}

fn register_module_source_aliases(
    wv: &mut libwebview::WebView,
    specifier: &str,
    full_url: &str,
    source: &str,
) {
    if source.is_empty() {
        return;
    }
    for alias in module_source_aliases(specifier, full_url) {
        wv.js_runtime().register_module_source(&alias, source);
    }
}

fn module_source_aliases(specifier: &str, full_url: &str) -> Vec<String> {
    let mut aliases = Vec::new();
    push_unique_alias(&mut aliases, specifier);
    push_unique_alias(&mut aliases, full_url);

    if let Some(path_start) = full_url.find("://").and_then(|scheme| {
        full_url[scheme + 3..]
            .find('/')
            .map(|path| scheme + 3 + path)
    }) {
        let path = &full_url[path_start..];
        push_unique_alias(&mut aliases, path);
        push_unique_alias(&mut aliases, path.trim_start_matches('/'));
        if let Some(file_start) = path.rfind('/') {
            let file = &path[file_start + 1..];
            push_unique_alias(&mut aliases, &format!("./{}", file));
            if path.contains("/chunks/") {
                push_unique_alias(&mut aliases, &format!("../chunks/{}", file));
                push_unique_alias(&mut aliases, &format!("./chunks/{}", file));
            }
            if path.contains("/entries/") {
                push_unique_alias(&mut aliases, &format!("../entries/{}", file));
                push_unique_alias(&mut aliases, &format!("./entries/{}", file));
            }
        }
    }

    aliases
}

fn push_unique_alias(aliases: &mut Vec<String>, alias: &str) {
    if alias.is_empty() || aliases.iter().any(|existing| existing == alias) {
        return;
    }
    aliases.push(alias.to_string());
}

fn run_javascript(
    wv: &mut libwebview::WebView,
    base_url: &str,
    cookies: &mut HostCookieJar,
) -> bool {
    // Register synchronous HTTP handler so fetch()/XHR work inside JS.
    register_http_handler(wv);

    // Collect script entries (inline + external) in document order.
    let entries = wv.script_entries();
    if entries.is_empty() {
        eprintln!("[js] no scripts found");
        return false;
    }

    let mut scripts: Vec<String> = Vec::new();
    let mut script_urls: Vec<Option<String>> = Vec::new();
    let mut external_count = 0u32;
    let mut inline_count = 0u32;

    for entry in &entries {
        match entry {
            libwebview::js::ScriptEntry::Inline { text, mode: _ } => {
                scripts.push(text.clone());
                script_urls.push(None);
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
                        script_urls.push(Some(full_url));
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
    prepend_debug_script(&mut scripts, &mut script_urls);
    patch_debug_event_target_errors(&mut scripts);
    patch_debug_gbar_dump_exception(&mut scripts);
    patch_debug_google_search_gate(&mut scripts);
    dump_debug_script_indexes(&scripts, &script_urls);

    let mut seen_modules = HashSet::new();
    prefetch_modulepreload_sources(wv, base_url, &mut seen_modules);
    for (idx, script) in scripts.iter().enumerate() {
        let referrer = script_urls
            .get(idx)
            .and_then(|u| u.as_deref())
            .unwrap_or(base_url);
        if let Some(Some(url)) = script_urls.get(idx) {
            register_module_source_aliases(wv, url, url, script);
        }
        prefetch_module_sources(wv, base_url, referrer, script, &mut seen_modules);
    }

    // Execute all scripts.
    let changed = wv.execute_js_with_limits(
        &scripts,
        libwebview::js::ScriptExecutionLimits {
            max_scripts: 256,
            max_script_bytes: None,
        },
    );
    apply_host_js_mutations(wv, base_url, cookies);

    // Print console output.
    for line in wv.js_console() {
        eprintln!("[js:console] {}", line);
    }
    changed
}

fn prepend_debug_script(scripts: &mut Vec<String>, script_urls: &mut Vec<Option<String>>) {
    let Some(source) = std::env::var_os("SURF_HOST_PRE_SCRIPT") else {
        return;
    };
    let source = source.to_string_lossy().into_owned();
    if source.trim().is_empty() {
        return;
    }
    scripts.insert(0, source);
    script_urls.insert(0, None);
}

fn patch_debug_event_target_errors(scripts: &mut [String]) {
    if std::env::var_os("SURF_HOST_DEBUG_EVENT_TARGET_ERRORS").is_none() {
        return;
    }
    let needle = "else throw Error(\"la\");";
    let replacement = "else { var __surfKeys=\"\"; try{__surfKeys=Object.keys(a||{}).slice(0,20).join(\",\")}catch(__surfE){} var __surfCtor=\"\"; try{__surfCtor=a&&a.constructor&&a.constructor.name||typeof a}catch(__surfE){} throw Error(\"la:\"+__surfCtor+\":\"+__surfKeys); }";
    for script in scripts {
        if script.contains(needle) {
            *script = script.replace(needle, replacement);
        }
    }
}

fn patch_debug_gbar_dump_exception(scripts: &mut [String]) {
    if std::env::var_os("SURF_HOST_DEBUG_GBAR_DUMP_EXCEPTION").is_none() {
        return;
    }
    let needle = "this.gbar_=this.gbar_||{};";
    let replacement = "this.gbar_=this.gbar_||{};this.gbar_._DumpException=function(e){throw e;};";
    for script in scripts {
        if script.contains(needle) {
            *script = script.replacen(needle, replacement, 1);
        }
    }
}

fn patch_debug_google_search_gate(scripts: &mut [String]) {
    if std::env::var_os("SURF_HOST_DEBUG_GOOGLE_SEARCH_GATE").is_none() {
        return;
    }
    let _ = scripts;
}

fn dump_debug_script_indexes(scripts: &[String], script_urls: &[Option<String>]) {
    let Some(spec) = std::env::var_os("SURF_HOST_DUMP_SCRIPT_INDEX") else {
        return;
    };
    for raw in spec.to_string_lossy().split(',') {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let Ok(idx) = raw.parse::<usize>() else {
            eprintln!("[js-debug] invalid script dump index '{}'", raw);
            continue;
        };
        let Some(script) = scripts.get(idx) else {
            eprintln!(
                "[js-debug] script #{} unavailable ({} script(s) total)",
                idx,
                scripts.len()
            );
            continue;
        };
        let path = format!("/tmp/surf-script-{}.js", idx);
        match std::fs::write(&path, script) {
            Ok(()) => {
                let src = script_urls
                    .get(idx)
                    .and_then(|url| url.as_deref())
                    .unwrap_or("<inline>");
                eprintln!(
                    "[js-debug] dumped script #{} ({} bytes, src={}) to {}",
                    idx,
                    script.len(),
                    src,
                    path
                );
            }
            Err(err) => {
                eprintln!(
                    "[js-debug] failed to dump script #{} to {}: {}",
                    idx, path, err
                );
            }
        }
    }
}

fn run_js_debug_probes(wv: &mut libwebview::WebView) {
    let Some(probes) = std::env::var_os("SURF_HOST_DEBUG_JS_PROBES") else {
        return;
    };
    let probes = probes.to_string_lossy();
    let defaults = [
        "typeof __d + '/' + typeof requireLazy",
        "document.querySelectorAll('script[data-sjs]').length",
        "document.querySelectorAll('script[data-sjs]:not([data-processed])').length",
        "document.body ? document.body.children.length : -1",
        "document.body ? document.body.textContent.length : -1",
    ];
    if probes.trim().is_empty() || probes.trim() == "1" {
        for source in defaults {
            let _ = wv.eval_js_for_devtools(source);
        }
    } else {
        for source in probes.split(";;") {
            let source = source.trim();
            if !source.is_empty() {
                let _ = wv.eval_js_for_devtools(source);
            }
        }
    }
    for line in wv.js_console() {
        eprintln!("[js:probe] {}", line);
    }
}

/// Run JS timers for `total_ms` milliseconds in 50ms steps.
/// This lets setTimeout(fn, 0) and short-delay timers fire.
fn run_js_timers(
    wv: &mut libwebview::WebView,
    base_url: &str,
    cookies: &mut HostCookieJar,
    total_ms: u64,
) {
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
        apply_host_js_mutations(wv, base_url, cookies);
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
    image_backend: ImageBackend,
) -> Option<(Vec<u32>, u32, u32)> {
    if is_svg(data) {
        return decode_svg(data, image_backend);
    }
    if image_backend == ImageBackend::Anyos {
        return decode_image_scaled_libimage(data, target_w, target_h);
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
    let (mut tw, mut th) = match (target_w, target_h) {
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
    let max_target = tw.max(th);
    if max_target > MAX_DECODE_DIM {
        tw = ((tw as u64 * MAX_DECODE_DIM as u64) / max_target as u64).max(1) as u32;
        th = ((th as u64 * MAX_DECODE_DIM as u64) / max_target as u64).max(1) as u32;
    }

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

fn decode_svg(data: &[u8], image_backend: ImageBackend) -> Option<(Vec<u32>, u32, u32)> {
    if image_backend == ImageBackend::Anyos {
        return decode_svg_libsvg(data);
    }
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

fn decode_svg_libsvg(data: &[u8]) -> Option<(Vec<u32>, u32, u32)> {
    let (w, h) = svg_intrinsic_raster_size(data)
        .or_else(|| {
            let mut out_w = 0.0f32;
            let mut out_h = 0.0f32;
            let rc = libsvg::svg_probe(data.as_ptr(), data.len() as u32, &mut out_w, &mut out_h);
            if rc == 0 {
                Some((out_w.max(1.0) as u32, out_h.max(1.0) as u32))
            } else {
                None
            }
        })
        .unwrap_or((256, 256));
    if w == 0 || h == 0 {
        return None;
    }
    let (rw, rh) = cap_svg_raster_size(w, h);
    let mut pixels = vec![0u32; (rw as usize).checked_mul(rh as usize)?];
    let bg = parse_svg_root_background(data).unwrap_or(0x00000000);
    let rc = libsvg::svg_render_to_size(
        data.as_ptr(),
        data.len() as u32,
        pixels.as_mut_ptr(),
        rw,
        rh,
        bg,
    );
    if rc == 0 {
        Some((pixels, rw, rh))
    } else {
        None
    }
}

fn cap_svg_raster_size(w: u32, h: u32) -> (u32, u32) {
    const MAX_SVG_DIM: u32 = 1024;
    if w == 0 || h == 0 {
        return (1, 1);
    }
    if w <= MAX_SVG_DIM && h <= MAX_SVG_DIM {
        return (w, h);
    }
    let max_dim = w.max(h) as u64;
    (
        ((w as u64 * MAX_SVG_DIM as u64) / max_dim).max(1) as u32,
        ((h as u64 * MAX_SVG_DIM as u64) / max_dim).max(1) as u32,
    )
}

fn apply_svg_inherited_color(svg: String, color: Option<u32>) -> String {
    let Some(color) = color else {
        return substitute_css_vars(svg, "#000000");
    };
    let rgb = color & 0x00FF_FFFF;
    let hex = format!("#{:06x}", rgb);
    let mut out = svg
        .replace("currentColor", &hex)
        .replace("currentcolor", &hex);
    out = substitute_css_vars(out, &hex);
    out = inject_svg_root_color(&out, &hex);
    inject_default_svg_fill(&out, &hex)
}

/// Replace `var(--name [, fallback])` occurrences with `replacement`.
/// libsvg/resvg both stumble on CSS custom properties inside fill/stroke.
fn substitute_css_vars(svg: String, replacement: &str) -> String {
    if !svg.contains("var(") && !svg.contains("VAR(") {
        return svg;
    }
    let mut out = String::with_capacity(svg.len());
    let bytes = svg.as_bytes();
    let mut i = 0usize;
    let mut copy_from = 0usize;
    while i + 4 <= bytes.len() {
        if (bytes[i] == b'v' || bytes[i] == b'V')
            && (bytes[i + 1] == b'a' || bytes[i + 1] == b'A')
            && (bytes[i + 2] == b'r' || bytes[i + 2] == b'R')
            && bytes[i + 3] == b'('
        {
            let mut depth = 1i32;
            let mut j = i + 4;
            while j < bytes.len() {
                match bytes[j] {
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            if j < bytes.len() {
                out.push_str(&svg[copy_from..i]);
                out.push_str(replacement);
                i = j + 1;
                copy_from = i;
                continue;
            }
        }
        i += 1;
    }
    out.push_str(&svg[copy_from..]);
    out
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
    let view_box =
        extract_attr_value(open_tag, "viewBox").or_else(|| extract_attr_value(open_tag, "viewbox"));

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
