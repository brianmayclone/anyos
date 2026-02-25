// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Async resource loading for the Surf browser.
//!
//! Covers:
//! - HTTP response body decoding (charset detection, Latin-1 → UTF-8)
//! - External CSS stylesheet fetching
//! - Asynchronous image fetching via a recurring timer

use alloc::string::String;
use alloc::vec::Vec;
use alloc::vec;
use libanyui_client as ui;

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
// Asynchronous CSS stylesheet loading
// ═══════════════════════════════════════════════════════════

/// Scan the DOM for `<link rel="stylesheet">` tags and enqueue them for async
/// fetching.
///
/// The page is rendered immediately with only the built-in user-agent CSS;
/// each external stylesheet is applied and the layout refreshed as it arrives,
/// giving a progressive-rendering effect without blocking the UI thread.
pub(crate) fn queue_stylesheets(
    dom: &libwebview::dom::Dom,
    base_url: &crate::http::Url,
    tab_index: usize,
) {
    let st = crate::state();
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
                    st.css_queue.push((tab_index, String::from(href), css_url));
                }
            }
        }
    }

    if !st.css_queue.is_empty() {
        anyos_std::println!(
            "[surf] queued {} stylesheet(s) for async loading",
            st.css_queue.len()
        );
        start_css_timer();
    }
}

/// Start the CSS fetch timer if it is not already running.
///
/// The timer fires every 10 ms so that one stylesheet is fetched per tick
/// without starving the UI event loop.
pub(crate) fn start_css_timer() {
    let st = crate::state();
    if st.css_timer != 0 {
        return;
    }
    st.css_timer = ui::set_timer(10, || {
        fetch_next_css();
    });
}

/// Fetch the next stylesheet from the queue.  Called by the recurring timer.
///
/// Applies the loaded CSS to the owning tab's webview and triggers a relayout
/// so the page progressively improves its appearance as stylesheets arrive.
/// Stops the timer when the queue is empty.
pub(crate) fn fetch_next_css() {
    let st = crate::state();

    if st.css_queue.is_empty() {
        if st.css_timer != 0 {
            ui::kill_timer(st.css_timer);
            st.css_timer = 0;
        }
        // Show "Done" only when the image queue is also fully drained.
        if st.image_queue.is_empty() {
            st.tabs[st.active_tab].status_text = String::from("Done");
            crate::ui::update_status();
        }
        return;
    }

    let (tab_idx, href, css_url) = st.css_queue.remove(0);

    // Show progress in the status bar.
    let remaining = st.css_queue.len();
    let mut status = String::from("Loading CSS (");
    crate::ui::push_u32(&mut status, remaining as u32 + 1);
    status.push_str(" left)");
    st.status_label.set_text(&status);

    anyos_std::println!("[surf]   fetching CSS: {}", href);
    match crate::http::fetch(&css_url, &mut st.cookies, &mut st.conn_pool) {
        Ok(resp) if resp.status >= 200 && resp.status < 400 && !resp.body.is_empty() => {
            let css_text = decode_http_body(&resp.body, &resp.headers);
            anyos_std::println!("[surf]   loaded CSS: {} ({} bytes)", href, css_text.len());
            if tab_idx < st.tabs.len() {
                st.tabs[tab_idx].webview.add_stylesheet(&css_text);
                st.tabs[tab_idx].webview.relayout();
            }
        }
        _ => {
            anyos_std::println!("[surf]   CSS fetch failed: {}", href);
        }
    }
}

// ═══════════════════════════════════════════════════════════
// Asynchronous image loading
// ═══════════════════════════════════════════════════════════

/// Scan the DOM for `<img src="…">` tags and enqueue them for async fetching.
///
/// Starts the image fetch timer if any images were found.
pub(crate) fn queue_images(
    dom: &libwebview::dom::Dom,
    base_url: &crate::http::Url,
    tab_index: usize,
) {
    let st = crate::state();
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
                st.image_queue.push((tab_index, String::from(src), img_url));
            }
        }
    }

    if !st.image_queue.is_empty() {
        anyos_std::println!("[surf] queued {} images for async loading", st.image_queue.len());
        start_image_timer();
    }
}

/// Start the image fetch timer if it is not already running.
pub(crate) fn start_image_timer() {
    let st = crate::state();
    if st.image_timer != 0 {
        return;
    }
    st.image_timer = ui::set_timer(10, || {
        fetch_next_image();
    });
}

/// Fetch the next image from the queue.  Called by the recurring timer.
///
/// Stops the timer when the queue is empty.
pub(crate) fn fetch_next_image() {
    let st = crate::state();

    if st.image_queue.is_empty() {
        if st.image_timer != 0 {
            ui::kill_timer(st.image_timer);
            st.image_timer = 0;
        }
        // Show "Done" only when the CSS queue is also fully drained.
        if st.css_queue.is_empty() {
            st.tabs[st.active_tab].status_text = String::from("Done");
            crate::ui::update_status();
        }
        return;
    }

    let (tab_idx, src, img_url) = st.image_queue.remove(0);

    // Show progress in the status bar.
    let remaining = st.image_queue.len();
    let mut status = String::from("Loading image (");
    crate::ui::push_u32(&mut status, remaining as u32 + 1);
    status.push_str(" left): ");
    status.push_str(&crate::ui::format_url(&img_url));
    st.status_label.set_text(&status);

    match crate::http::fetch(&img_url, &mut st.cookies, &mut st.conn_pool) {
        Ok(resp) => {
            if is_svg(&src, &resp.headers) {
                // SVG — rasterise with libsvg.
                decode_svg(&resp.body, &src, tab_idx);
            } else if let Some(info) = libimage_client::probe(&resp.body) {
                // Raster image (PNG, JPEG, BMP, …).
                let w = info.width as usize;
                let h = info.height as usize;
                let mut pixels = vec![0u32; w * h];
                let mut scratch = vec![0u8; info.scratch_needed as usize];
                if libimage_client::decode(&resp.body, &mut pixels, &mut scratch).is_ok() {
                    if tab_idx < st.tabs.len() {
                        st.tabs[tab_idx]
                            .webview
                            .add_image(&src, pixels, info.width, info.height);
                        st.tabs[tab_idx].webview.relayout();
                    }
                }
            }
        }
        Err(_) => {}
    }
}

/// Returns `true` when the fetched resource is an SVG document, detected
/// either by the URL extension or the `Content-Type` response header.
fn is_svg(src: &str, headers: &str) -> bool {
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

/// Rasterise an SVG document and add the result to the image cache of
/// `tab_idx`.  The render dimensions are taken from the SVG's own
/// `width`/`height` declarations (clamped to 4096), or fall back to 256×256.
fn decode_svg(data: &[u8], src: &str, tab_idx: usize) {
    // Probe declared dimensions; fall back to a sensible default.
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
            st.tabs[tab_idx].webview.relayout();
        }
    }
}
