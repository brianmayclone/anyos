//! SIMD-accelerated vec4 operations using SSE packed instructions.
//!
//! `Vec4` wraps `__m128` — one XMM register holding 4 × f32. All arithmetic
//! maps to single packed SSE instructions instead of 4 scalar operations.

use core::arch::x86_64::*;

/// 128-bit SIMD-aligned vec4, maps directly to one XMM register.
#[repr(C, align(16))]
#[derive(Copy, Clone)]
pub struct Vec4(pub __m128);

impl Vec4 {
    /// Load 4 floats from memory (unaligned OK, but aligned is faster).
    #[inline(always)]
    pub fn load(v: &[f32; 4]) -> Self {
        Self(unsafe { _mm_loadu_ps(v.as_ptr()) })
    }

    /// Store 4 floats to memory.
    #[inline(always)]
    pub fn store(self, v: &mut [f32; 4]) {
        unsafe { _mm_storeu_ps(v.as_mut_ptr(), self.0) }
    }

    /// Broadcast a single f32 to all 4 lanes.
    #[inline(always)]
    pub fn splat(x: f32) -> Self {
        Self(unsafe { _mm_set1_ps(x) })
    }

    /// All zeros.
    #[inline(always)]
    pub fn zero() -> Self {
        Self(unsafe { _mm_setzero_ps() })
    }

    /// Packed add (addps — 1 cycle).
    #[inline(always)]
    pub fn add(self, b: Self) -> Self {
        Self(unsafe { _mm_add_ps(self.0, b.0) })
    }

    /// Packed subtract (subps — 1 cycle).
    #[inline(always)]
    pub fn sub(self, b: Self) -> Self {
        Self(unsafe { _mm_sub_ps(self.0, b.0) })
    }

    /// Packed multiply (mulps — 1 cycle).
    #[inline(always)]
    pub fn mul(self, b: Self) -> Self {
        Self(unsafe { _mm_mul_ps(self.0, b.0) })
    }

    /// Packed divide with zero protection.
    /// Lanes where `b == 0.0` produce `0.0` instead of NaN/inf.
    #[inline(always)]
    pub fn div_safe(self, b: Self) -> Self {
        unsafe {
            let zero = _mm_setzero_ps();
            let mask = _mm_cmpneq_ps(b.0, zero);
            let quot = _mm_div_ps(self.0, b.0);
            Self(_mm_and_ps(quot, mask))
        }
    }

    /// Packed negate (xorps with sign-bit mask — 1 cycle).
    #[inline(always)]
    pub fn neg(self) -> Self {
        unsafe {
            let sign = _mm_set1_ps(-0.0);
            Self(_mm_xor_ps(self.0, sign))
        }
    }

    /// Packed absolute value (andps with 0x7FFFFFFF mask — 1 cycle).
    #[inline(always)]
    pub fn abs(self) -> Self {
        unsafe {
            let mask = _mm_castsi128_ps(_mm_set1_epi32(0x7FFFFFFFu32 as i32));
            Self(_mm_and_ps(self.0, mask))
        }
    }

    /// Packed minimum (minps — 1 cycle).
    #[inline(always)]
    pub fn min(self, b: Self) -> Self {
        Self(unsafe { _mm_min_ps(self.0, b.0) })
    }

    /// Packed maximum (maxps — 1 cycle).
    #[inline(always)]
    pub fn max(self, b: Self) -> Self {
        Self(unsafe { _mm_max_ps(self.0, b.0) })
    }

    /// Packed clamp to [lo, hi] (maxps + minps — 2 cycles).
    #[inline(always)]
    pub fn clamp(self, lo: Self, hi: Self) -> Self {
        self.max(lo).min(hi)
    }

    /// Packed lerp: a + (b - a) * t (subps + mulps + addps — 3 cycles).
    #[inline(always)]
    pub fn lerp(self, b: Self, t: Self) -> Self {
        self.add(b.sub(self).mul(t))
    }

    /// Packed square root (sqrtps — ~11 cycles, but all 4 at once).
    #[inline(always)]
    pub fn sqrt(self) -> Self {
        Self(unsafe { _mm_sqrt_ps(self.0) })
    }

    /// Packed reciprocal square root (rsqrtps — fast ~1 cycle approximation).
    #[inline(always)]
    pub fn rsqrt(self) -> Self {
        Self(unsafe { _mm_rsqrt_ps(self.0) })
    }

    /// 3-component dot product, broadcast to all lanes.
    /// Uses mulps + 2× hadd (SSE3) or shuffle+add fallback.
    #[inline(always)]
    pub fn dp3(self, b: Self) -> Self {
        unsafe {
            let prod = _mm_mul_ps(self.0, b.0);
            // Zero out w component before summing
            let mask = _mm_castsi128_ps(_mm_set_epi32(0, -1, -1, -1));
            let prod = _mm_and_ps(prod, mask);
            // Horizontal sum: x+y, z+0, x+y, z+0
            let shuf1 = _mm_movehdup_ps(prod);  // [y, y, w, w]
            let sum1 = _mm_add_ps(prod, shuf1);  // [x+y, ?, z+w, ?]
            let shuf2 = _mm_movehl_ps(sum1, sum1); // [z+w, ?, ?, ?]
            let sum = _mm_add_ss(sum1, shuf2);
            Self(_mm_shuffle_ps(sum, sum, 0x00)) // broadcast
        }
    }

    /// 4-component dot product, broadcast to all lanes.
    #[inline(always)]
    pub fn dp4(self, b: Self) -> Self {
        unsafe {
            let prod = _mm_mul_ps(self.0, b.0);
            let shuf1 = _mm_movehdup_ps(prod);
            let sum1 = _mm_add_ps(prod, shuf1);
            let shuf2 = _mm_movehl_ps(sum1, sum1);
            let sum = _mm_add_ss(sum1, shuf2);
            Self(_mm_shuffle_ps(sum, sum, 0x00))
        }
    }

    /// Less-than comparison: returns 1.0 where true, 0.0 where false.
    #[inline(always)]
    pub fn cmp_lt(self, b: Self) -> Self {
        unsafe {
            let mask = _mm_cmplt_ps(self.0, b.0);
            Self(_mm_and_ps(mask, _mm_set1_ps(1.0)))
        }
    }

    /// Equality comparison (with epsilon): returns 1.0 where true, 0.0 where false.
    #[inline(always)]
    pub fn cmp_eq_eps(self, b: Self) -> Self {
        unsafe {
            let diff = _mm_sub_ps(self.0, b.0);
            let abs_mask = _mm_castsi128_ps(_mm_set1_epi32(0x7FFFFFFFu32 as i32));
            let abs_diff = _mm_and_ps(diff, abs_mask);
            let eps = _mm_set1_ps(1e-6);
            let mask = _mm_cmplt_ps(abs_diff, eps);
            Self(_mm_and_ps(mask, _mm_set1_ps(1.0)))
        }
    }

    /// Select: where cond != 0.0, pick `a`; else pick `b`.
    #[inline(always)]
    pub fn select(cond: Self, a: Self, b: Self) -> Self {
        unsafe {
            let zero = _mm_setzero_ps();
            let mask = _mm_cmpneq_ps(cond.0, zero);
            // blendvps equivalent: (a & mask) | (b & ~mask)
            let sel_a = _mm_and_ps(mask, a.0);
            let sel_b = _mm_andnot_ps(mask, b.0);
            Self(_mm_or_ps(sel_a, sel_b))
        }
    }

    /// Extract a single lane (0–3).
    #[inline(always)]
    pub fn lane(self, i: usize) -> f32 {
        let mut out = [0.0f32; 4];
        self.store(&mut out);
        out[i]
    }
}
