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
    if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
        let exp_start = i;
        i += 1;
        if i < bytes.len() && (bytes[i] == b'-' || bytes[i] == b'+') {
            i += 1;
        }
        let digits_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if digits_start == i {
            i = exp_start;
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
        "in" => Some(CssValue::Length(val, Unit::In)),
        "cm" => Some(CssValue::Length(val, Unit::Cm)),
        "mm" => Some(CssValue::Length(val, Unit::Mm)),
        "pt" => Some(CssValue::Length(val, Unit::Pt)),
        "pc" => Some(CssValue::Length(val, Unit::Pc)),
        "q" => Some(CssValue::Length(val, Unit::Q)),
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
    const MAX_FIXED: i32 = i32::MAX / 4;
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
            .saturating_mul(10)
            .saturating_add((bytes[i] - b'0') as i32);
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

    let val = integer_part
        .saturating_mul(100)
        .saturating_add(frac)
        .min(MAX_FIXED);
    let mut val = val;
    if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
        i += 1;
        let exp_negative = if i < bytes.len() && bytes[i] == b'-' {
            i += 1;
            true
        } else if i < bytes.len() && bytes[i] == b'+' {
            i += 1;
            false
        } else {
            false
        };
        let mut exp = 0u32;
        let exp_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            exp = exp
                .saturating_mul(10)
                .saturating_add((bytes[i] - b'0') as u32)
                .min(64);
            i += 1;
        }
        if i == exp_start {
            return Option::None;
        }
        for _ in 0..exp {
            if exp_negative {
                val /= 10;
            } else {
                val = val.saturating_mul(10).min(MAX_FIXED);
            }
        }
    }
    Some(if negative { val.saturating_neg() } else { val })
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

#[cfg(test)]
mod primitive_tests {
    use super::*;

    #[test]
    fn dimensions_accept_scientific_notation() {
        assert!(matches!(
            try_parse_dimension("3.40282e38px"),
            Some(CssValue::Length(v, Unit::Px)) if v > 1_000_000
        ));
        assert!(matches!(
            try_parse_dimension("1.5e2px"),
            Some(CssValue::Length(15_000, Unit::Px))
        ));
        assert!(matches!(
            try_parse_dimension("1e-2px"),
            Some(CssValue::Length(1, Unit::Px))
        ));
    }
}
