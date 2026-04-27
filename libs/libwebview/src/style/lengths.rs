use crate::css::{CssValue, Unit};

use super::ComputedStyle;

static mut VIEWPORT_W: i32 = 800;
static mut VIEWPORT_H: i32 = 600;

pub(super) fn set_viewport_size(width: i32, height: i32) {
    unsafe {
        VIEWPORT_W = width;
        VIEWPORT_H = height;
    }
}

pub(super) fn resolve_length(val: &CssValue, parent_fs: i32, root_fs: i32) -> Option<i32> {
    match val {
        CssValue::Length(v, Unit::Px) => Some(v / 100),
        CssValue::Length(v, Unit::Em) => Some(v * parent_fs / 100),
        CssValue::Length(v, Unit::Rem) => Some(v * root_fs / 100),
        CssValue::Length(v, Unit::In) => Some(v * 96 / 100),
        CssValue::Length(v, Unit::Cm) => Some(v * 9600 / 25400),
        CssValue::Length(v, Unit::Mm) => Some(v * 960 / 2540),
        CssValue::Length(v, Unit::Pt) => Some(v * 96 / 7200),
        CssValue::Length(v, Unit::Pc) => Some(v * 16 / 100),
        CssValue::Length(v, Unit::Q) => Some(v * 96 / 10160),
        CssValue::Length(_, Unit::Percent) => Option::None,
        CssValue::Length(_, Unit::Fr) => Option::None,
        CssValue::Length(v, Unit::Vw) => {
            let vw = unsafe { VIEWPORT_W };
            Some((*v as i64 * vw as i64 / 10000) as i32)
        }
        CssValue::Length(v, Unit::Vh) => {
            let vh = unsafe { VIEWPORT_H };
            Some((*v as i64 * vh as i64 / 10000) as i32)
        }
        CssValue::Length(v, Unit::Vmin) => {
            let dim = unsafe { VIEWPORT_W.min(VIEWPORT_H) };
            Some((*v as i64 * dim as i64 / 10000) as i32)
        }
        CssValue::Length(v, Unit::Vmax) => {
            let dim = unsafe { VIEWPORT_W.max(VIEWPORT_H) };
            Some((*v as i64 * dim as i64 / 10000) as i32)
        }
        CssValue::Number(v) => Some(v / 100),
        CssValue::Percentage(_) => Option::None,
        CssValue::Calc(px, _pct) => Some(px / 100),
        _ => Option::None,
    }
}

fn resolve_margin_calc(calc: (i32, i32), containing_width: i32) -> i32 {
    calc.0 / 100 + (containing_width as i64 * calc.1 as i64 / 10000) as i32
}

pub fn resolve_inset(
    value: Option<i32>,
    calc: Option<(i32, i32)>,
    containing_size: i32,
    has_definite_size: bool,
) -> Option<i32> {
    if let Some((px, pct)) = calc {
        if pct != 0 && !has_definite_size {
            return None;
        }
        return Some(px / 100 + (containing_size as i64 * pct as i64 / 10000) as i32);
    }
    value
}

pub fn resolve_margins(style: &ComputedStyle, containing_width: i32) -> (i32, i32, i32, i32) {
    let mt = style
        .margin_top_calc
        .map(|calc| resolve_margin_calc(calc, containing_width))
        .unwrap_or(style.margin_top);
    let mr = style
        .margin_right_calc
        .map(|calc| resolve_margin_calc(calc, containing_width))
        .unwrap_or(style.margin_right);
    let mb = style
        .margin_bottom_calc
        .map(|calc| resolve_margin_calc(calc, containing_width))
        .unwrap_or(style.margin_bottom);
    let ml = style
        .margin_left_calc
        .map(|calc| resolve_margin_calc(calc, containing_width))
        .unwrap_or(style.margin_left);
    (mt, mr, mb, ml)
}

pub(super) fn parse_transform_length(s: &str, parent_fs: i32) -> i32 {
    let s = s.trim();
    if s.ends_with("px") {
        let num = &s[..s.len() - 2];
        parse_simple_float(num)
    } else if s.ends_with("em") {
        let num = &s[..s.len() - 2];
        let v = parse_simple_float(num);
        v * parent_fs / 100
    } else if s.ends_with("rem") {
        let num = &s[..s.len() - 3];
        let v = parse_simple_float(num);
        v * 16 / 100
    } else if s.ends_with('%') {
        0
    } else {
        parse_simple_float(s)
    }
}

pub(super) fn parse_simple_float(s: &str) -> i32 {
    let s = s.trim();
    let neg = s.starts_with('-');
    let s = if neg { &s[1..] } else { s };
    let mut int_part = 0i32;
    let mut frac = 0i32;
    let mut in_frac = false;
    let mut frac_mul = 10;
    for &b in s.as_bytes() {
        if b == b'.' {
            in_frac = true;
        } else if b.is_ascii_digit() {
            if in_frac {
                if frac_mul <= 100 {
                    frac += (b - b'0') as i32 * (100 / frac_mul);
                    frac_mul *= 10;
                }
            } else {
                int_part = int_part * 10 + (b - b'0') as i32;
            }
        }
    }
    if neg {
        -int_part
    } else {
        int_part
    }
}
