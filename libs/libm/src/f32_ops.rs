// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! f32 math operations using pure Rust — portable across x86_64 and aarch64.
//!
//! Mirror of f64_ops.rs with f32 types. Uses IEEE 754 bit manipulation
//! and polynomial approximations for all transcendental functions.

const PI: f32 = core::f32::consts::PI;
const FRAC_PI_2: f32 = core::f32::consts::FRAC_PI_2;

// ── Basic operations ────────────────────────────────────────────────

/// Square root via Newton-Raphson (2 iterations for f32 precision).
#[inline]
pub fn sqrtf(x: f32) -> f32 {
    if x < 0.0 { return f32::NAN; }
    if x == 0.0 || x != x || x == f32::INFINITY { return x; }
    let mut y = f32::from_bits((x.to_bits() >> 1) + 0x1FC00000);
    y = 0.5 * (y + x / y);
    y = 0.5 * (y + x / y);
    y
}

/// Absolute value via sign-bit clear.
#[inline]
pub fn fabsf(x: f32) -> f32 {
    f32::from_bits(x.to_bits() & 0x7FFFFFFF)
}

/// Minimum of two values (NaN-safe).
#[inline]
pub fn fminf(x: f32, y: f32) -> f32 {
    if x != x { return y; }
    if y != y { return x; }
    if x < y { x } else { y }
}

/// Maximum of two values (NaN-safe).
#[inline]
pub fn fmaxf(x: f32, y: f32) -> f32 {
    if x != x { return y; }
    if y != y { return x; }
    if x > y { x } else { y }
}

/// Copy sign of `y` onto magnitude of `x`.
#[inline]
pub fn copysignf(x: f32, y: f32) -> f32 {
    f32::from_bits((x.to_bits() & 0x7FFFFFFF) | (y.to_bits() & 0x80000000))
}

/// Truncate to integer via IEEE 754 bit manipulation.
#[inline]
pub fn truncf(x: f32) -> f32 {
    let bits = x.to_bits();
    let exponent = ((bits >> 23) & 0xFF) as i32 - 127;
    if exponent >= 23 { return x; }
    if exponent < 0 {
        return f32::from_bits(bits & 0x80000000);
    }
    let mask: u32 = !((1u32 << (23 - exponent as u32)) - 1);
    f32::from_bits(bits & mask)
}

/// Hypotenuse sqrt(x² + y²) — scaled to avoid overflow.
#[inline]
pub fn hypotf(x: f32, y: f32) -> f32 {
    let ax = fabsf(x);
    let ay = fabsf(y);
    if ax == f32::INFINITY || ay == f32::INFINITY { return f32::INFINITY; }
    if ax != ax || ay != ay { return f32::NAN; }
    let (big, small) = if ax >= ay { (ax, ay) } else { (ay, ax) };
    if big == 0.0 { return 0.0; }
    let ratio = small / big;
    big * sqrtf(1.0 + ratio * ratio)
}

// ── Trigonometric functions ─────────────────────────────────────────

/// Sine via minimax polynomial on [-π/2, π/2].
#[inline]
pub fn sinf(x: f32) -> f32 {
    let mut t = x * (1.0 / (2.0 * PI));
    t = t - floorf(t + 0.5);
    t *= 2.0 * PI;
    if t > FRAC_PI_2 {
        t = PI - t;
    } else if t < -FRAC_PI_2 {
        t = -PI - t;
    }
    let x2 = t * t;
    t * (1.0 + x2 * (-1.0/6.0 + x2 * (1.0/120.0 + x2 * (-1.0/5040.0 + x2 * (1.0/362880.0)))))
}

/// Cosine via sin(x + π/2).
#[inline]
pub fn cosf(x: f32) -> f32 {
    sinf(x + FRAC_PI_2)
}

/// Compute sin and cos simultaneously.
#[inline]
pub fn sincosf(x: f32, out_sin: &mut f32, out_cos: &mut f32) {
    *out_sin = sinf(x);
    *out_cos = cosf(x);
}

/// Tangent via sin/cos.
#[inline]
pub fn tanf(x: f32) -> f32 {
    let c = cosf(x);
    if fabsf(c) < 1e-7 {
        if c >= 0.0 { 1e7 } else { -1e7 }
    } else {
        sinf(x) / c
    }
}

/// Arctangent via minimax polynomial with range reduction.
#[inline]
pub fn atanf(x: f32) -> f32 {
    let negative = x < 0.0;
    let ax = if negative { -x } else { x };
    let invert = ax > 1.0;
    let t = if invert { 1.0 / ax } else { ax };
    let t2 = t * t;
    let mut r = t * (1.0 + t2 * (-1.0/3.0 + t2 * (1.0/5.0 + t2 * (-1.0/7.0 + t2 * (1.0/9.0 + t2 * (-1.0/11.0))))));
    if invert { r = FRAC_PI_2 - r; }
    if negative { -r } else { r }
}

/// Two-argument arctangent.
#[inline]
pub fn atan2f(y: f32, x: f32) -> f32 {
    if x == 0.0 {
        if y > 0.0 { return FRAC_PI_2; }
        if y < 0.0 { return -FRAC_PI_2; }
        return 0.0;
    }
    let a = atanf(y / x);
    if x > 0.0 { a }
    else if y >= 0.0 { a + PI }
    else { a - PI }
}

/// Arcsine: asin(x) = atan2(x, sqrt(1 - x²)).
#[inline]
pub fn asinf(x: f32) -> f32 {
    if x >= 1.0 { return FRAC_PI_2; }
    if x <= -1.0 { return -FRAC_PI_2; }
    atan2f(x, sqrtf(1.0 - x * x))
}

/// Arccosine: acos(x) = atan2(sqrt(1 - x²), x).
#[inline]
pub fn acosf(x: f32) -> f32 {
    if x >= 1.0 { return 0.0; }
    if x <= -1.0 { return PI; }
    atan2f(sqrtf(1.0 - x * x), x)
}

// ── Exponential and logarithmic functions ───────────────────────────

/// Base-2 logarithm via IEEE 754 decomposition + polynomial.
#[inline]
pub fn log2f(x: f32) -> f32 {
    if x <= 0.0 { return f32::NEG_INFINITY; }
    if x != x { return f32::NAN; }
    let bits = x.to_bits();
    let exp = ((bits >> 23) & 0xFF) as i32 - 127;
    let m = f32::from_bits((bits & 0x007FFFFF) | 0x3F800000);
    let t = m - 1.0;
    let log2_m = t * (1.4426950 + t * (-0.7213475 + t * (0.4808983 + t * (-0.3606738))));
    exp as f32 + log2_m
}

/// Natural logarithm.
#[inline]
pub fn logf(x: f32) -> f32 {
    log2f(x) * 0.6931472 // ln(2)
}

/// Base-10 logarithm.
#[inline]
pub fn log10f(x: f32) -> f32 {
    log2f(x) * 0.30103 // log10(2)
}

/// Base-2 exponential.
pub fn exp2f(x: f32) -> f32 {
    if x < -126.0 { return 0.0; }
    if x > 127.0 { return f32::INFINITY; }
    if x != x { return f32::NAN; }
    let xi = floorf(x) as i32;
    let xf = x - xi as f32;
    let int_part = f32::from_bits(((xi + 127) as u32) << 23);
    let frac_part = 1.0 + xf * (0.6931472 + xf * (0.2402265 + xf * (0.05550411 + xf * 0.009618129)));
    int_part * frac_part
}

/// Exponential e^x.
#[inline]
pub fn expf(x: f32) -> f32 {
    exp2f(x * 1.4426950) // log2(e)
}

/// Power function x^y.
pub fn powf(x: f32, y: f32) -> f32 {
    if y == 0.0 { return 1.0; }
    if x == 1.0 { return 1.0; }
    if x == 0.0 {
        if y > 0.0 { return 0.0; }
        return f32::INFINITY;
    }
    if x != x || y != y { return f32::NAN; }

    let negative_base = x < 0.0;
    let abs_x = if negative_base { -x } else { x };

    if negative_base {
        let y_trunc = truncf(y);
        if y != y_trunc { return f32::NAN; }
        let result = exp2f(y * log2f(abs_x));
        let y_int = y_trunc as i32;
        if y_int & 1 != 0 { return -result; }
        return result;
    }

    exp2f(y * log2f(abs_x))
}

// ── Rounding functions ──────────────────────────────────────────────

/// Floor: largest integer <= x.
pub fn floorf(x: f32) -> f32 {
    if x != x || x == f32::INFINITY || x == f32::NEG_INFINITY { return x; }
    let bits = x.to_bits();
    let exponent = ((bits >> 23) & 0xFF) as i32 - 127;
    if exponent >= 23 { return x; }
    if exponent < 0 {
        return if x >= 0.0 { 0.0 } else { -1.0 };
    }
    let mask: u32 = !((1u32 << (23 - exponent as u32)) - 1);
    let truncated = f32::from_bits(bits & mask);
    if x < 0.0 && truncated != x { truncated - 1.0 } else { truncated }
}

/// Ceiling: smallest integer >= x.
pub fn ceilf(x: f32) -> f32 {
    if x != x || x == f32::INFINITY || x == f32::NEG_INFINITY { return x; }
    let f = floorf(x);
    if f == x { x } else { f + 1.0 }
}

/// Round to nearest, ties away from zero.
#[inline]
pub fn roundf(x: f32) -> f32 {
    truncf(x + copysignf(0.5, x))
}

/// Floating-point remainder: x - trunc(x/y) * y.
pub fn fmodf(x: f32, y: f32) -> f32 {
    if y == 0.0 { return f32::NAN; }
    x - truncf(x / y) * y
}

/// Load-exponent: x * 2^n.
pub fn ldexpf(x: f32, n: i32) -> f32 {
    if n == 0 || x == 0.0 || x != x || x == f32::INFINITY || x == f32::NEG_INFINITY {
        return x;
    }
    let bits = x.to_bits();
    let exp = ((bits >> 23) & 0xFF) as i32;
    let new_exp = exp + n;
    if new_exp >= 0xFF { return copysignf(f32::INFINITY, x); }
    if new_exp <= 0 { return copysignf(0.0, x); }
    f32::from_bits((bits & 0x807FFFFF) | ((new_exp as u32) << 23))
}

/// Extract exponent and mantissa.
pub fn frexpf(x: f32, exp: &mut i32) -> f32 {
    let bits = x.to_bits();
    let raw_exp = ((bits >> 23) & 0xFF) as i32;

    if raw_exp == 0 {
        if bits & 0x7FFFFFFF == 0 {
            *exp = 0;
            return x;
        }
        let scaled = x * (1u32 << 23) as f32 * (1u32 << 2) as f32;
        let scaled_bits = scaled.to_bits();
        let scaled_exp = ((scaled_bits >> 23) & 0xFF) as i32;
        *exp = scaled_exp - 127 - 25 + 1;
        return f32::from_bits((scaled_bits & 0x807FFFFF) | 0x3F000000);
    }

    if raw_exp == 0xFF {
        *exp = 0;
        return x;
    }

    *exp = raw_exp - 127 + 1;
    f32::from_bits((bits & 0x807FFFFF) | 0x3F000000)
}

/// Cube root via Halley's method.
pub fn cbrtf(x: f32) -> f32 {
    if x == 0.0 || x != x { return x; }
    let negative = x < 0.0;
    let ax = if negative { -x } else { x };

    let bits = ax.to_bits();
    let guess_bits = bits / 3 + 0x2A555555;
    let mut y = f32::from_bits(guess_bits);

    for _ in 0..2 {
        let y3 = y * y * y;
        y = y * (y3 + ax + ax) / (y3 + y3 + ax);
    }

    if negative { -y } else { y }
}
