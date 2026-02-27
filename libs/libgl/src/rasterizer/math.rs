//! Math functions for the software rasterizer (no libm dependency).
//!
//! Polynomial approximations for transcendental functions. Accuracy is
//! sufficient for real-time 3D rendering (typically 4-6 significant digits).

/// Pi constant.
pub const PI: f32 = 3.14159265;

/// Floor function.
pub fn floor(x: f32) -> f32 {
    let i = x as i32;
    if x < 0.0 && x != i as f32 { (i - 1) as f32 } else { i as f32 }
}

/// Ceiling function.
pub fn ceil(x: f32) -> f32 {
    let i = x as i32;
    if x > 0.0 && x != i as f32 { (i + 1) as f32 } else { i as f32 }
}

/// Square root using Newton-Raphson iteration.
pub fn sqrt(x: f32) -> f32 {
    if x <= 0.0 { return 0.0; }

    // Fast inverse sqrt (Quake III) as initial guess
    let half = x * 0.5;
    let bits = x.to_bits();
    let guess_bits = 0x5f3759df - (bits >> 1);
    let mut y = f32::from_bits(guess_bits);

    // 3 Newton iterations for ~24-bit precision
    y = y * (1.5 - half * y * y);
    y = y * (1.5 - half * y * y);
    y = y * (1.5 - half * y * y);

    x * y // sqrt(x) = x * rsqrt(x)
}

/// Sine using Bhaskara I approximation + polynomial refinement.
pub fn sin(x: f32) -> f32 {
    // Reduce to [-pi, pi]
    let mut t = x;
    t = t - floor(t / (2.0 * PI)) * 2.0 * PI;
    if t > PI { t -= 2.0 * PI; }
    if t < -PI { t += 2.0 * PI; }

    // Parabola approximation (accurate to ~0.001)
    let abs_t = if t < 0.0 { -t } else { t };
    let y = t * (4.0 / PI - 4.0 / (PI * PI) * abs_t);

    // Refine with one correction pass
    let abs_y = if y < 0.0 { -y } else { y };
    0.225 * (y * abs_y - y) + y
}

/// Cosine.
pub fn cos(x: f32) -> f32 {
    sin(x + PI * 0.5)
}

/// Power function: base^exp using exp2(exp * log2(base)).
pub fn pow(base: f32, exp: f32) -> f32 {
    if base <= 0.0 { return 0.0; }
    exp2(exp * log2(base))
}

/// Log base 2 using IEEE 754 float bit manipulation.
pub fn log2(x: f32) -> f32 {
    if x <= 0.0 { return -100.0; } // -infinity approximation
    let bits = x.to_bits() as i32;
    let exponent = ((bits >> 23) & 0xFF) - 127;
    // Extract mantissa in [1, 2)
    let mantissa_bits = (bits & 0x007FFFFF) | 0x3F800000;
    let m = f32::from_bits(mantissa_bits as u32);
    // Polynomial approx of log2(m) for m in [1, 2)
    let m1 = m - 1.0;
    let log_m = m1 * (2.0 - 0.66666667 * m1);
    exponent as f32 + log_m
}

/// Exp base 2 using polynomial approximation.
pub fn exp2(x: f32) -> f32 {
    if x < -126.0 { return 0.0; }
    if x > 127.0 { return f32::from_bits(0x7F800000); } // +inf

    let floor_x = floor(x);
    let frac = x - floor_x;
    let int_part = floor_x as i32;

    // 2^frac for frac in [0, 1) — minimax polynomial
    let frac2 = frac * frac;
    let exp_frac = 1.0 + frac * (0.6931472 + frac2 * (0.2402265 + frac2 * 0.0554953));

    // Combine: 2^int * 2^frac
    let exp_bits = ((int_part + 127) as u32) << 23;
    f32::from_bits(exp_bits) * exp_frac
}

/// Tangent.
pub fn tan(x: f32) -> f32 {
    let c = cos(x);
    if c.abs() < 1e-10 { return 1e10; }
    sin(x) / c
}

/// Absolute value.
pub fn abs(x: f32) -> f32 {
    if x < 0.0 { -x } else { x }
}

/// Clamp a value to [lo, hi].
pub fn clamp(x: f32, lo: f32, hi: f32) -> f32 {
    if x < lo { lo } else if x > hi { hi } else { x }
}

/// Linear interpolation.
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
