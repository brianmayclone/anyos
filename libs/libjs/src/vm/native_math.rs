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
    let n = arg_num(args, 0);
    if n.is_nan() || n.is_infinite() { return JsValue::Number(n); }
    // Preserve -0
    if n == 0.0 { return JsValue::Number(n); }
    // Per spec: for n in [-0.5, 0), return -0 (ties go to +∞, so -0.5 rounds to -0 too)
    if n >= -0.5 && n < 0.0 { return JsValue::Number(-0.0_f64); }
    JsValue::Number(floor_f64(n + 0.5))
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
    let n = arg_num(args, 0);
    // Exact result for exact powers of 2 using IEEE 754 exponent extraction
    if n > 0.0 && n.is_finite() {
        let bits = n.to_bits();
        let mantissa = bits & 0x000F_FFFF_FFFF_FFFF;
        if mantissa == 0 {
            let exp = ((bits >> 52) & 0x7FF) as i32 - 1023;
            return JsValue::Number(exp as f64);
        }
    }
    JsValue::Number(ln_approx(n) / core::f64::consts::LN_2)
}

pub fn math_log10(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let n = arg_num(args, 0);
    // Exact result for exact powers of 10
    if n > 0.0 && n.is_finite() {
        let mut check = 1.0_f64;
        for exp in 0i32..=308 {
            if check == n { return JsValue::Number(exp as f64); }
            if check > n { break; }
            check *= 10.0;
        }
        let mut check = 0.1_f64;
        for exp in 1i32..=323 {
            if check == n { return JsValue::Number(-(exp as f64)); }
            if check < n { break; }
            check /= 10.0;
        }
    }
    JsValue::Number(ln_approx(n) / core::f64::consts::LN_10)
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
    // If any arg is ±Infinity → +Infinity (before checking NaN)
    let mut has_nan = false;
    let mut sum = 0.0f64;
    for a in args {
        let n = a.to_number();
        if n.is_infinite() { return JsValue::Number(f64::INFINITY); }
        if n.is_nan() { has_nan = true; } else { sum += n * n; }
    }
    if has_nan { return JsValue::Number(f64::NAN); }
    JsValue::Number(sqrt_f64(sum))
}

pub fn math_clz32(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    // ToUint32: NaN, ±0, ±Infinity all become 0 → 32 leading zeros
    let n = to_uint32(arg_num(args, 0));
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

pub fn math_exp(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    JsValue::Number(exp_approx(arg_num(args, 0)))
}

pub fn math_expm1(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let x = arg_num(args, 0);
    if x.is_nan() { return JsValue::Number(f64::NAN); }
    // Preserve -0: expm1(-0) = -0
    if x == 0.0 { return JsValue::Number(x); }
    if x.is_infinite() { return JsValue::Number(if x > 0.0 { f64::INFINITY } else { -1.0 }); }
    // For small x use Taylor series for precision: x + x²/2! + x³/3! + ...
    if x.abs() < 1e-4 {
        let x2 = x * x;
        return JsValue::Number(x + x2 / 2.0 + x2 * x / 6.0 + x2 * x2 / 24.0);
    }
    JsValue::Number(exp_approx(x) - 1.0)
}

pub fn math_log1p(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let x = arg_num(args, 0);
    if x.is_nan() { return JsValue::Number(f64::NAN); }
    if x < -1.0 { return JsValue::Number(f64::NAN); }
    if x == -1.0 { return JsValue::Number(f64::NEG_INFINITY); }
    if x.is_infinite() { return JsValue::Number(f64::INFINITY); }
    // For small x use Taylor series: x - x²/2 + x³/3 - x⁴/4 + ...
    if x.abs() < 1e-4 {
        let x2 = x * x;
        return JsValue::Number(x - x2 / 2.0 + x2 * x / 3.0 - x2 * x2 / 4.0);
    }
    JsValue::Number(ln_approx(1.0 + x))
}

pub fn math_asin(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let x = arg_num(args, 0);
    if x.is_nan() || x.abs() > 1.0 { return JsValue::Number(f64::NAN); }
    if x == 1.0 { return JsValue::Number(core::f64::consts::FRAC_PI_2); }
    if x == -1.0 { return JsValue::Number(-core::f64::consts::FRAC_PI_2); }
    if x == 0.0 { return JsValue::Number(0.0); }
    JsValue::Number(atan_approx(x / sqrt_f64(1.0 - x * x)))
}

pub fn math_acos(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let x = arg_num(args, 0);
    if x.is_nan() || x.abs() > 1.0 { return JsValue::Number(f64::NAN); }
    if x == 1.0 { return JsValue::Number(0.0); }
    if x == -1.0 { return JsValue::Number(core::f64::consts::PI); }
    JsValue::Number(core::f64::consts::FRAC_PI_2 - atan_approx(x / sqrt_f64(1.0 - x * x)))
}

pub fn math_atan(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let x = arg_num(args, 0);
    if x.is_nan() { return JsValue::Number(f64::NAN); }
    if x.is_infinite() {
        return JsValue::Number(if x > 0.0 { core::f64::consts::FRAC_PI_2 } else { -core::f64::consts::FRAC_PI_2 });
    }
    JsValue::Number(atan_approx(x))
}

pub fn math_sinh(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let x = arg_num(args, 0);
    if x.is_nan() { return JsValue::Number(f64::NAN); }
    // Preserve -0: sinh(-0) = -0
    if x == 0.0 { return JsValue::Number(x); }
    if x.is_infinite() { return JsValue::Number(x); }
    JsValue::Number((exp_approx(x) - exp_approx(-x)) / 2.0)
}

pub fn math_cosh(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let x = arg_num(args, 0);
    if x.is_nan() { return JsValue::Number(f64::NAN); }
    if x.is_infinite() { return JsValue::Number(f64::INFINITY); }
    JsValue::Number((exp_approx(x) + exp_approx(-x)) / 2.0)
}

pub fn math_tanh(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let x = arg_num(args, 0);
    if x.is_nan() { return JsValue::Number(f64::NAN); }
    if x.is_infinite() { return JsValue::Number(if x > 0.0 { 1.0 } else { -1.0 }); }
    // Preserve -0: tanh(-0) = -0
    if x == 0.0 { return JsValue::Number(x); }
    let ex = exp_approx(x);
    let enx = exp_approx(-x);
    JsValue::Number((ex - enx) / (ex + enx))
}

pub fn math_acosh(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let x = arg_num(args, 0);
    if x.is_nan() || x < 1.0 { return JsValue::Number(f64::NAN); }
    if x == 1.0 { return JsValue::Number(0.0); }
    if x.is_infinite() { return JsValue::Number(f64::INFINITY); }
    JsValue::Number(ln_approx(x + sqrt_f64(x * x - 1.0)))
}

pub fn math_asinh(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let x = arg_num(args, 0);
    if x.is_nan() { return JsValue::Number(f64::NAN); }
    if x.is_infinite() { return JsValue::Number(x); }
    // Preserve -0: asinh(-0) = -0
    if x == 0.0 { return JsValue::Number(x); }
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let ax = x * sign;
    JsValue::Number(sign * ln_approx(ax + sqrt_f64(ax * ax + 1.0)))
}

pub fn math_atanh(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let x = arg_num(args, 0);
    if x.is_nan() || x.abs() > 1.0 { return JsValue::Number(f64::NAN); }
    if x == 1.0 { return JsValue::Number(f64::INFINITY); }
    if x == -1.0 { return JsValue::Number(f64::NEG_INFINITY); }
    // Preserve -0: atanh(-0) = -0
    if x == 0.0 { return JsValue::Number(x); }
    JsValue::Number(ln_approx((1.0 + x) / (1.0 - x)) / 2.0)
}

pub fn math_imul(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    // Spec: ToUint32 each arg, then multiply modulo 2^32, then interpret as i32
    let a = to_uint32(arg_num(args, 0)) as i32;
    let b = to_uint32(arg_num(args, 1)) as i32;
    JsValue::Number(a.wrapping_mul(b) as f64)
}

// ═══════════════════════════════════════════════════════════
// no_std math utilities
// ═══════════════════════════════════════════════════════════

fn arg_num(args: &[JsValue], i: usize) -> f64 {
    args.get(i).map(|v| v.to_number()).unwrap_or(f64::NAN)
}

/// Safe i64 range for float-to-integer conversions (beyond this all floats are already integers).
const I64_MAX_F64: f64 = 9.2233720368547758e18_f64;

pub fn floor_f64(n: f64) -> f64 {
    if n.is_nan() || n.is_infinite() { return n; }
    // Preserve -0
    if n == 0.0 { return n; }
    // Beyond i64 range all floats are exact integers already
    if n >= I64_MAX_F64 || n <= -I64_MAX_F64 { return n; }
    let i = n as i64;
    if (i as f64) <= n { i as f64 } else { (i - 1) as f64 }
}

pub fn ceil_f64(n: f64) -> f64 {
    if n.is_nan() || n.is_infinite() { return n; }
    // Preserve -0
    if n == 0.0 { return n; }
    // For values in (-1, 0): ceil is -0 (same as -Math.floor(-x) rule)
    if n > -1.0 && n < 0.0 { return -0.0_f64; }
    // Beyond i64 range all floats are exact integers already
    if n >= I64_MAX_F64 || n <= -I64_MAX_F64 { return n; }
    let i = n as i64;
    if (i as f64) >= n { i as f64 } else { (i + 1) as f64 }
}

pub fn trunc_f64(n: f64) -> f64 {
    if n.is_nan() || n.is_infinite() { return n; }
    // Preserve -0
    if n == 0.0 { return n; }
    // Values in (-1, 0): trunc toward zero gives -0 (matching Math.ceil for negatives)
    if n > -1.0 && n < 0.0 { return -0.0_f64; }
    // Beyond i64 range all floats are exact integers already
    if n >= I64_MAX_F64 || n <= -I64_MAX_F64 { return n; }
    n as i64 as f64
}

pub fn sqrt_f64(n: f64) -> f64 {
    if n.is_nan() || n < 0.0 { return f64::NAN; }
    if n == 0.0 { return n; } // preserve -0 if ever passed
    if n.is_infinite() { return f64::INFINITY; }
    if n == 1.0 { return 1.0; }
    // Use IEEE 754 bit trick for initial estimate, then Newton-Raphson
    let bits = n.to_bits();
    let exp = (bits >> 52) as i64 - 1023;
    // Initial estimate: 2^(exp/2), accurate to ~50%
    let init_exp = ((exp / 2) + 1023) as u64;
    let mut x = f64::from_bits(init_exp << 52);
    // Newton-Raphson: converges quadratically, ~7 iterations for full f64 precision
    for _ in 0..8 {
        let next = (x + n / x) * 0.5;
        if next == x { break; }
        x = next;
    }
    x
}

pub fn pow_f64(base: f64, exp: f64) -> f64 {
    // ECMAScript spec section 21.3.2.26 / applying-the-exp-operator rules:

    // If exp is ±0, result is always 1 (even for NaN base)
    if exp == 0.0 { return 1.0; }
    // If exp is NaN, result is NaN
    if exp.is_nan() { return f64::NAN; }
    // If base is NaN, result is NaN
    if base.is_nan() { return f64::NAN; }

    let abs_base = base.abs();

    // Infinite exponent rules
    if exp.is_infinite() {
        // |base| == 1 with infinite exp → NaN
        if abs_base == 1.0 { return f64::NAN; }
        if exp > 0.0 {
            return if abs_base > 1.0 { f64::INFINITY } else { 0.0 };
        } else {
            return if abs_base > 1.0 { 0.0 } else { f64::INFINITY };
        }
    }

    // Infinite base rules
    if base.is_infinite() {
        if base > 0.0 {
            // +Infinity
            return if exp > 0.0 { f64::INFINITY } else { 0.0 };
        } else {
            // -Infinity
            let odd = is_odd_integer(exp);
            if exp > 0.0 {
                return if odd { f64::NEG_INFINITY } else { f64::INFINITY };
            } else {
                return if odd { -0.0_f64 } else { 0.0 };
            }
        }
    }

    // Zero base rules (including -0)
    if base == 0.0 {
        let neg_base = base.is_sign_negative();
        let odd = is_odd_integer(exp);
        if exp > 0.0 {
            return if neg_base && odd { -0.0_f64 } else { 0.0 };
        } else {
            // exp < 0
            return if neg_base && odd { f64::NEG_INFINITY } else { f64::INFINITY };
        }
    }

    // Negative base with non-integer exponent → NaN
    if base < 0.0 {
        if !is_integer_finite(exp) { return f64::NAN; }
        let abs_result = pow_positive(-base, exp.abs());
        let negate = is_odd_integer(exp);
        let result = if negate { -abs_result } else { abs_result };
        return if exp < 0.0 { 1.0 / result } else { result };
    }

    // Normal positive base
    pow_positive(base, exp)
}

fn is_integer_finite(n: f64) -> bool {
    n.is_finite() && n == floor_f64(n)
}

fn is_odd_integer(n: f64) -> bool {
    is_integer_finite(n) && {
        // n % 2 != 0 but safe for large values
        let half = n / 2.0;
        half != floor_f64(half)
    }
}

fn pow_positive(base: f64, exp: f64) -> f64 {
    // Integer exponent fast path (for small exponents)
    if exp == floor_f64(exp) && exp.abs() < 1000.0 {
        let n = exp as i64;
        if n >= 0 {
            let mut r = 1.0_f64;
            for _ in 0..n { r *= base; }
            r
        } else {
            let mut r = 1.0_f64;
            for _ in 0..(-n) { r *= base; }
            1.0 / r
        }
    } else {
        exp_approx(exp * ln_approx(base))
    }
}

pub fn ln_approx(x: f64) -> f64 {
    if x.is_nan() { return f64::NAN; }
    if x < 0.0 { return f64::NAN; }
    if x == 0.0 { return f64::NEG_INFINITY; }
    if x.is_infinite() { return f64::INFINITY; }
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
    if x.is_nan() || x.is_infinite() { return f64::NAN; }
    // cos(0) = 1 exactly
    if x == 0.0 { return 1.0; }
    sin_approx(x + core::f64::consts::FRAC_PI_2)
}

fn atan2_approx(y: f64, x: f64) -> f64 {
    // NaN in either arg → NaN
    if y.is_nan() || x.is_nan() { return f64::NAN; }
    let pi = core::f64::consts::PI;
    // Handle infinities per spec
    if y.is_infinite() && x.is_infinite() {
        return if y > 0.0 {
            if x > 0.0 { pi / 4.0 } else { 3.0 * pi / 4.0 }
        } else {
            if x > 0.0 { -pi / 4.0 } else { -3.0 * pi / 4.0 }
        };
    }
    if x.is_infinite() {
        return if x > 0.0 {
            if y.is_sign_negative() { -0.0_f64 } else { 0.0 }
        } else {
            if y < 0.0 { -pi } else { pi }
        };
    }
    if y.is_infinite() {
        return if y > 0.0 { pi / 2.0 } else { -pi / 2.0 };
    }
    if x == 0.0 {
        if y == 0.0 {
            return if x.is_sign_negative() {
                if y.is_sign_negative() { -pi } else { pi }
            } else {
                if y.is_sign_negative() { -0.0_f64 } else { 0.0 }
            };
        }
        return if y > 0.0 { pi / 2.0 } else { -pi / 2.0 };
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

/// Convert f64 to u32 following ECMAScript ToUint32 semantics.
/// NaN, ±0, ±Infinity all map to 0.
pub fn to_uint32(n: f64) -> u32 {
    if !n.is_finite() || n == 0.0 { return 0; }
    // Truncate to integer, then take modulo 2^32
    let trunc = n as i64;  // truncates toward zero
    trunc as u32
}
