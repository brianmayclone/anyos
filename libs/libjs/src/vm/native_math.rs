//! Math object methods + no_std math utility functions.

use crate::value::JsValue;
use super::Vm;

// ═══════════════════════════════════════════════════════════
// Math object native methods
// ═══════════════════════════════════════════════════════════

pub fn math_abs(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let n = arg_num(args, 0);
    JsValue::Number(if n < 0.0 { -n } else { n })
}

pub fn math_floor(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    JsValue::Number(floor_f64(arg_num(args, 0)))
}

pub fn math_ceil(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    JsValue::Number(ceil_f64(arg_num(args, 0)))
}

pub fn math_round(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    JsValue::Number(floor_f64(arg_num(args, 0) + 0.5))
}

pub fn math_trunc(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    JsValue::Number(trunc_f64(arg_num(args, 0)))
}

pub fn math_max(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if args.is_empty() { return JsValue::Number(f64::NEG_INFINITY); }
    let mut max = f64::NEG_INFINITY;
    for a in args {
        let n = a.to_number();
        if n.is_nan() { return JsValue::Number(f64::NAN); }
        if n > max { max = n; }
    }
    JsValue::Number(max)
}

pub fn math_min(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if args.is_empty() { return JsValue::Number(f64::INFINITY); }
    let mut min = f64::INFINITY;
    for a in args {
        let n = a.to_number();
        if n.is_nan() { return JsValue::Number(f64::NAN); }
        if n < min { min = n; }
    }
    JsValue::Number(min)
}

pub fn math_pow(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    JsValue::Number(pow_f64(arg_num(args, 0), arg_num(args, 1)))
}

pub fn math_sqrt(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    JsValue::Number(sqrt_f64(arg_num(args, 0)))
}

pub fn math_cbrt(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let n = arg_num(args, 0);
    if n == 0.0 || n.is_nan() || n.is_infinite() { return JsValue::Number(n); }
    let sign = if n < 0.0 { -1.0 } else { 1.0 };
    JsValue::Number(sign * pow_f64(n * sign, 1.0 / 3.0))
}

pub fn math_sign(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let n = arg_num(args, 0);
    if n.is_nan() { JsValue::Number(f64::NAN) }
    else if n > 0.0 { JsValue::Number(1.0) }
    else if n < 0.0 { JsValue::Number(-1.0) }
    else { JsValue::Number(0.0) }
}

pub fn math_log_fn(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    JsValue::Number(ln_approx(arg_num(args, 0)))
}

pub fn math_log2(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    JsValue::Number(ln_approx(arg_num(args, 0)) / core::f64::consts::LN_2)
}

pub fn math_log10(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    JsValue::Number(ln_approx(arg_num(args, 0)) / core::f64::consts::LN_10)
}

pub fn math_sin(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    JsValue::Number(sin_approx(arg_num(args, 0)))
}

pub fn math_cos(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    JsValue::Number(cos_approx(arg_num(args, 0)))
}

pub fn math_tan(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let x = arg_num(args, 0);
    let c = cos_approx(x);
    if c == 0.0 { JsValue::Number(f64::INFINITY) }
    else { JsValue::Number(sin_approx(x) / c) }
}

pub fn math_atan2(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    JsValue::Number(atan2_approx(arg_num(args, 0), arg_num(args, 1)))
}

pub fn math_hypot(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let mut sum = 0.0f64;
    for a in args { sum += a.to_number() * a.to_number(); }
    JsValue::Number(sqrt_f64(sum))
}

pub fn math_clz32(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let n = arg_num(args, 0) as u32;
    JsValue::Number(if n == 0 { 32.0 } else { n.leading_zeros() as f64 })
}

pub fn math_fround(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    JsValue::Number(arg_num(args, 0) as f32 as f64)
}

pub fn math_random(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    static mut SEED: u64 = 12345678901234567;
    unsafe {
        SEED ^= SEED << 13;
        SEED ^= SEED >> 7;
        SEED ^= SEED << 17;
        let val = (SEED & 0x000FFFFFFFFFFFFF) as f64 / (0x0010000000000000u64 as f64);
        JsValue::Number(val)
    }
}

// ═══════════════════════════════════════════════════════════
// no_std math utilities
// ═══════════════════════════════════════════════════════════

fn arg_num(args: &[JsValue], i: usize) -> f64 {
    args.get(i).map(|v| v.to_number()).unwrap_or(f64::NAN)
}

pub fn floor_f64(n: f64) -> f64 {
    if n.is_nan() || n.is_infinite() { return n; }
    let i = n as i64;
    if (i as f64) <= n { i as f64 } else { (i - 1) as f64 }
}

pub fn ceil_f64(n: f64) -> f64 {
    if n.is_nan() || n.is_infinite() { return n; }
    let i = n as i64;
    if (i as f64) >= n { i as f64 } else { (i + 1) as f64 }
}

pub fn trunc_f64(n: f64) -> f64 {
    if n.is_nan() || n.is_infinite() { return n; }
    n as i64 as f64
}

pub fn sqrt_f64(n: f64) -> f64 {
    if n < 0.0 { return f64::NAN; }
    if n == 0.0 || n == 1.0 { return n; }
    if n.is_nan() || n.is_infinite() { return n; }
    let mut x = n / 2.0;
    for _ in 0..64 {
        let next = (x + n / x) / 2.0;
        if (next - x).abs() < 1e-15 { break; }
        x = next;
    }
    x
}

pub fn pow_f64(base: f64, exp: f64) -> f64 {
    if exp == 0.0 { return 1.0; }
    if base == 1.0 { return 1.0; }
    if exp.is_nan() { return f64::NAN; }
    if exp == (exp as i32) as f64 && exp.abs() < 100.0 {
        let n = exp as i32;
        if n >= 0 {
            let mut r = 1.0;
            for _ in 0..n { r *= base; }
            r
        } else {
            let mut r = 1.0;
            for _ in 0..(-n) { r *= base; }
            1.0 / r
        }
    } else {
        if base <= 0.0 { return f64::NAN; }
        exp_approx(exp * ln_approx(base))
    }
}

pub fn ln_approx(x: f64) -> f64 {
    if x <= 0.0 { return f64::NEG_INFINITY; }
    let mut val = x;
    let mut e: f64 = 0.0;
    while val > 2.0 { val /= 2.0; e += 1.0; }
    while val < 0.5 { val *= 2.0; e -= 1.0; }
    let t = (val - 1.0) / (val + 1.0);
    let t2 = t * t;
    let mut sum = t;
    let mut term = t;
    for i in 0..20 {
        term *= t2;
        sum += term / (2 * i + 3) as f64;
    }
    2.0 * sum + e * core::f64::consts::LN_2
}

pub fn exp_approx(x: f64) -> f64 {
    if x > 709.0 { return f64::INFINITY; }
    if x < -709.0 { return 0.0; }
    let ratio = x / core::f64::consts::LN_2;
    let k = floor_f64(ratio + 0.5) as i32;
    let r = x - k as f64 * core::f64::consts::LN_2;
    let mut sum = 1.0;
    let mut term = 1.0;
    for i in 1..30 {
        term *= r / i as f64;
        sum += term;
        if term.abs() < 1e-15 { break; }
    }
    let mut result = sum;
    if k >= 0 { for _ in 0..k.min(1023) { result *= 2.0; } }
    else { for _ in 0..(-k).min(1023) { result /= 2.0; } }
    result
}

fn sin_approx(x: f64) -> f64 {
    if x.is_nan() || x.is_infinite() { return f64::NAN; }
    // Reduce to [-PI, PI]
    let pi = core::f64::consts::PI;
    let mut v = x % (2.0 * pi);
    if v > pi { v -= 2.0 * pi; }
    if v < -pi { v += 2.0 * pi; }
    // Taylor series
    let mut sum = v;
    let mut term = v;
    for i in 1..15 {
        term *= -v * v / ((2 * i) as f64 * (2 * i + 1) as f64);
        sum += term;
    }
    sum
}

fn cos_approx(x: f64) -> f64 {
    sin_approx(x + core::f64::consts::FRAC_PI_2)
}

fn atan2_approx(y: f64, x: f64) -> f64 {
    let pi = core::f64::consts::PI;
    if x == 0.0 {
        return if y > 0.0 { pi / 2.0 } else if y < 0.0 { -pi / 2.0 } else { 0.0 };
    }
    let a = atan_approx(y / x);
    if x > 0.0 { a }
    else if y >= 0.0 { a + pi }
    else { a - pi }
}

fn atan_approx(x: f64) -> f64 {
    if x.is_nan() { return f64::NAN; }
    let pi_2 = core::f64::consts::FRAC_PI_2;
    if x.abs() > 1.0 {
        let sign = if x > 0.0 { 1.0 } else { -1.0 };
        return sign * pi_2 - atan_approx(1.0 / x);
    }
    let mut sum = x;
    let mut term = x;
    let x2 = x * x;
    for i in 1..25 {
        term *= -x2;
        sum += term / (2 * i + 1) as f64;
    }
    sum
}
