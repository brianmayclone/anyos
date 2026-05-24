// Border helpers
// ---------------------------------------------------------------------------

fn resolve_border_width(val: &CssValue, parent_fs: i32, root_fs: i32, out: &mut i32) {
    if let Some(px) = resolve_length(val, parent_fs, root_fs) {
        *out = px;
    }
    if let CssValue::Keyword(ref kw) = *val {
        *out = match kw.as_str() {
            "thin" => 1,
            "medium" => 3,
            "thick" => 5,
            _ => *out,
        };
    }
}

fn resolve_border_style_val(val: &CssValue) -> BorderStyleVal {
    if matches!(*val, CssValue::None) {
        return BorderStyleVal::None;
    }
    if let CssValue::Keyword(ref kw) = *val {
        match kw.as_str() {
            "solid" => BorderStyleVal::Solid,
            "dashed" => BorderStyleVal::Dashed,
            "dotted" => BorderStyleVal::Dotted,
            "double" => BorderStyleVal::Double,
            "groove" => BorderStyleVal::Groove,
            "ridge" => BorderStyleVal::Ridge,
            "inset" => BorderStyleVal::Inset,
            "outset" => BorderStyleVal::Outset,
            "hidden" => BorderStyleVal::Hidden,
            "none" => BorderStyleVal::None,
            _ => BorderStyleVal::None,
        }
    } else {
        BorderStyleVal::None
    }
}

// ---------------------------------------------------------------------------
// Shadow parsing (litehtml-inspired)
// ---------------------------------------------------------------------------

/// Parse `box-shadow` value: `offset-x offset-y [blur [spread]] color [inset], ...`
fn parse_box_shadows(s: &str, parent_fs: i32, root_fs: i32) -> Vec<BoxShadowVal> {
    let mut shadows = Vec::new();
    for layer in split_comma_respecting_parens(s) {
        let layer = layer.trim();
        if layer.is_empty() || layer == "none" {
            continue;
        }
        let mut inset = false;
        let mut lengths: Vec<i32> = Vec::new();
        let mut color: u32 = 0xFF000000;
        let mut unresolved_var = false;
        // Tokenize respecting parentheses (for rgb()/rgba())
        let tokens = tokenize_respecting_parens(layer);
        for tok in &tokens {
            let lower = tok.to_ascii_lowercase();
            if lower == "inset" {
                inset = true;
            } else if lower.contains("var(") {
                unresolved_var = true;
            } else if let Some(c) = crate::css::try_parse_color_pub(tok) {
                color = c;
            } else if let Some(c) = crate::css::named_color_pub(&lower) {
                color = c;
            } else if let Some(dim) = crate::css::try_parse_dimension_pub(tok) {
                if let Some(px) = resolve_length(&dim, parent_fs, root_fs) {
                    lengths.push(px);
                }
            }
        }
        if unresolved_var {
            continue;
        }
        if lengths.len() >= 2 {
            shadows.push(BoxShadowVal {
                offset_x: lengths[0],
                offset_y: lengths[1],
                blur: if lengths.len() >= 3 { lengths[2] } else { 0 },
                spread: if lengths.len() >= 4 { lengths[3] } else { 0 },
                color,
                inset,
            });
        }
    }
    shadows
}

/// Parse `text-shadow` value: `offset-x offset-y [blur] color, ...`
fn parse_text_shadows(s: &str, parent_fs: i32, root_fs: i32) -> Vec<TextShadowVal> {
    let mut shadows = Vec::new();
    for layer in split_comma_respecting_parens(s) {
        let layer = layer.trim();
        if layer.is_empty() || layer == "none" {
            continue;
        }
        let mut lengths: Vec<i32> = Vec::new();
        let mut color: u32 = 0xFF000000;
        let mut unresolved_var = false;
        let tokens = tokenize_respecting_parens(layer);
        for tok in &tokens {
            let lower = tok.to_ascii_lowercase();
            if lower.contains("var(") {
                unresolved_var = true;
            } else if let Some(c) = crate::css::try_parse_color_pub(tok) {
                color = c;
            } else if let Some(c) = crate::css::named_color_pub(&lower) {
                color = c;
            } else if let Some(dim) = crate::css::try_parse_dimension_pub(tok) {
                if let Some(px) = resolve_length(&dim, parent_fs, root_fs) {
                    lengths.push(px);
                }
            }
        }
        if unresolved_var {
            continue;
        }
        if lengths.len() >= 2 {
            shadows.push(TextShadowVal {
                offset_x: lengths[0],
                offset_y: lengths[1],
                blur: if lengths.len() >= 3 { lengths[2] } else { 0 },
                color,
            });
        }
    }
    shadows
}

/// Tokenize a CSS value string, keeping parenthesized groups (like `rgb(...)`) as one token.
fn tokenize_respecting_parens(s: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let bytes = s.as_bytes();
    let mut start = 0;
    let mut depth: u32 = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            b' ' | b'\t' if depth == 0 => {
                if start < i {
                    tokens.push(&s[start..i]);
                }
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if start < bytes.len() {
        tokens.push(&s[start..]);
    }
    tokens
}

// ---------------------------------------------------------------------------
