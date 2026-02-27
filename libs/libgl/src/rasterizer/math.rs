//! Math functions for the software rasterizer using inline FPU/SSE instructions.
//!
//! Uses x87 FPU for transcendental functions (sin, cos, pow, log2, exp2) and
//! SSE2 for sqrt. IEEE 754 exact results with hardware acceleration — no
//! polynomial approximations needed.

use core::arch::asm;
use core::arch::x86_64::*;

/// Pi constant.
pub const PI: f32 = 3.14159265;

// ── SSE2-based functions ─────────────────────────────────────────────

/// Square root via SSE2 `sqrtss` (IEEE 754 exact, ~1 cycle).
#[inline]
pub fn sqrt(x: f32) -> f32 {
    let mut result: f32;
    unsafe {
        asm!(
            "sqrtss {out}, {x}",
            x = in(xmm_reg) x,
            out = out(xmm_reg) result,
        );
    }
    result
}

/// Absolute value via sign-bit clear.
#[inline]
pub fn abs(x: f32) -> f32 {
    f32::from_bits(x.to_bits() & 0x7FFFFFFF)
}

// ── x87 FPU-based transcendental functions ───────────────────────────

/// Sine via x87 `fsin`.
#[inline]
pub fn sin(x: f32) -> f32 {
    let mut result: f32 = 0.0;
    unsafe {
        asm!(
            "fld dword ptr [{x}]",
            "fsin",
            "fstp dword ptr [{out}]",
            x = in(reg) &x,
            out = in(reg) &mut result,
            options(nostack),
        );
    }
    result
}

/// Cosine via x87 `fcos`.
#[inline]
pub fn cos(x: f32) -> f32 {
    let mut result: f32 = 0.0;
    unsafe {
        asm!(
            "fld dword ptr [{x}]",
            "fcos",
            "fstp dword ptr [{out}]",
            x = in(reg) &x,
            out = in(reg) &mut result,
            options(nostack),
        );
    }
    result
}

/// Tangent via x87 `fptan`.
#[inline]
pub fn tan(x: f32) -> f32 {
    let mut result: f32 = 0.0;
    unsafe {
        asm!(
            "fld dword ptr [{x}]",
            "fptan",
            "fstp st(0)",
            "fstp dword ptr [{out}]",
            x = in(reg) &x,
            out = in(reg) &mut result,
            options(nostack),
        );
    }
    result
}

/// Base-2 logarithm via x87 `fyl2x`.
#[inline]
pub fn log2(x: f32) -> f32 {
    let mut result: f32 = 0.0;
    unsafe {
        asm!(
            "fld1",
            "fld dword ptr [{x}]",
            "fyl2x",
            "fstp dword ptr [{out}]",
            x = in(reg) &x,
            out = in(reg) &mut result,
            options(nostack),
        );
    }
    result
}

/// Base-2 exponential via x87 `f2xm1` + `fscale`.
#[inline]
pub fn exp2(x: f32) -> f32 {
    let xd = x as f64;
    let mut result: f64 = 0.0;
    unsafe {
        asm!(
            "fld qword ptr [{x}]",
            "fld st(0)",
            "frndint",
            "fsub st(1), st(0)",
            "fxch st(1)",
            "f2xm1",
            "fld1",
            "faddp",
            "fscale",
            "fstp st(1)",
            "fstp qword ptr [{out}]",
            x = in(reg) &xd,
            out = in(reg) &mut result,
            options(nostack),
        );
    }
    result as f32
}

/// Power function x^y via x87 FPU.
pub fn pow(base: f32, exp: f32) -> f32 {
    if exp == 0.0 { return 1.0; }
    if base == 1.0 { return 1.0; }
    if base == 0.0 {
        if exp > 0.0 { return 0.0; }
        return f32::from_bits(0x7F800000); // +inf
    }
    if base < 0.0 { return 0.0; } // negative base unsupported for fractional exp

    // x^y = 2^(y * log2(x)) using x87 fyl2x + f2xm1 + fscale
    let xd = base as f64;
    let yd = exp as f64;
    let mut result: f64 = 0.0;
    unsafe {
        asm!(
            "fld qword ptr [{y}]",
            "fld qword ptr [{x}]",
            "fyl2x",
            "fld st(0)",
            "frndint",
            "fsub st(1), st(0)",
            "fxch st(1)",
            "f2xm1",
            "fld1",
            "faddp",
            "fscale",
            "fstp st(1)",
            "fstp qword ptr [{out}]",
            x = in(reg) &xd,
            y = in(reg) &yd,
            out = in(reg) &mut result,
            options(nostack),
        );
    }
    result as f32
}

/// Floor via SSE4.1 `roundss` (1 cycle vs 16+ for x87 control word).
#[inline]
pub fn floor(x: f32) -> f32 {
    unsafe {
        let v = _mm_set_ss(x);
        _mm_cvtss_f32(_mm_floor_ss(v, v))
    }
}

/// Ceiling via SSE4.1 `roundss` (1 cycle vs 16+ for x87 control word).
#[inline]
pub fn ceil(x: f32) -> f32 {
    unsafe {
        let v = _mm_set_ss(x);
        _mm_cvtss_f32(_mm_ceil_ss(v, v))
    }
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
