// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Async resource loading for the Surf browser.
//!
//! Covers:
//! - HTTP response body decoding (charset detection, Latin-1 → UTF-8)
//! - External CSS stylesheet discovery and submission to the network worker
//! - External image discovery and submission to the network worker
//! - SVG rasterisation and raster image decoding (called from result handlers)

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

fn parse_dimension_attr(value: &str) -> Option<i32> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let digits = trimmed
        .trim_end_matches(|c: char| c.is_ascii_whitespace())
        .trim_end_matches("px")
        .trim();
    digits.parse::<i32>().ok().filter(|v| *v > 0)
}

fn deferred_image_rank(
    dom_index: usize,
    width: Option<i32>,
    height: Option<i32>,
    lazy_requested: bool,
    high_priority: bool,
) -> i32 {
    let mut score = 0i32;
    if high_priority {
        score += 5000;
    }
    if lazy_requested {
        score -= 5000;
    }
    if let (Some(w), Some(h)) = (width, height) {
        let area = w.saturating_mul(h);
        score += (area / 1500).clamp(0, 1800);
        if w >= 600 || h >= 300 {
            score += 400;
        }
    } else if width.is_some() || height.is_some() {
        score += 120;
    }
    score += 512i32.saturating_sub(dom_index as i32);
    score
}

fn deferred_image_runtime_rank(
    webview: &libwebview::WebView,
    req: &crate::tab::DeferredImageRequest,
    scroll_y: i32,
    viewport_h: i32,
) -> i32 {
    let mut score = deferred_image_rank(
        req.dom_index,
        req.width,
        req.height,
        req.lazy_requested,
        req.high_priority,
    );
    if let Some((_, y, _, h)) = webview.node_bounds(req.node_id) {
        let top = scroll_y;
        let bottom = scroll_y + viewport_h.max(1);
        let visible = y < bottom && y + h > top;
        if visible {
            score += 10_000;
        } else if y >= bottom {
            let distance = y - bottom;
            score += (4_000 - distance.saturating_mul(4)).clamp(-2_500, 4_000);
        } else {
            let distance = top - (y + h);
            score += (2_500 - distance.saturating_mul(3)).clamp(-2_000, 2_500);
        }
        if y <= bottom + viewport_h {
            score += 1_500;
        }
    }
    score
}

fn should_defer_font_face(display: libwebview::css::FontDisplay) -> bool {
    matches!(
        display,
        libwebview::css::FontDisplay::Swap
            | libwebview::css::FontDisplay::Fallback
            | libwebview::css::FontDisplay::Optional
    )
}

// ═══════════════════════════════════════════════════════════
// HTTP body decoding
// ═══════════════════════════════════════════════════════════

/// Decode an HTTP response body to a UTF-8 `String`.
///
/// Prefers valid UTF-8 regardless of the declared charset (many servers
/// incorrectly claim `ISO-8859-1` while sending UTF-8).  Falls back to
/// Latin-1 → UTF-8 transcoding when the body is not valid UTF-8.
pub(crate) fn decode_http_body(body: &[u8], headers: &str) -> String {
    // Happy path: valid UTF-8 — use it directly.
    if let Ok(s) = core::str::from_utf8(body) {
        return String::from(s);
    }

    // Body is not valid UTF-8; inspect charset declarations.
    let charset =
        detect_charset_from_headers(headers).or_else(|| detect_charset_from_html_bytes(body));

    match charset.as_deref() {
        Some("iso-8859-1") | Some("latin1") | Some("latin-1") | Some("windows-1252") | None => {
            latin1_to_utf8(body)
        }
        _ => String::from_utf8_lossy(body).into_owned(),
    }
}

/// Extract the charset from the `Content-Type` response header, if present.
fn detect_charset_from_headers(headers: &str) -> Option<String> {
    let ct = crate::http::find_header_value(headers, "content-type")?;
    extract_charset(ct)
}

/// Scan the first 2 KiB of the body for a `charset=` declaration in HTML
/// `<meta>` tags.
fn detect_charset_from_html_bytes(body: &[u8]) -> Option<String> {
    let scan_len = body.len().min(2048);
    // Only inspect the ASCII-safe prefix (non-UTF-8 bytes outside this range
    // cannot appear in the `charset=` attribute name anyway).
    let text = core::str::from_utf8(&body[..scan_len]).unwrap_or("");
    let lower = text.to_ascii_lowercase();

    if let Some(pos) = lower.find("charset=") {
        let rest = &lower[pos + 8..];
        let rest = rest.trim_start_matches(['"', '\'', ' '].as_ref());
        let end = rest
            .find(|c: char| c == '"' || c == '\'' || c == ';' || c == ' ' || c == '>')
            .unwrap_or(rest.len());
        let charset = rest[..end].trim();
        if !charset.is_empty() {
            return Some(String::from(charset));
        }
    }
    None
}

/// Parse the `charset` parameter out of a `Content-Type` header value.
fn extract_charset(content_type: &str) -> Option<String> {
    let lower = content_type.to_ascii_lowercase();
    if let Some(pos) = lower.find("charset=") {
        let rest = &lower[pos + 8..];
        let rest = rest.trim_start_matches(['"', '\''].as_ref());
        let end = rest
            .find(|c: char| c == '"' || c == '\'' || c == ';' || c == ' ')
            .unwrap_or(rest.len());
        let charset = rest[..end].trim();
        if !charset.is_empty() {
            return Some(String::from(charset));
        }
    }
    None
}

/// Transcode ISO-8859-1 / Latin-1 bytes to a UTF-8 `String`.
///
/// Every Latin-1 code point maps 1:1 to the same Unicode scalar value, so a
/// simple cast `b as char` is correct.
fn latin1_to_utf8(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(b as char);
    }
    out
}

// ═══════════════════════════════════════════════════════════
// CSS stylesheet discovery — submits to network worker
// ═══════════════════════════════════════════════════════════

/// Scan the DOM for `<link rel="stylesheet">` tags and submit them to the
/// background network worker for async fetching.
///
/// The page is rendered immediately with only the built-in user-agent CSS;
/// each external stylesheet is applied and the layout refreshed as it arrives,
/// giving a progressive-rendering effect without blocking the UI thread.
pub(crate) fn queue_stylesheets(
    dom: &libwebview::dom::Dom,
    base_url: &crate::http::Url,
    tab_index: usize,
) -> usize {
    let generation = crate::net_worker::current_generation();
    let mut count = 0u32;

    for (i, node) in dom.nodes.iter().enumerate() {
        if let libwebview::dom::NodeType::Element {
            tag: libwebview::dom::Tag::Link,
            ..
        } = &node.node_type
        {
            let rel = dom.attr(i, "rel").unwrap_or("");
            if !rel
                .split_ascii_whitespace()
                .any(|tok| tok.eq_ignore_ascii_case("stylesheet"))
            {
                continue;
            }
            if let Some(href) = dom.attr(i, "href") {
                if !href.is_empty() {
                    let css_url = crate::http::resolve_url(base_url, href);
                    crate::net_worker::submit(crate::net_worker::FetchRequest::Css {
                        tab_index,
                        href: crate::ui::format_url(&css_url),
                        url: css_url,
                        generation,
                    });
                    count += 1;
                }
            }
        }
    }

    if count > 0 {
        crate::surf_log!("[surf] submitted {} stylesheet(s) to worker", count);
        crate::ensure_net_poll_timer();
    }

    count as usize
}

// ═══════════════════════════════════════════════════════════
// @font-face discovery — submits web font fetches to network worker
// ═══════════════════════════════════════════════════════════

/// Queue font downloads for all @font-face rules found in inline <style> blocks.
pub(crate) fn queue_font_faces(
    webview: &libwebview::WebView,
    base_url: &crate::http::Url,
    tab_index: usize,
) {
    let generation = crate::net_worker::current_generation();
    let font_faces: Vec<(String, String, libwebview::css::FontDisplay)> = webview
        .all_font_faces()
        .iter()
        .map(|ff| (ff.family.clone(), ff.src_url.clone(), ff.display))
        .collect();
    queue_font_face_batch(tab_index, generation, base_url, &font_faces);
}

pub(crate) fn queue_font_face_batch(
    tab_index: usize,
    generation: u32,
    base_url: &crate::http::Url,
    font_faces: &[(String, String, libwebview::css::FontDisplay)],
) {
    let st = crate::state();
    if tab_index >= st.tabs.len() {
        return;
    }

    let mut immediate = 0u32;
    let mut deferred = 0u32;
    let mut queued_keys: Vec<(String, String)> = Vec::new();

    for (family, src_url, display) in font_faces {
        if src_url.is_empty() {
            continue;
        }
        if st.tabs[tab_index].webview.web_font_id(family).is_some() {
            continue;
        }
        if queued_keys.iter().any(|(f, s)| f == family && s == src_url) {
            continue;
        }
        queued_keys.push((family.clone(), src_url.clone()));
        if st.tabs[tab_index]
            .deferred_fonts
            .iter()
            .any(|req| req.family == *family)
        {
            continue;
        }

        let resolved = crate::http::resolve_url(base_url, src_url);
        if should_defer_font_face(*display) {
            st.tabs[tab_index]
                .deferred_fonts
                .push(crate::tab::DeferredFontRequest {
                    family: family.clone(),
                    url: resolved,
                    display: *display,
                    generation,
                });
            deferred += 1;
        } else {
            crate::net_worker::submit(crate::net_worker::FetchRequest::Font {
                tab_index,
                family: family.clone(),
                url: resolved,
                display: *display,
                generation,
            });
            immediate += 1;
        }
    }

    if immediate > 0 || deferred > 0 {
        crate::surf_log!(
            "[surf] queued web fonts: immediate={} deferred={}",
            immediate,
            deferred
        );
        crate::ensure_net_poll_timer();
    }
}

// ═══════════════════════════════════════════════════════════
// Base64 decode (for data: URI support)
// ═══════════════════════════════════════════════════════════

/// Decode a base64-encoded byte slice to raw bytes.
///
/// Skips whitespace characters (newlines, spaces, tabs) which are legal inside
/// base64 data URIs. Returns an empty Vec on invalid input.
fn base64_decode(input: &[u8]) -> Vec<u8> {
    static TABLE: [i8; 256] = {
        let mut t = [-1i8; 256];
        let mut i = 0usize;
        while i < 26 {
            t[b'A' as usize + i] = i as i8;
            i += 1;
        }
        let mut i = 0usize;
        while i < 26 {
            t[b'a' as usize + i] = (26 + i) as i8;
            i += 1;
        }
        let mut i = 0usize;
        while i < 10 {
            t[b'0' as usize + i] = (52 + i) as i8;
            i += 1;
        }
        t[b'+' as usize] = 62;
        t[b'/' as usize] = 63;
        t
    };
    let mut out = Vec::new();
    let mut buf = 0u32;
    let mut bits = 0u32;
    for &b in input {
        if b == b'=' {
            break;
        }
        if b == b' ' || b == b'\n' || b == b'\r' || b == b'\t' {
            continue;
        }
        let v = TABLE[b as usize];
        if v < 0 {
            continue;
        }
        buf = (buf << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xFF) as u8);
        }
    }
    out
}

/// Parse a `data:` URI into raw bytes and whether it is SVG.
///
/// Returns `None` for URIs that are not image data URIs or cannot be decoded.
fn decode_data_uri(src: &str) -> Option<(Vec<u8>, bool)> {
    let rest = src.strip_prefix("data:")?;
    let comma = rest.find(',')?;
    let meta = &rest[..comma];
    let payload = &rest[comma + 1..];

    // Determine MIME type (first segment before ';')
    let mime = meta.split(';').next().unwrap_or("").trim();
    // Only handle image/* MIME types
    if !mime.starts_with("image/") && !mime.eq_ignore_ascii_case("image/svg+xml") {
        return None;
    }
    let is_svg =
        mime.eq_ignore_ascii_case("image/svg+xml") || mime.eq_ignore_ascii_case("image/svg");

    let is_base64 = meta.contains(";base64");
    let bytes = if is_base64 {
        base64_decode(payload.as_bytes())
    } else {
        // URL-encoded (rare for images) — treat payload as UTF-8 bytes directly
        payload.as_bytes().to_vec()
    };

    if bytes.is_empty() {
        return None;
    }
    Some((bytes, is_svg))
}

// ═══════════════════════════════════════════════════════════
// Image discovery — submits to network worker
// ═══════════════════════════════════════════════════════════

/// Scan the DOM for `<img src="…">` tags and submit them to the background
/// network worker for async fetching. `data:` URIs are decoded inline without
/// a network round-trip and added directly to the image cache.
pub(crate) fn queue_images(
    dom: &libwebview::dom::Dom,
    base_url: &crate::http::Url,
    tab_index: usize,
    startup_critical_only: bool,
) {
    let generation = crate::net_worker::current_generation();
    let mut count = 0u32;
    let mut immediate_count = 0u32;
    // Keep the truly critical image path small so text/CSS/JS become usable earlier.
    // `loading=lazy` stays out of the startup path entirely, while explicit
    // eager/high-priority images get a small bonus budget similar to browsers'
    // visible-first behaviour.
    let mut immediate_budget = if startup_critical_only { 0usize } else { 8usize };
    let mut high_priority_budget = if startup_critical_only { 2usize } else { 4usize };
    let mut deferred: Vec<(i32, crate::tab::DeferredImageRequest)> = Vec::new();

    for (i, node) in dom.nodes.iter().enumerate() {
        let is_image_like = matches!(
            &node.node_type,
            libwebview::dom::NodeType::Element {
                tag: libwebview::dom::Tag::Img,
                ..
            }
        ) || dom.has_tag_name(i, "a-img");
        if is_image_like {
            if let Some(src) = dom.image_url(i) {
                if src.is_empty() {
                    continue;
                }

                if src.starts_with("data:") {
                    // Decode inline — no network fetch needed.
                    if let Some((bytes, is_svg)) = decode_data_uri(&src) {
                        let key = src.clone();
                        if is_svg {
                            decode_svg_no_relayout(&bytes, &key, tab_index);
                        } else {
                            decode_raster_no_relayout(&bytes, &key, tab_index);
                        }
                    }
                    continue;
                }

                let img_url = crate::http::resolve_url(base_url, &src);
                {
                    let st = crate::state();
                    if tab_index < st.tabs.len()
                        && st.tabs[tab_index].webview.has_decoded_image(&src)
                    {
                        continue;
                    }
                }
                let loading = dom.attr(i, "loading").unwrap_or("");
                let fetchpriority = dom.attr(i, "fetchpriority").unwrap_or("");
                let lazy_requested = loading.eq_ignore_ascii_case("lazy");
                let high_priority = loading.eq_ignore_ascii_case("eager")
                    || fetchpriority.eq_ignore_ascii_case("high");
                let initial_viewport_hit = {
                    let st = crate::state();
                    if tab_index >= st.tabs.len() {
                        false
                    } else {
                        let webview = &st.tabs[tab_index].webview;
                        let viewport_h = webview.viewport_height() as i32;
                        webview
                            .node_bounds(i)
                            .is_some_and(|(_, y, _, h)| y < viewport_h + 640 && y + h > -192)
                    }
                };
                let (width, height) = {
                    let st = crate::state();
                    if tab_index >= st.tabs.len() {
                        (
                            dom.attr(i, "width").and_then(parse_dimension_attr),
                            dom.attr(i, "height").and_then(parse_dimension_attr),
                        )
                    } else {
                        let webview = &st.tabs[tab_index].webview;
                        if let Some((_, _, w, h)) = webview.node_bounds(i) {
                            if w > 0 && h > 0 {
                                (Some(w), Some(h))
                            } else {
                                (
                                    dom.attr(i, "width").and_then(parse_dimension_attr),
                                    dom.attr(i, "height").and_then(parse_dimension_attr),
                                )
                            }
                        } else {
                            (
                                dom.attr(i, "width").and_then(parse_dimension_attr),
                                dom.attr(i, "height").and_then(parse_dimension_attr),
                            )
                        }
                    }
                };
                let should_start_immediately = if lazy_requested && !initial_viewport_hit {
                    false
                } else if startup_critical_only {
                    if high_priority && initial_viewport_hit && high_priority_budget > 0 {
                        high_priority_budget -= 1;
                        true
                    } else {
                        false
                    }
                } else if initial_viewport_hit && immediate_budget > 0 {
                    immediate_budget -= 1;
                    true
                } else if high_priority && high_priority_budget > 0 {
                    high_priority_budget -= 1;
                    true
                } else if immediate_budget > 0 {
                    immediate_budget -= 1;
                    true
                } else {
                    false
                };

                if should_start_immediately {
                    crate::net_worker::submit(crate::net_worker::FetchRequest::Image {
                        tab_index,
                        src,
                        url: img_url,
                        target_width: width.map(|v| v.max(1) as u32),
                        target_height: height.map(|v| v.max(1) as u32),
                        priority: crate::net_worker::ImagePriority::Viewport,
                        generation,
                    });
                    immediate_count += 1;
                } else {
                    let priority = if high_priority || (!lazy_requested && i < 24) {
                        crate::net_worker::ImagePriority::Viewport
                    } else {
                        crate::net_worker::ImagePriority::Deferred
                    };
                    let rank = deferred_image_rank(i, width, height, lazy_requested, high_priority);
                    deferred.push((
                        rank,
                        crate::tab::DeferredImageRequest {
                            node_id: i,
                            dom_index: i,
                            src,
                            url: img_url,
                            priority,
                            width,
                            height,
                            lazy_requested,
                            high_priority,
                            generation,
                        },
                    ));
                }
                count += 1;
            }
        }
    }

    {
        let st = crate::state();
        if tab_index < st.tabs.len() {
            let webview = &st.tabs[tab_index].webview;
            for i in 0..dom.nodes.len() {
                let Some(style) = webview.resolved_style_ref(i) else {
                    continue;
                };
                let bg_src = match &style.background_image {
                    libwebview::style::BackgroundImageVal::Url(src) if !src.is_empty() => src,
                    _ => continue,
                };
                if bg_src.starts_with("data:") {
                    if let Some((bytes, is_svg)) = decode_data_uri(bg_src) {
                        if is_svg {
                            decode_svg_no_relayout(&bytes, bg_src, tab_index);
                        } else {
                            decode_raster_no_relayout(&bytes, bg_src, tab_index);
                        }
                    }
                    continue;
                }
                if tab_index < st.tabs.len()
                    && st.tabs[tab_index].webview.has_decoded_image(bg_src)
                {
                    continue;
                }
                if deferred.iter().any(|(_, req)| req.src == *bg_src) {
                    continue;
                }
                let img_url = crate::http::resolve_url(base_url, bg_src);
                let priority = crate::net_worker::ImagePriority::Viewport;
                let rank = deferred_image_rank(i, None, None, false, true);
                deferred.push((
                    rank,
                    crate::tab::DeferredImageRequest {
                        node_id: i,
                        dom_index: i,
                        src: bg_src.clone(),
                        url: img_url,
                        priority,
                        width: None,
                        height: None,
                        lazy_requested: false,
                        high_priority: true,
                        generation,
                    },
                ));
                count += 1;
            }
        }
    }

    deferred.sort_by(|a, b| b.0.cmp(&a.0));

    let st = crate::state();
    if tab_index < st.tabs.len() {
        st.tabs[tab_index].deferred_images = deferred.into_iter().map(|(_, req)| req).collect();
        st.tabs[tab_index].deferred_images_inflight = 0;
    }

    if count > 0 {
        let deferred_count = count.saturating_sub(immediate_count);
        crate::surf_log!(
            "[surf] image queue summary: total={} immediate={} deferred={} startup_critical_only={} tab={}",
            count,
            immediate_count,
            deferred_count,
            startup_critical_only,
            tab_index
        );
        crate::ensure_net_poll_timer();
    }
}

pub(crate) fn submit_deferred_images(tab_index: usize, max_batch: usize) -> usize {
    if max_batch == 0 {
        return 0;
    }
    let st = crate::state();
    if tab_index >= st.tabs.len() {
        return 0;
    }
    let tab = &mut st.tabs[tab_index];
    let scroll_y = tab.webview.scroll_view().get_state() as i32;
    let viewport_h = tab.webview.viewport_height() as i32;
    let webview = &tab.webview;
    tab.deferred_images.sort_by(|a, b| {
        deferred_image_runtime_rank(webview, b, scroll_y, viewport_h)
            .cmp(&deferred_image_runtime_rank(webview, a, scroll_y, viewport_h))
    });
    let count = core::cmp::min(max_batch, tab.deferred_images.len());
    if count == 0 {
        return 0;
    }
    let requests: Vec<crate::tab::DeferredImageRequest> =
        tab.deferred_images.drain(0..count).collect();
    tab.deferred_images_inflight += requests.len();
    for req in requests {
        let dynamic_priority = if let Some((_, y, _, h)) = tab.webview.node_bounds(req.node_id) {
            let top = scroll_y;
            let bottom = scroll_y + viewport_h.max(1);
            let near_above = top - viewport_h;
            let near_below = bottom + viewport_h * 2;
            if y < near_below && y + h > near_above {
                crate::net_worker::ImagePriority::Viewport
            } else {
                req.priority
            }
        } else {
            req.priority
        };
        crate::net_worker::submit(crate::net_worker::FetchRequest::Image {
            tab_index,
            src: req.src,
            url: req.url,
            target_width: req.width.map(|v| v.max(1) as u32),
            target_height: req.height.map(|v| v.max(1) as u32),
            priority: dynamic_priority,
            generation: req.generation,
        });
    }
    crate::surf_log!(
        "[surf] submitted {} deferred image(s) for tab {} (remaining={} inflight={})",
        count,
        tab_index,
        tab.deferred_images.len(),
        tab.deferred_images_inflight
    );
    crate::ensure_net_poll_timer();
    count
}

pub(crate) fn submit_deferred_fonts(tab_index: usize, max_batch: usize) -> usize {
    if max_batch == 0 {
        return 0;
    }
    let st = crate::state();
    if tab_index >= st.tabs.len() {
        return 0;
    }
    let tab = &mut st.tabs[tab_index];
    let count = core::cmp::min(max_batch, tab.deferred_fonts.len());
    if count == 0 {
        return 0;
    }
    let requests: Vec<crate::tab::DeferredFontRequest> =
        tab.deferred_fonts.drain(0..count).collect();
    tab.deferred_fonts_inflight += requests.len();
    for req in requests {
        crate::net_worker::submit(crate::net_worker::FetchRequest::Font {
            tab_index,
            family: req.family,
            url: req.url,
            display: req.display,
            generation: req.generation,
        });
    }
    crate::surf_log!(
        "[surf] submitted {} deferred font(s) for tab {} (remaining={} inflight={})",
        count,
        tab_index,
        tab.deferred_fonts.len(),
        tab.deferred_fonts_inflight
    );
    crate::ensure_net_poll_timer();
    count
}

// ═══════════════════════════════════════════════════════════
// Inline SVG discovery — rasterises <svg>…</svg> blocks
// ═══════════════════════════════════════════════════════════

/// Generate the synthetic image-cache key used for an inline `<svg>` node.
pub(crate) fn inline_svg_key(node_id: usize) -> String {
    let mut s = String::from("__svg_");
    // Append node_id as decimal without format! macro
    let mut n = node_id;
    if n == 0 {
        s.push('0');
    } else {
        let mut buf = [0u8; 20];
        let mut pos = 20;
        while n > 0 {
            pos -= 1;
            buf[pos] = b'0' + (n % 10) as u8;
            n /= 10;
        }
        for &b in &buf[pos..] {
            s.push(b as char);
        }
    }
    s.push_str("__");
    s
}

/// Scan the DOM for `<svg>` nodes whose raw SVG markup has been captured as a
/// child text node by the HTML parser (see `html.rs` where `svg` is treated as
/// a raw-text element), rasterise each one with `libsvg_client`, and store the
/// result in the image cache under the synthetic key `__svg_<node_id>__`.
///
/// Called once per page load, immediately after `queue_images()`.
pub(crate) fn queue_inline_svgs(dom: &libwebview::dom::Dom, tab_index: usize) -> bool {
    let mut count = 0u32;

    for (node_id, node) in dom.nodes.iter().enumerate() {
        if let libwebview::dom::NodeType::Element {
            tag: libwebview::dom::Tag::Svg,
            attrs,
            ..
        } = &node.node_type
        {
            // The HTML parser stores the raw SVG inner-markup as a single Text child.
            let inner = node.children.iter().find_map(|&child| {
                if let libwebview::dom::NodeType::Text(ref s) = dom.nodes[child].node_type {
                    if !s.is_empty() {
                        Some(s.as_str())
                    } else {
                        None
                    }
                } else {
                    None
                }
            });

            let inner = match inner {
                Some(s) => s,
                None => continue,
            };

            // Reconstruct a self-contained SVG document from attrs + inner markup.
            let mut svg = String::from("<svg");
            // Pass through known SVG attributes
            for attr in attrs {
                let n = attr.name.as_str();
                if matches!(
                    n,
                    "width"
                        | "height"
                        | "viewBox"
                        | "viewbox"
                        | "xmlns"
                        | "version"
                        | "fill"
                        | "stroke"
                        | "class"
                        | "id"
                        | "style"
                        | "preserveAspectRatio"
                ) {
                    svg.push(' ');
                    svg.push_str(n);
                    svg.push_str("=\"");
                    // Basic XML attribute escaping
                    for c in attr.value.chars() {
                        match c {
                            '"' => svg.push_str("&quot;"),
                            '&' => svg.push_str("&amp;"),
                            '<' => svg.push_str("&lt;"),
                            '>' => svg.push_str("&gt;"),
                            c => svg.push(c),
                        }
                    }
                    svg.push('"');
                }
            }
            // Ensure there is an xmlns so libsvg is happy
            if !attrs.iter().any(|a| a.name == "xmlns") {
                svg.push_str(" xmlns=\"http://www.w3.org/2000/svg\"");
            }
            svg.push('>');
            svg.push_str(inner);
            svg.push_str("</svg>");

            let key = inline_svg_key(node_id);
            decode_svg_no_relayout(svg.as_bytes(), &key, tab_index);
            count += 1;
        }
    }

    if count > 0 {
        crate::surf_log!("[surf] rasterised {} inline SVG(s)", count);
        true
    } else {
        false
    }
}

// ═══════════════════════════════════════════════════════════
// Image decode helpers (called from main.rs result handlers)
// ═══════════════════════════════════════════════════════════

/// Returns `true` when the fetched resource is an SVG document, detected
/// either by the URL extension or the `Content-Type` response header.
pub(crate) fn is_svg(src: &str, headers: &str) -> bool {
    // URL extension check (fast path, strips query string first).
    let path = src.split('?').next().unwrap_or(src);
    let path_lower = path.to_ascii_lowercase();
    if path_lower.ends_with(".svg") {
        return true;
    }
    // Content-Type header (authoritative for mismatched extensions).
    if let Some(ct) = crate::http::find_header_value(headers, "content-type") {
        let ct_lower = ct.to_ascii_lowercase();
        if ct_lower.starts_with("image/svg") {
            return true;
        }
    }
    false
}

/// Decode a raster image (PNG, JPEG, BMP, GIF) and add the result to the
/// image cache of `tab_idx`, then relayout.
pub(crate) fn decode_raster(data: &[u8], src: &str, tab_idx: usize) {
    decode_raster_no_relayout(data, src, tab_idx);
    crate::request_image_refresh(tab_idx);
}

/// Decode a raster image without triggering a relayout.
///
/// Used by the batch result processor which does a single relayout at the end.
pub(crate) fn decode_raster_no_relayout(data: &[u8], src: &str, tab_idx: usize) {
    let info = match libimage_client::probe(data) {
        Some(i) => i,
        None => {
            crate::surf_log!(
                "[surf] image probe failed: src={} bytes={} tab={}",
                src,
                data.len(),
                tab_idx
            );
            return;
        }
    };
    let w = info.width;
    let h = info.height;
    if w == 0 || h == 0 || w > 16384 || h > 16384 || (w as u64 * h as u64) > 67_108_864 {
        crate::surf_log!(
            "[surf] image rejected by dimensions: src={} format={} {}x{} tab={}",
            src,
            libimage_client::format_name(info.format),
            w,
            h,
            tab_idx
        );
        return;
    }
    let mut pixels = vec![0u32; (w * h) as usize];
    let mut scratch = vec![0u8; info.scratch_needed as usize];
    match libimage_client::decode(data, &mut pixels, &mut scratch) {
        Ok(()) => {
            let suspicious_black = decoded_image_looks_suspiciously_black(&pixels);
            if let Some(black_ratio_ppm) = suspicious_black {
                crate::surf_log!(
                    "[surf] WARN suspicious image decode: src={} format={} {}x{} black_ratio_ppm={} tab={}",
                    src,
                    libimage_client::format_name(info.format),
                    w,
                    h,
                    black_ratio_ppm,
                    tab_idx
                );
            }
            let st = crate::state();
            if tab_idx < st.tabs.len() {
                st.tabs[tab_idx].webview.add_image(src, pixels, w, h);
            }
        }
        Err(err) => {
            crate::surf_log!(
                "[surf] image decode failed: src={} format={} {}x{} scratch={} err={:?} tab={}",
                src,
                libimage_client::format_name(info.format),
                w,
                h,
                info.scratch_needed,
                err,
                tab_idx
            );
        }
    }
}

/// Rasterise an SVG document and add the result to the image cache of
/// `tab_idx`, then relayout.
pub(crate) fn decode_svg(data: &[u8], src: &str, tab_idx: usize) {
    decode_svg_no_relayout(data, src, tab_idx);
    crate::request_image_refresh(tab_idx);
}

/// Rasterise an SVG document without triggering a relayout.
///
/// Used by the batch result processor which does a single relayout at the end.
pub(crate) fn decode_svg_no_relayout(data: &[u8], src: &str, tab_idx: usize) {
    let (rw, rh) = svg_intrinsic_raster_size(data).unwrap_or_else(|| match libsvg_client::probe(data) {
        Some((w, h)) => {
            let w = (w as u32).max(1).min(4096);
            let h = (h as u32).max(1).min(4096);
            (w, h)
        }
        None => (256, 256),
    });

    let mut pixels = vec![0u32; (rw * rh) as usize];
    let background = parse_svg_root_background(data).unwrap_or(0x00000000);
    if libsvg_client::render_to_size(data, &mut pixels, rw, rh, background) {
        let st = crate::state();
        if tab_idx < st.tabs.len() {
            st.tabs[tab_idx].webview.add_image(src, pixels, rw, rh);
        }
    }
}

fn parse_svg_root_background(data: &[u8]) -> Option<u32> {
    let text = core::str::from_utf8(data).ok()?;
    let svg_start = text.find("<svg")?;
    let after = &text[svg_start..];
    let tag_end = after.find('>')?;
    let svg_tag = &after[..tag_end];
    let style_attr = extract_attr_value(svg_tag, "style")?;
    parse_background_style(style_attr)
}

fn svg_intrinsic_raster_size(data: &[u8]) -> Option<(u32, u32)> {
    let text = core::str::from_utf8(data).ok()?;
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
        (Some(w), None, _, Some(r)) if r > 0.0 => (w, (((w as f32) / r) + 0.5) as u32),
        (None, Some(h), _, Some(r)) if r > 0.0 => ((((h as f32) * r) + 0.5) as u32, h),
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
        Some(((nums[2] + 0.5) as u32, (nums[3] + 0.5) as u32))
    } else {
        None
    }
}

fn extract_attr_value<'a>(tag_text: &'a str, name: &str) -> Option<&'a str> {
    let mut needle = String::with_capacity(name.len() + 1);
    needle.push_str(name);
    needle.push('=');
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

fn decoded_image_looks_suspiciously_black(pixels: &[u32]) -> Option<u32> {
    if pixels.len() < 4096 {
        return None;
    }
    let mut opaque_black = 0u32;
    let mut opaque_non_black = 0u32;
    for &px in pixels {
        let alpha = (px >> 24) & 0xFF;
        if alpha < 240 {
            continue;
        }
        if px & 0x00FF_FFFF == 0 {
            opaque_black = opaque_black.saturating_add(1);
        } else {
            opaque_non_black = opaque_non_black.saturating_add(1);
        }
    }
    let opaque_total = opaque_black.saturating_add(opaque_non_black);
    if opaque_total < 2048 {
        return None;
    }
    let black_ratio_ppm = opaque_black.saturating_mul(1_000_000) / opaque_total.max(1);
    if black_ratio_ppm >= 985_000 {
        Some(black_ratio_ppm)
    } else {
        None
    }
}

pub(crate) struct DecodedRasterImage {
    pub pixels: Vec<u32>,
    pub width: u32,
    pub height: u32,
    pub format: u32,
    pub suspicious_black_ppm: Option<u32>,
}

const MAX_DECODE_DIM: u32 = 2048;

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
            if orig_w > MAX_DECODE_DIM || orig_h > MAX_DECODE_DIM {
                let max_dim = orig_w.max(orig_h) as u64;
                return (
                    ((orig_w as u64 * MAX_DECODE_DIM as u64) / max_dim).max(1) as u32,
                    ((orig_h as u64 * MAX_DECODE_DIM as u64) / max_dim).max(1) as u32,
                );
            }
            return (orig_w, orig_h);
        }
    };

    if orig_w <= tw.saturating_mul(2) && orig_h <= th.saturating_mul(2) {
        return (orig_w, orig_h);
    }

    (tw.max(1), th.max(1))
}

pub(crate) fn decode_raster_to_image(
    data: &[u8],
    target_w: Option<u32>,
    target_h: Option<u32>,
) -> Result<DecodedRasterImage, libimage_client::ImageError> {
    let info = libimage_client::probe(data).ok_or(libimage_client::ImageError::InvalidData)?;
    let w = info.width;
    let h = info.height;
    if w == 0 || h == 0 || w > 16384 || h > 16384 || (w as u64 * h as u64) > 67_108_864 {
        return Err(libimage_client::ImageError::InvalidData);
    }
    let mut pixels = vec![0u32; (w * h) as usize];
    let mut scratch = vec![0u8; info.scratch_needed as usize];
    libimage_client::decode(data, &mut pixels, &mut scratch)?;
    let (final_w, final_h) = compute_decode_size(w, h, target_w, target_h);
    let pixels = if final_w != w || final_h != h {
        let mut scaled = vec![0u32; (final_w * final_h) as usize];
        if !libimage_client::scale_image(
            &pixels,
            w,
            h,
            &mut scaled,
            final_w,
            final_h,
            libimage_client::MODE_SCALE,
        ) {
            return Err(libimage_client::ImageError::InvalidData);
        }
        scaled
    } else {
        pixels
    };
    let suspicious_black_ppm = decoded_image_looks_suspiciously_black(&pixels);
    Ok(DecodedRasterImage {
        pixels,
        width: final_w,
        height: final_h,
        format: info.format,
        suspicious_black_ppm,
    })
}
