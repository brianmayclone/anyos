//! Vec4 operations used by the shader interpreter and rasterizer hot paths.
//!
//! On x86_64 we use explicit SSE intrinsics so this layer actually executes on
//! XMM registers instead of relying on auto-vectorization. Other targets keep
//! the portable scalar fallback.

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

#[cfg(target_arch = "x86_64")]
#[repr(transparent)]
#[derive(Copy, Clone)]
pub struct Vec4(__m128);

#[cfg(not(target_arch = "x86_64"))]
#[repr(C, align(16))]
#[derive(Copy, Clone)]
pub struct Vec4([f32; 4]);

impl Vec4 {
    #[inline(always)]
    pub fn load(v: &[f32; 4]) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self(_mm_loadu_ps(v.as_ptr()))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self(*v)
        }
    }

    #[inline(always)]
    pub fn store(self, v: &mut [f32; 4]) {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            _mm_storeu_ps(v.as_mut_ptr(), self.0);
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            *v = self.0;
        }
    }

    #[inline(always)]
    pub fn splat(x: f32) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self(_mm_set1_ps(x))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self([x, x, x, x])
        }
    }

    #[inline(always)]
    pub fn zero() -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self(_mm_setzero_ps())
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self([0.0; 4])
        }
    }

    #[inline(always)]
    pub fn add(self, b: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self(_mm_add_ps(self.0, b.0))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self([
                self.0[0] + b.0[0],
                self.0[1] + b.0[1],
                self.0[2] + b.0[2],
                self.0[3] + b.0[3],
            ])
        }
    }

    #[inline(always)]
    pub fn sub(self, b: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self(_mm_sub_ps(self.0, b.0))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self([
                self.0[0] - b.0[0],
                self.0[1] - b.0[1],
                self.0[2] - b.0[2],
                self.0[3] - b.0[3],
            ])
        }
    }

    #[inline(always)]
    pub fn mul(self, b: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self(_mm_mul_ps(self.0, b.0))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self([
                self.0[0] * b.0[0],
                self.0[1] * b.0[1],
                self.0[2] * b.0[2],
                self.0[3] * b.0[3],
            ])
        }
    }

    #[inline(always)]
    pub fn div_safe(self, b: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            let zero = _mm_setzero_ps();
            let mask = _mm_cmpneq_ps(b.0, zero);
            Self(_mm_and_ps(_mm_div_ps(self.0, b.0), mask))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self([
                if b.0[0] != 0.0 { self.0[0] / b.0[0] } else { 0.0 },
                if b.0[1] != 0.0 { self.0[1] / b.0[1] } else { 0.0 },
                if b.0[2] != 0.0 { self.0[2] / b.0[2] } else { 0.0 },
                if b.0[3] != 0.0 { self.0[3] / b.0[3] } else { 0.0 },
            ])
        }
    }

    #[inline(always)]
    pub fn neg(self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self(_mm_xor_ps(self.0, _mm_set1_ps(-0.0)))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self([-self.0[0], -self.0[1], -self.0[2], -self.0[3]])
        }
    }

    #[inline(always)]
    pub fn abs(self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            let mask = _mm_castsi128_ps(_mm_set1_epi32(0x7FFF_FFFF_u32 as i32));
            Self(_mm_and_ps(self.0, mask))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self([
                f32::from_bits(self.0[0].to_bits() & 0x7FFF_FFFF),
                f32::from_bits(self.0[1].to_bits() & 0x7FFF_FFFF),
                f32::from_bits(self.0[2].to_bits() & 0x7FFF_FFFF),
                f32::from_bits(self.0[3].to_bits() & 0x7FFF_FFFF),
            ])
        }
    }

    #[inline(always)]
    pub fn min(self, b: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self(_mm_min_ps(self.0, b.0))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self([
                if self.0[0] < b.0[0] { self.0[0] } else { b.0[0] },
                if self.0[1] < b.0[1] { self.0[1] } else { b.0[1] },
                if self.0[2] < b.0[2] { self.0[2] } else { b.0[2] },
                if self.0[3] < b.0[3] { self.0[3] } else { b.0[3] },
            ])
        }
    }

    #[inline(always)]
    pub fn max(self, b: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self(_mm_max_ps(self.0, b.0))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self([
                if self.0[0] > b.0[0] { self.0[0] } else { b.0[0] },
                if self.0[1] > b.0[1] { self.0[1] } else { b.0[1] },
                if self.0[2] > b.0[2] { self.0[2] } else { b.0[2] },
                if self.0[3] > b.0[3] { self.0[3] } else { b.0[3] },
            ])
        }
    }

    #[inline(always)]
    pub fn clamp(self, lo: Self, hi: Self) -> Self {
        self.max(lo).min(hi)
    }

    #[inline(always)]
    pub fn lerp(self, b: Self, t: Self) -> Self {
        self.add(b.sub(self).mul(t))
    }

    #[inline(always)]
    pub fn sqrt(self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self(_mm_sqrt_ps(self.0))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self([
                scalar_sqrt(self.0[0]),
                scalar_sqrt(self.0[1]),
                scalar_sqrt(self.0[2]),
                scalar_sqrt(self.0[3]),
            ])
        }
    }

    #[inline(always)]
    pub fn rsqrt(self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            let y = _mm_rsqrt_ps(self.0);
            let half = _mm_set1_ps(0.5);
            let three_halves = _mm_set1_ps(1.5);
            let y2 = _mm_mul_ps(y, y);
            let x_half_y2 = _mm_mul_ps(_mm_mul_ps(self.0, half), y2);
            Self(_mm_mul_ps(y, _mm_sub_ps(three_halves, x_half_y2)))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self([
                fast_inv_sqrt(self.0[0]),
                fast_inv_sqrt(self.0[1]),
                fast_inv_sqrt(self.0[2]),
                fast_inv_sqrt(self.0[3]),
            ])
        }
    }

    #[inline(always)]
    pub fn dp3(self, b: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self(_mm_dp_ps(self.0, b.0, 0x7F))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            let d = self.0[0] * b.0[0] + self.0[1] * b.0[1] + self.0[2] * b.0[2];
            Self([d, d, d, d])
        }
    }

    #[inline(always)]
    pub fn dp4(self, b: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self(_mm_dp_ps(self.0, b.0, 0xFF))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            let d = self.0[0] * b.0[0]
                + self.0[1] * b.0[1]
                + self.0[2] * b.0[2]
                + self.0[3] * b.0[3];
            Self([d, d, d, d])
        }
    }

    #[inline(always)]
    pub fn cmp_lt(self, b: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            let mask = _mm_cmplt_ps(self.0, b.0);
            Self(_mm_and_ps(mask, _mm_set1_ps(1.0)))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self([
                if self.0[0] < b.0[0] { 1.0 } else { 0.0 },
                if self.0[1] < b.0[1] { 1.0 } else { 0.0 },
                if self.0[2] < b.0[2] { 1.0 } else { 0.0 },
                if self.0[3] < b.0[3] { 1.0 } else { 0.0 },
            ])
        }
    }

    #[inline(always)]
    pub fn cmp_eq_eps(self, b: Self) -> Self {
        const EPS: f32 = 1e-6;
        #[cfg(target_arch = "x86_64")]
        unsafe {
            let diff = _mm_sub_ps(self.0, b.0);
            let mask = _mm_castsi128_ps(_mm_set1_epi32(0x7FFF_FFFF_u32 as i32));
            let abs_diff = _mm_and_ps(diff, mask);
            let cmp = _mm_cmplt_ps(abs_diff, _mm_set1_ps(EPS));
            Self(_mm_and_ps(cmp, _mm_set1_ps(1.0)))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self([
                if abs_f32(self.0[0] - b.0[0]) < EPS { 1.0 } else { 0.0 },
                if abs_f32(self.0[1] - b.0[1]) < EPS { 1.0 } else { 0.0 },
                if abs_f32(self.0[2] - b.0[2]) < EPS { 1.0 } else { 0.0 },
                if abs_f32(self.0[3] - b.0[3]) < EPS { 1.0 } else { 0.0 },
            ])
        }
    }

    #[inline(always)]
    pub fn select(cond: Self, a: Self, b: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            let mask = _mm_cmpneq_ps(cond.0, _mm_setzero_ps());
            Self(_mm_or_ps(_mm_and_ps(mask, a.0), _mm_andnot_ps(mask, b.0)))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self([
                if cond.0[0] != 0.0 { a.0[0] } else { b.0[0] },
                if cond.0[1] != 0.0 { a.0[1] } else { b.0[1] },
                if cond.0[2] != 0.0 { a.0[2] } else { b.0[2] },
                if cond.0[3] != 0.0 { a.0[3] } else { b.0[3] },
            ])
        }
    }

    #[inline(always)]
    pub fn lane(self, i: usize) -> f32 {
        #[cfg(target_arch = "x86_64")]
        {
            let mut out = [0.0f32; 4];
            self.store(&mut out);
            out[i]
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            self.0[i]
        }
    }
}

#[inline(always)]
fn abs_f32(x: f32) -> f32 {
    f32::from_bits(x.to_bits() & 0x7FFF_FFFF)
}

#[inline(always)]
fn scalar_sqrt(x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    let mut y = f32::from_bits((x.to_bits() >> 1) + 0x1FC0_0000);
    y = 0.5 * (y + x / y);
    y = 0.5 * (y + x / y);
    y
}

#[inline(always)]
pub fn fast_inv_sqrt(x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    let half = 0.5 * x;
    let i = 0x5F37_59DF_u32.wrapping_sub(x.to_bits() >> 1);
    let mut y = f32::from_bits(i);
    y = y * (1.5 - half * y * y);
    y
}
