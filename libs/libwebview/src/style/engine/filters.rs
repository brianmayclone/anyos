// Filter parsing (litehtml-inspired)
// ---------------------------------------------------------------------------

/// Parse a CSS `filter` value like `blur(5px) grayscale(50%) brightness(120%)`.
fn parse_filter_value(s: &str, parent_fs: i32, root_fs: i32) -> FilterVal {
    let mut f = FilterVal::none();
    let s = s.trim();
    if s == "none" {
        return f;
    }

    // Tokenize function calls like "blur(5px)" "grayscale(50%)"
    let mut pos = 0;
    let bytes = s.as_bytes();
    while pos < bytes.len() {
        // Skip whitespace
        while pos < bytes.len() && (bytes[pos] == b' ' || bytes[pos] == b'\t') {
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }

        // Read function name
        let name_start = pos;
        while pos < bytes.len() && bytes[pos] != b'(' && bytes[pos] != b' ' {
            pos += 1;
        }
        let name = &s[name_start..pos];
        if pos >= bytes.len() || bytes[pos] != b'(' {
            break;
        }
        pos += 1; // skip '('

        // Read argument until ')'
        let arg_start = pos;
        while pos < bytes.len() && bytes[pos] != b')' {
            pos += 1;
        }
        let arg = &s[arg_start..pos];
        if pos < bytes.len() {
            pos += 1;
        } // skip ')'

        let arg = arg.trim();
        match name {
            "blur" => {
                if let Some(dim) = crate::css::try_parse_dimension_pub(arg) {
                    if let Some(px) = resolve_length(&dim, parent_fs, root_fs) {
                        f.blur_px = px.max(0);
                    }
                }
            }
            "brightness" => {
                f.brightness = parse_filter_pct(arg);
            }
            "contrast" => {
                f.contrast = parse_filter_pct(arg);
            }
            "grayscale" => {
                f.grayscale = parse_filter_pct(arg);
            }
            "saturate" => {
                f.saturate = parse_filter_pct(arg);
            }
            "sepia" => {
                f.sepia = parse_filter_pct(arg);
            }
            "opacity" => {
                f.opacity = parse_filter_pct(arg);
            }
            "invert" => {
                f.invert = parse_filter_pct(arg);
            }
            "hue-rotate" => {
                let deg_str = arg.trim_end_matches("deg").trim();
                if let Ok(v) = deg_str.parse::<i32>() {
                    f.hue_rotate = v;
                }
            }
            _ => {} // drop-shadow, url() — not supported
        }
    }
    f
}

/// Parse a filter function argument as percentage (100% = 10000).
fn parse_filter_pct(s: &str) -> i32 {
    let s = s.trim();
    if s.ends_with('%') {
        let num = &s[..s.len() - 1];
        if let Ok(v) = num.parse::<i32>() {
            return v * 100;
        }
    }
    // Try as decimal (0.5 = 5000, 1.0 = 10000)
    if let Some(dim) = crate::css::try_parse_dimension_pub(s) {
        if let CssValue::Number(v) = dim {
            return v * 100; // v is already *100
        }
    }
    10000
}

/// Parse a simple float/int string to fixed-point * 100 (returns Option).
fn try_parse_simple_float(s: &str) -> Option<i32> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(dim) = crate::css::try_parse_dimension_pub(s) {
        match dim {
            CssValue::Number(v) => return Some(v),
            CssValue::Length(v, _) => return Some(v),
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------------------
