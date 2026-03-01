//! Vec4 operations using scalar f32 arithmetic.
//!
//! `Vec4` wraps `[f32; 4]` — portable pure-Rust implementation that works
//! on both x86_64 and aarch64. On x86_64 the compiler auto-vectorizes
//! most of these into SSE instructions anyway.

/// 128-bit aligned vec4, stored as 4 × f32.
#[repr(C, align(16))]
#[derive(Copy, Clone)]
pub struct Vec4(pub [f32; 4]);

impl Vec4 {
    /// Load 4 floats from memory.
    #[inline(always)]
    pub fn load(v: &[f32; 4]) -> Self {
        Self(*v)
    }

    /// Store 4 floats to memory.
    #[inline(always)]
    pub fn store(self, v: &mut [f32; 4]) {
        *v = self.0;
    }

    /// Broadcast a single f32 to all 4 lanes.
    #[inline(always)]
    pub fn splat(x: f32) -> Self {
        Self([x, x, x, x])
    }

    /// All zeros.
    #[inline(always)]
    pub fn zero() -> Self {
        Self([0.0; 4])
    }

    /// Packed add.
    #[inline(always)]
    pub fn add(self, b: Self) -> Self {
        Self([
            self.0[0] + b.0[0],
            self.0[1] + b.0[1],
            self.0[2] + b.0[2],
            self.0[3] + b.0[3],
        ])
    }

    /// Packed subtract.
    #[inline(always)]
    pub fn sub(self, b: Self) -> Self {
        Self([
            self.0[0] - b.0[0],
            self.0[1] - b.0[1],
            self.0[2] - b.0[2],
            self.0[3] - b.0[3],
        ])
    }

    /// Packed multiply.
    #[inline(always)]
    pub fn mul(self, b: Self) -> Self {
        Self([
            self.0[0] * b.0[0],
            self.0[1] * b.0[1],
            self.0[2] * b.0[2],
            self.0[3] * b.0[3],
        ])
    }

    /// Packed divide with zero protection.
    /// Lanes where `b == 0.0` produce `0.0` instead of NaN/inf.
    #[inline(always)]
    pub fn div_safe(self, b: Self) -> Self {
        Self([
            if b.0[0] != 0.0 { self.0[0] / b.0[0] } else { 0.0 },
            if b.0[1] != 0.0 { self.0[1] / b.0[1] } else { 0.0 },
            if b.0[2] != 0.0 { self.0[2] / b.0[2] } else { 0.0 },
            if b.0[3] != 0.0 { self.0[3] / b.0[3] } else { 0.0 },
        ])
    }

    /// Packed negate.
    #[inline(always)]
    pub fn neg(self) -> Self {
        Self([-self.0[0], -self.0[1], -self.0[2], -self.0[3]])
    }

    /// Packed absolute value.
    #[inline(always)]
    pub fn abs(self) -> Self {
        Self([
            f32::from_bits(self.0[0].to_bits() & 0x7FFFFFFF),
            f32::from_bits(self.0[1].to_bits() & 0x7FFFFFFF),
            f32::from_bits(self.0[2].to_bits() & 0x7FFFFFFF),
            f32::from_bits(self.0[3].to_bits() & 0x7FFFFFFF),
        ])
    }

    /// Packed minimum.
    #[inline(always)]
    pub fn min(self, b: Self) -> Self {
        Self([
            if self.0[0] < b.0[0] { self.0[0] } else { b.0[0] },
            if self.0[1] < b.0[1] { self.0[1] } else { b.0[1] },
            if self.0[2] < b.0[2] { self.0[2] } else { b.0[2] },
            if self.0[3] < b.0[3] { self.0[3] } else { b.0[3] },
        ])
    }

    /// Packed maximum.
    #[inline(always)]
    pub fn max(self, b: Self) -> Self {
        Self([
            if self.0[0] > b.0[0] { self.0[0] } else { b.0[0] },
            if self.0[1] > b.0[1] { self.0[1] } else { b.0[1] },
            if self.0[2] > b.0[2] { self.0[2] } else { b.0[2] },
            if self.0[3] > b.0[3] { self.0[3] } else { b.0[3] },
        ])
    }

    /// Packed clamp to [lo, hi].
    #[inline(always)]
    pub fn clamp(self, lo: Self, hi: Self) -> Self {
        self.max(lo).min(hi)
    }

    /// Packed lerp: a + (b - a) * t.
    #[inline(always)]
    pub fn lerp(self, b: Self, t: Self) -> Self {
        self.add(b.sub(self).mul(t))
    }

    /// Packed square root.
    #[inline(always)]
    pub fn sqrt(self) -> Self {
        Self([
            scalar_sqrt(self.0[0]),
            scalar_sqrt(self.0[1]),
            scalar_sqrt(self.0[2]),
            scalar_sqrt(self.0[3]),
        ])
    }

    /// Packed reciprocal square root (fast approximation).
    #[inline(always)]
    pub fn rsqrt(self) -> Self {
        Self([
            fast_inv_sqrt(self.0[0]),
            fast_inv_sqrt(self.0[1]),
            fast_inv_sqrt(self.0[2]),
            fast_inv_sqrt(self.0[3]),
        ])
    }

    /// 3-component dot product, broadcast to all lanes.
    #[inline(always)]
    pub fn dp3(self, b: Self) -> Self {
        let d = self.0[0] * b.0[0] + self.0[1] * b.0[1] + self.0[2] * b.0[2];
        Self([d, d, d, d])
    }

    /// 4-component dot product, broadcast to all lanes.
    #[inline(always)]
    pub fn dp4(self, b: Self) -> Self {
        let d = self.0[0] * b.0[0] + self.0[1] * b.0[1]
              + self.0[2] * b.0[2] + self.0[3] * b.0[3];
        Self([d, d, d, d])
    }

    /// Less-than comparison: returns 1.0 where true, 0.0 where false.
    #[inline(always)]
    pub fn cmp_lt(self, b: Self) -> Self {
        Self([
            if self.0[0] < b.0[0] { 1.0 } else { 0.0 },
            if self.0[1] < b.0[1] { 1.0 } else { 0.0 },
            if self.0[2] < b.0[2] { 1.0 } else { 0.0 },
            if self.0[3] < b.0[3] { 1.0 } else { 0.0 },
        ])
    }

    /// Equality comparison (with epsilon): returns 1.0 where true, 0.0 where false.
    #[inline(always)]
    pub fn cmp_eq_eps(self, b: Self) -> Self {
        const EPS: f32 = 1e-6;
        Self([
            if abs_f32(self.0[0] - b.0[0]) < EPS { 1.0 } else { 0.0 },
            if abs_f32(self.0[1] - b.0[1]) < EPS { 1.0 } else { 0.0 },
            if abs_f32(self.0[2] - b.0[2]) < EPS { 1.0 } else { 0.0 },
            if abs_f32(self.0[3] - b.0[3]) < EPS { 1.0 } else { 0.0 },
        ])
    }

    /// Select: where cond != 0.0, pick `a`; else pick `b`.
    #[inline(always)]
    pub fn select(cond: Self, a: Self, b: Self) -> Self {
        Self([
            if cond.0[0] != 0.0 { a.0[0] } else { b.0[0] },
            if cond.0[1] != 0.0 { a.0[1] } else { b.0[1] },
            if cond.0[2] != 0.0 { a.0[2] } else { b.0[2] },
            if cond.0[3] != 0.0 { a.0[3] } else { b.0[3] },
        ])
    }

    /// Extract a single lane (0–3).
    #[inline(always)]
    pub fn lane(self, i: usize) -> f32 {
        self.0[i]
    }
}

/// Scalar absolute value via bit manipulation.
#[inline(always)]
fn abs_f32(x: f32) -> f32 {
    f32::from_bits(x.to_bits() & 0x7FFFFFFF)
}

/// Scalar square root via Newton-Raphson (2 iterations, ~23-bit accuracy).
#[inline(always)]
fn scalar_sqrt(x: f32) -> f32 {
    if x <= 0.0 { return 0.0; }
    // Initial guess from float bit manipulation
    let mut y = f32::from_bits((x.to_bits() >> 1) + 0x1FC00000);
    y = 0.5 * (y + x / y);
    y = 0.5 * (y + x / y);
    y
}

/// Fast inverse square root (Quake III style + Newton-Raphson refinement).
#[inline(always)]
pub fn fast_inv_sqrt(x: f32) -> f32 {
    if x <= 0.0 { return 0.0; }
    let half = 0.5 * x;
    let i = 0x5F3759DF_u32.wrapping_sub(x.to_bits() >> 1);
    let mut y = f32::from_bits(i);
    y = y * (1.5 - half * y * y);
    y
}
