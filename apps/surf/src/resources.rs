// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Async resource loading for the Surf browser.
//!
//! Covers:
//! - HTTP response body decoding (charset detection, Latin-1 → UTF-8)
//! - External CSS stylesheet discovery and submission to the network worker
//! - External image discovery and submission to the network worker
//! - SVG rasterisation and raster image decoding helpers for worker threads

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

pub(crate) const FAVICON_SRC_PREFIX: &str = "surf-internal:favicon:";
const FAVICON_SIZE: u32 = 16;

fn fnv1a64(data: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for &b in data.as_bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash ^ ((data.len() as u64) << 32)
}

pub(crate) fn is_favicon_src(src: &str) -> bool {
    src.starts_with(FAVICON_SRC_PREFIX)
}

pub(crate) fn scale_icon_pixels(src: &[u32], sw: u32, sh: u32) -> Option<(Vec<u32>, u32, u32)> {
    if src.is_empty() || sw == 0 || sh == 0 {
        return None;
    }
    if sw == FAVICON_SIZE && sh == FAVICON_SIZE {
        return Some((Vec::from(src), sw, sh));
    }
    let mut dst = vec![0u32; (FAVICON_SIZE * FAVICON_SIZE) as usize];
    for y in 0..FAVICON_SIZE {
        let sy = (y * sh / FAVICON_SIZE).min(sh - 1);
        for x in 0..FAVICON_SIZE {
            let sx = (x * sw / FAVICON_SIZE).min(sw - 1);
            dst[(y * FAVICON_SIZE + x) as usize] = src[(sy * sw + sx) as usize];
        }
    }
    Some((dst, FAVICON_SIZE, FAVICON_SIZE))
}

fn image_already_queued_or_decoded(tab_index: usize, src: &str) -> bool {
    let st = crate::state();
    if tab_index >= st.tabs.len() {
        return true;
    }
    let tab = &st.tabs[tab_index];
    tab.webview.has_decoded_image(src)
        || tab
            .requested_image_urls
            .iter()
            .any(|existing| existing == src)
        || tab.deferred_images.iter().any(|req| req.src == src)
}

fn mark_image_requested(tab_index: usize, src: &str) -> bool {
    let st = crate::state();
    if tab_index >= st.tabs.len() {
        return false;
    }
    let tab = &mut st.tabs[tab_index];
    if tab.webview.has_decoded_image(src)
        || tab
            .requested_image_urls
            .iter()
            .any(|existing| existing == src)
        || tab.deferred_images.iter().any(|req| req.src == src)
    {
        return false;
    }
    tab.requested_image_urls.push(String::from(src));
    true
}

fn inline_svg_cache_matches(tab_index: usize, node_id: usize, content_hash: u64) -> bool {
    let st = crate::state();
    if tab_index >= st.tabs.len() {
        return false;
    }
    st.tabs[tab_index]
        .inline_svg_cache
        .iter()
        .any(|entry| entry.node_id == node_id && entry.content_hash == content_hash)
}

fn remember_inline_svg_cache(tab_index: usize, node_id: usize, content_hash: u64) {
    let st = crate::state();
    if tab_index >= st.tabs.len() {
        return;
    }
    let cache = &mut st.tabs[tab_index].inline_svg_cache;
    if let Some(entry) = cache.iter_mut().find(|entry| entry.node_id == node_id) {
        entry.content_hash = content_hash;
    } else {
        cache.push(crate::tab::InlineSvgCacheEntry {
            node_id,
            content_hash,
        });
    }
}

pub(crate) fn resolve_css_resource_urls(css: &str, css_url: &crate::http::Url) -> String {
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

fn resolve_css_url(css_url: &crate::http::Url, raw_url: &str) -> String {
    let trimmed = raw_url.trim();
    if trimmed.is_empty()
        || trimmed.starts_with('#')
        || starts_with_ignore_case(trimmed, "data:")
        || starts_with_ignore_case(trimmed, "blob:")
        || starts_with_ignore_case(trimmed, "http://")
        || starts_with_ignore_case(trimmed, "https://")
        || trimmed.starts_with("//")
    {
        return String::from(raw_url);
    }
    let resolved = crate::http::resolve_url(css_url, trimmed);
    crate::ui::format_url(&resolved)
}

fn rest_starts_with_ci(haystack: &[u8], pos: usize, needle: &[u8]) -> bool {
    haystack
        .get(pos..pos.saturating_add(needle.len()))
        .is_some_and(|slice| slice.eq_ignore_ascii_case(needle))
}

fn starts_with_ignore_case(value: &str, prefix: &str) -> bool {
    value
        .as_bytes()
        .get(..prefix.len())
        .is_some_and(|slice| slice.eq_ignore_ascii_case(prefix.as_bytes()))
}

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

fn explicit_image_decode_hints(
    webview: &libwebview::WebView,
    node_id: usize,
    bounds: Option<(i32, i32, i32, i32)>,
) -> (Option<i32>, Option<i32>) {
    let Some((_, _, box_w, box_h)) = bounds else {
        return (None, None);
    };
    let Some(style) = webview.resolved_style_ref(node_id) else {
        return (None, None);
    };
    let width = if style.width.is_some() && box_w > 0 {
        Some(box_w)
    } else {
        None
    };
    let height = if style.height.is_some() && box_h > 0 {
        Some(box_h)
    } else {
        None
    };
    (width, height)
}

fn image_decode_dimensions(
    dom: &libwebview::dom::Dom,
    webview: &libwebview::WebView,
    node_id: usize,
) -> (Option<i32>, Option<i32>) {
    let bounds = webview.node_bounds(node_id);
    let (css_w, css_h) = explicit_image_decode_hints(webview, node_id, bounds);
    let has_attr_w = dom
        .attr(node_id, "width")
        .and_then(parse_dimension_attr)
        .is_some();
    let has_attr_h = dom
        .attr(node_id, "height")
        .and_then(parse_dimension_attr)
        .is_some();
    let attr_bounds = bounds.and_then(|(_, _, w, h)| {
        if w > 0 && h > 0 && (has_attr_w || has_attr_h) {
            Some((w, h))
        } else {
            None
        }
    });
    (
        css_w.or_else(|| attr_bounds.map(|(w, _)| w)),
        css_h.or_else(|| attr_bounds.map(|(_, h)| h)),
    )
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

pub(crate) fn queue_favicon(
    dom: &libwebview::dom::Dom,
    base_url: &crate::http::Url,
    tab_index: usize,
) -> bool {
    if base_url.host.is_empty() || (base_url.scheme != "http" && base_url.scheme != "https") {
        return false;
    }

    let mut best_href: Option<&str> = None;
    let mut best_score = 0u8;
    for (i, node) in dom.nodes.iter().enumerate() {
        if let libwebview::dom::NodeType::Element {
            tag: libwebview::dom::Tag::Link,
            ..
        } = &node.node_type
        {
            let rel = dom.attr(i, "rel").unwrap_or("");
            let score = favicon_rel_score(rel);
            if score == 0 || score < best_score {
                continue;
            }
            let Some(href) = dom.attr(i, "href") else {
                continue;
            };
            if href.is_empty() || href.starts_with("data:") {
                continue;
            }
            best_href = Some(href);
            best_score = score;
            if score >= 3 {
                break;
            }
        }
    }

    let icon_url = if let Some(href) = best_href {
        crate::http::resolve_url(base_url, href)
    } else {
        favicon_fallback_url(base_url)
    };
    submit_favicon_url(tab_index, icon_url)
}

pub(crate) fn queue_favicon_fallback(tab_index: usize) -> bool {
    let base_url = {
        let st = crate::state();
        if tab_index >= st.tabs.len() {
            return false;
        }
        let Some(url) = &st.tabs[tab_index].current_url else {
            return false;
        };
        url.clone()
    };
    if base_url.host.is_empty() || (base_url.scheme != "http" && base_url.scheme != "https") {
        return false;
    }
    submit_favicon_url(tab_index, favicon_fallback_url(&base_url))
}

fn favicon_fallback_url(base_url: &crate::http::Url) -> crate::http::Url {
    let mut url = base_url.clone();
    url.path = String::from("/favicon.ico");
    url
}

fn favicon_rel_score(rel: &str) -> u8 {
    let mut score = 0u8;
    for token in rel.split_ascii_whitespace() {
        if token.eq_ignore_ascii_case("icon") {
            score = score.max(3);
        } else if token.eq_ignore_ascii_case("apple-touch-icon")
            || token.eq_ignore_ascii_case("apple-touch-icon-precomposed")
        {
            score = score.max(2);
        } else if token.eq_ignore_ascii_case("shortcut") {
            score = score.max(1);
        }
    }
    score
}

fn submit_favicon_url(tab_index: usize, icon_url: crate::http::Url) -> bool {
    let generation = {
        let st = crate::state();
        if tab_index >= st.tabs.len() {
            return false;
        }
        st.tabs[tab_index].load_state.generation
    };
    let url_text = crate::ui::format_url(&icon_url);
    {
        let st = crate::state();
        if tab_index >= st.tabs.len() {
            return false;
        }
        if st.tabs[tab_index].requested_favicon_url == url_text {
            return false;
        }
        st.tabs[tab_index].requested_favicon_url = url_text.clone();
    }

    let mut src = String::from(FAVICON_SRC_PREFIX);
    src.push_str(&url_text);
    crate::net_worker::submit(crate::net_worker::FetchRequest::Image {
        tab_index,
        src,
        url: icon_url,
        target_width: Some(FAVICON_SIZE),
        target_height: Some(FAVICON_SIZE),
        priority: crate::net_worker::ImagePriority::Viewport,
        from_deferred: false,
        generation,
    });
    crate::ensure_net_poll_timer();
    true
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
    let font_faces: Vec<(String, String, u32, bool, libwebview::css::FontDisplay)> = webview
        .all_font_faces()
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
    queue_font_face_batch(tab_index, generation, base_url, &font_faces);
}

pub(crate) fn queue_font_face_batch(
    tab_index: usize,
    generation: u32,
    base_url: &crate::http::Url,
    font_faces: &[(String, String, u32, bool, libwebview::css::FontDisplay)],
) {
    let st = crate::state();
    if tab_index >= st.tabs.len() {
        return;
    }

    let mut immediate = 0u32;
    let mut deferred = 0u32;
    let mut queued_faces: Vec<(String, u32, bool)> = Vec::new();

    for (family, src_url, weight, italic, display) in font_faces {
        if src_url.is_empty() {
            continue;
        }
        if st.tabs[tab_index]
            .webview
            .has_web_font_style(family, *weight, *italic)
        {
            continue;
        }
        if queued_faces
            .iter()
            .any(|(f, w, i)| f == family && *w == *weight && *i == *italic)
        {
            continue;
        }
        queued_faces.push((family.clone(), *weight, *italic));
        if src_url.starts_with("data:") {
            if let Some(font_data) = decode_font_data_uri(src_url) {
                if let Some(font_id) = load_valid_web_font_data(family, &font_data) {
                    st.tabs[tab_index]
                        .webview
                        .register_web_font_with_style(family, *weight, *italic, font_id);
                    immediate += 1;
                    crate::surf_log!(
                        "[surf] loaded inline data font '{}' -> id {} ({} bytes)",
                        family,
                        font_id,
                        font_data.len()
                    );
                }
            }
            continue;
        }

        let resolved = crate::http::resolve_url(base_url, src_url);
        let resolved_key = crate::ui::format_url(&resolved);
        if st.tabs[tab_index]
            .requested_font_urls
            .iter()
            .any(|url| url == &resolved_key)
        {
            continue;
        }
        st.tabs[tab_index].requested_font_urls.push(resolved_key);

        if should_defer_font_face(*display) {
            st.tabs[tab_index]
                .deferred_fonts
                .push(crate::tab::DeferredFontRequest {
                    family: family.clone(),
                    weight: *weight,
                    italic: *italic,
                    url: resolved,
                    display: *display,
                    generation,
                });
            deferred += 1;
        } else {
            crate::net_worker::submit(crate::net_worker::FetchRequest::Font {
                tab_index,
                family: family.clone(),
                weight: *weight,
                italic: *italic,
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

pub(crate) fn load_valid_web_font_data(family: &str, data: &[u8]) -> Option<u32> {
    if web_font_is_cff_only(data) {
        crate::surf_log!(
            "[surf] skipped web font '{}' ({}): CFF outlines are disabled until rasterization is correct",
            family,
            web_font_format_hint(data)
        );
        return None;
    }

    let Some(font_id) = libfont_client::load_data(data) else {
        crate::surf_log!(
            "[surf] rejected web font '{}' ({}): unsupported or invalid data",
            family,
            web_font_format_hint(data)
        );
        return None;
    };
    if web_font_is_renderable(font_id) {
        return Some(font_id);
    }
    libfont_client::unload(font_id);
    crate::surf_log!(
        "[surf] rejected web font '{}' -> id {}: render validation failed",
        family,
        font_id
    );
    None
}

fn web_font_is_cff_only(data: &[u8]) -> bool {
    (data.len() >= 8
        && ((&data[..4] == b"wOF2" && &data[4..8] == b"OTTO")
            || (&data[..4] == b"wOFF" && &data[4..8] == b"OTTO")))
        || (data.len() >= 4 && &data[..4] == b"OTTO")
}

fn web_font_format_hint(data: &[u8]) -> &'static str {
    if data.len() >= 8 && &data[..4] == b"wOF2" {
        if &data[4..8] == b"OTTO" {
            "WOFF2/CFF"
        } else if &data[4..8] == b"\0\x01\0\0" || &data[4..8] == b"true" {
            "WOFF2/TrueType"
        } else {
            "WOFF2"
        }
    } else if data.len() >= 8 && &data[..4] == b"wOFF" {
        if &data[4..8] == b"OTTO" {
            "WOFF/CFF"
        } else {
            "WOFF"
        }
    } else if data.len() >= 4 && &data[..4] == b"OTTO" {
        "OpenType/CFF"
    } else if data.len() >= 4 && (&data[..4] == b"\0\x01\0\0" || &data[..4] == b"true") {
        "TrueType"
    } else {
        "unknown"
    }
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

fn percent_decode_data_uri_payload(payload: &str) -> Vec<u8> {
    let bytes = payload.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = hex_val(bytes[i + 1]);
            let lo = hex_val(bytes[i + 2]);
            if hi >= 0 && lo >= 0 {
                out.push(((hi as u8) << 4) | lo as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

fn hex_val(b: u8) -> i8 {
    match b {
        b'0'..=b'9' => (b - b'0') as i8,
        b'a'..=b'f' => (10 + b - b'a') as i8,
        b'A'..=b'F' => (10 + b - b'A') as i8,
        _ => -1,
    }
}

fn decode_data_uri_bytes(src: &str, allow_mime: fn(&str) -> bool) -> Option<(Vec<u8>, String)> {
    let rest = src.strip_prefix("data:")?;
    let comma = rest.find(',')?;
    let meta = &rest[..comma];
    let payload = &rest[comma + 1..];

    let mime = meta.split(';').next().unwrap_or("").trim();
    if !allow_mime(mime) {
        return None;
    }

    let is_base64 = meta.contains(";base64");
    let bytes = if is_base64 {
        base64_decode(payload.as_bytes())
    } else {
        percent_decode_data_uri_payload(payload)
    };

    if bytes.is_empty() {
        return None;
    }
    Some((bytes, String::from(mime)))
}

/// Parse an image `data:` URI into raw bytes and whether it is SVG.
fn decode_data_uri(src: &str) -> Option<(Vec<u8>, bool)> {
    let (bytes, mime) = decode_data_uri_bytes(src, |mime| {
        mime.starts_with("image/") || mime.eq_ignore_ascii_case("image/svg+xml")
    })?;
    let is_svg =
        mime.eq_ignore_ascii_case("image/svg+xml") || mime.eq_ignore_ascii_case("image/svg");
    Some((bytes, is_svg))
}

fn decode_font_data_uri(src: &str) -> Option<Vec<u8>> {
    decode_data_uri_bytes(src, |mime| {
        mime.starts_with("font/")
            || mime.starts_with("application/font")
            || mime.starts_with("application/x-font")
            || mime.eq_ignore_ascii_case("application/octet-stream")
    })
    .map(|(bytes, _)| bytes)
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
    let mut immediate_budget = if startup_critical_only {
        12usize
    } else {
        32usize
    };
    let mut high_priority_budget = if startup_critical_only {
        8usize
    } else {
        16usize
    };
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
                    if image_already_queued_or_decoded(tab_index, &src) {
                        continue;
                    }
                    if let Some((bytes, is_svg)) = decode_data_uri(&src) {
                        let key = src.clone();
                        mark_image_requested(tab_index, &key);
                        if is_svg {
                            decode_svg_no_relayout(&bytes, &key, tab_index);
                        } else {
                            decode_raster_no_relayout(&bytes, &key, tab_index);
                        }
                    }
                    continue;
                }

                if image_already_queued_or_decoded(tab_index, &src) {
                    continue;
                }
                let img_url = crate::http::resolve_url(base_url, &src);
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
                        image_decode_dimensions(dom, webview, i)
                    }
                };
                let should_start_immediately = if lazy_requested && !initial_viewport_hit {
                    false
                } else if startup_critical_only {
                    if high_priority && initial_viewport_hit && high_priority_budget > 0 {
                        high_priority_budget -= 1;
                        true
                    } else if initial_viewport_hit && immediate_budget > 0 {
                        immediate_budget -= 1;
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
                    if !mark_image_requested(tab_index, &src) {
                        continue;
                    }
                    crate::net_worker::submit(crate::net_worker::FetchRequest::Image {
                        tab_index,
                        src,
                        url: img_url,
                        target_width: width.map(|v| v.max(1) as u32),
                        target_height: height.map(|v| v.max(1) as u32),
                        priority: crate::net_worker::ImagePriority::Viewport,
                        from_deferred: false,
                        generation,
                    });
                    immediate_count += 1;
                } else {
                    if !mark_image_requested(tab_index, &src) {
                        continue;
                    }
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

    let mut bg_data_uri_decodes: Vec<String> = Vec::new();
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
                    if webview.has_decoded_image(bg_src)
                        || st.tabs[tab_index]
                            .requested_image_urls
                            .iter()
                            .any(|existing| existing == bg_src)
                        || bg_data_uri_decodes.iter().any(|k| k == bg_src)
                    {
                        continue;
                    }
                    bg_data_uri_decodes.push(bg_src.clone());
                    continue;
                }
                if image_already_queued_or_decoded(tab_index, bg_src)
                    || deferred.iter().any(|(_, req)| req.src == *bg_src)
                {
                    continue;
                }
                let img_url = crate::http::resolve_url(base_url, bg_src);
                let priority = crate::net_worker::ImagePriority::Viewport;
                let (width, height, initial_viewport_hit) = {
                    let viewport_h = webview.viewport_height() as i32;
                    if let Some((_, y, w, h)) = webview.node_bounds(i) {
                        (Some(w), Some(h), y < viewport_h + 640 && y + h > -192)
                    } else {
                        (None, None, false)
                    }
                };
                if initial_viewport_hit && immediate_budget > 0 {
                    immediate_budget -= 1;
                    if !mark_image_requested(tab_index, bg_src) {
                        continue;
                    }
                    crate::net_worker::submit(crate::net_worker::FetchRequest::Image {
                        tab_index,
                        src: bg_src.clone(),
                        url: img_url,
                        target_width: width.map(|v| v.max(1) as u32),
                        target_height: height.map(|v| v.max(1) as u32),
                        priority,
                        from_deferred: false,
                        generation,
                    });
                    immediate_count += 1;
                    count += 1;
                    continue;
                }
                if !mark_image_requested(tab_index, bg_src) {
                    continue;
                }
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

    for key in bg_data_uri_decodes {
        if let Some((bytes, is_svg)) = decode_data_uri(&key) {
            mark_image_requested(tab_index, &key);
            if is_svg {
                decode_svg_no_relayout(&bytes, &key, tab_index);
            } else {
                decode_raster_no_relayout(&bytes, &key, tab_index);
            }
        }
    }

    deferred.sort_by(|a, b| b.0.cmp(&a.0));

    let st = crate::state();
    if tab_index < st.tabs.len() {
        st.tabs[tab_index]
            .deferred_images
            .extend(deferred.into_iter().map(|(_, req)| req));
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

pub(crate) fn queue_background_images(
    base_url: &crate::http::Url,
    tab_index: usize,
    immediate_budget: usize,
) -> usize {
    let generation = crate::net_worker::current_generation();
    let mut candidates: Vec<(i32, crate::tab::DeferredImageRequest)> = Vec::new();
    let mut data_uri_decodes: Vec<String> = Vec::new();

    {
        let st = crate::state();
        if tab_index >= st.tabs.len() {
            return 0;
        }
        let Some(dom) = st.tabs[tab_index].webview.dom() else {
            return 0;
        };
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
                if webview.has_decoded_image(bg_src)
                    || st.tabs[tab_index]
                        .requested_image_urls
                        .iter()
                        .any(|existing| existing == bg_src)
                    || data_uri_decodes.iter().any(|k| k == bg_src)
                {
                    continue;
                }
                data_uri_decodes.push(bg_src.clone());
                continue;
            }
            if webview.has_decoded_image(bg_src)
                || st.tabs[tab_index]
                    .requested_image_urls
                    .iter()
                    .any(|existing| existing == bg_src)
                || st.tabs[tab_index]
                    .deferred_images
                    .iter()
                    .any(|req| req.src == *bg_src)
            {
                continue;
            }
            let img_url = crate::http::resolve_url(base_url, bg_src);
            let (width, height) = webview
                .node_bounds(i)
                .map(|(_, _, w, h)| (Some(w), Some(h)))
                .unwrap_or((None, None));
            let rank = deferred_image_rank(i, width, height, false, true);
            candidates.push((
                rank,
                crate::tab::DeferredImageRequest {
                    node_id: i,
                    dom_index: i,
                    src: bg_src.clone(),
                    url: img_url,
                    priority: crate::net_worker::ImagePriority::Viewport,
                    width,
                    height,
                    lazy_requested: false,
                    high_priority: true,
                    generation,
                },
            ));
        }
    }

    for key in data_uri_decodes {
        if let Some((bytes, is_svg)) = decode_data_uri(&key) {
            mark_image_requested(tab_index, &key);
            if is_svg {
                decode_svg_no_relayout(&bytes, &key, tab_index);
            } else {
                decode_raster_no_relayout(&bytes, &key, tab_index);
            }
        }
    }

    if candidates.is_empty() {
        return 0;
    }

    candidates.sort_by(|a, b| b.0.cmp(&a.0));
    let immediate_count = core::cmp::min(immediate_budget, candidates.len());
    let mut submitted_immediate = 0usize;
    let mut deferred = Vec::new();
    for (idx, (_, req)) in candidates.into_iter().enumerate() {
        if idx < immediate_count {
            if !mark_image_requested(tab_index, &req.src) {
                continue;
            }
            crate::net_worker::submit(crate::net_worker::FetchRequest::Image {
                tab_index,
                src: req.src,
                url: req.url,
                target_width: req.width.map(|v| v.max(1) as u32),
                target_height: req.height.map(|v| v.max(1) as u32),
                priority: crate::net_worker::ImagePriority::Viewport,
                from_deferred: false,
                generation,
            });
            submitted_immediate += 1;
        } else {
            if !mark_image_requested(tab_index, &req.src) {
                continue;
            }
            deferred.push(req);
        }
    }

    let deferred_count = deferred.len();
    if deferred_count > 0 {
        let st = crate::state();
        if tab_index < st.tabs.len() {
            st.tabs[tab_index].deferred_images.extend(deferred);
        }
    }
    crate::surf_log!(
        "[surf] background image queue summary: total={} immediate={} deferred={} tab={}",
        submitted_immediate + deferred_count,
        submitted_immediate,
        deferred_count,
        tab_index
    );
    crate::ensure_net_poll_timer();
    submitted_immediate + deferred_count
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
        deferred_image_runtime_rank(webview, b, scroll_y, viewport_h).cmp(
            &deferred_image_runtime_rank(webview, a, scroll_y, viewport_h),
        )
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
            from_deferred: true,
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

pub(crate) fn submit_viewport_deferred_images(tab_index: usize, max_batch: usize) -> usize {
    if max_batch == 0 {
        return 0;
    }
    let st = crate::state();
    if tab_index >= st.tabs.len() {
        return 0;
    }
    let tab = &mut st.tabs[tab_index];
    if tab.deferred_images.is_empty() {
        return 0;
    }

    let scroll_y = tab.webview.scroll_view().get_state() as i32;
    let viewport_h = tab.webview.viewport_height() as i32;
    let mut candidates: Vec<(usize, i32)> = {
        let webview = &tab.webview;
        tab.deferred_images
            .iter()
            .enumerate()
            .filter_map(|(idx, req)| {
                let (_, y, _, h) = webview.node_bounds(req.node_id)?;
                let top = scroll_y - viewport_h / 2;
                let bottom = scroll_y + viewport_h.max(1) + viewport_h;
                if y < bottom && y + h > top {
                    Some((
                        idx,
                        deferred_image_runtime_rank(webview, req, scroll_y, viewport_h),
                    ))
                } else {
                    None
                }
            })
            .collect()
    };
    if candidates.is_empty() {
        return 0;
    }
    candidates.sort_by(|a, b| b.1.cmp(&a.1));
    candidates.truncate(max_batch);
    candidates.sort_by(|a, b| b.0.cmp(&a.0));

    let mut requests = Vec::new();
    for (idx, _) in candidates {
        requests.push(tab.deferred_images.remove(idx));
    }
    tab.deferred_images_inflight += requests.len();
    let count = requests.len();

    for req in requests {
        crate::net_worker::submit(crate::net_worker::FetchRequest::Image {
            tab_index,
            src: req.src,
            url: req.url,
            target_width: req.width.map(|v| v.max(1) as u32),
            target_height: req.height.map(|v| v.max(1) as u32),
            priority: crate::net_worker::ImagePriority::Viewport,
            from_deferred: true,
            generation: req.generation,
        });
    }
    crate::surf_log!(
        "[surf] promoted {} visible deferred image(s) for tab {} (remaining={} inflight={})",
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
            weight: req.weight,
            italic: req.italic,
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

/// Scan the DOM for `<svg>` nodes, reconstruct their inner markup, rasterise
/// each one with `libsvg_client`, and store the result in the image cache under
/// the synthetic key `__svg_<node_id>__`.
///
/// Called once per page load, immediately after `queue_images()`.
pub(crate) fn queue_inline_svgs(
    webview: &libwebview::WebView,
    base_url: &crate::http::Url,
    tab_index: usize,
) -> bool {
    let Some(dom) = webview.dom() else {
        return false;
    };
    let mut count = 0u32;
    let mut total = 0u32;
    let mut skipped_no_markup = 0u32;
    let mut skipped_cached = 0u32;
    let mut sprite_urls = Vec::new();

    for (node_id, node) in dom.nodes.iter().enumerate() {
        if let libwebview::dom::NodeType::Element {
            tag: libwebview::dom::Tag::Svg,
            attrs,
            ..
        } = &node.node_type
        {
            total += 1;
            let inner = match inline_svg_inner_markup(dom, node_id) {
                Some(s) => s,
                None => {
                    skipped_no_markup += 1;
                    continue;
                }
            };

            let svg = reconstruct_inline_svg(attrs, &inner);
            let svg = apply_svg_inherited_color(
                svg,
                webview.resolved_style_ref(node_id).map(|style| style.color),
            );
            collect_external_svg_use_urls(&svg, base_url, &mut sprite_urls);

            let key = inline_svg_key(node_id);
            let content_hash = fnv1a64(&svg);
            if inline_svg_cache_matches(tab_index, node_id, content_hash) {
                skipped_cached += 1;
                continue;
            }
            decode_svg_no_relayout(svg.as_bytes(), &key, tab_index);
            remember_inline_svg_cache(tab_index, node_id, content_hash);
            count += 1;
        }
    }

    let sprite_count = sprite_urls.len();
    queue_svg_sprite_fetches(tab_index, sprite_urls);

    crate::surf_log!(
        "[surf] inline SVG scan: total={} rasterised={} skipped_cached={} skipped_no_markup={} sprites_queued={} tab={}",
        total,
        count,
        skipped_cached,
        skipped_no_markup,
        sprite_count,
        tab_index
    );
    count > 0
}

pub(crate) fn reraster_inline_svgs_with_sprite(
    webview: &libwebview::WebView,
    base_url: &crate::http::Url,
    tab_index: usize,
    sprite_url: &str,
    sprite_body: &[u8],
) -> bool {
    let Some(dom) = webview.dom() else {
        return false;
    };
    let Ok(sprite_text) = core::str::from_utf8(sprite_body) else {
        return false;
    };
    let mut count = 0u32;

    for (node_id, node) in dom.nodes.iter().enumerate() {
        if let libwebview::dom::NodeType::Element {
            tag: libwebview::dom::Tag::Svg,
            attrs,
            ..
        } = &node.node_type
        {
            let Some(inner) = inline_svg_inner_markup(dom, node_id) else {
                continue;
            };
            let svg = reconstruct_inline_svg(attrs, &inner);
            // Sprite content (e.g. fill="var(--primary-color)") would survive
            // through apply_svg_inherited_color if we ran it first, so expand
            // the sprite uses *before* substituting CSS variables / fills.
            let Some(expanded) =
                expand_svg_uses_from_sprite(&svg, base_url, sprite_url, sprite_text)
            else {
                continue;
            };
            let expanded = apply_svg_inherited_color(
                expanded,
                webview.resolved_style_ref(node_id).map(|style| style.color),
            );
            let key = inline_svg_key(node_id);
            decode_svg_no_relayout(expanded.as_bytes(), &key, tab_index);
            count += 1;
        }
    }

    if count > 0 {
        crate::surf_log!(
            "[surf] rerasterised {} inline SVG(s) after sprite load {}",
            count,
            sprite_url
        );
        true
    } else {
        false
    }
}

fn reconstruct_inline_svg(attrs: &[libwebview::dom::Attr], inner: &str) -> String {
    let mut svg = String::from("<svg");
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
            svg.push_str(canonical_svg_attr_name(n));
            svg.push_str("=\"");
            svg.push_str(&escape_xml_attr(&attr.value));
            svg.push('"');
        }
    }
    if !attrs.iter().any(|a| a.name == "xmlns") {
        svg.push_str(" xmlns=\"http://www.w3.org/2000/svg\"");
    }
    svg.push('>');
    svg.push_str(inner);
    svg.push_str("</svg>");
    svg
}

fn inline_svg_inner_markup(dom: &libwebview::dom::Dom, node_id: usize) -> Option<String> {
    let node = dom.nodes.get(node_id)?;

    for &child_id in &node.children {
        if let libwebview::dom::NodeType::Text(ref text) = dom.nodes[child_id].node_type {
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

fn serialize_svg_dom_node(dom: &libwebview::dom::Dom, node_id: usize, out: &mut String) {
    let Some(node) = dom.nodes.get(node_id) else {
        return;
    };
    match &node.node_type {
        libwebview::dom::NodeType::Text(text) => escape_xml_text_into(text, out),
        libwebview::dom::NodeType::Element { tag, attrs } => {
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

fn apply_svg_inherited_color(svg: String, color: Option<u32>) -> String {
    let Some(color) = color else {
        return substitute_css_vars(svg, "#000000");
    };
    let rgb = color & 0x00FF_FFFF;
    let hex = alloc::format!("#{:06x}", rgb);
    let mut out = svg
        .replace("currentColor", &hex)
        .replace("currentcolor", &hex);
    out = substitute_css_vars(out, &hex);
    out = inject_svg_root_color(&out, &hex);
    inject_default_svg_fill(&out, &hex)
}

/// Replace `var(--name [, fallback])` occurrences with `replacement`.
///
/// libsvg has no notion of CSS custom properties, so any `fill="var(...)"`
/// or `stroke="var(...)"` attribute would otherwise leave the shape with an
/// unresolvable paint and render nothing. The fallback inside the `var()` is
/// ignored — we always substitute the caller's preferred colour, since we
/// have no CSS engine to evaluate the fallback expression here either.
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
            // Find matching ')' honouring nested parens (e.g. `var(--x, var(--y))`).
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

fn collect_external_svg_use_urls(svg: &str, base_url: &crate::http::Url, out: &mut Vec<String>) {
    let mut cursor = 0usize;
    while let Some(rel_start) = svg[cursor..].find("<use") {
        let start = cursor + rel_start;
        let Some(tag_end_rel) = svg[start..].find('>') else {
            break;
        };
        let tag_end = start + tag_end_rel + 1;
        let use_tag = &svg[start..tag_end];
        if let Some(sprite_url) = svg_use_sprite_url(use_tag, base_url) {
            if !out.iter().any(|url| url == &sprite_url) {
                out.push(sprite_url);
            }
        }
        cursor = tag_end;
    }
}

fn queue_svg_sprite_fetches(tab_index: usize, sprite_urls: Vec<String>) {
    if sprite_urls.is_empty() {
        return;
    }
    let st = crate::state();
    if tab_index >= st.tabs.len() {
        return;
    }
    let generation = st.tabs[tab_index].load_state.generation;
    let mut queued = 0u32;
    for src in sprite_urls {
        if st.tabs[tab_index].webview.has_decoded_image(&src) {
            continue;
        }
        let url = match crate::http::parse_url(&src) {
            Ok(url) => url,
            Err(_) => continue,
        };
        crate::net_worker::submit(crate::net_worker::FetchRequest::Image {
            tab_index,
            src,
            url,
            target_width: None,
            target_height: None,
            priority: crate::net_worker::ImagePriority::Viewport,
            from_deferred: false,
            generation,
        });
        queued += 1;
    }
    if queued > 0 {
        crate::surf_log!("[surf] queued {} external SVG sprite(s)", queued);
        crate::ensure_net_poll_timer();
    }
}

fn svg_use_sprite_url(use_tag: &str, base_url: &crate::http::Url) -> Option<String> {
    let href = extract_attr_value(use_tag, "href")
        .or_else(|| extract_attr_value(use_tag, "xlink:href"))?
        .trim();
    if href.is_empty() || href.starts_with('#') || href.starts_with("data:") {
        return None;
    }
    let hash = href.rfind('#')?;
    if hash == 0 || href[hash + 1..].is_empty() {
        return None;
    }
    let url = crate::http::resolve_url(base_url, &href[..hash]);
    Some(crate::ui::format_url(&url))
}

fn expand_svg_uses_from_sprite(
    svg: &str,
    base_url: &crate::http::Url,
    sprite_url: &str,
    sprite_text: &str,
) -> Option<String> {
    let mut out = String::with_capacity(svg.len());
    let mut cursor = 0usize;
    let mut changed = false;

    while let Some(rel_start) = svg[cursor..].find("<use") {
        let start = cursor + rel_start;
        out.push_str(&svg[cursor..start]);
        let Some(tag_end_rel) = svg[start..].find('>') else {
            out.push_str(&svg[start..]);
            return changed.then_some(out);
        };
        let tag_end = start + tag_end_rel + 1;
        let use_tag = &svg[start..tag_end];

        let replacement = extract_attr_value(use_tag, "href")
            .or_else(|| extract_attr_value(use_tag, "xlink:href"))
            .and_then(|href| expand_sprite_use(href, base_url, sprite_url, sprite_text));

        if let Some(replacement) = replacement {
            out.push_str(&replacement);
            changed = true;
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
    changed.then_some(out)
}

fn expand_sprite_use(
    href: &str,
    base_url: &crate::http::Url,
    sprite_url: &str,
    sprite_text: &str,
) -> Option<String> {
    let href = href.trim();
    if href.is_empty() || href.starts_with('#') || href.starts_with("data:") {
        return None;
    }
    let hash = href.rfind('#')?;
    let url = crate::http::resolve_url(base_url, &href[..hash]);
    if crate::ui::format_url(&url) != sprite_url {
        return None;
    }
    let symbol_id = &href[hash + 1..];
    if symbol_id.is_empty() {
        return None;
    }
    extract_svg_fragment_by_id(sprite_text, symbol_id)
}

fn extract_svg_fragment_by_id(sprite: &str, id: &str) -> Option<String> {
    let mut id_dq = String::from("id=\"");
    id_dq.push_str(id);
    id_dq.push('"');
    let mut id_sq = String::from("id='");
    id_sq.push_str(id);
    id_sq.push('\'');
    let mut id_bare = String::from("id=");
    id_bare.push_str(id);

    let id_pos = [id_dq.as_str(), id_sq.as_str(), id_bare.as_str()]
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
        return Some(String::from(open_tag));
    }

    let mut close = String::from("</");
    close.push_str(tag_name);
    close.push('>');
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
    let intrinsic = svg_intrinsic_raster_size(data);
    let probed = if intrinsic.is_none() {
        libsvg_client::probe(data)
    } else {
        None
    };
    let (rw, rh) = intrinsic.unwrap_or_else(|| match probed {
        Some((w, h)) => {
            let w = (w as u32).max(1);
            let h = (h as u32).max(1);
            cap_svg_raster_size(w, h)
        }
        None => (256, 256),
    });
    let (rw, rh) = cap_svg_raster_size(rw, rh);

    let Some(pixel_count) = (rw as usize).checked_mul(rh as usize) else {
        crate::surf_log!(
            "[surf] svg pixel-count overflow: src={} {}x{} tab={}",
            src,
            rw,
            rh,
            tab_idx
        );
        return;
    };
    let background = parse_svg_root_background(data).unwrap_or(0x00000000);
    let supersample = svg_supersample_factor(rw, rh);
    let (ok, pixels) = if supersample > 1 {
        let hi_w = rw.saturating_mul(supersample);
        let hi_h = rh.saturating_mul(supersample);
        let Some(hi_count) = (hi_w as usize).checked_mul(hi_h as usize) else {
            crate::surf_log!(
                "[surf] svg supersample overflow: src={} {}x{} factor={} tab={}",
                src,
                rw,
                rh,
                supersample,
                tab_idx
            );
            return;
        };
        let mut hi_pixels = vec![0u32; hi_count];
        let ok = libsvg_client::render_to_size(data, &mut hi_pixels, hi_w, hi_h, background);
        let mut lo_pixels = vec![0u32; pixel_count];
        if ok {
            let scaled = libimage_client::scale_image(
                &hi_pixels,
                hi_w,
                hi_h,
                &mut lo_pixels,
                rw,
                rh,
                libimage_client::MODE_SCALE,
            );
            (scaled, lo_pixels)
        } else {
            (false, lo_pixels)
        }
    } else {
        let mut pixels = vec![0u32; pixel_count];
        let ok = libsvg_client::render_to_size(data, &mut pixels, rw, rh, background);
        (ok, pixels)
    };
    crate::surf_log!(
        "[surf] svg decode: src={} bytes={} {}x{} aa={} intrinsic={} probed={} ok={} tab={}",
        src,
        data.len(),
        rw,
        rh,
        supersample,
        intrinsic.is_some(),
        probed.is_some(),
        ok,
        tab_idx
    );
    if ok {
        let st = crate::state();
        if tab_idx < st.tabs.len() {
            st.tabs[tab_idx].webview.add_image(src, pixels, rw, rh);
        }
    }
}

fn svg_supersample_factor(w: u32, h: u32) -> u32 {
    let max_dim = w.max(h);
    if max_dim <= 256 {
        3
    } else if max_dim <= 768 {
        2
    } else {
        1
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
    let (mut tw, mut th) = match (target_w, target_h) {
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
    let max_target = tw.max(th);
    if max_target > MAX_DECODE_DIM {
        tw = ((tw as u64 * MAX_DECODE_DIM as u64) / max_target as u64).max(1) as u32;
        th = ((th as u64 * MAX_DECODE_DIM as u64) / max_target as u64).max(1) as u32;
    }

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
    let preferred_icon_size = target_w.or(target_h).unwrap_or(FAVICON_SIZE).max(1);
    let is_ico = data.len() >= 4 && data[0] == 0 && data[1] == 0 && data[2] == 1 && data[3] == 0;
    let info = if is_ico {
        libimage_client::probe_ico_size(data, preferred_icon_size)
            .or_else(|| libimage_client::probe(data))
    } else {
        libimage_client::probe(data)
            .or_else(|| libimage_client::probe_ico_size(data, preferred_icon_size))
    }
    .ok_or(libimage_client::ImageError::InvalidData)?;
    let w = info.width;
    let h = info.height;
    if w == 0 || h == 0 || w > 16384 || h > 16384 || (w as u64 * h as u64) > 67_108_864 {
        return Err(libimage_client::ImageError::InvalidData);
    }
    let mut pixels = vec![0u32; (w * h) as usize];
    let mut scratch = vec![0u8; info.scratch_needed as usize];
    if info.format == libimage_client::FMT_ICO {
        libimage_client::decode_ico_size(data, preferred_icon_size, &mut pixels, &mut scratch)?;
    } else {
        libimage_client::decode(data, &mut pixels, &mut scratch)?;
    }
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

pub(crate) fn decode_svg_to_image(data: &[u8]) -> Result<DecodedRasterImage, ()> {
    let intrinsic = svg_intrinsic_raster_size(data);
    let probed = if intrinsic.is_none() {
        libsvg_client::probe(data)
    } else {
        None
    };
    let (rw, rh) = intrinsic.unwrap_or_else(|| match probed {
        Some((w, h)) => {
            let w = (w as u32).max(1);
            let h = (h as u32).max(1);
            cap_svg_raster_size(w, h)
        }
        None => (256, 256),
    });
    let (rw, rh) = cap_svg_raster_size(rw, rh);

    let pixel_count = (rw as usize).checked_mul(rh as usize).ok_or(())?;
    let background = parse_svg_root_background(data).unwrap_or(0x00000000);
    let supersample = svg_supersample_factor(rw, rh);
    let (ok, pixels) = if supersample > 1 {
        let hi_w = rw.saturating_mul(supersample);
        let hi_h = rh.saturating_mul(supersample);
        let hi_count = (hi_w as usize).checked_mul(hi_h as usize).ok_or(())?;
        let mut hi_pixels = vec![0u32; hi_count];
        let ok = libsvg_client::render_to_size(data, &mut hi_pixels, hi_w, hi_h, background);
        let mut lo_pixels = vec![0u32; pixel_count];
        if ok {
            let scaled = libimage_client::scale_image(
                &hi_pixels,
                hi_w,
                hi_h,
                &mut lo_pixels,
                rw,
                rh,
                libimage_client::MODE_SCALE,
            );
            (scaled, lo_pixels)
        } else {
            (false, lo_pixels)
        }
    } else {
        let mut pixels = vec![0u32; pixel_count];
        let ok = libsvg_client::render_to_size(data, &mut pixels, rw, rh, background);
        (ok, pixels)
    };
    if !ok {
        return Err(());
    }
    let suspicious_black_ppm = decoded_image_looks_suspiciously_black(&pixels);
    Ok(DecodedRasterImage {
        pixels,
        width: rw,
        height: rh,
        format: libimage_client::FMT_UNKNOWN,
        suspicious_black_ppm,
    })
}
