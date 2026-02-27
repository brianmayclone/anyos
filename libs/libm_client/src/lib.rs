// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Client library for libm.so — hardware-accelerated math functions.
//!
//! Provides safe Rust wrappers around libm's exported symbols,
//! resolved at runtime via `dl_open` / `dl_sym` (ELF dynamic linking).
//!
//! # Usage
//! ```rust
//! libm_client::load(); // load libm.so once at startup
//! let x = libm_client::sin(1.0);
//! let y = libm_client::sqrtf(2.0f32);
//! ```

#![no_std]

use dynlink::{DlHandle, dl_open, dl_sym};

/// Function pointer table for all 58 libm exports.
struct MathLib {
    _handle: DlHandle,
    // f64 functions
    sqrt_fn: extern "C" fn(f64) -> f64,
    sin_fn: extern "C" fn(f64) -> f64,
    cos_fn: extern "C" fn(f64) -> f64,
    sincos_fn: extern "C" fn(f64, *mut f64, *mut f64),
    tan_fn: extern "C" fn(f64) -> f64,
    asin_fn: extern "C" fn(f64) -> f64,
    acos_fn: extern "C" fn(f64) -> f64,
    atan_fn: extern "C" fn(f64) -> f64,
    atan2_fn: extern "C" fn(f64, f64) -> f64,
    pow_fn: extern "C" fn(f64, f64) -> f64,
    exp_fn: extern "C" fn(f64) -> f64,
    exp2_fn: extern "C" fn(f64) -> f64,
    log_fn: extern "C" fn(f64) -> f64,
    log2_fn: extern "C" fn(f64) -> f64,
    log10_fn: extern "C" fn(f64) -> f64,
    floor_fn: extern "C" fn(f64) -> f64,
    ceil_fn: extern "C" fn(f64) -> f64,
    round_fn: extern "C" fn(f64) -> f64,
    trunc_fn: extern "C" fn(f64) -> f64,
    fabs_fn: extern "C" fn(f64) -> f64,
    fmod_fn: extern "C" fn(f64, f64) -> f64,
    fmin_fn: extern "C" fn(f64, f64) -> f64,
    fmax_fn: extern "C" fn(f64, f64) -> f64,
    hypot_fn: extern "C" fn(f64, f64) -> f64,
    copysign_fn: extern "C" fn(f64, f64) -> f64,
    ldexp_fn: extern "C" fn(f64, i32) -> f64,
    frexp_fn: extern "C" fn(f64, *mut i32) -> f64,
    cbrt_fn: extern "C" fn(f64) -> f64,
    // f32 functions
    sqrtf_fn: extern "C" fn(f32) -> f32,
    sinf_fn: extern "C" fn(f32) -> f32,
    cosf_fn: extern "C" fn(f32) -> f32,
    sincosf_fn: extern "C" fn(f32, *mut f32, *mut f32),
    tanf_fn: extern "C" fn(f32) -> f32,
    asinf_fn: extern "C" fn(f32) -> f32,
    acosf_fn: extern "C" fn(f32) -> f32,
    atanf_fn: extern "C" fn(f32) -> f32,
    atan2f_fn: extern "C" fn(f32, f32) -> f32,
    powf_fn: extern "C" fn(f32, f32) -> f32,
    expf_fn: extern "C" fn(f32) -> f32,
    exp2f_fn: extern "C" fn(f32) -> f32,
    logf_fn: extern "C" fn(f32) -> f32,
    log2f_fn: extern "C" fn(f32) -> f32,
    log10f_fn: extern "C" fn(f32) -> f32,
    floorf_fn: extern "C" fn(f32) -> f32,
    ceilf_fn: extern "C" fn(f32) -> f32,
    roundf_fn: extern "C" fn(f32) -> f32,
    truncf_fn: extern "C" fn(f32) -> f32,
    fabsf_fn: extern "C" fn(f32) -> f32,
    fmodf_fn: extern "C" fn(f32, f32) -> f32,
    fminf_fn: extern "C" fn(f32, f32) -> f32,
    fmaxf_fn: extern "C" fn(f32, f32) -> f32,
    hypotf_fn: extern "C" fn(f32, f32) -> f32,
    copysignf_fn: extern "C" fn(f32, f32) -> f32,
    ldexpf_fn: extern "C" fn(f32, i32) -> f32,
    frexpf_fn: extern "C" fn(f32, *mut i32) -> f32,
    cbrtf_fn: extern "C" fn(f32) -> f32,
}

static mut LIB: Option<MathLib> = None;

fn lib() -> &'static MathLib {
    unsafe { LIB.as_ref().expect("libm not loaded") }
}

/// Resolve a function pointer from the loaded library, or panic.
unsafe fn resolve<T: Copy>(handle: &DlHandle, name: &str) -> T {
    let ptr = dl_sym(handle, name).expect("symbol not found in libm.so");
    core::mem::transmute_copy::<*const (), T>(&ptr)
}

/// Load libm.so and resolve all function pointers.
///
/// Call once at program start. Returns `true` on success.
/// No initialization function is called — libm is stateless.
pub fn load() -> bool {
    let handle = match dl_open("/Libraries/libm.so") {
        Some(h) => h,
        None => return false,
    };

    unsafe {
        let m = MathLib {
            // f64
            sqrt_fn: resolve(&handle, "math_sqrt"),
            sin_fn: resolve(&handle, "math_sin"),
            cos_fn: resolve(&handle, "math_cos"),
            sincos_fn: resolve(&handle, "math_sincos"),
            tan_fn: resolve(&handle, "math_tan"),
            asin_fn: resolve(&handle, "math_asin"),
            acos_fn: resolve(&handle, "math_acos"),
            atan_fn: resolve(&handle, "math_atan"),
            atan2_fn: resolve(&handle, "math_atan2"),
            pow_fn: resolve(&handle, "math_pow"),
            exp_fn: resolve(&handle, "math_exp"),
            exp2_fn: resolve(&handle, "math_exp2"),
            log_fn: resolve(&handle, "math_log"),
            log2_fn: resolve(&handle, "math_log2"),
            log10_fn: resolve(&handle, "math_log10"),
            floor_fn: resolve(&handle, "math_floor"),
            ceil_fn: resolve(&handle, "math_ceil"),
            round_fn: resolve(&handle, "math_round"),
            trunc_fn: resolve(&handle, "math_trunc"),
            fabs_fn: resolve(&handle, "math_fabs"),
            fmod_fn: resolve(&handle, "math_fmod"),
            fmin_fn: resolve(&handle, "math_fmin"),
            fmax_fn: resolve(&handle, "math_fmax"),
            hypot_fn: resolve(&handle, "math_hypot"),
            copysign_fn: resolve(&handle, "math_copysign"),
            ldexp_fn: resolve(&handle, "math_ldexp"),
            frexp_fn: resolve(&handle, "math_frexp"),
            cbrt_fn: resolve(&handle, "math_cbrt"),
            // f32
            sqrtf_fn: resolve(&handle, "math_sqrtf"),
            sinf_fn: resolve(&handle, "math_sinf"),
            cosf_fn: resolve(&handle, "math_cosf"),
            sincosf_fn: resolve(&handle, "math_sincosf"),
            tanf_fn: resolve(&handle, "math_tanf"),
            asinf_fn: resolve(&handle, "math_asinf"),
            acosf_fn: resolve(&handle, "math_acosf"),
            atanf_fn: resolve(&handle, "math_atanf"),
            atan2f_fn: resolve(&handle, "math_atan2f"),
            powf_fn: resolve(&handle, "math_powf"),
            expf_fn: resolve(&handle, "math_expf"),
            exp2f_fn: resolve(&handle, "math_exp2f"),
            logf_fn: resolve(&handle, "math_logf"),
            log2f_fn: resolve(&handle, "math_log2f"),
            log10f_fn: resolve(&handle, "math_log10f"),
            floorf_fn: resolve(&handle, "math_floorf"),
            ceilf_fn: resolve(&handle, "math_ceilf"),
            roundf_fn: resolve(&handle, "math_roundf"),
            truncf_fn: resolve(&handle, "math_truncf"),
            fabsf_fn: resolve(&handle, "math_fabsf"),
            fmodf_fn: resolve(&handle, "math_fmodf"),
            fminf_fn: resolve(&handle, "math_fminf"),
            fmaxf_fn: resolve(&handle, "math_fmaxf"),
            hypotf_fn: resolve(&handle, "math_hypotf"),
            copysignf_fn: resolve(&handle, "math_copysignf"),
            ldexpf_fn: resolve(&handle, "math_ldexpf"),
            frexpf_fn: resolve(&handle, "math_frexpf"),
            cbrtf_fn: resolve(&handle, "math_cbrtf"),
            _handle: handle,
        };
        LIB = Some(m);
    }

    true
}

// ── f64 wrappers ─────────────────────────────────────────────────────

/// Square root (IEEE 754 exact).
pub fn sqrt(x: f64) -> f64 { (lib().sqrt_fn)(x) }

/// Sine.
pub fn sin(x: f64) -> f64 { (lib().sin_fn)(x) }

/// Cosine.
pub fn cos(x: f64) -> f64 { (lib().cos_fn)(x) }

/// Compute sin and cos simultaneously (more efficient than separate calls).
pub fn sincos(x: f64) -> (f64, f64) {
    let mut s: f64 = 0.0;
    let mut c: f64 = 0.0;
    (lib().sincos_fn)(x, &mut s, &mut c);
    (s, c)
}

/// Tangent.
pub fn tan(x: f64) -> f64 { (lib().tan_fn)(x) }

/// Arcsine.
pub fn asin(x: f64) -> f64 { (lib().asin_fn)(x) }

/// Arccosine.
pub fn acos(x: f64) -> f64 { (lib().acos_fn)(x) }

/// Arctangent.
pub fn atan(x: f64) -> f64 { (lib().atan_fn)(x) }

/// Two-argument arctangent: angle of point (x, y).
pub fn atan2(y: f64, x: f64) -> f64 { (lib().atan2_fn)(y, x) }

/// Power: x raised to y.
pub fn pow(x: f64, y: f64) -> f64 { (lib().pow_fn)(x, y) }

/// Exponential: e^x.
pub fn exp(x: f64) -> f64 { (lib().exp_fn)(x) }

/// Base-2 exponential: 2^x.
pub fn exp2(x: f64) -> f64 { (lib().exp2_fn)(x) }

/// Natural logarithm: ln(x).
pub fn log(x: f64) -> f64 { (lib().log_fn)(x) }

/// Base-2 logarithm.
pub fn log2(x: f64) -> f64 { (lib().log2_fn)(x) }

/// Base-10 logarithm.
pub fn log10(x: f64) -> f64 { (lib().log10_fn)(x) }

/// Floor: largest integer <= x.
pub fn floor(x: f64) -> f64 { (lib().floor_fn)(x) }

/// Ceiling: smallest integer >= x.
pub fn ceil(x: f64) -> f64 { (lib().ceil_fn)(x) }

/// Round to nearest integer, ties away from zero.
pub fn round(x: f64) -> f64 { (lib().round_fn)(x) }

/// Truncate toward zero.
pub fn trunc(x: f64) -> f64 { (lib().trunc_fn)(x) }

/// Absolute value.
pub fn fabs(x: f64) -> f64 { (lib().fabs_fn)(x) }

/// Floating-point remainder: x - trunc(x/y) * y.
pub fn fmod(x: f64, y: f64) -> f64 { (lib().fmod_fn)(x, y) }

/// Minimum of two values (NaN-safe).
pub fn fmin(x: f64, y: f64) -> f64 { (lib().fmin_fn)(x, y) }

/// Maximum of two values (NaN-safe).
pub fn fmax(x: f64, y: f64) -> f64 { (lib().fmax_fn)(x, y) }

/// Hypotenuse: sqrt(x² + y²) with overflow protection.
pub fn hypot(x: f64, y: f64) -> f64 { (lib().hypot_fn)(x, y) }

/// Copy sign of y onto magnitude of x.
pub fn copysign(x: f64, y: f64) -> f64 { (lib().copysign_fn)(x, y) }

/// Load exponent: x * 2^n.
pub fn ldexp(x: f64, n: i32) -> f64 { (lib().ldexp_fn)(x, n) }

/// Extract normalized fraction and exponent: x = frac * 2^exp.
pub fn frexp(x: f64) -> (f64, i32) {
    let mut exp: i32 = 0;
    let frac = (lib().frexp_fn)(x, &mut exp);
    (frac, exp)
}

/// Cube root.
pub fn cbrt(x: f64) -> f64 { (lib().cbrt_fn)(x) }

// ── f32 wrappers ─────────────────────────────────────────────────────

/// Square root (f32, IEEE 754 exact).
pub fn sqrtf(x: f32) -> f32 { (lib().sqrtf_fn)(x) }

/// Sine (f32).
pub fn sinf(x: f32) -> f32 { (lib().sinf_fn)(x) }

/// Cosine (f32).
pub fn cosf(x: f32) -> f32 { (lib().cosf_fn)(x) }

/// Compute sin and cos simultaneously (f32).
pub fn sincosf(x: f32) -> (f32, f32) {
    let mut s: f32 = 0.0;
    let mut c: f32 = 0.0;
    (lib().sincosf_fn)(x, &mut s, &mut c);
    (s, c)
}

/// Tangent (f32).
pub fn tanf(x: f32) -> f32 { (lib().tanf_fn)(x) }

/// Arcsine (f32).
pub fn asinf(x: f32) -> f32 { (lib().asinf_fn)(x) }

/// Arccosine (f32).
pub fn acosf(x: f32) -> f32 { (lib().acosf_fn)(x) }

/// Arctangent (f32).
pub fn atanf(x: f32) -> f32 { (lib().atanf_fn)(x) }

/// Two-argument arctangent (f32).
pub fn atan2f(y: f32, x: f32) -> f32 { (lib().atan2f_fn)(y, x) }

/// Power (f32).
pub fn powf(x: f32, y: f32) -> f32 { (lib().powf_fn)(x, y) }

/// Exponential e^x (f32).
pub fn expf(x: f32) -> f32 { (lib().expf_fn)(x) }

/// Base-2 exponential (f32).
pub fn exp2f(x: f32) -> f32 { (lib().exp2f_fn)(x) }

/// Natural logarithm (f32).
pub fn logf(x: f32) -> f32 { (lib().logf_fn)(x) }

/// Base-2 logarithm (f32).
pub fn log2f(x: f32) -> f32 { (lib().log2f_fn)(x) }

/// Base-10 logarithm (f32).
pub fn log10f(x: f32) -> f32 { (lib().log10f_fn)(x) }

/// Floor (f32).
pub fn floorf(x: f32) -> f32 { (lib().floorf_fn)(x) }

/// Ceiling (f32).
pub fn ceilf(x: f32) -> f32 { (lib().ceilf_fn)(x) }

/// Round to nearest (f32), ties away from zero.
pub fn roundf(x: f32) -> f32 { (lib().roundf_fn)(x) }

/// Truncate toward zero (f32).
pub fn truncf(x: f32) -> f32 { (lib().truncf_fn)(x) }

/// Absolute value (f32).
pub fn fabsf(x: f32) -> f32 { (lib().fabsf_fn)(x) }

/// Floating-point remainder (f32).
pub fn fmodf(x: f32, y: f32) -> f32 { (lib().fmodf_fn)(x, y) }

/// Minimum (f32, NaN-safe).
pub fn fminf(x: f32, y: f32) -> f32 { (lib().fminf_fn)(x, y) }

/// Maximum (f32, NaN-safe).
pub fn fmaxf(x: f32, y: f32) -> f32 { (lib().fmaxf_fn)(x, y) }

/// Hypotenuse (f32) with overflow protection.
pub fn hypotf(x: f32, y: f32) -> f32 { (lib().hypotf_fn)(x, y) }

/// Copy sign (f32).
pub fn copysignf(x: f32, y: f32) -> f32 { (lib().copysignf_fn)(x, y) }

/// Load exponent (f32): x * 2^n.
pub fn ldexpf(x: f32, n: i32) -> f32 { (lib().ldexpf_fn)(x, n) }

/// Extract normalized fraction and exponent (f32).
pub fn frexpf(x: f32) -> (f32, i32) {
    let mut exp: i32 = 0;
    let frac = (lib().frexpf_fn)(x, &mut exp);
    (frac, exp)
}

/// Cube root (f32).
pub fn cbrtf(x: f32) -> f32 { (lib().cbrtf_fn)(x) }
