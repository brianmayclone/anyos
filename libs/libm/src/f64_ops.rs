// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! f64 math operations using x86-64 SSE2 and x87 FPU hardware instructions.
//!
//! SSE2 is used for: sqrt, fabs, fmin, fmax, copysign, trunc, hypot.
//! x87 FPU is used for: sin, cos, sincos, tan, asin, acos, atan, atan2,
//!   pow, exp, exp2, log, log2, log10, floor, ceil, fmod, ldexp.
//! Pure Rust is used for: round, frexp, cbrt.

use core::arch::asm;

// ── SSE2-based functions ─────────────────────────────────────────────

/// Square root (IEEE 754 exact) via SSE2 `sqrtsd`.
#[inline]
pub fn sqrt(x: f64) -> f64 {
    let mut result: f64 = 0.0;
    unsafe {
        asm!(
            "sqrtsd {out}, {x}",
            x = in(xmm_reg) x,
            out = out(xmm_reg) result,
        );
    }
    result
}

/// Absolute value via SSE2 — clears the sign bit.
#[inline]
pub fn fabs(x: f64) -> f64 {
    let mut result: f64 = 0.0;
    unsafe {
        asm!(
            "pcmpeqd {mask}, {mask}",  // all 1s
            "psrlq {mask}, 1",         // 0x7FFFFFFFFFFFFFFF
            "andpd {val}, {mask}",
            mask = out(xmm_reg) _,
            val = inlateout(xmm_reg) x => result,
        );
    }
    result
}

/// Minimum of two values via SSE2 `minsd`.
#[inline]
pub fn fmin(x: f64, y: f64) -> f64 {
    // Handle NaN: if either is NaN, return the other
    if x != x { return y; }
    if y != y { return x; }
    let mut result: f64 = 0.0;
    unsafe {
        asm!(
            "minsd {x}, {y}",
            x = inlateout(xmm_reg) x => result,
            y = in(xmm_reg) y,
        );
    }
    result
}

/// Maximum of two values via SSE2 `maxsd`.
#[inline]
pub fn fmax(x: f64, y: f64) -> f64 {
    // Handle NaN: if either is NaN, return the other
    if x != x { return y; }
    if y != y { return x; }
    let mut result: f64 = 0.0;
    unsafe {
        asm!(
            "maxsd {x}, {y}",
            x = inlateout(xmm_reg) x => result,
            y = in(xmm_reg) y,
        );
    }
    result
}

/// Copy sign of `y` onto magnitude of `x` via IEEE 754 bit manipulation.
#[inline]
pub fn copysign(x: f64, y: f64) -> f64 {
    f64::from_bits((x.to_bits() & 0x7FFFFFFFFFFFFFFF) | (y.to_bits() & 0x8000000000000000))
}

/// Truncate to integer via SSE2 `cvttsd2si` + `cvtsi2sd`.
///
/// Handles the range of i64. Values outside i64 range are returned as-is
/// (they are already integers by IEEE 754 representation).
#[inline]
pub fn trunc(x: f64) -> f64 {
    // Values >= 2^52 are already integers in f64 representation
    let bits = x.to_bits();
    let exponent = ((bits >> 52) & 0x7FF) as i32 - 1023;
    if exponent >= 52 {
        return x; // already integer (or Inf/NaN)
    }
    if exponent < 0 {
        // |x| < 1.0 → truncate to ±0.0
        return f64::from_bits(bits & 0x8000000000000000);
    }
    // Mask off fractional bits
    let mask: u64 = !((1u64 << (52 - exponent as u32)) - 1);
    f64::from_bits(bits & mask)
}

/// Hypotenuse sqrt(x² + y²) via SSE2 multiply + add + sqrtsd.
///
/// Uses a scaled approach to avoid overflow/underflow for large/small values.
#[inline]
pub fn hypot(x: f64, y: f64) -> f64 {
    let ax = fabs(x);
    let ay = fabs(y);
    // Handle Inf: if either is infinite, result is infinite
    if ax == f64::INFINITY || ay == f64::INFINITY {
        return f64::INFINITY;
    }
    // Handle NaN
    if ax != ax || ay != ay {
        return f64::NAN;
    }
    // Scale to avoid overflow: compute max * sqrt(1 + (min/max)²)
    let (big, small) = if ax >= ay { (ax, ay) } else { (ay, ax) };
    if big == 0.0 {
        return 0.0;
    }
    let ratio = small / big;
    big * sqrt(1.0 + ratio * ratio)
}

// ── x87 FPU-based functions ──────────────────────────────────────────

/// Sine via x87 `fsin`.
#[inline]
pub fn sin(x: f64) -> f64 {
    let mut result: f64 = 0.0;
    unsafe {
        asm!(
            "fld qword ptr [{x}]",
            "fsin",
            "fstp qword ptr [{out}]",
            x = in(reg) &x,
            out = in(reg) &mut result,
            options(nostack),
        );
    }
    result
}

/// Cosine via x87 `fcos`.
#[inline]
pub fn cos(x: f64) -> f64 {
    let mut result: f64 = 0.0;
    unsafe {
        asm!(
            "fld qword ptr [{x}]",
            "fcos",
            "fstp qword ptr [{out}]",
            x = in(reg) &x,
            out = in(reg) &mut result,
            options(nostack),
        );
    }
    result
}

/// Compute sin and cos simultaneously via x87 `fsincos`.
///
/// More efficient than calling sin() and cos() separately.
/// `fsincos` pushes cos(ST0), then replaces ST0 with sin(ST0).
/// Stack after fsincos: ST0 = sin, ST1 = cos.
#[inline]
pub fn sincos(x: f64, out_sin: &mut f64, out_cos: &mut f64) {
    unsafe {
        asm!(
            "fld qword ptr [{x}]",
            "fsincos",
            "fstp qword ptr [{out_cos}]",  // pop cos (ST0 after fsincos = cos)
            "fstp qword ptr [{out_sin}]",  // pop sin (ST1 after fsincos = sin, now ST0)
            x = in(reg) &x,
            out_sin = in(reg) out_sin as *mut f64,
            out_cos = in(reg) out_cos as *mut f64,
            options(nostack),
        );
    }
}

/// Tangent via x87 `fptan`.
///
/// `fptan` replaces ST0 with tan(ST0), then pushes 1.0.
/// Stack after fptan: ST0 = 1.0, ST1 = tan(x). We pop the 1.0.
#[inline]
pub fn tan(x: f64) -> f64 {
    let mut result: f64 = 0.0;
    unsafe {
        asm!(
            "fld qword ptr [{x}]",
            "fptan",
            "fstp st(0)",               // pop the 1.0
            "fstp qword ptr [{out}]",   // store tan(x)
            x = in(reg) &x,
            out = in(reg) &mut result,
            options(nostack),
        );
    }
    result
}

/// Arctangent via x87 `fpatan`.
///
/// `fpatan` computes atan2(ST1, ST0) = atan(ST1/ST0).
/// For atan(x): push 1.0 (denominator), then x (numerator).
#[inline]
pub fn atan(x: f64) -> f64 {
    let mut result: f64 = 0.0;
    unsafe {
        asm!(
            "fld qword ptr [{x}]",
            "fld1",
            "fpatan",                    // atan2(x, 1.0) = atan(x)
            "fstp qword ptr [{out}]",
            x = in(reg) &x,
            out = in(reg) &mut result,
            options(nostack),
        );
    }
    result
}

/// Two-argument arctangent via x87 `fpatan`.
///
/// `fpatan` computes atan2(ST1, ST0). We load x first (becomes ST1),
/// then y (becomes ST0... wait, fpatan needs ST1=y, ST0=x).
/// Correction: load x first (ST0), then load y → ST0=y, ST1=x.
/// fpatan → atan2(ST1, ST0) = atan2(x, y)... that's wrong.
///
/// Actually: fpatan computes atan(ST1/ST0) with correct quadrant.
/// So for atan2(y, x): ST1=y, ST0=x. Load x first, then y.
/// Wait: when we `fld x`, ST0=x. Then `fld y`, ST0=y, ST1=x.
/// fpatan → atan2(ST1, ST0) = atan(ST1/ST0) = atan(x/y)... no.
///
/// Intel manual: FPATAN computes arctan(ST(1)/ST(0)), pops both, pushes result.
/// For atan2(y, x) we want arctan(y/x) with quadrant handling.
/// So we need ST(1)=y, ST(0)=x. Load x first (→ST0), then y (→ST0, x→ST1)...
/// No: after fld x → ST0=x. After fld y → ST0=y, ST1=x.
/// fpatan → arctan(ST1/ST0) = arctan(x/y). That gives atan2(x, y), not atan2(y, x).
///
/// Fix: load y first, then x. After fld y → ST0=y. After fld x → ST0=x, ST1=y.
/// fpatan → arctan(ST1/ST0) = arctan(y/x) = atan2(y, x). Correct!
#[inline]
pub fn atan2(y: f64, x: f64) -> f64 {
    let mut result: f64 = 0.0;
    unsafe {
        asm!(
            "fld qword ptr [{y}]",      // ST0 = y
            "fld qword ptr [{x}]",      // ST0 = x, ST1 = y
            "fpatan",                    // atan2(ST1, ST0) = atan2(y, x)
            "fstp qword ptr [{out}]",
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
pub fn asin(x: f64) -> f64 {
    if x >= 1.0 {
        return core::f64::consts::FRAC_PI_2;
    }
    if x <= -1.0 {
        return -core::f64::consts::FRAC_PI_2;
    }
    atan2(x, sqrt(1.0 - x * x))
}

/// Arccosine: acos(x) = atan2(sqrt(1 - x²), x).
#[inline]
pub fn acos(x: f64) -> f64 {
    if x >= 1.0 {
        return 0.0;
    }
    if x <= -1.0 {
        return core::f64::consts::PI;
    }
    atan2(sqrt(1.0 - x * x), x)
}

/// Power function x^y via x87: 2^(y * log2(x)).
///
/// Uses `fyl2x` to compute y*log2(|x|), then decomposes into integer + fraction,
/// applies `f2xm1` for the fractional part, and `fscale` for the integer part.
pub fn pow(x: f64, y: f64) -> f64 {
    // Special cases
    if y == 0.0 {
        return 1.0; // x^0 = 1 for all x (including 0, NaN)
    }
    if x == 1.0 {
        return 1.0; // 1^y = 1 for all y
    }
    if x == 0.0 {
        if y > 0.0 {
            return 0.0;
        }
        return f64::INFINITY; // 0^(negative) = Inf
    }
    if x != x || y != y {
        return f64::NAN; // propagate NaN
    }

    // Handle negative base: only valid for integer exponents
    let negative_base = x < 0.0;
    let abs_x = if negative_base { -x } else { x };

    if negative_base {
        // Check if y is an integer
        let y_trunc = trunc(y);
        if y != y_trunc {
            return f64::NAN; // negative base with non-integer exponent
        }
        let result = pow_positive(abs_x, y);
        // Odd integer exponent → negate result
        let y_int = y_trunc as i64;
        if y_int & 1 != 0 {
            return -result;
        }
        return result;
    }

    pow_positive(abs_x, y)
}

/// Power for positive base via x87 FPU: 2^(y * log2(x)).
fn pow_positive(x: f64, y: f64) -> f64 {
    let mut result: f64 = 0.0;
    unsafe {
        asm!(
            // Compute y * log2(x) via fyl2x
            "fld qword ptr [{y}]",      // ST0 = y
            "fld qword ptr [{x}]",      // ST0 = x, ST1 = y
            "fyl2x",                     // ST0 = y * log2(x)
            // Decompose into integer + fractional part
            "fld st(0)",                 // ST0 = ST1 = y*log2(x)
            "frndint",                   // ST0 = round(y*log2(x)), ST1 = y*log2(x)
            "fsub st(1), st(0)",         // ST1 = fractional part, ST0 = integer part
            "fxch st(1)",               // ST0 = frac, ST1 = int
            // 2^frac via f2xm1 (valid for -1 <= x <= 1)
            "f2xm1",                    // ST0 = 2^frac - 1
            "fld1",
            "faddp",                    // ST0 = 2^frac
            // Scale by integer part: 2^frac * 2^int = 2^(frac+int)
            "fscale",                   // ST0 = 2^frac * 2^int
            "fstp st(1)",              // pop integer part, ST0 = result
            "fstp qword ptr [{out}]",
            x = in(reg) &x,
            y = in(reg) &y,
            out = in(reg) &mut result,
            options(nostack),
        );
    }
    result
}

/// Exponential e^x via x87: 2^(x * log2(e)).
pub fn exp(x: f64) -> f64 {
    let mut result: f64 = 0.0;
    unsafe {
        asm!(
            "fld qword ptr [{x}]",      // ST0 = x
            "fldl2e",                    // ST0 = log2(e), ST1 = x
            "fmulp",                    // ST0 = x * log2(e)
            // Decompose into integer + fractional part
            "fld st(0)",
            "frndint",                   // ST0 = int, ST1 = x*log2(e)
            "fsub st(1), st(0)",         // ST1 = frac, ST0 = int
            "fxch st(1)",               // ST0 = frac, ST1 = int
            "f2xm1",                    // ST0 = 2^frac - 1
            "fld1",
            "faddp",                    // ST0 = 2^frac
            "fscale",                   // ST0 = result
            "fstp st(1)",
            "fstp qword ptr [{out}]",
            x = in(reg) &x,
            out = in(reg) &mut result,
            options(nostack),
        );
    }
    result
}

/// Base-2 exponential 2^x via x87.
pub fn exp2(x: f64) -> f64 {
    let mut result: f64 = 0.0;
    unsafe {
        asm!(
            "fld qword ptr [{x}]",      // ST0 = x
            // Decompose into integer + fractional part
            "fld st(0)",
            "frndint",                   // ST0 = int, ST1 = x
            "fsub st(1), st(0)",         // ST1 = frac, ST0 = int
            "fxch st(1)",               // ST0 = frac, ST1 = int
            "f2xm1",                    // ST0 = 2^frac - 1
            "fld1",
            "faddp",                    // ST0 = 2^frac
            "fscale",                   // ST0 = result
            "fstp st(1)",
            "fstp qword ptr [{out}]",
            x = in(reg) &x,
            out = in(reg) &mut result,
            options(nostack),
        );
    }
    result
}

/// Natural logarithm via x87: ln(x) = log2(x) * ln(2).
///
/// Uses `fyl2x` with y = ln(2) (loaded via `fldln2`).
pub fn log(x: f64) -> f64 {
    let mut result: f64 = 0.0;
    unsafe {
        asm!(
            "fldln2",                    // ST0 = ln(2)
            "fld qword ptr [{x}]",      // ST0 = x, ST1 = ln(2)
            "fyl2x",                     // ST0 = ln(2) * log2(x) = ln(x)
            "fstp qword ptr [{out}]",
            x = in(reg) &x,
            out = in(reg) &mut result,
            options(nostack),
        );
    }
    result
}

/// Base-2 logarithm via x87: log2(x) = 1.0 * log2(x).
///
/// Uses `fyl2x` with y = 1.0.
pub fn log2(x: f64) -> f64 {
    let mut result: f64 = 0.0;
    unsafe {
        asm!(
            "fld1",                      // ST0 = 1.0
            "fld qword ptr [{x}]",      // ST0 = x, ST1 = 1.0
            "fyl2x",                     // ST0 = 1.0 * log2(x) = log2(x)
            "fstp qword ptr [{out}]",
            x = in(reg) &x,
            out = in(reg) &mut result,
            options(nostack),
        );
    }
    result
}

/// Base-10 logarithm via x87: log10(x) = log10(2) * log2(x).
///
/// Uses `fyl2x` with y = log10(2) (loaded via `fldlg2`).
pub fn log10(x: f64) -> f64 {
    let mut result: f64 = 0.0;
    unsafe {
        asm!(
            "fldlg2",                    // ST0 = log10(2)
            "fld qword ptr [{x}]",      // ST0 = x, ST1 = log10(2)
            "fyl2x",                     // ST0 = log10(2) * log2(x) = log10(x)
            "fstp qword ptr [{out}]",
            x = in(reg) &x,
            out = in(reg) &mut result,
            options(nostack),
        );
    }
    result
}

/// Floor (round toward negative infinity) via x87.
///
/// Temporarily sets the x87 rounding mode to round-down (mode 01),
/// applies `frndint`, then restores the original control word.
pub fn floor(x: f64) -> f64 {
    let mut result: f64 = 0.0;
    let mut cw_save: u16 = 0;
    let mut cw_new: u16 = 0;
    unsafe {
        asm!(
            // Save current control word and set round-down mode
            "fnstcw [{cw_save}]",
            "movzx {tmp:e}, word ptr [{cw_save}]",
            "and {tmp:e}, 0xF3FF",       // clear RC bits (bits 10-11)
            "or {tmp:e}, 0x0400",        // set RC = 01 (round down)
            "mov word ptr [{cw_new}], {tmp:x}",
            "fldcw [{cw_new}]",
            // Round
            "fld qword ptr [{x}]",
            "frndint",
            "fstp qword ptr [{out}]",
            // Restore original control word
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

/// Ceiling (round toward positive infinity) via x87.
///
/// Temporarily sets the x87 rounding mode to round-up (mode 10),
/// applies `frndint`, then restores the original control word.
pub fn ceil(x: f64) -> f64 {
    let mut result: f64 = 0.0;
    let mut cw_save: u16 = 0;
    let mut cw_new: u16 = 0;
    unsafe {
        asm!(
            // Save current control word and set round-up mode
            "fnstcw [{cw_save}]",
            "movzx {tmp:e}, word ptr [{cw_save}]",
            "and {tmp:e}, 0xF3FF",       // clear RC bits
            "or {tmp:e}, 0x0800",        // set RC = 10 (round up)
            "mov word ptr [{cw_new}], {tmp:x}",
            "fldcw [{cw_new}]",
            // Round
            "fld qword ptr [{x}]",
            "frndint",
            "fstp qword ptr [{out}]",
            // Restore original control word
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

/// Round to nearest integer, with ties rounding away from zero (C99 `round`).
///
/// Implemented as: trunc(x + copysign(0.5, x)).
#[inline]
pub fn round(x: f64) -> f64 {
    trunc(x + copysign(0.5, x))
}

/// Floating-point remainder via x87 `fprem`.
///
/// `fprem` computes partial remainder: ST0 = ST0 - Q * ST1, where Q is truncated.
/// Must loop until C2 flag in status word is cleared (reduction complete).
pub fn fmod(x: f64, y: f64) -> f64 {
    if y == 0.0 {
        return f64::NAN;
    }
    let mut result: f64 = 0.0;
    unsafe {
        asm!(
            "fld qword ptr [{y}]",      // ST0 = y (divisor)
            "fld qword ptr [{x}]",      // ST0 = x, ST1 = y
            "2:",
            "fprem",                     // ST0 = partial remainder
            "fnstsw ax",                // status word → AX
            "test ah, 0x04",            // check C2 (bit 10 of status = bit 2 of AH)
            "jnz 2b",                   // loop if reduction incomplete
            "fstp qword ptr [{out}]",   // store result
            "fstp st(0)",              // pop y
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
pub fn ldexp(x: f64, n: i32) -> f64 {
    let mut result: f64 = 0.0;
    let n_f64 = n as f64;
    unsafe {
        asm!(
            "fld qword ptr [{n}]",      // ST0 = n (as f64)
            "fld qword ptr [{x}]",      // ST0 = x, ST1 = n
            "fscale",                    // ST0 = x * 2^int(ST1)
            "fstp st(1)",              // pop n
            "fstp qword ptr [{out}]",
            x = in(reg) &x,
            n = in(reg) &n_f64,
            out = in(reg) &mut result,
            options(nostack),
        );
    }
    result
}

/// Extract exponent and mantissa from a floating-point number.
///
/// Returns the normalized fraction in [0.5, 1.0) and stores the exponent
/// in `*exp`. If x is zero, both fraction and exponent are zero.
///
/// Implemented via IEEE 754 bit manipulation (no FPU needed).
pub fn frexp(x: f64, exp: &mut i32) -> f64 {
    let bits = x.to_bits();
    let raw_exp = ((bits >> 52) & 0x7FF) as i32;

    // Zero or denormalized
    if raw_exp == 0 {
        if bits & 0x7FFFFFFFFFFFFFFF == 0 {
            // ±0
            *exp = 0;
            return x;
        }
        // Denormalized: multiply by 2^64 to normalize, then adjust
        let scaled = x * (1u64 << 52) as f64 * (1u64 << 12) as f64;
        let scaled_bits = scaled.to_bits();
        let scaled_exp = ((scaled_bits >> 52) & 0x7FF) as i32;
        *exp = scaled_exp - 1023 - 64 + 1;
        return f64::from_bits((scaled_bits & 0x800FFFFFFFFFFFFF) | 0x3FE0000000000000);
    }

    // Inf or NaN
    if raw_exp == 0x7FF {
        *exp = 0;
        return x;
    }

    // Normal number: set exponent to -1 (biased: 1022 = 0x3FE)
    *exp = raw_exp - 1023 + 1;
    f64::from_bits((bits & 0x800FFFFFFFFFFFFF) | 0x3FE0000000000000)
}

/// Cube root via Newton-Raphson iteration.
///
/// Uses IEEE 754 bit manipulation for initial estimate,
/// then 3 iterations of Halley's method for full f64 precision.
pub fn cbrt(x: f64) -> f64 {
    if x == 0.0 || x != x {
        return x; // ±0.0, NaN
    }
    let negative = x < 0.0;
    let ax = if negative { -x } else { x };

    // Initial estimate: cbrt(x) ≈ 2^(log2(x)/3)
    // Manipulate the IEEE 754 exponent field
    let bits = ax.to_bits();
    let guess_bits = bits / 3 + (0x2AA0000000000000); // bias adjustment
    let mut y = f64::from_bits(guess_bits);

    // Halley's method: y = y * (y³ + 2*x) / (2*y³ + x) — cubic convergence
    for _ in 0..3 {
        let y3 = y * y * y;
        y = y * (y3 + ax + ax) / (y3 + y3 + ax);
    }

    if negative { -y } else { y }
}
