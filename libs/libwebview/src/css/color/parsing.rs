fn try_parse_color(s: &str) -> Option<u32> {
    let bytes = s.as_bytes();
    if bytes.first() == Some(&b'#') {
        return parse_hex_color(&s[1..]);
    }
    let lower = to_ascii_lower(s);
    if lower.starts_with("rgba(") && lower.ends_with(')') {
        return parse_rgba_func(&lower[5..lower.len() - 1]);
    }
    if lower.starts_with("rgb(") && lower.ends_with(')') {
        return parse_rgb_func(&lower[4..lower.len() - 1]);
    }
    if lower.starts_with("hsla(") && lower.ends_with(')') {
        return parse_hsla_func(&lower[5..lower.len() - 1]);
    }
    if lower.starts_with("hsl(") && lower.ends_with(')') {
        return parse_hsl_func(&lower[4..lower.len() - 1]);
    }
    if lower.starts_with("hwb(") && lower.ends_with(')') {
        return parse_hwb_func(&lower[4..lower.len() - 1]);
    }
    if lower.starts_with("lab(") && lower.ends_with(')') {
        return parse_lab_func(&lower[4..lower.len() - 1]);
    }
    if lower.starts_with("lch(") && lower.ends_with(')') {
        return parse_lch_func(&lower[4..lower.len() - 1]);
    }
    if lower.starts_with("oklab(") && lower.ends_with(')') {
        return parse_oklab_func(&lower[6..lower.len() - 1]);
    }
    if lower.starts_with("oklch(") && lower.ends_with(')') {
        return parse_oklch_func(&lower[6..lower.len() - 1]);
    }
    if lower.starts_with("color(") && lower.ends_with(')') {
        return parse_color_func(&lower[6..lower.len() - 1]);
    }
    if lower.starts_with("color-mix(") && lower.ends_with(')') {
        return parse_color_mix_func(&lower[10..lower.len() - 1]);
    }
    if lower.starts_with("light-dark(") && lower.ends_with(')') {
        let inner = &s[11..s.len() - 1];
        if let Some(comma) = inner.find(',') {
            return try_parse_color(inner[..comma].trim());
        }
    }
    named_color(&lower)
}

fn parse_hex_color(hex: &str) -> Option<u32> {
    let len = hex.len();
    match len {
        3 => {
            let r = hex_digit(hex.as_bytes()[0])? as u32;
            let g = hex_digit(hex.as_bytes()[1])? as u32;
            let b = hex_digit(hex.as_bytes()[2])? as u32;
            Some(0xFF000000 | (r * 17) << 16 | (g * 17) << 8 | (b * 17))
        }
        4 => {
            let r = hex_digit(hex.as_bytes()[0])? as u32;
            let g = hex_digit(hex.as_bytes()[1])? as u32;
            let b = hex_digit(hex.as_bytes()[2])? as u32;
            let a = hex_digit(hex.as_bytes()[3])? as u32;
            Some((a * 17) << 24 | (r * 17) << 16 | (g * 17) << 8 | (b * 17))
        }
        6 => {
            let v = parse_hex_u32(hex)?;
            Some(0xFF000000 | v)
        }
        8 => {
            let v = parse_hex_u32(hex)?;
            let rr = (v >> 24) & 0xFF;
            let gg = (v >> 16) & 0xFF;
            let bb = (v >> 8) & 0xFF;
            let aa = v & 0xFF;
            Some(aa << 24 | rr << 16 | gg << 8 | bb)
        }
        _ => Option::None,
    }
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => Option::None,
    }
}

fn parse_hex_u32(hex: &str) -> Option<u32> {
    let mut val: u32 = 0;
    for &b in hex.as_bytes() {
        val = val.checked_shl(4)?;
        val |= hex_digit(b)? as u32;
    }
    Some(val)
}

fn parse_rgb_func(args: &str) -> Option<u32> {
    let clean;
    let args = if args.contains("/*") { clean = strip_css_comments(args); clean.as_str() } else { args };
    let (color_part, alpha_part) = split_color_alpha(args);
    let parts = split_args(color_part);
    if parts.len() < 3 {
        return Option::None;
    }
    let r = parse_color_component(parts[0])?.min(255);
    let g = parse_color_component(parts[1])?.min(255);
    let b = parse_color_component(parts[2])?.min(255);
    if let Some(alpha_str) = alpha_part {
        let a = parse_alpha_value(alpha_str);
        Some((a << 24) | (r << 16) | (g << 8) | b)
    } else if parts.len() >= 4 {
        let a = parse_alpha_value(parts[3]);
        Some((a << 24) | (r << 16) | (g << 8) | b)
    } else {
        Some(0xFF000000 | (r << 16) | (g << 8) | b)
    }
}

fn parse_rgba_func(args: &str) -> Option<u32> {
    let clean;
    let args = if args.contains("/*") { clean = strip_css_comments(args); clean.as_str() } else { args };
    let (color_part, alpha_part) = split_color_alpha(args);
    let parts = split_args(color_part);
    if parts.len() < 3 {
        return Option::None;
    }
    let r = parse_color_component(parts[0])?.min(255);
    let g = parse_color_component(parts[1])?.min(255);
    let b = parse_color_component(parts[2])?.min(255);
    let a = if let Some(alpha_str) = alpha_part {
        parse_alpha_value(alpha_str)
    } else if parts.len() >= 4 {
        parse_alpha_value(parts[3])
    } else {
        255u32
    };
    Some((a << 24) | (r << 16) | (g << 8) | b)
}

fn split_color_alpha(args: &str) -> (&str, Option<&str>) {
    let mut depth: u32 = 0;
    let bytes = args.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'(' {
            depth += 1;
        } else if b == b')' {
            depth = depth.saturating_sub(1);
        } else if b == b'/' && depth == 0 {
            let color = args[..i].trim();
            let alpha = args[i + 1..].trim();
            return (color, Some(alpha));
        }
    }
    (args, Option::None)
}

fn parse_alpha_value(s: &str) -> u32 {
    let t = s.trim();
    if t.starts_with("var(") || t.contains("var(") {
        return 255;
    }
    if t.ends_with('%') {
        let num = &t[..t.len() - 1];
        if num.contains('.') {
            if let Some(fp) = parse_fixed_point(num) {
                return ((fp as i64 * 255 / 10000).max(0).min(255)) as u32;
            }
        }
        if let Some(pct) = parse_int(num) {
            return ((pct.max(0).min(100) as u32) * 255) / 100;
        }
        return 255;
    }
    if t.contains('.') {
        if let Some(fp) = parse_fixed_point(t) {
            return ((fp * 255) / 100).max(0).min(255) as u32;
        }
        return 255;
    }
    if let Some(v) = parse_int(t) {
        if v <= 1 {
            return (v.max(0) as u32) * 255;
        }
        return v.max(0).min(255) as u32;
    }
    255
}

fn parse_color_component(s: &str) -> Option<u32> {
    let t = s.trim();
    if t.ends_with('%') {
        let num = &t[..t.len() - 1];
        if num.contains('.') {
            let fp = parse_fixed_point(num)?;
            return Some(((fp as i64 * 255 / 10000).max(0).min(255)) as u32);
        }
        let pct = parse_int(num)?;
        Some(((pct.max(0).min(100) as u32) * 255) / 100)
    } else if t.contains('.') {
        let fp = parse_fixed_point(t)?;
        Some((fp / 100).max(0).min(255) as u32)
    } else {
        Some(parse_int(t)?.max(0).min(255) as u32)
    }
}

fn strip_css_comments(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut inserted_sep = false;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            if !out.is_empty() && !out.ends_with(char::is_whitespace) {
                out.push(' ');
                inserted_sep = true;
            }
            i += 2;
            while i + 1 < bytes.len() {
                if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    i += 2;
                    break;
                }
                i += 1;
            }
        } else {
            if inserted_sep && bytes[i].is_ascii_whitespace() {
                i += 1;
                continue;
            }
            out.push(bytes[i] as char);
            inserted_sep = false;
            i += 1;
        }
    }
    out
}

fn split_args(s: &str) -> Vec<&str> {
    if s.contains(',') {
        s.split(',').collect()
    } else {
        s.split_whitespace().collect()
    }
}

fn parse_hsl_func(args: &str) -> Option<u32> {
    let clean;
    let args = if args.contains("/*") { clean = strip_css_comments(args); clean.as_str() } else { args };
    let (color_part, alpha_part) = split_color_alpha(args);
    let parts = split_args(color_part);
    if parts.len() < 3 {
        return Option::None;
    }
    let h = parse_hue(parts[0])?;
    let s = parse_percent_val(parts[1])?;
    let l = parse_percent_val(parts[2])?;
    let (r, g, b) = hsl_to_rgb(h, s, l);
    if let Some(alpha_str) = alpha_part {
        let a = parse_alpha_value(alpha_str);
        Some((a << 24) | (r << 16) | (g << 8) | b)
    } else if parts.len() >= 4 {
        let a = parse_alpha_value(parts[3]);
        Some((a << 24) | (r << 16) | (g << 8) | b)
    } else {
        Some(0xFF000000 | (r << 16) | (g << 8) | b)
    }
}

fn parse_hsla_func(args: &str) -> Option<u32> {
    let clean;
    let args = if args.contains("/*") { clean = strip_css_comments(args); clean.as_str() } else { args };
    let (color_part, alpha_part) = split_color_alpha(args);
    let parts = split_args(color_part);
    if parts.len() < 3 {
        return Option::None;
    }
    let h = parse_hue(parts[0])?;
    let s = parse_percent_val(parts[1])?;
    let l = parse_percent_val(parts[2])?;
    let (r, g, b) = hsl_to_rgb(h, s, l);
    let a = if let Some(alpha_str) = alpha_part {
        parse_alpha_value(alpha_str)
    } else if parts.len() >= 4 {
        parse_alpha_value(parts[3])
    } else {
        255u32
    };
    Some((a << 24) | (r << 16) | (g << 8) | b)
}

fn parse_hue(s: &str) -> Option<i32> {
    let t = s.trim();
    if t.ends_with("deg") {
        return parse_hue_number(&t[..t.len() - 3]);
    }
    if t.ends_with("turn") {
        let scaled = parse_decimal_scaled(&t[..t.len() - 4])?;
        return Some(div_round_i64(scaled * 360, 1_000_000) as i32);
    }
    if t.ends_with("rad") {
        let scaled = parse_decimal_scaled(&t[..t.len() - 3])?;
        return Some(div_round_i64(scaled * 180, 3_141_593) as i32);
    }
    if t.ends_with("grad") {
        let scaled = parse_decimal_scaled(&t[..t.len() - 4])?;
        return Some(div_round_i64(scaled * 9, 10_000_000) as i32);
    }
    parse_hue_number(t)
}

fn parse_decimal_scaled(s: &str) -> Option<i64> {
    let bytes = s.trim().as_bytes();
    if bytes.is_empty() {
        return Option::None;
    }

    let mut i = 0usize;
    let negative = if bytes[i] == b'-' {
        i += 1;
        true
    } else if bytes[i] == b'+' {
        i += 1;
        false
    } else {
        false
    };
    if i >= bytes.len() {
        return Option::None;
    }

    let mut integer_part: i64 = 0;
    let mut saw_digit = false;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        integer_part = integer_part * 10 + (bytes[i] - b'0') as i64;
        i += 1;
        saw_digit = true;
    }

    let mut frac: i64 = 0;
    let mut frac_scale: i64 = 1_000_000;
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            if frac_scale > 1 {
                frac_scale /= 10;
                frac += (bytes[i] - b'0') as i64 * frac_scale;
            }
            i += 1;
            saw_digit = true;
        }
    }

    if !saw_digit {
        return Option::None;
    }

    let mut value = integer_part * 1_000_000 + frac;
    if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
        i += 1;
        if i >= bytes.len() {
            return Option::None;
        }
        let exp_negative = if bytes[i] == b'-' {
            i += 1;
            true
        } else if bytes[i] == b'+' {
            i += 1;
            false
        } else {
            false
        };
        if i >= bytes.len() || !bytes[i].is_ascii_digit() {
            return Option::None;
        }
        let mut exponent: u32 = 0;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            exponent = exponent.saturating_mul(10).saturating_add((bytes[i] - b'0') as u32);
            i += 1;
        }
        let pow10 = 10_i64.saturating_pow(exponent.min(9));
        value = if exp_negative {
            value / pow10.max(1)
        } else {
            value.saturating_mul(pow10)
        };
    }
    if i != bytes.len() {
        return Option::None;
    }
    Some(if negative { -value } else { value })
}

fn div_round_i64(value: i64, divisor: i64) -> i64 {
    if value >= 0 {
        (value + divisor / 2) / divisor
    } else {
        (value - divisor / 2) / divisor
    }
}

fn parse_hue_number(s: &str) -> Option<i32> {
    let scaled = parse_decimal_scaled(s.trim())?;
    Some(div_round_i64(scaled, 1_000_000) as i32)
}

fn parse_percent_val(s: &str) -> Option<i32> {
    let t = s.trim();
    if t.ends_with('%') {
        parse_int(&t[..t.len() - 1])
    } else {
        parse_int(t)
    }
}

fn hsl_to_rgb(h: i32, s: i32, l: i32) -> (u32, u32, u32) {
    let h = ((h % 360) + 360) % 360;
    let s = s.max(0).min(100);
    let l = l.max(0).min(100);
    if s == 0 {
        let v = (l * 255 / 100) as u32;
        return (v, v, v);
    }
    let l1000 = l as i64 * 10;
    let s1000 = s as i64 * 10;
    let q = if l1000 < 500 {
        l1000 * (1000 + s1000) / 1000
    } else {
        l1000 + s1000 - (l1000 * s1000 / 1000)
    };
    let p = 2 * l1000 - q;
    let r = hue_to_rgb_channel(p, q, h as i64 + 120);
    let g = hue_to_rgb_channel(p, q, h as i64);
    let b = hue_to_rgb_channel(p, q, h as i64 - 120);
    (r as u32, g as u32, b as u32)
}

fn hue_to_rgb_channel(p: i64, q: i64, mut h: i64) -> i64 {
    if h < 0 {
        h += 360;
    }
    if h >= 360 {
        h -= 360;
    }
    let val = if h < 60 {
        p + (q - p) * h / 60
    } else if h < 180 {
        q
    } else if h < 240 {
        p + (q - p) * (240 - h) / 60
    } else {
        p
    };
    (val * 255 / 1000).max(0).min(255)
}
