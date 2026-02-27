// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! f32 math operations using x86-64 SSE2 and x87 FPU hardware instructions.
//!
//! Mirror of f64_ops.rs with f32 types. SSE2 uses scalar single-precision
//! instructions (sqrtss, minss, maxss). x87 loads/stores use dword (32-bit).

use core::arch::asm;

// ── SSE2-based functions ─────────────────────────────────────────────

/// Square root (IEEE 754 exact) via SSE2 `sqrtss`.
#[inline]
pub fn sqrtf(x: f32) -> f32 {
    let mut result: f32 = 0.0;
    unsafe {
        asm!(
            "sqrtss {out}, {x}",
            x = in(xmm_reg) x,
            out = out(xmm_reg) result,
        );
    }
    result
}

/// Absolute value via SSE2 — clears the sign bit.
#[inline]
pub fn fabsf(x: f32) -> f32 {
    f32::from_bits(x.to_bits() & 0x7FFFFFFF)
}

/// Minimum of two values via SSE2 `minss`.
#[inline]
pub fn fminf(x: f32, y: f32) -> f32 {
    if x != x { return y; }
    if y != y { return x; }
    let mut result: f32 = 0.0;
    unsafe {
        asm!(
            "minss {x}, {y}",
            x = inlateout(xmm_reg) x => result,
            y = in(xmm_reg) y,
        );
    }
    result
}

/// Maximum of two values via SSE2 `maxss`.
#[inline]
pub fn fmaxf(x: f32, y: f32) -> f32 {
    if x != x { return y; }
    if y != y { return x; }
    let mut result: f32 = 0.0;
    unsafe {
        asm!(
            "maxss {x}, {y}",
            x = inlateout(xmm_reg) x => result,
            y = in(xmm_reg) y,
        );
    }
    result
}

/// Copy sign of `y` onto magnitude of `x` via bit manipulation.
#[inline]
pub fn copysignf(x: f32, y: f32) -> f32 {
    let bits_x: u32 = x.to_bits();
    let bits_y: u32 = y.to_bits();
    f32::from_bits((bits_x & 0x7FFFFFFF) | (bits_y & 0x80000000))
}

/// Truncate to integer via IEEE 754 bit manipulation.
#[inline]
pub fn truncf(x: f32) -> f32 {
    let bits = x.to_bits();
    let exponent = ((bits >> 23) & 0xFF) as i32 - 127;
    if exponent >= 23 {
        return x; // already integer (or Inf/NaN)
    }
    if exponent < 0 {
        return f32::from_bits(bits & 0x80000000); // |x| < 1 → ±0
    }
    let mask: u32 = !((1u32 << (23 - exponent as u32)) - 1);
    f32::from_bits(bits & mask)
}

/// Hypotenuse sqrt(x² + y²) via SSE2.
#[inline]
pub fn hypotf(x: f32, y: f32) -> f32 {
    let ax = fabsf(x);
    let ay = fabsf(y);
    if ax == f32::INFINITY || ay == f32::INFINITY {
        return f32::INFINITY;
    }
    if ax != ax || ay != ay {
        return f32::NAN;
    }
    let (big, small) = if ax >= ay { (ax, ay) } else { (ay, ax) };
    if big == 0.0 {
        return 0.0;
    }
    let ratio = small / big;
    big * sqrtf(1.0 + ratio * ratio)
}

// ── x87 FPU-based functions ──────────────────────────────────────────

/// Sine via x87 `fsin`.
#[inline]
pub fn sinf(x: f32) -> f32 {
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
pub fn cosf(x: f32) -> f32 {
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

/// Compute sin and cos simultaneously via x87 `fsincos`.
#[inline]
pub fn sincosf(x: f32, out_sin: &mut f32, out_cos: &mut f32) {
    unsafe {
        asm!(
            "fld dword ptr [{x}]",
            "fsincos",
            "fstp dword ptr [{out_cos}]",
            "fstp dword ptr [{out_sin}]",
            x = in(reg) &x,
            out_sin = in(reg) out_sin as *mut f32,
            out_cos = in(reg) out_cos as *mut f32,
            options(nostack),
        );
    }
}

/// Tangent via x87 `fptan`.
#[inline]
pub fn tanf(x: f32) -> f32 {
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

/// Arctangent via x87 `fpatan`.
#[inline]
pub fn atanf(x: f32) -> f32 {
    let mut result: f32 = 0.0;
    unsafe {
        asm!(
            "fld dword ptr [{x}]",
            "fld1",
            "fpatan",
            "fstp dword ptr [{out}]",
            x = in(reg) &x,
            out = in(reg) &mut result,
            options(nostack),
        );
    }
    result
}

/// Two-argument arctangent via x87 `fpatan`.
///
/// Load y first, then x. fpatan computes atan2(ST1, ST0) = atan2(y, x).
#[inline]
pub fn atan2f(y: f32, x: f32) -> f32 {
    let mut result: f32 = 0.0;
    unsafe {
        asm!(
            "fld dword ptr [{y}]",
            "fld dword ptr [{x}]",
            "fpatan",
            "fstp dword ptr [{out}]",
            y = in(reg) &y,
            x = in(reg) &x,
            out = in(reg) &mut result,
            options(nostack),
        );
    }
    result
}

/// Arcsine: asin(x) = atan2(x, sqrt(1 - x²)).
#[inline]
pub fn asinf(x: f32) -> f32 {
    if x >= 1.0 {
        return core::f32::consts::FRAC_PI_2;
    }
    if x <= -1.0 {
        return -core::f32::consts::FRAC_PI_2;
    }
    atan2f(x, sqrtf(1.0 - x * x))
}

/// Arccosine: acos(x) = atan2(sqrt(1 - x²), x).
#[inline]
pub fn acosf(x: f32) -> f32 {
    if x >= 1.0 {
        return 0.0;
    }
    if x <= -1.0 {
        return core::f32::consts::PI;
    }
    atan2f(sqrtf(1.0 - x * x), x)
}

/// Power function x^y via x87.
pub fn powf(x: f32, y: f32) -> f32 {
    if y == 0.0 {
        return 1.0;
    }
    if x == 1.0 {
        return 1.0;
    }
    if x == 0.0 {
        if y > 0.0 { return 0.0; }
        return f32::INFINITY;
    }
    if x != x || y != y {
        return f32::NAN;
    }

    let negative_base = x < 0.0;
    let abs_x = if negative_base { -x } else { x };

    if negative_base {
        let y_trunc = truncf(y);
        if y != y_trunc {
            return f32::NAN;
        }
        let result = powf_positive(abs_x, y);
        let y_int = y_trunc as i32;
        if y_int & 1 != 0 {
            return -result;
        }
        return result;
    }

    powf_positive(abs_x, y)
}

/// Power for positive base via x87 FPU.
fn powf_positive(x: f32, y: f32) -> f32 {
    // Promote to f64 for x87 internal precision, then demote result
    let xd = x as f64;
    let yd = y as f64;
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

/// Exponential e^x via x87.
pub fn expf(x: f32) -> f32 {
    let xd = x as f64;
    let mut result: f64 = 0.0;
    unsafe {
        asm!(
            "fld qword ptr [{x}]",
            "fldl2e",
            "fmulp",
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

/// Base-2 exponential 2^x via x87.
pub fn exp2f(x: f32) -> f32 {
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

/// Natural logarithm via x87.
pub fn logf(x: f32) -> f32 {
    let mut result: f32 = 0.0;
    unsafe {
        asm!(
            "fldln2",
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

/// Base-2 logarithm via x87.
pub fn log2f(x: f32) -> f32 {
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

/// Base-10 logarithm via x87.
pub fn log10f(x: f32) -> f32 {
    let mut result: f32 = 0.0;
    unsafe {
        asm!(
            "fldlg2",
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

/// Floor via x87 with round-down control word.
pub fn floorf(x: f32) -> f32 {
    let mut result: f32 = 0.0;
    let mut cw_save: u16 = 0;
    let mut cw_new: u16 = 0;
    unsafe {
        asm!(
            "fnstcw [{cw_save}]",
            "movzx {tmp:e}, word ptr [{cw_save}]",
            "and {tmp:e}, 0xF3FF",
            "or {tmp:e}, 0x0400",
            "mov word ptr [{cw_new}], {tmp:x}",
            "fldcw [{cw_new}]",
            "fld dword ptr [{x}]",
            "frndint",
            "fstp dword ptr [{out}]",
            "fldcw [{cw_save}]",
            x = in(reg) &x,
            out = in(reg) &mut result,
            cw_save = in(reg) &mut cw_save,
            cw_new = in(reg) &mut cw_new,
            tmp = out(reg) _,
            options(nostack),
        );
    }
    result
}

/// Ceiling via x87 with round-up control word.
pub fn ceilf(x: f32) -> f32 {
    let mut result: f32 = 0.0;
    let mut cw_save: u16 = 0;
    let mut cw_new: u16 = 0;
    unsafe {
        asm!(
            "fnstcw [{cw_save}]",
            "movzx {tmp:e}, word ptr [{cw_save}]",
            "and {tmp:e}, 0xF3FF",
            "or {tmp:e}, 0x0800",
            "mov word ptr [{cw_new}], {tmp:x}",
            "fldcw [{cw_new}]",
            "fld dword ptr [{x}]",
            "frndint",
            "fstp dword ptr [{out}]",
            "fldcw [{cw_save}]",
            x = in(reg) &x,
            out = in(reg) &mut result,
            cw_save = in(reg) &mut cw_save,
            cw_new = in(reg) &mut cw_new,
            tmp = out(reg) _,
            options(nostack),
        );
    }
    result
}

/// Round to nearest integer, ties away from zero.
#[inline]
pub fn roundf(x: f32) -> f32 {
    truncf(x + copysignf(0.5, x))
}

/// Floating-point remainder via x87 `fprem`.
pub fn fmodf(x: f32, y: f32) -> f32 {
    if y == 0.0 {
        return f32::NAN;
    }
    let mut result: f32 = 0.0;
    unsafe {
        asm!(
            "fld dword ptr [{y}]",
            "fld dword ptr [{x}]",
            "2:",
            "fprem",
            "fnstsw ax",
            "test ah, 0x04",
            "jnz 2b",
            "fstp dword ptr [{out}]",
            "fstp st(0)",
            x = in(reg) &x,
            y = in(reg) &y,
            out = in(reg) &mut result,
            out("ax") _,
            options(nostack),
        );
    }
    result
}

/// Load-exponent: x * 2^n via x87 `fscale`.
pub fn ldexpf(x: f32, n: i32) -> f32 {
    let n_f64 = n as f64;
    let mut result: f32 = 0.0;
    unsafe {
        asm!(
            "fld qword ptr [{n}]",
            "fld dword ptr [{x}]",
            "fscale",
            "fstp st(1)",
            "fstp dword ptr [{out}]",
            x = in(reg) &x,
            n = in(reg) &n_f64,
            out = in(reg) &mut result,
            options(nostack),
        );
    }
    result
}

/// Extract exponent and mantissa (IEEE 754 bit manipulation).
pub fn frexpf(x: f32, exp: &mut i32) -> f32 {
    let bits = x.to_bits();
    let raw_exp = ((bits >> 23) & 0xFF) as i32;

    if raw_exp == 0 {
        if bits & 0x7FFFFFFF == 0 {
            *exp = 0;
            return x;
        }
        // Denormalized: scale up
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

/// Cube root via Newton-Raphson / Halley iteration.
pub fn cbrtf(x: f32) -> f32 {
    if x == 0.0 || x != x {
        return x;
    }
    let negative = x < 0.0;
    let ax = if negative { -x } else { x };

    let bits = ax.to_bits();
    let guess_bits = bits / 3 + 0x2A555555;
    let mut y = f32::from_bits(guess_bits);

    // Halley's method: 2 iterations suffice for f32 precision
    for _ in 0..2 {
        let y3 = y * y * y;
        y = y * (y3 + ax + ax) / (y3 + y3 + ax);
    }

    if negative { -y } else { y }
}
