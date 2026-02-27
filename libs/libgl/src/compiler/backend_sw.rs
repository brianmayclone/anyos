//! Software interpreter backend for compiled shader IR.
//!
//! Executes IR instructions on a register file of `[f32; 4]` vectors.
//! Used by the software rasterizer to run both vertex and fragment shaders.

use alloc::vec;
use super::ir::*;
use crate::rasterizer::math;

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
            Inst::Add(dst, a, b) => {
                let ra = self.regs[*a as usize];
                let rb = self.regs[*b as usize];
                self.regs[*dst as usize] = [ra[0]+rb[0], ra[1]+rb[1], ra[2]+rb[2], ra[3]+rb[3]];
            }
            Inst::Sub(dst, a, b) => {
                let ra = self.regs[*a as usize];
                let rb = self.regs[*b as usize];
                self.regs[*dst as usize] = [ra[0]-rb[0], ra[1]-rb[1], ra[2]-rb[2], ra[3]-rb[3]];
            }
            Inst::Mul(dst, a, b) => {
                let ra = self.regs[*a as usize];
                let rb = self.regs[*b as usize];
                self.regs[*dst as usize] = [ra[0]*rb[0], ra[1]*rb[1], ra[2]*rb[2], ra[3]*rb[3]];
            }
            Inst::Div(dst, a, b) => {
                let ra = self.regs[*a as usize];
                let rb = self.regs[*b as usize];
                self.regs[*dst as usize] = [
                    if rb[0] != 0.0 { ra[0]/rb[0] } else { 0.0 },
                    if rb[1] != 0.0 { ra[1]/rb[1] } else { 0.0 },
                    if rb[2] != 0.0 { ra[2]/rb[2] } else { 0.0 },
                    if rb[3] != 0.0 { ra[3]/rb[3] } else { 0.0 },
                ];
            }
            Inst::Neg(dst, src) => {
                let r = self.regs[*src as usize];
                self.regs[*dst as usize] = [-r[0], -r[1], -r[2], -r[3]];
            }
            Inst::Dp3(dst, a, b) => {
                let ra = self.regs[*a as usize];
                let rb = self.regs[*b as usize];
                let d = ra[0]*rb[0] + ra[1]*rb[1] + ra[2]*rb[2];
                self.regs[*dst as usize] = [d, d, d, d];
            }
            Inst::Dp4(dst, a, b) => {
                let ra = self.regs[*a as usize];
                let rb = self.regs[*b as usize];
                let d = ra[0]*rb[0] + ra[1]*rb[1] + ra[2]*rb[2] + ra[3]*rb[3];
                self.regs[*dst as usize] = [d, d, d, d];
            }
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
            Inst::Normalize(dst, src) => {
                let r = self.regs[*src as usize];
                let len = math::sqrt(r[0]*r[0] + r[1]*r[1] + r[2]*r[2]);
                if len > 1e-10 {
                    let inv = 1.0 / len;
                    self.regs[*dst as usize] = [r[0]*inv, r[1]*inv, r[2]*inv, r[3]*inv];
                } else {
                    self.regs[*dst as usize] = [0.0; 4];
                }
            }
            Inst::Length(dst, src) => {
                let r = self.regs[*src as usize];
                let len = math::sqrt(r[0]*r[0] + r[1]*r[1] + r[2]*r[2]);
                self.regs[*dst as usize] = [len, len, len, len];
            }
            Inst::Min(dst, a, b) => {
                let ra = self.regs[*a as usize];
                let rb = self.regs[*b as usize];
                self.regs[*dst as usize] = [
                    if ra[0] < rb[0] { ra[0] } else { rb[0] },
                    if ra[1] < rb[1] { ra[1] } else { rb[1] },
                    if ra[2] < rb[2] { ra[2] } else { rb[2] },
                    if ra[3] < rb[3] { ra[3] } else { rb[3] },
                ];
            }
            Inst::Max(dst, a, b) => {
                let ra = self.regs[*a as usize];
                let rb = self.regs[*b as usize];
                self.regs[*dst as usize] = [
                    if ra[0] > rb[0] { ra[0] } else { rb[0] },
                    if ra[1] > rb[1] { ra[1] } else { rb[1] },
                    if ra[2] > rb[2] { ra[2] } else { rb[2] },
                    if ra[3] > rb[3] { ra[3] } else { rb[3] },
                ];
            }
            Inst::Clamp(dst, x, lo, hi) => {
                let rx = self.regs[*x as usize];
                let rlo = self.regs[*lo as usize];
                let rhi = self.regs[*hi as usize];
                self.regs[*dst as usize] = [
                    clamp_f32(rx[0], rlo[0], rhi[0]),
                    clamp_f32(rx[1], rlo[1], rhi[1]),
                    clamp_f32(rx[2], rlo[2], rhi[2]),
                    clamp_f32(rx[3], rlo[3], rhi[3]),
                ];
            }
            Inst::Mix(dst, a, b, t) => {
                let ra = self.regs[*a as usize];
                let rb = self.regs[*b as usize];
                let rt = self.regs[*t as usize];
                self.regs[*dst as usize] = [
                    ra[0] + (rb[0] - ra[0]) * rt[0],
                    ra[1] + (rb[1] - ra[1]) * rt[1],
                    ra[2] + (rb[2] - ra[2]) * rt[2],
                    ra[3] + (rb[3] - ra[3]) * rt[3],
                ];
            }
            Inst::Abs(dst, src) => {
                let r = self.regs[*src as usize];
                self.regs[*dst as usize] = [abs_f32(r[0]), abs_f32(r[1]), abs_f32(r[2]), abs_f32(r[3])];
            }
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
            Inst::Sqrt(dst, src) => {
                let r = self.regs[*src as usize];
                self.regs[*dst as usize] = [
                    math::sqrt(r[0]), math::sqrt(r[1]),
                    math::sqrt(r[2]), math::sqrt(r[3]),
                ];
            }
            Inst::Rsqrt(dst, src) => {
                let r = self.regs[*src as usize];
                self.regs[*dst as usize] = [
                    rsqrt(r[0]), rsqrt(r[1]), rsqrt(r[2]), rsqrt(r[3]),
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
            Inst::Reflect(dst, i, n) => {
                let ri = self.regs[*i as usize];
                let rn = self.regs[*n as usize];
                let d = ri[0]*rn[0] + ri[1]*rn[1] + ri[2]*rn[2];
                self.regs[*dst as usize] = [
                    ri[0] - 2.0 * d * rn[0],
                    ri[1] - 2.0 * d * rn[1],
                    ri[2] - 2.0 * d * rn[2],
                    0.0,
                ];
            }
            Inst::TexSample(dst, sampler, coord) => {
                let unit = self.regs[*sampler as usize][0] as u32;
                let uv = self.regs[*coord as usize];
                self.regs[*dst as usize] = tex_sample(unit, uv[0], uv[1]);
            }
            Inst::MatMul4(dst, mat, vec) => {
                let v = self.regs[*vec as usize];
                let c0 = self.regs[*mat as usize];
                let c1 = self.regs[(*mat + 1) as usize];
                let c2 = self.regs[(*mat + 2) as usize];
                let c3 = self.regs[(*mat + 3) as usize];
                self.regs[*dst as usize] = [
                    c0[0]*v[0] + c1[0]*v[1] + c2[0]*v[2] + c3[0]*v[3],
                    c0[1]*v[0] + c1[1]*v[1] + c2[1]*v[2] + c3[1]*v[3],
                    c0[2]*v[0] + c1[2]*v[1] + c2[2]*v[2] + c3[2]*v[3],
                    c0[3]*v[0] + c1[3]*v[1] + c2[3]*v[2] + c3[3]*v[3],
                ];
            }
            Inst::MatMul3(dst, mat, vec) => {
                let v = self.regs[*vec as usize];
                let c0 = self.regs[*mat as usize];
                let c1 = self.regs[(*mat + 1) as usize];
                let c2 = self.regs[(*mat + 2) as usize];
                self.regs[*dst as usize] = [
                    c0[0]*v[0] + c1[0]*v[1] + c2[0]*v[2],
                    c0[1]*v[0] + c1[1]*v[1] + c2[1]*v[2],
                    c0[2]*v[0] + c1[2]*v[1] + c2[2]*v[2],
                    0.0,
                ];
            }
            Inst::Swizzle(dst, src, indices, count) => {
                let r = self.regs[*src as usize];
                let mut out = [0.0f32; 4];
                for i in 0..(*count as usize) {
                    out[i] = r[indices[i] as usize];
                }
                // For scalar swizzle (.x), broadcast to all
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
            Inst::CmpLt(dst, a, b) => {
                let ra = self.regs[*a as usize];
                let rb = self.regs[*b as usize];
                self.regs[*dst as usize] = [
                    if ra[0] < rb[0] { 1.0 } else { 0.0 },
                    if ra[1] < rb[1] { 1.0 } else { 0.0 },
                    if ra[2] < rb[2] { 1.0 } else { 0.0 },
                    if ra[3] < rb[3] { 1.0 } else { 0.0 },
                ];
            }
            Inst::CmpEq(dst, a, b) => {
                let ra = self.regs[*a as usize];
                let rb = self.regs[*b as usize];
                self.regs[*dst as usize] = [
                    if (ra[0] - rb[0]).abs() < 1e-6 { 1.0 } else { 0.0 },
                    if (ra[1] - rb[1]).abs() < 1e-6 { 1.0 } else { 0.0 },
                    if (ra[2] - rb[2]).abs() < 1e-6 { 1.0 } else { 0.0 },
                    if (ra[3] - rb[3]).abs() < 1e-6 { 1.0 } else { 0.0 },
                ];
            }
            Inst::Select(dst, cond, a, b) => {
                let rc = self.regs[*cond as usize];
                let ra = self.regs[*a as usize];
                let rb = self.regs[*b as usize];
                self.regs[*dst as usize] = [
                    if rc[0] != 0.0 { ra[0] } else { rb[0] },
                    if rc[1] != 0.0 { ra[1] } else { rb[1] },
                    if rc[2] != 0.0 { ra[2] } else { rb[2] },
                    if rc[3] != 0.0 { ra[3] } else { rb[3] },
                ];
            }
            Inst::IntToFloat(dst, src) => {
                let r = self.regs[*src as usize];
                self.regs[*dst as usize] = r; // Already floats in our representation
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

fn clamp_f32(x: f32, lo: f32, hi: f32) -> f32 {
    if x < lo { lo } else if x > hi { hi } else { x }
}

fn abs_f32(x: f32) -> f32 {
    if x < 0.0 { -x } else { x }
}

fn rsqrt(x: f32) -> f32 {
    if x > 1e-10 { 1.0 / math::sqrt(x) } else { 0.0 }
}
