//! Math functions for the software rasterizer using pure Rust.
//!
//! Portable implementations that work on both x86_64 and aarch64.
//! Uses bit manipulation for sqrt, polynomial approximations for
//! transcendentals (sin, cos, tan, log2, exp2, pow).

/// Pi constant.
pub const PI: f32 = 3.14159265;

// ── Square root ─────────────────────────────────────────────────────

/// Square root via Newton-Raphson (2 iterations, ~23-bit accuracy).
#[inline]
pub fn sqrt(x: f32) -> f32 {
    if x <= 0.0 { return 0.0; }
    let mut y = f32::from_bits((x.to_bits() >> 1) + 0x1FC00000);
    y = 0.5 * (y + x / y);
    y = 0.5 * (y + x / y);
    y
}

/// Absolute value via sign-bit clear.
#[inline]
pub fn abs(x: f32) -> f32 {
    f32::from_bits(x.to_bits() & 0x7FFFFFFF)
}

// ── Transcendental functions (polynomial approximations) ────────────

/// Sine via 7th-order minimax polynomial on [-π, π].
#[inline]
pub fn sin(x: f32) -> f32 {
    // Range reduction to [-π, π]
    let mut t = x * (1.0 / (2.0 * PI));
    t = t - floor(t + 0.5);
    t *= 2.0 * PI;
    // Coefficients for sin(x) ≈ x - x³/6 + x⁵/120 - x⁷/5040
    let x2 = t * t;
    let x3 = x2 * t;
    let x5 = x3 * x2;
    let x7 = x5 * x2;
    t - x3 * (1.0 / 6.0) + x5 * (1.0 / 120.0) - x7 * (1.0 / 5040.0)
}

/// Cosine via sin(x + π/2).
#[inline]
pub fn cos(x: f32) -> f32 {
    sin(x + PI * 0.5)
}

/// Tangent via sin/cos.
#[inline]
pub fn tan(x: f32) -> f32 {
    let c = cos(x);
    if abs(c) < 1e-10 {
        if c >= 0.0 { 1e10 } else { -1e10 }
    } else {
        sin(x) / c
    }
}

/// Base-2 logarithm via IEEE 754 float decomposition.
#[inline]
pub fn log2(x: f32) -> f32 {
    if x <= 0.0 { return f32::from_bits(0xFF800000); } // -inf
    let bits = x.to_bits();
    let exp = ((bits >> 23) & 0xFF) as i32 - 127;
    // Reconstruct mantissa in [1, 2)
    let m = f32::from_bits((bits & 0x007FFFFF) | 0x3F800000);
    // Polynomial approximation of log2(m) for m in [1, 2)
    // log2(m) ≈ -1.6862 + m*(2.4375 + m*(-0.7515))
    let log2_m = -1.6862 + m * (2.4375 + m * (-0.7515));
    exp as f32 + log2_m
}

/// Base-2 exponential via integer + fractional decomposition.
#[inline]
pub fn exp2(x: f32) -> f32 {
    if x < -126.0 { return 0.0; }
    if x > 127.0 { return f32::from_bits(0x7F800000); } // +inf
    let xi = floor(x) as i32;
    let xf = x - xi as f32;
    // 2^integer part via bit manipulation
    let int_part = f32::from_bits(((xi + 127) as u32) << 23);
    // 2^fractional part via polynomial (minimax on [0, 1])
    // 2^x ≈ 1 + x*(0.6931 + x*(0.2402 + x*0.0554))
    let frac_part = 1.0 + xf * (0.6931 + xf * (0.2402 + xf * 0.0554));
    int_part * frac_part
}

/// Power function x^y via 2^(y * log2(x)).
pub fn pow(base: f32, exp: f32) -> f32 {
    if exp == 0.0 { return 1.0; }
    if base == 1.0 { return 1.0; }
    if base == 0.0 {
        if exp > 0.0 { return 0.0; }
        return f32::from_bits(0x7F800000); // +inf
    }
    if base < 0.0 { return 0.0; } // negative base unsupported for fractional exp
    exp2(exp * log2(base))
}

/// Floor: largest integer <= x.
#[inline]
pub fn floor(x: f32) -> f32 {
    let i = x as i32;
    let fi = i as f32;
    if fi > x { fi - 1.0 } else { fi }
}

/// Ceiling: smallest integer >= x.
#[inline]
pub fn ceil(x: f32) -> f32 {
    let i = x as i32;
    let fi = i as f32;
    if fi < x { fi + 1.0 } else { fi }
}

// ── Pure Rust helpers (no asm needed) ────────────────────────────────

/// Clamp a value to [lo, hi].
#[inline]
pub fn clamp(x: f32, lo: f32, hi: f32) -> f32 {
    if x < lo { lo } else if x > hi { hi } else { x }
}

/// Linear interpolation.
#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
