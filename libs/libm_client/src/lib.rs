// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Client library for libm.so — hardware-accelerated math functions.
//!
//! Provides safe Rust wrappers around libm's exported symbols,
//! resolved at runtime via `dl_open` / `dl_sym` (ELF dynamic linking).
//!
//! # Usage
//! ```rust
//! libm_client::init(); // load libm.so once at startup
//! let x = libm_client::sin(1.0);
//! let y = libm_client::sqrtf(2.0f32);
//! ```

#![no_std]

dynlink::dll_exports! {
    lib_path: "/Libraries/libm.so",
    lib_struct: MathLib,
    symbols: {
        // f64 functions
        math_sqrt(x: f64) -> f64,
        math_sin(x: f64) -> f64,
        math_cos(x: f64) -> f64,
        math_sincos(x: f64, s: *mut f64, c: *mut f64) -> (),
        math_tan(x: f64) -> f64,
        math_asin(x: f64) -> f64,
        math_acos(x: f64) -> f64,
        math_atan(x: f64) -> f64,
        math_atan2(y: f64, x: f64) -> f64,
        math_pow(x: f64, y: f64) -> f64,
        math_exp(x: f64) -> f64,
        math_exp2(x: f64) -> f64,
        math_log(x: f64) -> f64,
        math_log2(x: f64) -> f64,
        math_log10(x: f64) -> f64,
        math_floor(x: f64) -> f64,
        math_ceil(x: f64) -> f64,
        math_round(x: f64) -> f64,
        math_trunc(x: f64) -> f64,
        math_fabs(x: f64) -> f64,
        math_fmod(x: f64, y: f64) -> f64,
        math_fmin(x: f64, y: f64) -> f64,
        math_fmax(x: f64, y: f64) -> f64,
        math_hypot(x: f64, y: f64) -> f64,
        math_copysign(x: f64, y: f64) -> f64,
        math_ldexp(x: f64, n: i32) -> f64,
        math_frexp(x: f64, exp: *mut i32) -> f64,
        math_cbrt(x: f64) -> f64,
        // f32 functions
        math_sqrtf(x: f32) -> f32,
        math_sinf(x: f32) -> f32,
        math_cosf(x: f32) -> f32,
        math_sincosf(x: f32, s: *mut f32, c: *mut f32) -> (),
        math_tanf(x: f32) -> f32,
        math_asinf(x: f32) -> f32,
        math_acosf(x: f32) -> f32,
        math_atanf(x: f32) -> f32,
        math_atan2f(y: f32, x: f32) -> f32,
        math_powf(x: f32, y: f32) -> f32,
        math_expf(x: f32) -> f32,
        math_exp2f(x: f32) -> f32,
        math_logf(x: f32) -> f32,
        math_log2f(x: f32) -> f32,
        math_log10f(x: f32) -> f32,
        math_floorf(x: f32) -> f32,
        math_ceilf(x: f32) -> f32,
        math_roundf(x: f32) -> f32,
        math_truncf(x: f32) -> f32,
        math_fabsf(x: f32) -> f32,
        math_fmodf(x: f32, y: f32) -> f32,
        math_fminf(x: f32, y: f32) -> f32,
        math_fmaxf(x: f32, y: f32) -> f32,
        math_hypotf(x: f32, y: f32) -> f32,
        math_copysignf(x: f32, y: f32) -> f32,
        math_ldexpf(x: f32, n: i32) -> f32,
        math_frexpf(x: f32, exp: *mut i32) -> f32,
        math_cbrtf(x: f32) -> f32,
    }
}

// ── f64 wrappers ─────────────────────────────────────────────────────

/// Square root (IEEE 754 exact).
pub fn sqrt(x: f64) -> f64 { (lib().math_sqrt)(x) }

/// Sine.
pub fn sin(x: f64) -> f64 { (lib().math_sin)(x) }

/// Cosine.
pub fn cos(x: f64) -> f64 { (lib().math_cos)(x) }

/// Compute sin and cos simultaneously (more efficient than separate calls).
pub fn sincos(x: f64) -> (f64, f64) {
    let mut s: f64 = 0.0;
    let mut c: f64 = 0.0;
    (lib().math_sincos)(x, &mut s, &mut c);
    (s, c)
}

/// Tangent.
pub fn tan(x: f64) -> f64 { (lib().math_tan)(x) }

/// Arcsine.
pub fn asin(x: f64) -> f64 { (lib().math_asin)(x) }

/// Arccosine.
pub fn acos(x: f64) -> f64 { (lib().math_acos)(x) }

/// Arctangent.
pub fn atan(x: f64) -> f64 { (lib().math_atan)(x) }

/// Two-argument arctangent: angle of point (x, y).
pub fn atan2(y: f64, x: f64) -> f64 { (lib().math_atan2)(y, x) }

/// Power: x raised to y.
pub fn pow(x: f64, y: f64) -> f64 { (lib().math_pow)(x, y) }

/// Exponential: e^x.
pub fn exp(x: f64) -> f64 { (lib().math_exp)(x) }

/// Base-2 exponential: 2^x.
pub fn exp2(x: f64) -> f64 { (lib().math_exp2)(x) }

/// Natural logarithm: ln(x).
pub fn log(x: f64) -> f64 { (lib().math_log)(x) }

/// Base-2 logarithm.
pub fn log2(x: f64) -> f64 { (lib().math_log2)(x) }

/// Base-10 logarithm.
pub fn log10(x: f64) -> f64 { (lib().math_log10)(x) }

/// Floor: largest integer <= x.
pub fn floor(x: f64) -> f64 { (lib().math_floor)(x) }

/// Ceiling: smallest integer >= x.
pub fn ceil(x: f64) -> f64 { (lib().math_ceil)(x) }

/// Round to nearest integer, ties away from zero.
pub fn round(x: f64) -> f64 { (lib().math_round)(x) }

/// Truncate toward zero.
pub fn trunc(x: f64) -> f64 { (lib().math_trunc)(x) }

/// Absolute value.
pub fn fabs(x: f64) -> f64 { (lib().math_fabs)(x) }

/// Floating-point remainder: x - trunc(x/y) * y.
pub fn fmod(x: f64, y: f64) -> f64 { (lib().math_fmod)(x, y) }

/// Minimum of two values (NaN-safe).
pub fn fmin(x: f64, y: f64) -> f64 { (lib().math_fmin)(x, y) }

/// Maximum of two values (NaN-safe).
pub fn fmax(x: f64, y: f64) -> f64 { (lib().math_fmax)(x, y) }

/// Hypotenuse: sqrt(x² + y²) with overflow protection.
pub fn hypot(x: f64, y: f64) -> f64 { (lib().math_hypot)(x, y) }

/// Copy sign of y onto magnitude of x.
pub fn copysign(x: f64, y: f64) -> f64 { (lib().math_copysign)(x, y) }

/// Load exponent: x * 2^n.
pub fn ldexp(x: f64, n: i32) -> f64 { (lib().math_ldexp)(x, n) }

/// Extract normalized fraction and exponent: x = frac * 2^exp.
pub fn frexp(x: f64) -> (f64, i32) {
    let mut exp: i32 = 0;
    let frac = (lib().math_frexp)(x, &mut exp);
    (frac, exp)
}

/// Cube root.
pub fn cbrt(x: f64) -> f64 { (lib().math_cbrt)(x) }

// ── f32 wrappers ─────────────────────────────────────────────────────

/// Square root (f32, IEEE 754 exact).
pub fn sqrtf(x: f32) -> f32 { (lib().math_sqrtf)(x) }

/// Sine (f32).
pub fn sinf(x: f32) -> f32 { (lib().math_sinf)(x) }

/// Cosine (f32).
pub fn cosf(x: f32) -> f32 { (lib().math_cosf)(x) }

/// Compute sin and cos simultaneously (f32).
pub fn sincosf(x: f32) -> (f32, f32) {
    let mut s: f32 = 0.0;
    let mut c: f32 = 0.0;
    (lib().math_sincosf)(x, &mut s, &mut c);
    (s, c)
}

/// Tangent (f32).
pub fn tanf(x: f32) -> f32 { (lib().math_tanf)(x) }

/// Arcsine (f32).
pub fn asinf(x: f32) -> f32 { (lib().math_asinf)(x) }

/// Arccosine (f32).
pub fn acosf(x: f32) -> f32 { (lib().math_acosf)(x) }

/// Arctangent (f32).
pub fn atanf(x: f32) -> f32 { (lib().math_atanf)(x) }

/// Two-argument arctangent (f32).
pub fn atan2f(y: f32, x: f32) -> f32 { (lib().math_atan2f)(y, x) }

/// Power (f32).
pub fn powf(x: f32, y: f32) -> f32 { (lib().math_powf)(x, y) }

/// Exponential e^x (f32).
pub fn expf(x: f32) -> f32 { (lib().math_expf)(x) }

/// Base-2 exponential (f32).
pub fn exp2f(x: f32) -> f32 { (lib().math_exp2f)(x) }

/// Natural logarithm (f32).
pub fn logf(x: f32) -> f32 { (lib().math_logf)(x) }

/// Base-2 logarithm (f32).
pub fn log2f(x: f32) -> f32 { (lib().math_log2f)(x) }

/// Base-10 logarithm (f32).
pub fn log10f(x: f32) -> f32 { (lib().math_log10f)(x) }

/// Floor (f32).
pub fn floorf(x: f32) -> f32 { (lib().math_floorf)(x) }

/// Ceiling (f32).
pub fn ceilf(x: f32) -> f32 { (lib().math_ceilf)(x) }

/// Round to nearest (f32), ties away from zero.
pub fn roundf(x: f32) -> f32 { (lib().math_roundf)(x) }

/// Truncate toward zero (f32).
pub fn truncf(x: f32) -> f32 { (lib().math_truncf)(x) }

/// Absolute value (f32).
pub fn fabsf(x: f32) -> f32 { (lib().math_fabsf)(x) }

/// Floating-point remainder (f32).
pub fn fmodf(x: f32, y: f32) -> f32 { (lib().math_fmodf)(x, y) }

/// Minimum (f32, NaN-safe).
pub fn fminf(x: f32, y: f32) -> f32 { (lib().math_fminf)(x, y) }

/// Maximum (f32, NaN-safe).
pub fn fmaxf(x: f32, y: f32) -> f32 { (lib().math_fmaxf)(x, y) }

/// Hypotenuse (f32) with overflow protection.
pub fn hypotf(x: f32, y: f32) -> f32 { (lib().math_hypotf)(x, y) }

/// Copy sign (f32).
pub fn copysignf(x: f32, y: f32) -> f32 { (lib().math_copysignf)(x, y) }

/// Load exponent (f32): x * 2^n.
pub fn ldexpf(x: f32, n: i32) -> f32 { (lib().math_ldexpf)(x, n) }

/// Extract normalized fraction and exponent (f32).
pub fn frexpf(x: f32) -> (f32, i32) {
    let mut exp: i32 = 0;
    let frac = (lib().math_frexpf)(x, &mut exp);
    (frac, exp)
}

/// Cube root (f32).
pub fn cbrtf(x: f32) -> f32 { (lib().math_cbrtf)(x) }
