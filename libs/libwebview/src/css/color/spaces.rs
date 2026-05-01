fn powf_approx(x: f64, p: f64) -> f64 {
    if x <= 0.0 { return 0.0; }
    if x >= 1.0 && p >= 0.0 { return 1.0_f64.min(exp_approx(p * ln_approx(x))); }
    exp_approx(p * ln_approx(x))
}

fn ln_approx(x: f64) -> f64 {
    if x <= 0.0 { return -1e30; }
    let mut val = x;
    let mut exp = 0i32;
    while val >= 2.0 { val /= 2.0; exp += 1; }
    while val < 0.5 { val *= 2.0; exp -= 1; }
    let u = val - 1.0;
    let ln_val = u * (6.0 + u) / (6.0 + 4.0 * u);
    ln_val + (exp as f64) * 0.6931471805599453
}

fn exp_approx(x: f64) -> f64 {
    if x > 88.0 { return 1e38; }
    if x < -88.0 { return 0.0; }
    let ln2 = 0.6931471805599453;
    let k_raw = x / ln2;
    let k = if k_raw >= 0.0 { (k_raw + 0.5) as i64 as f64 } else { (k_raw - 0.5) as i64 as f64 };
    let r = x - k * ln2;
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

fn cos_f64(x: f64) -> f64 {
    let pi2 = 6.283185307179586;
    let mut a = x % pi2;
    if a < 0.0 { a += pi2; }
    let (a, sign) = if a > 3.141592653589793 { (pi2 - a, 1.0) } else { (a, 1.0) };
    let (a, sign) = if a > 1.5707963267948966 { (3.141592653589793 - a, -sign) } else { (a, sign) };
    let a2 = a * a;
    sign * (1.0 - a2 * (0.5 - a2 * (1.0/24.0 - a2 * (1.0/720.0 - a2 / 40320.0))))
}

fn sin_f64(x: f64) -> f64 {
    cos_f64(x - 1.5707963267948966)
}

fn parse_color_float(s: &str) -> Option<i64> {
    let t = s.trim();
    if t.ends_with('%') {
        let num = &t[..t.len() - 1];
        if num.contains('.') {
            let fp = parse_fixed_point(num)?;
            return Some(fp as i64 * 100);
        }
        return Some(parse_int(num)? as i64 * 10000);
    }
    if t.contains('.') {
        let fp = parse_fixed_point(t)?;
        return Some(fp as i64 * 100);
    }
    Some(parse_int(t)? as i64 * 10000)
}

fn srgb_to_linear(c: f64) -> f64 {
    if c <= 0.04045 { c / 12.92 } else { powf_approx((c + 0.055) / 1.055, 2.4) }
}

fn parse_hwb_func(args: &str) -> Option<u32> {
    let (color_part, alpha_part) = split_color_alpha(args);
    let parts = split_args(color_part);
    if parts.len() < 3 { return None; }
    let h = parse_hue(parts[0])?;
    let w = parse_percent_val(parts[1])?.max(0).min(100) as f64 / 100.0;
    let b = parse_percent_val(parts[2])?.max(0).min(100) as f64 / 100.0;
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

fn parse_lab_func(args: &str) -> Option<u32> {
    let (color_part, alpha_part) = split_color_alpha(args);
    let parts = split_args(color_part);
    if parts.len() < 3 { return None; }
    let l_raw = parse_color_float(parts[0])?;
    let a_raw = parse_color_float(parts[1])?;
    let b_raw = parse_color_float(parts[2])?;
    let l = l_raw as f64 / 10000.0;
    let a_val = if parts[1].trim().ends_with('%') { a_raw as f64 / 10000.0 * 1.25 } else { a_raw as f64 / 10000.0 };
    let b_val = if parts[2].trim().ends_with('%') { b_raw as f64 / 10000.0 * 1.25 } else { b_raw as f64 / 10000.0 };
    let (r, g, b) = lab_to_srgb(l, a_val, b_val);
    let alpha = if let Some(a_str) = alpha_part { parse_alpha_value(a_str) } else { 255 };
    Some((alpha << 24) | (r << 16) | (g << 8) | b)
}

fn parse_lch_func(args: &str) -> Option<u32> {
    let (color_part, alpha_part) = split_color_alpha(args);
    let parts = split_args(color_part);
    if parts.len() < 3 { return None; }
    let l = parse_color_float(parts[0])? as f64 / 10000.0;
    let c = if parts[1].trim().ends_with('%') { parse_color_float(parts[1])? as f64 / 10000.0 * 1.5 } else { parse_color_float(parts[1])? as f64 / 10000.0 };
    let h = parse_hue(parts[2])? as f64;
    let h_rad = h * core::f64::consts::PI / 180.0;
    let a_val = c * cos_f64(h_rad);
    let b_val = c * sin_f64(h_rad);
    let (r, g, b) = lab_to_srgb(l, a_val, b_val);
    let alpha = if let Some(a_str) = alpha_part { parse_alpha_value(a_str) } else { 255 };
    Some((alpha << 24) | (r << 16) | (g << 8) | b)
}

fn parse_oklab_func(args: &str) -> Option<u32> {
    let (color_part, alpha_part) = split_color_alpha(args);
    let parts = split_args(color_part);
    if parts.len() < 3 { return None; }
    let l = if parts[0].trim().ends_with('%') { parse_color_float(parts[0])? as f64 / 10000.0 / 100.0 } else { parse_color_float(parts[0])? as f64 / 10000.0 };
    let a_val = if parts[1].trim().ends_with('%') { parse_color_float(parts[1])? as f64 / 10000.0 / 100.0 * 0.4 } else { parse_color_float(parts[1])? as f64 / 10000.0 };
    let b_val = if parts[2].trim().ends_with('%') { parse_color_float(parts[2])? as f64 / 10000.0 / 100.0 * 0.4 } else { parse_color_float(parts[2])? as f64 / 10000.0 };
    let (r, g, b) = oklab_to_srgb(l, a_val, b_val);
    let alpha = if let Some(a_str) = alpha_part { parse_alpha_value(a_str) } else { 255 };
    Some((alpha << 24) | (r << 16) | (g << 8) | b)
}

fn parse_oklch_func(args: &str) -> Option<u32> {
    let (color_part, alpha_part) = split_color_alpha(args);
    let parts = split_args(color_part);
    if parts.len() < 3 { return None; }
    let l = if parts[0].trim().ends_with('%') { parse_color_float(parts[0])? as f64 / 10000.0 / 100.0 } else { parse_color_float(parts[0])? as f64 / 10000.0 };
    let c = if parts[1].trim().ends_with('%') { parse_color_float(parts[1])? as f64 / 10000.0 / 100.0 * 0.4 } else { parse_color_float(parts[1])? as f64 / 10000.0 };
    let h = parse_hue(parts[2])? as f64;
    let h_rad = h * core::f64::consts::PI / 180.0;
    let a_val = c * cos_f64(h_rad);
    let b_val = c * sin_f64(h_rad);
    let (r, g, b) = oklab_to_srgb(l, a_val, b_val);
    let alpha = if let Some(a_str) = alpha_part { parse_alpha_value(a_str) } else { 255 };
    Some((alpha << 24) | (r << 16) | (g << 8) | b)
}

fn parse_color_func(args: &str) -> Option<u32> {
    let (color_part, alpha_part) = split_color_alpha(args);
    let parts: Vec<&str> = color_part.split_whitespace().collect();
    if parts.len() < 4 { return None; }
    let space = parts[0];
    let r_f = parse_color_float(parts[1])? as f64 / 10000.0;
    let g_f = parse_color_float(parts[2])? as f64 / 10000.0;
    let b_f = parse_color_float(parts[3])? as f64 / 10000.0;
    let (sr, sg, sb) = match space {
        "srgb" => (r_f, g_f, b_f),
        "srgb-linear" => {
            let gamma = |c: f64| -> f64 { if c <= 0.0031308 { c * 12.92 } else { 1.055 * powf_approx(c, 1.0 / 2.4) - 0.055 } };
            (gamma(r_f), gamma(g_f), gamma(b_f))
        }
        "display-p3" => {
            let rl = srgb_to_linear(r_f);
            let gl = srgb_to_linear(g_f);
            let bl = srgb_to_linear(b_f);
            let x = 0.4865709486 * rl + 0.2656676932 * gl + 0.1982172852 * bl;
            let y = 0.2289745641 * rl + 0.6917385218 * gl + 0.0792869141 * bl;
            let z = 0.0000000000 * rl + 0.0451133819 * gl + 0.1043944934 * bl;
            xyz_to_srgb(x, y, z)
        }
        "a98-rgb" => {
            let degamma = |c: f64| -> f64 {
                powf_approx(if c < 0.0 { -c } else { c }, 563.0 / 256.0)
                    * if c < 0.0 { -1.0 } else { 1.0 }
            };
            let rl = degamma(r_f);
            let gl = degamma(g_f);
            let bl = degamma(b_f);
            let x = 0.5766690429 * rl + 0.1855582379 * gl + 0.1882286462 * bl;
            let y = 0.2973449753 * rl + 0.6273635663 * gl + 0.0752914585 * bl;
            let z = 0.0270313614 * rl + 0.0706888525 * gl + 0.9913375368 * bl;
            xyz_to_srgb(x, y, z)
        }
        "prophoto-rgb" => {
            let degamma = |c: f64| -> f64 {
                if (if c < 0.0 { -c } else { c }) <= 16.0 / 512.0 {
                    c / 16.0
                } else {
                    powf_approx(if c < 0.0 { -c } else { c }, 1.8)
                        * if c < 0.0 { -1.0 } else { 1.0 }
                }
            };
            let rl = degamma(r_f);
            let gl = degamma(g_f);
            let bl = degamma(b_f);
            let x = 0.7977604896 * rl + 0.1351917082 * gl + 0.0313493429 * bl;
            let y = 0.2880711282 * rl + 0.7118432178 * gl + 0.0000856540 * bl;
            let z = 0.0000000000 * rl + 0.0000000000 * gl + 0.8251046026 * bl;
            let xd = 0.9555766 * x - 0.0230393 * y + 0.0631636 * z;
            let yd = -0.0282895 * x + 1.0099416 * y + 0.0210077 * z;
            let zd = 0.0122982 * x - 0.0204830 * y + 1.3299098 * z;
            xyz_to_srgb(xd, yd, zd)
        }
        "rec2020" => {
            let alpha_rc = 1.09929682680944;
            let beta_rc = 0.018053968510807;
            let degamma = |c: f64| -> f64 {
                if (if c < 0.0 { -c } else { c }) < beta_rc * 4.5 {
                    c / 4.5
                } else {
                    powf_approx(
                        ((if c < 0.0 { -c } else { c }) + alpha_rc - 1.0) / alpha_rc,
                        1.0 / 0.45,
                    ) * if c < 0.0 { -1.0 } else { 1.0 }
                }
            };
            let rl = degamma(r_f);
            let gl = degamma(g_f);
            let bl = degamma(b_f);
            let x = 0.6369580483 * rl + 0.1446169036 * gl + 0.1688809752 * bl;
            let y = 0.2627002120 * rl + 0.6779980715 * gl + 0.0593017165 * bl;
            let z = 0.0000000000 * rl + 0.0280726930 * gl + 1.0609850577 * bl;
            xyz_to_srgb(x, y, z)
        }
        "xyz" | "xyz-d65" => xyz_to_srgb(r_f, g_f, b_f),
        "xyz-d50" => {
            let xd = 0.9555766 * r_f - 0.0230393 * g_f + 0.0631636 * b_f;
            let yd = -0.0282895 * r_f + 1.0099416 * g_f + 0.0210077 * b_f;
            let zd = 0.0122982 * r_f - 0.0204830 * g_f + 1.3299098 * b_f;
            xyz_to_srgb(xd, yd, zd)
        }
        _ => return None,
    };
    let ri = (sr.max(0.0).min(1.0) * 255.0 + 0.5) as u32;
    let gi = (sg.max(0.0).min(1.0) * 255.0 + 0.5) as u32;
    let bi = (sb.max(0.0).min(1.0) * 255.0 + 0.5) as u32;
    let a = if let Some(a_str) = alpha_part { parse_alpha_value(a_str) } else { 255 };
    Some((a << 24) | (ri << 16) | (gi << 8) | bi)
}

fn xyz_to_srgb(x: f64, y: f64, z: f64) -> (f64, f64, f64) {
    let sr = 3.2404541621 * x - 1.5371385940 * y - 0.4985314096 * z;
    let sg = -0.9692660305 * x + 1.8760108454 * y + 0.0415560175 * z;
    let sb = 0.0556434309 * x - 0.2040259135 * y + 1.0572251882 * z;
    let gamma = |c: f64| -> f64 {
        let c = c.max(0.0).min(1.0);
        if c <= 0.0031308 { c * 12.92 } else { 1.055 * powf_approx(c, 1.0 / 2.4) - 0.055 }
    };
    (gamma(sr), gamma(sg), gamma(sb))
}

fn parse_color_mix_func(args: &str) -> Option<u32> {
    let args = args.trim();
    if !args.starts_with("in ") { return None; }
    let rest = &args[3..];
    let comma1 = rest.find(',')?;
    let rest = rest[comma1 + 1..].trim();
    let comma2 = rest.find(',')?;
    let part1 = rest[..comma2].trim();
    let part2 = rest[comma2 + 1..].trim();
    let (c1, p1) = parse_color_mix_part(part1)?;
    let (c2, p2) = parse_color_mix_part(part2)?;
    let (p1, p2) = if p1 < 0 && p2 < 0 { (50, 50) } else if p1 < 0 { (100 - p2, p2) } else if p2 < 0 { (p1, 100 - p1) } else { (p1, p2) };
    let total = (p1 + p2).max(1);
    let a1 = (c1 >> 24) & 0xFF;
    let r1 = (c1 >> 16) & 0xFF;
    let g1 = (c1 >> 8) & 0xFF;
    let b1 = c1 & 0xFF;
    let a2 = (c2 >> 24) & 0xFF;
    let r2 = (c2 >> 16) & 0xFF;
    let g2 = (c2 >> 8) & 0xFF;
    let b2 = c2 & 0xFF;
    let wa1 = a1 as i64 * p1 as i64;
    let wa2 = a2 as i64 * p2 as i64;
    let out_a = ((wa1 + wa2) / total as i64).max(0).min(255) as u32;
    if out_a == 0 {
        return Some(0);
    }
    let premul_total = (wa1 + wa2).max(1);
    let mix = |v1: u32, v2: u32| -> u32 {
        ((v1 as i64 * wa1 + v2 as i64 * wa2) / premul_total).max(0).min(255) as u32
    };
    Some((out_a << 24) | (mix(r1, r2) << 16) | (mix(g1, g2) << 8) | mix(b1, b2))
}

fn parse_color_mix_part(s: &str) -> Option<(u32, i32)> {
    let s = s.trim();
    let parts: Vec<&str> = s.rsplitn(2, ' ').collect();
    if parts.len() == 2 && parts[0].ends_with('%') {
        let pct_str = &parts[0][..parts[0].len() - 1];
        if let Some(pct) = parse_int(pct_str) {
            let color = try_parse_color(parts[1].trim())?;
            return Some((color, pct));
        }
    }
    let color = try_parse_color(s)?;
    Some((color, -1))
}

fn lab_to_srgb(l: f64, a: f64, b: f64) -> (u32, u32, u32) {
    let fy = (l + 16.0) / 116.0;
    let fx = a / 500.0 + fy;
    let fz = fy - b / 200.0;
    let eps = 216.0 / 24389.0;
    let kappa = 24389.0 / 27.0;
    let x = if fx * fx * fx > eps { fx * fx * fx } else { (116.0 * fx - 16.0) / kappa };
    let y = if l > kappa * eps { let t = (l + 16.0) / 116.0; t * t * t } else { l / kappa };
    let z = if fz * fz * fz > eps { fz * fz * fz } else { (116.0 * fz - 16.0) / kappa };
    let x = x * 0.3457 / 0.3585;
    let z = z * (1.0 - 0.3457 - 0.3585) / 0.3585;
    let xd = 0.9555766 * x - 0.0230393 * y + 0.0631636 * z;
    let yd = -0.0282895 * x + 1.0099416 * y + 0.0210077 * z;
    let zd = 0.0122982 * x - 0.0204830 * y + 1.3299098 * z;
    let rl = 3.2404541621 * xd - 1.5371385940 * yd - 0.4985314096 * zd;
    let gl = -0.9692660305 * xd + 1.8760108454 * yd + 0.0415560175 * zd;
    let bl = 0.0556434309 * xd - 0.2040259135 * yd + 1.0572251882 * zd;
    let gamma = |c: f64| -> u32 {
        let c = c.max(0.0).min(1.0);
        let g = if c <= 0.0031308 { c * 12.92 } else { 1.055 * powf_approx(c, 1.0/2.4) - 0.055 };
        (g * 255.0 + 0.5).max(0.0).min(255.0) as u32
    };
    (gamma(rl), gamma(gl), gamma(bl))
}

fn oklab_to_srgb(l: f64, a: f64, b: f64) -> (u32, u32, u32) {
    let l_ = l + 0.3963377774 * a + 0.2158037573 * b;
    let m_ = l - 0.1055613458 * a - 0.0638541728 * b;
    let s_ = l - 0.0894841775 * a - 1.2914855480 * b;
    let l3 = l_ * l_ * l_;
    let m3 = m_ * m_ * m_;
    let s3 = s_ * s_ * s_;
    let rl = 4.0767416621 * l3 - 3.3077115913 * m3 + 0.2309699292 * s3;
    let gl = -1.2684380046 * l3 + 2.6097574011 * m3 - 0.3413193965 * s3;
    let bl = -0.0041960863 * l3 - 0.7034186147 * m3 + 1.7076147010 * s3;
    let gamma = |c: f64| -> u32 {
        let c = c.max(0.0).min(1.0);
        let g = if c <= 0.0031308 { c * 12.92 } else { 1.055 * powf_approx(c, 1.0/2.4) - 0.055 };
        (g * 255.0 + 0.5).max(0.0).min(255.0) as u32
    };
    (gamma(rl), gamma(gl), gamma(bl))
}

#[cfg(test)]
mod color_mix_tests {
    use super::try_parse_color;

    #[test]
    fn color_mix_with_transparent_preserves_unpremultiplied_rgb() {
        assert_eq!(
            try_parse_color("color-mix(in oklab, #7e14ff 10%, transparent)"),
            Some(0x197e14ff)
        );
        assert_eq!(
            try_parse_color("color-mix(in oklab, #ffffff 5%, transparent)"),
            Some(0x0cffffff)
        );
    }
}
