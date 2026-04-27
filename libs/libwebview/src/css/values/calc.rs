fn parse_calc_value(s: &str) -> CssValue {
    let s = s.trim();
    let lower = s.to_ascii_lowercase();
    let inner = if lower.starts_with("calc(") {
        let without_prefix = &s[5..];
        without_prefix
            .strip_suffix(')')
            .unwrap_or(without_prefix)
            .trim()
    } else {
        s
    };

    let (px, pct) = eval_calc_components(inner);
    if pct == 0 {
        CssValue::Length(px, Unit::Px)
    } else if px == 0 {
        CssValue::Percentage(pct)
    } else {
        CssValue::Calc(px, pct)
    }
}

fn eval_calc_components(s: &str) -> (i32, i32) {
    let s = s.trim();
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
            if lpct == 0 && rpct == 0 {
                return (lpx * rpx / 100, 0);
            } else if lpct == 0 && rpct != 0 {
                let mul = lpx;
                return (rpx * mul / 100, rpct * mul / 100);
            } else if rpct == 0 {
                let mul = rpx;
                return (lpx * mul / 100, lpct * mul / 100);
            } else {
                // Invalid CSS math: dimensions/percentages cannot be multiplied
                // by another percentage-bearing term into a length-percentage.
                return (0, 0);
            }
        } else {
            let div = rpx;
            if div != 0 {
                return (lpx * 100 / div, lpct * 100 / div);
            } else {
                return (0, 0);
            }
        }
    }

    parse_calc_operand(s)
}

fn split_calc_expr(s: &str) -> Option<(&str, u8, &str)> {
    let bytes = s.as_bytes();
    let mut depth: usize = 0;
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

fn parse_calc_operand(s: &str) -> (i32, i32) {
    let s = s.trim();
    let lower = s.to_ascii_lowercase();
    let lower = lower.trim();

    if lower.starts_with("calc(") || (lower.starts_with('(') && lower.ends_with(')')) {
        let inner = if lower.starts_with("calc(") {
            lower[5..].strip_suffix(')').unwrap_or(&lower[5..])
        } else {
            &lower[1..lower.len() - 1]
        };
        return eval_calc_components(inner);
    }
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
        (val * 16, 0)
    } else if lower.ends_with("vw")
        || lower.ends_with("vh")
        || lower.ends_with("vmin")
        || lower.ends_with("vmax")
    {
        let suffix_len = if lower.ends_with("vmin") || lower.ends_with("vmax") {
            4
        } else {
            2
        };
        let num = &lower[..lower.len() - suffix_len];
        let val = parse_fixed_100(num);
        (0, val)
    } else {
        let val = parse_fixed_100(s);
        (val, 0)
    }
}
