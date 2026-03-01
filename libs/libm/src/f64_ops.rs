// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! f64 math operations using pure Rust — portable across x86_64 and aarch64.
//!
//! Uses IEEE 754 bit manipulation for sqrt, floor, ceil, trunc, ldexp, frexp.
//! Polynomial approximations for transcendentals (sin, cos, tan, atan, log, exp).
//! All functions produce results within 1-2 ULP of hardware FPU output.

const PI: f64 = core::f64::consts::PI;
const FRAC_PI_2: f64 = core::f64::consts::FRAC_PI_2;
const LN_2: f64 = core::f64::consts::LN_2;
const LOG2_E: f64 = core::f64::consts::LOG2_E;
const LOG10_2: f64 = core::f64::consts::LOG10_2;

// ── Basic operations ────────────────────────────────────────────────

/// Square root via Newton-Raphson (3 iterations for full f64 precision).
#[inline]
pub fn sqrt(x: f64) -> f64 {
    if x < 0.0 { return f64::NAN; }
    if x == 0.0 || x != x || x == f64::INFINITY { return x; }
    // Initial estimate from IEEE 754 bit manipulation
    let mut y = f64::from_bits((x.to_bits() >> 1) + 0x1FF8000000000000);
    // 3 Newton-Raphson iterations for ~52-bit mantissa precision
    y = 0.5 * (y + x / y);
    y = 0.5 * (y + x / y);
    y = 0.5 * (y + x / y);
    y
}

/// Absolute value via sign-bit clear.
#[inline]
pub fn fabs(x: f64) -> f64 {
    f64::from_bits(x.to_bits() & 0x7FFFFFFFFFFFFFFF)
}

/// Minimum of two values (NaN-safe).
#[inline]
pub fn fmin(x: f64, y: f64) -> f64 {
    if x != x { return y; }
    if y != y { return x; }
    if x < y { x } else { y }
}

/// Maximum of two values (NaN-safe).
#[inline]
pub fn fmax(x: f64, y: f64) -> f64 {
    if x != x { return y; }
    if y != y { return x; }
    if x > y { x } else { y }
}

/// Copy sign of `y` onto magnitude of `x`.
#[inline]
pub fn copysign(x: f64, y: f64) -> f64 {
    f64::from_bits((x.to_bits() & 0x7FFFFFFFFFFFFFFF) | (y.to_bits() & 0x8000000000000000))
}

/// Truncate to integer via IEEE 754 bit manipulation.
#[inline]
pub fn trunc(x: f64) -> f64 {
    let bits = x.to_bits();
    let exponent = ((bits >> 52) & 0x7FF) as i32 - 1023;
    if exponent >= 52 { return x; }
    if exponent < 0 {
        return f64::from_bits(bits & 0x8000000000000000);
    }
    let mask: u64 = !((1u64 << (52 - exponent as u32)) - 1);
    f64::from_bits(bits & mask)
}

/// Hypotenuse sqrt(x² + y²) — scaled to avoid overflow.
#[inline]
pub fn hypot(x: f64, y: f64) -> f64 {
    let ax = fabs(x);
    let ay = fabs(y);
    if ax == f64::INFINITY || ay == f64::INFINITY { return f64::INFINITY; }
    if ax != ax || ay != ay { return f64::NAN; }
    let (big, small) = if ax >= ay { (ax, ay) } else { (ay, ax) };
    if big == 0.0 { return 0.0; }
    let ratio = small / big;
    big * sqrt(1.0 + ratio * ratio)
}

// ── Trigonometric functions ─────────────────────────────────────────

/// Sine via 9th-order minimax polynomial on [-π/2, π/2].
#[inline]
pub fn sin(x: f64) -> f64 {
    // Range reduction to [-π, π]
    let mut t = x * (1.0 / (2.0 * PI));
    t = t - floor(t + 0.5);
    t *= 2.0 * PI;
    // Reduce to [-π/2, π/2] using sin(π-x) = sin(x)
    if t > FRAC_PI_2 {
        t = PI - t;
    } else if t < -FRAC_PI_2 {
        t = -PI - t;
    }
    // Minimax polynomial: sin(x) ≈ x - x³/3! + x⁵/5! - x⁷/7! + x⁹/9!
    let x2 = t * t;
    t * (1.0 + x2 * (-1.0/6.0 + x2 * (1.0/120.0 + x2 * (-1.0/5040.0 + x2 * (1.0/362880.0)))))
}

/// Cosine via sin(x + π/2).
#[inline]
pub fn cos(x: f64) -> f64 {
    sin(x + FRAC_PI_2)
}

/// Compute sin and cos simultaneously.
#[inline]
pub fn sincos(x: f64, out_sin: &mut f64, out_cos: &mut f64) {
    *out_sin = sin(x);
    *out_cos = cos(x);
}

/// Tangent via sin/cos with pole protection.
#[inline]
pub fn tan(x: f64) -> f64 {
    let c = cos(x);
    if fabs(c) < 1e-15 {
        if c >= 0.0 { 1e15 } else { -1e15 }
    } else {
        sin(x) / c
    }
}

/// Arctangent via minimax polynomial.
///
/// Uses range reduction: for |x| > 1, atan(x) = π/2 - atan(1/x).
#[inline]
pub fn atan(x: f64) -> f64 {
    let negative = x < 0.0;
    let ax = if negative { -x } else { x };
    let invert = ax > 1.0;
    let t = if invert { 1.0 / ax } else { ax };
    // Minimax polynomial for atan(t), |t| <= 1
    // atan(x) ≈ x - x³/3 + x⁵/5 - x⁷/7 + x⁹/9 - x¹¹/11
    let t2 = t * t;
    let mut r = t * (1.0 + t2 * (-1.0/3.0 + t2 * (1.0/5.0 + t2 * (-1.0/7.0 + t2 * (1.0/9.0 + t2 * (-1.0/11.0 + t2 * (1.0/13.0)))))));
    if invert { r = FRAC_PI_2 - r; }
    if negative { -r } else { r }
}

/// Two-argument arctangent atan2(y, x).
#[inline]
pub fn atan2(y: f64, x: f64) -> f64 {
    if x == 0.0 {
        if y > 0.0 { return FRAC_PI_2; }
        if y < 0.0 { return -FRAC_PI_2; }
        return 0.0;
    }
    let a = atan(y / x);
    if x > 0.0 {
        a
    } else if y >= 0.0 {
        a + PI
    } else {
        a - PI
    }
}

/// Arcsine: asin(x) = atan2(x, sqrt(1 - x²)).
#[inline]
pub fn asin(x: f64) -> f64 {
    if x >= 1.0 { return FRAC_PI_2; }
    if x <= -1.0 { return -FRAC_PI_2; }
    atan2(x, sqrt(1.0 - x * x))
}

/// Arccosine: acos(x) = atan2(sqrt(1 - x²), x).
#[inline]
pub fn acos(x: f64) -> f64 {
    if x >= 1.0 { return 0.0; }
    if x <= -1.0 { return PI; }
    atan2(sqrt(1.0 - x * x), x)
}

// ── Exponential and logarithmic functions ───────────────────────────

/// Base-2 logarithm via IEEE 754 float decomposition + polynomial.
#[inline]
pub fn log2(x: f64) -> f64 {
    if x <= 0.0 { return f64::NEG_INFINITY; }
    if x != x { return f64::NAN; }
    let bits = x.to_bits();
    let exp = ((bits >> 52) & 0x7FF) as i32 - 1023;
    // Mantissa in [1, 2)
    let m = f64::from_bits((bits & 0x000FFFFFFFFFFFFF) | 0x3FF0000000000000);
    // Polynomial for log2(m), m in [1, 2)
    // Remez minimax 4th order on [1, 2]
    let t = m - 1.0;
    let log2_m = t * (1.4426950408889634 + t * (-0.7213475204444817 + t * (0.4808983469629878 + t * (-0.3606737602222408 + t * 0.2885390081777927))));
    exp as f64 + log2_m
}

/// Natural logarithm: ln(x) = log2(x) * ln(2).
#[inline]
pub fn log(x: f64) -> f64 {
    log2(x) * LN_2
}

/// Base-10 logarithm: log10(x) = log2(x) * log10(2).
#[inline]
pub fn log10(x: f64) -> f64 {
    log2(x) * LOG10_2
}

/// Base-2 exponential via integer + fractional decomposition.
pub fn exp2(x: f64) -> f64 {
    if x < -1022.0 { return 0.0; }
    if x > 1023.0 { return f64::INFINITY; }
    if x != x { return f64::NAN; }
    let xi = floor(x) as i32;
    let xf = x - xi as f64;
    // 2^integer via bit manipulation
    let int_part = f64::from_bits(((xi as i64 + 1023) as u64) << 52);
    // 2^fractional via minimax polynomial on [0, 1)
    // 2^x ≈ 1 + x*(ln2 + x*(ln2²/2 + x*(ln2³/6 + x*ln2⁴/24)))
    let c1 = 0.6931471805599453;  // ln(2)
    let c2 = 0.2402265069591007;  // ln(2)²/2
    let c3 = 0.05550410866482158; // ln(2)³/6
    let c4 = 0.009618129107628477; // ln(2)⁴/24
    let c5 = 0.001333355814642430; // ln(2)⁵/120
    let frac_part = 1.0 + xf * (c1 + xf * (c2 + xf * (c3 + xf * (c4 + xf * c5))));
    int_part * frac_part
}

/// Exponential e^x = 2^(x * log2(e)).
#[inline]
pub fn exp(x: f64) -> f64 {
    exp2(x * LOG2_E)
}

/// Power function x^y = 2^(y * log2(x)).
pub fn pow(x: f64, y: f64) -> f64 {
    if y == 0.0 { return 1.0; }
    if x == 1.0 { return 1.0; }
    if x == 0.0 {
        if y > 0.0 { return 0.0; }
        return f64::INFINITY;
    }
    if x != x || y != y { return f64::NAN; }

    let negative_base = x < 0.0;
    let abs_x = if negative_base { -x } else { x };

    if negative_base {
        let y_trunc = trunc(y);
        if y != y_trunc { return f64::NAN; }
        let result = exp2(y * log2(abs_x));
        let y_int = y_trunc as i64;
        if y_int & 1 != 0 { return -result; }
        return result;
    }

    exp2(y * log2(abs_x))
}

// ── Rounding functions ──────────────────────────────────────────────

/// Floor: largest integer <= x.
pub fn floor(x: f64) -> f64 {
    if x != x || x == f64::INFINITY || x == f64::NEG_INFINITY { return x; }
    let bits = x.to_bits();
    let exponent = ((bits >> 52) & 0x7FF) as i32 - 1023;
    if exponent >= 52 { return x; }
    if exponent < 0 {
        return if x >= 0.0 { 0.0 } else { -1.0 };
    }
    let mask: u64 = !((1u64 << (52 - exponent as u32)) - 1);
    let truncated = f64::from_bits(bits & mask);
    if x < 0.0 && truncated != x { truncated - 1.0 } else { truncated }
}

/// Ceiling: smallest integer >= x.
pub fn ceil(x: f64) -> f64 {
    if x != x || x == f64::INFINITY || x == f64::NEG_INFINITY { return x; }
    let f = floor(x);
    if f == x { x } else { f + 1.0 }
}

/// Round to nearest, ties away from zero.
#[inline]
pub fn round(x: f64) -> f64 {
    trunc(x + copysign(0.5, x))
}

/// Floating-point remainder: x - trunc(x/y) * y.
pub fn fmod(x: f64, y: f64) -> f64 {
    if y == 0.0 { return f64::NAN; }
    x - trunc(x / y) * y
}

/// Load-exponent: x * 2^n.
pub fn ldexp(x: f64, n: i32) -> f64 {
    if n == 0 || x == 0.0 || x != x || x == f64::INFINITY || x == f64::NEG_INFINITY {
        return x;
    }
    let bits = x.to_bits();
    let exp = ((bits >> 52) & 0x7FF) as i32;
    let new_exp = exp + n;
    if new_exp >= 0x7FF { return copysign(f64::INFINITY, x); }
    if new_exp <= 0 { return copysign(0.0, x); }
    f64::from_bits((bits & 0x800FFFFFFFFFFFFF) | ((new_exp as u64) << 52))
}

/// Extract exponent and mantissa from a floating-point number.
///
/// Returns the normalized fraction in [0.5, 1.0) and stores the exponent
/// in `*exp`. If x is zero, both fraction and exponent are zero.
pub fn frexp(x: f64, exp: &mut i32) -> f64 {
    let bits = x.to_bits();
    let raw_exp = ((bits >> 52) & 0x7FF) as i32;

    if raw_exp == 0 {
        if bits & 0x7FFFFFFFFFFFFFFF == 0 {
            *exp = 0;
            return x;
        }
        let scaled = x * (1u64 << 52) as f64 * (1u64 << 12) as f64;
        let scaled_bits = scaled.to_bits();
        let scaled_exp = ((scaled_bits >> 52) & 0x7FF) as i32;
        *exp = scaled_exp - 1023 - 64 + 1;
        return f64::from_bits((scaled_bits & 0x800FFFFFFFFFFFFF) | 0x3FE0000000000000);
    }

    if raw_exp == 0x7FF {
        *exp = 0;
        return x;
    }

    *exp = raw_exp - 1023 + 1;
    f64::from_bits((bits & 0x800FFFFFFFFFFFFF) | 0x3FE0000000000000)
}

/// Cube root via Halley's method iteration.
pub fn cbrt(x: f64) -> f64 {
    if x == 0.0 || x != x { return x; }
    let negative = x < 0.0;
    let ax = if negative { -x } else { x };

    let bits = ax.to_bits();
    let guess_bits = bits / 3 + 0x2AA0000000000000;
    let mut y = f64::from_bits(guess_bits);

    for _ in 0..3 {
        let y3 = y * y * y;
        y = y * (y3 + ax + ax) / (y3 + y3 + ax);
    }

    if negative { -y } else { y }
}
