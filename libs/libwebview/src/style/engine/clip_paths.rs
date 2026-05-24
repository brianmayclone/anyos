// Clip-path parsing
// ---------------------------------------------------------------------------

/// Parse `clip-path: circle(...)` or `clip-path: inset(...)`.
/// Parse `clip: rect(top, right, bottom, left)` into [top, right, bottom, left] in px.
/// Also accepts space-separated values (legacy syntax).
fn parse_clip_rect(s: &str, parent_fs: i32, root_fs: i32) -> Option<[i32; 4]> {
    let s = s.trim();
    // Must start with "rect("
    let inner = s.strip_prefix("rect(")?.trim_end_matches(')').trim();
    // Values can be comma- or space-separated.
    let parts: Vec<&str> = if inner.contains(',') {
        inner.split(',').map(|p| p.trim()).collect()
    } else {
        inner.split_whitespace().collect()
    };
    if parts.len() < 4 {
        return None;
    }
    let mut vals = [0i32; 4];
    for (i, p) in parts[..4].iter().enumerate() {
        vals[i] = if *p == "auto" {
            0
        } else {
            let cv = crate::css::parse_value(&crate::css::Property::Top, p);
            resolve_length(&cv, parent_fs, root_fs).unwrap_or(0)
        };
    }
    Some(vals)
}

/// Parse a CSS `content` property value.
///
/// Handles:
/// - Quoted strings: `"text"` or `'text'`
/// - `none` / `normal` → (None, None)
/// - `counter(name)` / `counter(name, style)` → encoded as `\x01COUNTER:name\x01` in text
/// - `counters(name, sep)` → encoded as `\x01COUNTER:name\x01`
/// - `url("...")` → (Some(""), Some(url))
/// - Multi-value: `"(" counter(n) ")"` → concatenated result
/// - Icon/unicode: `"\e900"` → kept as-is (Unicode escape)
///
/// Returns `(text_content, url_content)`.
pub(crate) fn parse_content_value(raw: &str) -> (Option<String>, Option<String>) {
    let s = raw.trim();
    if s.is_empty() {
        return (None, None);
    }

    let lower = s.to_ascii_lowercase();
    if lower == "none" || lower == "normal" || lower == "no-open-quote" || lower == "no-close-quote"
    {
        return (None, None);
    }

    // Pure url(...) without any surrounding text
    if lower.starts_with("url(") && !lower.contains('"') && !lower.contains('\'')
        || lower.starts_with("url(\"")
        || lower.starts_with("url('")
    {
        // Check if the whole value is url(...)
        let trimmed = s.trim_end_matches(')').trim();
        if trimmed.starts_with("url(") || trimmed.to_ascii_lowercase().starts_with("url(") {
            let url = extract_css_url(s);
            return (Some(String::new()), Some(url));
        }
    }

    // Multi-value parser: iterate over tokens
    let mut result = String::new();
    let mut url_found: Option<String> = None;
    let bytes = s.as_bytes();
    let mut pos = 0;

    while pos < bytes.len() {
        // Skip whitespace
        while pos < bytes.len() && (bytes[pos] == b' ' || bytes[pos] == b'\t') {
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }

        if bytes[pos] == b'"' || bytes[pos] == b'\'' {
            // Quoted string: collect content between quotes
            let quote = bytes[pos];
            pos += 1;
            let start = pos;
            while pos < bytes.len() && bytes[pos] != quote {
                pos += 1;
            }
            let text = core::str::from_utf8(&bytes[start..pos]).unwrap_or("");
            // Unescape CSS unicode escapes like \e900
            result.push_str(&unescape_css_string(text));
            if pos < bytes.len() {
                pos += 1;
            } // skip closing quote
        } else if rest_starts_with_ci(bytes, pos, b"counter(") {
            pos += 8;
            let (name, new_pos) = read_counter_name(bytes, pos);
            pos = new_pos;
            result.push('\x01');
            result.push_str("COUNTER:");
            result.push_str(&name);
            result.push('\x01');
        } else if rest_starts_with_ci(bytes, pos, b"counters(") {
            pos += 9;
            let (name, new_pos) = read_counter_name(bytes, pos);
            pos = new_pos;
            result.push('\x01');
            result.push_str("COUNTER:");
            result.push_str(&name);
            result.push('\x01');
        } else if rest_starts_with_ci(bytes, pos, b"url(") {
            // url(...) inside multi-value content
            pos += 4;
            // Skip past closing paren
            let mut depth = 1usize;
            let url_start = pos;
            while pos < bytes.len() && depth > 0 {
                if bytes[pos] == b'(' {
                    depth += 1;
                } else if bytes[pos] == b')' {
                    depth -= 1;
                }
                if depth > 0 {
                    pos += 1;
                }
            }
            let url_raw = core::str::from_utf8(&bytes[url_start..pos]).unwrap_or("");
            let url = url_raw.trim().trim_matches('"').trim_matches('\'');
            url_found = Some(String::from(url));
            if pos < bytes.len() {
                pos += 1;
            }
        } else if rest_starts_with_ci(bytes, pos, b"open-quote") {
            result.push('\u{201C}');
            pos += 10;
        } else if rest_starts_with_ci(bytes, pos, b"close-quote") {
            result.push('\u{201D}');
            pos += 11;
        } else if rest_starts_with_ci(bytes, pos, b"attr(") {
            // attr(name) — skip for now
            pos += 5;
            while pos < bytes.len() && bytes[pos] != b')' {
                pos += 1;
            }
            if pos < bytes.len() {
                pos += 1;
            }
        } else {
            // Unknown token — skip to next whitespace or quote
            while pos < bytes.len()
                && bytes[pos] != b' '
                && bytes[pos] != b'\t'
                && bytes[pos] != b'"'
                && bytes[pos] != b'\''
            {
                pos += 1;
            }
        }
    }

    if result.is_empty() && url_found.is_none() {
        // Nothing useful parsed — treat the raw value as a plain text string
        // (handles icon font chars stored as unquoted keywords)
        let stripped = s.trim_matches('"').trim_matches('\'');
        if stripped == "none" || stripped == "normal" {
            return (None, None);
        }
        if stripped.is_empty() {
            return (Some(String::new()), None);
        }
        return (Some(String::from(stripped)), None);
    }

    let text = if result.is_empty() {
        Some(String::new())
    } else {
        Some(result)
    };
    (text, url_found)
}

/// Unescape CSS string escapes: `\e900` → U+E900, `\n` → newline, etc.
fn unescape_css_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 1;
            // Hex escape: up to 6 hex digits
            if bytes[i].is_ascii_hexdigit() {
                let start = i;
                let mut hex_end = i;
                while hex_end < bytes.len()
                    && hex_end - start < 6
                    && bytes[hex_end].is_ascii_hexdigit()
                {
                    hex_end += 1;
                }
                let hex_str = core::str::from_utf8(&bytes[start..hex_end]).unwrap_or("0");
                if let Ok(code) = u32::from_str_radix(hex_str, 16) {
                    if let Some(c) = char::from_u32(code) {
                        out.push(c);
                    }
                }
                i = hex_end;
                // Skip optional single whitespace after hex escape
                if i < bytes.len()
                    && (bytes[i] == b' '
                        || bytes[i] == b'\n'
                        || bytes[i] == b'\r'
                        || bytes[i] == b'\t')
                {
                    i += 1;
                }
            } else {
                // Simple escape: \n, \t, \", \\, etc.
                let c = match bytes[i] {
                    b'n' => '\n',
                    b't' => '\t',
                    b'r' => '\r',
                    b => b as char,
                };
                out.push(c);
                i += 1;
            }
        } else {
            // Pass through non-escape bytes as UTF-8.
            // Collect a run of non-backslash bytes and decode them.
            let start = i;
            while i < bytes.len() && bytes[i] != b'\\' {
                i += 1;
            }
            if let Ok(s) = core::str::from_utf8(&bytes[start..i]) {
                out.push_str(s);
            } else {
                // Fallback: push individual ASCII chars
                for b in &bytes[start..i] {
                    if *b < 128 {
                        out.push(*b as char);
                    }
                }
            }
        }
    }
    out
}

/// Check if `bytes[pos..]` starts with `prefix` (case-insensitive ASCII).
fn rest_starts_with_ci(bytes: &[u8], pos: usize, prefix: &[u8]) -> bool {
    if pos + prefix.len() > bytes.len() {
        return false;
    }
    for (i, &pb) in prefix.iter().enumerate() {
        let b = bytes[pos + i];
        let bl = if b >= b'A' && b <= b'Z' { b + 32 } else { b };
        let pl = if pb >= b'A' && pb <= b'Z' {
            pb + 32
        } else {
            pb
        };
        if bl != pl {
            return false;
        }
    }
    true
}

/// Read a counter name from bytes starting at `pos` (inside counter(...) after the `(`).
/// Returns (name, new_pos) where new_pos is after the closing `)`.
fn read_counter_name(bytes: &[u8], mut pos: usize) -> (String, usize) {
    // Skip whitespace
    while pos < bytes.len() && (bytes[pos] == b' ' || bytes[pos] == b'\t') {
        pos += 1;
    }
    let start = pos;
    // Read until comma or closing paren
    while pos < bytes.len() && bytes[pos] != b',' && bytes[pos] != b')' {
        pos += 1;
    }
    let name = core::str::from_utf8(&bytes[start..pos])
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    // Skip past closing paren (and anything between comma and paren)
    let mut depth = 1i32;
    while pos < bytes.len() && depth > 0 {
        if bytes[pos] == b'(' {
            depth += 1;
        } else if bytes[pos] == b')' {
            depth -= 1;
        }
        pos += 1;
    }
    (name, pos)
}

/// Extract the URL from `url("...")` or `url(...)`.
fn extract_css_url(s: &str) -> String {
    let s = s.trim();
    let inner = if let Some(rest) = s.strip_prefix("url(") {
        rest.trim_end_matches(')').trim()
    } else if let Some(rest) = s.to_ascii_lowercase().strip_prefix("url(").map(|_| &s[4..]) {
        rest.trim_end_matches(')').trim()
    } else {
        s
    };
    String::from(inner.trim_matches('"').trim_matches('\''))
}

fn parse_clip_path_value(s: &str, parent_fs: i32, root_fs: i32) -> ClipPathVal {
    let s = s.trim();
    if s == "none" {
        return ClipPathVal::None;
    }

    if s.starts_with("circle(") {
        let inner = s.trim_start_matches("circle(").trim_end_matches(')').trim();
        // "50px at 100px 100px" or "50%" or "50px"
        let parts: Vec<&str> = inner.split_whitespace().collect();
        let radius = if !parts.is_empty() {
            resolve_clip_dim(parts[0], parent_fs, root_fs)
        } else {
            50
        };
        let (cx, cy) = if parts.len() >= 4 && parts[1] == "at" {
            (
                resolve_clip_dim(parts[2], parent_fs, root_fs),
                resolve_clip_dim(parts[3], parent_fs, root_fs),
            )
        } else {
            (50, 50)
        }; // default: center (percentage-like)
        return ClipPathVal::Circle { radius, cx, cy };
    }

    if s.starts_with("inset(") {
        let inner = s.trim_start_matches("inset(").trim_end_matches(')').trim();
        // Split on "round" for optional border-radius
        let (dims_str, radius) = if let Some(round_pos) = inner.find("round") {
            let r_str = inner[round_pos + 5..].trim();
            let r = resolve_clip_dim(r_str, parent_fs, root_fs);
            (&inner[..round_pos], r)
        } else {
            (inner, 0)
        };
        let parts: Vec<&str> = dims_str.split_whitespace().collect();
        let (t, r, b, l) = match parts.len() {
            1 => {
                let v = resolve_clip_dim(parts[0], parent_fs, root_fs);
                (v, v, v, v)
            }
            2 => {
                let tb = resolve_clip_dim(parts[0], parent_fs, root_fs);
                let lr = resolve_clip_dim(parts[1], parent_fs, root_fs);
                (tb, lr, tb, lr)
            }
            3 => {
                let t = resolve_clip_dim(parts[0], parent_fs, root_fs);
                let lr = resolve_clip_dim(parts[1], parent_fs, root_fs);
                let b = resolve_clip_dim(parts[2], parent_fs, root_fs);
                (t, lr, b, lr)
            }
            _ => (
                resolve_clip_dim(parts[0], parent_fs, root_fs),
                resolve_clip_dim(parts[1], parent_fs, root_fs),
                resolve_clip_dim(parts[2], parent_fs, root_fs),
                resolve_clip_dim(parts[3], parent_fs, root_fs),
            ),
        };
        return ClipPathVal::Inset {
            top: t,
            right: r,
            bottom: b,
            left: l,
            radius,
        };
    }

    ClipPathVal::None
}

fn resolve_clip_dim(s: &str, parent_fs: i32, root_fs: i32) -> i32 {
    if let Some(dim) = crate::css::try_parse_dimension_pub(s) {
        if let Some(px) = resolve_length(&dim, parent_fs, root_fs) {
            return px;
        }
    }
    0
}

// ---------------------------------------------------------------------------
