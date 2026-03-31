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
//! In window mode: F5=screenshot, Esc=quit, mouse wheel=scroll.

// Force-link libfont so its #[no_mangle] symbols are available to libfont_client.
extern crate libfont;

use std::io::Read;
use minifb::{Key, Window, WindowOptions};

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

// ── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    let args = parse_args();
    let width = args.width;
    let height = args.height;

    // Initialize font engine
    libfont_client::init();

    // Create WebView
    let mut wv = libwebview::WebView::new(width, height);

    // Fetch page
    eprintln!("[surf-host] fetching: {}", args.url);
    let (html, base_url) = fetch_page(&args.url);
    eprintln!("[surf-host] got {} bytes HTML", html.len());

    // Set URL and render HTML
    wv.set_url(&base_url);
    wv.set_html(&html);

    // Discover and load external resources from the DOM (same as anyOS surf)
    load_resources(&mut wv, &base_url);

    // Build initial framebuffer
    let mut framebuffer = vec![0xFFFFFFFFu32; (width * height) as usize];
    extract_pixels(&wv, &mut framebuffer, width as usize, height as usize, 0);

    // ── Screenshot-only mode ─────────────────────────────────────────────
    if let Some(ref path) = args.screenshot {
        if args.delay_ms > 0 {
            eprintln!("[surf-host] waiting {}ms before screenshot...", args.delay_ms);
            std::thread::sleep(std::time::Duration::from_millis(args.delay_ms));
        }
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
    let mut window = Window::new(
        &format!("surf-host — {}", args.url),
        width as usize,
        height as usize,
        WindowOptions {
            resize: true,
            ..WindowOptions::default()
        },
    )
    .expect("Failed to create window");

    window.set_target_fps(30);
    let mut scroll_y: i32 = 0;
    let mut needs_redraw = true;
    let mut screenshot_count: u32 = 0;
    let mut f5_was_pressed = false;
    let mut f6_was_pressed = false;

    eprintln!("[surf-host] window open. F5=screenshot, F6=full-page, Esc=quit.");

    while window.is_open() && !window.is_key_down(Key::Escape) {
        // Handle scroll
        if let Some(scroll) = window.get_scroll_wheel() {
            let delta = -(scroll.1 as i32) * 40;
            scroll_y = (scroll_y + delta).max(0);
            needs_redraw = true;
        }

        // F5 = screenshot (current viewport)
        let f5_down = window.is_key_down(Key::F5);
        if f5_down && !f5_was_pressed {
            screenshot_count += 1;
            let path = format!("screenshot_{}.png", screenshot_count);
            save_screenshot(&framebuffer, width, height, &path);
            eprintln!("[surf-host] screenshot saved: {}", path);
        }
        f5_was_pressed = f5_down;

        // F6 = full-page screenshot (entire document height)
        let f6_down = window.is_key_down(Key::F6);
        if f6_down && !f6_was_pressed {
            screenshot_count += 1;
            let path = format!("screenshot_{}_full.png", screenshot_count);
            save_fullpage_screenshot(&mut wv, width, &path);
            eprintln!("[surf-host] full-page screenshot saved: {}", path);
        }
        f6_was_pressed = f6_down;

        if needs_redraw {
            framebuffer.fill(0xFFFFFFFF);
            extract_pixels(&wv, &mut framebuffer, width as usize, height as usize, scroll_y);
            needs_redraw = false;
        }

        window
            .update_with_buffer(&framebuffer, width as usize, height as usize)
            .unwrap();
    }
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
        std::fs::read(&url[7..]).ok()
    } else {
        match ureq::get(url).call() {
            Ok(resp) => {
                let mut buf = Vec::new();
                resp.into_reader().read_to_end(&mut buf).ok()?;
                Some(buf)
            }
            Err(_) => None,
        }
    }
}

fn resolve_url(base: &str, relative: &str) -> String {
    if relative.starts_with("http://") || relative.starts_with("https://") {
        return relative.to_string();
    }
    if relative.starts_with("//") {
        let scheme = if base.starts_with("https") { "https:" } else { "http:" };
        return format!("{}{}", scheme, relative);
    }
    if relative.starts_with('/') {
        if let Some(idx) = base.find("://") {
            if let Some(slash) = base[idx + 3..].find('/') {
                return format!("{}{}", &base[..idx + 3 + slash], relative);
            }
        }
        return format!("{}{}", base.trim_end_matches('/'), relative);
    }
    if let Some(last_slash) = base.rfind('/') {
        format!("{}/{}", &base[..last_slash], relative)
    } else {
        format!("{}/{}", base, relative)
    }
}

// ── Resource loading (DOM-based, identical to anyOS surf) ────────────────────

use libwebview::dom::{Dom, NodeType, Tag};

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

    // 3. Images: <img src="...">
    let img_srcs = {
        let dom = match wv.dom() { Some(d) => d, None => return };
        let mut srcs = Vec::new();
        for (i, node) in dom.nodes.iter().enumerate() {
            if let NodeType::Element { tag: Tag::Img, .. } = &node.node_type {
                if let Some(src) = dom.attr(i, "src") {
                    if src.is_empty() || src.starts_with("data:") { continue; }
                    srcs.push((resolve_url(base_url, src), String::from(src)));
                }
            }
        }
        srcs
    };

    for (img_url, src_attr) in &img_srcs {
        eprintln!("[surf-host] fetching image: {}", img_url);
        if let Some(img_data) = fetch_resource(img_url) {
            if let Some((pixels, w, h)) = decode_image(&img_data) {
                wv.add_image(src_attr, pixels, w, h);
                eprintln!("[surf-host]   decoded {}x{}", w, h);
            } else {
                eprintln!("[surf-host]   decode failed ({}B)", img_data.len());
            }
        }
    }

    // Re-layout with all resources loaded
    wv.relayout();
}

// ── Image decoding ───────────────────────────────────────────────────────────

fn decode_image(data: &[u8]) -> Option<(Vec<u32>, u32, u32)> {
    if is_svg(data) {
        return decode_svg(data);
    }
    let img = image::load_from_memory(data).ok()?;
    let rgba = img.to_rgba8();
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
