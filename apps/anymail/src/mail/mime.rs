// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! MIME parser for multipart email messages (RFC 2045, 2046).
//!
//! Handles:
//! - multipart/mixed, multipart/alternative, multipart/related
//! - Content-Transfer-Encoding: base64, quoted-printable, 7bit, 8bit
//! - Content-Type parameters (charset, boundary, name)
//! - Inline and attachment dispositions

use super::message::{self, Attachment, FullMessage, MessageSummary};
use super::rfc2822::EmailAddress;
use super::{base64, quoted_printable, rfc2047, rfc2822};
use alloc::string::String;
use alloc::vec::Vec;

/// Parsed Content-Type header.
struct ContentType {
    media_type: String, // e.g., "text/plain", "multipart/mixed"
    charset: String,    // e.g., "utf-8"
    boundary: String,   // for multipart
    name: String,       // attachment filename from Content-Type
}

impl ContentType {
    fn new() -> Self {
        Self {
            media_type: String::from("text/plain"),
            charset: String::from("utf-8"),
            boundary: String::new(),
            name: String::new(),
        }
    }

    fn is_multipart(&self) -> bool {
        starts_with_ci(&self.media_type, "multipart/")
    }

    fn is_text(&self) -> bool {
        starts_with_ci(&self.media_type, "text/")
    }

    fn is_html(&self) -> bool {
        eq_ci(&self.media_type, "text/html")
    }

    fn is_plain(&self) -> bool {
        eq_ci(&self.media_type, "text/plain")
    }
}

/// Parsed Content-Disposition header.
struct ContentDisposition {
    disposition: String, // "inline" or "attachment"
    filename: String,
}

impl ContentDisposition {
    fn new() -> Self {
        Self {
            disposition: String::new(),
            filename: String::new(),
        }
    }

    fn is_attachment(&self) -> bool {
        eq_ci(&self.disposition, "attachment")
    }
}

/// A single MIME part (recursive for multipart).
struct MimePart {
    content_type: ContentType,
    transfer_encoding: String,
    disposition: ContentDisposition,
    content_id: String,
    body: Vec<u8>,
    parts: Vec<MimePart>,
}

/// Parse a raw RFC 2822 message into a FullMessage.
pub fn parse_message(raw: &[u8]) -> FullMessage {
    let mut msg = FullMessage::new();
    msg.raw = Vec::from(raw);

    // Split headers and body
    let (headers_bytes, body_bytes) = split_headers_body(raw);
    let headers_str = lossy_utf8(headers_bytes);

    // Parse headers
    let headers = unfold_headers(&headers_str);
    let mut content_type = ContentType::new();
    let mut transfer_encoding = String::from("7bit");

    for (name, value) in &headers {
        let name_lower = to_lower(name);
        match name_lower.as_str() {
            "from" => {
                let decoded = rfc2047::decode_header(value);
                msg.summary.from = rfc2822::parse_address(&decoded);
            }
            "to" => {
                let decoded = rfc2047::decode_header(value);
                msg.summary.to = rfc2822::parse_address_list(&decoded);
            }
            "cc" => {
                let decoded = rfc2047::decode_header(value);
                msg.cc = rfc2822::parse_address_list(&decoded);
            }
            "bcc" => {
                let decoded = rfc2047::decode_header(value);
                msg.bcc = rfc2822::parse_address_list(&decoded);
            }
            "reply-to" => {
                let decoded = rfc2047::decode_header(value);
                msg.reply_to = rfc2822::parse_address_list(&decoded);
            }
            "subject" => {
                msg.summary.subject = rfc2047::decode_header(value);
            }
            "date" => {
                msg.summary.date = rfc2822::parse_date(value);
            }
            "message-id" => {
                msg.summary.message_id = String::from(value.trim());
            }
            "in-reply-to" => {
                msg.summary.in_reply_to = String::from(value.trim());
            }
            "references" => {
                msg.summary.references = String::from(value.trim());
            }
            "content-type" => {
                content_type = parse_content_type(value);
            }
            "content-transfer-encoding" => {
                transfer_encoding = String::from(value.trim());
            }
            _ => {}
        }
    }

    msg.summary.size = raw.len() as u64;

    // Parse body based on content type
    if content_type.is_multipart() {
        let parts = split_multipart(body_bytes, &content_type.boundary);
        for part_bytes in &parts {
            process_part(part_bytes, &mut msg);
        }
    } else {
        // Single-part message
        let decoded_body = decode_body(body_bytes, &transfer_encoding);
        let text = convert_to_utf8(&decoded_body, &content_type.charset);

        if content_type.is_html() {
            msg.html_body = text;
        } else {
            msg.text_body = text;
        }
    }

    // Generate preview from text body
    if !msg.text_body.is_empty() {
        let preview: String = msg.text_body.chars().take(150).collect();
        msg.summary.preview = preview.replace('\n', " ").replace('\r', "");
    } else if !msg.html_body.is_empty() {
        // Strip HTML tags for preview
        msg.summary.preview = strip_html_tags(&msg.html_body, 150);
    }

    // Set attachment flag
    if !msg.attachments.is_empty() {
        msg.summary.flags |= message::FLAG_HAS_ATTACHMENT;
    }

    msg
}

/// Parse just headers from raw data (faster for index building).
pub fn parse_headers_only(raw: &[u8]) -> MessageSummary {
    let mut summary = MessageSummary::new();
    let (headers_bytes, _) = split_headers_body(raw);
    let headers_str = lossy_utf8(headers_bytes);
    let headers = unfold_headers(&headers_str);

    let mut has_attachment = false;

    for (name, value) in &headers {
        let name_lower = to_lower(name);
        match name_lower.as_str() {
            "from" => {
                summary.from = rfc2822::parse_address(&rfc2047::decode_header(value));
            }
            "to" => {
                summary.to = rfc2822::parse_address_list(&rfc2047::decode_header(value));
            }
            "subject" => {
                summary.subject = rfc2047::decode_header(value);
            }
            "date" => {
                summary.date = rfc2822::parse_date(value);
            }
            "message-id" => {
                summary.message_id = String::from(value.trim());
            }
            "in-reply-to" => {
                summary.in_reply_to = String::from(value.trim());
            }
            "references" => {
                summary.references = String::from(value.trim());
            }
            "content-type" => {
                // Check for multipart/mixed (likely has attachments)
                if find_ci(value, "multipart/mixed").is_some() {
                    has_attachment = true;
                }
            }
            _ => {}
        }
    }

    summary.size = raw.len() as u64;
    if has_attachment {
        summary.flags |= message::FLAG_HAS_ATTACHMENT;
    }

    summary
}

// --- Internal parsing helpers ---

fn split_headers_body(data: &[u8]) -> (&[u8], &[u8]) {
    // Find \r\n\r\n or \n\n
    for i in 0..data.len() {
        if i + 3 < data.len()
            && data[i] == b'\r'
            && data[i + 1] == b'\n'
            && data[i + 2] == b'\r'
            && data[i + 3] == b'\n'
        {
            return (&data[..i], &data[i + 4..]);
        }
        if i + 1 < data.len() && data[i] == b'\n' && data[i + 1] == b'\n' {
            return (&data[..i], &data[i + 2..]);
        }
    }
    (data, &[])
}

fn unfold_headers(headers: &str) -> Vec<(String, String)> {
    let mut result: Vec<(String, String)> = Vec::new();
    let mut current_name = String::new();
    let mut current_value = String::new();

    for line in headers.split('\n') {
        let line = line.trim_end_matches('\r');

        if line.is_empty() {
            break;
        }

        // Continuation line (starts with whitespace)
        if line.starts_with(' ') || line.starts_with('\t') {
            if !current_name.is_empty() {
                current_value.push(' ');
                current_value.push_str(line.trim());
            }
            continue;
        }

        // Save previous header
        if !current_name.is_empty() {
            result.push((current_name.clone(), current_value.clone()));
        }

        // Parse new header
        if let Some(colon) = line.find(':') {
            current_name = String::from(&line[..colon]);
            current_value = String::from(line[colon + 1..].trim());
        } else {
            current_name = String::new();
            current_value = String::new();
        }
    }

    // Save last header
    if !current_name.is_empty() {
        result.push((current_name, current_value));
    }

    result
}

fn parse_content_type(value: &str) -> ContentType {
    let mut ct = ContentType::new();

    // Split on ';' for parameters
    let mut parts = value.splitn(2, ';');
    if let Some(media) = parts.next() {
        ct.media_type = String::from(media.trim());
    }

    if let Some(params_str) = parts.next() {
        let params = parse_params(params_str);
        for (k, v) in &params {
            let k_lower = to_lower(k);
            match k_lower.as_str() {
                "charset" => ct.charset = String::from(v.as_str()),
                "boundary" => ct.boundary = String::from(v.as_str()),
                "name" => ct.name = String::from(v.as_str()),
                _ => {}
            }
        }
    }

    ct
}

fn parse_params(params_str: &str) -> Vec<(String, String)> {
    let mut result = Vec::new();

    for param in params_str.split(';') {
        let param = param.trim();
        if let Some(eq) = param.find('=') {
            let key = param[..eq].trim();
            let mut val = param[eq + 1..].trim();
            // Strip quotes
            if val.starts_with('"') && val.ends_with('"') && val.len() >= 2 {
                val = &val[1..val.len() - 1];
            }
            result.push((String::from(key), String::from(val)));
        }
    }

    result
}

fn split_multipart<'a>(body: &'a [u8], boundary: &str) -> Vec<&'a [u8]> {
    let mut parts = Vec::new();
    let delim = alloc::format!("--{}", boundary);
    let end_delim = alloc::format!("--{}--", boundary);
    let body_str = lossy_utf8(body);

    let mut in_part = false;
    let mut part_start = 0;

    let lines: Vec<&str> = body_str.split('\n').collect();
    let mut offset = 0;

    for line in &lines {
        let trimmed = line.trim_end_matches('\r');
        let line_len = line.len() + 1; // +1 for \n

        if trimmed == end_delim.as_str() {
            if in_part && part_start < offset {
                // Remove trailing \r\n before delimiter
                let mut end = offset;
                if end > 0 && body[end - 1] == b'\n' {
                    end -= 1;
                }
                if end > 0 && body[end - 1] == b'\r' {
                    end -= 1;
                }
                if part_start < end {
                    parts.push(&body[part_start..end]);
                }
            }
            break;
        }

        if trimmed == delim.as_str() {
            if in_part && part_start < offset {
                let mut end = offset;
                if end > 0 && body[end - 1] == b'\n' {
                    end -= 1;
                }
                if end > 0 && body[end - 1] == b'\r' {
                    end -= 1;
                }
                if part_start < end {
                    parts.push(&body[part_start..end]);
                }
            }
            in_part = true;
            part_start = offset + line_len;
        }

        offset += line_len;
    }

    parts
}

fn process_part(part_data: &[u8], msg: &mut FullMessage) {
    let (headers_bytes, body_bytes) = split_headers_body(part_data);
    let headers_str = lossy_utf8(headers_bytes);
    let headers = unfold_headers(&headers_str);

    let mut content_type = ContentType::new();
    let mut transfer_encoding = String::from("7bit");
    let mut disposition = ContentDisposition::new();
    let mut content_id = String::new();

    for (name, value) in &headers {
        let name_lower = to_lower(name);
        match name_lower.as_str() {
            "content-type" => {
                content_type = parse_content_type(value);
            }
            "content-transfer-encoding" => {
                transfer_encoding = String::from(value.trim());
            }
            "content-disposition" => {
                disposition = parse_content_disposition(value);
            }
            "content-id" => {
                let mut cid = String::from(value.trim());
                // Strip angle brackets
                if cid.starts_with('<') && cid.ends_with('>') {
                    cid = String::from(&cid[1..cid.len() - 1]);
                }
                content_id = cid;
            }
            _ => {}
        }
    }

    // Recurse into nested multipart
    if content_type.is_multipart() {
        let sub_parts = split_multipart(body_bytes, &content_type.boundary);
        for sub in &sub_parts {
            process_part(sub, msg);
        }
        return;
    }

    // Decode body
    let decoded = decode_body(body_bytes, &transfer_encoding);

    // Handle based on type/disposition
    if disposition.is_attachment() || !content_type.name.is_empty() {
        let filename = if !disposition.filename.is_empty() {
            disposition.filename.clone()
        } else if !content_type.name.is_empty() {
            content_type.name.clone()
        } else {
            String::from("attachment")
        };

        msg.attachments.push(Attachment {
            filename: rfc2047::decode_header(&filename),
            content_type: content_type.media_type.clone(),
            data: decoded.clone(),
            size: decoded.len() as u64,
            content_id,
        });
    } else if content_type.is_html() {
        if msg.html_body.is_empty() {
            msg.html_body = convert_to_utf8(&decoded, &content_type.charset);
        }
    } else if content_type.is_plain() {
        if msg.text_body.is_empty() {
            msg.text_body = convert_to_utf8(&decoded, &content_type.charset);
        }
    } else if content_type.is_text() {
        // Other text/* types - treat as plain text
        if msg.text_body.is_empty() {
            msg.text_body = convert_to_utf8(&decoded, &content_type.charset);
        }
    } else {
        // Binary/other content type → attachment
        let filename = if !disposition.filename.is_empty() {
            disposition.filename.clone()
        } else {
            let ext = guess_extension(&content_type.media_type);
            alloc::format!("attachment{}", ext)
        };
        msg.attachments.push(Attachment {
            filename,
            content_type: content_type.media_type.clone(),
            data: decoded.clone(),
            size: decoded.len() as u64,
            content_id,
        });
    }
}

fn parse_content_disposition(value: &str) -> ContentDisposition {
    let mut disp = ContentDisposition::new();
    let mut parts = value.splitn(2, ';');
    if let Some(d) = parts.next() {
        disp.disposition = String::from(d.trim());
    }
    if let Some(params_str) = parts.next() {
        let params = parse_params(params_str);
        for (k, v) in &params {
            if eq_ci(k, "filename") {
                disp.filename = String::from(v.as_str());
            }
        }
    }
    disp
}

fn decode_body(body: &[u8], encoding: &str) -> Vec<u8> {
    let enc = to_lower(encoding);
    match enc.as_str() {
        "base64" => base64::decode(body),
        "quoted-printable" => quoted_printable::decode(body),
        "7bit" | "8bit" | "binary" | "" => Vec::from(body),
        _ => Vec::from(body),
    }
}

fn convert_to_utf8(data: &[u8], charset: &str) -> String {
    let cs = to_upper(charset);
    match cs.as_str() {
        "UTF-8" | "UTF8" | "" => core::str::from_utf8(data).unwrap_or("").into(),
        "ISO-8859-1" | "LATIN1" | "LATIN-1" | "ISO_8859-1" | "ISO-8859-15" => {
            let mut s = String::with_capacity(data.len() * 2);
            for &b in data {
                s.push(b as char);
            }
            s
        }
        "WINDOWS-1252" | "CP1252" => {
            // Simplified: treat like latin1 for most chars
            let mut s = String::with_capacity(data.len() * 2);
            for &b in data {
                s.push(b as char);
            }
            s
        }
        "US-ASCII" | "ASCII" => {
            let mut s = String::new();
            for &b in data {
                s.push(if b < 0x80 { b as char } else { '?' });
            }
            s
        }
        _ => {
            // Try UTF-8, fallback to latin1
            match core::str::from_utf8(data) {
                Ok(s) => String::from(s),
                Err(_) => {
                    let mut s = String::with_capacity(data.len() * 2);
                    for &b in data {
                        s.push(b as char);
                    }
                    s
                }
            }
        }
    }
}

fn guess_extension(media_type: &str) -> &'static str {
    let mt = to_lower(media_type);
    if mt.contains("jpeg") || mt.contains("jpg") {
        return ".jpg";
    }
    if mt.contains("png") {
        return ".png";
    }
    if mt.contains("gif") {
        return ".gif";
    }
    if mt.contains("pdf") {
        return ".pdf";
    }
    if mt.contains("zip") {
        return ".zip";
    }
    if mt.contains("html") {
        return ".html";
    }
    if mt.contains("xml") {
        return ".xml";
    }
    if mt.contains("octet-stream") {
        return ".bin";
    }
    ""
}

fn strip_html_tags(html: &str, max_len: usize) -> String {
    let mut result = String::new();
    let mut in_tag = false;

    for ch in html.chars() {
        if result.len() >= max_len {
            break;
        }
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => {
                if ch == '\n' || ch == '\r' || ch == '\t' {
                    if !result.ends_with(' ') {
                        result.push(' ');
                    }
                } else {
                    result.push(ch);
                }
            }
            _ => {}
        }
    }

    result
}

fn lossy_utf8(data: &[u8]) -> String {
    match core::str::from_utf8(data) {
        Ok(s) => String::from(s),
        Err(_) => {
            // Fallback: Latin-1 (ISO-8859-1) conversion
            // This properly maps bytes 0x00-0xFF to Unicode code points
            let mut s = String::with_capacity(data.len());
            for &b in data {
                s.push(b as char); // In Rust, byte b casts to Unicode code point U+00xx
            }
            s
        }
    }
}

// Case-insensitive string helpers
fn to_lower(s: &str) -> String {
    let mut r = String::with_capacity(s.len());
    for c in s.chars() {
        if c >= 'A' && c <= 'Z' {
            r.push((c as u8 + 32) as char);
        } else {
            r.push(c);
        }
    }
    r
}

fn to_upper(s: &str) -> String {
    let mut r = String::with_capacity(s.len());
    for c in s.chars() {
        if c >= 'a' && c <= 'z' {
            r.push((c as u8 - 32) as char);
        } else {
            r.push(c);
        }
    }
    r
}

fn starts_with_ci(s: &str, prefix: &str) -> bool {
    if s.len() < prefix.len() {
        return false;
    }
    let a = to_lower(&s[..prefix.len()]);
    let b = to_lower(prefix);
    a == b
}

fn eq_ci(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    to_lower(a) == to_lower(b)
}

fn find_ci(haystack: &str, needle: &str) -> Option<usize> {
    let h = to_lower(haystack);
    let n = to_lower(needle);
    h.find(&n)
}
