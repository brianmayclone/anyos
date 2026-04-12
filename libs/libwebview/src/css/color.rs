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
        // light-dark(light, dark) — use light value for now (no dark mode support)
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
            // #RGB -> AARRGGBB
            let r = hex_digit(hex.as_bytes()[0])? as u32;
            let g = hex_digit(hex.as_bytes()[1])? as u32;
            let b = hex_digit(hex.as_bytes()[2])? as u32;
            Some(0xFF000000 | (r * 17) << 16 | (g * 17) << 8 | (b * 17))
        }
        4 => {
            // #RGBA
            let r = hex_digit(hex.as_bytes()[0])? as u32;
            let g = hex_digit(hex.as_bytes()[1])? as u32;
            let b = hex_digit(hex.as_bytes()[2])? as u32;
            let a = hex_digit(hex.as_bytes()[3])? as u32;
            Some((a * 17) << 24 | (r * 17) << 16 | (g * 17) << 8 | (b * 17))
        }
        6 => {
            // #RRGGBB
            let v = parse_hex_u32(hex)?;
            Some(0xFF000000 | v)
        }
        8 => {
            // #RRGGBBAA
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
    // Modern CSS: rgb(R G B) or rgb(R G B / alpha)
    // Tailwind: rgb(R G B/var(--tw-bg-opacity,1))
    // Strip CSS comments: rgb(10/* comment */175/* comment */10)
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
        // Legacy: rgb(R, G, B, A) with comma syntax
        let a = parse_alpha_value(parts[3]);
        Some((a << 24) | (r << 16) | (g << 8) | b)
    } else {
        Some(0xFF000000 | (r << 16) | (g << 8) | b)
    }
}

fn parse_rgba_func(args: &str) -> Option<u32> {
    // rgba() is identical to rgb() in modern CSS
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

/// Split "R G B / alpha" or "R G B/alpha" into color part and optional alpha.
/// Handles var() references by not splitting on / inside parentheses.
fn split_color_alpha(args: &str) -> (&str, Option<&str>) {
    // Find the `/` that separates color from alpha, respecting parentheses
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

/// Parse an alpha value string. Handles fractional (0.0-1.0), integer (0-255),
/// var() references (default to 1.0), and percentage.
fn parse_alpha_value(s: &str) -> u32 {
    let t = s.trim();
    // If it's a var() or other unresolvable expression, default to fully opaque
    if t.starts_with("var(") || t.contains("var(") {
        return 255;
    }
    if t.ends_with('%') {
        let num = &t[..t.len() - 1];
        if num.contains('.') {
            // Decimal percentage: "50.5%" → 50.5% of 255
            if let Some(fp) = parse_fixed_point(num) {
                // fp is value * 100, so 50.5% → 5050
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
    // Integer alpha: if <= 1, treat as 0 or 1 (fraction)
    if let Some(v) = parse_int(t) {
        if v <= 1 {
            return (v.max(0) as u32) * 255;
        }
        return v.max(0).min(255) as u32;
    }
    255 // default: fully opaque
}

fn parse_color_component(s: &str) -> Option<u32> {
    let t = s.trim();
    if t.ends_with('%') {
        let num = &t[..t.len() - 1];
        if num.contains('.') {
            let fp = parse_fixed_point(num)?; // value * 100
            return Some(((fp as i64 * 255 / 10000).max(0).min(255)) as u32);
        }
        let pct = parse_int(num)?;
        Some(((pct.max(0).min(100) as u32) * 255) / 100)
    } else if t.contains('.') {
        // Decimal component: 10.0 → 10
        let fp = parse_fixed_point(t)?; // value * 100
        Some((fp / 100).max(0).min(255) as u32)
    } else {
        Some(parse_int(t)?.max(0).min(255) as u32)
    }
}

/// Strip CSS comments (`/* ... */`) from a string.
fn strip_css_comments(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            // Skip until */
            i += 2;
            while i + 1 < bytes.len() {
                if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    i += 2;
                    break;
                }
                i += 1;
            }
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

fn split_args(s: &str) -> Vec<&str> {
    // Split on ',' or whitespace-separated (modern CSS syntax)
    if s.contains(',') {
        s.split(',').collect()
    } else {
        s.split_whitespace().collect()
    }
}


fn parse_hsl_func(args: &str) -> Option<u32> {
    // Modern CSS: hsl(H S L) or hsl(H S L / alpha)
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
    // hsla() is identical to hsl() in modern CSS
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
        // 1turn = 360deg
        let fp = parse_fixed_point(&t[..t.len() - 4])?; // value * 100
        return Some((fp as i64 * 360 / 100) as i32);
    }
    if t.ends_with("rad") {
        // 2π rad = 360deg → deg = rad * 180 / π
        let fp = parse_fixed_point(&t[..t.len() - 3])?; // value * 100
        // π ≈ 314 (×100), so deg = fp * 180 / 314
        return Some((fp as i64 * 18000 / 31416) as i32);
    }
    if t.ends_with("grad") {
        // 400grad = 360deg → deg = grad * 0.9
        let fp = parse_fixed_point(&t[..t.len() - 4])?; // value * 100
        return Some((fp as i64 * 9 / 1000) as i32);
    }
    // Bare number = degrees
    parse_hue_number(t)
}

fn parse_hue_number(s: &str) -> Option<i32> {
    let t = s.trim();
    if t.contains('.') {
        let fp = parse_fixed_point(t)?; // value * 100
        Some(fp / 100)
    } else {
        parse_int(t)
    }
}

fn parse_percent_val(s: &str) -> Option<i32> {
    let t = s.trim();
    if t.ends_with('%') {
        parse_int(&t[..t.len() - 1])
    } else {
        parse_int(t)
    }
}

/// Convert HSL to RGB. h in degrees [0..360], s and l in percent [0..100].
/// Returns (r, g, b) each in [0..255].
fn hsl_to_rgb(h: i32, s: i32, l: i32) -> (u32, u32, u32) {
    let h = ((h % 360) + 360) % 360;
    let s = s.max(0).min(100);
    let l = l.max(0).min(100);

    if s == 0 {
        let v = (l * 255 / 100) as u32;
        return (v, v, v);
    }

    // Use fixed-point * 1000 arithmetic.
    let l1000 = l as i64 * 10; // l in 0..1000
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

// ── CSS Color Level 4: hwb(), lab(), lch(), oklab(), oklch(), color() ─────

// Math helpers for no_std (no libm). Uses polynomial/lookup approximations.

/// Approximate x^p for color gamma (p is 2.4, 1/2.4, 1.8, 1/0.45, 563/256 etc.)
/// Uses exp/ln: x^p = exp(p * ln(x)).
fn powf_approx(x: f64, p: f64) -> f64 {
    if x <= 0.0 { return 0.0; }
    if x >= 1.0 && p >= 0.0 { return 1.0_f64.min(exp_approx(p * ln_approx(x))); }
    exp_approx(p * ln_approx(x))
}

/// Approximate ln(x) using a series expansion around 1. Works for x > 0.
fn ln_approx(x: f64) -> f64 {
    if x <= 0.0 { return -1e30; }
    // Reduce to [0.5, 2): extract exponent via bit manipulation or iterative normalization
    let mut val = x;
    let mut exp = 0i32;
    while val >= 2.0 { val /= 2.0; exp += 1; }
    while val < 0.5 { val *= 2.0; exp -= 1; }
    // ln(val) for val in [0.5, 2) using Padé-like: ln(1+u) ≈ u(6+u)/(6+4u) for |u|<1
    let u = val - 1.0;
    let ln_val = u * (6.0 + u) / (6.0 + 4.0 * u);
    ln_val + (exp as f64) * 0.6931471805599453 // ln(2)
}

/// Approximate exp(x).
fn exp_approx(x: f64) -> f64 {
    if x > 88.0 { return 1e38; }
    if x < -88.0 { return 0.0; }
    // Reduce: exp(x) = 2^k * exp(r) where x = k*ln2 + r
    let ln2 = 0.6931471805599453;
    let k_raw = x / ln2;
    let k = if k_raw >= 0.0 { (k_raw + 0.5) as i64 as f64 } else { (k_raw - 0.5) as i64 as f64 };
    let r = x - k * ln2;
    // exp(r) for |r| < 0.5 using Taylor: 1 + r + r²/2 + r³/6 + r⁴/24 + r⁵/120
    let er = 1.0 + r * (1.0 + r * (0.5 + r * (1.0/6.0 + r * (1.0/24.0 + r * (1.0/120.0)))));
    er * pow2_approx(k as i32)
}

fn pow2_approx(n: i32) -> f64 {
    if n >= 0 {
        let mut v = 1.0_f64;
        for _ in 0..n.min(63) { v *= 2.0; }
        v
    } else {
        let mut v = 1.0_f64;
        for _ in 0..(-n).min(63) { v /= 2.0; }
        v
    }
}

/// Approximate cos(x) in radians. Minimax polynomial.
fn cos_f64(x: f64) -> f64 {
    // Reduce to [0, 2π)
    let pi2 = 6.283185307179586;
    let mut a = x % pi2;
    if a < 0.0 { a += pi2; }
    // Reduce to [0, π] using symmetry
    let (a, sign) = if a > 3.141592653589793 { (pi2 - a, 1.0) } else { (a, 1.0) };
    let (a, sign) = if a > 1.5707963267948966 { (3.141592653589793 - a, -sign) } else { (a, sign) };
    // cos(a) for a in [0, π/2] using Taylor: 1 - a²/2 + a⁴/24 - a⁶/720
    let a2 = a * a;
    sign * (1.0 - a2 * (0.5 - a2 * (1.0/24.0 - a2 * (1.0/720.0 - a2 / 40320.0))))
}

fn sin_f64(x: f64) -> f64 {
    cos_f64(x - 1.5707963267948966)
}

/// Parse a float-like value for color functions. Returns value × 10000 for precision.
fn parse_color_float(s: &str) -> Option<i64> {
    let t = s.trim();
    if t.ends_with('%') {
        let num = &t[..t.len() - 1];
        if num.contains('.') {
            let fp = parse_fixed_point(num)?; // × 100
            return Some(fp as i64 * 100); // → × 10000
        }
        return Some(parse_int(num)? as i64 * 10000);
    }
    if t.contains('.') {
        let fp = parse_fixed_point(t)?; // × 100
        return Some(fp as i64 * 100); // → × 10000
    }
    Some(parse_int(t)? as i64 * 10000)
}

fn srgb_to_linear(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        powf_approx((c + 0.055) / 1.055, 2.4)
    }
}

/// hwb(H W B) or hwb(H W B / alpha)
fn parse_hwb_func(args: &str) -> Option<u32> {
    let (color_part, alpha_part) = split_color_alpha(args);
    let parts = split_args(color_part);
    if parts.len() < 3 { return None; }
    let h = parse_hue(parts[0])?;
    let w = parse_percent_val(parts[1])?.max(0).min(100) as f64 / 100.0;
    let b = parse_percent_val(parts[2])?.max(0).min(100) as f64 / 100.0;
    // HWB to RGB: if w + b >= 1, result is grey
    let (r, g, bl) = if w + b >= 1.0 {
        let grey = w / (w + b);
        (grey, grey, grey)
    } else {
        let (hr, hg, hb) = hsl_to_rgb(h, 100, 50);
        let rf = hr as f64 / 255.0;
        let gf = hg as f64 / 255.0;
        let bf = hb as f64 / 255.0;
        (rf * (1.0 - w - b) + w, gf * (1.0 - w - b) + w, bf * (1.0 - w - b) + w)
    };
    let ri = (r * 255.0 + 0.5).max(0.0).min(255.0) as u32;
    let gi = (g * 255.0 + 0.5).max(0.0).min(255.0) as u32;
    let bi = (bl * 255.0 + 0.5).max(0.0).min(255.0) as u32;
    let a = if let Some(a_str) = alpha_part { parse_alpha_value(a_str) } else { 255 };
    Some((a << 24) | (ri << 16) | (gi << 8) | bi)
}

/// lab(L a b) → XYZ-D50 → sRGB
fn parse_lab_func(args: &str) -> Option<u32> {
    let (color_part, alpha_part) = split_color_alpha(args);
    let parts = split_args(color_part);
    if parts.len() < 3 { return None; }
    let l_raw = parse_color_float(parts[0])?;
    let a_raw = parse_color_float(parts[1])?;
    let b_raw = parse_color_float(parts[2])?;
    // L: 0-100 (or percentage of 100), a/b: typically -125..125
    let l = if parts[0].trim().ends_with('%') { l_raw as f64 / 10000.0 } else { l_raw as f64 / 10000.0 };
    let a_val = if parts[1].trim().ends_with('%') { a_raw as f64 / 10000.0 * 1.25 } else { a_raw as f64 / 10000.0 };
    let b_val = if parts[2].trim().ends_with('%') { b_raw as f64 / 10000.0 * 1.25 } else { b_raw as f64 / 10000.0 };
    let (r, g, b) = lab_to_srgb(l, a_val, b_val);
    let alpha = if let Some(a_str) = alpha_part { parse_alpha_value(a_str) } else { 255 };
    Some((alpha << 24) | (r << 16) | (g << 8) | b)
}

/// lch(L C H) → Lab → XYZ-D50 → sRGB
fn parse_lch_func(args: &str) -> Option<u32> {
    let (color_part, alpha_part) = split_color_alpha(args);
    let parts = split_args(color_part);
    if parts.len() < 3 { return None; }
    let l = if parts[0].trim().ends_with('%') {
        parse_color_float(parts[0])? as f64 / 10000.0
    } else {
        parse_color_float(parts[0])? as f64 / 10000.0
    };
    let c = if parts[1].trim().ends_with('%') {
        parse_color_float(parts[1])? as f64 / 10000.0 * 1.5
    } else {
        parse_color_float(parts[1])? as f64 / 10000.0
    };
    let h = parse_hue(parts[2])? as f64;
    let h_rad = h * core::f64::consts::PI / 180.0;
    let a_val = c * cos_f64(h_rad);
    let b_val = c * sin_f64(h_rad);
    let (r, g, b) = lab_to_srgb(l, a_val, b_val);
    let alpha = if let Some(a_str) = alpha_part { parse_alpha_value(a_str) } else { 255 };
    Some((alpha << 24) | (r << 16) | (g << 8) | b)
}

/// oklab(L a b) → XYZ-D65 → sRGB
fn parse_oklab_func(args: &str) -> Option<u32> {
    let (color_part, alpha_part) = split_color_alpha(args);
    let parts = split_args(color_part);
    if parts.len() < 3 { return None; }
    let l = if parts[0].trim().ends_with('%') {
        parse_color_float(parts[0])? as f64 / 10000.0 / 100.0
    } else {
        parse_color_float(parts[0])? as f64 / 10000.0
    };
    let a_val = if parts[1].trim().ends_with('%') {
        parse_color_float(parts[1])? as f64 / 10000.0 / 100.0 * 0.4
    } else {
        parse_color_float(parts[1])? as f64 / 10000.0
    };
    let b_val = if parts[2].trim().ends_with('%') {
        parse_color_float(parts[2])? as f64 / 10000.0 / 100.0 * 0.4
    } else {
        parse_color_float(parts[2])? as f64 / 10000.0
    };
    let (r, g, b) = oklab_to_srgb(l, a_val, b_val);
    let alpha = if let Some(a_str) = alpha_part { parse_alpha_value(a_str) } else { 255 };
    Some((alpha << 24) | (r << 16) | (g << 8) | b)
}

/// oklch(L C H) → OKLab → sRGB
fn parse_oklch_func(args: &str) -> Option<u32> {
    let (color_part, alpha_part) = split_color_alpha(args);
    let parts = split_args(color_part);
    if parts.len() < 3 { return None; }
    let l = if parts[0].trim().ends_with('%') {
        parse_color_float(parts[0])? as f64 / 10000.0 / 100.0
    } else {
        parse_color_float(parts[0])? as f64 / 10000.0
    };
    let c = if parts[1].trim().ends_with('%') {
        parse_color_float(parts[1])? as f64 / 10000.0 / 100.0 * 0.4
    } else {
        parse_color_float(parts[1])? as f64 / 10000.0
    };
    let h = parse_hue(parts[2])? as f64;
    let h_rad = h * core::f64::consts::PI / 180.0;
    let a_val = c * cos_f64(h_rad);
    let b_val = c * sin_f64(h_rad);
    let (r, g, b) = oklab_to_srgb(l, a_val, b_val);
    let alpha = if let Some(a_str) = alpha_part { parse_alpha_value(a_str) } else { 255 };
    Some((alpha << 24) | (r << 16) | (g << 8) | b)
}

/// color(colorspace R G B [/ alpha]) — e.g. color(srgb 1 0.5 0), color(display-p3 0.5 0.3 0.2)
fn parse_color_func(args: &str) -> Option<u32> {
    let (color_part, alpha_part) = split_color_alpha(args);
    let parts: Vec<&str> = color_part.split_whitespace().collect();
    if parts.len() < 4 { return None; }
    let space = parts[0];
    let r_f = parse_color_float(parts[1])? as f64 / 10000.0;
    let g_f = parse_color_float(parts[2])? as f64 / 10000.0;
    let b_f = parse_color_float(parts[3])? as f64 / 10000.0;
    // Convert from color space to sRGB
    let (sr, sg, sb) = match space {
        "srgb" => (r_f, g_f, b_f),
        "srgb-linear" => {
            // Linear sRGB → gamma sRGB
            let gamma = |c: f64| -> f64 {
                if c <= 0.0031308 { c * 12.92 }
                else { 1.055 * powf_approx(c, 1.0 / 2.4) - 0.055 }
            };
            (gamma(r_f), gamma(g_f), gamma(b_f))
        }
        "display-p3" => {
            // Display P3 → linear P3 → XYZ-D65 → linear sRGB → sRGB
            let rl = srgb_to_linear(r_f);
            let gl = srgb_to_linear(g_f);
            let bl = srgb_to_linear(b_f);
            // P3 to XYZ (D65)
            let x = 0.4865709486 * rl + 0.2656676932 * gl + 0.1982172852 * bl;
            let y = 0.2289745641 * rl + 0.6917385218 * gl + 0.0792869141 * bl;
            let z = 0.0000000000 * rl + 0.0451133819 * gl + 0.1043944934 * bl;
            // XYZ to linear sRGB
            let sr =  3.2404541621 * x - 1.5371385940 * y - 0.4985314096 * z;
            let sg = -0.9692660305 * x + 1.8760108454 * y + 0.0415560175 * z;
            let sb =  0.0556434309 * x - 0.2040259135 * y + 1.0572251882 * z;
            let gamma = |c: f64| -> f64 {
                let c = c.max(0.0).min(1.0);
                if c <= 0.0031308 { c * 12.92 }
                else { 1.055 * powf_approx(c, 1.0 / 2.4) - 0.055 }
            };
            (gamma(sr), gamma(sg), gamma(sb))
        }
        "a98-rgb" => {
            // Adobe RGB (1998) → linear → XYZ-D65 → linear sRGB → sRGB
            let degamma = |c: f64| -> f64 { powf_approx(if c < 0.0 { -c } else { c }, 563.0 / 256.0) * if c < 0.0 { -1.0 } else { 1.0 } };
            let rl = degamma(r_f); let gl = degamma(g_f); let bl = degamma(b_f);
            let x = 0.5766690429 * rl + 0.1855582379 * gl + 0.1882286462 * bl;
            let y = 0.2973449753 * rl + 0.6273635663 * gl + 0.0752914585 * bl;
            let z = 0.0270313614 * rl + 0.0706888525 * gl + 0.9913375368 * bl;
            let sr =  3.2404541621 * x - 1.5371385940 * y - 0.4985314096 * z;
            let sg = -0.9692660305 * x + 1.8760108454 * y + 0.0415560175 * z;
            let sb =  0.0556434309 * x - 0.2040259135 * y + 1.0572251882 * z;
            let gamma = |c: f64| -> f64 {
                let c = c.max(0.0).min(1.0);
                if c <= 0.0031308 { c * 12.92 } else { 1.055 * powf_approx(c, 1.0/2.4) - 0.055 }
            };
            (gamma(sr), gamma(sg), gamma(sb))
        }
        "prophoto-rgb" => {
            let degamma = |c: f64| -> f64 {
                if (if c < 0.0 { -c } else { c }) <= 16.0/512.0 { c / 16.0 } else { powf_approx(if c < 0.0 { -c } else { c }, 1.8) * if c < 0.0 { -1.0 } else { 1.0 } }
            };
            let rl = degamma(r_f); let gl = degamma(g_f); let bl = degamma(b_f);
            // ProPhoto to XYZ-D50
            let x = 0.7977604896 * rl + 0.1351917082 * gl + 0.0313493429 * bl;
            let y = 0.2880711282 * rl + 0.7118432178 * gl + 0.0000856540 * bl;
            let z = 0.0000000000 * rl + 0.0000000000 * gl + 0.8251046026 * bl;
            // D50 to D65 (Bradford)
            let xd = 0.9555766 * x - 0.0230393 * y + 0.0631636 * z;
            let yd = -0.0282895 * x + 1.0099416 * y + 0.0210077 * z;
            let zd = 0.0122982 * x - 0.0204830 * y + 1.3299098 * z;
            let sr =  3.2404541621 * xd - 1.5371385940 * yd - 0.4985314096 * zd;
            let sg = -0.9692660305 * xd + 1.8760108454 * yd + 0.0415560175 * zd;
            let sb =  0.0556434309 * xd - 0.2040259135 * yd + 1.0572251882 * zd;
            let gamma = |c: f64| -> f64 {
                let c = c.max(0.0).min(1.0);
                if c <= 0.0031308 { c * 12.92 } else { 1.055 * powf_approx(c, 1.0/2.4) - 0.055 }
            };
            (gamma(sr), gamma(sg), gamma(sb))
        }
        "rec2020" => {
            let alpha_rc = 1.09929682680944;
            let beta_rc = 0.018053968510807;
            let degamma = |c: f64| -> f64 {
                if (if c < 0.0 { -c } else { c }) < beta_rc * 4.5 { c / 4.5 }
                else { powf_approx((if c < 0.0 { -c } else { c } + alpha_rc - 1.0) / alpha_rc, 1.0/0.45) * if c < 0.0 { -1.0 } else { 1.0 } }
            };
            let rl = degamma(r_f); let gl = degamma(g_f); let bl = degamma(b_f);
            let x = 0.6369580483 * rl + 0.1446169036 * gl + 0.1688809752 * bl;
            let y = 0.2627002120 * rl + 0.6779980715 * gl + 0.0593017165 * bl;
            let z = 0.0000000000 * rl + 0.0280726930 * gl + 1.0609850577 * bl;
            let sr =  3.2404541621 * x - 1.5371385940 * y - 0.4985314096 * z;
            let sg = -0.9692660305 * x + 1.8760108454 * y + 0.0415560175 * z;
            let sb =  0.0556434309 * x - 0.2040259135 * y + 1.0572251882 * z;
            let gamma = |c: f64| -> f64 {
                let c = c.max(0.0).min(1.0);
                if c <= 0.0031308 { c * 12.92 } else { 1.055 * powf_approx(c, 1.0/2.4) - 0.055 }
            };
            (gamma(sr), gamma(sg), gamma(sb))
        }
        "xyz" | "xyz-d65" => {
            let sr =  3.2404541621 * r_f - 1.5371385940 * g_f - 0.4985314096 * b_f;
            let sg = -0.9692660305 * r_f + 1.8760108454 * g_f + 0.0415560175 * b_f;
            let sb =  0.0556434309 * r_f - 0.2040259135 * g_f + 1.0572251882 * b_f;
            let gamma = |c: f64| -> f64 {
                let c = c.max(0.0).min(1.0);
                if c <= 0.0031308 { c * 12.92 } else { 1.055 * powf_approx(c, 1.0/2.4) - 0.055 }
            };
            (gamma(sr), gamma(sg), gamma(sb))
        }
        "xyz-d50" => {
            // D50 → D65 Bradford
            let xd = 0.9555766 * r_f - 0.0230393 * g_f + 0.0631636 * b_f;
            let yd = -0.0282895 * r_f + 1.0099416 * g_f + 0.0210077 * b_f;
            let zd = 0.0122982 * r_f - 0.0204830 * g_f + 1.3299098 * b_f;
            let sr =  3.2404541621 * xd - 1.5371385940 * yd - 0.4985314096 * zd;
            let sg = -0.9692660305 * xd + 1.8760108454 * yd + 0.0415560175 * zd;
            let sb =  0.0556434309 * xd - 0.2040259135 * yd + 1.0572251882 * zd;
            let gamma = |c: f64| -> f64 {
                let c = c.max(0.0).min(1.0);
                if c <= 0.0031308 { c * 12.92 } else { 1.055 * powf_approx(c, 1.0/2.4) - 0.055 }
            };
            (gamma(sr), gamma(sg), gamma(sb))
        }
        _ => return None,
    };
    let ri = (sr.max(0.0).min(1.0) * 255.0 + 0.5) as u32;
    let gi = (sg.max(0.0).min(1.0) * 255.0 + 0.5) as u32;
    let bi = (sb.max(0.0).min(1.0) * 255.0 + 0.5) as u32;
    let a = if let Some(a_str) = alpha_part { parse_alpha_value(a_str) } else { 255 };
    Some((a << 24) | (ri << 16) | (gi << 8) | bi)
}

/// color-mix(in <space>, <color1> [<p1>%], <color2> [<p2>%])
fn parse_color_mix_func(args: &str) -> Option<u32> {
    // Simple implementation: parse "in srgb, <color1> <p1>%, <color2> <p2>%"
    let args = args.trim();
    if !args.starts_with("in ") { return None; }
    let rest = &args[3..];
    // Find the comma after the color space
    let comma1 = rest.find(',')?;
    let _space = rest[..comma1].trim(); // color space (we mix in sRGB for simplicity)
    let rest = rest[comma1 + 1..].trim();
    // Split remaining by comma into two color+percentage parts
    let comma2 = rest.find(',')?;
    let part1 = rest[..comma2].trim();
    let part2 = rest[comma2 + 1..].trim();
    // Parse color and optional percentage from each part
    let (c1, p1) = parse_color_mix_part(part1)?;
    let (c2, p2) = parse_color_mix_part(part2)?;
    // Normalize percentages
    let (p1, p2) = if p1 < 0 && p2 < 0 {
        (50, 50)
    } else if p1 < 0 {
        (100 - p2, p2)
    } else if p2 < 0 {
        (p1, 100 - p1)
    } else {
        (p1, p2)
    };
    let total = (p1 + p2).max(1);
    // Mix in sRGB
    let a1 = (c1 >> 24) & 0xFF;
    let r1 = (c1 >> 16) & 0xFF;
    let g1 = (c1 >> 8) & 0xFF;
    let b1 = c1 & 0xFF;
    let a2 = (c2 >> 24) & 0xFF;
    let r2 = (c2 >> 16) & 0xFF;
    let g2 = (c2 >> 8) & 0xFF;
    let b2 = c2 & 0xFF;
    let mix = |v1: u32, v2: u32| -> u32 {
        ((v1 as i64 * p1 as i64 + v2 as i64 * p2 as i64) / total as i64).max(0).min(255) as u32
    };
    Some((mix(a1, a2) << 24) | (mix(r1, r2) << 16) | (mix(g1, g2) << 8) | mix(b1, b2))
}

fn parse_color_mix_part(s: &str) -> Option<(u32, i32)> {
    let s = s.trim();
    // Try to find a trailing percentage
    let parts: Vec<&str> = s.rsplitn(2, ' ').collect();
    if parts.len() == 2 && parts[0].ends_with('%') {
        let pct_str = &parts[0][..parts[0].len() - 1];
        if let Some(pct) = parse_int(pct_str) {
            let color = try_parse_color(parts[1].trim())?;
            return Some((color, pct));
        }
    }
    // No percentage — parse whole thing as color
    let color = try_parse_color(s)?;
    Some((color, -1)) // -1 = unspecified
}

/// Lab (L: 0-100, a: -125..125, b: -125..125) → sRGB bytes
fn lab_to_srgb(l: f64, a: f64, b: f64) -> (u32, u32, u32) {
    // Lab → XYZ (D50)
    let fy = (l + 16.0) / 116.0;
    let fx = a / 500.0 + fy;
    let fz = fy - b / 200.0;
    let eps = 216.0 / 24389.0;
    let kappa = 24389.0 / 27.0;
    let x = if fx * fx * fx > eps { fx * fx * fx } else { (116.0 * fx - 16.0) / kappa };
    let y = if l > kappa * eps { { let t = (l + 16.0) / 116.0; t * t * t } } else { l / kappa };
    let z = if fz * fz * fz > eps { fz * fz * fz } else { (116.0 * fz - 16.0) / kappa };
    // D50 white point
    let x = x * 0.3457 / 0.3585;
    let z = z * (1.0 - 0.3457 - 0.3585) / 0.3585;
    // D50 → D65 Bradford
    let xd = 0.9555766 * x - 0.0230393 * y + 0.0631636 * z;
    let yd = -0.0282895 * x + 1.0099416 * y + 0.0210077 * z;
    let zd = 0.0122982 * x - 0.0204830 * y + 1.3299098 * z;
    // XYZ-D65 → linear sRGB
    let rl =  3.2404541621 * xd - 1.5371385940 * yd - 0.4985314096 * zd;
    let gl = -0.9692660305 * xd + 1.8760108454 * yd + 0.0415560175 * zd;
    let bl =  0.0556434309 * xd - 0.2040259135 * yd + 1.0572251882 * zd;
    let gamma = |c: f64| -> u32 {
        let c = c.max(0.0).min(1.0);
        let g = if c <= 0.0031308 { c * 12.92 } else { 1.055 * powf_approx(c, 1.0/2.4) - 0.055 };
        (g * 255.0 + 0.5).max(0.0).min(255.0) as u32
    };
    (gamma(rl), gamma(gl), gamma(bl))
}

/// OKLab (L: 0-1, a: ~-0.4..0.4, b: ~-0.4..0.4) → sRGB bytes
fn oklab_to_srgb(l: f64, a: f64, b: f64) -> (u32, u32, u32) {
    // OKLab → LMS (cube root)
    let l_ = l + 0.3963377774 * a + 0.2158037573 * b;
    let m_ = l - 0.1055613458 * a - 0.0638541728 * b;
    let s_ = l - 0.0894841775 * a - 1.2914855480 * b;
    let l3 = l_ * l_ * l_;
    let m3 = m_ * m_ * m_;
    let s3 = s_ * s_ * s_;
    // LMS → linear sRGB
    let rl =  4.0767416621 * l3 - 3.3077115913 * m3 + 0.2309699292 * s3;
    let gl = -1.2684380046 * l3 + 2.6097574011 * m3 - 0.3413193965 * s3;
    let bl = -0.0041960863 * l3 - 0.7034186147 * m3 + 1.7076147010 * s3;
    let gamma = |c: f64| -> u32 {
        let c = c.max(0.0).min(1.0);
        let g = if c <= 0.0031308 { c * 12.92 } else { 1.055 * powf_approx(c, 1.0/2.4) - 0.055 };
        (g * 255.0 + 0.5).max(0.0).min(255.0) as u32
    };
    (gamma(rl), gamma(gl), gamma(bl))
}

