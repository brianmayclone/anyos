// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Minimal XML tokenizer for SVG subset.
//!
//! Emits a flat stream of [`Token`]s from UTF-8 XML bytes.  Only the subset
//! needed for SVG 1.1 static content is implemented — no DTD, no processing
//! instructions beyond ignoring them, no namespace resolution.

use alloc::string::String;
use alloc::vec::Vec;

// ── Token types ──────────────────────────────────────────────────────

/// A single XML attribute key/value pair.
#[derive(Debug, Clone)]
pub struct Attr {
    pub key: String,
    pub value: String,
}

/// Tokenizer output.
#[derive(Debug)]
pub enum Token {
    /// Opening tag, e.g. `<circle cx="5">` — `self_closing` is false.
    Open {
        tag: String,
        attrs: Vec<Attr>,
        self_closing: bool,
    },
    /// Closing tag, e.g. `</g>`.
    Close { tag: String },
    /// Character data between tags (trimmed, may be empty).
    Text(String),
}

// ── Tokenizer ────────────────────────────────────────────────────────

/// Tokenize `xml_bytes` into a `Vec<Token>`.
///
/// Skips comments (`<!-- -->`), `<!DOCTYPE>`, and `<?...?>` processing
/// instructions. Attribute entity references `&amp;`, `&lt;`, `&gt;`,
/// `&quot;`, `&apos;` and numeric references `&#nn;` / `&#xhh;` are decoded.
pub fn tokenize(xml_bytes: &[u8]) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut pos = 0;
    let len = xml_bytes.len();

    while pos < len {
        if xml_bytes[pos] == b'<' {
            // Determine what kind of tag this is
            let peek = pos + 1;
            if peek >= len {
                break;
            }

            if xml_bytes[peek..].starts_with(b"!--") {
                // Comment — skip to -->
                pos = find_after(xml_bytes, pos + 4, b"-->").unwrap_or(len);
                continue;
            }

            if xml_bytes[peek] == b'?' {
                // Processing instruction — skip to ?>
                pos = find_after(xml_bytes, pos + 2, b"?>").unwrap_or(len);
                continue;
            }

            if xml_bytes[peek..].starts_with(b"![CDATA[") {
                // CDATA — include raw text up to ]]>
                let start = pos + 9;
                let end = find_after(xml_bytes, start, b"]]>").unwrap_or(len);
                let text = String::from_utf8_lossy(&xml_bytes[start..end.saturating_sub(3)])
                    .into_owned();
                if !text.is_empty() {
                    tokens.push(Token::Text(text));
                }
                pos = end;
                continue;
            }

            if xml_bytes[peek] == b'!' {
                // DOCTYPE or similar declaration — skip to >
                pos = find_char(xml_bytes, pos + 2, b'>').unwrap_or(len) + 1;
                continue;
            }

            if xml_bytes[peek] == b'/' {
                // Closing tag </tag>
                let tag_start = pos + 2;
                let tag_end = find_char(xml_bytes, tag_start, b'>').unwrap_or(len);
                let tag = trim_str(&xml_bytes[tag_start..tag_end]).to_ascii_lowercase();
                tokens.push(Token::Close { tag: String::from(tag) });
                pos = tag_end + 1;
                continue;
            }

            // Opening tag (possibly self-closing)
            let tag_end = find_tag_end(xml_bytes, pos + 1);
            let inner = &xml_bytes[pos + 1..tag_end];
            let self_closing = inner.last().copied() == Some(b'/');
            let inner = if self_closing { &inner[..inner.len() - 1] } else { inner };

            let (tag, attrs) = parse_tag_inner(inner);
            tokens.push(Token::Open { tag, attrs, self_closing });
            pos = tag_end + 1;
        } else {
            // Character data
            let end = find_char(xml_bytes, pos, b'<').unwrap_or(len);
            let text = decode_entities(trim_str(&xml_bytes[pos..end]));
            if !text.is_empty() {
                tokens.push(Token::Text(text));
            }
            pos = end;
        }
    }

    tokens
}

// ── Tag parsing helpers ──────────────────────────────────────────────

/// Find the closing `>` of a tag, respecting quoted attribute values.
fn find_tag_end(data: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < data.len() {
        match data[i] {
            b'>' => return i,
            b'"' => {
                i += 1;
                while i < data.len() && data[i] != b'"' { i += 1; }
            }
            b'\'' => {
                i += 1;
                while i < data.len() && data[i] != b'\'' { i += 1; }
            }
            _ => {}
        }
        i += 1;
    }
    data.len()
}

/// Parse `tagname attr1="val1" attr2='val2' ...` from a raw inner tag slice.
fn parse_tag_inner(inner: &[u8]) -> (String, Vec<Attr>) {
    let inner = trim_bytes(inner);
    // Split tag name from attributes
    let name_end = inner.iter().position(|&b| matches!(b, b' '|b'\t'|b'\n'|b'\r'))
        .unwrap_or(inner.len());
    let tag = core::str::from_utf8(&inner[..name_end])
        .unwrap_or("")
        .to_ascii_lowercase();
    let rest = trim_bytes(&inner[name_end..]);
    let attrs = parse_attrs(rest);
    (String::from(tag.as_str()), attrs)
}

/// Parse `key="value" key2='value2' ...` into a `Vec<Attr>`.
fn parse_attrs(data: &[u8]) -> Vec<Attr> {
    let mut attrs = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        // skip whitespace
        while pos < data.len() && matches!(data[pos], b' '|b'\t'|b'\n'|b'\r') {
            pos += 1;
        }
        if pos >= data.len() { break; }

        // attribute name
        let key_start = pos;
        while pos < data.len() && !matches!(data[pos], b'='|b' '|b'\t'|b'\n'|b'\r'|b'>') {
            pos += 1;
        }
        let key = core::str::from_utf8(&data[key_start..pos])
            .unwrap_or("")
            .to_ascii_lowercase();
        if key.is_empty() { break; }

        // skip whitespace around '='
        while pos < data.len() && matches!(data[pos], b' '|b'\t'|b'\n'|b'\r') {
            pos += 1;
        }
        if pos >= data.len() || data[pos] != b'=' {
            // boolean attribute — treat value as key name
            attrs.push(Attr { key: String::from(key.as_str()), value: String::from(key.as_str()) });
            continue;
        }
        pos += 1; // skip '='

        // skip whitespace
        while pos < data.len() && matches!(data[pos], b' '|b'\t'|b'\n'|b'\r') {
            pos += 1;
        }
        if pos >= data.len() { break; }

        let value = if data[pos] == b'"' || data[pos] == b'\'' {
            let quote = data[pos];
            pos += 1;
            let val_start = pos;
            while pos < data.len() && data[pos] != quote { pos += 1; }
            let val = decode_entities(
                core::str::from_utf8(&data[val_start..pos]).unwrap_or("")
            );
            pos += 1; // skip closing quote
            val
        } else {
            let val_start = pos;
            while pos < data.len() && !matches!(data[pos], b' '|b'\t'|b'\n'|b'\r'|b'>') {
                pos += 1;
            }
            String::from(core::str::from_utf8(&data[val_start..pos]).unwrap_or(""))
        };

        if !key.is_empty() {
            attrs.push(Attr {
                key: String::from(key.as_str()),
                value,
            });
        }
    }
    attrs
}

// ── Entity decoding ──────────────────────────────────────────────────

fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return String::from(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        if let Some(semi) = rest.find(';') {
            let entity = &rest[1..semi];
            let replacement = match entity {
                "amp"  => "&",
                "lt"   => "<",
                "gt"   => ">",
                "quot" => "\"",
                "apos" => "'",
                "nbsp" => "\u{00A0}",
                _ if entity.starts_with('#') => {
                    if let Some(hex) = entity.strip_prefix("#x").or_else(|| entity.strip_prefix("#X")) {
                        if let Ok(n) = u32::from_str_radix(hex, 16) {
                            if let Some(c) = char::from_u32(n) {
                                out.push(c);
                                rest = &rest[semi + 1..];
                                continue;
                            }
                        }
                    } else if let Some(dec) = entity.strip_prefix('#') {
                        if let Ok(n) = dec.parse::<u32>() {
                            if let Some(c) = char::from_u32(n) {
                                out.push(c);
                                rest = &rest[semi + 1..];
                                continue;
                            }
                        }
                    }
                    "&"
                }
                _ => "&",
            };
            out.push_str(replacement);
            rest = &rest[semi + 1..];
        } else {
            out.push('&');
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    out
}

// ── Byte-level helpers ───────────────────────────────────────────────

fn find_char(data: &[u8], from: usize, ch: u8) -> Option<usize> {
    data[from..].iter().position(|&b| b == ch).map(|i| from + i)
}

fn find_after(data: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if from + needle.len() > data.len() { return None; }
    for i in from..=(data.len() - needle.len()) {
        if &data[i..i + needle.len()] == needle {
            return Some(i + needle.len());
        }
    }
    None
}

fn trim_bytes(b: &[u8]) -> &[u8] {
    let b = match b.iter().position(|&x| !matches!(x, b' '|b'\t'|b'\n'|b'\r')) {
        Some(i) => &b[i..],
        None    => return &[],
    };
    match b.iter().rposition(|&x| !matches!(x, b' '|b'\t'|b'\n'|b'\r')) {
        Some(i) => &b[..=i],
        None    => b,
    }
}

fn trim_str(b: &[u8]) -> &str {
    core::str::from_utf8(trim_bytes(b)).unwrap_or("")
}
