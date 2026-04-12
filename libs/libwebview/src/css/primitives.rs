fn try_parse_dimension(s: &str) -> Option<CssValue> {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return Option::None;
    }

    let first = bytes[0];
    if !(first.is_ascii_digit() || first == b'-' || first == b'+' || first == b'.') {
        return Option::None;
    }

    let mut i = 0;
    if bytes[i] == b'-' || bytes[i] == b'+' {
        i += 1;
    }
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }

    if i == 0 || (i == 1 && (bytes[0] == b'-' || bytes[0] == b'+' || bytes[0] == b'.')) {
        return Option::None;
    }

    let num_str = core::str::from_utf8(&bytes[..i]).ok()?;
    let suffix = core::str::from_utf8(&bytes[i..]).ok()?.trim();
    let val = parse_fixed_point(num_str)?;

    if suffix.is_empty() {
        if val == 0 {
            return Some(CssValue::Length(0, Unit::Px));
        }
        return Some(CssValue::Number(val));
    }

    let lower_suffix = to_ascii_lower(suffix);
    match lower_suffix.as_str() {
        "px" => Some(CssValue::Length(val, Unit::Px)),
        "em" => Some(CssValue::Length(val, Unit::Em)),
        "rem" => Some(CssValue::Length(val, Unit::Rem)),
        "pt" => Some(CssValue::Length(val, Unit::Pt)),
        "%" => Some(CssValue::Percentage(val)),
        "fr" => Some(CssValue::Length(val, Unit::Fr)),
        "vw" => Some(CssValue::Length(val, Unit::Vw)),
        "vh" => Some(CssValue::Length(val, Unit::Vh)),
        "vmin" => Some(CssValue::Length(val, Unit::Vmin)),
        "vmax" => Some(CssValue::Length(val, Unit::Vmax)),
        _ => Option::None,
    }
}

fn parse_fixed_point(s: &str) -> Option<i32> {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return Option::None;
    }

    let mut i = 0;
    let negative = if bytes[i] == b'-' {
        i += 1;
        true
    } else if bytes[i] == b'+' {
        i += 1;
        false
    } else {
        false
    };

    let mut integer_part: i32 = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        integer_part = integer_part
            .wrapping_mul(10)
            .wrapping_add((bytes[i] - b'0') as i32);
        i += 1;
    }

    let mut frac: i32 = 0;
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        let d1 = if i < bytes.len() && bytes[i].is_ascii_digit() {
            let d = (bytes[i] - b'0') as i32;
            i += 1;
            d
        } else {
            0
        };
        let d2 = if i < bytes.len() && bytes[i].is_ascii_digit() {
            let d = (bytes[i] - b'0') as i32;
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            d
        } else {
            0
        };
        frac = d1 * 10 + d2;
    }

    let val = integer_part * 100 + frac;
    Some(if negative { -val } else { val })
}

fn parse_int(s: &str) -> Option<i32> {
    let s = s.trim();
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return Option::None;
    }
    let mut i = 0;
    let neg = if bytes[0] == b'-' {
        i += 1;
        true
    } else {
        false
    };
    let mut val: i32 = 0;
    if i >= bytes.len() {
        return Option::None;
    }
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        val = val * 10 + (bytes[i] - b'0') as i32;
        i += 1;
    }
    Some(if neg { -val } else { val })
}

fn to_ascii_lower(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if b >= b'A' && b <= b'Z' {
            out.push((b + 32) as char);
        } else {
            out.push(b as char);
        }
    }
    out
}

pub fn try_parse_color_pub(s: &str) -> Option<u32> {
    try_parse_color(s)
}

pub fn named_color_pub(name: &str) -> Option<u32> {
    named_color(name)
}

pub fn try_parse_dimension_pub(s: &str) -> Option<CssValue> {
    try_parse_dimension(s)
}
