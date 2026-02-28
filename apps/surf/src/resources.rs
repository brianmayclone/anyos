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
use alloc::vec::Vec;
use alloc::vec;

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
    let charset = detect_charset_from_headers(headers)
        .or_else(|| detect_charset_from_html_bytes(body));

    match charset.as_deref() {
        Some("iso-8859-1")
        | Some("latin1")
        | Some("latin-1")
        | Some("windows-1252")
        | None => latin1_to_utf8(body),
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
) {
    let generation = crate::net_worker::current_generation();
    let mut count = 0u32;

    for (i, node) in dom.nodes.iter().enumerate() {
        if let libwebview::dom::NodeType::Element {
            tag: libwebview::dom::Tag::Link,
            ..
        } = &node.node_type
        {
            let rel = dom.attr(i, "rel").unwrap_or("");
            if !rel.eq_ignore_ascii_case("stylesheet") {
                continue;
            }
            if let Some(href) = dom.attr(i, "href") {
                if !href.is_empty() {
                    let css_url = crate::http::resolve_url(base_url, href);
                    crate::net_worker::submit(crate::net_worker::FetchRequest::Css {
                        tab_index,
                        href: String::from(href),
                        url: css_url,
                        generation,
                    });
                    count += 1;
                }
            }
        }
    }

    if count > 0 {
        anyos_std::println!("[surf] submitted {} stylesheet(s) to worker", count);
    }
}

// ═══════════════════════════════════════════════════════════
// Image discovery — submits to network worker
// ═══════════════════════════════════════════════════════════

/// Scan the DOM for `<img src="…">` tags and submit them to the background
/// network worker for async fetching.
pub(crate) fn queue_images(
    dom: &libwebview::dom::Dom,
    base_url: &crate::http::Url,
    tab_index: usize,
) {
    let generation = crate::net_worker::current_generation();
    let mut count = 0u32;

    for (i, node) in dom.nodes.iter().enumerate() {
        if let libwebview::dom::NodeType::Element {
            tag: libwebview::dom::Tag::Img,
            ..
        } = &node.node_type
        {
            if let Some(src) = dom.attr(i, "src") {
                if src.is_empty() || src.starts_with("data:") {
                    continue;
                }
                let img_url = crate::http::resolve_url(base_url, src);
                crate::net_worker::submit(crate::net_worker::FetchRequest::Image {
                    tab_index,
                    src: String::from(src),
                    url: img_url,
                    generation,
                });
                count += 1;
            }
        }
    }

    if count > 0 {
        anyos_std::println!("[surf] submitted {} image(s) to worker", count);
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
    let st = crate::state();
    if tab_idx < st.tabs.len() {
        st.tabs[tab_idx].webview.relayout();
    }
}

/// Decode a raster image without triggering a relayout.
///
/// Used by the batch result processor which does a single relayout at the end.
pub(crate) fn decode_raster_no_relayout(data: &[u8], src: &str, tab_idx: usize) {
    let info = match libimage_client::probe(data) {
        Some(i) => i,
        None => return,
    };
    let w = info.width;
    let h = info.height;
    if w == 0 || h == 0 || w > 4096 || h > 4096 {
        return;
    }
    let mut pixels = vec![0u32; (w * h) as usize];
    let mut scratch = vec![0u8; info.scratch_needed as usize];
    if libimage_client::decode(data, &mut pixels, &mut scratch).is_ok() {
        let st = crate::state();
        if tab_idx < st.tabs.len() {
            st.tabs[tab_idx].webview.add_image(src, pixels, w, h);
        }
    }
}

/// Rasterise an SVG document and add the result to the image cache of
/// `tab_idx`, then relayout.
pub(crate) fn decode_svg(data: &[u8], src: &str, tab_idx: usize) {
    decode_svg_no_relayout(data, src, tab_idx);
    let st = crate::state();
    if tab_idx < st.tabs.len() {
        st.tabs[tab_idx].webview.relayout();
    }
}

/// Rasterise an SVG document without triggering a relayout.
///
/// Used by the batch result processor which does a single relayout at the end.
pub(crate) fn decode_svg_no_relayout(data: &[u8], src: &str, tab_idx: usize) {
    let (rw, rh) = match libsvg_client::probe(data) {
        Some((w, h)) => {
            let w = (w as u32).max(1).min(4096);
            let h = (h as u32).max(1).min(4096);
            (w, h)
        }
        None => (256, 256),
    };

    let mut pixels = vec![0u32; (rw * rh) as usize];
    if libsvg_client::render_to_size(data, &mut pixels, rw, rh, 0x00000000) {
        let st = crate::state();
        if tab_idx < st.tabs.len() {
            st.tabs[tab_idx].webview.add_image(src, pixels, rw, rh);
        }
    }
}
