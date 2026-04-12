pub fn parse_value(property: &Property, value_str: &str) -> CssValue {
    let s = value_str.trim();
    if s.is_empty() {
        return CssValue::None;
    }

    // Check common keywords first
    let lower = to_ascii_lower(s);
    match lower.as_str() {
        "auto" => return CssValue::Auto,
        "none" => return CssValue::None,
        "inherit" => return CssValue::Inherit,
        "transparent" => return CssValue::Color(0x00000000),
        _ => {}
    }

    // var() — CSS custom property reference.
    if lower.starts_with("var(") {
        return parse_var_value(s);
    }

    // calc() — CSS math expression.
    if lower.starts_with("calc(") {
        return parse_calc_value(s);
    }

    // min(), max(), clamp() — CSS comparison functions.
    if lower.starts_with("min(") {
        return parse_min_max_clamp_value(s, CssMathFunc::Min);
    }
    if lower.starts_with("max(") {
        return parse_min_max_clamp_value(s, CssMathFunc::Max);
    }
    if lower.starts_with("clamp(") {
        return parse_min_max_clamp_value(s, CssMathFunc::Clamp);
    }

    // currentColor keyword — resolves to the element's computed `color` property.
    if lower == "currentcolor" {
        return CssValue::CurrentColor;
    }

    // Color properties — try color parsing
    if is_color_property(property) {
        if let Some(c) = try_parse_color(s) {
            return CssValue::Color(c);
        }
    }

    // Try color regardless of property if it starts with # or rgb
    if s.starts_with('#') || lower.starts_with("rgb") {
        if let Some(c) = try_parse_color(s) {
            return CssValue::Color(c);
        }
    }

    // Try named colors for color properties
    if is_color_property(property) {
        if let Some(c) = named_color(&lower) {
            return CssValue::Color(c);
        }
    }

    // Try length/percentage/number
    if let Some(v) = try_parse_dimension(s) {
        return v;
    }

    // Fall back to keyword.
    // Grid placement properties use <custom-ident> which is case-sensitive per spec (§7.3).
    // Preserve original case for these properties.
    let is_case_sensitive = matches!(
        property,
        Property::GridColumn
            | Property::GridColumnStart
            | Property::GridColumnEnd
            | Property::GridRow
            | Property::GridRowStart
            | Property::GridRowEnd
            | Property::GridArea
            | Property::GridTemplateAreas
            | Property::FontFamily
            | Property::Content
    );
    if is_case_sensitive {
        CssValue::Keyword(String::from(s))
    } else {
        CssValue::Keyword(lower)
    }
}

/// Parse `var(--name)` or `var(--name, fallback)`.
fn parse_var_value(s: &str) -> CssValue {
    // Strip "var(" and trailing ")".
    let inner = s.trim();
    let inner = if inner.starts_with("var(") || inner.starts_with("VAR(") {
        &inner[4..]
    } else {
        inner
    };
    let inner = inner.trim_end_matches(')').trim();

    // Split on first comma for fallback.
    if let Some(comma) = inner.find(',') {
        let name = inner[..comma].trim();
        let fallback_str = inner[comma + 1..].trim();
        let fallback = if fallback_str.is_empty() {
            None
        } else {
            Some(Box::new(parse_value(&Property::Color, fallback_str)))
        };
        CssValue::Var(String::from(name), fallback)
    } else {
        CssValue::Var(String::from(inner), None)
    }
}

/// Parse `calc(expr)` into a CssValue.
/// Evaluates pure-px expressions to Length, pure-% to Percentage, mixed to Calc.
fn parse_calc_value(s: &str) -> CssValue {
    let s = s.trim();
    // Strip outer "calc(" and matching ")" — find the matching closing paren.
    let lower = s.to_ascii_lowercase();
    let inner = if lower.starts_with("calc(") {
        // Find the matching closing paren (last ')' in simple expressions).
        let without_prefix = &s[5..]; // after "calc("
                                      // Strip trailing ')'.
        let inner = without_prefix.trim_end_matches(')').trim();
        inner
    } else {
        s
    };

    // Use the more precise 2-component evaluator: (px*100, pct*100).
    let (px, pct) = eval_calc_components(inner);

    if pct == 0 {
        CssValue::Length(px, Unit::Px)
    } else if px == 0 {
        CssValue::Percentage(pct)
    } else {
        CssValue::Calc(px, pct)
    }
}

/// Evaluate a calc() expression into two components: (px * 100, pct * 100).
/// Supports: +, -, *, /, nested parens, rem/em/px/% units.
fn eval_calc_components(s: &str) -> (i32, i32) {
    let s = s.trim();

    // Find the last + or - operator at depth 0 (left-to-right lowest precedence).
    // Scan right-to-left so we handle e.g. "a - b - c" as "(a-b)-c".
    let bytes = s.as_bytes();
    let mut depth: i32 = 0;
    let mut split_i: Option<usize> = None;
    let mut split_op: u8 = 0;
    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        match bytes[i] {
            b')' => depth += 1,
            b'(' => depth -= 1,
            b'+' | b'-' if depth == 0 && i > 0 => {
                let prev = bytes[i - 1];
                if prev == b' ' || prev.is_ascii_digit() || prev == b')' || prev == b'%' {
                    split_i = Some(i);
                    split_op = bytes[i];
                    break;
                }
            }
            _ => {}
        }
    }
    if let Some(pos) = split_i {
        let (lpx, lpct) = eval_calc_components(&s[..pos]);
        let (rpx, rpct) = eval_calc_components(&s[pos + 1..]);
        return if split_op == b'+' {
            (lpx + rpx, lpct + rpct)
        } else {
            (lpx - rpx, lpct - rpct)
        };
    }

    // Find * or / at top level (scan left for last occurrence for left-assoc).
    depth = 0;
    split_i = None;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b'*' | b'/' if depth == 0 => {
                split_i = Some(i);
                split_op = b;
            }
            _ => {}
        }
    }
    if let Some(pos) = split_i {
        let (lpx, lpct) = eval_calc_components(&s[..pos]);
        let (rpx, rpct) = eval_calc_components(&s[pos + 1..]);
        if split_op == b'*' {
            // One side is always a pure number (no %).
            if lpct == 0 && rpct == 0 {
                // Both px-like, treat as: (lpx/100) * (rpx/100) * 100 = lpx*rpx/100
                return (lpx * rpx / 100, 0);
            } else if lpct == 0 {
                // Right has pct, left is multiplier
                let mul = lpx; // *100 fixed point
                return (0, lpct * mul / 100);
            } else {
                let mul = rpx;
                return (lpx * mul / 100, lpct * mul / 100);
            }
        } else {
            // Division: right must be a plain number.
            let div = rpx; // *100 fixed point
            if div != 0 {
                return (lpx * 100 / div, lpct * 100 / div);
            } else {
                return (0, 0);
            }
        }
    }

    // Atom.
    parse_calc_operand(s)
}

/// Split a calc expression on the main binary operator (respects parentheses).
/// Handles `100% - 32px`, `50% + 10px`, `16px * 2`.
fn split_calc_expr(s: &str) -> Option<(&str, u8, &str)> {
    let bytes = s.as_bytes();
    let mut depth: usize = 0;
    // Look for ` + ` or ` - ` first (addition/subtraction have lower precedence).
    // Scan right-to-left for left-associativity.
    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        match bytes[i] {
            b')' => depth += 1,
            b'(' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            b'+' | b'-' if depth == 0 && i > 0 => {
                let prev = bytes[i - 1];
                if prev == b' ' || prev.is_ascii_digit() || prev == b')' {
                    let left = s[..i].trim_end();
                    let right = s[i + 1..].trim_start();
                    return Some((left, bytes[i], right));
                }
            }
            _ => {}
        }
    }
    // Look for * or / at top level.
    depth = 0;
    let mut last_mul_div: Option<(usize, u8)> = None;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            b'*' | b'/' if depth == 0 => {
                last_mul_div = Some((i, b));
            }
            _ => {}
        }
    }
    if let Some((i, op)) = last_mul_div {
        return Some((&s[..i], op, &s[i + 1..]));
    }
    None
}

/// Parse a single calc operand into (px * 100, pct * 100).
fn parse_calc_operand(s: &str) -> (i32, i32) {
    let s = s.trim();
    let lower = s.to_ascii_lowercase();
    let lower = lower.trim();

    // Nested calc() or parenthesized expression.
    if lower.starts_with("calc(") || (lower.starts_with('(') && lower.ends_with(')')) {
        let inner = if lower.starts_with("calc(") {
            &lower[5..lower.len() - 1]
        } else {
            &lower[1..lower.len() - 1]
        };
        return eval_calc_components(inner);
    }
    // min()/max()/clamp() as operand — evaluate to px approximation.
    if lower.starts_with("min(") || lower.starts_with("max(") || lower.starts_with("clamp(") {
        let func = if lower.starts_with("clamp(") {
            CssMathFunc::Clamp
        } else if lower.starts_with("min(") {
            CssMathFunc::Min
        } else {
            CssMathFunc::Max
        };
        match eval_min_max_clamp(lower, func) {
            CssValue::Length(v, Unit::Px) => return (v * 100, 0),
            CssValue::Percentage(p) => return (0, p),
            CssValue::Calc(px, pct) => return (px, pct),
            _ => {}
        }
    }

    if lower.ends_with('%') {
        let num = &lower[..lower.len() - 1];
        let val = parse_fixed_100(num);
        (0, val)
    } else if lower.ends_with("px") {
        let num = &lower[..lower.len() - 2];
        let val = parse_fixed_100(num);
        (val, 0)
    } else if lower.ends_with("rem") {
        let num = &lower[..lower.len() - 3];
        let val = parse_fixed_100(num);
        (val * 16, 0)
    } else if lower.ends_with("em") {
        let num = &lower[..lower.len() - 2];
        let val = parse_fixed_100(num);
        // Treat em as px * 16 (approximate).
        (val * 16, 0)
    } else if lower.ends_with("vw")
        || lower.ends_with("vh")
        || lower.ends_with("vmin")
        || lower.ends_with("vmax")
    {
        // Viewport units in calc — treated as percentage-like (resolved at layout time).
        let suffix_len = if lower.ends_with("vmin") || lower.ends_with("vmax") {
            4
        } else {
            2
        };
        let num = &lower[..lower.len() - suffix_len];
        let val = parse_fixed_100(num);
        (0, val)
    } else {
        // Pure number.
        let val = parse_fixed_100(s);
        (val, 0)
    }
}

enum CssMathFunc {
    Min,
    Max,
    Clamp,
}

/// Parse and evaluate min(), max(), clamp() CSS functions.
fn parse_min_max_clamp_value(s: &str, func: CssMathFunc) -> CssValue {
    let lower = s.to_ascii_lowercase();
    eval_min_max_clamp(&lower, func)
}

/// Split top-level comma-separated arguments (respecting parentheses).
fn split_top_level_commas(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let bytes = s.as_bytes();
    let mut depth: usize = 0;
    let mut start = 0;
    for i in 0..bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            b',' if depth == 0 => {
                parts.push(s[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(s[start..].trim());
    parts
}

/// Evaluate a min/max/clamp function expression. Expects lowercase input.
fn eval_min_max_clamp(s: &str, func: CssMathFunc) -> CssValue {
    // Find opening paren.
    let paren_start = match s.find('(') {
        Some(i) => i,
        None => return CssValue::None,
    };
    let inner = s[paren_start + 1..].trim_end_matches(')').trim();
    let args: Vec<&str> = split_top_level_commas(inner);

    // Evaluate each arg as a calc-like expression.
    let vals: Vec<(i32, i32)> = args.iter().map(|a| eval_calc_components(a)).collect();

    match func {
        CssMathFunc::Min => {
            // If all pure-px, return the minimum.
            if vals.iter().all(|(_, pct)| *pct == 0) {
                let min_px = vals.iter().map(|(px, _)| *px).min().unwrap_or(0);
                return CssValue::Length(min_px / 100, Unit::Px);
            }
            // Mixed: return first arg as approximation.
            let (px, pct) = vals.first().copied().unwrap_or((0, 0));
            if pct == 0 {
                CssValue::Length(px / 100, Unit::Px)
            } else if px == 0 {
                CssValue::Percentage(pct)
            } else {
                CssValue::Calc(px, pct)
            }
        }
        CssMathFunc::Max => {
            if vals.iter().all(|(_, pct)| *pct == 0) {
                let max_px = vals.iter().map(|(px, _)| *px).max().unwrap_or(0);
                return CssValue::Length(max_px / 100, Unit::Px);
            }
            let (px, pct) = vals.last().copied().unwrap_or((0, 0));
            if pct == 0 {
                CssValue::Length(px / 100, Unit::Px)
            } else if px == 0 {
                CssValue::Percentage(pct)
            } else {
                CssValue::Calc(px, pct)
            }
        }
        CssMathFunc::Clamp => {
            // clamp(min, val, max) — use val if all are pure-px, then clamp.
            if vals.len() >= 3 {
                let (min_px, min_pct) = vals[0];
                let (val_px, val_pct) = vals[1];
                let (max_px, max_pct) = vals[2];
                // If all pure-px, fully resolve.
                if min_pct == 0 && val_pct == 0 && max_pct == 0 {
                    let v = val_px.max(min_px).min(max_px);
                    return CssValue::Length(v / 100, Unit::Px);
                }
                // Otherwise return the middle (val) as best approximation.
                if val_pct == 0 {
                    CssValue::Length(val_px / 100, Unit::Px)
                } else if val_px == 0 {
                    CssValue::Percentage(val_pct)
                } else {
                    CssValue::Calc(val_px, val_pct)
                }
            } else {
                // Malformed — return first arg.
                let (px, pct) = vals.first().copied().unwrap_or((0, 0));
                if pct == 0 {
                    CssValue::Length(px / 100, Unit::Px)
                } else {
                    CssValue::Percentage(pct)
                }
            }
        }
    }
}

/// Parse a number string into fixed-point * 100.
fn parse_fixed_100(s: &str) -> i32 {
    let s = s.trim();
    let neg = s.starts_with('-');
    let s = if neg { &s[1..] } else { s };
    let mut int_part: i32 = 0;
    let mut frac_part: i32 = 0;
    let mut in_frac = false;
    let mut frac_digits = 0;
    for b in s.as_bytes() {
        if *b == b'.' {
            in_frac = true;
            continue;
        }
        if *b >= b'0' && *b <= b'9' {
            if in_frac {
                if frac_digits < 2 {
                    frac_part = frac_part * 10 + (*b - b'0') as i32;
                    frac_digits += 1;
                }
            } else {
                int_part = int_part * 10 + (*b - b'0') as i32;
            }
        } else {
            break;
        }
    }
    // Pad fraction to 2 digits.
    while frac_digits < 2 {
        frac_part *= 10;
        frac_digits += 1;
    }
    let val = int_part * 100 + frac_part;
    if neg {
        -val
    } else {
        val
    }
}

fn is_color_property(p: &Property) -> bool {
    matches!(
        p,
        Property::Color
            | Property::BackgroundColor
            | Property::Background
            | Property::BorderColor
            | Property::BorderTopColor
            | Property::BorderRightColor
            | Property::BorderBottomColor
            | Property::BorderLeftColor
            | Property::OutlineColor
            | Property::TextDecorationColor
            | Property::AccentColor
    )
}
