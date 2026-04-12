fn parse_media_rule(
    p: &mut Parser,
    current_layer: Option<&str>,
    layer_order: &mut Vec<String>,
    anon_layer_counter: &mut u32,
) -> Option<MediaRule> {
    p.skip_whitespace();

    // Read everything until '{' as the media query text.
    let query_start = p.pos;
    while !p.eof() && p.peek() != b'{' {
        p.pos += 1;
    }
    let query_text = core::str::from_utf8(&p.input[query_start..p.pos]).unwrap_or("");
    let query = parse_media_query(query_text);

    if p.eof() {
        return None;
    }
    p.pos += 1; // consume '{'

    // Parse inner rules until matching '}'.
    let mut inner_rules = Vec::new();
    let mut layer_stack: Vec<String> = Vec::new();
    if let Some(layer) = current_layer {
        layer_stack.push(String::from(layer));
    }
    let base_layer_depth = layer_stack.len();
    loop {
        p.skip_whitespace();
        if p.eof() {
            break;
        }
        if p.peek() == b'}' {
            p.pos += 1;
            if layer_stack.len() > base_layer_depth {
                layer_stack.pop();
                continue;
            }
            break;
        }
        // Handle nested at-rules inside @media.
        if p.peek() == b'@' {
            p.pos += 1;
            let kw = p.read_ident();
            let kw_lower = {
                let mut buf = [0u8; 32];
                let len = kw.len().min(32);
                for (i, &b) in kw.as_bytes()[..len].iter().enumerate() {
                    buf[i] = if b >= b'A' && b <= b'Z' { b + 32 } else { b };
                }
                String::from(core::str::from_utf8(&buf[..len]).unwrap_or(""))
            };
            // Handle @supports nested inside @media.
            if kw_lower == "supports" {
                if let Some(sr) = parse_supports_rule(
                    p,
                    layer_stack.last().map(|s| s.as_str()),
                    layer_order,
                    anon_layer_counter,
                ) {
                    for rule in sr.rules {
                        inner_rules.push(rule);
                    }
                }
            } else if kw_lower == "container" {
                let container_query = parse_container_query_prelude(p);
                if p.peek() == b'{' {
                    p.pos += 1;
                    let mut nested_media_rules = Vec::new();
                    parse_container_block(
                        p,
                        layer_stack.last().map(|s| s.as_str()),
                        layer_order,
                        anon_layer_counter,
                        container_query,
                        &mut inner_rules,
                        &mut nested_media_rules,
                    );
                    for nested in nested_media_rules {
                        inner_rules.extend(nested.rules);
                    }
                }
            } else if kw_lower == "layer" {
                p.skip_whitespace();
                let name_start = p.pos;
                while !p.eof() && p.peek() != b'{' && p.peek() != b';' {
                    p.pos += 1;
                }
                let name_text = core::str::from_utf8(&p.input[name_start..p.pos])
                    .unwrap_or("")
                    .trim();
                if p.peek() == b';' {
                    register_layer_statement(
                        name_text,
                        layer_stack.last().map(|s| s.as_str()),
                        layer_order,
                    );
                    p.pos += 1;
                } else if p.peek() == b'{' {
                    let full_name = resolve_layer_block_name(
                        name_text,
                        layer_stack.last().map(|s| s.as_str()),
                        layer_order,
                        anon_layer_counter,
                    );
                    p.pos += 1;
                    layer_stack.push(full_name);
                }
            } else {
                // Skip other nested at-rules.
                loop {
                    p.skip_whitespace();
                    if p.eof() {
                        break;
                    }
                    if p.peek() == b'{' {
                        p.skip_block();
                        break;
                    }
                    if p.peek() == b';' {
                        p.pos += 1;
                        break;
                    }
                    p.pos += 1;
                }
            }
            continue;
        }
        if let Some(rule) = parse_rule(p, layer_stack.last().map(|s| s.as_str())) {
            inner_rules.push(rule);
        }
    }

    Some(MediaRule {
        query,
        rules: inner_rules,
    })
}

/// Parse a media query string like `screen and (max-width: 768px)`.
fn parse_media_query(text: &str) -> MediaQuery {
    let mut conditions = Vec::new();
    let trimmed = text.trim();
    let mut media_type = MediaType::All;
    let mut negated = false;

    // Split on "and" (case-insensitive).
    for part in split_and(trimmed) {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }

        let lower = p.to_ascii_lowercase();

        // Track `not` modifier.
        if lower == "not" {
            negated = true;
            continue;
        }

        // Skip `only` modifier (has no effect on matching).
        if lower == "only" {
            continue;
        }

        // Recognize media types.
        if lower == "screen" {
            let mt = MediaType::Screen;
            media_type = if negated {
                MediaType::Not(Box::new(mt))
            } else {
                mt
            };
            negated = false;
            continue;
        }
        if lower == "print" {
            let mt = MediaType::Print;
            media_type = if negated {
                MediaType::Not(Box::new(mt))
            } else {
                mt
            };
            negated = false;
            continue;
        }
        if lower == "all" {
            let mt = MediaType::All;
            media_type = if negated {
                MediaType::Not(Box::new(mt))
            } else {
                mt
            };
            negated = false;
            continue;
        }

        // Parenthesized condition: (min-width: 768px)
        if p.starts_with('(') && p.ends_with(')') {
            let inner = &p[1..p.len() - 1];
            if let Some(cond) = parse_media_condition(inner) {
                conditions.push(cond);
            }
        }
    }

    MediaQuery {
        conditions,
        media_type,
    }
}

/// Split a media query string on " and " (case-insensitive).
fn split_and(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let bytes = s.as_bytes();
    let mut start = 0;

    for i in 0..bytes.len() {
        // Check for " and " (with spaces).
        if i + 5 <= bytes.len() {
            let chunk = &bytes[i..i + 5];
            if (chunk[0] == b' ')
                && (chunk[1] | 32 == b'a')
                && (chunk[2] | 32 == b'n')
                && (chunk[3] | 32 == b'd')
                && (chunk[4] == b' ')
            {
                parts.push(&s[start..i]);
                start = i + 5;
            }
        }
    }
    parts.push(&s[start..]);
    parts
}

/// Parse a single media condition like `max-width: 768px`.
/// Returns `Some(Unsupported)` for unknown features (so they evaluate to false).
fn parse_media_condition(inner: &str) -> Option<MediaCondition> {
    let inner = inner.trim();

    // Range syntax: `width>=64rem`, `width<=1024px`, `width>40rem`
    if let Some(idx) = inner.find(">=") {
        let feature = inner[..idx].trim().to_ascii_lowercase();
        let val = inner[idx + 2..].trim();
        if feature == "width" {
            if let Some(px) = parse_px_value(val) {
                return Some(MediaCondition::MinWidth(px));
            }
        }
    }
    if let Some(idx) = inner.find("<=") {
        let feature = inner[..idx].trim().to_ascii_lowercase();
        let val = inner[idx + 2..].trim();
        if feature == "width" {
            if let Some(px) = parse_px_value(val) {
                return Some(MediaCondition::MaxWidth(px));
            }
        }
    }

    // Boolean media feature with no value: (color), (hover), etc.
    if !inner.contains(':') {
        let feature = inner.to_ascii_lowercase();
        return match feature.as_str() {
            "color" | "color-index" => Some(MediaCondition::Known(true)),
            "monochrome" => Some(MediaCondition::Known(false)),
            "hover" => Some(MediaCondition::Known(true)), // assume hover capable
            _ => Some(MediaCondition::Unsupported),
        };
    }

    let colon = inner.find(':')?;
    let name = inner[..colon].trim().to_ascii_lowercase();
    let value_str = inner[colon + 1..].trim();

    match name.as_str() {
        "min-width" => {
            let px = parse_px_value(value_str)?;
            Some(MediaCondition::MinWidth(px))
        }
        "max-width" => {
            let px = parse_px_value(value_str)?;
            Some(MediaCondition::MaxWidth(px))
        }
        "min-height" => {
            let px = parse_px_value(value_str)?;
            Some(MediaCondition::MinHeight(px))
        }
        "max-height" => {
            let px = parse_px_value(value_str)?;
            Some(MediaCondition::MaxHeight(px))
        }
        "prefers-color-scheme" => Some(MediaCondition::PrefersColorScheme(String::from(
            value_str.trim(),
        ))),
        // Interaction media features — we're a desktop browser with mouse.
        "hover" => Some(MediaCondition::Known(value_str == "hover")),
        "any-hover" => Some(MediaCondition::Known(value_str == "hover")),
        "pointer" => Some(MediaCondition::Known(value_str == "fine")),
        "any-pointer" => Some(MediaCondition::Known(value_str == "fine")),
        // Motion preferences — we don't animate, so treat as no-preference.
        "prefers-reduced-motion" => Some(MediaCondition::Known(value_str == "no-preference")),
        // Contrast preferences — we render standard contrast.
        "prefers-contrast" => Some(MediaCondition::Known(value_str == "no-preference")),
        // Data/update preferences.
        "prefers-reduced-data" | "prefers-reduced-transparency" => {
            Some(MediaCondition::Known(value_str == "no-preference"))
        }
        // Color gamut — we support sRGB.
        "color-gamut" => Some(MediaCondition::Known(value_str == "srgb")),
        // Resolution — assume standard 96dpi.
        "resolution" | "min-resolution" | "max-resolution" => {
            // Accept all — high-DPI media queries don't affect layout.
            Some(MediaCondition::Known(true))
        }
        // orientation — we're always landscape for wide viewports.
        "orientation" => {
            // True for landscape; false for portrait.
            Some(MediaCondition::Known(value_str == "landscape"))
        }
        // Dynamic viewport — unknown, skip.
        "dynamic-viewport-height" | "environment" => Some(MediaCondition::Unsupported),
        // Anything else unknown — treat as false.
        _ => Some(MediaCondition::Unsupported),
    }
}

/// Parse a CSS pixel value like "768px", "48rem", or "calc(640px - 1px)" into i32.
fn parse_px_value(s: &str) -> Option<i32> {
    let s = s.trim();

    // Handle calc() expressions — evaluate simple arithmetic at parse time.
    if s.to_ascii_lowercase().starts_with("calc(") {
        return eval_media_calc(s);
    }

    // Rem/em units — multiply by 16 (default root font size).
    if s.ends_with("rem") {
        let n = &s[..s.len() - 3];
        return parse_float_px(n).map(|v| (v * 16.0) as i32);
    }
    if s.ends_with("em") {
        let n = &s[..s.len() - 2];
        return parse_float_px(n).map(|v| (v * 16.0) as i32);
    }

    // Strip "px" and parse integer.
    let s = s.trim_end_matches("px").trim();
    let mut val: i32 = 0;
    for b in s.as_bytes() {
        if *b >= b'0' && *b <= b'9' {
            val = val * 10 + (*b - b'0') as i32;
        } else if *b == b'.' {
            break; // ignore fractional part
        } else {
            break;
        }
    }
    if val > 0 || s == "0" {
        Some(val)
    } else {
        None
    }
}

/// Parse a floating-point number string (no unit) into f32.
fn parse_float_px(s: &str) -> Option<f32> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut result: f32 = 0.0;
    let mut frac: f32 = 0.0;
    let mut in_frac = false;
    let mut frac_div: f32 = 10.0;
    let mut has_digit = false;
    for b in s.as_bytes() {
        match b {
            b'0'..=b'9' => {
                has_digit = true;
                if in_frac {
                    frac += (*b - b'0') as f32 / frac_div;
                    frac_div *= 10.0;
                } else {
                    result = result * 10.0 + (*b - b'0') as f32;
                }
            }
            b'.' if !in_frac => {
                in_frac = true;
            }
            _ => break,
        }
    }
    if has_digit {
        Some(result + frac)
    } else {
        None
    }
}

/// Evaluate a simple calc() expression for @media conditions.
/// Only handles px/rem/em values with +, -, *, / operators.
/// Examples: "calc(640px - 1px)" → 639, "calc(48rem)" → 768
fn eval_media_calc(s: &str) -> Option<i32> {
    let lower = s.to_ascii_lowercase();
    let inner = lower.strip_prefix("calc(")?;
    // Strip trailing ')' — handle nested parens by finding the matching one.
    let inner = strip_outer_paren(inner)?;
    eval_calc_expr_px(inner)
}

/// Strip one layer of trailing ')' from a string that may have nested parens.
fn strip_outer_paren(s: &str) -> Option<&str> {
    // Just strip the last ')' — for simple media calc expressions this is enough.
    let s = s.trim();
    if s.ends_with(')') {
        Some(&s[..s.len() - 1])
    } else {
        Some(s)
    }
}

/// Evaluate a calc expression string to pixels (f32).
/// Supports px, rem, em units and +, -, *, / operators.
fn eval_calc_expr_px(s: &str) -> Option<i32> {
    let val = eval_calc_f32(s.trim())?;
    Some((val + 0.5) as i32)
}

/// Recursively evaluate a calc arithmetic expression, returning value in px as f32.
fn eval_calc_f32(s: &str) -> Option<f32> {
    let s = s.trim();

    // Find the last + or - operator (lowest precedence) respecting parentheses.
    // We scan right-to-left to get left-associativity.
    let bytes = s.as_bytes();
    let mut depth: i32 = 0;
    let mut split_pos: Option<usize> = None;
    let mut split_op: u8 = 0;
    // Scan right-to-left to handle: a - b - c = (a-b)-c correctly.
    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        match bytes[i] {
            b')' => depth += 1,
            b'(' => depth -= 1,
            b'+' | b'-' if depth == 0 && i > 0 => {
                // Must be binary op, not unary (preceded by space or digit).
                let prev = bytes[i - 1];
                if prev == b' ' || prev.is_ascii_digit() || prev == b')' {
                    split_pos = Some(i);
                    split_op = bytes[i];
                    break;
                }
            }
            _ => {}
        }
    }

    if let Some(pos) = split_pos {
        let left = eval_calc_f32(&s[..pos])?;
        let right = eval_calc_f32(&s[pos + 1..])?;
        return Some(if split_op == b'+' {
            left + right
        } else {
            left - right
        });
    }

    // Find * or / at top level.
    depth = 0;
    split_pos = None;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b'*' | b'/' if depth == 0 => {
                split_pos = Some(i);
                split_op = b;
                // Don't break — find last one for left-associativity.
            }
            _ => {}
        }
    }

    if let Some(pos) = split_pos {
        let left = eval_calc_f32(&s[..pos])?;
        let right = eval_calc_f32(&s[pos + 1..])?;
        return if split_op == b'*' {
            Some(left * right)
        } else if right != 0.0 {
            Some(left / right)
        } else {
            None
        };
    }

    // Atom — a number with optional unit.
    let s_lower = s.to_ascii_lowercase();
    let s_lower = s_lower.trim();

    // Nested calc()
    if s_lower.starts_with("calc(") {
        let inner = s_lower.strip_prefix("calc(")?;
        let inner = strip_outer_paren(inner)?;
        return eval_calc_f32(inner);
    }

    // Parenthesized expression
    if s_lower.starts_with('(') && s_lower.ends_with(')') {
        return eval_calc_f32(&s_lower[1..s_lower.len() - 1]);
    }

    if s_lower.ends_with("px") {
        return parse_float_px(&s_lower[..s_lower.len() - 2]);
    }
    if s_lower.ends_with("rem") {
        return parse_float_px(&s_lower[..s_lower.len() - 3]).map(|v| v * 16.0);
    }
    if s_lower.ends_with("em") {
        return parse_float_px(&s_lower[..s_lower.len() - 2]).map(|v| v * 16.0);
    }
    if s_lower.ends_with("vw") || s_lower.ends_with("vh") {
        // For media calc, treat vw/vh as 0 (viewport not known at parse time).
        return Some(0.0);
    }
    // Plain number (treat as px).
    let neg = s_lower.starts_with('-');
    let s2 = if neg { &s_lower[1..] } else { s_lower };
    parse_float_px(s2).map(|v| if neg { -v } else { v })
}

/// Evaluate a media query against viewport dimensions.
/// We always render as `screen` media type.
pub fn evaluate_media_query(query: &MediaQuery, viewport_width: i32, viewport_height: i32) -> bool {
    // Check media type first.  We are always "screen".
    match &query.media_type {
        MediaType::All => {}    // matches everything
        MediaType::Screen => {} // we ARE screen
        MediaType::Print => {
            return false;
        } // we are NOT print
        MediaType::Not(inner) => match inner.as_ref() {
            MediaType::Print => {} // not print → we match (we're screen)
            MediaType::Screen => {
                return false;
            } // not screen → we don't match
            MediaType::All => {
                return false;
            } // not all → matches nothing
            MediaType::Not(_) => {} // double negation → treat as all
        },
    }

    for cond in &query.conditions {
        let ok = match cond {
            MediaCondition::MinWidth(w) => viewport_width >= *w,
            MediaCondition::MaxWidth(w) => viewport_width <= *w,
            MediaCondition::MinHeight(h) => viewport_height >= *h,
            MediaCondition::MaxHeight(h) => viewport_height <= *h,
            MediaCondition::PrefersColorScheme(scheme) => {
                // Report light theme — most sites default to dark-on-light text.
                scheme == "light"
            }
            MediaCondition::Known(v) => *v,
            MediaCondition::Unsupported => false,
        };
        if !ok {
            return false;
        }
    }
    true
}

/// Parse a `@keyframes name { stop { … } … }` block.
/// Parse `@supports (condition) { rules }`.
/// Evaluates whether the condition references supported properties.
/// Returns the inner rules if the condition is met, None otherwise.
/// Result of parsing a @supports block: plain rules + nested @media rules.
struct SupportsResult {
    rules: Vec<Rule>,
    media_rules: Vec<MediaRule>,
}

