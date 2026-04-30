enum CssMathFunc {
    Min,
    Max,
    Clamp,
}

fn parse_min_max_clamp_value(s: &str, func: CssMathFunc) -> CssValue {
    let lower = s.to_ascii_lowercase();
    eval_min_max_clamp(&lower, func)
}

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

fn eval_min_max_clamp(s: &str, func: CssMathFunc) -> CssValue {
    let paren_start = match s.find('(') {
        Some(i) => i,
        None => return CssValue::None,
    };
    let inner = s[paren_start + 1..].trim_end_matches(')').trim();
    let args: Vec<&str> = split_top_level_commas(inner);
    let vals: Vec<(i32, i32)> = args.iter().map(|a| eval_calc_components(a)).collect();

    match func {
        CssMathFunc::Min => {
            if vals.iter().all(|(_, pct)| *pct == 0) {
                let min_px = vals.iter().map(|(px, _)| *px).min().unwrap_or(0);
                return CssValue::Length(min_px, Unit::Px);
            }
            let (px, pct) = vals.first().copied().unwrap_or((0, 0));
            if pct == 0 {
                CssValue::Length(px, Unit::Px)
            } else if px == 0 {
                CssValue::Percentage(pct)
            } else {
                CssValue::Calc(px, pct)
            }
        }
        CssMathFunc::Max => {
            if vals.iter().all(|(_, pct)| *pct == 0) {
                let max_px = vals.iter().map(|(px, _)| *px).max().unwrap_or(0);
                return CssValue::Length(max_px, Unit::Px);
            }
            let (px, pct) = vals.last().copied().unwrap_or((0, 0));
            if pct == 0 {
                CssValue::Length(px, Unit::Px)
            } else if px == 0 {
                CssValue::Percentage(pct)
            } else {
                CssValue::Calc(px, pct)
            }
        }
        CssMathFunc::Clamp => {
            if vals.len() >= 3 {
                let (min_px, min_pct) = vals[0];
                let (val_px, val_pct) = vals[1];
                let (max_px, max_pct) = vals[2];
                if min_pct == 0 && val_pct == 0 && max_pct == 0 {
                    let v = val_px.max(min_px).min(max_px);
                    return CssValue::Length(v, Unit::Px);
                }
                if val_pct == 0 {
                    CssValue::Length(val_px, Unit::Px)
                } else if val_px == 0 {
                    CssValue::Percentage(val_pct)
                } else {
                    CssValue::Calc(val_px, val_pct)
                }
            } else {
                let (px, pct) = vals.first().copied().unwrap_or((0, 0));
                if pct == 0 {
                    CssValue::Length(px, Unit::Px)
                } else {
                    CssValue::Percentage(pct)
                }
            }
        }
    }
}

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
    while frac_digits < 2 {
        frac_part *= 10;
        frac_digits += 1;
    }
    let val = int_part * 100 + frac_part;
    if neg { -val } else { val }
}
