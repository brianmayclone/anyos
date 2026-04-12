fn parse_px_value(s: &str) -> Option<i32> {
    let s = s.trim();
    if s.to_ascii_lowercase().starts_with("calc(") {
        return eval_media_calc(s);
    }
    if s.ends_with("rem") {
        let n = &s[..s.len() - 3];
        return parse_float_px(n).map(|v| (v * 16.0) as i32);
    }
    if s.ends_with("em") {
        let n = &s[..s.len() - 2];
        return parse_float_px(n).map(|v| (v * 16.0) as i32);
    }
    let s = s.trim_end_matches("px").trim();
    let mut val: i32 = 0;
    for b in s.as_bytes() {
        if *b >= b'0' && *b <= b'9' {
            val = val * 10 + (*b - b'0') as i32;
        } else if *b == b'.' {
            break;
        } else {
            break;
        }
    }
    if val > 0 || s == "0" { Some(val) } else { None }
}

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
            b'.' if !in_frac => in_frac = true,
            _ => break,
        }
    }
    if has_digit { Some(result + frac) } else { None }
}

fn eval_media_calc(s: &str) -> Option<i32> {
    let lower = s.to_ascii_lowercase();
    let inner = lower.strip_prefix("calc(")?;
    let inner = strip_outer_paren(inner)?;
    eval_calc_expr_px(inner)
}

fn strip_outer_paren(s: &str) -> Option<&str> {
    let s = s.trim();
    if s.ends_with(')') { Some(&s[..s.len() - 1]) } else { Some(s) }
}

fn eval_calc_expr_px(s: &str) -> Option<i32> {
    let val = eval_calc_f32(s.trim())?;
    Some((val + 0.5) as i32)
}

fn eval_calc_f32(s: &str) -> Option<f32> {
    let s = s.trim();
    let bytes = s.as_bytes();
    let mut depth: i32 = 0;
    let mut split_pos: Option<usize> = None;
    let mut split_op: u8 = 0;
    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        match bytes[i] {
            b')' => depth += 1,
            b'(' => depth -= 1,
            b'+' | b'-' if depth == 0 && i > 0 => {
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
        return Some(if split_op == b'+' { left + right } else { left - right });
    }
    depth = 0;
    split_pos = None;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b'*' | b'/' if depth == 0 => {
                split_pos = Some(i);
                split_op = b;
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
    let s_lower = s.to_ascii_lowercase();
    let s_lower = s_lower.trim();
    if s_lower.starts_with("calc(") {
        let inner = s_lower.strip_prefix("calc(")?;
        let inner = strip_outer_paren(inner)?;
        return eval_calc_f32(inner);
    }
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
        return Some(0.0);
    }
    let neg = s_lower.starts_with('-');
    let s2 = if neg { &s_lower[1..] } else { s_lower };
    parse_float_px(s2).map(|v| if neg { -v } else { v })
}
