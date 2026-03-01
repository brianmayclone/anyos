// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! libm.so — Hardware-accelerated math library (shared library).
//!
//! Provides IEEE 754-conformant math functions for f64 and f32 types,
//! using x86-64 SSE2 and x87 FPU hardware instructions for maximum
//! performance on x86 systems.
//!
//! This library is pure computation — no heap allocation, no state.
//! The only syscall used is `exit()` for the panic handler.
//!
//! ## Naming Convention
//! - f64 functions: `math_<name>` (e.g. `math_sin`, `math_sqrt`)
//! - f32 functions: `math_<name>f` (e.g. `math_sinf`, `math_sqrtf`)

#![no_std]
#![no_main]

mod f64_ops;
mod f32_ops;

// ── f64 C API exports ────────────────────────────────────────────────

/// Square root (IEEE 754 exact) via SSE2 `sqrtsd`.
#[no_mangle]
pub extern "C" fn math_sqrt(x: f64) -> f64 {
    f64_ops::sqrt(x)
}

/// Sine via x87 `fsin`.
#[no_mangle]
pub extern "C" fn math_sin(x: f64) -> f64 {
    f64_ops::sin(x)
}

/// Cosine via x87 `fcos`.
#[no_mangle]
pub extern "C" fn math_cos(x: f64) -> f64 {
    f64_ops::cos(x)
}

/// Compute sin and cos simultaneously via x87 `fsincos`.
#[no_mangle]
pub extern "C" fn math_sincos(x: f64, out_sin: *mut f64, out_cos: *mut f64) {
    unsafe { f64_ops::sincos(x, &mut *out_sin, &mut *out_cos) }
}

/// Tangent via x87 `fptan`.
#[no_mangle]
pub extern "C" fn math_tan(x: f64) -> f64 {
    f64_ops::tan(x)
}

/// Arcsine: asin(x) = atan2(x, sqrt(1-x²)).
#[no_mangle]
pub extern "C" fn math_asin(x: f64) -> f64 {
    f64_ops::asin(x)
}

/// Arccosine: acos(x) = atan2(sqrt(1-x²), x).
#[no_mangle]
pub extern "C" fn math_acos(x: f64) -> f64 {
    f64_ops::acos(x)
}

/// Arctangent via x87 `fpatan`.
#[no_mangle]
pub extern "C" fn math_atan(x: f64) -> f64 {
    f64_ops::atan(x)
}

/// Two-argument arctangent via x87 `fpatan`.
#[no_mangle]
pub extern "C" fn math_atan2(y: f64, x: f64) -> f64 {
    f64_ops::atan2(y, x)
}

/// Power function x^y via x87 FPU.
#[no_mangle]
pub extern "C" fn math_pow(x: f64, y: f64) -> f64 {
    f64_ops::pow(x, y)
}

/// Exponential e^x via x87.
#[no_mangle]
pub extern "C" fn math_exp(x: f64) -> f64 {
    f64_ops::exp(x)
}

/// Base-2 exponential 2^x via x87.
#[no_mangle]
pub extern "C" fn math_exp2(x: f64) -> f64 {
    f64_ops::exp2(x)
}

/// Natural logarithm ln(x) via x87 `fyl2x`.
#[no_mangle]
pub extern "C" fn math_log(x: f64) -> f64 {
    f64_ops::log(x)
}

/// Base-2 logarithm via x87 `fyl2x`.
#[no_mangle]
pub extern "C" fn math_log2(x: f64) -> f64 {
    f64_ops::log2(x)
}

/// Base-10 logarithm via x87 `fyl2x`.
#[no_mangle]
pub extern "C" fn math_log10(x: f64) -> f64 {
    f64_ops::log10(x)
}

/// Floor (round toward -infinity) via x87 `frndint`.
#[no_mangle]
pub extern "C" fn math_floor(x: f64) -> f64 {
    f64_ops::floor(x)
}

/// Ceiling (round toward +infinity) via x87 `frndint`.
#[no_mangle]
pub extern "C" fn math_ceil(x: f64) -> f64 {
    f64_ops::ceil(x)
}

/// Round to nearest, ties away from zero.
#[no_mangle]
pub extern "C" fn math_round(x: f64) -> f64 {
    f64_ops::round(x)
}

/// Truncate toward zero via IEEE 754 bit manipulation.
#[no_mangle]
pub extern "C" fn math_trunc(x: f64) -> f64 {
    f64_ops::trunc(x)
}

/// Absolute value via SSE2 sign-bit masking.
#[no_mangle]
pub extern "C" fn math_fabs(x: f64) -> f64 {
    f64_ops::fabs(x)
}

/// Floating-point remainder via x87 `fprem`.
#[no_mangle]
pub extern "C" fn math_fmod(x: f64, y: f64) -> f64 {
    f64_ops::fmod(x, y)
}

/// Minimum, with NaN handling.
#[no_mangle]
pub extern "C" fn math_fmin(x: f64, y: f64) -> f64 {
    f64_ops::fmin(x, y)
}

/// Maximum, with NaN handling.
#[no_mangle]
pub extern "C" fn math_fmax(x: f64, y: f64) -> f64 {
    f64_ops::fmax(x, y)
}

/// Hypotenuse sqrt(x² + y²) with overflow protection.
#[no_mangle]
pub extern "C" fn math_hypot(x: f64, y: f64) -> f64 {
    f64_ops::hypot(x, y)
}

/// Copy sign of y onto magnitude of x.
#[no_mangle]
pub extern "C" fn math_copysign(x: f64, y: f64) -> f64 {
    f64_ops::copysign(x, y)
}

/// Load exponent: x * 2^n via x87 `fscale`.
#[no_mangle]
pub extern "C" fn math_ldexp(x: f64, n: i32) -> f64 {
    f64_ops::ldexp(x, n)
}

/// Extract exponent and normalized fraction.
#[no_mangle]
pub extern "C" fn math_frexp(x: f64, exp: *mut i32) -> f64 {
    unsafe { f64_ops::frexp(x, &mut *exp) }
}

/// Cube root via Halley iteration.
#[no_mangle]
pub extern "C" fn math_cbrt(x: f64) -> f64 {
    f64_ops::cbrt(x)
}

// ── f32 C API exports ────────────────────────────────────────────────

/// Square root (f32) via SSE2 `sqrtss`.
#[no_mangle]
pub extern "C" fn math_sqrtf(x: f32) -> f32 {
    f32_ops::sqrtf(x)
}

/// Sine (f32) via x87.
#[no_mangle]
pub extern "C" fn math_sinf(x: f32) -> f32 {
    f32_ops::sinf(x)
}

/// Cosine (f32) via x87.
#[no_mangle]
pub extern "C" fn math_cosf(x: f32) -> f32 {
    f32_ops::cosf(x)
}

/// Compute sin and cos simultaneously (f32) via x87 `fsincos`.
#[no_mangle]
pub extern "C" fn math_sincosf(x: f32, out_sin: *mut f32, out_cos: *mut f32) {
    unsafe { f32_ops::sincosf(x, &mut *out_sin, &mut *out_cos) }
}

/// Tangent (f32) via x87.
#[no_mangle]
pub extern "C" fn math_tanf(x: f32) -> f32 {
    f32_ops::tanf(x)
}

/// Arcsine (f32).
#[no_mangle]
pub extern "C" fn math_asinf(x: f32) -> f32 {
    f32_ops::asinf(x)
}

/// Arccosine (f32).
#[no_mangle]
pub extern "C" fn math_acosf(x: f32) -> f32 {
    f32_ops::acosf(x)
}

/// Arctangent (f32) via x87.
#[no_mangle]
pub extern "C" fn math_atanf(x: f32) -> f32 {
    f32_ops::atanf(x)
}

/// Two-argument arctangent (f32) via x87.
#[no_mangle]
pub extern "C" fn math_atan2f(y: f32, x: f32) -> f32 {
    f32_ops::atan2f(y, x)
}

/// Power function (f32) via x87.
#[no_mangle]
pub extern "C" fn math_powf(x: f32, y: f32) -> f32 {
    f32_ops::powf(x, y)
}

/// Exponential e^x (f32) via x87.
#[no_mangle]
pub extern "C" fn math_expf(x: f32) -> f32 {
    f32_ops::expf(x)
}

/// Base-2 exponential (f32) via x87.
#[no_mangle]
pub extern "C" fn math_exp2f(x: f32) -> f32 {
    f32_ops::exp2f(x)
}

/// Natural logarithm (f32) via x87.
#[no_mangle]
pub extern "C" fn math_logf(x: f32) -> f32 {
    f32_ops::logf(x)
}

/// Base-2 logarithm (f32) via x87.
#[no_mangle]
pub extern "C" fn math_log2f(x: f32) -> f32 {
    f32_ops::log2f(x)
}

/// Base-10 logarithm (f32) via x87.
#[no_mangle]
pub extern "C" fn math_log10f(x: f32) -> f32 {
    f32_ops::log10f(x)
}

/// Floor (f32) via x87.
#[no_mangle]
pub extern "C" fn math_floorf(x: f32) -> f32 {
    f32_ops::floorf(x)
}

/// Ceiling (f32) via x87.
#[no_mangle]
pub extern "C" fn math_ceilf(x: f32) -> f32 {
    f32_ops::ceilf(x)
}

/// Round to nearest (f32), ties away from zero.
#[no_mangle]
pub extern "C" fn math_roundf(x: f32) -> f32 {
    f32_ops::roundf(x)
}

/// Truncate toward zero (f32).
#[no_mangle]
pub extern "C" fn math_truncf(x: f32) -> f32 {
    f32_ops::truncf(x)
}

/// Absolute value (f32).
#[no_mangle]
pub extern "C" fn math_fabsf(x: f32) -> f32 {
    f32_ops::fabsf(x)
}

/// Floating-point remainder (f32) via x87.
#[no_mangle]
pub extern "C" fn math_fmodf(x: f32, y: f32) -> f32 {
    f32_ops::fmodf(x, y)
}

/// Minimum (f32), with NaN handling.
#[no_mangle]
pub extern "C" fn math_fminf(x: f32, y: f32) -> f32 {
    f32_ops::fminf(x, y)
}

/// Maximum (f32), with NaN handling.
#[no_mangle]
pub extern "C" fn math_fmaxf(x: f32, y: f32) -> f32 {
    f32_ops::fmaxf(x, y)
}

/// Hypotenuse (f32) with overflow protection.
#[no_mangle]
pub extern "C" fn math_hypotf(x: f32, y: f32) -> f32 {
    f32_ops::hypotf(x, y)
}

/// Copy sign (f32).
#[no_mangle]
pub extern "C" fn math_copysignf(x: f32, y: f32) -> f32 {
    f32_ops::copysignf(x, y)
}

/// Load exponent (f32): x * 2^n.
#[no_mangle]
pub extern "C" fn math_ldexpf(x: f32, n: i32) -> f32 {
    f32_ops::ldexpf(x, n)
}

/// Extract exponent and normalized fraction (f32).
#[no_mangle]
pub extern "C" fn math_frexpf(x: f32, exp: *mut i32) -> f32 {
    unsafe { f32_ops::frexpf(x, &mut *exp) }
}

/// Cube root (f32) via Halley iteration.
#[no_mangle]
pub extern "C" fn math_cbrtf(x: f32) -> f32 {
    f32_ops::cbrtf(x)
}

// ── Panic handler ────────────────────────────────────────────────────

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    libsyscall::exit(1);
}
