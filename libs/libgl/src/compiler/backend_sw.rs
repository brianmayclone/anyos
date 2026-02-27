//! Software interpreter backend for compiled shader IR.
//!
//! Executes IR instructions on a register file of `[f32; 4]` vectors.
//! Uses packed SSE instructions via [`crate::simd::Vec4`] for 4-wide SIMD
//! execution — each arithmetic op processes all 4 components in a single
//! instruction instead of 4 scalar operations.

use alloc::vec;
use super::ir::*;
use crate::rasterizer::math;
use crate::simd::Vec4;

/// Register file: each register holds 4 floats (xyzw).
pub type RegFile = [[f32; 4]];

/// Execution context for one shader invocation.
pub struct ShaderExec {
    /// Register file.
    pub regs: alloc::vec::Vec<[f32; 4]>,
    /// gl_Position output (vertex shader).
    pub position: [f32; 4],
    /// gl_FragColor output (fragment shader).
    pub frag_color: [f32; 4],
    /// gl_PointSize output.
    pub point_size: f32,
    /// Varying outputs (vertex shader) / inputs (fragment shader).
    pub varyings: alloc::vec::Vec<[f32; 4]>,
}

/// Callback for texture sampling.
pub type TexSampleFn = fn(unit: u32, u: f32, v: f32) -> [f32; 4];

impl ShaderExec {
    /// Create a new execution context for the given program.
    pub fn new(num_regs: u32, num_varyings: usize) -> Self {
        Self {
            regs: vec![[0.0f32; 4]; num_regs as usize],
            position: [0.0; 4],
            frag_color: [0.0, 0.0, 0.0, 1.0],
            point_size: 1.0,
            varyings: vec![[0.0f32; 4]; num_varyings],
        }
    }

    /// Execute a compiled program.
    pub fn execute(
        &mut self,
        program: &Program,
        attributes: &[[f32; 4]],
        uniforms: &[[f32; 4]],
        varying_in: Option<&[[f32; 4]]>,
        tex_sample: TexSampleFn,
    ) {
        for inst in &program.instructions {
            self.exec_inst(inst, attributes, uniforms, varying_in, tex_sample);
        }
    }

    fn exec_inst(
        &mut self,
        inst: &Inst,
        attributes: &[[f32; 4]],
        uniforms: &[[f32; 4]],
        varying_in: Option<&[[f32; 4]]>,
        tex_sample: TexSampleFn,
    ) {
        match inst {
            Inst::LoadConst(dst, val) => {
                self.regs[*dst as usize] = *val;
            }
            Inst::Mov(dst, src) => {
                self.regs[*dst as usize] = self.regs[*src as usize];
            }
            // ── Packed SSE arithmetic ────────────────────────────────────
            Inst::Add(dst, a, b) => {
                let va = Vec4::load(&self.regs[*a as usize]);
                let vb = Vec4::load(&self.regs[*b as usize]);
                va.add(vb).store(&mut self.regs[*dst as usize]);
            }
            Inst::Sub(dst, a, b) => {
                let va = Vec4::load(&self.regs[*a as usize]);
                let vb = Vec4::load(&self.regs[*b as usize]);
                va.sub(vb).store(&mut self.regs[*dst as usize]);
            }
            Inst::Mul(dst, a, b) => {
                let va = Vec4::load(&self.regs[*a as usize]);
                let vb = Vec4::load(&self.regs[*b as usize]);
                va.mul(vb).store(&mut self.regs[*dst as usize]);
            }
            Inst::Div(dst, a, b) => {
                let va = Vec4::load(&self.regs[*a as usize]);
                let vb = Vec4::load(&self.regs[*b as usize]);
                va.div_safe(vb).store(&mut self.regs[*dst as usize]);
            }
            Inst::Neg(dst, src) => {
                Vec4::load(&self.regs[*src as usize])
                    .neg()
                    .store(&mut self.regs[*dst as usize]);
            }
            Inst::Abs(dst, src) => {
                Vec4::load(&self.regs[*src as usize])
                    .abs()
                    .store(&mut self.regs[*dst as usize]);
            }
            Inst::Min(dst, a, b) => {
                let va = Vec4::load(&self.regs[*a as usize]);
                let vb = Vec4::load(&self.regs[*b as usize]);
                va.min(vb).store(&mut self.regs[*dst as usize]);
            }
            Inst::Max(dst, a, b) => {
                let va = Vec4::load(&self.regs[*a as usize]);
                let vb = Vec4::load(&self.regs[*b as usize]);
                va.max(vb).store(&mut self.regs[*dst as usize]);
            }
            Inst::Clamp(dst, x, lo, hi) => {
                let vx = Vec4::load(&self.regs[*x as usize]);
                let vlo = Vec4::load(&self.regs[*lo as usize]);
                let vhi = Vec4::load(&self.regs[*hi as usize]);
                vx.clamp(vlo, vhi).store(&mut self.regs[*dst as usize]);
            }
            Inst::Mix(dst, a, b, t) => {
                let va = Vec4::load(&self.regs[*a as usize]);
                let vb = Vec4::load(&self.regs[*b as usize]);
                let vt = Vec4::load(&self.regs[*t as usize]);
                va.lerp(vb, vt).store(&mut self.regs[*dst as usize]);
            }
            // ── Dot products ─────────────────────────────────────────────
            Inst::Dp3(dst, a, b) => {
                let va = Vec4::load(&self.regs[*a as usize]);
                let vb = Vec4::load(&self.regs[*b as usize]);
                va.dp3(vb).store(&mut self.regs[*dst as usize]);
            }
            Inst::Dp4(dst, a, b) => {
                let va = Vec4::load(&self.regs[*a as usize]);
                let vb = Vec4::load(&self.regs[*b as usize]);
                va.dp4(vb).store(&mut self.regs[*dst as usize]);
            }
            // ── Cross product ────────────────────────────────────────────
            Inst::Cross(dst, a, b) => {
                let ra = self.regs[*a as usize];
                let rb = self.regs[*b as usize];
                self.regs[*dst as usize] = [
                    ra[1]*rb[2] - ra[2]*rb[1],
                    ra[2]*rb[0] - ra[0]*rb[2],
                    ra[0]*rb[1] - ra[1]*rb[0],
                    0.0,
                ];
            }
            // ── Normalize (dp3 + rsqrt + mul) ────────────────────────────
            Inst::Normalize(dst, src) => {
                let v = Vec4::load(&self.regs[*src as usize]);
                let dp = v.dp3(v);
                let len_sq = dp.lane(0);
                if len_sq > 1e-20 {
                    let inv_len = Vec4::splat(1.0 / math::sqrt(len_sq));
                    v.mul(inv_len).store(&mut self.regs[*dst as usize]);
                } else {
                    self.regs[*dst as usize] = [0.0; 4];
                }
            }
            Inst::Length(dst, src) => {
                let v = Vec4::load(&self.regs[*src as usize]);
                let dp = v.dp3(v);
                let len = math::sqrt(dp.lane(0));
                self.regs[*dst as usize] = [len, len, len, len];
            }
            Inst::Reflect(dst, i, n) => {
                let vi = Vec4::load(&self.regs[*i as usize]);
                let vn = Vec4::load(&self.regs[*n as usize]);
                let dp = vi.dp3(vn);
                let two_dp = dp.add(dp);
                let result = vi.sub(vn.mul(two_dp));
                let mut out = [0.0f32; 4];
                result.store(&mut out);
                out[3] = 0.0;
                self.regs[*dst as usize] = out;
            }
            // ── Packed sqrt ──────────────────────────────────────────────
            Inst::Sqrt(dst, src) => {
                Vec4::load(&self.regs[*src as usize])
                    .sqrt()
                    .store(&mut self.regs[*dst as usize]);
            }
            Inst::Rsqrt(dst, src) => {
                let r = self.regs[*src as usize];
                self.regs[*dst as usize] = [
                    if r[0] > 1e-10 { 1.0 / math::sqrt(r[0]) } else { 0.0 },
                    if r[1] > 1e-10 { 1.0 / math::sqrt(r[1]) } else { 0.0 },
                    if r[2] > 1e-10 { 1.0 / math::sqrt(r[2]) } else { 0.0 },
                    if r[3] > 1e-10 { 1.0 / math::sqrt(r[3]) } else { 0.0 },
                ];
            }
            // ── Transcendentals (x87 FPU, per-component) ─────────────────
            Inst::Floor(dst, src) => {
                let r = self.regs[*src as usize];
                self.regs[*dst as usize] = [
                    math::floor(r[0]), math::floor(r[1]),
                    math::floor(r[2]), math::floor(r[3]),
                ];
            }
            Inst::Fract(dst, src) => {
                let r = self.regs[*src as usize];
                self.regs[*dst as usize] = [
                    r[0] - math::floor(r[0]),
                    r[1] - math::floor(r[1]),
                    r[2] - math::floor(r[2]),
                    r[3] - math::floor(r[3]),
                ];
            }
            Inst::Pow(dst, base, exp) => {
                let rb = self.regs[*base as usize];
                let re = self.regs[*exp as usize];
                self.regs[*dst as usize] = [
                    math::pow(rb[0], re[0]),
                    math::pow(rb[1], re[1]),
                    math::pow(rb[2], re[2]),
                    math::pow(rb[3], re[3]),
                ];
            }
            Inst::Sin(dst, src) => {
                let r = self.regs[*src as usize];
                self.regs[*dst as usize] = [
                    math::sin(r[0]), math::sin(r[1]),
                    math::sin(r[2]), math::sin(r[3]),
                ];
            }
            Inst::Cos(dst, src) => {
                let r = self.regs[*src as usize];
                self.regs[*dst as usize] = [
                    math::cos(r[0]), math::cos(r[1]),
                    math::cos(r[2]), math::cos(r[3]),
                ];
            }
            // ── Texture sampling ─────────────────────────────────────────
            Inst::TexSample(dst, sampler, coord) => {
                let unit = self.regs[*sampler as usize][0] as u32;
                let uv = self.regs[*coord as usize];
                self.regs[*dst as usize] = tex_sample(unit, uv[0], uv[1]);
            }
            // ── Matrix multiply (SIMD: splat + mul + add chain) ──────────
            Inst::MatMul4(dst, mat, vec) => {
                let v = Vec4::load(&self.regs[*vec as usize]);
                let c0 = Vec4::load(&self.regs[*mat as usize]);
                let c1 = Vec4::load(&self.regs[(*mat + 1) as usize]);
                let c2 = Vec4::load(&self.regs[(*mat + 2) as usize]);
                let c3 = Vec4::load(&self.regs[(*mat + 3) as usize]);
                unsafe {
                    use core::arch::x86_64::*;
                    let vx = _mm_shuffle_ps(v.0, v.0, 0x00);
                    let vy = _mm_shuffle_ps(v.0, v.0, 0x55);
                    let vz = _mm_shuffle_ps(v.0, v.0, 0xAA);
                    let vw = _mm_shuffle_ps(v.0, v.0, 0xFF);
                    let r = _mm_add_ps(
                        _mm_add_ps(_mm_mul_ps(c0.0, vx), _mm_mul_ps(c1.0, vy)),
                        _mm_add_ps(_mm_mul_ps(c2.0, vz), _mm_mul_ps(c3.0, vw)),
                    );
                    Vec4(r).store(&mut self.regs[*dst as usize]);
                }
            }
            Inst::MatMul3(dst, mat, vec) => {
                let v = Vec4::load(&self.regs[*vec as usize]);
                let c0 = Vec4::load(&self.regs[*mat as usize]);
                let c1 = Vec4::load(&self.regs[(*mat + 1) as usize]);
                let c2 = Vec4::load(&self.regs[(*mat + 2) as usize]);
                unsafe {
                    use core::arch::x86_64::*;
                    let vx = _mm_shuffle_ps(v.0, v.0, 0x00);
                    let vy = _mm_shuffle_ps(v.0, v.0, 0x55);
                    let vz = _mm_shuffle_ps(v.0, v.0, 0xAA);
                    let r = _mm_add_ps(
                        _mm_add_ps(_mm_mul_ps(c0.0, vx), _mm_mul_ps(c1.0, vy)),
                        _mm_mul_ps(c2.0, vz),
                    );
                    let mask = _mm_castsi128_ps(_mm_set_epi32(0, -1, -1, -1));
                    Vec4(_mm_and_ps(r, mask)).store(&mut self.regs[*dst as usize]);
                }
            }
            // ── Swizzle / WriteMask ──────────────────────────────────────
            Inst::Swizzle(dst, src, indices, count) => {
                let r = self.regs[*src as usize];
                let mut out = [0.0f32; 4];
                for i in 0..(*count as usize) {
                    out[i] = r[indices[i] as usize];
                }
                if *count == 1 {
                    out = [out[0]; 4];
                }
                self.regs[*dst as usize] = out;
            }
            Inst::WriteMask(dst, src, mask) => {
                let s = self.regs[*src as usize];
                let d = &mut self.regs[*dst as usize];
                if mask & 0x1 != 0 { d[0] = s[0]; }
                if mask & 0x2 != 0 { d[1] = s[1]; }
                if mask & 0x4 != 0 { d[2] = s[2]; }
                if mask & 0x8 != 0 { d[3] = s[3]; }
            }
            // ── Comparisons (packed SSE) ─────────────────────────────────
            Inst::CmpLt(dst, a, b) => {
                let va = Vec4::load(&self.regs[*a as usize]);
                let vb = Vec4::load(&self.regs[*b as usize]);
                va.cmp_lt(vb).store(&mut self.regs[*dst as usize]);
            }
            Inst::CmpEq(dst, a, b) => {
                let va = Vec4::load(&self.regs[*a as usize]);
                let vb = Vec4::load(&self.regs[*b as usize]);
                va.cmp_eq_eps(vb).store(&mut self.regs[*dst as usize]);
            }
            Inst::Select(dst, cond, a, b) => {
                let vc = Vec4::load(&self.regs[*cond as usize]);
                let va = Vec4::load(&self.regs[*a as usize]);
                let vb = Vec4::load(&self.regs[*b as usize]);
                Vec4::select(vc, va, vb).store(&mut self.regs[*dst as usize]);
            }
            // ── Type conversions ─────────────────────────────────────────
            Inst::IntToFloat(dst, src) => {
                self.regs[*dst as usize] = self.regs[*src as usize];
            }
            Inst::FloatToInt(dst, src) => {
                let r = self.regs[*src as usize];
                self.regs[*dst as usize] = [
                    (r[0] as i32) as f32,
                    (r[1] as i32) as f32,
                    (r[2] as i32) as f32,
                    (r[3] as i32) as f32,
                ];
            }
            // ── I/O ──────────────────────────────────────────────────────
            Inst::StorePosition(src) => {
                self.position = self.regs[*src as usize];
            }
            Inst::StoreFragColor(src) => {
                self.frag_color = self.regs[*src as usize];
            }
            Inst::StorePointSize(src) => {
                self.point_size = self.regs[*src as usize][0];
            }
            Inst::LoadVarying(dst, idx) => {
                if let Some(vi) = varying_in {
                    if (*idx as usize) < vi.len() {
                        self.regs[*dst as usize] = vi[*idx as usize];
                    }
                } else if (*idx as usize) < self.varyings.len() {
                    self.regs[*dst as usize] = self.varyings[*idx as usize];
                }
            }
            Inst::StoreVarying(idx, src) => {
                let val = self.regs[*src as usize];
                if (*idx as usize) < self.varyings.len() {
                    self.varyings[*idx as usize] = val;
                } else {
                    while self.varyings.len() <= *idx as usize {
                        self.varyings.push([0.0; 4]);
                    }
                    self.varyings[*idx as usize] = val;
                }
            }
            Inst::LoadUniform(dst, idx) => {
                if (*idx as usize) < uniforms.len() {
                    self.regs[*dst as usize] = uniforms[*idx as usize];
                }
            }
            Inst::LoadAttribute(dst, idx) => {
                if (*idx as usize) < attributes.len() {
                    self.regs[*dst as usize] = attributes[*idx as usize];
                }
            }
        }
    }
}
